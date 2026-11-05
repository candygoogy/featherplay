//! "Sign in with Microsoft Edge": Google's sign-in in a real Edge window (its own address
//! bar, padlock, passkeys and Windows Hello), for people who'd rather not type their
//! password into a window of the app. The WebView2 window (`webview`) stays the default.
//!
//! How it goes, on its own thread:
//! 1. Find `msedge.exe` (`browser`: App Paths, then Program Files).
//! 2. Make a temporary profile, `{cache}/signin-{pid}-{random}` (`profile`). The user's own
//!    Edge profile is never touched, and no browser's cookie files are read.
//! 3. Start Edge on it with `--remote-debugging-pipe`, on Google's sign-in page, inside a
//!    kill-on-close job object (`launch`, which documents how the pipe reaches Chromium).
//!    A pipe only: never `--remote-debugging-port`, so no other program can connect. No
//!    switch that marks the browser as automated.
//! 4. About once a second, ask for the profile's cookies (`cdp`: the app can send only
//!    `Storage.getCookies` and `Browser.close`).
//! 5. Signed in = the YouTube session cookies (`session::signed_in`, the rule the WebView2
//!    path and the controller use). Then wait 3 s so music.youtube.com can set its own
//!    cookies, read once more, and build the header both sign-ins build
//!    (`session::music_cookies`); every other site's cookies are dropped.
//! 6. `Browser.close`, wait up to 5 s for Edge to exit (then end the job), delete the
//!    profile, and hand the header over, as the WebView2 window does.
//!
//! It ends early, ending the job and deleting the profile, when Edge's process exits (the
//! user closed it), after 10 minutes without a sign-in, when the pipe breaks, when Edge
//! doesn't answer within 15 s (a policy may block DevTools), or on Cancel. Profiles a crash
//! left behind are deleted at the next start (`sweep`).
//!
//! "Switch Google account" stays on the WebView2 window: it relies on that window's
//! profile remembering earlier logins for Google's account chooser, and this profile is
//! thrown away after every sign-in.
//!
//! Cookie values are never logged, printed or written here.

mod browser;
mod cdp;
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
use browser::{Browser, EDGE};
use cdp::{Client, Method, Reply};

/// How an Edge sign-in ended.
#[derive(PartialEq)]
pub enum Outcome {
    /// The `cookie:` header for music.youtube.com, with a session cookie in it.
    SignedIn(String),
    /// Edge isn't installed (or can't be found).
    NotFound,
    /// Edge couldn't be started, or exited before it answered.
    DidntStart(String),
    /// Edge never answered: a policy may block DevTools.
    Blocked,
    /// Edge's `RemoteDebuggingAllowed` policy is off, so Edge wasn't started.
    PolicyOff,
    /// The user closed Edge before signing in.
    Closed,
    /// No sign-in within the time limit.
    TimedOut,
    /// The pipe broke, or Edge stopped answering.
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

/// Ends a running Edge sign-in.
#[derive(Clone)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Starts an Edge sign-in on its own thread, with its profile under `cache`; `done` runs on
/// that thread once Edge is gone and the profile deleted, always (a panic on the way ends
/// as `Lost`, after the guards have ended Edge and deleted the profile).
pub fn start(
    cache: PathBuf,
    done: impl FnOnce(Outcome) + Send + 'static,
) -> Result<Cancel, String> {
    let cancel = Cancel(Arc::new(AtomicBool::new(false)));
    let flag = cancel.0.clone();
    std::thread::Builder::new()
        .name("featherplay-edge-signin".into())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| run(&cache, &flag)))
                .unwrap_or_else(|_| Outcome::Lost("the sign-in thread panicked".into()));
            done(outcome)
        })
        .map(|_| cancel)
        .map_err(|e| format!("Edge sign-in thread: {e}"))
}

/// A policy on this PC turns off the DevTools pipe, so the Edge sign-in can't work. Read
/// from the registry each time (only read, never written).
pub fn pipe_blocked() -> bool {
    EDGE.pipe_blocked(&browser::ThisPc)
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
    /// For any reply; the first one missing means Edge doesn't allow the pipe.
    reply: Duration,
    /// For the whole sign-in.
    timeout: Duration,
    /// For Edge to exit after `Browser.close`.
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

/// 10 minutes. A debug build takes `FEATHERPLAY_EDGE_TIMEOUT_SECS` instead, to try the
/// timeout by hand.
fn timeout() -> Duration {
    let dev = cfg!(debug_assertions)
        .then(|| std::env::var("FEATHERPLAY_EDGE_TIMEOUT_SECS").ok())
        .flatten()
        .and_then(|s| s.trim().parse().ok());
    Duration::from_secs(dev.unwrap_or(10 * 60))
}

/// What the sign-in watches besides the pipe: Edge's own process.
trait Process {
    fn exited(&self) -> bool;
    fn wait(&self, limit: Duration) -> bool;
    /// Ends it and everything it started.
    fn kill(&self);
}

impl Process for launch::Edge {
    fn exited(&self) -> bool {
        launch::Edge::exited(self)
    }
    fn wait(&self, limit: Duration) -> bool {
        launch::Edge::wait(self, limit)
    }
    fn kill(&self) {
        launch::Edge::kill(self)
    }
}

fn run(cache: &Path, cancel: &AtomicBool) -> Outcome {
    run_on(&browser::ThisPc, cache, cancel)
}

/// `run`, with what it reads of the PC given.
fn run_on(pc: &impl browser::Lookup, cache: &Path, cancel: &AtomicBool) -> Outcome {
    let Some(exe) = EDGE.find(pc) else {
        log!("edge sign-in: {} not found", EDGE.name);
        return Outcome::NotFound;
    };
    // Edge would open and never answer: don't start it at all.
    if EDGE.pipe_blocked(pc) {
        log!("edge sign-in: Edge's RemoteDebuggingAllowed policy is off");
        return Outcome::PolicyOff;
    }
    let profile = match profile::create(cache) {
        Ok(p) => TempProfile(p),
        Err(e) => return Outcome::DidntStart(format!("temporary profile: {}", e.kind())),
    };
    log!(
        "edge sign-in: temporary profile {}",
        display_name(&profile.0)
    );
    // Edge (its job) is gone when this returns; dropping `profile` deletes the folder.
    launch_and_watch(&exe, &profile.0, cancel)
}

/// Deletes the temporary profile when dropped, on every way out (a panic too).
struct TempProfile(PathBuf);

impl Drop for TempProfile {
    fn drop(&mut self) {
        if profile::remove(&self.0, 20, Duration::from_millis(250)) {
            log!("edge sign-in: temporary profile deleted");
        } else {
            log!("edge sign-in: the temporary profile is still there; the next start deletes it");
        }
    }
}

fn display_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn launch_and_watch(exe: &Path, profile: &Path, cancel: &AtomicBool) -> Outcome {
    let args = args(&EDGE, profile, session::SIGN_IN_URL);
    let (edge, pipes) = match launch::launch(exe, &args) {
        Ok(started) => started,
        Err(e) => {
            log!("edge sign-in: Edge didn't start: {e}");
            return Outcome::DidntStart(e.to_string());
        }
    };
    log!("edge sign-in: Edge started (pid {})", edge.pid);
    let replies = match cdp::read_replies(pipes.from_browser) {
        Ok(rx) => rx,
        Err(e) => {
            edge.kill();
            return Outcome::DidntStart(format!("pipe reader: {e}"));
        }
    };
    let mut client = Client::new(pipes.to_browser);
    let outcome = watch(
        &edge,
        &mut client,
        &replies,
        cancel,
        &Limits::real(),
        wall_now,
    );
    drop(client);
    // Closing the job ends whatever helper is still running.
    drop(edge);
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

/// Reads the cookies until the session is there and settled, then closes Edge; or ends
/// Edge when the sign-in ends any other way. `wall` is the clock cookies expire by.
fn watch<W: Write>(
    edge: &impl Process,
    client: &mut Client<W>,
    replies: &Receiver<Reply>,
    cancel: &AtomicBool,
    limits: &Limits,
    wall: impl Fn() -> f64,
) -> Outcome {
    let outcome = poll(edge, client, replies, cancel, limits, wall);
    match &outcome {
        Outcome::SignedIn(_) => {
            let asked = client.send(Method::CloseBrowser).is_ok();
            if asked && edge.wait(limits.close) {
                log!("edge sign-in: Edge closed");
            } else {
                log!("edge sign-in: Edge didn't close; ending it");
                edge.kill();
            }
        }
        _ => edge.kill(),
    }
    outcome
}

/// Why waiting stopped.
enum Stop {
    Cancelled,
    /// Edge's process exited.
    Exited,
    /// The pipe closed or broke.
    PipeClosed,
    /// No reply in time.
    NoReply,
}

fn poll<W: Write>(
    edge: &impl Process,
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
            .and_then(|id| await_reply(id, replies, edge, cancel, limits.reply));
        let result = match reply {
            Ok(r) => r,
            Err(stop) => return settle.or(stopped(stop, answered, started.elapsed(), edge)),
        };
        if !answered {
            log!("edge sign-in: Edge answers on the pipe");
            answered = true;
        }
        let cookies = match result.and_then(cdp::cookies) {
            Ok(c) => c,
            Err(e) => {
                log!("edge sign-in: reading the cookies failed: {e}");
                return Outcome::Lost(e);
            }
        };
        let next = match settle.see(Instant::now(), &cookies, wall()) {
            Seen::Ready(header) => return Outcome::SignedIn(header),
            Seen::Settling(left) => left,
            Seen::NotYet if started.elapsed() >= limits.timeout => {
                log!("edge sign-in: no sign-in in {} s", limits.timeout.as_secs());
                return Outcome::TimedOut;
            }
            Seen::NotYet => limits.poll,
        };
        if let Err(stop) = pause(next, replies, edge, cancel) {
            return settle.or(stopped(stop, answered, started.elapsed(), edge));
        }
    }
}

/// Edge exiting sooner than this after the start, before it answered, didn't start; later,
/// the user closed it (on a PC whose policy turns the pipe off, it never answers).
const EARLY_EXIT: Duration = Duration::from_secs(3);

/// How the sign-in ends on `stop`, `ran` after Edge started.
fn stopped(stop: Stop, answered: bool, ran: Duration, edge: &impl Process) -> Outcome {
    match stop {
        Stop::Cancelled => {
            log!("edge sign-in: cancelled");
            Outcome::Cancelled
        }
        Stop::Exited if !answered && ran < EARLY_EXIT => {
            log!("edge sign-in: Edge exited before it answered");
            Outcome::DidntStart("Edge exited before it answered".into())
        }
        Stop::Exited => {
            log!("edge sign-in: Edge was closed");
            Outcome::Closed
        }
        // The pipe closing alone doesn't prove Edge is gone: ask its process.
        Stop::PipeClosed if edge.wait(Duration::from_secs(1)) => {
            stopped(Stop::Exited, answered, ran, edge)
        }
        Stop::PipeClosed => {
            log!("edge sign-in: the pipe closed while Edge runs");
            Outcome::Lost("the pipe to Edge closed".into())
        }
        Stop::NoReply if !answered => {
            log!("edge sign-in: no answer on the pipe; a policy may block it");
            Outcome::Blocked
        }
        Stop::NoReply => {
            log!("edge sign-in: Edge stopped answering");
            Outcome::Lost("Edge stopped answering".into())
        }
    }
}

/// How often waits look at Cancel and Edge's process.
const SLICE: Duration = Duration::from_millis(100);

/// Waits for the reply to `id`, at most `within`.
fn await_reply(
    id: u64,
    replies: &Receiver<Reply>,
    edge: &impl Process,
    cancel: &AtomicBool,
    within: Duration,
) -> Result<Result<serde_json::Value, String>, Stop> {
    let deadline = Instant::now() + within;
    loop {
        check(edge, cancel)?;
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

/// Waits `span` between reads, watching Cancel, Edge's process and the pipe.
fn pause(
    span: Duration,
    replies: &Receiver<Reply>,
    edge: &impl Process,
    cancel: &AtomicBool,
) -> Result<(), Stop> {
    let until = Instant::now() + span;
    loop {
        check(edge, cancel)?;
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

fn check(edge: &impl Process, cancel: &AtomicBool) -> Result<(), Stop> {
    if cancel.load(Ordering::Relaxed) {
        Err(Stop::Cancelled)
    } else if edge.exited() {
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

    /// `outcome`, unless Edge was closed while the session settled: then the sign-in had
    /// already worked, and the last read's header counts.
    fn or(&mut self, outcome: Outcome) -> Outcome {
        match (outcome, self.last.take()) {
            (Outcome::Closed, Some(header)) => {
                log!("edge sign-in: Edge closed while settling; using the last read");
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
            log!("edge sign-in: session cookies present; settling");
            now
        });
        let waited = now.saturating_duration_since(since);
        if waited >= self.settle {
            log!("edge sign-in: signed in, {} cookies", music.len());
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

    impl browser::Lookup for PolicyOffPc {
        fn app_path(&self, _: browser::Hive, _: &str) -> Option<String> {
            Some(r"C:\Edge\msedge.exe".into())
        }
        fn env(&self, _: &str) -> Option<OsString> {
            None
        }
        fn is_file(&self, _: &Path) -> bool {
            true
        }
        fn dword(&self, hive: browser::Hive, _: &str, value: &str) -> Option<u32> {
            (hive == browser::Hive::LocalMachine && value == "RemoteDebuggingAllowed").then_some(0)
        }
    }

    #[test]
    fn with_the_policy_off_edge_isnt_started() {
        let home = std::env::var_os("FEATHERPLAY_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let cache = home.join(format!("test-policy-{}", std::process::id()));
        let out = run_on(&PolicyOffPc, &cache, &AtomicBool::new(false));
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

    /// Edge's process, as the tests drive it.
    #[derive(Default)]
    struct FakeEdge {
        exited: AtomicBool,
        killed: AtomicBool,
        /// `Browser.close` is answered but the process stays.
        ignores_close: AtomicBool,
    }

    impl Process for FakeEdge {
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
        edge: Arc<FakeEdge>,
        methods: Arc<Mutex<Vec<String>>>,
        client: Client<PipeWriter>,
        replies: Receiver<Reply>,
    }

    fn fake_browser(script: impl Fn(usize) -> Answer + Send + 'static) -> FakeBrowser {
        let (req_r, req_w) = std::io::pipe().unwrap();
        let (rep_r, rep_w) = std::io::pipe().unwrap();
        let edge = Arc::new(FakeEdge::default());
        let methods = Arc::new(Mutex::new(Vec::new()));
        let (e, m) = (edge.clone(), methods.clone());
        std::thread::spawn(move || serve(req_r, rep_w, &e, &m, script));
        FakeBrowser {
            edge,
            methods,
            client: Client::new(req_w),
            replies: cdp::read_replies(rep_r).unwrap(),
        }
    }

    fn serve(
        from: PipeReader,
        mut to: PipeWriter,
        edge: &FakeEdge,
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
                if !edge.ignores_close.load(Ordering::Relaxed) {
                    edge.exited.store(true, Ordering::Relaxed);
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
                Answer::ExitProcess => edge.exited.store(true, Ordering::Relaxed),
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
            &*fb.edge,
            &mut fb.client,
            &fb.replies,
            cancel,
            limits,
            || NOW,
        )
    }

    #[test]
    fn a_settled_session_gives_the_music_header_and_closes_edge() {
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
            !fb.edge.killed.load(Ordering::Relaxed),
            "it closed by itself"
        );
    }

    #[test]
    fn edge_that_stays_after_browser_close_is_ended() {
        let mut fb = fake_browser(|_| Answer::Cookies(signed_in_cookies()));
        fb.edge.ignores_close.store(true, Ordering::Relaxed);
        let limits = Limits {
            close: Duration::from_millis(100),
            ..quick()
        };
        let out = run_fake(&mut fb, &AtomicBool::new(false), &limits);
        assert!(matches!(out, Outcome::SignedIn(_)), "{out:?}");
        assert!(fb.edge.killed.load(Ordering::Relaxed));
    }

    #[test]
    fn closing_edge_while_the_session_settles_keeps_the_sign_in() {
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
    fn edge_exiting_at_once_didnt_start() {
        let mut fb = fake_browser(|_| Answer::ExitProcess);
        assert!(matches!(
            run_fake(&mut fb, &AtomicBool::new(false), &quick()),
            Outcome::DidntStart(_)
        ));
    }

    #[test]
    fn edge_closed_later_without_ever_answering_was_closed() {
        // The pipe turned off by policy: no answer, then the user closes the window.
        let edge = FakeEdge::default();
        edge.exited.store(true, Ordering::Relaxed);
        let ran = EARLY_EXIT + Duration::from_secs(5);
        assert_eq!(stopped(Stop::Exited, false, ran, &edge), Outcome::Closed);
        assert_eq!(
            stopped(Stop::PipeClosed, false, ran, &edge),
            Outcome::Closed
        );
    }

    #[test]
    fn the_pipe_closing_as_edge_exits_means_closed() {
        let edge = FakeEdge::default();
        edge.exited.store(true, Ordering::Relaxed);
        assert_eq!(
            stopped(Stop::PipeClosed, true, Duration::from_secs(30), &edge),
            Outcome::Closed
        );
    }

    #[test]
    fn the_debug_form_leaves_the_header_out() {
        let o = Outcome::SignedIn("SAPISID=secret".into());
        assert!(!format!("{o:?}").contains("secret"));
    }

    #[test]
    fn closing_edge_ends_the_sign_in() {
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
        assert!(fb.edge.killed.load(Ordering::Relaxed));
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
        assert!(fb.edge.killed.load(Ordering::Relaxed));
    }

    #[test]
    fn a_broken_pipe_while_edge_runs_means_lost() {
        let mut fb = fake_browser(|n| match n {
            1 => Answer::Cookies(vec![]),
            _ => Answer::ClosePipe,
        });
        assert!(matches!(
            run_fake(&mut fb, &AtomicBool::new(false), &quick()),
            Outcome::Lost(_)
        ));
        assert!(fb.edge.killed.load(Ordering::Relaxed));
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
        assert!(fb.edge.killed.load(Ordering::Relaxed));
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
        assert!(fb.edge.killed.load(Ordering::Relaxed));
    }

    #[allow(clippy::print_stderr, reason = "the live checks report what they saw")]
    fn report(what: &str) {
        eprintln!("edge: {what}");
    }

    /// A browser started on the pipe for a live check: Edge, or the Chromium-based exe in
    /// `FEATHERPLAY_PIPE_TEST_BROWSER` (for a PC whose Edge policy turns DevTools off). On
    /// about:blank, with a temporary profile under the test home. Dropping it ends the
    /// browser and deletes the profile.
    struct Live {
        edge: Option<launch::Edge>,
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
                .or_else(|| EDGE.find(&browser::ThisPc))
                .expect("Edge isn't installed");
            report(&format!("starting {}", exe.display()));
            let profile = profile::create(&cache).unwrap();
            let (edge, pipes) =
                launch::launch(&exe, &args(&EDGE, &profile, "about:blank")).unwrap();
            Self {
                edge: Some(edge),
                client: Client::new(pipes.to_browser),
                replies: cdp::read_replies(pipes.from_browser).unwrap(),
                profile,
            }
        }

        fn edge(&self) -> &launch::Edge {
            self.edge.as_ref().unwrap()
        }

        /// Asks for the cookies; the reply, if one comes within 30 s.
        fn cookies(&mut self) -> Option<Result<Vec<Cookie>, String>> {
            let id = self.client.send(Method::GetCookies).ok()?;
            let never = AtomicBool::new(false);
            let r = await_reply(
                id,
                &self.replies,
                self.edge(),
                &never,
                Duration::from_secs(30),
            );
            r.ok().map(|r| r.and_then(cdp::cookies))
        }

        /// Ends the browser and deletes the profile; says whether the profile is gone.
        fn finish(&mut self) -> bool {
            drop(self.edge.take());
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
        let exited = live.edge().wait(Duration::from_secs(10));
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
            pid: live.edge().pid,
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
        let exited = live.edge().wait(Duration::from_secs(10));
        report(&format!("exited within 10 s of WM_CLOSE: {exited}"));
        assert!(live.finish(), "the temporary profile is still there");
        assert!(exited, "the browser kept running after its window closed");
    }
}
