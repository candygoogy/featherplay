use super::*;
use std::collections::HashMap;

#[derive(Default)]
struct FakePc {
    env: HashMap<&'static str, &'static str>,
    files: Vec<&'static str>,
    links: HashMap<&'static str, &'static str>,
    heads: HashMap<&'static str, &'static str>,
    texts: HashMap<&'static str, &'static str>,
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
}

fn pc(files: &[&'static str]) -> FakePc {
    FakePc {
        env: HashMap::from([("PATH", "/usr/local/bin:/usr/bin"), ("HOME", "/home/ada")]),
        files: files.to_vec(),
        ..FakePc::default()
    }
}

fn ids(pc: &FakePc) -> Vec<&'static str> {
    found(pc).iter().map(|f| f.browser.id).collect()
}

/// Where `browser` is found on `pc`, and the folder its profile goes in (`None`: the app's
/// cache).
fn place_of(browser: &Browser, pc: &FakePc) -> (Sandbox, Option<PathBuf>) {
    let found = browser.locate(pc).expect("found");
    let profiles = found.sandbox.profiles(pc);
    (found.sandbox, profiles)
}

#[test]
fn every_browser_has_a_place() {
    for browser in ALL {
        assert!(place(browser).is_some(), "{}", browser.id);
    }
}

#[test]
fn the_browsers_are_found_on_path_in_the_order_chrome_chromium_firefox_edge_brave_vivaldi() {
    let pc = pc(&[
        "/usr/bin/vivaldi-stable",
        "/usr/bin/brave-browser",
        "/usr/bin/microsoft-edge-stable",
        "/usr/bin/firefox-esr",
        "/usr/bin/chromium",
        "/usr/bin/google-chrome-stable",
    ]);
    assert_eq!(
        ids(&pc),
        ["chrome", "chromium", "firefox", "edge", "brave", "vivaldi"]
    );
    assert!(found(&pc).iter().all(|f| f.blocked.is_none()));
    assert_eq!(
        CHROME.find(&pc),
        Some("/usr/bin/google-chrome-stable".into())
    );
    assert_eq!(FIREFOX.find(&pc), Some("/usr/bin/firefox-esr".into()));
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
fn a_snap_gets_its_profile_in_its_own_common_folder() {
    // /snap/bin/chromium, a link to snap itself.
    let mut pc = pc(&["/snap/bin/chromium"]);
    pc.links.insert("/snap/bin/chromium", "/usr/bin/snap");
    assert_eq!(
        place_of(&CHROMIUM, &pc),
        (
            Sandbox::Snap("chromium".into()),
            Some("/home/ada/snap/chromium/common".into())
        )
    );
    // Ubuntu's chromium-browser package: a script that runs the Snap.
    let mut pc = self::pc(&["/usr/bin/chromium-browser"]);
    pc.heads.insert(
        "/usr/bin/chromium-browser",
        "#!/bin/sh\n# transitional package\nexec /snap/bin/chromium \"$@\"\n",
    );
    assert_eq!(place_of(&CHROMIUM, &pc).0, Sandbox::Snap("chromium".into()));
    // Firefox's Snap, and a second install of a Snap (`{name}_{key}`).
    let mut pc = self::pc(&["/snap/bin/firefox"]);
    pc.links.insert("/snap/bin/firefox", "/usr/bin/snap");
    assert_eq!(place_of(&FIREFOX, &pc).0, Sandbox::Snap("firefox".into()));
    assert_eq!(snap_name(b"brave_beta.brave"), Some("brave_beta".into()));
    assert_eq!(snap_name(b"chromium \"$@\"\n"), Some("chromium".into()));
    assert_eq!(snap_name(b"\n"), None);
}

#[test]
fn a_flatpak_gets_its_profile_in_its_own_cache_folder() {
    let pc = pc(&["/home/ada/.local/share/flatpak/exports/bin/org.mozilla.firefox"]);
    assert_eq!(
        place_of(&FIREFOX, &pc),
        (
            Sandbox::Flatpak("org.mozilla.firefox"),
            Some("/home/ada/.var/app/org.mozilla.firefox/cache".into())
        )
    );
    let mut pc = self::pc(&["/usr/local/bin/brave"]);
    pc.heads.insert(
        "/usr/local/bin/brave",
        "#!/bin/sh\nexec flatpak run com.brave.Browser \"$@\"\n",
    );
    assert_eq!(
        place_of(&BRAVE, &pc).0,
        Sandbox::Flatpak("com.brave.Browser")
    );
    // No home: nowhere for its profile.
    let mut pc = self::pc(&["/var/lib/flatpak/exports/bin/org.chromium.Chromium"]);
    pc.env.remove("HOME");
    assert_eq!(place_of(&CHROMIUM, &pc).1, None);
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
    assert_eq!(
        place_of(&CHROMIUM, &pc),
        (Sandbox::None, None),
        "its profile goes in the app's cache"
    );
    assert_eq!(CHROMIUM.find(&pc), Some("/usr/bin/chromium-browser".into()));
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
    // Firefox, once it's installed.
    pc.texts.insert(
        "/home/ada/.config/mimeapps.list",
        "[Default Applications]\nx-scheme-handler/https=firefox.desktop\n",
    );
    assert_eq!(pc.https_handler(), None, "Firefox isn't installed");
    pc.files.push("/usr/bin/firefox");
    assert_eq!(pc.https_handler(), Some("/usr/bin/firefox".into()));
    // A browser the sign-in doesn't know: none.
    pc.texts.insert(
        "/home/ada/.config/mimeapps.list",
        "[Default Applications]\nx-scheme-handler/https=librewolf.desktop\n",
    );
    assert_eq!(pc.https_handler(), None);
}

/// A new, empty folder for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "featherplay-test-browsers-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A process that runs a while and does nothing, with `profile` in its command line (as a
/// browser started on it has), in a process group of its own.
fn started_on(profile: &Path) -> Running {
    // The shell stays (it doesn't hand over to `sleep`, which would drop `profile`), and
    // ends on SIGINT, whichever shell /bin/sh is.
    let script = "trap 'exit 130' INT; sleep 60 & wait";
    let args = ["-c".into(), script.into(), profile.into()];
    launch_plain(Path::new("/bin/sh"), &args, None).unwrap()
}

fn link(profile: &Path, name: &str, target: &str) {
    std::os::unix::fs::symlink(target, profile.join(name)).unwrap();
}

#[test]
fn a_lock_counts_while_a_process_started_on_the_profile_holds_it() {
    let dir = scratch("lock");
    let chromium = ProfileLock::new(&dir, Kind::Chromium);
    let firefox = ProfileLock::new(&dir, Kind::Firefox);
    assert_eq!(chromium.holder(), None, "no lock");

    let browser = started_on(&dir);
    link(&dir, "SingletonLock", &format!("ada-pc-{}", browser.pid));
    link(&dir, "lock", &format!("127.0.1.1:+{}", browser.pid));
    assert_eq!(chromium.holder(), Some(browser.pid));
    assert_eq!(firefox.holder(), Some(browser.pid));

    // A live process that wasn't started on this profile (this test's own): stale.
    let other = scratch("lock-other");
    link(
        &other,
        "SingletonLock",
        &format!("ada-pc-{}", std::process::id()),
    );
    assert_eq!(ProfileLock::new(&other, Kind::Chromium).holder(), None);

    // Its holder gone: stale too. So is one that names no pid.
    drop(browser);
    assert_eq!(chromium.holder(), None);
    assert_eq!(firefox.holder(), None);
    std::fs::remove_file(other.join("SingletonLock")).unwrap();
    link(&other, "SingletonLock", "ada-pc");
    assert_eq!(ProfileLock::new(&other, Kind::Chromium).holder(), None);
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&other).unwrap();
}

#[test]
fn a_browser_whose_launcher_exited_runs_until_its_lock_is_let_go() {
    let dir = scratch("launcher");
    // The browser itself, started by a launcher that has exited since (this test is the
    // launcher here: `holder` stands in for the browser it handed over to).
    let holder = started_on(&dir);
    link(&dir, "lock", &format!("127.0.1.1:+{}", holder.pid));
    let launcher = launch_plain(
        Path::new("/bin/true"),
        &[],
        Some(ProfileLock::new(&dir, Kind::Firefox)),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(!launcher.exited(), "the lock is still held");
    // Asked to quit, the holder gets the signal (sh dies on SIGINT).
    launcher.quit();
    assert!(
        launcher.wait(Duration::from_secs(5)),
        "it let go once asked"
    );
    assert!(holder.exited());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn killing_the_browser_ends_its_group_and_the_lock_holder() {
    let dir = scratch("kill");
    let holder = started_on(&dir);
    link(&dir, "SingletonLock", &format!("ada-pc-{}", holder.pid));
    // A shell that starts a helper of its own, as a browser does.
    let browser = launch_plain(
        Path::new("/bin/sh"),
        &["-c".into(), "sleep 60 & wait".into()],
        Some(ProfileLock::new(&dir, Kind::Chromium)),
    )
    .unwrap();
    let pgid = libc::pid_t::try_from(browser.pid).unwrap();
    // SAFETY: getpgid only reads the process group of our own child.
    assert_eq!(unsafe { libc::getpgid(pgid) }, pgid, "a group of its own");
    browser.kill();
    let gone = (0..250).any(|_| {
        std::thread::sleep(Duration::from_millis(20));
        // SAFETY: signal 0 to the group only asks whether any of it is left.
        let group = unsafe { libc::killpg(pgid, 0) } == 0;
        !group && holder.exited()
    });
    assert!(gone, "the group or the holder outlived the kill");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_exe_is_an_error() {
    assert!(launch_plain(Path::new("/definitely/not/here/chrome"), &[], None).is_err());
}

#[test]
fn quit_asks_the_process_to_exit() {
    let child = launch_plain(Path::new("/bin/sleep"), &["60".into()], None).unwrap();
    child.quit();
    assert!(child.wait(Duration::from_secs(5)), "sleep exits on SIGINT");
    // Once it's gone, quitting again does nothing.
    child.quit();
}
