//! The preprocessed players kept in `{home}/resolver/players`, and the preprocessing that
//! makes them, in this process or in a child.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use tokio::io::AsyncWriteExt;

use super::embed::valid_player_id;
use super::{Inner, KEEP_PLAYERS, NativeError, PREPROCESS_TIMEOUT, Player, STRAY_AFTER, failed};
use crate::Preprocessor;
use crate::js::{self, EJS_VERSION};

/// The player's `signatureTimestamp`, as yt-dlp finds it.
pub(crate) fn player_sts(js: &str) -> Option<u64> {
    ["signatureTimestamp", "sts"].iter().find_map(|key| {
        js.match_indices(key).find_map(|(i, _)| {
            let rest = js[i + key.len()..]
                .trim_start()
                .strip_prefix(':')?
                .trim_start();
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            (digits.len() == 5).then(|| digits.parse().ok()).flatten()
        })
    })
}

/// The installed player `id`, if it was preprocessed by these EJS scripts.
pub(crate) fn installed(players: &Path, id: &str) -> Option<Player> {
    if !valid_player_id(id) {
        return None;
    }
    let path = players.join(format!("{id}.js"));
    let mut head = [0u8; 128];
    let n = std::io::Read::read(&mut std::fs::File::open(&path).ok()?, &mut head).ok()?;
    let line = std::str::from_utf8(&head[..n]).ok()?.lines().next()?;
    // `// featherplay player {id} sts {sts} ejs {version}`
    let mut w = line.strip_prefix("// featherplay player ")?.split(' ');
    let (pid, _, sts, _, ejs) = (w.next()?, w.next()?, w.next()?, w.next()?, w.next()?);
    let sts = sts
        .parse()
        .ok()
        .filter(|_| pid == id && ejs == EJS_VERSION)?;
    Some(Player {
        id: id.to_string(),
        sts,
        path,
    })
}

/// The files in `players` with this extension, oldest first.
fn files(players: &Path, extension: &str) -> Vec<(SystemTime, PathBuf)> {
    let mut files: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(players)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let p = e.ok()?.path();
            let ok = p.extension().is_some_and(|x| x == extension);
            ok.then(|| Some((p.metadata().ok()?.modified().ok()?, p)))?
        })
        .collect();
    files.sort();
    files
}

/// The newest installed player that `good` takes: what answers while a page's own player
/// is being installed.
pub(super) fn newest(players: &Path, good: impl Fn(&str) -> bool) -> Option<Player> {
    files(players, "js")
        .into_iter()
        .rev()
        .filter_map(|(_, p)| installed(players, p.file_stem()?.to_str()?))
        .find(|p| good(&p.id))
}

/// Saves a preprocessed player and drops all but the newest `KEEP_PLAYERS`, each with its
/// bytecode (`js`).
pub(crate) fn install(players: &Path, id: &str, sts: u64, pre: &str) -> std::io::Result<Player> {
    if !valid_player_id(id) {
        return Err(std::io::Error::other("bad player id"));
    }
    std::fs::create_dir_all(players)?;
    let path = players.join(format!("{id}.js"));
    let body = format!("// featherplay player {id} sts {sts} ejs {EJS_VERSION}\n{pre}");
    write_atomic(&path, body.as_bytes())?;
    // An earlier file's bytecode isn't this one's.
    let _ = std::fs::remove_file(js::bytecode_path(&path));
    let others = files(players, "js").into_iter().filter(|(_, p)| *p != path);
    let others: Vec<_> = others.collect();
    let drop_n = (others.len() + 1).saturating_sub(KEEP_PLAYERS);
    for (_, p) in others.into_iter().take(drop_n) {
        let _ = std::fs::remove_file(js::bytecode_path(&p));
        let _ = std::fs::remove_file(p);
    }
    // A save cut short (a crash, the power) left its temporary file.
    for (written, part) in files(players, "part") {
        if written.elapsed().is_ok_and(|age| age > STRAY_AFTER) {
            let _ = std::fs::remove_file(part);
        }
    }
    Ok(Player {
        id: id.to_string(),
        sts,
        path,
    })
}

/// Numbers the temporary files of `write_atomic`, so two writes of one file at once (two
/// installs of a player, or two saves of the state) each rename their own.
static WRITES: AtomicU64 = AtomicU64::new(0);

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let part = path.with_extension(format!(
        "{}-{}.part",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

/// Loads `player` once, which saves its bytecode: the first song it answers then loads it
/// in ~15 ms instead of compiling 4 MB of JS (~190 ms).
pub(super) async fn warm(player: &Player) {
    let path = player.path.clone();
    let warmed = tokio::task::spawn_blocking(move || js::warm(&path)).await;
    if let Err(e) = warmed.map_err(|e| e.to_string()).and_then(|w| w) {
        log::warn!("player {} didn't answer once loaded: {e}", player.id);
    }
}

/// Why a player couldn't be preprocessed.
pub(super) enum PreprocessError {
    /// The work didn't start, so nothing is known about the player.
    NotStarted(NativeError),
    /// EJS ran on the player and failed, or ran past its own time limit, and said so.
    Unsolved(NativeError),
    /// The child preprocessing it ended before it said how it went (killed, crashed, hung
    /// past its own limit): nothing is known about the player, and it's tried again later.
    Interrupted(NativeError),
}

/// `work` on the blocking pool: it waits for a QuickJS thread, which an async thread
/// never does.
async fn blocking(
    work: impl FnOnce() -> Result<String, String> + Send + 'static,
) -> Result<String, PreprocessError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| PreprocessError::NotStarted(failed(e)))?
        .map_err(|e| PreprocessError::Unsolved(failed(e)))
}

/// Runs the process that preprocesses a player (`crate::preprocess_stdio`): the player's
/// JS goes to its stdin, the preprocessed player comes from its stdout. Killed at
/// `timeout`, or when the future is dropped.
async fn run_with_stdin(
    cmd: tokio::process::Command,
    stdin: &[u8],
    timeout: Duration,
) -> Result<String, PreprocessError> {
    let not_started =
        |e: String| PreprocessError::NotStarted(failed(format!("starting the preprocessor: {e}")));
    // Starting a process blocks for as long as an antivirus checks the exe, and a 30 MB one
    // can take a few hundred ms: not on an async thread, where resolves run.
    let mut child = tokio::task::spawn_blocking(move || crate::sidecars::spawn(cmd))
        .await
        .map_err(|e| not_started(e.to_string()))?
        .map_err(|e| not_started(e.to_string()))?;
    // Held for the run: if the app exits or crashes, the job's handle closes and the child
    // (~205 MB) ends with it instead of running out its own time limit.
    #[cfg(windows)]
    let process = child.raw_handle();
    #[cfg(not(windows))]
    let process = child.id();
    let _job = match process {
        Some(process) => featherplay_platform::process::kill_on_close(process)
            .inspect_err(|e| log::warn!("the preprocessor isn't tied to the app: {e}"))
            .ok(),
        None => None,
    };
    let unsolved = |m: String| PreprocessError::Unsolved(failed(m));
    let mut pipe = child
        .stdin
        .take()
        .expect("the caller's command pipes stdin");
    let run = async {
        // The child reads all of it before it writes, so nothing waits on a full pipe.
        pipe.write_all(stdin).await?;
        drop(pipe);
        child.wait_with_output().await
    };
    // A child that hangs past its own limit, or whose pipes broke, says nothing of the
    // player either.
    let interrupted = |m: String| PreprocessError::Interrupted(failed(m));
    let out = tokio::time::timeout(timeout, run)
        .await
        .map_err(|_| interrupted("the preprocessor timed out".into()))?
        .map_err(|e| interrupted(format!("the preprocessor: {e}")))?;
    if !out.status.success() {
        return Err(child_failed(
            out.status.code(),
            &String::from_utf8_lossy(&out.stderr),
        ));
    }
    String::from_utf8(out.stdout)
        .ok()
        .filter(|player| !player.is_empty())
        .ok_or_else(|| unsolved("the preprocessor gave no player".into()))
}

/// Why a preprocessing child that exited with `code` and `stderr` failed: EJS failed when
/// it exits with 1 and says why (`preprocess_stdio`); one that ended without a reason was
/// killed or crashed, which says nothing of the player.
fn child_failed(code: Option<i32>, stderr: &str) -> PreprocessError {
    let e = failed(format!(
        "the preprocessor exited with {}: {}",
        code.unwrap_or(-1),
        crate::sidecars::tail(stderr, 300)
    ));
    if code == Some(1) && !stderr.trim().is_empty() {
        PreprocessError::Unsolved(e)
    } else {
        PreprocessError::Interrupted(e)
    }
}

impl Inner {
    /// Runs EJS on a player's JS, as `self.preprocessor` says. A child that can't be
    /// started (an antivirus may refuse it) leaves the work to this process.
    pub(super) async fn preprocess(&self, js: String) -> Result<String, PreprocessError> {
        match &self.preprocessor {
            Preprocessor::Child(program) => {
                let mut cmd = crate::sidecars::hidden_command(program);
                cmd.arg(crate::PREPROCESS_ARG)
                    .stdin(std::process::Stdio::piped());
                match run_with_stdin(cmd, js.as_bytes(), PREPROCESS_TIMEOUT).await {
                    Err(PreprocessError::NotStarted(e)) => {
                        log::warn!("{e}; preprocessing the player in this process instead");
                        blocking(move || js::preprocess(&js)).await
                    }
                    result => result,
                }
            }
            Preprocessor::InProcess => blocking(move || js::preprocess(&js)).await,
            #[cfg(test)]
            Preprocessor::Stub(stub) => {
                let stub = stub.clone();
                blocking(move || stub(&js)).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::tests::{FAKE_PLAYER, home};

    #[test]
    fn the_sts_comes_from_the_player() {
        assert_eq!(
            player_sts("var a={signatureTimestamp:20752,b:1}"),
            Some(20752)
        );
        assert_eq!(player_sts("x.sts : 20391;"), Some(20391));
        assert_eq!(player_sts("sts:1; signatureTimestamp:123"), None);
    }

    #[test]
    fn the_transform_cache_keeps_the_newest_players() {
        let root = home("cache");
        let dir = root.join("players");
        assert_eq!(installed(&dir, "aaaa0001"), None);
        let p = install(&dir, "aaaa0001", 20000, FAKE_PLAYER).unwrap();
        assert_eq!(installed(&dir, "aaaa0001"), Some(p.clone()));
        assert_eq!(p.sts, 20000);
        let text = std::fs::read_to_string(&p.path).unwrap();
        assert!(text.ends_with(FAKE_PLAYER), "the player follows the header");
        let bytecode = js::bytecode_path(&p.path);
        std::fs::write(&bytecode, b"its bytecode").unwrap();
        for (i, id) in ["aaaa0002", "aaaa0003", "aaaa0004"].iter().enumerate() {
            std::thread::sleep(Duration::from_millis(20));
            install(&dir, id, 20001 + i as u64, FAKE_PLAYER).unwrap();
        }
        assert_eq!(installed(&dir, "aaaa0001"), None, "the oldest went");
        assert!(!bytecode.exists(), "and its bytecode with it");
        assert_eq!(newest(&dir, |_| true).unwrap().id, "aaaa0004");
        for id in ["aaaa0002", "aaaa0003", "aaaa0004"] {
            assert!(installed(&dir, id).is_some(), "{id}");
        }
        // Made by other EJS scripts: made again.
        let other = format!("// featherplay player bbbb0001 sts 1 ejs 0.0.1\n{FAKE_PLAYER}");
        std::fs::write(dir.join("bbbb0001.js"), other).unwrap();
        assert_eq!(installed(&dir, "bbbb0001"), None);
        assert!(install(&dir, "../evil", 1, "").is_err());
        assert_eq!(installed(&dir, "../aaaa0002"), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A preprocessing child that ended without a reason (killed, crashed) says nothing of
    /// the player; one that failed says why on stderr.
    #[test]
    fn a_preprocessor_that_ends_without_a_reason_leaves_the_player_good() {
        let unsolved =
            |code, err: &str| matches!(child_failed(code, err), PreprocessError::Unsolved(_));
        assert!(unsolved(Some(1), "EJS: no n function"));
        assert!(!unsolved(Some(1), ""), "killed (TerminateProcess's code 1)");
        assert!(!unsolved(Some(-1_073_741_819), ""), "crashed");
        assert!(!unsolved(None, ""));
    }
}
