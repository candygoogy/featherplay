use super::*;
use std::collections::HashMap;
use std::io::Write;
use std::net::TcpListener;
use std::sync::mpsc;

#[derive(Default)]
struct FakePc {
    env: HashMap<&'static str, &'static str>,
    files: Vec<&'static str>,
    links: HashMap<&'static str, &'static str>,
    heads: HashMap<&'static str, &'static str>,
    texts: HashMap<&'static str, &'static str>,
    policies: HashMap<&'static str, Vec<&'static str>>,
}

impl Lookup for FakePc {
    fn env(&self, var: &str) -> Option<OsString> {
        self.env.get(var).map(OsString::from)
    }
    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|f| Path::new(f) == path)
    }
    fn resolve(&self, path: &Path) -> Option<PathBuf> {
        let p = path.to_str()?;
        Some(PathBuf::from(self.links.get(p).copied().unwrap_or(p)))
    }
    fn head(&self, path: &Path) -> Option<Vec<u8>> {
        let text = self.heads.get(path.to_str()?)?;
        Some(text.as_bytes().to_vec())
    }
    fn read(&self, path: &Path) -> Option<String> {
        self.texts.get(path.to_str()?).map(|t| t.to_string())
    }
    fn json_files(&self, dir: &Path) -> Vec<String> {
        let dir = dir.to_str().unwrap_or_default();
        self.policies
            .get(dir)
            .map(|v| v.iter().map(|t| t.to_string()).collect())
            .unwrap_or_default()
    }
}

fn pc(files: &[&'static str]) -> FakePc {
    FakePc {
        env: HashMap::from([("PATH", "/usr/local/bin:/usr/bin"), ("HOME", "/home/ada")]),
        files: files.to_vec(),
        ..FakePc::default()
    }
}

fn ids(pc: &FakePc) -> Vec<(&'static str, Option<Block>)> {
    found(pc)
        .iter()
        .map(|f| (f.browser.id, f.blocked))
        .collect()
}

#[test]
fn every_browser_has_a_place() {
    for browser in ALL {
        assert!(place(browser).is_some(), "{}", browser.id);
    }
}

#[test]
fn the_browsers_are_found_on_path_in_the_order_chrome_chromium_edge_brave_vivaldi() {
    let pc = pc(&[
        "/usr/bin/vivaldi-stable",
        "/usr/bin/brave-browser",
        "/usr/bin/microsoft-edge-stable",
        "/usr/bin/chromium",
        "/usr/bin/google-chrome-stable",
    ]);
    assert_eq!(
        ids(&pc),
        [
            ("chrome", None),
            ("chromium", None),
            ("edge", None),
            ("brave", None),
            ("vivaldi", None)
        ]
    );
    assert_eq!(
        CHROME.find(&pc),
        Some("/usr/bin/google-chrome-stable".into())
    );
    assert!(found(&self::pc(&[])).is_empty());
}

#[test]
fn path_comes_first_then_the_usual_folders_then_the_package_folder() {
    let mut pc = pc(&[
        "/opt/google/chrome/google-chrome",
        "/usr/bin/google-chrome",
        "/home/ada/bin/google-chrome",
    ]);
    pc.env.insert("PATH", "/home/ada/bin:relative/bin");
    assert_eq!(CHROME.find(&pc), Some("/home/ada/bin/google-chrome".into()));
    pc.env.remove("PATH");
    assert_eq!(CHROME.find(&pc), Some("/usr/bin/google-chrome".into()));
    let pc = self::pc(&["/opt/google/chrome/google-chrome"]);
    assert_eq!(
        CHROME.find(&pc),
        Some("/opt/google/chrome/google-chrome".into())
    );
}

#[test]
fn a_snap_is_found_but_cant_be_used() {
    // /snap/bin/chromium, a link to snap itself.
    let mut pc = pc(&["/snap/bin/chromium"]);
    pc.links.insert("/snap/bin/chromium", "/usr/bin/snap");
    assert_eq!(ids(&pc), [("chromium", Some(Block::Snap))]);
    // Ubuntu's chromium-browser package: a script that runs the Snap.
    let mut pc = self::pc(&["/usr/bin/chromium-browser"]);
    pc.heads.insert(
        "/usr/bin/chromium-browser",
        "#!/bin/sh\n# transitional package\nexec /snap/bin/chromium \"$@\"\n",
    );
    assert_eq!(ids(&pc), [("chromium", Some(Block::Snap))]);
}

#[test]
fn a_flatpak_is_found_but_cant_be_used() {
    let pc = pc(&["/home/ada/.local/share/flatpak/exports/bin/com.brave.Browser"]);
    assert_eq!(ids(&pc), [("brave", Some(Block::Flatpak))]);
    let mut pc = self::pc(&["/usr/local/bin/brave"]);
    pc.heads.insert(
        "/usr/local/bin/brave",
        "#!/bin/sh\nexec flatpak run com.brave.Browser \"$@\"\n",
    );
    assert_eq!(ids(&pc), [("brave", Some(Block::Flatpak))]);
}

#[test]
fn a_package_comes_before_a_snap_or_a_flatpak_of_the_same_browser() {
    let mut pc = pc(&[
        "/snap/bin/chromium",
        "/var/lib/flatpak/exports/bin/org.chromium.Chromium",
        "/usr/bin/chromium-browser",
    ]);
    pc.heads.insert(
        "/usr/bin/chromium-browser",
        "#!/bin/sh\nexec /usr/lib/chromium/chromium \"$@\"\n",
    );
    assert_eq!(CHROMIUM.find(&pc), Some("/usr/bin/chromium-browser".into()));
    assert_eq!(CHROMIUM.blocked(&pc), None);
}

#[test]
fn a_policy_file_turning_the_pipe_off_blocks_it() {
    let mut pc = pc(&["/usr/bin/google-chrome", "/usr/bin/chromium"]);
    pc.policies.insert(
        "/etc/opt/chrome/policies/managed",
        vec![
            "not json",
            r#"{"HomepageLocation": "https://example.com"}"#,
            r#"{"RemoteDebuggingAllowed": false}"#,
        ],
    );
    assert_eq!(
        ids(&pc),
        [("chrome", Some(Block::Policy)), ("chromium", None)]
    );
    // True, or a policy file that doesn't say, leaves it alone.
    pc.policies.insert(
        "/etc/opt/chrome/policies/managed",
        vec![r#"{"RemoteDebuggingAllowed": true}"#, "{}"],
    );
    assert_eq!(CHROME.blocked(&pc), None);
}

#[test]
fn the_default_browser_comes_from_mimeapps_list() {
    let mut pc = pc(&["/usr/bin/google-chrome", "/usr/bin/brave-browser"]);
    pc.env.insert("XDG_CURRENT_DESKTOP", "ubuntu:GNOME");
    pc.texts.insert(
        "/usr/share/applications/gnome-mimeapps.list",
        "[Default Applications]\nx-scheme-handler/https=google-chrome.desktop\n",
    );
    assert_eq!(pc.https_handler(), Some("/usr/bin/google-chrome".into()));
    // The user's own choice comes first; an entry in another section doesn't count.
    pc.texts.insert(
        "/home/ada/.config/mimeapps.list",
        "[Added Associations]\nx-scheme-handler/https=firefox.desktop;\n\
         [Default Applications]\ntext/html=firefox.desktop\n\
         x-scheme-handler/https = brave-browser.desktop;google-chrome.desktop;\n",
    );
    assert_eq!(pc.https_handler(), Some("/usr/bin/brave-browser".into()));
    let found = found(&pc);
    let offered = pick(&found, None, pc.https_handler().as_deref());
    assert_eq!(offered.map(|i| found[i].browser.id), Some("brave"));
    // A browser the sign-in doesn't know: none.
    pc.texts.insert(
        "/home/ada/.config/mimeapps.list",
        "[Default Applications]\nx-scheme-handler/https=firefox.desktop\n",
    );
    assert_eq!(pc.https_handler(), None);
}

#[test]
fn a_blocked_browser_gives_way_to_one_that_works() {
    let mut pc = pc(&["/snap/bin/chromium", "/usr/bin/microsoft-edge"]);
    pc.links.insert("/snap/bin/chromium", "/usr/bin/snap");
    let found = found(&pc);
    let offered = pick(&found, Some("chromium"), None).map(|i| found[i].browser.id);
    assert_eq!(offered, Some("edge"));
}

/// A process to start that runs a while and does nothing: `sleep`.
fn sleeper() -> (PathBuf, Vec<OsString>) {
    ("/bin/sleep".into(), vec!["60".into()])
}

#[test]
fn a_launched_process_runs_and_kill_ends_it() {
    let (exe, args) = sleeper();
    let (child, _pipes) = launch(&exe, &args).unwrap();
    assert!(!child.exited(), "it should be running");
    child.kill();
    assert!(child.exited());
}

#[test]
fn dropping_the_browser_ends_its_whole_process_group() {
    // A shell that starts a helper of its own, as a browser does, and says its pid.
    let (child, pipes) = launch(
        Path::new("/bin/sh"),
        &["-c".into(), "sleep 60 & echo $! >&4; wait".into()],
    )
    .unwrap();
    let mut said = String::new();
    let mut from = pipes.from_browser;
    while !said.ends_with('\n') {
        let mut b = [0u8; 16];
        let n = from.read(&mut b).unwrap();
        assert!(n > 0, "the shell closed the pipe");
        said.push_str(std::str::from_utf8(&b[..n]).unwrap());
    }
    let helper: libc::pid_t = said.trim().parse().unwrap();
    drop(child);
    let gone = (0..250).any(|_| {
        std::thread::sleep(Duration::from_millis(20));
        // SAFETY: signal 0 only asks whether the process exists.
        let alive = unsafe { libc::kill(helper, 0) } == 0;
        !alive
    });
    assert!(gone, "the helper outlived the group");
}

#[test]
fn the_browser_gets_the_pipe_on_3_and_4_and_nothing_else() {
    // A file the app has open while it starts the browser.
    let exe = std::env::current_exe().unwrap();
    let _open = File::open(&exe).unwrap();
    // A shell that echoes what it reads on 3 to 4, then lists its open descriptors and what
    // each one is.
    let script = "read line <&3; echo \"got $line\" >&4;                   for f in /proc/$$/fd/*; do echo \"${f##*/} $(readlink $f)\"; done >&4";
    let (child, pipes) = launch(Path::new("/bin/sh"), &["-c".into(), script.into()]).unwrap();
    let mut to = pipes.to_browser;
    to.write_all(b"hello\n").unwrap();
    drop(to);
    let mut out = String::new();
    let mut from = pipes.from_browser;
    from.read_to_string(&mut out).unwrap();
    assert!(child.wait(Duration::from_secs(5)));
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some("got hello"));
    let fds: HashMap<u32, &str> = lines
        .filter_map(|l| {
            let (fd, target) = l.split_once(' ')?;
            Some((fd.parse().ok()?, target))
        })
        .collect();
    assert!(
        fds[&3].starts_with("pipe:") && fds[&4].starts_with("pipe:"),
        "{fds:?}"
    );
    // The two pipes reach it as 3 and 4 only (and as the shell's stdout while it echoes, a
    // copy of 4): the app's own ends don't. What the test runner left open without
    // close-on-exec (a terminal, cargo's jobserver) may be there too: it isn't the app's.
    let ours: Vec<u32> = fds
        .iter()
        .filter(|(_, t)| [fds[&3], fds[&4]].contains(t))
        .map(|(fd, _)| *fd)
        .collect();
    assert!(ours.iter().all(|fd| [1, 3, 4].contains(fd)), "{fds:?}");
    let exe = exe.to_string_lossy();
    assert!(
        !fds.values().any(|t| *t == exe),
        "the app's open file reached it: {fds:?}"
    );
}

#[test]
fn a_missing_exe_is_an_error() {
    assert!(launch(Path::new("/definitely/not/here/chrome"), &[]).is_err());
}

/// What this PC really has: the known browsers found, the default browser, and the one
/// offered. Nothing is started.
/// `cargo nextest run -p featherplay-platform --lib --run-ignored only -E 'test(this_pc)' --no-capture`
#[test]
#[ignore = "reads this PC's browsers and reports what it finds"]
#[allow(clippy::print_stderr, reason = "the live check reports what it saw")]
fn this_pc_lists_its_browsers_and_its_default() {
    let found = found(&ThisPc);
    for f in &found {
        eprintln!(
            "found {}: {} (blocked: {:?})",
            f.browser.name,
            f.exe.display(),
            f.blocked
        );
        assert!(ThisPc.is_file(&f.exe));
    }
    let default = ThisPc.https_handler();
    eprintln!("https links open with: {default:?}");
    let offered = pick(&found, None, default.as_deref()).map(|i| found[i].browser.name);
    eprintln!("offered: {offered:?}");
}

/// A web server on 127.0.0.1 whose one page sets two test cookies named like Google's
/// session cookies (for 127.0.0.1 only, with test values). Serves until the test ends.
fn cookie_page() -> String {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/signin", server.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in server.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let body = "<!doctype html><title>featherplay test page</title><p>Test cookies set.";
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\
                 Set-Cookie: SAPISID=test-sapisid; Path=/\r\n\
                 Set-Cookie: SID=test-sid; Path=/; HttpOnly\r\n\
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

/// Sends one of the app's two commands and waits for its reply.
fn ask(
    to: &mut File,
    replies: &mpsc::Receiver<serde_json::Value>,
    id: u64,
    method: &str,
) -> Option<serde_json::Value> {
    let command = serde_json::json!({ "id": id, "method": method });
    to.write_all(format!("{command}\0").as_bytes()).ok()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let reply = replies.recv_timeout(left).ok()?;
        if reply["id"] == id {
            return Some(reply);
        }
    }
}

/// A real browser on the pipe, end to end, with no Google page and no account: a browser
/// found here (or the Chromium-based one in `FEATHERPLAY_PIPE_TEST_BROWSER`) starts with a
/// temporary profile under `FEATHERPLAY_HOME`, on a local page that sets test cookies; the
/// app's two commands go over the pipe (`Storage.getCookies` until the cookies show, then
/// `Browser.close`); the browser exits, and its profile is deleted. Its window shows for a
/// moment (in WSL, on the Windows desktop).
/// `FEATHERPLAY_HOME=<a test home> cargo nextest run -p featherplay-platform --lib --run-ignored only
/// -E 'test(live_browser)' --no-capture`
#[test]
#[ignore = "starts a browser found here (local page, temporary profile); needs a display"]
#[allow(clippy::print_stderr, reason = "the live check reports what it saw")]
fn live_browser_answers_on_the_pipe_with_a_local_pages_cookies() {
    let (browser, exe) = match std::env::var_os("FEATHERPLAY_PIPE_TEST_BROWSER") {
        Some(exe) => (&CHROMIUM, PathBuf::from(exe)),
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
    let mut args = vec![dir, "--remote-debugging-pipe".into()];
    args.extend(browser.switches.iter().map(OsString::from));
    args.push(cookie_page().into());
    let (running, pipes) = launch(&exe, &args).unwrap();
    let replies = replies(pipes.from_browser);
    let mut to = pipes.to_browser;

    let mut seen = Vec::new();
    for id in 1..=60 {
        let reply = ask(&mut to, &replies, id, "Storage.getCookies").expect("no reply on the pipe");
        let cookies = reply["result"]["cookies"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        seen = cookies
            .iter()
            .filter(|c| c["domain"] == "127.0.0.1")
            .filter_map(|c| Some(format!("{}={}", c["name"].as_str()?, c["value"].as_str()?)))
            .collect();
        seen.sort();
        if seen.len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    eprintln!("cookies for the local page: {seen:?}");
    assert_eq!(seen, ["SAPISID=test-sapisid", "SID=test-sid"]);

    assert!(ask(&mut to, &replies, 100, "Browser.close").is_some());
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
