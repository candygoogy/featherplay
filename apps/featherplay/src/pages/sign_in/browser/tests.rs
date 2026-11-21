use super::*;
use featherplay_platform::browsers::EDGE;
#[cfg(windows)]
use std::io::{BufRead, BufReader, PipeReader, PipeWriter};
#[cfg(windows)]
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

#[cfg(windows)]
#[test]
fn with_the_policy_off_the_browser_isnt_started() {
    let Some(dir) = crate::testing::scratch("policy") else {
        return;
    };
    let cache = dir.join("cache");
    let out = run_on(&PolicyOffPc, &EDGE, &cache, &AtomicBool::new(false));
    let made = cache.exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out, Outcome::Unusable(Block::Policy));
    // No profile was made, so nothing was launched.
    assert!(!made);
}

/// The sign-in window is the browser as the user would start it: the profile, its own
/// switches (which neither mark it as automated nor hide that, see `browsers`), the page. No
/// pipe.
#[test]
fn the_sign_in_window_has_no_pipe() {
    let a = window_args(&EDGE, Path::new("signin-1-ab"), session::SIGN_IN_URL);
    let mut want = vec![OsString::from("--user-data-dir=signin-1-ab")];
    want.extend(EDGE.switches.iter().map(OsString::from));
    want.push(session::SIGN_IN_URL.into());
    assert_eq!(a, want);
}

/// Firefox gets its profile as `--profile <dir>`, and a new instance of its own.
#[cfg(target_os = "linux")]
#[test]
fn firefox_starts_on_its_own_instance_with_the_profile() {
    use featherplay_platform::browsers::FIREFOX;
    let a = window_args(&FIREFOX, Path::new("/p/signin-1-ab"), session::SIGN_IN_URL);
    let want: [OsString; 4] = [
        "--profile".into(),
        "/p/signin-1-ab".into(),
        "--new-instance".into(),
        session::SIGN_IN_URL.into(),
    ];
    assert_eq!(a, want);
}

/// A Snap's or a Flatpak's profile goes where its sandbox lets it reach, named for
/// featherplay; any other browser's in the app's cache.
#[cfg(target_os = "linux")]
#[test]
fn a_sandboxed_browsers_profile_goes_where_it_can_reach() {
    use browsers::Sandbox;
    struct Home;
    impl browsers::Lookup for Home {
        fn env(&self, var: &str) -> Option<OsString> {
            (var == "HOME").then(|| "/home/ada".into())
        }
        fn is_file(&self, _: &Path) -> bool {
            false
        }
        fn resolve(&self, _: &Path) -> Option<PathBuf> {
            None
        }
        fn head(&self, _: &Path) -> Option<Vec<u8>> {
            None
        }
        fn read(&self, _: &Path) -> Option<String> {
            None
        }
    }
    let cache = Path::new("/home/ada/.cache/featherplay");
    let cases = [
        (Sandbox::None, "/home/ada/.cache/featherplay", "signin-"),
        (
            Sandbox::Snap("firefox".into()),
            "/home/ada/snap/firefox/common",
            "featherplay-signin-",
        ),
        (
            Sandbox::Flatpak("org.chromium.Chromium"),
            "/home/ada/.var/app/org.chromium.Chromium/cache",
            "featherplay-signin-",
        ),
    ];
    for (sandbox, dir, prefix) in cases {
        assert_eq!(
            profile_place(&sandbox, &Home, cache),
            (PathBuf::from(dir), prefix),
            "{sandbox:?}"
        );
    }
}

/// The read that follows is headless, on a blank page: the pipe is never open on Google's.
#[cfg(windows)]
#[test]
fn the_read_is_headless_on_a_blank_page() {
    let a = read_args(&EDGE, Path::new("signin-1-ab"));
    let mut want: Vec<OsString> = vec![
        "--user-data-dir=signin-1-ab".into(),
        "--remote-debugging-pipe".into(),
        "--headless=new".into(),
    ];
    want.extend(EDGE.switches.iter().map(OsString::from));
    want.push("about:blank".into());
    assert_eq!(a, want);
}

/// The browser's process, as the tests drive it.
#[derive(Default)]
struct FakeProcess {
    exited: AtomicBool,
    killed: AtomicBool,
    /// `Browser.close` is answered but the process stays.
    #[cfg(windows)]
    ignores_close: AtomicBool,
    /// Asked to quit, as closing its window would.
    quit: AtomicBool,
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
    fn quit(&self) {
        self.quit.store(true, Ordering::Relaxed);
        self.exited.store(true, Ordering::Relaxed);
    }
}

fn quick() -> Limits {
    Limits {
        #[cfg(windows)]
        first_reply: Duration::from_millis(1000),
        #[cfg(windows)]
        resend: Duration::from_millis(100),
        timeout: Duration::from_secs(5),
        early_exit: Duration::from_millis(100),
        close: Duration::from_secs(2),
    }
}

/// `quick`, with the waits for the fake browser's reply and exit beyond what a loaded machine
/// keeps its thread waiting (under a build beside the suite, its reply took over 100 ms: the
/// read asked again, 2026-11-20), and below nextest's minute. For the tests about what's
/// sent and read, not about the time.
#[cfg(windows)]
fn patient() -> Limits {
    Limits {
        first_reply: Duration::from_secs(20),
        close: Duration::from_secs(20),
        ..quick()
    }
}

/// What the fake browser does with each `Storage.getCookies`, by its number (from 1).
#[cfg(windows)]
enum Answer {
    Cookies(Vec<Cookie>),
    Silence,
    /// No cookies, after this long.
    Late(Duration),
    ExitProcess,
    ClosePipe,
}

/// A fake browser on real pipes: answers the requests by `script`, records the
/// methods, and exits on `Browser.close`.
#[cfg(windows)]
struct FakeBrowser {
    process: Arc<FakeProcess>,
    methods: Arc<Mutex<Vec<String>>>,
    client: Client<PipeWriter>,
    replies: Receiver<Reply>,
}

#[cfg(windows)]
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

#[cfg(windows)]
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
            Answer::ExitProcess => process.exited.store(true, Ordering::Relaxed),
            Answer::ClosePipe => return,
        }
    }
}

#[cfg(windows)]
fn write_reply(to: &mut PipeWriter, id: u64, result: serde_json::Value) -> std::io::Result<()> {
    let mut bytes = serde_json::json!({ "id": id, "result": result })
        .to_string()
        .into_bytes();
    bytes.push(0);
    to.write_all(&bytes)
}

#[test]
fn the_debug_form_leaves_the_header_out() {
    let o = Outcome::SignedIn("SAPISID=secret".into());
    assert!(!format!("{o:?}").contains("secret"));
}

#[test]
fn the_sign_in_waits_15_minutes_and_the_read_15_s() {
    let l = Limits::real();
    #[cfg(windows)]
    assert_eq!(
        (l.first_reply, l.resend),
        (Duration::from_secs(15), Duration::from_secs(5))
    );
    assert_eq!(l.timeout, Duration::from_secs(15 * 60));
    assert_eq!(l.early_exit, Duration::from_secs(3));
}

/// The headless read, against the fake browser.
#[cfg(windows)]
fn read_fake(fb: &mut FakeBrowser, limits: &Limits) -> Result<Vec<Cookie>, Outcome> {
    let never = AtomicBool::new(false);
    read_and_close(&*fb.process, &mut fb.client, &fb.replies, &never, limits)
}

#[cfg(windows)]
#[test]
fn the_read_sends_exactly_the_two_commands() {
    let mut fb = fake_browser(|_| Answer::Cookies(signed_in_cookies()));
    // Nothing asked again: the reply comes before a resend is due, however late it is.
    let limits = Limits {
        resend: patient().first_reply,
        ..patient()
    };
    let cookies = read_fake(&mut fb, &limits).unwrap();
    assert_eq!(cookies, signed_in_cookies());
    assert_eq!(
        *fb.methods.lock().unwrap(),
        ["Storage.getCookies", "Browser.close"]
    );
    assert!(
        !fb.process.killed.load(Ordering::Relaxed),
        "it closed by itself"
    );
}

#[cfg(windows)]
#[test]
fn a_read_with_no_answer_is_blocked_and_the_browser_ended() {
    let mut fb = fake_browser(|_| Answer::Silence);
    assert_eq!(read_fake(&mut fb, &quick()).unwrap_err(), Outcome::Blocked);
    assert!(fb.process.killed.load(Ordering::Relaxed));
    let methods = fb.methods.lock().unwrap().clone();
    assert!(
        methods.iter().all(|m| m == "Storage.getCookies"),
        "{methods:?}"
    );
}

#[cfg(windows)]
#[test]
fn a_headless_browser_gone_before_it_answers_didnt_start() {
    let answers: [fn(usize) -> Answer; 2] = [|_| Answer::ExitProcess, |_| Answer::ClosePipe];
    for answer in answers {
        let mut fb = fake_browser(answer);
        let out = read_fake(&mut fb, &patient()).unwrap_err();
        assert!(matches!(out, Outcome::DidntStart(_)), "{out:?}");
        assert!(fb.process.killed.load(Ordering::Relaxed));
    }
}

#[cfg(windows)]
#[test]
fn a_browser_that_stays_after_browser_close_is_ended() {
    let mut fb = fake_browser(|_| Answer::Cookies(signed_in_cookies()));
    fb.process.ignores_close.store(true, Ordering::Relaxed);
    let limits = Limits {
        close: Duration::from_millis(100),
        ..quick()
    };
    assert_eq!(read_fake(&mut fb, &limits).unwrap(), signed_in_cookies());
    assert!(fb.process.killed.load(Ordering::Relaxed));
}

/// A browser that starts slowly answers late: the read waits for it.
#[cfg(windows)]
#[test]
fn a_slow_first_answer_is_waited_for_by_the_read() {
    // Asked again every 100 ms meanwhile; the answer to the first still counts.
    let mut fb = fake_browser(|_| Answer::Late(Duration::from_millis(600)));
    assert_eq!(
        read_fake(&mut fb, &patient()).unwrap(),
        Vec::<Cookie>::new()
    );
}

#[test]
fn the_profile_with_a_session_is_signed_in_and_one_without_was_closed() {
    assert_eq!(
        session_or(&signed_in_cookies(), NOW, Ended::Closed),
        Outcome::SignedIn("SAPISID=v-SAPISID; SID=v-SID".into())
    );
    let google_only = [cookie("SAPISID", ".google.com")];
    assert_eq!(
        session_or(&google_only, NOW, Ended::Closed),
        Outcome::Closed
    );
}

/// A sign-in window the user closes `after` the wait first looks at it: timed from that look,
/// not by a thread of its own, which a loaded machine may start late.
struct ClosedLater {
    after: Duration,
    first_look: std::sync::OnceLock<Instant>,
    killed: AtomicBool,
}

impl Process for ClosedLater {
    fn exited(&self) -> bool {
        self.first_look.get_or_init(Instant::now).elapsed() >= self.after
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
    }
    fn quit(&self) {}
}

#[test]
fn closing_the_sign_in_window_goes_on_to_the_read() {
    let never = AtomicBool::new(false);
    let limits = quick();
    // The user closes it once it has been up a while.
    let process = ClosedLater {
        after: limits.early_exit + Duration::from_millis(200),
        first_look: std::sync::OnceLock::new(),
        killed: AtomicBool::new(false),
    };
    assert_eq!(wait_closed(&process, &never, &limits), Ok(Ended::Closed));
    assert!(!process.killed.load(Ordering::Relaxed));
}

#[test]
fn the_sign_in_window_ends_on_an_early_exit_or_cancel() {
    let exited = FakeProcess::default();
    exited.exited.store(true, Ordering::Relaxed);
    let never = AtomicBool::new(false);
    assert!(matches!(
        wait_closed(&exited, &never, &quick()),
        Err(Outcome::DidntStart(_))
    ));

    let running = FakeProcess::default();
    let cancel = AtomicBool::new(true);
    assert_eq!(
        wait_closed(&running, &cancel, &quick()),
        Err(Outcome::Cancelled)
    );
    assert!(running.killed.load(Ordering::Relaxed));
}

/// Someone who signed in and left the window open keeps the sign-in: at the limit the
/// browser is asked to quit (not killed), and its profile is read as after a close.
#[test]
fn at_the_time_limit_the_window_is_closed_and_a_sign_in_still_read() {
    let open = FakeProcess::default();
    let never = AtomicBool::new(false);
    let limits = Limits {
        timeout: Duration::from_millis(100),
        ..quick()
    };
    assert_eq!(wait_closed(&open, &never, &limits), Ok(Ended::TimeLimit));
    assert!(open.quit.load(Ordering::Relaxed));
    assert!(!open.killed.load(Ordering::Relaxed), "it quit when asked");
    assert_eq!(
        session_or(&signed_in_cookies(), NOW, Ended::TimeLimit),
        Outcome::SignedIn("SAPISID=v-SAPISID; SID=v-SID".into())
    );
    assert_eq!(session_or(&[], NOW, Ended::TimeLimit), Outcome::TimedOut);
}

#[cfg(windows)]
#[allow(clippy::print_stderr, reason = "the live checks report what they saw")]
fn report(what: &str) {
    eprintln!("browser: {what}");
}

/// What a live check starts: every known browser installed on this PC whose policy
/// leaves the pipe on; or only the Chromium-based exe in `FEATHERPLAY_PIPE_TEST_BROWSER`
/// (for a PC whose policies turn DevTools off), started as Edge is.
#[cfg(windows)]
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
#[cfg(windows)]
struct Live {
    process: Option<browsers::Running>,
    client: Client<std::fs::File>,
    replies: Receiver<Reply>,
    profile: PathBuf,
}

#[cfg(windows)]
impl Live {
    fn start(browser: &Browser, exe: &Path) -> Self {
        let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
        let cache = featherplay_store::Paths::from_home(Path::new(&home)).cache;
        report(&format!("{}: starting {}", browser.name, exe.display()));
        let profile = profile::create(&cache, profile::PREFIX).unwrap();
        if let Some(prefs) = browser.preferences {
            profile::seed(&profile, prefs).unwrap();
        }
        // As the sign-in starts it with the pipe: the headless read.
        let args = read_args(browser, &profile);
        let (process, pipes) = browsers::launch(exe, &args).unwrap();
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

    /// Asks for the cookies, again every 5 s as the sign-in does before a first reply; the
    /// reply, if one comes within 30 s, or the sign-in's wait for a first reply if longer
    /// (Linux: a browser's first start on a fresh CI runner is slow).
    fn cookies(&mut self) -> Option<Result<Vec<Cookie>, String>> {
        let never = AtomicBool::new(false);
        let limits = Limits::real();
        let within = limits.first_reply.max(Duration::from_secs(30));
        let process = self.process.as_ref().unwrap();
        let resend = Some(limits.resend);
        let r = ask(
            &mut self.client,
            &self.replies,
            process,
            &never,
            within,
            resend,
        );
        r.ok().map(|r| r.and_then(cdp::cookies))
    }

    /// Ends the browser and deletes the profile; says whether the profile is gone.
    fn finish(&mut self) -> bool {
        drop(self.process.take());
        profile::remove(&self.profile, 40, Duration::from_millis(250))
    }
}

#[cfg(windows)]
impl Drop for Live {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Every known browser installed here, for real, on the pipe: headless on about:blank, as the
/// sign-in's read is, with a temporary profile under the test home, so no Google page and no
/// account. Windows only: Linux has no pipe (its live check is the platform's).
/// `FEATHERPLAY_HOME=<a test home> cargo nextest run -p featherplay --lib --run-ignored only
/// -E 'test(live_browsers)' --no-capture`
#[cfg(windows)]
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

/// Does each browser quit when asked (`Running::quit`, WM_CLOSE to its windows), as the
/// sign-in asks a window left open at its time limit? Started as the sign-in window is, with
/// no pipe, on about:blank with a temporary profile as above. Windows only: Linux's quit is a
/// signal (`quit_asks_the_process_to_exit` in the platform).
#[cfg(windows)]
#[test]
#[ignore = "starts every known browser that's installed (about:blank, temporary profile)"]
fn live_browsers_quit_when_asked() {
    for (browser, exe) in live_browsers() {
        quits_when_asked(browser, &exe);
    }
}

#[cfg(windows)]
fn quits_when_asked(browser: &Browser, exe: &Path) {
    let name = browser.name;
    let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
    let cache = featherplay_store::Paths::from_home(Path::new(&home)).cache;
    report(&format!("{name}: starting {}", exe.display()));
    let profile = TempProfile(profile::create(&cache, profile::PREFIX).unwrap());
    if let Some(prefs) = browser.preferences {
        profile::seed(&profile.0, prefs).unwrap();
    }
    let window = browsers::launch_plain(exe, &window_args(browser, &profile.0, "about:blank"));
    let window = window.unwrap();
    // Its window may take a moment.
    let shown = (0..50).any(|_| {
        !browsers::visible_windows(window.pid).is_empty() || {
            std::thread::sleep(Duration::from_millis(200));
            false
        }
    });
    assert!(shown, "{name}: no visible window");
    window.quit();
    let exited = window.wait(Duration::from_secs(10));
    report(&format!("{name}: exited within 10 s of quit: {exited}"));
    drop(window);
    assert!(exited, "{name} kept running after it was asked to quit");
}
