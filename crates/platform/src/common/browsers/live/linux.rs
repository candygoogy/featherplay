//! Linux: the sign-in window, then the read of its profile from disk.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::browsers::{
    Browser, CHROMIUM, FIREFOX, Kind, Located, ProfileLock, ThisPc, cookies, launch_plain,
};

/// Every browser found here, in each of its places: the launcher the sign-in would use, and
/// each exe in `FEATHERPLAY_TEST_BROWSERS` (paths split by `:`, e.g. a Snap's or a Flatpak's,
/// taken for Firefox when their name says so and for Chromium otherwise).
fn browsers() -> Vec<(&'static Browser, Located)> {
    let mut out: Vec<_> = crate::browsers::ALL
        .into_iter()
        .filter_map(|b| Some((b, b.locate(&ThisPc)?)))
        .collect();
    if let Some(list) = std::env::var_os("FEATHERPLAY_TEST_BROWSERS") {
        for exe in std::env::split_paths(&list) {
            let firefox = exe.to_string_lossy().contains("firefox");
            let browser = if firefox { &FIREFOX } else { &CHROMIUM };
            let sandbox = browser.sandbox_of(&exe, &ThisPc);
            out.push((browser, Located { exe, sandbox }));
        }
    }
    out
}

/// Each browser found here (`browsers`), end to end, with no Google page and no account: it
/// starts plainly on a temporary profile (under `FEATHERPLAY_HOME`, or where its Snap or
/// Flatpak lets it reach one, named as the sign-in names it there), as the sign-in starts it,
/// on a local page that sets test cookies, says what `navigator.webdriver` is and closes the
/// window. Once the browser has exited and let go of its profile's lock, the cookies it kept
/// are read from the profile on disk, and the profile is deleted. Its window shows for a
/// moment (from WSL, on the Windows desktop).
/// `FEATHERPLAY_HOME=<a test home> cargo nextest run -p featherplay-platform --lib --run-ignored only
/// -E 'test(live_browser)' --no-capture`
#[test]
#[ignore = "starts every browser found here (local page, temporary profile); needs a display"]
#[allow(clippy::print_stderr, reason = "the live check reports what it saw")]
fn live_browser_signs_in_plainly_then_is_read_from_disk() {
    let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
    let all = browsers();
    assert!(!all.is_empty(), "no known browser here");
    for (browser, found) in all {
        let parent = found
            .sandbox
            .profiles(&ThisPc)
            .unwrap_or_else(|| PathBuf::from(&home));
        let profile = parent.join(format!("featherplay-signin-test-{}", std::process::id()));
        eprintln!(
            "starting {} ({:?}): {}, profile in {}",
            browser.name,
            found.sandbox,
            found.exe.display(),
            parent.display()
        );
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).unwrap();
        let mut args: Vec<OsString> = match browser.kind {
            Kind::Chromium => {
                let mut dir = OsString::from("--user-data-dir=");
                dir.push(&profile);
                vec![dir]
            }
            Kind::Firefox => {
                std::fs::write(profile.join("user.js"), browser.preferences.unwrap()).unwrap();
                vec!["--profile".into(), profile.clone().into()]
            }
        };
        args.extend(browser.switches.iter().map(OsString::from));
        args.push(super::cookie_page().into());
        let lock = ProfileLock::new(&profile, browser.kind);
        let window = launch_plain(&found.exe, &args, Some(lock)).unwrap();
        let closed = window.wait(Duration::from_secs(60));
        eprintln!(
            "{}: closed its window and let go of the profile: {closed}",
            browser.name
        );
        drop(window);
        assert!(
            closed,
            "{} didn't exit when its window closed",
            browser.name
        );

        let read = cookies::read(&profile, browser.kind);
        let removed = std::fs::remove_dir_all(&profile).is_ok() && !profile.exists();
        let mut seen: Vec<String> = read
            .unwrap_or_else(|e| panic!("{}: {e:?}", browser.name))
            .into_iter()
            .filter(|c| c.domain == "127.0.0.1")
            .map(|c| format!("{}={}", c.name, c.value))
            .collect();
        seen.sort();
        eprintln!("{}: cookies read from the profile: {seen:?}", browser.name);
        // The page didn't see an automated browser, as Google's sign-in won't.
        assert_eq!(
            seen,
            ["SAPISID=test-sapisid", "SID=test-sid", "webdriver=false"],
            "{}",
            browser.name
        );
        assert!(
            removed,
            "{}: the temporary profile is still there",
            browser.name
        );
    }
}
