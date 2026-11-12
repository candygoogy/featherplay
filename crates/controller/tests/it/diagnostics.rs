//! The log file and crash files, in a process of their own each (nextest): `log!` and the
//! crates' `log` records reach `featherplay.log` with their secrets masked; a panic writes a
//! crash file and a log line.

use std::path::PathBuf;

use featherplay_controller::diagnostics;

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var("FEATHERPLAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"));
    let dir = base
        .join("test-diagnostics")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn read_log(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("featherplay.log")).unwrap_or_default()
}

#[test]
fn log_lines_reach_the_file_without_their_secrets() {
    let dir = scratch("log");
    diagnostics::init(&dir).expect("init");
    featherplay_controller::log!("plain line {}", 42);
    featherplay_controller::log!(
        "request: Cookie: SAPISID=secret-one/A; __Secure-3PSID=secret-two"
    );
    featherplay_controller::log!("cookies SID=secret-three; HSID=secret-four");
    featherplay_controller::log!(
        "GET https://ws.audioscrobbler.com/2.0/?method=auth.getSession&api_key=secret-five&token=secret-six&api_sig=secret-seven"
    );
    featherplay_controller::log!("scrobble failed: sk=secret-eight");
    featherplay_controller::log!("ListenBrainz: Authorization: Token secret-nine");
    featherplay_controller::log!(r#"answer {{"session_key":"secret-ten","name":"x"}}"#);
    // A crate's record, through the `log` facade.
    log::warn!(target: "featherplay_lastfm", "token=secret-eleven failed");
    diagnostics::flush();

    let text = read_log(&dir);
    for n in [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
    ] {
        assert!(
            !text.contains(&format!("secret-{n}")),
            "secret-{n} in\n{text}"
        );
    }
    assert!(text.contains("plain line 42"), "{text}");
    assert!(
        text.contains("token=*** failed"),
        "the crate's line is missing:\n{text}"
    );
    // A wall-clock time, then the uptime: `2026-11-02 14:03:11.123 [   0.012] plain line 42`.
    let line = text.lines().find(|l| l.ends_with("plain line 42")).unwrap();
    let b = line.as_bytes();
    assert!(
        b[4] == b'-' && b[10] == b' ' && b[13] == b':' && b[19] == b'.',
        "{line}"
    );
    assert!(line[23..].starts_with(" ["), "{line}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[cfg(not(windows))]
fn log_lines_name_no_home_folder() {
    let dir = scratch("home");
    diagnostics::init(&dir).expect("init");
    // This machine's own home, so no path is written into the test.
    let home = std::env::var("HOME").expect("HOME");
    featherplay_controller::log!("cache {home}/.cache/featherplay, logs {home}/.local/state");
    diagnostics::flush();

    let text = read_log(&dir);
    let user = home.rsplit('/').next().expect("a user folder");
    assert!(
        !text.contains(&format!("/{user}/")),
        "the home folder is in\n{text}"
    );
    assert!(
        text.contains("cache %HOME%/.cache/featherplay, logs %HOME%/.local/state"),
        "{text}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[cfg(windows)]
fn log_lines_name_no_users_folder() {
    let dir = scratch("profile");
    diagnostics::init(&dir).expect("init");
    // This machine's own folders, so no path is written into the test.
    let local = std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA");
    let home = std::env::var("USERPROFILE").expect("USERPROFILE");
    featherplay_controller::log!("cache {local}\\featherplay, music {home}\\Music");
    featherplay_controller::log!("lower {}", home.to_lowercase().replace('\\', "/"));
    featherplay_controller::log!(r"another C:\Users\fp-other-user\Downloads");
    log::warn!(target: "featherplay_streams", "no deno in {local}\\featherplay\\sidecars");
    diagnostics::flush();

    let text = read_log(&dir);
    let user = home.rsplit(['\\', '/']).next().expect("a user folder");
    assert!(
        !text.to_lowercase().contains(&user.to_lowercase()),
        "the user's folder is in\n{text}"
    );
    assert!(!text.contains("fp-other-user"), "{text}");
    assert!(
        text.contains(r"cache %LOCALAPPDATA%\featherplay, music %USERPROFILE%\Music"),
        "{text}"
    );
    assert!(
        text.contains(r"no deno in %LOCALAPPDATA%\featherplay\sidecars"),
        "{text}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_panic_message_names_no_users_folder() {
    let dir = scratch("crash-profile");
    diagnostics::init(&dir).expect("init");
    diagnostics::install_panic_hook(dir.clone());
    let joined = std::thread::Builder::new()
        .name("forced-panic".into())
        .spawn(|| panic!(r"can't open C:\Users\fp-other-user\AppData\Local\x"))
        .expect("spawn")
        .join();
    assert!(joined.is_err(), "the thread didn't panic");
    diagnostics::flush();

    let crash = std::fs::read_dir(&dir)
        .expect("log dir")
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-"))
        })
        .expect("a crash file");
    let report = std::fs::read_to_string(crash).unwrap();
    assert!(!report.contains("fp-other-user"), "{report}");
    assert!(
        report.contains(r"can't open %USERPROFILE%\AppData\Local\x"),
        "{report}"
    );
    let log = read_log(&dir);
    assert!(!log.contains("fp-other-user"), "{log}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_failed_resolve_logs_the_exit_code_and_the_stderr_tail() {
    let dir = scratch("resolve");
    diagnostics::init(&dir).expect("init");
    // What the streams crate logs for the step, then what the controller logs for the resolve.
    log::warn!(
        target: "featherplay_streams::ytdlp",
        "resolve h46YpWrg55Q [default] failed: yt-dlp exited with code 1: ERROR: [youtube] h46YpWrg55Q: Sign in to confirm you're not a bot"
    );
    diagnostics::resolve_failed(
        "h46YpWrg55Q",
        4.2,
        "yt-dlp failed (1): ERROR: [youtube] h46YpWrg55Q: Sign in to confirm you're not a bot",
    );
    diagnostics::flush();
    let text = read_log(&dir);
    assert!(
        text.contains("[default] failed: yt-dlp exited with code 1: ERROR: [youtube]"),
        "{text}"
    );
    assert!(
        text.contains("resolve h46YpWrg55Q failed in 4.2s: yt-dlp failed (1): ERROR: [youtube] h46YpWrg55Q: Sign in"),
        "{text}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_panic_writes_a_crash_file_and_a_log_line() {
    let dir = scratch("crash");
    diagnostics::init(&dir).expect("init");
    diagnostics::install_panic_hook(dir.clone());
    let joined = std::thread::Builder::new()
        .name("forced-panic".into())
        .spawn(|| panic!("forced for the crash file test"))
        .expect("spawn")
        .join();
    assert!(joined.is_err(), "the thread didn't panic");
    diagnostics::flush();

    let crashes: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("log dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    assert_eq!(crashes.len(), 1, "{crashes:?}");
    let report = std::fs::read_to_string(&crashes[0]).unwrap();
    assert!(
        report.contains("forced for the crash file test"),
        "{report}"
    );
    assert!(report.contains("forced-panic"), "no thread name:\n{report}");
    assert!(report.contains("diagnostics.rs"), "no location:\n{report}");
    assert!(report.contains("Backtrace:"), "{report}");
    let name = crashes[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let log = read_log(&dir);
    assert!(log.contains(&name), "the log doesn't name {name}:\n{log}");
    std::fs::remove_dir_all(&dir).unwrap();
}
