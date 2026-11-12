use super::*;
use crate::native::test_server;
use crate::sidecars::{DENO_EXE, YTDLP_EXE};

fn test_home(name: &str) -> PathBuf {
    let base = std::env::var("FEATHERPLAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"));
    let home = base
        .join("test-streams")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("sidecars")).unwrap();
    home
}

/// The daily `yt-dlp -U` runs only when the last one is a day old (or there's none), and
/// only with the helpers there.
#[tokio::test]
async fn the_daily_update_runs_when_the_last_one_is_a_day_old() {
    let home = test_home("update-daily");
    let dir = home.join("sidecars");
    let r = YtDlpResolver::with_native(Sidecars { dir: dir.clone() }, None, None);
    assert!(!r.update_daily().await, "no helpers, nothing to update");

    // Not programs: an update fails at once, which still counts as one that ran.
    std::fs::write(dir.join(YTDLP_EXE), b"not a program").unwrap();
    std::fs::write(dir.join(DENO_EXE), b"not a program").unwrap();
    assert!(r.update_daily().await, "never updated");

    let stamp = dir.join("yt-dlp.updated");
    std::fs::write(&stamp, b"").unwrap();
    assert!(!r.update_daily().await, "updated today");

    let day_old = SystemTime::now() - Duration::from_secs(25 * 3600);
    std::fs::File::options()
        .write(true)
        .open(&stamp)
        .unwrap()
        .set_modified(day_old)
        .unwrap();
    assert!(r.update_daily().await, "a day later");
    assert!(
        !r.helpers_tried(),
        "a daily update isn't the one after a failure"
    );
    std::fs::remove_dir_all(&home).unwrap();
}

/// The first failure of a session updates yt-dlp at once, however fresh the daily stamp;
/// after that, only once the stamp is a day old. No update loop.
#[tokio::test]
async fn the_helpers_are_forced_up_to_date_once_a_session() {
    let home = test_home("update-once");
    let dir = home.join("sidecars");
    // Not a program: the update fails at once, which still counts as an update tried.
    std::fs::write(dir.join(YTDLP_EXE), b"not a program").unwrap();
    let stamp = dir.join("yt-dlp.updated");
    std::fs::write(&stamp, b"").unwrap();
    let r = YtDlpResolver::with_native(Sidecars { dir: dir.clone() }, None, None);

    assert!(!r.helpers_tried());
    assert!(
        r.update_helpers().await,
        "forced, though the stamp is fresh"
    );
    assert!(r.helpers_tried());
    assert!(!r.update_helpers().await, "not twice in a session");

    let day_old = SystemTime::now() - Duration::from_secs(25 * 3600);
    std::fs::File::options()
        .write(true)
        .open(&stamp)
        .unwrap()
        .set_modified(day_old)
        .unwrap();
    assert!(r.update_helpers().await, "the daily update, a day later");
    std::fs::remove_dir_all(&home).unwrap();
}

/// yt-dlp says why a format is missing only in a warning ("n challenge solving failed",
/// "formats have been skipped"), so the stderr tail a failure logs must keep them.
#[test]
fn a_run_keeps_yt_dlps_warnings_for_the_failure_log() {
    let r = YtDlpResolver::with_native(
        Sidecars {
            dir: PathBuf::from("sidecars"),
        },
        None,
        None,
    );
    let cmd = r.command(
        &TrackId("kXbeLiZY6jc".into()),
        None,
        Some(EMBEDDED_ARGS),
        Quality::Standard,
    );
    let args = command_args(&cmd);
    assert!(!args.iter().any(|a| a == "--no-warnings"), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["-f", "140"]), "{args:?}");
    assert_eq!(
        args.last().map(String::as_str),
        Some("https://music.youtube.com/watch?v=kXbeLiZY6jc")
    );
}

#[tokio::test]
async fn a_check_asks_for_the_first_byte_and_reads_the_answer() {
    let (base, log) = test_server::serve(|_, path, _| match path {
        "/plays" => (206, "x".into()),
        "/refused" => (403, String::new()),
        _ => (500, String::new()),
    });
    let http = reqwest::Client::new();
    let at = |path: &str| format!("{base}{path}");
    assert_eq!(first_byte(&http, &at("/plays")).await, Check::Plays);
    assert_eq!(first_byte(&http, &at("/refused")).await, Check::Refused);
    assert_eq!(first_byte(&http, &at("/broken")).await, Check::Unknown);
    assert_eq!(log.lock().unwrap().len(), 3, "one request each, no retry");
    // Nothing listens on port 1.
    assert_eq!(
        first_byte(&http, "http://127.0.0.1:1/x").await,
        Check::Unknown
    );
}

#[tokio::test]
async fn a_failed_native_resolve_falls_back_to_yt_dlp() {
    let base = std::env::var("FEATHERPLAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"));
    let home = base
        .join("test-streams")
        .join(format!("fallback-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("sidecars")).unwrap();
    // The embed page and a player whose preprocessing fails: EJS finds nothing in it.
    // No yt-dlp, so the fallback ends in `ToolMissing`.
    let (url, log) = test_server::serve(|_, path, _| {
        if path.starts_with("/embed/") {
            (200, r#"<script src="/s/player/abcd1234/base.js">"#.into())
        } else {
            (200, "var x={signatureTimestamp:20752};".into())
        }
    });
    let native = NativeResolver::with_dirs(home.join("resolver"), &url);
    let r = YtDlpResolver::with_native(
        Sidecars {
            dir: home.join("sidecars"),
        },
        None,
        native,
    );
    let err = r.resolve(&TrackId("h46YpWrg55Q".into())).await.unwrap_err();
    assert_eq!(err, ResolveError::ToolMissing, "yt-dlp was tried next");
    let id = TrackId("h46YpWrg55Q".into());
    let why = r.take_fallback_reason(&id).expect("a reason");
    assert!(why.contains("EJS"), "{why}");
    assert_eq!(
        *log.lock().unwrap(),
        [
            "GET /embed/h46YpWrg55Q?html5=1",
            "GET /s/player/abcd1234/player_ias.vflset/en_US/base.js"
        ]
    );
    assert_eq!(r.take_fallback_reason(&id), None, "told once");
    std::fs::remove_dir_all(&home).unwrap();
}

/// A first run: the helpers are missing, so resolves download them, from a local
/// server here (the pins and the GitHub-only policy are `Downloads::pinned`'s).
mod first_run {
    use std::io::Write;
    use std::sync::atomic::AtomicUsize;

    use featherplay_core::failure::HelperFault;
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::native::test_server::Log;

    const YTDLP: &[u8] = b"not really yt-dlp";

    fn sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// deno's release zip, with a stand-in `deno.exe` (`deno`).
    fn deno_zip() -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file(DENO_EXE, stored).unwrap();
        zip.write_all(b"not really deno").unwrap();
        zip.finish().unwrap().into_inner()
    }

    /// Serves the two helpers, each after `delay`; the first `failures` requests for
    /// yt-dlp answer 503. The embed page and a player for the native request too.
    fn helpers_server(failures: usize, delay: Duration) -> (Downloads, String, Log) {
        failing_server(failures, 503, delay)
    }

    /// `helpers_server`, with the failures answering `status`.
    fn failing_server(failures: usize, status: u16, delay: Duration) -> (Downloads, String, Log) {
        let zip = deno_zip();
        let deno_sha = sha256(&zip);
        let failed = AtomicUsize::new(0);
        let (base, log) = test_server::serve_bytes(move |path| {
            if path == "/yt-dlp.exe" || path == "/deno.zip" {
                std::thread::sleep(delay);
            }
            match path {
                "/yt-dlp.exe" if failed.fetch_add(1, Ordering::SeqCst) < failures => {
                    (status, b"try later".to_vec())
                }
                "/yt-dlp.exe" => (200, YTDLP.to_vec()),
                "/deno.zip" => (200, zip.clone()),
                p if p.starts_with("/embed/") => (
                    200,
                    br#"<script src="/s/player/abcd1234/base.js">"#.to_vec(),
                ),
                _ => (200, b"var x={signatureTimestamp:20752};".to_vec()),
            }
        });
        let downloads = Downloads::local(&[&base], &sha256(YTDLP), &deno_sha);
        (downloads, base, log)
    }

    fn requests(log: &Log, path: &str) -> usize {
        let want = format!("GET {path}");
        log.lock().unwrap().iter().filter(|l| **l == want).count()
    }

    fn resolver(home: &Path, downloads: Downloads) -> YtDlpResolver {
        let dir = home.join("sidecars");
        YtDlpResolver::with_native(Sidecars { dir }, None, None).with_downloads(downloads)
    }

    /// Past the helpers: the stand-ins aren't programs, so yt-dlp fails to start.
    fn went_on(result: &Result<ResolvedStream, ResolveError>) -> bool {
        matches!(result, Err(ResolveError::ToolFailed { .. }))
    }

    /// The native request needs no helper, so it goes first. The song waits for the
    /// helpers only once that gave nothing and yt-dlp is next, and says so then.
    #[tokio::test]
    async fn a_resolve_waits_for_the_helpers_only_once_the_native_step_failed() {
        let home = test_home("first-run-wait");
        let (downloads, base, log) = helpers_server(0, Duration::ZERO);
        let dir = home.join("sidecars");
        // Its player can't be preprocessed (EJS finds nothing in it), so it fails.
        let native = NativeResolver::with_dirs(home.join("resolver"), &base);
        let told = std::sync::Arc::new(Mutex::new(Vec::new()));
        let waits = told.clone();
        let r = YtDlpResolver::with_native(Sidecars { dir: dir.clone() }, None, native)
            .with_downloads(downloads)
            .on_helpers_wait(move |id| waits.lock().unwrap().push(id.clone()));
        let id = TrackId("h46YpWrg55Q".into());

        let result = r.resolve(&id).await;

        assert!(went_on(&result), "got {result:?}");
        assert!(dir.join(YTDLP_EXE).is_file() && dir.join(DENO_EXE).is_file());
        assert_eq!(
            *log.lock().unwrap(),
            [
                "GET /embed/h46YpWrg55Q?html5=1",
                "GET /s/player/abcd1234/player_ias.vflset/en_US/base.js",
                "GET /yt-dlp.exe",
                "GET /deno.zip",
            ],
            "the native request comes before the downloads"
        );
        assert_eq!(
            *told.lock().unwrap(),
            std::slice::from_ref(&id),
            "told once"
        );
        assert!(!r.waits_for_helpers(&id), "the wait is over");
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[tokio::test]
    async fn resolves_at_once_share_one_download() {
        let home = test_home("first-run-share");
        let (downloads, _, log) = helpers_server(0, Duration::from_millis(300));
        let r = resolver(&home, downloads);

        let (one, two) = (TrackId("aaaaaaaaaaa".into()), TrackId("bbbbbbbbbbb".into()));
        let (started, a, b) = tokio::join!(r.install_helpers(), r.resolve(&one), r.resolve(&two));

        assert_eq!(started, Ok(()));
        assert!(went_on(&a) && went_on(&b), "got {a:?} and {b:?}");
        assert_eq!(
            (requests(&log, "/yt-dlp.exe"), requests(&log, "/deno.zip")),
            (1, 1)
        );
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// A server error is tried twice more, after a pause, before the download fails.
    #[tokio::test]
    async fn a_download_that_fails_twice_is_tried_again() {
        let home = test_home("first-run-again");
        let (downloads, _, log) = helpers_server(2, Duration::ZERO);
        let r = resolver(&home, downloads);

        let result = r.resolve(&TrackId("h46YpWrg55Q".into())).await;

        assert!(went_on(&result), "got {result:?}");
        assert_eq!(requests(&log, "/yt-dlp.exe"), 3);
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// GitHub can't be reached for yt-dlp and sends a damaged Deno: both come from the
    /// second source instead, after GitHub's own tries.
    #[tokio::test]
    async fn what_github_cant_give_comes_from_the_second_source() {
        let home = test_home("first-run-second");
        let (github, github_log) = test_server::serve_bytes(|path| match path {
            "/yt-dlp.exe" => (503, b"down".to_vec()),
            _ => (200, b"not the zip".to_vec()),
        });
        let (_, second, second_log) = helpers_server(0, Duration::ZERO);
        let downloads = Downloads::local(&[&github, &second], &sha256(YTDLP), &sha256(&deno_zip()));
        let r = resolver(&home, downloads);

        let result = r.resolve(&TrackId("h46YpWrg55Q".into())).await;

        assert!(went_on(&result), "got {result:?}");
        assert_eq!(
            (
                requests(&github_log, "/yt-dlp.exe"),
                requests(&github_log, "/deno.zip")
            ),
            (3, 1),
            "a server error is tried again, a damaged file isn't"
        );
        assert_eq!(
            (
                requests(&second_log, "/yt-dlp.exe"),
                requests(&second_log, "/deno.zip")
            ),
            (1, 1)
        );
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// Both sources failing: the toast says what GitHub's failure was.
    #[tokio::test]
    async fn a_file_neither_source_gives_fails_with_githubs_fault() {
        let home = test_home("first-run-neither");
        let (github, _) = test_server::serve_bytes(|_| (503, b"down".to_vec()));
        let (second, _) = test_server::serve_bytes(|_| (404, b"gone".to_vec()));
        let r = resolver(&home, Downloads::local(&[&github, &second], "00", "00"));

        let result = r.resolve(&TrackId("h46YpWrg55Q".into())).await;

        let Err(ResolveError::Sidecar(e)) = &result else {
            panic!("got {result:?}");
        };
        assert_eq!((e.fault, e.status), (HelperFault::Unreachable, Some(503)));
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// A file that doesn't match its pin is deleted, and said to be damaged.
    #[tokio::test]
    async fn a_download_that_doesnt_match_its_pin_is_damaged() {
        let home = test_home("first-run-damaged");
        let (_, base, _) = helpers_server(0, Duration::ZERO);
        let wrong = sha256(b"another file");
        let r = resolver(&home, Downloads::local(&[&base], &wrong, &wrong));

        let result = r.resolve(&TrackId("h46YpWrg55Q".into())).await;

        let Err(ResolveError::Sidecar(e)) = &result else {
            panic!("got {result:?}");
        };
        assert_eq!(e.fault, HelperFault::Damaged, "{e:?}");
        assert!(
            !home
                .join("sidecars")
                .join(format!("{YTDLP_EXE}.part"))
                .exists()
        );
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// A missing file stays missing: no second try.
    #[tokio::test]
    async fn a_download_that_isnt_there_is_not_tried_again() {
        let home = test_home("first-run-404");
        let (downloads, _, log) = failing_server(1, 404, Duration::ZERO);
        let r = resolver(&home, downloads);

        let result = r.resolve(&TrackId("h46YpWrg55Q".into())).await;

        assert!(
            matches!(result, Err(ResolveError::Sidecar(_))),
            "got {result:?}"
        );
        assert_eq!(requests(&log, "/yt-dlp.exe"), 1);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[tokio::test]
    async fn a_failed_download_fails_the_resolve_and_the_next_resolve_tries_again() {
        let home = test_home("first-run-retry");
        let (downloads, _, log) = helpers_server(3, Duration::ZERO);
        let r = resolver(&home, downloads);
        let id = TrackId("h46YpWrg55Q".into());

        let first = r.resolve(&id).await;
        let Err(ResolveError::Sidecar(e)) = &first else {
            panic!("got {first:?}");
        };
        assert_eq!(
            (e.fault, e.host.as_deref(), e.status),
            (HelperFault::Unreachable, Some("127.0.0.1"), Some(503))
        );
        assert!(e.message.contains("503"), "{e:?}");
        assert_eq!(requests(&log, "/yt-dlp.exe"), 3, "three tries, no loop");

        let second = r.resolve(&id).await;
        assert!(went_on(&second), "got {second:?}");
        assert_eq!(requests(&log, "/yt-dlp.exe"), 4);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[tokio::test]
    async fn resolves_waiting_on_a_failed_download_share_its_error() {
        let home = test_home("first-run-fail-shared");
        let (downloads, _, log) = helpers_server(usize::MAX, Duration::from_millis(300));
        let r = resolver(&home, downloads);

        let (one, two) = (TrackId("aaaaaaaaaaa".into()), TrackId("bbbbbbbbbbb".into()));
        let (a, b) = tokio::join!(r.resolve(&one), r.resolve(&two));

        assert!(matches!(a, Err(ResolveError::Sidecar(_))), "got {a:?}");
        assert_eq!(a, b);
        assert_eq!(
            requests(&log, "/yt-dlp.exe"),
            3,
            "one download, tried three times"
        );
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[tokio::test]
    async fn a_fresh_download_counts_as_todays_update() {
        let home = test_home("first-run-stamp");
        let (downloads, _, _) = helpers_server(0, Duration::ZERO);
        let r = resolver(&home, downloads);

        assert_eq!(r.install_helpers().await, Ok(()));

        // The pinned release is new enough today: no `yt-dlp -U` right after it.
        assert!(!r.update_daily().await);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[tokio::test]
    async fn nothing_downloads_when_the_helpers_are_there() {
        let home = test_home("first-run-present");
        let dir = home.join("sidecars");
        std::fs::write(dir.join(YTDLP_EXE), YTDLP).unwrap();
        std::fs::write(dir.join(DENO_EXE), b"not really deno").unwrap();
        let (downloads, _, log) = helpers_server(0, Duration::ZERO);
        let r = resolver(&home, downloads);

        assert_eq!(r.install_helpers().await, Ok(()));
        let result = r.resolve(&TrackId("h46YpWrg55Q".into())).await;

        assert!(went_on(&result), "got {result:?}");
        assert!(log.lock().unwrap().is_empty(), "{:?}", log.lock().unwrap());
        std::fs::remove_dir_all(&home).unwrap();
    }
}

/// Behind a proxy, yt-dlp goes through the one the HTTP clients take when its run
/// starts: a script's or auto-detection's answer may have come since the resolver was made.
#[test]
fn a_proxy_is_handed_to_yt_dlp() {
    let mut r = YtDlpResolver::with_native(
        Sidecars {
            dir: PathBuf::from("sidecars"),
        },
        None,
        None,
    );
    let id = TrackId("kXbeLiZY6jc".into());
    let args = command_args(&r.command(&id, None, None, Quality::High));
    assert!(!args.iter().any(|a| a == "--proxy"), "{args:?}");

    r.proxy = || Proxy::parse("http://proxy.corp:3128");
    let args = command_args(&r.command(&id, None, None, Quality::High));
    assert!(
        args.windows(2)
            .any(|w| w == ["--proxy", "http://proxy.corp:3128/"]),
        "{args:?}"
    );
}

/// High asks yt-dlp for Opus (251), else AAC (140); Standard for AAC only.
#[test]
fn the_quality_setting_picks_the_format_yt_dlp_is_asked_for() {
    let r = YtDlpResolver::with_native(
        Sidecars {
            dir: PathBuf::from("sidecars"),
        },
        None,
        None,
    );
    let id = TrackId("kXbeLiZY6jc".into());
    let format = |r: &YtDlpResolver| {
        let args = command_args(&r.command(&id, None, None, r.quality()));
        let i = args.iter().position(|a| a == "-f").expect("-f");
        args[i + 1].clone()
    };
    assert_eq!(format(&r), "251/140", "High by default");
    r.set_quality(Quality::Standard);
    assert_eq!(format(&r), "140");
    r.set_quality(Quality::High);
    assert_eq!(format(&r), "251/140");
}

fn command_args(cmd: &tokio::process::Command) -> Vec<String> {
    cmd.as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}
