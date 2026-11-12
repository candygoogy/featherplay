use super::*;
use featherplay_platform::browsers::EDGE;
use std::io::{BufRead, BufReader, PipeReader, PipeWriter};
use std::sync::Mutex;

const NOW: f64 = 1_800_000_000.0;

fn cookie(name: &str, domain: &str) -> Cookie {
    Cookie {
        name: name.into(),
        value: format!("v-{name}"),
        domain: domain.into(),
        path: "/".into(),
        secure: true,
        expires: Some(NOW + 3600.0),
        partitioned: false,
    }
}

fn signed_in_cookies() -> Vec<Cookie> {
    vec![
        cookie("SID", ".google.com"),
        cookie("SAPISID", ".youtube.com"),
        cookie("SID", ".youtube.com"),
    ]
}

#[test]
fn the_session_counts_once_it_has_settled_for_3_s() {
    let t0 = Instant::now();
    let mut s = Settle::new(Duration::from_secs(3));
    assert_eq!(
        s.see(t0, &[cookie("PREF", ".youtube.com")], NOW),
        Seen::NotYet
    );
    let at = t0 + Duration::from_secs(1);
    assert_eq!(
        s.see(at, &signed_in_cookies(), NOW),
        Seen::Settling(Duration::from_secs(3))
    );
    assert_eq!(
        s.see(at + Duration::from_secs(1), &signed_in_cookies(), NOW),
        Seen::Settling(Duration::from_secs(2))
    );
    // The last read decides the header: music.youtube.com's own cookie is in it now.
    let mut later = signed_in_cookies();
    later.push(cookie("YSC", "music.youtube.com"));
    assert_eq!(
        s.see(at + Duration::from_secs(3), &later, NOW),
        Seen::Ready("SAPISID=v-SAPISID; SID=v-SID; YSC=v-YSC".into())
    );
}

#[test]
fn losing_the_session_while_settling_starts_over() {
    let t0 = Instant::now();
    let mut s = Settle::new(Duration::from_secs(3));
    assert!(matches!(
        s.see(t0, &signed_in_cookies(), NOW),
        Seen::Settling(_)
    ));
    assert_eq!(s.see(t0 + Duration::from_secs(2), &[], NOW), Seen::NotYet);
    assert_eq!(
        s.see(t0 + Duration::from_secs(4), &signed_in_cookies(), NOW),
        Seen::Settling(Duration::from_secs(3))
    );
}

#[test]
fn a_google_session_without_youtube_isnt_signed_in() {
    let mut s = Settle::new(Duration::ZERO);
    let google = [cookie("SAPISID", ".google.com")];
    assert_eq!(s.see(Instant::now(), &google, NOW), Seen::NotYet);
}

/// Edge installed, with its pipe turned off by the machine's policy.
#[cfg(windows)]
struct PolicyOffPc;

#[cfg(windows)]
impl browsers::Lookup for PolicyOffPc {
    fn app_path(&self, _: browsers::Hive, _: &str) -> Option<String> {
        Some(r"C:\Edge\msedge.exe".into())
    }
    fn env(&self, _: &str) -> Option<OsString> {
        None
    }
    fn is_file(&self, _: &Path) -> bool {
        true
    }
    fn dword(&self, hive: browsers::Hive, _: &str, value: &str) -> Option<u32> {
        (hive == browsers::Hive::LocalMachine && value == "RemoteDebuggingAllowed").then_some(0)
    }
    fn https_handler(&self) -> Option<PathBuf> {
        None
    }
}

/// Edge installed, with its pipe turned off by a policy file.
#[cfg(target_os = "linux")]
struct PolicyOffPc;

#[cfg(target_os = "linux")]
impl browsers::Lookup for PolicyOffPc {
    fn env(&self, _: &str) -> Option<OsString> {
        None
    }
    fn is_file(&self, path: &Path) -> bool {
        path == Path::new("/usr/bin/microsoft-edge-stable")
    }
    fn resolve(&self, path: &Path) -> Option<PathBuf> {
        Some(path.into())
    }
    fn head(&self, _: &Path) -> Option<Vec<u8>> {
        None
    }
    fn read(&self, _: &Path) -> Option<String> {
        None
    }
    fn json_files(&self, dir: &Path) -> Vec<String> {
        if dir == Path::new("/etc/opt/edge/policies/managed") {
            vec![r#"{"RemoteDebuggingAllowed": false}"#.into()]
        } else {
            Vec::new()
        }
    }
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "Windows only: the policy is read from the registry"
)]
fn with_the_policy_off_the_browser_isnt_started() {
    let home = std::env::var_os("FEATHERPLAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let cache = home.join(format!("test-policy-{}", std::process::id()));
    let out = run_on(&PolicyOffPc, &EDGE, &cache, &AtomicBool::new(false));
    assert_eq!(out, Outcome::Unusable(Block::Policy));
    // No profile was made, so nothing was launched.
    assert!(!cache.exists());
}

#[test]
fn the_arguments_are_the_profile_the_pipe_and_the_sign_in_page() {
    let a = args(
        &EDGE,
        Path::new(r"C:\cache\signin-1-ab"),
        session::SIGN_IN_URL,
    );
    let a: Vec<&str> = a.iter().map(|s| s.to_str().unwrap()).collect();
    let quiet: &[&str] = if cfg!(windows) {
        &["--no-first-run", "--no-default-browser-check"]
    } else {
        // Linux keeps the throwaway profile's cookies out of the desktop's keyring, and
        // `navigator.webdriver` false, without a bar about the switch.
        &[
            "--no-first-run",
            "--no-default-browser-check",
            "--password-store=basic",
            "--disable-blink-features=AutomationControlled",
            "--test-type",
        ]
    };
    let expected: Vec<&str> = [
        r"--user-data-dir=C:\cache\signin-1-ab",
        "--remote-debugging-pipe",
    ]
    .into_iter()
    .chain(quiet.iter().copied())
    .chain([session::SIGN_IN_URL])
    .collect();
    assert_eq!(a, expected);
    assert!(!a.iter().any(|s| s.contains("remote-debugging-port")));
}

/// The browser's process, as the tests drive it.
#[derive(Default)]
struct FakeProcess {
    exited: AtomicBool,
    killed: AtomicBool,
    /// `Browser.close` is answered but the process stays.
    ignores_close: AtomicBool,
}

impl Process for FakeProcess {
    fn exited(&self) -> bool {
        self.exited.load(Ordering::Relaxed)
    }
    fn wait(&self, limit: Duration) -> bool {
        let until = Instant::now() + limit;
        while Instant::now() < until && !self.exited() {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.exited()
    }
    fn kill(&self) {
        self.killed.store(true, Ordering::Relaxed);
        self.exited.store(true, Ordering::Relaxed);
    }
}

fn quick() -> Limits {
    Limits {
        poll: Duration::from_millis(10),
        settle: Duration::from_millis(30),
        first_reply: Duration::from_millis(1000),
        reply: Duration::from_millis(300),
        timeout: Duration::from_secs(5),
        close: Duration::from_secs(2),
    }
}

/// What the fake browser does with each `Storage.getCookies`, by its number (from 1).
enum Answer {
    Cookies(Vec<Cookie>),
    Silence,
    /// No cookies, after this long.
    Late(Duration),
    Hang,
    ExitProcess,
    ClosePipe,
}

/// A fake browser on real pipes: answers the requests by `script`, records the
/// methods, and exits on `Browser.close`.
struct FakeBrowser {
    process: Arc<FakeProcess>,
    methods: Arc<Mutex<Vec<String>>>,
    client: Client<PipeWriter>,
    replies: Receiver<Reply>,
}

fn fake_browser(script: impl Fn(usize) -> Answer + Send + 'static) -> FakeBrowser {
    let (req_r, req_w) = std::io::pipe().unwrap();
    let (rep_r, rep_w) = std::io::pipe().unwrap();
    let process = Arc::new(FakeProcess::default());
    let methods = Arc::new(Mutex::new(Vec::new()));
    let (e, m) = (process.clone(), methods.clone());
    std::thread::spawn(move || serve(req_r, rep_w, &e, &m, script));
    FakeBrowser {
        process,
        methods,
        client: Client::new(req_w),
        replies: cdp::read_replies(rep_r).unwrap(),
    }
}

fn serve(
    from: PipeReader,
    mut to: PipeWriter,
    process: &FakeProcess,
    methods: &Mutex<Vec<String>>,
    script: impl Fn(usize) -> Answer,
) {
    let mut reader = BufReader::new(from);
    let mut n = 0;
    loop {
        let mut msg = Vec::new();
        if reader.read_until(0, &mut msg).unwrap_or(0) == 0 {
            return;
        }
        msg.pop();
        let v: serde_json::Value = serde_json::from_slice(&msg).unwrap();
        let method = v["method"].as_str().unwrap().to_string();
        methods.lock().unwrap().push(method.clone());
        let id = v["id"].as_u64().unwrap();
        if method == "Browser.close" {
            let _ = write_reply(&mut to, id, serde_json::json!({}));
            if !process.ignores_close.load(Ordering::Relaxed) {
                process.exited.store(true, Ordering::Relaxed);
            }
            return;
        }
        n += 1;
        match script(n) {
            Answer::Cookies(c) => {
                let list: Vec<_> = c
                    .iter()
                    .map(|c| {
                        serde_json::json!({ "name": c.name, "value": c.value,
                            "domain": c.domain, "path": c.path, "secure": c.secure,
                            "expires": c.expires.unwrap_or(-1.0),
                            "session": c.expires.is_none() })
                    })
                    .collect();
                // An event first: it must be skipped.
                let _ = to.write_all(b"{\"method\":\"Some.event\",\"params\":{}}\0");
                if write_reply(&mut to, id, serde_json::json!({ "cookies": list })).is_err() {
                    return;
                }
            }
            Answer::Silence => {}
            Answer::Late(after) => {
                std::thread::sleep(after);
                if write_reply(&mut to, id, serde_json::json!({ "cookies": [] })).is_err() {
                    return;
                }
            }
            Answer::Hang => std::thread::sleep(Duration::from_secs(60)),
            Answer::ExitProcess => process.exited.store(true, Ordering::Relaxed),
            Answer::ClosePipe => return,
        }
    }
}

fn write_reply(to: &mut PipeWriter, id: u64, result: serde_json::Value) -> std::io::Result<()> {
    let mut bytes = serde_json::json!({ "id": id, "result": result })
        .to_string()
        .into_bytes();
    bytes.push(0);
    to.write_all(&bytes)
}

fn run_fake(fb: &mut FakeBrowser, cancel: &AtomicBool, limits: &Limits) -> Outcome {
    watch(
        &*fb.process,
        &mut fb.client,
        &fb.replies,
        cancel,
        limits,
        || NOW,
    )
}

#[test]
fn a_settled_session_gives_the_music_header_and_closes_the_browser() {
    let mut fb = fake_browser(|n| match n {
        1 | 2 => Answer::Cookies(vec![cookie("NID", ".google.com")]),
        _ => Answer::Cookies(signed_in_cookies()),
    });
    let out = run_fake(&mut fb, &AtomicBool::new(false), &quick());
    assert_eq!(
        out,
        Outcome::SignedIn("SAPISID=v-SAPISID; SID=v-SID".into())
    );
    let methods = fb.methods.lock().unwrap().clone();
    assert_eq!(methods.last().map(String::as_str), Some("Browser.close"));
    assert!(
        methods[..methods.len() - 1]
            .iter()
            .all(|m| m == "Storage.getCookies"),
        "{methods:?}"
    );
    assert!(
        !fb.process.killed.load(Ordering::Relaxed),
        "it closed by itself"
    );
}

#[test]
fn a_browser_that_stays_after_browser_close_is_ended() {
    let mut fb = fake_browser(|_| Answer::Cookies(signed_in_cookies()));
    fb.process.ignores_close.store(true, Ordering::Relaxed);
    let limits = Limits {
        close: Duration::from_millis(100),
        ..quick()
    };
    let out = run_fake(&mut fb, &AtomicBool::new(false), &limits);
    assert!(matches!(out, Outcome::SignedIn(_)), "{out:?}");
    assert!(fb.process.killed.load(Ordering::Relaxed));
}

#[test]
fn closing_the_browser_while_the_session_settles_keeps_the_sign_in() {
    let mut fb = fake_browser(|n| match n {
        1 => Answer::Cookies(signed_in_cookies()),
        _ => Answer::ExitProcess,
    });
    let limits = Limits {
        settle: Duration::from_millis(200),
        ..quick()
    };
    assert_eq!(
        run_fake(&mut fb, &AtomicBool::new(false), &limits),
        Outcome::SignedIn("SAPISID=v-SAPISID; SID=v-SID".into())
    );
}

#[test]
fn a_browser_exiting_at_once_didnt_start() {
    let mut fb = fake_browser(|_| Answer::ExitProcess);
    assert!(matches!(
        run_fake(&mut fb, &AtomicBool::new(false), &quick()),
        Outcome::DidntStart(_)
    ));
}

#[test]
fn a_browser_closed_later_without_ever_answering_was_closed() {
    // The pipe turned off by policy: no answer, then the user closes the window.
    let process = FakeProcess::default();
    process.exited.store(true, Ordering::Relaxed);
    let ran = EARLY_EXIT + Duration::from_secs(5);
    assert_eq!(stopped(Stop::Exited, false, ran, &process), Outcome::Closed);
    assert_eq!(
        stopped(Stop::PipeClosed, false, ran, &process),
        Outcome::Closed
    );
}

#[test]
fn the_pipe_closing_as_the_browser_exits_means_closed() {
    let process = FakeProcess::default();
    process.exited.store(true, Ordering::Relaxed);
    assert_eq!(
        stopped(Stop::PipeClosed, true, Duration::from_secs(30), &process),
        Outcome::Closed
    );
}

#[test]
fn the_debug_form_leaves_the_header_out() {
    let o = Outcome::SignedIn("SAPISID=secret".into());
    assert!(!format!("{o:?}").contains("secret"));
}

#[test]
fn closing_the_browser_ends_the_sign_in() {
    let mut fb = fake_browser(|n| match n {
        1 => Answer::Cookies(vec![]),
        _ => Answer::ExitProcess,
    });
    assert_eq!(
        run_fake(&mut fb, &AtomicBool::new(false), &quick()),
        Outcome::Closed
    );
}

#[test]
fn no_first_answer_means_blocked() {
    let mut fb = fake_browser(|_| Answer::Silence);
    assert_eq!(
        run_fake(&mut fb, &AtomicBool::new(false), &quick()),
        Outcome::Blocked
    );
    assert!(fb.process.killed.load(Ordering::Relaxed));
}

/// A browser that starts slowly answers its first command late: the first reply gets
/// longer than the others.
#[test]
fn a_slow_first_answer_is_waited_for() {
    let mut fb = fake_browser(|n| match n {
        1 => Answer::Late(Duration::from_millis(600)),
        _ => Answer::Cookies(signed_in_cookies()),
    });
    let out = run_fake(&mut fb, &AtomicBool::new(false), &quick());
    assert!(matches!(out, Outcome::SignedIn(_)), "{out:?}");
}

#[test]
fn the_first_answer_waits_longer_on_linux_only() {
    let l = Limits::real();
    let first = if cfg!(target_os = "linux") { 60 } else { 15 };
    assert_eq!(l.first_reply, Duration::from_secs(first));
    assert_eq!(l.reply, Duration::from_secs(15));
}

#[test]
fn no_later_answer_means_lost() {
    let mut fb = fake_browser(|n| match n {
        1 => Answer::Cookies(vec![]),
        _ => Answer::Hang,
    });
    assert!(matches!(
        run_fake(&mut fb, &AtomicBool::new(false), &quick()),
        Outcome::Lost(_)
    ));
    assert!(fb.process.killed.load(Ordering::Relaxed));
}

#[test]
fn a_broken_pipe_while_the_browser_runs_means_lost() {
    let mut fb = fake_browser(|n| match n {
        1 => Answer::Cookies(vec![]),
        _ => Answer::ClosePipe,
    });
    assert!(matches!(
        run_fake(&mut fb, &AtomicBool::new(false), &quick()),
        Outcome::Lost(_)
    ));
    assert!(fb.process.killed.load(Ordering::Relaxed));
}

#[test]
fn no_sign_in_in_time_ends_it() {
    let mut fb = fake_browser(|_| Answer::Cookies(vec![]));
    let limits = Limits {
        timeout: Duration::from_millis(100),
        ..quick()
    };
    assert_eq!(
        run_fake(&mut fb, &AtomicBool::new(false), &limits),
        Outcome::TimedOut
    );
    assert!(fb.process.killed.load(Ordering::Relaxed));
}

#[test]
fn cancel_ends_it() {
    let mut fb = fake_browser(|_| Answer::Cookies(vec![]));
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        c.store(true, Ordering::Relaxed);
    });
    assert_eq!(run_fake(&mut fb, &cancel, &quick()), Outcome::Cancelled);
    assert!(fb.process.killed.load(Ordering::Relaxed));
}

#[allow(clippy::print_stderr, reason = "the live checks report what they saw")]
fn report(what: &str) {
    eprintln!("browser: {what}");
}

/// What a live check starts: every known browser installed on this PC whose policy
/// leaves the pipe on; or only the Chromium-based exe in `FEATHERPLAY_PIPE_TEST_BROWSER`
/// (for a PC whose policies turn DevTools off), started as Edge is.
fn live_browsers() -> Vec<(&'static Browser, PathBuf)> {
    if let Some(exe) = std::env::var_os("FEATHERPLAY_PIPE_TEST_BROWSER") {
        return vec![(&EDGE, PathBuf::from(exe))];
    }
    let mut usable = Vec::new();
    for found in browsers::found(&browsers::ThisPc) {
        if let Some(why) = found.blocked {
            report(&format!(
                "{}: skipped, the sign-in can't use it ({why:?})",
                found.browser.name
            ));
        } else {
            usable.push((found.browser, found.exe));
        }
    }
    assert!(
        !usable.is_empty(),
        "no known browser is installed with its pipe on"
    );
    usable
}

/// A browser started on the pipe for a live check, on about:blank, with a temporary
/// profile under the test home. Dropping it ends the browser and deletes the profile.
struct Live {
    process: Option<browsers::Running>,
    client: Client<std::fs::File>,
    replies: Receiver<Reply>,
    profile: PathBuf,
}

impl Live {
    fn start(browser: &Browser, exe: &Path) -> Self {
        let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
        let cache = featherplay_store::Paths::from_home(Path::new(&home)).cache;
        report(&format!("{}: starting {}", browser.name, exe.display()));
        let profile = profile::create(&cache).unwrap();
        if let Some(prefs) = browser.preferences {
            profile::seed(&profile, prefs).unwrap();
        }
        let (process, pipes) =
            browsers::launch(exe, &args(browser, &profile, "about:blank")).unwrap();
        Self {
            process: Some(process),
            client: Client::new(pipes.to_browser),
            replies: cdp::read_replies(pipes.from_browser).unwrap(),
            profile,
        }
    }

    fn process(&self) -> &browsers::Running {
        self.process.as_ref().unwrap()
    }

    /// Asks for the cookies; the reply, if one comes within 30 s, or the sign-in's wait for a
    /// first reply if longer (Linux: a browser's first start on a fresh CI runner is slow).
    fn cookies(&mut self) -> Option<Result<Vec<Cookie>, String>> {
        let id = self.client.send(Method::GetCookies).ok()?;
        let never = AtomicBool::new(false);
        let within = Limits::real().first_reply.max(Duration::from_secs(30));
        let r = await_reply(id, &self.replies, self.process(), &never, within);
        r.ok().map(|r| r.and_then(cdp::cookies))
    }

    /// Ends the browser and deletes the profile; says whether the profile is gone.
    fn finish(&mut self) -> bool {
        drop(self.process.take());
        profile::remove(&self.profile, 40, Duration::from_millis(250))
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Every known browser installed here, for real, on the pipe: on about:blank with a
/// temporary profile under the test home, so no Google page and no account. Each one's
/// window shows for a moment.
/// `FEATHERPLAY_HOME=<a test home> cargo nextest run -p featherplay --lib --run-ignored only
/// -E 'test(live_browsers)' --no-capture`
#[test]
#[ignore = "starts every known browser that's installed (about:blank, temporary profile)"]
fn live_browsers_answer_on_the_pipe_and_close() {
    for (browser, exe) in live_browsers() {
        let name = browser.name;
        let mut live = Live::start(browser, &exe);
        let cookies = live
            .cookies()
            .unwrap_or_else(|| panic!("{name}: no reply on the pipe"))
            .unwrap();
        report(&format!(
            "{name}: Storage.getCookies: {} cookies",
            cookies.len()
        ));
        assert!(
            session::music_cookies(&cookies, wall_now()).is_empty(),
            "{name}: a new profile has no YouTube cookies"
        );

        live.client.send(Method::CloseBrowser).unwrap();
        let exited = live.process().wait(Duration::from_secs(10));
        report(&format!(
            "{name}: exited within 10 s of Browser.close: {exited}"
        ));
        assert!(exited, "{name} didn't exit on Browser.close");
        assert!(
            live.finish(),
            "{name}: the temporary profile is still there"
        );
        assert!(!live.profile.exists());
    }
}

/// Does each browser's process exit when its last window is closed (WM_CLOSE), with the
/// pipe open? The sign-in takes that as "the user closed it". About:blank, temporary
/// profile, as above. Windows only: Linux has no `visible_windows` to close.
#[cfg(windows)]
#[test]
#[ignore = "starts every known browser that's installed (about:blank, temporary profile)"]
fn live_browsers_exit_when_their_window_closes() {
    for (browser, exe) in live_browsers() {
        exits_when_its_window_closes(browser, &exe);
    }
}

#[cfg(windows)]
fn exits_when_its_window_closes(browser: &Browser, exe: &Path) {
    let name = browser.name;
    let mut live = Live::start(browser, exe);
    let answered = live.cookies().is_some();
    report(&format!("{name}: answers on the pipe: {answered}"));
    let mut found = Vec::new();
    // Its window may take a moment.
    for _ in 0..50 {
        found = browsers::visible_windows(live.process().pid);
        if !found.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(!found.is_empty(), "{name}: no visible window");
    report(&format!(
        "{name}: {} visible window(s); sending WM_CLOSE",
        found.len()
    ));
    for &window in &found {
        browsers::close_window(window);
    }
    let exited = live.process().wait(Duration::from_secs(10));
    report(&format!("{name}: exited within 10 s of WM_CLOSE: {exited}"));
    assert!(
        live.finish(),
        "{name}: the temporary profile is still there"
    );
    assert!(exited, "{name} kept running after its window closed");
}
