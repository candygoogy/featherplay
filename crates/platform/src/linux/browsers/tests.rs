use super::*;
use std::collections::HashMap;
use std::io::Write;

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

/// A process to start that runs a while and does nothing: `sleep`.
fn sleeper() -> (PathBuf, Vec<OsString>) {
    ("/bin/sleep".into(), vec!["60".into()])
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

#[test]
fn a_plain_launch_runs_in_its_own_group_and_kill_ends_it() {
    let (exe, args) = sleeper();
    let child = launch_plain(&exe, &args).unwrap();
    assert!(!child.exited(), "it should be running");
    let pid = libc::pid_t::try_from(child.pid).unwrap();
    // SAFETY: getpgid only reads the process group of our own child.
    assert_eq!(unsafe { libc::getpgid(pid) }, pid);
    child.kill();
    assert!(child.exited());
}

#[test]
fn quit_asks_the_process_to_exit() {
    let (exe, args) = sleeper();
    let child = launch_plain(&exe, &args).unwrap();
    child.quit();
    assert!(child.wait(Duration::from_secs(5)), "sleep exits on SIGINT");
    // Once it's gone, quitting again does nothing.
    child.quit();
}
