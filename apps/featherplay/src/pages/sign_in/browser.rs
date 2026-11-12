//! "Use Microsoft Edge" (or Google Chrome, Brave or Vivaldi: `featherplay_platform::browsers`):
//! Google's sign-in in a real browser window (its own address bar, padlock, passkeys and
//! Windows Hello), for people who'd rather not type their password into a window of the app. The WebView2
//! window (`webview`) stays the default.
//!
//! Which browser: the page offers one of those found on this PC (`offers`) and starts the
//! sign-in with it (`start`).
//!
//! How it goes, on its own thread:
//! 1. Find the browser's exe (`browsers`: App Paths, then where its installer puts it).
//! 2. Make a temporary profile, `{cache}/signin-{pid}-{random}` (`profile`). The user's own
//!    browser profile is never touched, and no browser's cookie files are read.
//! 3. Start the browser on it with `--remote-debugging-pipe`, on Google's sign-in page,
//!    inside a kill-on-close job object (`browsers::launch`, which documents how the pipe
//!    reaches Chromium). A pipe only: never `--remote-debugging-port`, so no other program can
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
use featherplay_platform::browsers;
pub use featherplay_platform::browsers::{Browser, EDGE, by_id};

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
            let run = AssertUnwindSafe(|| run_on(&browsers::ThisPc, browser, &cache, &flag));
            let outcome = std::panic::catch_unwind(run)
                .unwrap_or_else(|_| Outcome::Lost("the sign-in thread panicked".into()));
            done(outcome)
        })
        .map(|_| cancel)
        .map_err(|e| format!("browser sign-in thread: {e}"))
}

/// A known browser that's installed on this PC, as the sign-in page offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    pub browser: &'static Browser,
    /// A policy on this PC turns off its DevTools pipe, so the sign-in can't use it.
    pub blocked: bool,
}

/// The known browsers installed on this PC, in the order the page lists them, and which
/// one it offers (`None` with none found): the one used last (`remembered`, its id), else
/// Windows' default browser, else the first; a blocked one only when every one is blocked.
/// Read from the registry and the disk each time (only read, never written).
pub fn offers(remembered: Option<&str>) -> (Vec<Offer>, Option<usize>) {
    use browsers::Lookup;
    let pc = browsers::ThisPc;
    let found = browsers::found(&pc);
    // The default browser only matters between two or more.
    let default = (found.len() > 1).then(|| pc.https_handler()).flatten();
    let pick = browsers::pick(&found, remembered, default.as_deref());
    let offers = found
        .iter()
        .map(|f| Offer {
            browser: f.browser,
            blocked: f.blocked,
        })
        .collect();
    (offers, pick)
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

impl Process for browsers::Running {
    fn exited(&self) -> bool {
        browsers::Running::exited(self)
    }
    fn wait(&self, limit: Duration) -> bool {
        browsers::Running::wait(self, limit)
    }
    fn kill(&self) {
        browsers::Running::kill(self)
    }
}

/// One sign-in in `browser`, with what it reads of the PC given.
fn run_on(
    pc: &impl browsers::Lookup,
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
    if let Some(prefs) = browser.preferences
        && let Err(e) = profile::seed(&profile.0, prefs)
    {
        // The browser still works with its own defaults.
        log!("browser sign-in: the profile's preferences: {}", e.kind());
    }
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
    let (process, pipes) = match browsers::launch(exe, &args) {
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
mod tests;
