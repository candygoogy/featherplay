use super::*;

/// Every system's pins: Windows x64 and ARM64, Linux x86_64 and aarch64.
const ALL: [&Pins; 4] = [&X64, &ARM64, &LINUX_X64, &LINUX_ARM64];

#[test]
fn downloads_stay_on_their_hosts_over_https() {
    let ok = |u: &str| download_host(&reqwest::Url::parse(u).unwrap());
    for pins in ALL {
        assert!(ok(pins.ytdlp_url) && ok(pins.deno_url));
        assert!(pins.ytdlp_mirror.is_none_or(ok) && ok(pins.deno_cdn_url));
    }
    assert!(!ok(
        "http://featherplay.candygoogy.dev/helpers/yt-dlp/x/yt-dlp.exe"
    ));
    assert!(!ok("http://dl.deno.land/release/v2.9.8/deno.zip"));
    assert!(!ok("https://dl.deno.land.example.net/"));
    assert!(!ok("https://evil.featherplay.candygoogy.dev/"));
    assert!(!ok("https://candygoogy.dev/"));
    assert!(ok(
        "https://release-assets.githubusercontent.com/github-production-release-asset/1"
    ));
    assert!(!ok(
        "http://github.com/yt-dlp/yt-dlp/releases/download/x/yt-dlp.exe"
    ));
    assert!(!ok("https://github.com.example.net/"));
    assert!(!ok("https://githubusercontent.com.example.net/"));
    assert!(!ok("https://example.net/yt-dlp.exe"));
}

#[test]
fn versions_are_read_from_either_tool() {
    assert_eq!(
        version_line("2026.10.26\r\n").as_deref(),
        Some("2026.10.26")
    );
    assert_eq!(
        version_line(
            "deno 2.9.8 (stable, release, x86_64-pc-windows-msvc)\nv8 14.1\ntypescript 5.9\n"
        )
        .as_deref(),
        Some("2.9.8")
    );
    assert_eq!(version_line("\n  \n"), None);
}

#[test]
fn the_pins_are_sha256_hex() {
    for pins in ALL {
        for h in [pins.ytdlp_sha256, pins.deno_sha256, pins.deno_cdn_sha256] {
            assert_eq!(h.len(), 64);
            assert!(
                h.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
        }
        assert!(!pins.ytdlp_url.contains("/latest/") && !pins.deno_url.contains("/latest/"));
        assert!(pins.deno_url.ends_with(&format!("/{}", pins.deno_zip)));
    }
}

/// Each system and architecture downloads its own build, and all pin the same releases, so
/// the weekly pin check (sidecar-pins.yml) reads one version for each helper. Linux's are
/// the glibc builds, named without `.exe`, with no yt-dlp mirror (over the site's 25 MiB a
/// file).
#[test]
fn the_pins_are_the_same_releases_of_each_systems_own_builds() {
    let release = |url: &'static str| url.rsplit_once('/').map(|(dir, _)| dir);
    let cases: [(&str, &Pins, &str, &str, bool); 4] = [
        (
            "Windows x64",
            &X64,
            "/yt-dlp.exe",
            "x86_64-pc-windows-msvc.zip",
            true,
        ),
        (
            "Windows ARM64",
            &ARM64,
            "/yt-dlp_arm64.exe",
            "aarch64-pc-windows-msvc.zip",
            true,
        ),
        (
            "Linux x86_64",
            &LINUX_X64,
            "/yt-dlp_linux",
            "x86_64-unknown-linux-gnu.zip",
            false,
        ),
        (
            "Linux aarch64",
            &LINUX_ARM64,
            "/yt-dlp_linux_aarch64",
            "aarch64-unknown-linux-gnu.zip",
            false,
        ),
    ];
    for (label, pins, ytdlp, deno, mirror) in cases {
        assert_eq!(release(pins.ytdlp_url), release(X64.ytdlp_url), "{label}");
        assert_eq!(release(pins.deno_url), release(X64.deno_url), "{label}");
        assert!(pins.ytdlp_url.ends_with(ytdlp), "{label}");
        assert!(pins.deno_zip.ends_with(deno), "{label}");
        assert_eq!(pins.ytdlp_mirror.is_some(), mirror, "{label}");
    }
    // Each its own build: no two share a hash.
    let distinct = |hash: fn(&Pins) -> &'static str| {
        ALL.iter()
            .map(|p| hash(p))
            .collect::<std::collections::HashSet<_>>()
            .len()
    };
    assert_eq!(distinct(|p| p.ytdlp_sha256), ALL.len(), "yt-dlp");
    assert_eq!(distinct(|p| p.deno_sha256), ALL.len(), "deno");
    assert_eq!(
        distinct(|p| p.deno_cdn_sha256),
        ALL.len(),
        "deno's second source"
    );
}

/// The second sources hold the same releases, under the same file names: the site's
/// `helpers/yt-dlp/<version>/<file>`, and Deno's `release/v<version>/<zip>`.
#[test]
fn the_second_sources_hold_the_pinned_releases() {
    assert!(X64.ytdlp_mirror.is_some() && ARM64.ytdlp_mirror.is_some());
    for pins in ALL {
        let (_, ytdlp) = pins.ytdlp_url.split_once("/releases/download/").unwrap();
        if let Some(mirror) = pins.ytdlp_mirror {
            assert_eq!(
                mirror,
                format!("https://featherplay.candygoogy.dev/helpers/yt-dlp/{ytdlp}")
            );
        }
        let (_, deno) = pins.deno_url.split_once("/releases/download/").unwrap();
        assert_eq!(
            pins.deno_cdn_url,
            format!("https://dl.deno.land/release/{deno}")
        );
        assert_ne!(pins.deno_cdn_sha256, pins.deno_sha256, "not GitHub's bytes");
    }
}

/// GitHub first, then the second source, each with its own pin.
#[cfg(all(windows, target_arch = "aarch64"))]
#[test]
fn an_arm64_build_downloads_the_arm64_helpers() {
    let urls = |a: &Asset| a.sources.iter().map(|s| s.url.clone()).collect::<Vec<_>>();
    let d = Downloads::pinned();
    assert_eq!(
        urls(&d.ytdlp),
        [ARM64.ytdlp_url, ARM64.ytdlp_mirror.unwrap()]
    );
    assert_eq!(urls(&d.deno), [ARM64.deno_url, ARM64.deno_cdn_url]);
    assert_eq!(d.deno.sources[1].sha256, ARM64.deno_cdn_sha256);
}

/// GitHub first, then the second source, each with its own pin.
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn an_x64_build_downloads_the_x64_helpers() {
    let urls = |a: &Asset| a.sources.iter().map(|s| s.url.clone()).collect::<Vec<_>>();
    let d = Downloads::pinned();
    assert_eq!(urls(&d.ytdlp), [X64.ytdlp_url, X64.ytdlp_mirror.unwrap()]);
    assert_eq!(urls(&d.deno), [X64.deno_url, X64.deno_cdn_url]);
    assert_eq!(d.deno.sources[1].sha256, X64.deno_cdn_sha256);
    assert_eq!(d.ytdlp.sources[1].sha256, X64.ytdlp_sha256, "the same file");
}

/// A Linux build downloads its own system's and architecture's helpers: yt-dlp from GitHub
/// only, Deno from GitHub then dl.deno.land. Their files have no `.exe`.
#[cfg(target_os = "linux")]
#[test]
fn a_linux_build_downloads_the_linux_helpers() {
    let pins = if cfg!(target_arch = "aarch64") {
        &LINUX_ARM64
    } else {
        &LINUX_X64
    };
    let urls = |a: &Asset| a.sources.iter().map(|s| s.url.clone()).collect::<Vec<_>>();
    let d = Downloads::pinned();
    assert_eq!(urls(&d.ytdlp), [pins.ytdlp_url]);
    assert_eq!(urls(&d.deno), [pins.deno_url, pins.deno_cdn_url]);
    assert_eq!(d.deno.sources[1].sha256, pins.deno_cdn_sha256);
    let sidecars = Sidecars { dir: "s".into() };
    assert_eq!(sidecars.ytdlp(), Path::new("s/yt-dlp"));
    assert_eq!(sidecars.deno(), Path::new("s/deno"));
}

/// An installed helper is executable, whatever mode its download was written with.
#[cfg(unix)]
#[tokio::test]
async fn an_installed_helper_is_executable() {
    use std::os::unix::fs::PermissionsExt;
    let Some(dir) = crate::test_dir("mode") else {
        return;
    };
    let (part, exe) = (dir.join("yt-dlp.part"), dir.join("yt-dlp"));
    std::fs::write(&part, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o600)).unwrap();

    put_in_place(&part, &exe).await.unwrap();

    let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755, "{mode:o}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Downloads both pinned releases from GitHub (~60 MB, ~85 MB on Linux) into a scratch dir,
/// checks them against their pins, and runs their `--version`.
#[tokio::test]
#[ignore = "live: downloads yt-dlp and deno from GitHub"]
async fn live_the_pinned_releases_install() {
    let base =
        std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME to a D: dir for tests");
    let dir = PathBuf::from(base)
        .join("test-streams")
        .join(format!("ensure-{}", std::process::id()));
    let sidecars = Sidecars { dir: dir.clone() };
    sidecars.ensure(|_, _, _| {}).await.unwrap();
    assert!(sidecars.ytdlp().is_file() && sidecars.deno().is_file());
    let (ytdlp, deno) = sidecars.versions().await;
    // The versions the pins name: `/download/<version>/` and `/download/v<version>/`.
    let pinned = |url: &str| {
        let (_, rest) = url
            .split_once("/releases/download/")
            .expect("a release URL");
        rest.split('/')
            .next()
            .map(|v| v.trim_start_matches('v').to_string())
    };
    assert_eq!(ytdlp, pinned(PINS.ytdlp_url));
    assert_eq!(deno, pinned(PINS.deno_url));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The update stamp of 2026-11-03 stayed that old through the daily updates after it
/// (writing an empty file over an empty one leaves its time), so every start ran
/// `yt-dlp -U` (~10 s, beside the first resolves).
#[tokio::test]
async fn an_old_update_stamp_is_made_recent() {
    let Some(dir) = crate::test_dir("stamp") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    let stamp = dir.join(UPDATE_STAMP);
    std::fs::write(&stamp, b"").unwrap();
    let day_old = SystemTime::now() - Duration::from_secs(25 * 3600);
    std::fs::File::options()
        .write(true)
        .open(&stamp)
        .unwrap()
        .set_modified(day_old)
        .unwrap();
    assert!(!is_recent(&stamp));
    touch(&stamp).await;
    assert!(is_recent(&stamp));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// What a file step does while antivirus scans the new file: "in use" (os error 32)
/// `held` times, then it goes through. Counts its calls.
fn held_step(
    held: usize,
    code: i32,
) -> (
    impl FnMut() -> std::future::Ready<std::io::Result<u8>>,
    std::sync::Arc<AtomicU64>,
) {
    let calls = std::sync::Arc::new(AtomicU64::new(0));
    let counted = calls.clone();
    let step = move || {
        let n = counted.fetch_add(1, Ordering::SeqCst);
        std::future::ready(if (n as usize) < held {
            Err(std::io::Error::from_raw_os_error(code))
        } else {
            Ok(7)
        })
    };
    (step, calls)
}

/// A file step waits while antivirus holds the file, fails after five tries if it holds it
/// too long, and fails at once on other errors.
#[tokio::test]
async fn a_file_step_waits_while_antivirus_holds_the_file() {
    /// A label, how often the file is held, the os error, the answer (the step's, or the
    /// error's code) and the calls.
    type Case = (&'static str, usize, i32, Result<u8, i32>, u64);
    let cases: [Case; 3] = [
        ("held twice, then through", 2, 32, Ok(7), 3),
        ("held too long", usize::MAX, 5, Err(5), 5),
        ("disk full: at once", usize::MAX, 112, Err(112), 1),
    ];
    for (label, held, code, want, tries) in cases {
        let (step, calls) = held_step(held, code);
        let got = patiently(step)
            .await
            .map_err(|e| e.raw_os_error().unwrap_or(-1));
        assert_eq!(got, want, "{label}");
        assert_eq!(calls.load(Ordering::SeqCst), tries, "{label}");
    }
}

/// A connection that drops mid-file is tried again, like one that couldn't connect.
#[tokio::test]
async fn a_download_cut_off_mid_file_is_tried_again() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for (n, stream) in listener.incoming().enumerate() {
            let mut s = stream.unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") && s.read(&mut byte).unwrap_or(0) == 1 {
                request.push(byte[0]);
            }
            // The first answer promises 100 bytes and stops after 7.
            let answer: &[u8] = if n == 0 {
                b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial"
            } else {
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello"
            };
            let _ = s.write_all(answer);
        }
    });
    let Some(dir) = crate::test_dir("cut") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("yt-dlp.exe.part");
    let from = Downloads::local(&[&base], "", "");
    let http = from.client().unwrap();

    let url = format!("{base}/yt-dlp.exe");
    let got = download_patiently(&http, &from, &url, &dest, "yt-dlp", &|_, _, _| {}).await;

    assert_eq!(got, Ok(()));
    assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// What a failed file step is, from the system's error.
#[test]
fn a_file_error_says_what_kept_the_helpers_out() {
    let fault = |os: i32| {
        let e = io_err(
            HelperFault::Blocked,
            "installing yt-dlp.exe",
            std::io::Error::from_raw_os_error(os),
        );
        match e {
            ResolveError::Sidecar(e) => (e.fault, e.os_error),
            e => panic!("{e:?}"),
        }
    };
    // Still held after the retries.
    assert_eq!(fault(32), (HelperFault::Blocked, Some(32)));
    assert_eq!(fault(5), (HelperFault::Blocked, Some(5)));
    // ERROR_DISK_FULL, ERROR_HANDLE_DISK_FULL.
    assert_eq!(fault(112), (HelperFault::NoSpace, Some(112)));
    assert_eq!(fault(39), (HelperFault::NoSpace, Some(39)));
    let damaged = io_err(
        HelperFault::Blocked,
        "extracting deno.exe",
        std::io::Error::new(std::io::ErrorKind::InvalidData, "the deno zip: bad"),
    );
    assert!(matches!(damaged, ResolveError::Sidecar(e) if e.fault == HelperFault::Damaged));
}

/// Copy diagnostics' line: what and where, never a path.
#[test]
fn a_helper_error_is_summed_up_without_paths() {
    let e = SidecarError {
        fault: HelperFault::Unreachable,
        message: r"C:\Users\ada\x: boom".into(),
        host: Some("github.com".into()),
        status: Some(503),
        os_error: None,
    };
    assert_eq!(e.summary(), "unreachable, github.com, HTTP 503");
    let e = SidecarError {
        fault: HelperFault::Blocked,
        host: None,
        status: None,
        os_error: Some(32),
        ..e
    };
    assert_eq!(e.summary(), "blocked, os error 32");
}

/// The real waits: about six seconds for a held file, as long as antivirus usually
/// holds one, and two more tries for a download.
#[test]
fn the_waits_are_few_and_short() {
    let total: Duration = HELD_WAITS_LIVE.iter().sum();
    assert!(
        total >= Duration::from_secs(5) && total <= Duration::from_secs(7),
        "{total:?}"
    );
    assert_eq!(HELD_WAITS_LIVE.len(), 4, "five tries");
    assert_eq!(DOWNLOAD_WAITS_LIVE.len(), 2, "three tries");
    assert!(DOWNLOAD_WAITS_LIVE.iter().sum::<Duration>() <= Duration::from_secs(10));
}

#[tokio::test]
async fn sha256_mismatch_rejected() {
    let Some(dir) = crate::test_dir("sha") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    let part = dir.join("yt-dlp.exe.part");
    std::fs::write(&part, b"corrupted download").unwrap();
    // sha256 of "hello", not of the file.
    let expected = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    let err = verify_or_delete(&part, expected).await.unwrap_err();
    assert!(
        matches!(&err, ResolveError::Sidecar(e)
            if e.message.contains("sha256") && e.fault == HelperFault::Damaged),
        "got {err:?}"
    );
    assert!(!part.exists(), "the corrupted download was not deleted");
    // The matching hash passes and keeps the file.
    std::fs::write(&part, b"hello").unwrap();
    verify_or_delete(&part, &expected.to_uppercase())
        .await
        .unwrap();
    assert!(part.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
