//! The update check, offline: the version comparison, GitHub's replies (hand-written
//! fixtures in `tests/fixtures/github/`), the stamp (a day between checks, an hour after a
//! failed one, the download's retries, the update that didn't install), the rate limit and
//! the cooldown, and what the automatic update picks: the setup (or the AppImage) for this OS
//! and architecture, its line in `SHA256SUMS.txt`, the minisign signature on that file (made
//! with test keys, `tests/fixtures/signing/`), and whether this is the installed copy, or
//! Scoop's. No request is made.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use featherplay_controller::updates::install::{
    self, Keys, Ready, SignatureError, is_installed, is_scoop, parse_sums, setup_name, update_name,
    verify_sums,
};
use featherplay_controller::updates::{
    Asset, CheckError, Release, Stamp, classify, is_newer, may_request, parse_issue, parse_release,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

const RELEASE: &str = include_str!("../fixtures/github/release_latest.json");
const ISSUES: &str = include_str!("../fixtures/github/issue_search.json");
const SECOND: Duration = Duration::from_secs(1);
const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    pairs
        .iter()
        .map(|(k, v)| {
            (
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            )
        })
        .collect()
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
fn an_issue_anyone_else_opens_with_the_title_gives_nothing() {
    // The notice reaches every user whose songs fail: only the maintainer's issue counts.
    for association in ["NONE", "CONTRIBUTOR", "FIRST_TIME_CONTRIBUTOR"] {
        let json = ISSUES.replace("\"OWNER\"", &format!("\"{association}\""));
        assert_eq!(parse_issue(&json), None, "{association}");
    }
    let unknown = ISSUES.replace("\"author_association\": \"OWNER\",", "");
    assert_eq!(parse_issue(&unknown), None);
}

#[test]
fn a_title_that_only_holds_the_words_gives_nothing() {
    // GitHub's `in:title` search matches the words anywhere in a title.
    for title in [
        "Playback broken on my laptop",
        "playback broken",
        "Daily play check failed",
    ] {
        let json = ISSUES.replace("\"Playback broken\"", &format!("\"{title}\""));
        assert_eq!(parse_issue(&json), None, "{title}");
    }
}

#[test]
fn the_maintainers_issue_is_found_behind_a_users() {
    let v: serde_json::Value = serde_json::from_str(ISSUES).unwrap();
    let mut user = v["items"][0].clone();
    user["author_association"] = "NONE".into();
    user["html_url"] = "https://github.com/candygoogy/featherplay/issues/41".into();
    let reply = serde_json::json!({ "total_count": 2, "items": [user, v["items"][0]] });
    assert_eq!(
        parse_issue(&reply.to_string()).as_deref(),
        Some("https://github.com/candygoogy/featherplay/issues/42")
    );
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
fn a_failed_check_is_due_again_after_an_hour() {
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    s.record(Err(CheckError::Offline), now() + 25 * HOUR);
    assert_eq!(s.latest, Some(release("0.2.0")), "the last answer stays");
    assert_eq!(
        s.answered,
        unix(now()),
        "\"Checked … ago\" is the last answer's"
    );
    assert!(!s.due(now() + 25 * HOUR + 59 * MINUTE));
    assert!(s.due(now() + 26 * HOUR), "an hour later, not the next day");
    // Answered again: a day.
    s.record(Ok(Some(release("0.2.0"))), now() + 26 * HOUR);
    assert!(!s.due(now() + 27 * HOUR));
}

#[test]
fn a_403_with_no_remaining_requests_is_limited_until_reset() {
    let reset = unix(now()) + 1200;
    let headers = headers(&[
        ("x-ratelimit-remaining", "0"),
        ("x-ratelimit-reset", &reset.to_string()),
    ]);
    assert_eq!(
        classify(403, &headers, now()),
        CheckError::Limited { until: reset }
    );
    // Another 403 (a blocked network, a policy) is a plain HTTP error.
    let left = self::headers(&[("x-ratelimit-remaining", "41")]);
    assert_eq!(classify(403, &left, now()), CheckError::Http(403));
    assert_eq!(
        classify(502, &HeaderMap::new(), now()),
        CheckError::Http(502)
    );
    // A reset far off is taken as an hour at most.
    let far = self::headers(&[
        ("x-ratelimit-remaining", "0"),
        ("x-ratelimit-reset", "99999999999"),
    ]);
    assert_eq!(
        classify(403, &far, now()),
        CheckError::Limited {
            until: unix(now()) + 3600
        }
    );
}

#[test]
fn a_429_with_retry_after_is_limited() {
    let headers = headers(&[("retry-after", "90")]);
    assert_eq!(
        classify(429, &headers, now()),
        CheckError::Limited {
            until: unix(now()) + 90
        }
    );
    // The stamp holds the check back until then, whatever asks.
    let mut s = Stamp::default();
    s.record(Err(classify(429, &headers, now())), now());
    assert!(!may_request(
        s.checked,
        now() + 80 * SECOND,
        s.limited_until()
    ));
    assert!(may_request(
        s.checked,
        now() + 90 * SECOND,
        s.limited_until()
    ));
}

#[test]
fn a_click_within_a_minute_makes_no_request() {
    let last = unix(now());
    assert!(may_request(0, now(), None), "never asked");
    assert!(!may_request(last, now() + 59 * SECOND, None));
    assert!(may_request(last, now() + MINUTE, None));
    assert!(may_request(last, now() - HOUR, None), "the clock went back");
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

/// `SHA256SUMS.txt` for 9.0.0, and its signatures: by the test main key, the test backup
/// key, a key featherplay doesn't know, and the main key for 8.0.0.
const SIGNED_SUMS: &[u8] = include_bytes!("../fixtures/signing/SHA256SUMS.txt");
const BY_MAIN: &str = include_str!("../fixtures/signing/main.minisig");
const BY_BACKUP: &str = include_str!("../fixtures/signing/backup.minisig");
const BY_UNKNOWN: &str = include_str!("../fixtures/signing/unknown.minisig");
const FOR_8: &str = include_str!("../fixtures/signing/old-version.minisig");

fn test_keys() -> Keys {
    Keys::from_pub(&[
        include_str!("../fixtures/signing/test-main.pub"),
        include_str!("../fixtures/signing/test-backup.pub"),
    ])
}

#[test]
fn checksums_signed_with_the_main_key_are_trusted() {
    assert_eq!(
        verify_sums(SIGNED_SUMS, BY_MAIN, "9.0.0", &test_keys()),
        Ok(())
    );
}

#[test]
fn checksums_signed_with_the_backup_key_are_trusted() {
    assert_eq!(
        verify_sums(SIGNED_SUMS, BY_BACKUP, "9.0.0", &test_keys()),
        Ok(())
    );
}

#[test]
fn a_damaged_signature_is_refused() {
    // One character of the signature itself, past the algorithm and the key id.
    let mut lines: Vec<String> = BY_MAIN.lines().map(str::to_string).collect();
    let mut sig: Vec<char> = lines[1].chars().collect();
    sig[40] = if sig[40] == 'A' { 'B' } else { 'A' };
    lines[1] = sig.into_iter().collect();
    let damaged = lines.join("\n");
    assert_eq!(
        verify_sums(SIGNED_SUMS, &damaged, "9.0.0", &test_keys()),
        Err(SignatureError::Invalid)
    );
    assert_eq!(
        verify_sums(SIGNED_SUMS, "not a signature", "9.0.0", &test_keys()),
        Err(SignatureError::Unreadable)
    );
}

#[test]
fn a_signature_from_an_unknown_key_is_refused() {
    assert_eq!(
        verify_sums(SIGNED_SUMS, BY_UNKNOWN, "9.0.0", &test_keys()),
        Err(SignatureError::UnknownKey)
    );
}

#[test]
fn a_signature_for_another_version_is_refused() {
    // 8.0.0's checksums, validly signed, uploaded again as 9.0.0's.
    assert_eq!(
        verify_sums(SIGNED_SUMS, FOR_8, "9.0.0", &test_keys()),
        Err(SignatureError::WrongVersion("featherplay v8.0.0".into()))
    );
    assert_eq!(
        verify_sums(SIGNED_SUMS, FOR_8, "8.0.0", &test_keys()),
        Ok(())
    );
}

#[test]
fn a_trusted_comment_edited_for_another_version_is_refused() {
    // The comment is signed as well: rewriting 8.0.0's to say 9.0.0 breaks the signature.
    let forged = FOR_8.replace("featherplay v8.0.0", "featherplay v9.0.0");
    assert_eq!(
        verify_sums(SIGNED_SUMS, &forged, "9.0.0", &test_keys()),
        Err(SignatureError::Invalid)
    );
}

#[test]
fn checksums_changed_after_signing_are_refused() {
    let changed = String::from_utf8(SIGNED_SUMS.to_vec())
        .unwrap()
        .replacen('d', "e", 1);
    assert_eq!(
        verify_sums(changed.as_bytes(), BY_MAIN, "9.0.0", &test_keys()),
        Err(SignatureError::Invalid)
    );
}

#[test]
fn the_test_keys_sign_nothing_for_the_release_keys() {
    // The built-in keys are featherplay's own two, and neither is a test key.
    let release = Keys::release();
    assert_eq!(release.len(), 2, "the main key and the backup key");
    assert_eq!(
        verify_sums(SIGNED_SUMS, BY_MAIN, "9.0.0", &release),
        Err(SignatureError::UnknownKey)
    );
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
    let plan = install::plan(&x64_only, "windows", "x86_64").expect("x64 has its setup");
    assert_eq!(plan.setup.name, "featherplay-setup-0.2.0.exe");
    assert_eq!(plan.sums.name, "SHA256SUMS.txt");
    assert_eq!(install::plan(&x64_only, "windows", "aarch64"), None);

    let both = with_assets(&[
        "featherplay-setup-0.2.0.exe",
        "featherplay-setup-0.2.0-arm64.exe",
        "SHA256SUMS.txt",
    ]);
    let arm = install::plan(&both, "windows", "aarch64").expect("arm64 has its setup");
    assert_eq!(arm.setup.name, "featherplay-setup-0.2.0-arm64.exe");
}

#[test]
fn linux_takes_the_appimage_for_its_architecture() {
    assert_eq!(
        update_name("linux", "0.2.0", "x86_64").as_deref(),
        Some("featherplay-x86_64.AppImage")
    );
    assert_eq!(
        update_name("linux", "0.2.0", "aarch64").as_deref(),
        Some("featherplay-aarch64.AppImage")
    );
    assert_eq!(update_name("linux", "0.2.0", "riscv64"), None);
    assert_eq!(update_name("linux", "0.2.0/..", "x86_64"), None);
    assert_eq!(
        update_name("windows", "0.2.0", "x86_64").as_deref(),
        Some("featherplay-setup-0.2.0.exe")
    );
    assert_eq!(update_name("macos", "0.2.0", "aarch64"), None);

    let r = with_assets(&[
        "featherplay-setup-0.2.0.exe",
        "featherplay-x86_64.AppImage",
        "featherplay-x86_64.tar.gz",
        "SHA256SUMS.txt",
    ]);
    let plan = install::plan(&r, "linux", "x86_64").expect("x86_64 has its AppImage");
    assert_eq!(plan.setup.name, "featherplay-x86_64.AppImage");
    assert_eq!(install::plan(&r, "linux", "aarch64"), None);

    let both = with_assets(&[
        "featherplay-x86_64.AppImage",
        "featherplay-aarch64.AppImage",
        "featherplay-aarch64.tar.gz",
        "SHA256SUMS.txt",
    ]);
    let arm = install::plan(&both, "linux", "aarch64").expect("aarch64 has its AppImage");
    assert_eq!(arm.setup.name, "featherplay-aarch64.AppImage");
    let x64 = install::plan(&both, "linux", "x86_64").expect("x86_64 has its AppImage");
    assert_eq!(x64.setup.name, "featherplay-x86_64.AppImage");
}

#[test]
fn a_release_without_checksums_is_only_announced() {
    let r = with_assets(&["featherplay-setup-0.2.0.exe"]);
    assert_eq!(install::plan(&r, "windows", "x86_64"), None);
}

#[test]
fn the_plan_carries_the_checksums_signature_if_the_release_has_one() {
    let signed = with_assets(&[
        "featherplay-setup-0.2.0.exe",
        "SHA256SUMS.txt",
        "SHA256SUMS.txt.minisig",
    ]);
    let plan = install::plan(&signed, "windows", "x86_64").expect("a plan");
    assert_eq!(
        plan.sig.map(|a| a.name).as_deref(),
        Some("SHA256SUMS.txt.minisig")
    );
    // Unsigned: still a plan, which the download then refuses (`fetch`).
    let unsigned = with_assets(&["featherplay-setup-0.2.0.exe", "SHA256SUMS.txt"]);
    assert_eq!(
        install::plan(&unsigned, "windows", "x86_64")
            .expect("a plan")
            .sig,
        None
    );
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "Windows only: install folders compare without case"
)]
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

#[test]
fn scoops_copy_is_told_apart() {
    assert!(is_scoop(Path::new(
        r"C:\Users\Ann\scoop\apps\featherplay\current\featherplay.exe"
    )));
    assert!(is_scoop(Path::new(
        r"D:\Scoop\apps\featherplay\0.1.2\featherplay.exe"
    )));
    assert!(!is_scoop(Path::new(
        r"C:\Users\Ann\AppData\Local\Programs\featherplay\featherplay.exe"
    )));
    assert!(!is_scoop(Path::new(
        r"C:\Users\Ann\scoop\apps\featherplay-fork\current\featherplay.exe"
    )));
}

fn ready(version: &str) -> Ready {
    Ready {
        version: version.into(),
        file: format!("featherplay-setup-{version}.exe"),
        sha256: "ab".repeat(32),
        signed: true,
    }
}

#[test]
fn a_failed_download_is_retried_after_an_hour_at_most_three_times() {
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    assert_eq!(s.to_fetch("0.1.0", now(), false), Some(&release("0.2.0")));
    let mut at = now();
    for attempt in 1..=3 {
        s.fetch_tried("0.2.0", at, false);
        assert_eq!(
            s.to_fetch("0.1.0", at + 59 * MINUTE, false),
            None,
            "not within the hour"
        );
        at += HOUR;
        let again = s.to_fetch("0.1.0", at, false).is_some();
        assert_eq!(again, attempt < 3, "attempt {attempt}");
    }
    // A click tries again all the same; the next release starts over.
    assert!(s.to_fetch("0.1.0", at, true).is_some());
    s.record(Ok(Some(release("0.3.0"))), at);
    assert_eq!(s.to_fetch("0.1.0", at, false), Some(&release("0.3.0")));
}

#[test]
fn a_checksum_mismatch_is_not_retried() {
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    s.fetch_tried("0.2.0", now(), true);
    assert_eq!(s.to_fetch("0.1.0", now() + 2 * HOUR, false), None);
    assert_eq!(
        s.to_fetch("0.1.0", now() + 2 * HOUR, true),
        None,
        "nor on a click"
    );
    assert_eq!(s.fetch_due_at("0.1.0"), None, "nothing to wake for");
}

#[test]
fn a_release_not_newer_than_the_running_one_is_never_downloaded() {
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    s.ready = Some(ready("0.2.0"));
    for running in ["0.2.0", "0.3.0"] {
        assert_eq!(s.to_fetch(running, now(), true), None, "{running}");
        assert_eq!(s.pending(running), None, "{running}");
    }
}

#[test]
fn an_unverified_update_is_checked_again_a_day_later() {
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    s.fetch_unverified("0.2.0", now());
    let fetch = s.fetch.clone().expect("tried");
    assert!(fetch.refused && fetch.unverified);
    assert_eq!(
        s.to_fetch("0.1.0", now() + 2 * HOUR, true),
        None,
        "nor on a click"
    );
    // A signature uploaded late is found at the next day's check, and checked as ever.
    assert_eq!(
        s.to_fetch("0.1.0", now() + 25 * HOUR, false),
        Some(&release("0.2.0"))
    );
}

#[test]
fn a_setup_downloaded_without_a_signature_check_is_not_offered() {
    // 0.1.2 and before checked only the sha256: their downloaded setup isn't installed.
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    s.ready = Some(Ready {
        signed: false,
        ..ready("0.2.0")
    });
    assert_eq!(s.pending("0.1.0"), None);
    assert!(
        s.to_fetch("0.1.0", now(), false).is_some(),
        "downloaded again"
    );
}

#[test]
fn an_older_running_version_after_launch_is_a_failed_install() {
    let mut s = Stamp::default();
    s.record(Ok(Some(release("0.2.0"))), now());
    s.launched = Some("0.2.0".into());
    assert_eq!(s.failed_install("0.1.0"), Some("0.2.0"), "0.1.0 still runs");
    assert_eq!(s.failed_install("0.2.0"), None, "it installed");
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

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::var("FEATHERPLAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"))
        .join("test-updates")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn the_downloaded_setup_survives_a_restart() {
    let dir = test_dir("ready");
    let path = dir.join("update-check.json");
    let mut s = Stamp::default();
    s.record(Ok(Some(with_assets(&["SHA256SUMS.txt"]))), now());
    s.fetch_tried("0.2.0", now(), false);
    s.ready = Some(ready("0.2.0"));
    s.launched = Some("0.2.0".into());
    s.save(&path).expect("saved");
    assert_eq!(Stamp::load(&path), s);
    assert!(
        !dir.join("update-check.json.tmp").exists(),
        "renamed into place"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_old_stamp_with_fetched_still_loads() {
    let dir = test_dir("old-stamp");
    let path = dir.join("update-check.json");
    std::fs::create_dir_all(&dir).unwrap();
    // From before automatic updates.
    std::fs::write(
        &path,
        r#"{"checked":1,"latest":{"version":"0.2.0","url":"https://github.com/candygoogy/featherplay/releases/tag/v0.2.0"},"announced":null}"#,
    )
    .unwrap();
    let old = Stamp::load(&path);
    assert_eq!(old.latest, Some(release("0.2.0")));
    assert_eq!((old.fetch, old.ready), (None, None));
    assert_eq!(old.answered, 1, "its check answered");
    // From 0.1.0-0.1.2: the download was tried once, whatever came of it. It reads as tried
    // three times, so it isn't retried by itself; a click still can.
    std::fs::write(
        &path,
        r#"{"checked":1,"latest":{"version":"0.2.0","url":"https://github.com/candygoogy/featherplay/releases/tag/v0.2.0"},"announced":"0.2.0","fetched":"0.2.0"}"#,
    )
    .unwrap();
    let old = Stamp::load(&path);
    let fetch = old.fetch.clone().expect("migrated");
    assert_eq!((fetch.version.as_str(), fetch.attempts), ("0.2.0", 3));
    assert_eq!(old.to_fetch("0.1.0", now(), false), None);
    assert!(old.to_fetch("0.1.0", now(), true).is_some());
    std::fs::remove_dir_all(&dir).unwrap();
}
