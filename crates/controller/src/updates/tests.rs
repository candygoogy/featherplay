use super::install::tests::{BY_MAIN, SETUP, SIGNED_SUMS, serve, test_cache};
use super::*;

/// `releases/latest` for 9.0.0, its files on the project's releases, as GitHub gives;
/// `signed`: with `SHA256SUMS.txt.minisig`.
fn reply(signed: bool) -> Vec<u8> {
    let file = |name: &str| {
        format!(
            r#"{{"name":"{name}","browser_download_url":"{}v9.0.0/{name}"}}"#,
            install::DOWNLOADS
        )
    };
    // The setup for the machine the tests run on (x64 or ARM64, as the updater picks it).
    let setup = install::setup_name("9.0.0", std::env::consts::ARCH).expect("x64 or ARM64");
    let mut files = vec![file(&setup), file("SHA256SUMS.txt")];
    if signed {
        files.push(file("SHA256SUMS.txt.minisig"));
    }
    format!(
        r#"{{"tag_name":"v9.0.0","html_url":"{PAGES}releases/tag/v9.0.0","assets":[{}]}}"#,
        files.join(",")
    )
    .into_bytes()
}

/// GitHub's reply (`signed` or not) and 9.0.0's files, signed with the test main key.
fn github(signed: bool) -> String {
    let setup = format!(
        "/{}",
        install::setup_name("9.0.0", std::env::consts::ARCH).expect("x64 or ARM64")
    );
    serve(move |path| match path {
        "/latest" => (200, reply(signed)),
        "/SHA256SUMS.txt" => (200, SIGNED_SUMS.to_vec()),
        "/SHA256SUMS.txt.minisig" => (200, BY_MAIN.to_vec()),
        p if p == setup => (200, SETUP.to_vec()),
        _ => (404, Vec::new()),
    })
}

#[tokio::test]
async fn a_manual_check_on_the_installed_copy_downloads_the_setup() {
    let cache = test_cache("manual-run");
    let base = github(true);

    // "Check for updates" off: the click asks all the same.
    let running = Running::begin(true).expect("no run going");
    let remote = Remote::local(&base, true);
    let found = run(running, cache.clone(), Trigger::Manual, false, true, remote).await;

    assert_eq!(
        found.newer.as_ref().map(|r| r.version.as_str()),
        Some("9.0.0")
    );
    let ready = found.ready.expect("the setup downloaded and checked");
    assert!(ready.verified(&cache));
    assert_eq!(found.announce, None, "a click is answered in About");
    let stamp = Stamp::load(&stamp_path(&cache));
    assert_eq!(stamp.ready, Some(ready));
    assert_eq!(
        stamp.announced.as_deref(),
        Some("9.0.0"),
        "nor toasted later"
    );
    assert!(!status().running);
    std::fs::remove_dir_all(&cache).unwrap();
}

#[tokio::test]
async fn an_unsigned_release_is_refused_for_good_and_said() {
    let cache = test_cache("unsigned-run");
    let base = github(false);

    let running = Running::begin(true).expect("no run going");
    let remote = Remote::local(&base, true);
    let found = run(running, cache.clone(), Trigger::Startup, true, true, remote).await;

    assert_eq!(found.ready, None, "never installed unsigned");
    assert!(found.unverified);
    assert!(!cache.join("updates").exists(), "nothing downloaded");
    let stamp = Stamp::load(&stamp_path(&cache));
    assert!(stamp.unverified(CURRENT));
    assert_eq!(stamp.to_fetch(CURRENT, SystemTime::now(), true), None);
    let about = Status {
        stamp,
        kind: Kind::Installed,
        ..Status::default()
    };
    assert_eq!(about.banner(true), Banner::Unverified);
    assert_eq!(
        Notice::of(&found, Trigger::Startup),
        Some(Notice::Unverified("9.0.0".into()))
    );
    std::fs::remove_dir_all(&cache).unwrap();
}

#[test]
fn a_second_run_while_one_is_running_does_nothing() {
    let first = Running::begin(true).expect("no run going");
    assert!(Running::begin(true).is_none(), "one run at a time");
    assert!(status().checking);
    drop(first);
    assert!(!status().checking);
    assert!(Running::begin(true).is_some(), "free again once it ended");
}
