//! The update check, offline: the version comparison, GitHub's replies (hand-written
//! fixtures in `tests/fixtures/github/`), and the once-a-day stamp. No request is made.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use featherplay::updates::{Release, Stamp, is_newer, parse_issue, parse_release};

const RELEASE: &str = include_str!("../fixtures/github/release_latest.json");
const ISSUES: &str = include_str!("../fixtures/github/issue_search.json");
const HOUR: Duration = Duration::from_secs(3600);

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

fn release(version: &str) -> Release {
    Release {
        version: version.into(),
        url: format!("https://github.com/candygoogy/featherplay/releases/tag/v{version}"),
    }
}

#[test]
fn a_newer_version_is_newer() {
    assert!(is_newer("v0.2.0", "0.1.0"));
    assert!(is_newer("0.1.1", "0.1.0"));
    assert!(is_newer("v1.0.0", "0.9.9"));
    // Numbers, not text: 10 comes after 9.
    assert!(is_newer("v0.10.0", "0.9.0"));
}

#[test]
fn the_same_version_is_not_newer() {
    assert!(!is_newer("v0.1.0", "0.1.0"));
    assert!(!is_newer("0.1.0", "0.1.0"));
}

#[test]
fn an_older_version_is_not_newer() {
    assert!(!is_newer("v0.0.9", "0.1.0"));
    assert!(!is_newer("v0.1.0", "1.0.0"));
}

#[test]
fn a_pre_release_is_ignored() {
    assert!(!is_newer("v0.2.0-beta.1", "0.1.0"));
    assert!(!is_newer("v9.0.0-rc1", "0.1.0"));
}

#[test]
fn a_release_is_newer_than_its_own_pre_release() {
    assert!(is_newer("v0.2.0", "0.2.0-dev"));
}

#[test]
fn a_malformed_version_is_never_newer() {
    for tag in [
        "", "v", "latest", "v1.2", "1.2.3.4", "v1.x.0", "vv1.0.0", " 1.0.0", "1..0",
    ] {
        assert!(!is_newer(tag, "0.1.0"), "{tag:?}");
    }
    assert!(!is_newer("v9.0.0", "not a version"));
}

#[test]
fn the_latest_release_reply_gives_the_version_and_its_page() {
    assert_eq!(parse_release(RELEASE), Some(release("0.2.0")));
}

#[test]
fn a_pre_release_or_draft_reply_gives_nothing() {
    let pre = RELEASE.replace("\"prerelease\": false", "\"prerelease\": true");
    assert_eq!(parse_release(&pre), None);
    let draft = RELEASE.replace("\"draft\": false", "\"draft\": true");
    assert_eq!(parse_release(&draft), None);
    let tagged = RELEASE.replace("\"tag_name\": \"v0.2.0\"", "\"tag_name\": \"v0.2.0-rc.1\"");
    assert_eq!(parse_release(&tagged), None);
}

#[test]
fn a_release_page_elsewhere_is_refused() {
    // It's opened in the browser: only the project's own pages on GitHub.
    for url in [
        "https://example.com/candygoogy/featherplay/releases/tag/v0.2.0",
        "http://github.com/candygoogy/featherplay/releases/tag/v0.2.0",
        "https://github.com/someone-else/featherplay/releases/tag/v0.2.0",
        "https://github.com/candygoogy/featherplay.evil/releases/tag/v0.2.0",
    ] {
        let json = RELEASE.replace(
            "https://github.com/candygoogy/featherplay/releases/tag/v0.2.0",
            url,
        );
        assert_eq!(parse_release(&json), None, "{url}");
    }
}

#[test]
fn a_reply_that_is_not_a_release_gives_nothing() {
    assert_eq!(parse_release(""), None);
    assert_eq!(parse_release("{\"message\": \"Not Found\"}"), None);
    assert_eq!(parse_release("[1, 2]"), None);
}

#[test]
fn the_issue_search_gives_the_open_issue() {
    assert_eq!(
        parse_issue(ISSUES).as_deref(),
        Some("https://github.com/candygoogy/featherplay/issues/42")
    );
}

#[test]
fn an_empty_issue_search_gives_nothing() {
    assert_eq!(
        parse_issue("{\"total_count\": 0, \"incomplete_results\": false, \"items\": []}"),
        None
    );
    assert_eq!(parse_issue("{\"message\": \"Validation Failed\"}"), None);
    let elsewhere = ISSUES.replace("https://github.com/candygoogy", "https://example.com/x");
    assert_eq!(parse_issue(&elsewhere), None);
}

#[test]
fn a_first_run_is_due_for_a_check() {
    assert!(Stamp::default().due(now()));
}

#[test]
fn a_check_lasts_a_day() {
    let mut s = Stamp::default();
    s.record(Ok(None), now());
    assert!(!s.due(now() + HOUR));
    assert!(!s.due(now() + 23 * HOUR));
    assert!(s.due(now() + 24 * HOUR));
}

#[test]
fn a_clock_set_back_makes_the_check_due() {
    let mut s = Stamp::default();
    s.record(Ok(None), now());
    assert!(s.due(now() - HOUR));
}

#[test]
fn a_failed_check_keeps_the_last_answer_and_waits_a_day() {
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    s.record(Err("HTTP 404".into()), now() + 25 * HOUR);
    assert_eq!(s.latest, Some(release("0.2.0")));
    assert!(!s.due(now() + 26 * HOUR), "no retry before the next day");
}

#[test]
fn a_newer_release_is_announced_once() {
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    assert_eq!(s.newer("0.1.0"), Some(&release("0.2.0")));
    assert_eq!(s.to_announce("0.1.0"), Some(release("0.2.0")));
    s.announced("0.2.0");
    assert_eq!(s.to_announce("0.1.0"), None);
    // The banner stays.
    assert_eq!(s.newer("0.1.0"), Some(&release("0.2.0")));
    // The next one is announced again.
    s.record(Ok(Some(release("0.3.0"))), now() + 25 * HOUR);
    assert_eq!(s.to_announce("0.1.0"), Some(release("0.3.0")));
}

#[test]
fn the_running_version_or_an_older_one_is_not_news() {
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.1.0"))), now());
    assert_eq!(s.newer("0.1.0"), None);
    assert_eq!(s.to_announce("0.1.0"), None);
    s.record(Ok(Some(release("0.0.9"))), now());
    assert_eq!(s.newer("0.1.0"), None);
}

#[test]
fn the_stamp_survives_a_restart() {
    let dir = std::env::var("FEATHERPLAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"))
        .join("test-updates")
        .join(format!("stamp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("update-check.json");
    assert_eq!(Stamp::load(&path), Stamp::default(), "none yet");

    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    s.announced("0.2.0");
    s.save(&path).expect("saved");
    assert_eq!(Stamp::load(&path), s);

    std::fs::write(&path, "not json").unwrap();
    assert_eq!(
        Stamp::load(&path),
        Stamp::default(),
        "a broken file starts over"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
