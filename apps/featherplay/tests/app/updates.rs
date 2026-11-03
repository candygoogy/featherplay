//! The update check, offline: the version comparison, GitHub's replies (hand-written
//! fixtures in `tests/fixtures/github/`), the once-a-day stamp, and what the automatic
//! update picks: the setup for this architecture, its line in `SHA256SUMS.txt`, and whether
//! this is the installed copy. No request is made.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use std::path::Path;

use featherplay::updates::install::{self, Ready, is_installed, parse_sums, setup_name};
use featherplay::updates::{Asset, Release, Stamp, is_newer, parse_issue, parse_release};

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
        assets: Vec::new(),
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
    let r = parse_release(RELEASE).expect("a release");
    let want = release("0.2.0");
    assert_eq!((r.version, r.url), (want.version, want.url));
}

#[test]
fn the_reply_keeps_only_the_files_downloaded_from_the_projects_releases() {
    let r = parse_release(RELEASE).expect("a release");
    let names: Vec<_> = r.assets.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "featherplay.exe",
            "featherplay-setup-0.2.0.exe",
            "SHA256SUMS.txt"
        ]
    );
    assert!(
        r.assets
            .iter()
            .all(|a| a.url.starts_with(install::DOWNLOADS))
    );
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

const SUMS: &str = "\
1111111111111111111111111111111111111111111111111111111111111111  featherplay.exe
AbCdEf0123456789abcdef0123456789ABCDEF0123456789abcdef0123456789  featherplay-setup-0.2.0.exe
2222222222222222222222222222222222222222222222222222222222222222 *THIRD_PARTY.md
";

#[test]
fn the_checksums_give_the_setups_sha256_in_lower_case() {
    assert_eq!(
        parse_sums(SUMS, "featherplay-setup-0.2.0.exe").as_deref(),
        Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
    );
    // Binary-mode lines (`*name`) and CRLF line ends read the same.
    assert_eq!(
        parse_sums(&SUMS.replace('\n', "\r\n"), "THIRD_PARTY.md").as_deref(),
        Some("2222222222222222222222222222222222222222222222222222222222222222")
    );
}

#[test]
fn a_file_the_checksums_dont_list_has_no_sha256() {
    assert_eq!(parse_sums(SUMS, "featherplay-setup-0.2.0-arm64.exe"), None);
    // Not a prefix match either.
    assert_eq!(parse_sums(SUMS, "featherplay-setup-0.2.0"), None);
    assert_eq!(parse_sums("", "featherplay.exe"), None);
}

#[test]
fn a_malformed_or_contradictory_checksum_is_refused() {
    assert_eq!(parse_sums("abc  featherplay.exe", "featherplay.exe"), None);
    let twice = format!("{SUMS}{}  featherplay.exe\n", "3".repeat(64));
    assert_eq!(parse_sums(&twice, "featherplay.exe"), None);
}

#[test]
fn the_setup_is_picked_by_architecture() {
    assert_eq!(
        setup_name("0.2.0", "x86_64").as_deref(),
        Some("featherplay-setup-0.2.0.exe")
    );
    assert_eq!(
        setup_name("0.2.0", "aarch64").as_deref(),
        Some("featherplay-setup-0.2.0-arm64.exe")
    );
    assert_eq!(setup_name("0.2.0", "x86"), None);
    assert_eq!(
        setup_name("../0.2.0", "x86_64"),
        None,
        "only digits and dots"
    );
}

fn with_assets(names: &[&str]) -> Release {
    let mut r = release("0.2.0");
    r.assets = names
        .iter()
        .map(|n| Asset {
            name: (*n).into(),
            url: format!("{}v0.2.0/{n}", install::DOWNLOADS),
        })
        .collect();
    r
}

#[test]
fn a_release_without_this_architectures_setup_is_only_announced() {
    let x64_only = with_assets(&["featherplay-setup-0.2.0.exe", "SHA256SUMS.txt"]);
    let plan = install::plan(&x64_only, "x86_64").expect("x64 has its setup");
    assert_eq!(plan.setup.name, "featherplay-setup-0.2.0.exe");
    assert_eq!(plan.sums.name, "SHA256SUMS.txt");
    assert_eq!(install::plan(&x64_only, "aarch64"), None);

    let both = with_assets(&[
        "featherplay-setup-0.2.0.exe",
        "featherplay-setup-0.2.0-arm64.exe",
        "SHA256SUMS.txt",
    ]);
    let arm = install::plan(&both, "aarch64").expect("arm64 has its setup");
    assert_eq!(arm.setup.name, "featherplay-setup-0.2.0-arm64.exe");
}

#[test]
fn a_release_without_checksums_is_only_announced() {
    let r = with_assets(&["featherplay-setup-0.2.0.exe"]);
    assert_eq!(install::plan(&r, "x86_64"), None);
}

#[test]
fn the_exe_in_the_install_folder_is_the_installed_copy() {
    let location = Path::new(r"C:\Users\Ann\AppData\Local\Programs\featherplay\");
    let exe = Path::new(r"C:\Users\Ann\AppData\Local\Programs\featherplay\featherplay.exe");
    assert!(is_installed(exe, location));
    // Windows paths compare without case, with either slash.
    let shouty = Path::new("c:/users/ann/appdata/local/programs/FEATHERPLAY/FeatherPlay.exe");
    assert!(is_installed(shouty, location));
}

#[test]
fn a_portable_or_dev_exe_is_not_the_installed_copy() {
    let location = Path::new(r"C:\Users\Ann\AppData\Local\Programs\featherplay\");
    for exe in [
        r"C:\Users\Ann\Downloads\featherplay.exe",
        r"D:\src\featherplay\target\release\featherplay.exe",
        r"C:\Users\Ann\AppData\Local\Programs\featherplay\old\featherplay.exe",
        r"C:\Users\Ann\AppData\Local\Programs\featherplay\other.exe",
    ] {
        assert!(!is_installed(Path::new(exe), location), "{exe}");
    }
    assert!(!is_installed(
        Path::new(r"C:\featherplay.exe"),
        Path::new("")
    ));
}

fn ready(version: &str) -> Ready {
    Ready {
        version: version.into(),
        file: format!("featherplay-setup-{version}.exe"),
        sha256: "ab".repeat(32),
    }
}

#[test]
fn each_newer_version_is_downloaded_once() {
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    assert_eq!(s.to_fetch("0.1.0"), Some(&release("0.2.0")));
    s.fetched = Some("0.2.0".into());
    assert_eq!(s.to_fetch("0.1.0"), None, "tried, whatever came of it");
    s.record(Ok(Some(release("0.3.0"))), now() + 25 * HOUR);
    assert_eq!(s.to_fetch("0.1.0"), Some(&release("0.3.0")));
}

#[test]
fn a_downloaded_setup_is_offered_until_it_is_installed() {
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    s.ready = Some(ready("0.2.0"));
    assert_eq!(s.pending("0.1.0"), Some(&ready("0.2.0")));
    // Running 0.2.0: it's installed; nothing to offer.
    assert_eq!(s.pending("0.2.0"), None);
    // A newer release than the one downloaded: that setup isn't offered.
    s.record(Ok(Some(release("0.3.0"))), now() + 25 * HOUR);
    assert_eq!(s.pending("0.1.0"), None);
}

#[test]
fn the_downloaded_setup_survives_a_restart() {
    let dir = std::env::var("FEATHERPLAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"))
        .join("test-updates")
        .join(format!("ready-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("update-check.json");
    let mut s = Stamp::default();
    s.record(Ok(Some(with_assets(&["SHA256SUMS.txt"]))), now());
    s.fetched = Some("0.2.0".into());
    s.ready = Some(ready("0.2.0"));
    s.save(&path).expect("saved");
    assert_eq!(Stamp::load(&path), s);
    // A stamp from before automatic updates still loads.
    std::fs::write(
        &path,
        r#"{"checked":1,"latest":{"version":"0.2.0","url":"https://github.com/candygoogy/featherplay/releases/tag/v0.2.0"},"announced":null}"#,
    )
    .unwrap();
    let old = Stamp::load(&path);
    assert_eq!(old.latest, Some(release("0.2.0")));
    assert_eq!((old.fetched, old.ready), (None, None));
    std::fs::remove_dir_all(&dir).unwrap();
}
