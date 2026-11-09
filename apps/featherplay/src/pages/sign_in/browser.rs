//! "Use Microsoft Edge": Google's sign-in in a real browser window (its own address bar,
//! padlock, passkeys and Windows Hello), for people who'd rather not type their password
//! into a window of the app. The WebView2 window (`webview`) stays the default.
//!
//! How it goes, on its own thread:
//! 1. Find the browser's exe (`known`: App Paths, then where its installer puts it).
//! 2. Make a temporary profile, `{cache}/signin-{pid}-{random}` (`profile`). The user's own
//!    browser profile is never touched, and no browser's cookie files are read.
//! 3. Start the browser on it with `--remote-debugging-pipe`, on Google's sign-in page,
//!    inside a kill-on-close job object (`launch`, which documents how the pipe reaches
//!    Chromium). A pipe only: never `--remote-debugging-port`, so no other program can
//!    connect. No switch that marks the browser as automated.
//! 4. About once a second, ask for the profile's cookies (`cdp`: the app can send only
//!    `Storage.getCookies` and `Browser.close`).
//! 5. Signed in = the YouTube session cookies (`session::signed_in`, the rule the WebView2
//!    path and the controller use). Then wait 3 s so music.youtube.com can set its own
//!    cookies, read once more, and build the header both sign-ins build
//!    (`session::music_cookies`); every other site's cookies are dropped.
//! 6. `Browser.close`, wait up to 5 s for the browser to exit (then end the job), delete
//!    the profile, and hand the header over, as the WebView2 window does.
//!
//! It ends early, ending the job and deleting the profile, when the browser's process exits
//! (the user closed it), after 10 minutes without a sign-in, when the pipe breaks, when the
//! browser doesn't answer within 15 s (a policy may block DevTools), or on Cancel. Profiles
//! a crash left behind are deleted at the next start (`sweep`).
//!
//! "Switch Google account" stays on the WebView2 window: it relies on that window's
//! profile remembering earlier logins for Google's account chooser, and this profile is
//! thrown away after every sign-in.
//!
//! Cookie values are never logged, printed or written here.

mod cdp;
mod known;
mod launch;
mod profile;

use std::ffi::OsString;
use std::io::Write;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::session::{self, Cookie};
use cdp::{Client, Method, Reply};
pub use known::{Browser, EDGE};

/// How a browser sign-in ended.
#[derive(PartialEq)]
pub enum Outcome {
    /// The `cookie:` header for music.youtube.com, with a session cookie in it.
    SignedIn(String),
    /// The browser isn't installed (or can't be found).
    NotFound,
    /// The browser couldn't be started, or exited before it answered.
    DidntStart(String),
    /// The browser never answered: a policy may block DevTools.
    Blocked,
    /// The browser's `RemoteDebuggingAllowed` policy is off, so the browser wasn't started.
    PolicyOff,
    /// The user closed the browser before signing in.
    Closed,
    /// No sign-in within the time limit.
    TimedOut,
    /// The pipe broke, or the browser stopped answering.
    Lost(String),
    /// The user pressed Cancel.
    Cancelled,
}

/// The header stays out of debug output.
impl std::fmt::Debug for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::SignedIn(h) => write!(f, "SignedIn({} bytes)", h.len()),
            Outcome::NotFound => f.write_str("NotFound"),
            Outcome::DidntStart(e) => write!(f, "DidntStart({e:?})"),
            Outcome::Blocked => f.write_str("Blocked"),
            Outcome::PolicyOff => f.write_str("PolicyOff"),
            Outcome::Closed => f.write_str("Closed"),
            Outcome::TimedOut => f.write_str("TimedOut"),
            Outcome::Lost(e) => write!(f, "Lost({e:?})"),
            Outcome::Cancelled => f.write_str("Cancelled"),
        }
    }
}

/// Ends a running browser sign-in.
#[derive(Clone)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Starts a sign-in in `browser` on its own thread, with its profile under `cache`; `done`
/// runs on that thread once the browser is gone and the profile deleted, always (a panic on
/// the way ends as `Lost`, after the guards have ended the browser and deleted the profile).
pub fn start(
    browser: &'static Browser,
    cache: PathBuf,
    done: impl FnOnce(Outcome) + Send + 'static,
) -> Result<Cancel, String> {
    let cancel = Cancel(Arc::new(AtomicBool::new(false)));
    let flag = cancel.0.clone();
    std::thread::Builder::new()
        .name("featherplay-browser-signin".into())
        .spawn(move || {
            let run = AssertUnwindSafe(|| run_on(&known::ThisPc, browser, &cache, &flag));
            let outcome = std::panic::catch_unwind(run)
                .unwrap_or_else(|_| Outcome::Lost("the sign-in thread panicked".into()));
            done(outcome)
        })
        .map(|_| cancel)
        .map_err(|e| format!("browser sign-in thread: {e}"))
}

/// A policy on this PC turns off `browser`'s DevTools pipe, so the sign-in can't use it.
/// Read from the registry each time (only read, never written).
pub fn pipe_blocked(browser: &Browser) -> bool {
    browser.pipe_blocked(&known::ThisPc)
}

/// Deletes the temporary profiles that earlier runs left in `cache` (a crash, a kill).
pub fn sweep(cache: &Path) -> usize {
    profile::sweep(cache)
}

/// The waits and limits of a sign-in.
#[derive(Clone, Debug)]
struct Limits {
    /// Between two cookie reads.
    poll: Duration,
    /// After the session cookies appear, before the last read.
    settle: Duration,
    /// For any reply; the first one missing means the browser doesn't allow the pipe.
    reply: Duration,
    /// For the whole sign-in.
    timeout: Duration,
    /// For the browser to exit after `Browser.close`.
    close: Duration,
}

impl Limits {
    fn real() -> Self {
        Self {
            poll: Duration::from_secs(1),
            settle: Duration::from_secs(3),
            reply: Duration::from_secs(15),
            timeout: timeout(),
            close: Duration::from_secs(5),
        }
    }
}

/// 10 minutes. A debug build takes `FEATHERPLAY_BROWSER_TIMEOUT_SECS` instead, to try the
/// timeout by hand.
fn timeout() -> Duration {
    let dev = cfg!(debug_assertions)
        .then(|| std::env::var("FEATHERPLAY_BROWSER_TIMEOUT_SECS").ok())
        .flatten()
        .and_then(|s| s.trim().parse().ok());
    Duration::from_secs(dev.unwrap_or(10 * 60))
}

/// What the sign-in watches besides the pipe: the browser's own process.
trait Process {
    fn exited(&self) -> bool;
    fn wait(&self, limit: Duration) -> bool;
    /// Ends it and everything it started.
    fn kill(&self);
}

impl Process for launch::Running {
    fn exited(&self) -> bool {
        launch::Running::exited(self)
    }
    fn wait(&self, limit: Duration) -> bool {
        launch::Running::wait(self, limit)
    }
    fn kill(&self) {
        launch::Running::kill(self)
    }
}

/// One sign-in in `browser`, with what it reads of the PC given.
fn run_on(
    pc: &impl known::Lookup,
    browser: &Browser,
    cache: &Path,
    cancel: &AtomicBool,
) -> Outcome {
    let Some(exe) = browser.find(pc) else {
        log!("browser sign-in: {} not found", browser.name);
        return Outcome::NotFound;
    };
    // The browser would open and never answer: don't start it at all.
    if browser.pipe_blocked(pc) {
        log!(
            "browser sign-in: {}'s RemoteDebuggingAllowed policy is off",
            browser.name
        );
        return Outcome::PolicyOff;
    }
    let profile = match profile::create(cache) {
        Ok(p) => TempProfile(p),
        Err(e) => return Outcome::DidntStart(format!("temporary profile: {}", e.kind())),
    };
    log!(
        "browser sign-in: temporary profile {}",
        display_name(&profile.0)
    );
    // The browser (its job) is gone when this returns; dropping `profile` deletes the folder.
    launch_and_watch(browser, &exe, &profile.0, cancel)
}

/// Deletes the temporary profile when dropped, on every way out (a panic too).
struct TempProfile(PathBuf);

impl Drop for TempProfile {
    fn drop(&mut self) {
        if profile::remove(&self.0, 20, Duration::from_millis(250)) {
            log!("browser sign-in: temporary profile deleted");
        } else {
            log!("browser sign-in: the temporary profile stays; the next start deletes it");
        }
    }
}

fn display_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn launch_and_watch(browser: &Browser, exe: &Path, profile: &Path, cancel: &AtomicBool) -> Outcome {
    let args = args(browser, profile, session::SIGN_IN_URL);
    let (process, pipes) = match launch::launch(exe, &args) {
        Ok(started) => started,
        Err(e) => {
            log!("browser sign-in: {} didn't start: {e}", browser.name);
            return Outcome::DidntStart(e.to_string());
        }
    };
    log!(
        "browser sign-in: {} started (pid {})",
        browser.name,
        process.pid
    );
    let replies = match cdp::read_replies(pipes.from_browser) {
        Ok(rx) => rx,
        Err(e) => {
            process.kill();
            return Outcome::DidntStart(format!("pipe reader: {e}"));
        }
    };
    let mut client = Client::new(pipes.to_browser);
    let outcome = watch(
        &process,
        &mut client,
        &replies,
        cancel,
        &Limits::real(),
        wall_now,
    );
    drop(client);
    // Closing the job ends whatever helper is still running.
    drop(process);
    outcome
}

/// The browser's arguments: the temporary profile, the pipe, its own switches, the page.
fn args(b: &Browser, profile: &Path, url: &str) -> Vec<OsString> {
    let mut dir = OsString::from("--user-data-dir=");
    dir.push(profile);
    let mut out = vec![dir, "--remote-debugging-pipe".into()];
    out.extend(b.switches.iter().map(OsString::from));
    out.push(url.into());
    out
}

fn wall_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Reads the cookies until the session is there and settled, then closes the browser; or
/// ends the browser when the sign-in ends any other way. `wall` is the clock cookies expire
/// by.
fn watch<W: Write>(
    process: &impl Process,
    client: &mut Client<W>,
    replies: &Receiver<Reply>,
    cancel: &AtomicBool,
    limits: &Limits,
    wall: impl Fn() -> f64,
) -> Outcome {
    let outcome = poll(process, client, replies, cancel, limits, wall);
    match &outcome {
        Outcome::SignedIn(_) => {
            let asked = client.send(Method::CloseBrowser).is_ok();
            if asked && process.wait(limits.close) {
                log!("browser sign-in: the browser closed");
            } else {
                log!("browser sign-in: the browser didn't close; ending it");
                process.kill();
            }
        }
        _ => process.kill(),
    }
    outcome
}

/// Why waiting stopped.
enum Stop {
    Cancelled,
    /// The browser's process exited.
    Exited,
    /// The pipe closed or broke.
    PipeClosed,
    /// No reply in time.
    NoReply,
}

fn poll<W: Write>(
    process: &impl Process,
    client: &mut Client<W>,
    replies: &Receiver<Reply>,
    cancel: &AtomicBool,
    limits: &Limits,
    wall: impl Fn() -> f64,
) -> Outcome {
    let started = Instant::now();
    let mut settle = Settle::new(limits.settle);
    let mut answered = false;
    loop {
        let reply = client
            .send(Method::GetCookies)
            .map_err(|_| Stop::PipeClosed)
            .and_then(|id| await_reply(id, replies, process, cancel, limits.reply));
        let result = match reply {
            Ok(r) => r,
            Err(stop) => return settle.or(stopped(stop, answered, started.elapsed(), process)),
        };
        if !answered {
            log!("browser sign-in: the browser answers on the pipe");
            answered = true;
        }
        let cookies = match result.and_then(cdp::cookies) {
            Ok(c) => c,
            Err(e) => {
                log!("browser sign-in: reading the cookies failed: {e}");
                return Outcome::Lost(e);
            }
        };
        let next = match settle.see(Instant::now(), &cookies, wall()) {
            Seen::Ready(header) => return Outcome::SignedIn(header),
            Seen::Settling(left) => left,
            Seen::NotYet if started.elapsed() >= limits.timeout => {
                log!(
                    "browser sign-in: no sign-in in {} s",
                    limits.timeout.as_secs()
                );
                return Outcome::TimedOut;
            }
            Seen::NotYet => limits.poll,
        };
        if let Err(stop) = pause(next, replies, process, cancel) {
            return settle.or(stopped(stop, answered, started.elapsed(), process));
        }
    }
}

/// The browser exiting sooner than this after the start, before it answered, didn't start;
/// later, the user closed it (on a PC whose policy turns the pipe off, it never answers).
const EARLY_EXIT: Duration = Duration::from_secs(3);

/// How the sign-in ends on `stop`, `ran` after the browser started.
fn stopped(stop: Stop, answered: bool, ran: Duration, process: &impl Process) -> Outcome {
    match stop {
        Stop::Cancelled => {
            log!("browser sign-in: cancelled");
            Outcome::Cancelled
        }
        Stop::Exited if !answered && ran < EARLY_EXIT => {
            log!("browser sign-in: the browser exited before it answered");
            Outcome::DidntStart("the browser exited before it answered".into())
        }
        Stop::Exited => {
            log!("browser sign-in: the browser was closed");
            Outcome::Closed
        }
        // The pipe closing alone doesn't prove the browser is gone: ask its process.
        Stop::PipeClosed if process.wait(Duration::from_secs(1)) => {
            stopped(Stop::Exited, answered, ran, process)
        }
        Stop::PipeClosed => {
            log!("browser sign-in: the pipe closed while the browser runs");
            Outcome::Lost("the pipe to the browser closed".into())
        }
        Stop::NoReply if !answered => {
            log!("browser sign-in: no answer on the pipe; a policy may block it");
            Outcome::Blocked
        }
        Stop::NoReply => {
            log!("browser sign-in: the browser stopped answering");
            Outcome::Lost("the browser stopped answering".into())
        }
    }
}

/// How often waits look at Cancel and the browser's process.
const SLICE: Duration = Duration::from_millis(100);

/// Waits for the reply to `id`, at most `within`.
fn await_reply(
    id: u64,
    replies: &Receiver<Reply>,
    process: &impl Process,
    cancel: &AtomicBool,
    within: Duration,
) -> Result<Result<serde_json::Value, String>, Stop> {
    let deadline = Instant::now() + within;
    loop {
        check(process, cancel)?;
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Stop::NoReply);
        }
        match replies.recv_timeout(left.min(SLICE)) {
            Ok(r) if r.id == id => return Ok(r.result),
            // A late reply to an earlier command.
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err(Stop::PipeClosed),
        }
    }
}

/// Waits `span` between reads, watching Cancel, the browser's process and the pipe.
fn pause(
    span: Duration,
    replies: &Receiver<Reply>,
    process: &impl Process,
    cancel: &AtomicBool,
) -> Result<(), Stop> {
    let until = Instant::now() + span;
    loop {
        check(process, cancel)?;
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        match replies.recv_timeout(left.min(SLICE)) {
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err(Stop::PipeClosed),
        }
    }
}

fn check(process: &impl Process, cancel: &AtomicBool) -> Result<(), Stop> {
    if cancel.load(Ordering::Relaxed) {
        Err(Stop::Cancelled)
    } else if process.exited() {
        Err(Stop::Exited)
    } else {
        Ok(())
    }
}

/// The signed-in rule with its settle: the session cookies first seen, then the header
/// from a read `settle` later (music.youtube.com sets its own cookies in between).
struct Settle {
    settle: Duration,
    since: Option<Instant>,
    /// The header from the latest read with a session in it, while settling.
    last: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Seen {
    /// No session yet.
    NotYet,
    /// Signed in; read again after this.
    Settling(Duration),
    /// Signed in and settled: the header.
    Ready(String),
}

impl Settle {
    fn new(settle: Duration) -> Self {
        Self {
            settle,
            since: None,
            last: None,
        }
    }

    /// `outcome`, unless the browser was closed while the session settled: then the sign-in had
    /// already worked, and the last read's header counts.
    fn or(&mut self, outcome: Outcome) -> Outcome {
        match (outcome, self.last.take()) {
            (Outcome::Closed, Some(header)) => {
                log!("browser sign-in: the browser closed while settling; using the last read");
                Outcome::SignedIn(header)
            }
            (outcome, _) => outcome,
        }
    }

    /// One read of the cookies at `now`; `wall` is the clock they expire by.
    fn see(&mut self, now: Instant, cookies: &[Cookie], wall: f64) -> Seen {
        let music = session::music_cookies(cookies, wall);
        if !session::signed_in(&music) {
            self.since = None;
            self.last = None;
            return Seen::NotYet;
        }
        let since = *self.since.get_or_insert_with(|| {
            log!("browser sign-in: session cookies present; settling");
            now
        });
        let waited = now.saturating_duration_since(since);
        if waited >= self.settle {
            log!("browser sign-in: signed in, {} cookies", music.len());
            self.last = None;
            Seen::Ready(session::header(&music))
        } else {
            self.last = Some(session::header(&music));
            Seen::Settling(self.settle - waited)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    struct PolicyOffPc;

    impl known::Lookup for PolicyOffPc {
        fn app_path(&self, _: known::Hive, _: &str) -> Option<String> {
            Some(r"C:\Edge\msedge.exe".into())
        }
        fn env(&self, _: &str) -> Option<OsString> {
            None
        }
        fn is_file(&self, _: &Path) -> bool {
            true
        }
        fn dword(&self, hive: known::Hive, _: &str, value: &str) -> Option<u32> {
            (hive == known::Hive::LocalMachine && value == "RemoteDebuggingAllowed").then_some(0)
        }
    }

    #[test]
    fn with_the_policy_off_the_browser_isnt_started() {
        let home = std::env::var_os("FEATHERPLAY_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let cache = home.join(format!("test-policy-{}", std::process::id()));
        let out = run_on(&PolicyOffPc, &EDGE, &cache, &AtomicBool::new(false));
        assert_eq!(out, Outcome::PolicyOff);
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
        assert_eq!(
            a,
            [
                r"--user-data-dir=C:\cache\signin-1-ab",
                "--remote-debugging-pipe",
                "--no-first-run",
                "--no-default-browser-check",
                session::SIGN_IN_URL,
            ]
        );
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
            reply: Duration::from_millis(300),
            timeout: Duration::from_secs(5),
            close: Duration::from_secs(2),
        }
    }

    /// What the fake browser does with each `Storage.getCookies`, by its number (from 1).
    enum Answer {
        Cookies(Vec<Cookie>),
        Silence,
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

    /// A browser started on the pipe for a live check: Edge, or the Chromium-based exe in
    /// `FEATHERPLAY_PIPE_TEST_BROWSER` (for a PC whose Edge policy turns DevTools off). On
    /// about:blank, with a temporary profile under the test home. Dropping it ends the
    /// browser and deletes the profile.
    struct Live {
        process: Option<launch::Running>,
        client: Client<std::fs::File>,
        replies: Receiver<Reply>,
        profile: PathBuf,
    }

    impl Live {
        fn start() -> Self {
            let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
            let cache = featherplay_store::Paths::from_home(Path::new(&home)).cache;
            let exe = std::env::var_os("FEATHERPLAY_PIPE_TEST_BROWSER")
                .map(PathBuf::from)
                .or_else(|| EDGE.find(&known::ThisPc))
                .expect("Edge isn't installed");
            report(&format!("starting {}", exe.display()));
            let profile = profile::create(&cache).unwrap();
            let (process, pipes) =
                launch::launch(&exe, &args(&EDGE, &profile, "about:blank")).unwrap();
            Self {
                process: Some(process),
                client: Client::new(pipes.to_browser),
                replies: cdp::read_replies(pipes.from_browser).unwrap(),
                profile,
            }
        }

        fn process(&self) -> &launch::Running {
            self.process.as_ref().unwrap()
        }

        /// Asks for the cookies; the reply, if one comes within 30 s.
        fn cookies(&mut self) -> Option<Result<Vec<Cookie>, String>> {
            let id = self.client.send(Method::GetCookies).ok()?;
            let never = AtomicBool::new(false);
            let r = await_reply(
                id,
                &self.replies,
                self.process(),
                &never,
                Duration::from_secs(30),
            );
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

    /// The real Edge on the pipe, on about:blank with a temporary profile under the test
    /// home: no Google page, no account. Its window shows for a moment.
    /// `FEATHERPLAY_HOME=<a test home> cargo nextest run -p featherplay --lib --run-ignored only
    /// -E 'test(live_edge)' --no-capture`
    #[test]
    #[ignore = "starts Microsoft Edge (about:blank, temporary profile)"]
    fn live_edge_answers_on_the_pipe_and_closes() {
        let mut live = Live::start();
        let cookies = live
            .cookies()
            .expect("no reply on the pipe (is the RemoteDebuggingAllowed policy off?)")
            .unwrap();
        report(&format!("Storage.getCookies: {} cookies", cookies.len()));
        assert!(
            session::music_cookies(&cookies, wall_now()).is_empty(),
            "a new profile has no YouTube cookies"
        );

        live.client.send(Method::CloseBrowser).unwrap();
        let exited = live.process().wait(Duration::from_secs(10));
        report(&format!("exited within 10 s of Browser.close: {exited}"));
        assert!(exited, "the browser didn't exit on Browser.close");
        assert!(live.finish(), "the temporary profile is still there");
        assert!(!live.profile.exists());
    }

    /// Review point: does Edge's process exit when its last window is closed (WM_CLOSE),
    /// with the pipe open? About:blank, temporary profile, as above.
    #[test]
    #[ignore = "starts Microsoft Edge (about:blank, temporary profile)"]
    fn live_edge_exits_when_its_window_closes() {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
        };
        use windows::core::BOOL;

        struct Find {
            pid: u32,
            found: Vec<HWND>,
        }
        unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: `lparam` is the `Find` passed to EnumWindows below, alive for the call.
            let find = unsafe { &mut *(lparam.0 as *mut Find) };
            let mut pid = 0u32;
            // SAFETY: a window handle Windows gave us and a live out pointer.
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            // SAFETY: handle-only query.
            if pid == find.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
                find.found.push(hwnd);
            }
            true.into()
        }

        let mut live = Live::start();
        let answered = live.cookies().is_some();
        report(&format!("answers on the pipe: {answered}"));
        let mut find = Find {
            pid: live.process().pid,
            found: Vec::new(),
        };
        // Its window may take a moment.
        for _ in 0..50 {
            find.found.clear();
            // SAFETY: `each` matches WNDENUMPROC and `find` outlives the call.
            unsafe {
                let _ = EnumWindows(Some(each), LPARAM((&raw mut find) as isize));
            }
            if !find.found.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(!find.found.is_empty(), "no visible window");
        report(&format!(
            "{} visible window(s); sending WM_CLOSE",
            find.found.len()
        ));
        for hwnd in &find.found {
            // SAFETY: only queues a message for a window of the browser we started.
            unsafe {
                let _ = PostMessageW(Some(*hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        let exited = live.process().wait(Duration::from_secs(10));
        report(&format!("exited within 10 s of WM_CLOSE: {exited}"));
        assert!(live.finish(), "the temporary profile is still there");
        assert!(exited, "the browser kept running after its window closed");
    }
}
