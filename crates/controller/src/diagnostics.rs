//! Diagnostics: the log file, crash reports, and the text Settings > About copies for a bug
//! report. A release build is a GUI program, so its stderr goes nowhere; these are what's
//! left after something goes wrong.
//!
//! Every line (`log!`, and the crates' `log` macros through `Facade`) is redacted, then
//! handed to the `featherplay-log` thread, which appends it to `featherplay.log` in
//! `log_dir`. The thread sleeps on its channel, so an idle app never wakes for the log.
//! Debug builds print every line to stderr too, as does a process that never called `init`
//! (the tests, `--snapshot`).

pub mod clock;
mod crash;
mod logfile;
mod redact;
mod report;

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use featherplay_core::sync::LockExt;
use featherplay_store::{Paths, Settings};

use clock::Stamp;
use logfile::LogFile;

pub use redact::redact;
pub use report::Report;

/// How many resolver errors Copy diagnostics lists.
const RECENT_ERRORS: usize = 5;
/// How many song starts Copy diagnostics lists, with the step that gave each its stream.
const RECENT_SONGS: usize = 5;
/// How long `flush` waits for the log thread.
const FLUSH_WAIT: Duration = Duration::from_secs(1);

enum Msg {
    Line(String),
    /// Answered once every line before it is written.
    Flush(mpsc::SyncSender<()>),
}

static LOG: OnceLock<Sender<Msg>> = OnceLock::new();
/// The yt-dlp and Deno versions, once read.
static TOOLS: Mutex<(Option<String>, Option<String>)> = Mutex::new((None, None));
static RECENT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// The last song starts (`song_started`), oldest first.
static PLAYED: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// How the last helpers download failed (`SidecarError::summary`).
static HELPERS: Mutex<Option<String>> = Mutex::new(None);
/// Why the system's credential store wouldn't keep the last sign-in (`sign_in_kept`).
static SIGN_IN_SAVE: Mutex<Option<String>> = Mutex::new(None);

/// Time since the process started; the first call starts the clock.
pub fn uptime() -> Duration {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

/// `{state}/logs` (the cache folder but on Linux): the log, the one before it and the crash
/// files.
pub fn log_dir(paths: &Paths) -> PathBuf {
    paths.state.join("logs")
}

/// At startup: the log file, the panic hook, and a first line saying which featherplay and
/// which system this is (`os`). A log folder that can't be written leaves only stderr.
pub fn start(paths: &Paths) {
    let dir = log_dir(paths);
    if let Err(e) = init(&dir) {
        write(format_args!("no log file: {e}"));
    }
    install_panic_hook(dir);
    write(format_args!(
        "featherplay {} on {}",
        env!("CARGO_PKG_VERSION"),
        os()
    ));
}

/// Starts writing the log to `dir` and routes the `log` crate's records there. Only the
/// first call in a process does anything.
pub fn init(dir: &Path) -> Result<(), String> {
    if LOG.get().is_some() {
        return Ok(());
    }
    let file = LogFile::open(dir, logfile::LIMIT)
        .map_err(|e| format!("{}: {e}", dir.join(logfile::CURRENT).display()))?;
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("featherplay-log".into())
        .spawn(move || run(file, rx))
        .map_err(|e| format!("log thread: {e}"))?;
    if LOG.set(tx).is_ok() {
        // Another logger set first (none in the app) keeps its place.
        if log::set_logger(&Facade).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
    }
    Ok(())
}

/// The log thread: blocks on the channel, writes what comes, and stops when the app ends.
#[allow(
    clippy::print_stderr,
    reason = "the log can't report its own failure to itself"
)]
fn run(mut file: LogFile, rx: Receiver<Msg>) {
    let mut failed = false;
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Line(line) => {
                if let Err(e) = file.write(&line)
                    && !std::mem::replace(&mut failed, true)
                {
                    eprintln!("featherplay: writing the log: {e}");
                }
            }
            Msg::Flush(done) => {
                let _ = file.flush();
                let _ = done.send(());
            }
        }
    }
}

/// Writes a crash file to `dir` for every panic, then runs the default hook.
pub fn install_panic_hook(dir: PathBuf) {
    crash::install(dir);
}

/// One log line: the seconds since start, then `args`. What `log!` calls.
pub fn write(args: fmt::Arguments<'_>) {
    line(&args.to_string());
}

#[allow(
    clippy::print_stderr,
    reason = "the logger's own echo: debug builds and runs before it starts"
)]
fn line(msg: &str) {
    let msg = redact(msg);
    let up = uptime().as_secs_f64();
    let tagged = format!("[{up:>8.3}] {msg}");
    let log = LOG.get();
    if cfg!(debug_assertions) || log.is_none() {
        eprintln!("{tagged}");
    }
    if let Some(tx) = log {
        // A message of several lines (a stderr tail) keeps its lines under its stamp.
        let indented = tagged.replace('\n', "\n    ");
        let _ = tx.send(Msg::Line(format!("{} {indented}\n", Stamp::now().line())));
    }
}

/// Waits (up to a second) until every line written so far is in the file.
pub fn flush() {
    let Some(tx) = LOG.get() else { return };
    let (done, wait) = mpsc::sync_channel(1);
    if tx.send(Msg::Flush(done)).is_ok() {
        let _ = wait.recv_timeout(FLUSH_WAIT);
    }
}

/// The crates' `log` records: featherplay's own from `info` up, other crates' warnings and
/// errors only (their debug output would fill the file).
struct Facade;

fn ours(target: &str) -> bool {
    target.starts_with("featherplay")
}

impl log::Log for Facade {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        let most = if ours(m.target()) {
            log::Level::Info
        } else if cfg!(target_os = "linux") && m.target().starts_with("arboard") {
            // Slint's clipboard warns at every start on a compositor without the Wayland
            // data-control protocol (GNOME's, WSLg's), then uses the X11 one as it should.
            log::Level::Error
        } else {
            log::Level::Warn
        };
        m.level() <= most
    }

    fn log(&self, r: &log::Record<'_>) {
        if !self.enabled(r.metadata()) {
            return;
        }
        if ours(r.target()) {
            line(&r.args().to_string());
        } else {
            line(&format!("{} {}: {}", r.level(), r.target(), r.args()));
        }
    }

    fn flush(&self) {
        flush();
    }
}

/// Which system this is, read once: `Windows 11 25H2, build 26200.7171`; on Linux the
/// distribution's name (`Ubuntu 24.04.3 LTS`).
pub fn os() -> &'static str {
    static OS: OnceLock<String> = OnceLock::new();
    OS.get_or_init(featherplay_platform::system::os_version)
}

/// What Copy diagnostics and a crash file call the `os` line: `Windows` there, as before,
/// and `System` elsewhere, whose line is the distribution's name.
pub(crate) const OS_LABEL: &str = if cfg!(windows) { "Windows" } else { "System" };

/// The yt-dlp and Deno versions, once the controller has read them (`None`: not installed,
/// or it failed). Logged once.
pub fn set_tool_versions(ytdlp: Option<String>, deno: Option<String>) {
    let shown = |v: &Option<String>| v.clone().unwrap_or_else(|| "unknown".into());
    write(format_args!(
        "yt-dlp {}, deno {}",
        shown(&ytdlp),
        shown(&deno)
    ));
    *TOOLS.lock_or_recover() = (ytdlp, deno);
}

/// A resolve failed: its log line (the stage lines from `featherplay-streams` come before
/// it), and one of the errors Copy diagnostics lists.
pub fn resolve_failed(id: &str, took_secs: f64, msg: &str) {
    write(format_args!(
        "resolve {id} failed in {took_secs:.1}s: {msg}"
    ));
    let mut recent = RECENT.lock_or_recover();
    if recent.len() == RECENT_ERRORS {
        recent.pop_front();
    }
    recent.push_back(format!("{} {id}: {msg}", Stamp::now().time()));
}

/// A song started playing from a stream `via` gave (`featherplay_streams::Via::label`):
/// one of the song starts Copy diagnostics lists.
pub fn song_started(id: &str, via: &str) {
    let mut played = PLAYED.lock_or_recover();
    if played.len() == RECENT_SONGS {
        played.pop_front();
    }
    played.push_back(format!("{} {id}: {via}", Stamp::now().time()));
}

/// The helpers download failed: Copy diagnostics names its kind, host, HTTP status and os
/// error (the log has the rest, from `featherplay-streams`).
pub fn helpers_failed(e: &featherplay_streams::SidecarError) {
    *HELPERS.lock_or_recover() = Some(e.summary());
}

/// A new sign-in was stored for the next start, or `refused`: the credential store's error,
/// with the system's code and never the cookie header. Its log line, and Copy diagnostics names
/// it until a sign-in is stored.
pub fn sign_in_kept(refused: Option<String>) {
    if let Some(why) = &refused {
        write(format_args!("saving the sign-in failed: {why}"));
    }
    *SIGN_IN_SAVE.lock_or_recover() = refused;
}

/// What Copy diagnostics says now. `signed_in` only: the account itself never reaches it.
pub fn report(signed_in: bool, settings: &Settings) -> Report {
    let (ytdlp, deno) = TOOLS.lock_or_recover().clone();
    let errors = RECENT.lock_or_recover().iter().cloned().collect();
    let played = PLAYED.lock_or_recover().iter().cloned().collect();
    let helpers = HELPERS.lock_or_recover().clone();
    Report {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os: os().to_string(),
        ytdlp,
        deno,
        js_engine: featherplay_streams::js_engine(),
        output_device: settings.output_device.clone(),
        signed_in,
        lastfm: settings.scrobbling && settings.lastfm_enabled,
        listenbrainz: settings.scrobbling && settings.listenbrainz_enabled,
        discord: settings.discord,
        proxy: Some(featherplay_proxy::summary()),
        helpers,
        sign_in_save: SIGN_IN_SAVE.lock_or_recover().clone(),
        errors,
        played,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_five_resolve_errors_are_listed() {
        for n in 0..7 {
            resolve_failed(&format!("id{n}"), 1.0, &format!("failure {n}"));
        }
        let r = report(false, &Settings::default());
        assert_eq!(r.errors.len(), RECENT_ERRORS);
        assert!(r.errors[0].ends_with("id2: failure 2"), "{:?}", r.errors);
        assert!(r.errors[4].ends_with("id6: failure 6"), "{:?}", r.errors);
    }

    #[test]
    fn the_report_reads_the_settings() {
        let settings = Settings {
            output_device: Some("Speakers".into()),
            scrobbling: true,
            lastfm_enabled: true,
            listenbrainz_enabled: false,
            discord: true,
            ..Settings::default()
        };
        set_tool_versions(Some("2026.10.26".into()), None);
        let r = report(true, &settings);
        assert_eq!(r.output_device.as_deref(), Some("Speakers"));
        assert!(r.signed_in && r.lastfm && !r.listenbrainz && r.discord);
        assert_eq!(r.ytdlp.as_deref(), Some("2026.10.26"));
        assert_eq!(r.deno, None);
        assert_eq!(r.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn scrobbling_off_turns_both_services_off() {
        let settings = Settings {
            scrobbling: false,
            lastfm_enabled: true,
            listenbrainz_enabled: true,
            ..Settings::default()
        };
        let r = report(false, &settings);
        assert!(!r.lastfm && !r.listenbrainz);
    }
}
