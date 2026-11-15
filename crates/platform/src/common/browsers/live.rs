//! The sign-in's two starts of a real browser, end to end, on this OS.

use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::browsers::{HEADLESS, ThisPc, found, launch, launch_plain};

/// A web server on 127.0.0.1 whose one page sets two test cookies named like Google's
/// session cookies (for 127.0.0.1 only, with test values, kept for an hour: Google's have an
/// expiry too), and a third, from the page's script, with what `navigator.webdriver` says.
/// Then the page closes its window, as the user closes the sign-in's. Serves until the test
/// ends.
fn cookie_page() -> String {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/signin", server.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in server.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let body = "<!doctype html><title>featherplay test page</title><p>Test cookies set.\
                        <script>document.cookie = 'webdriver=' + navigator.webdriver + \
                        '; max-age=3600'; setTimeout(() => window.close(), 1000)</script>";
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\
                 Set-Cookie: SAPISID=test-sapisid; Path=/; Max-Age=3600\r\n\
                 Set-Cookie: SID=test-sid; Path=/; Max-Age=3600; HttpOnly\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    url
}

/// The replies on the pipe, split at their NUL bytes and parsed, on a thread of their own.
fn replies(mut from: File) -> mpsc::Receiver<serde_json::Value> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut pending = Vec::new();
        let mut buf = [0u8; 64 * 1024];
        while let Ok(n @ 1..) = from.read(&mut buf) {
            pending.extend_from_slice(&buf[..n]);
            while let Some(end) = pending.iter().position(|&b| b == 0) {
                let message: Vec<u8> = pending.drain(..=end).collect();
                if let Ok(v) = serde_json::from_slice(&message[..end]) {
                    let _ = tx.send(v);
                }
            }
        }
    });
    rx
}

/// Sends one of the app's two commands and waits up to `within` for its reply.
fn ask(
    to: &mut File,
    replies: &mpsc::Receiver<serde_json::Value>,
    id: u64,
    method: &str,
    within: Duration,
) -> Option<serde_json::Value> {
    let command = serde_json::json!({ "id": id, "method": method });
    to.write_all(format!("{command}\0").as_bytes()).ok()?;
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let reply = replies.recv_timeout(left).ok()?;
        if reply["id"] == id {
            return Some(reply);
        }
    }
}

/// A real browser, end to end, as the sign-in starts it twice, with no Google page and no
/// account: a browser found here (or the Chromium-based one in `FEATHERPLAY_PIPE_TEST_BROWSER`)
/// starts plainly, with no pipe, on a temporary profile under `FEATHERPLAY_HOME`, on a local
/// page that sets test cookies, says what `navigator.webdriver` is and closes the window. Once
/// that browser has exited, it starts headless on the same profile with the pipe; the app's two
/// commands go over it (`Storage.getCookies`, then `Browser.close`) and the cookies the first
/// start kept come back; it exits, and the profile is deleted. Its window shows for a moment
/// (from WSL, on the Windows desktop).
/// `FEATHERPLAY_HOME=<a test home> cargo nextest run -p featherplay-platform --lib --run-ignored only
/// -E 'test(live_browser)' --no-capture`
#[test]
#[ignore = "starts a browser found here (local page, temporary profile); needs a display"]
#[allow(clippy::print_stderr, reason = "the live check reports what it saw")]
fn live_browser_signs_in_plainly_then_is_read_headless() {
    let (browser, exe) = match std::env::var_os("FEATHERPLAY_PIPE_TEST_BROWSER") {
        #[cfg(windows)]
        Some(exe) => (&crate::browsers::EDGE, PathBuf::from(exe)),
        #[cfg(target_os = "linux")]
        Some(exe) => (&crate::browsers::CHROMIUM, PathBuf::from(exe)),
        None => {
            let usable = found(&ThisPc).into_iter().find(|f| f.blocked.is_none());
            let f = usable.expect("no known browser here that the sign-in can use");
            (f.browser, f.exe)
        }
    };
    eprintln!("starting {}: {}", browser.name, exe.display());
    let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
    let profile = PathBuf::from(home).join(format!("signin-test-{}", std::process::id()));
    std::fs::create_dir_all(&profile).unwrap();
    let mut dir = OsString::from("--user-data-dir=");
    dir.push(&profile);
    let switches = browser.switches.iter().map(OsString::from);

    // The sign-in window: no pipe, nothing about automation.
    let mut args = vec![dir.clone()];
    args.extend(switches.clone());
    args.push(cookie_page().into());
    let window = launch_plain(&exe, &args).unwrap();
    let closed = window.wait(Duration::from_secs(60));
    eprintln!("the plain start closed its window and exited: {closed}");
    drop(window);
    assert!(
        closed,
        "{} didn't exit when its window closed",
        browser.name
    );

    // The read: headless, on a blank page, with the pipe.
    let mut args = vec![dir, "--remote-debugging-pipe".into(), HEADLESS.into()];
    args.extend(switches);
    args.push("about:blank".into());
    let (running, pipes) = launch(&exe, &args).unwrap();
    let replies = replies(pipes.from_browser);
    let mut to = pipes.to_browser;
    let reply = ask(
        &mut to,
        &replies,
        1,
        "Storage.getCookies",
        Duration::from_secs(60),
    )
    .expect("no reply on the pipe");
    let cookies = reply["result"]["cookies"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut seen: Vec<String> = cookies
        .iter()
        .filter(|c| c["domain"] == "127.0.0.1")
        .filter_map(|c| Some(format!("{}={}", c["name"].as_str()?, c["value"].as_str()?)))
        .collect();
    seen.sort();
    eprintln!("cookies the headless start read: {seen:?}");
    // The page didn't see an automated browser, as Google's sign-in won't.
    assert_eq!(
        seen,
        ["SAPISID=test-sapisid", "SID=test-sid", "webdriver=false"]
    );

    let close = ask(
        &mut to,
        &replies,
        2,
        "Browser.close",
        Duration::from_secs(30),
    );
    assert!(close.is_some());
    let exited = running.wait(Duration::from_secs(10));
    eprintln!("exited within 10 s of Browser.close: {exited}");
    drop(running);
    assert!(exited, "{} didn't exit on Browser.close", browser.name);
    // Its helpers may hold files for a moment.
    let removed = (0..40).any(|_| {
        std::fs::remove_dir_all(&profile).is_ok() || {
            std::thread::sleep(Duration::from_millis(250));
            false
        }
    });
    assert!(
        removed && !profile.exists(),
        "the temporary profile is still there"
    );
}
