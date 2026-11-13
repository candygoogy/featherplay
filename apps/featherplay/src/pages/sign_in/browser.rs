//! "Use Microsoft Edge" (or Google Chrome, Brave or Vivaldi, and Chromium on Linux:
//! `featherplay_platform::browsers`): Google's sign-in in a real browser window (its own
//! address bar, padlock, passkeys and Windows Hello), for people who'd rather not type their
//! password into a window of the app. On Windows the WebView2 window (`webview`) stays the
//! default; Linux has no such window, so this is its way in, with the pasted cookie header.
//!
//! Which browser: the page offers one of those found on this PC (`offers`) and starts the
//! sign-in with it (`start`).
//!
//! How it goes, on its own thread:
//! 1. Find the browser's exe (`browsers`: on Windows App Paths, then where its installer puts
//!    it; on Linux `PATH`, then where its package puts it). A Snap or a Flatpak, or a policy
//!    that turns the pipe off, means it isn't started (`Outcome::Unusable`).
//! 2. Make a temporary profile, `{cache}/signin-{pid}-{random}` (`profile`). The user's own
//!    browser profile is never touched, and featherplay reads no browser's cookie files
//!    itself: the browser reports its cookies.
//!
//! Then, on Windows:
//! 3. Start the browser on it with `--remote-debugging-pipe`, on Google's sign-in page,
//!    inside a kill-on-close job object (`browsers::launch`, which documents how the pipe
//!    reaches Chromium). A pipe only: never `--remote-debugging-port`, so no other program can
//!    connect. No switch that marks the browser as automated.
//! 4. About once a second, ask for the profile's cookies (`cdp`: the app can send only
//!    `Storage.getCookies` and `Browser.close`). Until the browser first answers, the
//!    question goes again every 5 s, in case one sent before it was up is lost (`ask`).
//! 5. Signed in = the YouTube session cookies (`session::signed_in`, the rule the WebView2
//!    path and the controller use). Then wait 3 s so music.youtube.com can set its own
//!    cookies, read once more, and build the header both sign-ins build
//!    (`session::music_cookies`); every other site's cookies are dropped.
//! 6. `Browser.close`, wait up to 5 s for the browser to exit (then end it), delete
//!    the profile, and hand the header over, as the WebView2 window does.
//!
//! It ends early, ending the job and deleting the profile, when the browser's process exits
//! (the user closed it), after 10 minutes without a sign-in, when the pipe breaks, when the
//! browser doesn't answer within 15 s (a policy may block DevTools), or on Cancel.
//!
//! On Linux the pipe and Google's pages are never open at once: with the pipe, Chromium tells
//! every page it's automated (`navigator.webdriver`), and Google's sign-in refuses such a
//! browser. So:
//! 3. Start the browser plainly on the profile, on Google's sign-in page, in a process group
//!    of its own (`browsers::launch_plain`): no pipe, no switch about automation. The user
//!    signs in and closes it; closing it writes the profile's cookies to its folder. Left
//!    open for 15 minutes, it's asked to quit as closing it would (`Running::quit`), and
//!    ended 5 s later if it's still there; the read follows all the same, so a sign-in that
//!    happened isn't lost.
//! 4. Once its process has exited, start it again on that profile, headless and on a blank
//!    page, with the pipe (`browsers::launch`, `browsers::HEADLESS`), so no Google page is
//!    open while the pipe is. Ask for the cookies once (again every 5 s until the first
//!    answer, which gets 60 s, see `first_reply`), then `Browser.close`, as above.
//! 5. Signed in = the session cookies, and the same header, as above; none means the browser
//!    was closed before the sign-in finished (or, at the time limit, `TimedOut`). Then the
//!    profile is deleted.
//!
//! It ends early, ending the browser and deleting the profile, when the browser exits within
//! 3 s (it didn't start) or on Cancel. Profiles a crash left behind are deleted at the next
//! start (`sweep`).
//!
//! "Switch Google account" stays on the WebView2 window: it relies on that window's
//! profile remembering earlier logins for Google's account chooser, and this profile is
//! thrown away after every sign-in. On Linux it opens the Sign in page instead.
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
pub use featherplay_platform::browsers::{ALL, Block, Browser, by_id};

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
    /// The browser can't be used (its `RemoteDebuggingAllowed` policy is off; on Linux, it's
    /// a Snap or a Flatpak), so it wasn't started.
    Unusable(Block),
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
            Outcome::Unusable(why) => write!(f, "Unusable({why:?})"),
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
    /// Why the sign-in can't use it (a policy on this PC turns off its DevTools pipe; on
    /// Linux, a Snap's or a Flatpak's sandbox), if it can't.
    pub blocked: Option<Block>,
}

/// The known browsers installed on this PC, in the order the page lists them, and which
/// one it offers (`None` with none found): the one used last (`remembered`, its id), else
/// the system's default browser, else the first; a blocked one only when every one is
/// blocked. Read from the registry (Linux: the policy files) and the disk each time (only
/// read, never written).
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
    /// Between two cookie reads (Windows, which reads while the user signs in).
    #[cfg(windows)]
    poll: Duration,
    /// After the session cookies appear, before the last read.
    #[cfg(windows)]
    settle: Duration,
    /// For the first reply, which comes once the browser has started; none means it doesn't
    /// allow the pipe.
    first_reply: Duration,
    /// Until the first reply, how often the cookies are asked for again (`ask`).
    resend: Duration,
    /// For any later reply.
    #[cfg(windows)]
    reply: Duration,
    /// For the whole sign-in (on Linux, for the sign-in window).
    timeout: Duration,
    /// For the browser to exit after `Browser.close`.
    close: Duration,
}

impl Limits {
    fn real() -> Self {
        Self {
            #[cfg(windows)]
            poll: Duration::from_secs(1),
            #[cfg(windows)]
            settle: Duration::from_secs(3),
            first_reply: first_reply(),
            resend: Duration::from_secs(5),
            #[cfg(windows)]
            reply: Duration::from_secs(15),
            timeout: timeout(),
            close: Duration::from_secs(5),
        }
    }
}

/// 15 s on Windows. 60 s on Linux, where a browser's first start after boot, with none of it
/// in the page cache yet, can take longer than 15 s; the commands it gets before it's up
/// usually wait in the pipe (Chrome 156 started cold in WSL answered all of them at once,
/// after 3 to 7 s), and the cookies are asked for again meanwhile in case one is lost (`ask`).
/// On a fresh GitHub runner (2026-11-12) Edge 157's first start gave no answer within 30 s,
/// and the next one answered after 11 s (1 s in WSL); Chromium 156 gave none in 60 s, and
/// answered in 0.4 s when started again. Linux reads the `RemoteDebuggingAllowed` policy
/// before the start (`Browser::blocked`), so a missing answer there is rarely a block. There
/// it's the headless read's first answer, which comes right after the sign-in window closed,
/// so the browser is rarely cold by then.
fn first_reply() -> Duration {
    Duration::from_secs(if cfg!(target_os = "linux") { 60 } else { 15 })
}

/// 10 minutes on Windows. 15 on Linux, where the time runs until the user closes the window
/// rather than until the session shows, and a slow 2-step verification shouldn't run into
/// it. A debug build takes `FEATHERPLAY_BROWSER_TIMEOUT_SECS` instead, to try the timeout by
/// hand.
fn timeout() -> Duration {
    let dev = cfg!(debug_assertions)
        .then(|| std::env::var("FEATHERPLAY_BROWSER_TIMEOUT_SECS").ok())
        .flatten()
        .and_then(|s| s.trim().parse().ok());
    let minutes = if cfg!(target_os = "linux") { 15 } else { 10 };
    Duration::from_secs(dev.unwrap_or(minutes * 60))
}

/// What the sign-in watches besides the pipe: the browser's own process.
trait Process {
    fn exited(&self) -> bool;
    fn wait(&self, limit: Duration) -> bool;
    /// Ends it and everything it started.
    fn kill(&self);
    /// Asks it to quit, as closing its window would.
    #[cfg(target_os = "linux")]
    fn quit(&self);
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
    #[cfg(target_os = "linux")]
    fn quit(&self) {
        browsers::Running::quit(self)
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
    if let Some(why) = browser.blocked(pc) {
        match why {
            Block::Policy => log!(
                "browser sign-in: {}'s RemoteDebuggingAllowed policy is off",
                browser.name
            ),
            Block::Snap | Block::Flatpak => log!(
                "browser sign-in: {} is a {why:?}, whose sandbox keeps the sign-in out",
                browser.name
            ),
        }
        return Outcome::Unusable(why);
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

/// Windows: the browser on Google's sign-in page with the pipe, its cookies read until the
/// session is there.
#[cfg(windows)]
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
    // Closing the job (Linux: ending the process group) ends whatever helper is still running.
    drop(process);
    outcome
}

/// Windows: the browser's arguments: the temporary profile, the pipe, its own switches, the
/// page.
#[cfg(windows)]
fn args(b: &Browser, profile: &Path, url: &str) -> Vec<OsString> {
    let mut out = vec![user_data_dir(profile), "--remote-debugging-pipe".into()];
    out.extend(b.switches.iter().map(OsString::from));
    out.push(url.into());
    out
}

/// Linux: the sign-in window with no pipe, then, once the user has closed it, the headless
/// read of its profile.
#[cfg(target_os = "linux")]
fn launch_and_watch(browser: &Browser, exe: &Path, profile: &Path, cancel: &AtomicBool) -> Outcome {
    let limits = Limits::real();
    let url = session::SIGN_IN_URL;
    let ended = match sign_in_window(browser, exe, profile, url, cancel, &limits) {
        Ok(ended) => ended,
        Err(outcome) => return outcome,
    };
    match read_profile(browser, exe, profile, cancel, &limits) {
        Ok(cookies) => session_or(&cookies, wall_now(), ended),
        Err(outcome) => outcome,
    }
}

/// How Linux's sign-in window went away.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    /// The user closed it.
    Closed,
    /// featherplay closed it at the time limit.
    TimeLimit,
}

/// Linux's sign-in window: the temporary profile, its own switches, the page. No pipe, and
/// nothing that marks the browser as automated or hides that mark.
#[cfg(target_os = "linux")]
fn window_args(b: &Browser, profile: &Path, url: &str) -> Vec<OsString> {
    let mut out = vec![user_data_dir(profile)];
    out.extend(b.switches.iter().map(OsString::from));
    out.push(url.into());
    out
}

/// Linux's read of that profile: the profile, the pipe, headless, its own switches (the same
/// cookie store), a blank page.
#[cfg(target_os = "linux")]
fn read_args(b: &Browser, profile: &Path) -> Vec<OsString> {
    let mut out = vec![
        user_data_dir(profile),
        "--remote-debugging-pipe".into(),
        browsers::HEADLESS.into(),
    ];
    out.extend(b.switches.iter().map(OsString::from));
    out.push("about:blank".into());
    out
}

fn user_data_dir(profile: &Path) -> OsString {
    let mut dir = OsString::from("--user-data-dir=");
    dir.push(profile);
    dir
}

/// Starts `browser` plainly on `url` and waits for it to go (`wait_closed`): how it went, or
/// the outcome when the sign-in ends here.
#[cfg(target_os = "linux")]
fn sign_in_window(
    browser: &Browser,
    exe: &Path,
    profile: &Path,
    url: &str,
    cancel: &AtomicBool,
    limits: &Limits,
) -> Result<Ended, Outcome> {
    let process =
        browsers::launch_plain(exe, &window_args(browser, profile, url)).map_err(|e| {
            log!("browser sign-in: {} didn't start: {e}", browser.name);
            Outcome::DidntStart(e.to_string())
        })?;
    log!(
        "browser sign-in: {} started (pid {}), no pipe",
        browser.name,
        process.pid
    );
    let closed = wait_closed(&process, cancel, limits);
    // Ending its group ends any helper that's left, before the read starts on the profile.
    drop(process);
    closed
}

/// Waits for the browser's process to exit: the user closed it. At `limits.timeout` it's asked
/// to quit, as closing it would, so it writes its cookies, and ended if it hasn't within
/// `limits.close`; either way its profile is read next. Ended on Cancel; one that exits within
/// `EARLY_EXIT` didn't start.
#[cfg(target_os = "linux")]
fn wait_closed(
    process: &impl Process,
    cancel: &AtomicBool,
    limits: &Limits,
) -> Result<Ended, Outcome> {
    let started = Instant::now();
    loop {
        match check(process, cancel) {
            Ok(()) if started.elapsed() >= limits.timeout => {
                log!(
                    "browser sign-in: still open after {} s; closing it",
                    limits.timeout.as_secs()
                );
                process.quit();
                if !process.wait(limits.close) {
                    log!("browser sign-in: the browser didn't quit; ending it");
                    process.kill();
                }
                return Ok(Ended::TimeLimit);
            }
            Ok(()) => std::thread::sleep(SLICE),
            Err(Stop::Cancelled) => {
                log!("browser sign-in: cancelled");
                process.kill();
                return Err(Outcome::Cancelled);
            }
            Err(_) if started.elapsed() < EARLY_EXIT => {
                log!("browser sign-in: the browser exited at once");
                return Err(Outcome::DidntStart("the browser exited at once".into()));
            }
            Err(_) => {
                log!("browser sign-in: the browser was closed; reading its profile");
                return Ok(Ended::Closed);
            }
        }
    }
}

/// Starts `browser` headless on the closed sign-in window's profile and reads its cookies
/// (`read_and_close`).
#[cfg(target_os = "linux")]
fn read_profile(
    browser: &Browser,
    exe: &Path,
    profile: &Path,
    cancel: &AtomicBool,
    limits: &Limits,
) -> Result<Vec<Cookie>, Outcome> {
    let (process, pipes) = browsers::launch(exe, &read_args(browser, profile)).map_err(|e| {
        log!(
            "browser sign-in: {} didn't start headless: {e}",
            browser.name
        );
        Outcome::DidntStart(e.to_string())
    })?;
    log!(
        "browser sign-in: {} started headless to read the profile (pid {})",
        browser.name,
        process.pid
    );
    let replies = match cdp::read_replies(pipes.from_browser) {
        Ok(rx) => rx,
        Err(e) => {
            process.kill();
            return Err(Outcome::DidntStart(format!("pipe reader: {e}")));
        }
    };
    let mut client = Client::new(pipes.to_browser);
    let read = read_and_close(&process, &mut client, &replies, cancel, limits);
    drop(client);
    drop(process);
    read
}

/// The headless browser's two commands: `Storage.getCookies` (sent again every
/// `limits.resend` until the first answer), then `Browser.close`; ended if it doesn't exit
/// within `limits.close`, or when the read failed.
#[cfg(target_os = "linux")]
fn read_and_close<W: Write>(
    process: &impl Process,
    client: &mut Client<W>,
    replies: &Receiver<Reply>,
    cancel: &AtomicBool,
    limits: &Limits,
) -> Result<Vec<Cookie>, Outcome> {
    let resend = Some(limits.resend);
    let read = match ask(client, replies, process, cancel, limits.first_reply, resend) {
        Ok(result) => result.and_then(cdp::cookies).map_err(|e| {
            log!("browser sign-in: reading the cookies failed: {e}");
            Outcome::Lost(e)
        }),
        Err(Stop::Cancelled) => {
            log!("browser sign-in: cancelled");
            Err(Outcome::Cancelled)
        }
        Err(Stop::NoReply) => {
            log!("browser sign-in: no answer on the pipe; a policy may block it");
            Err(Outcome::Blocked)
        }
        Err(Stop::Exited | Stop::PipeClosed) => {
            log!("browser sign-in: the headless browser exited before it answered");
            Err(Outcome::DidntStart(
                "the browser exited before it answered".into(),
            ))
        }
    };
    if read.is_ok() && client.send(Method::CloseBrowser).is_ok() && process.wait(limits.close) {
        log!("browser sign-in: the browser closed");
    } else {
        process.kill();
    }
    read
}

/// The sign-in, from the cookies the closed browser kept: the header with its session, else
/// `Closed` (it was closed before the sign-in finished) or, at the time limit, `TimedOut`.
/// `wall` is the clock cookies expire by.
#[cfg(target_os = "linux")]
fn session_or(cookies: &[Cookie], wall: f64, ended: Ended) -> Outcome {
    let music = session::music_cookies(cookies, wall);
    if session::signed_in(&music) {
        log!("browser sign-in: signed in, {} cookies", music.len());
        Outcome::SignedIn(session::header(&music))
    } else {
        log!("browser sign-in: no session in the profile");
        match ended {
            Ended::Closed => Outcome::Closed,
            Ended::TimeLimit => Outcome::TimedOut,
        }
    }
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
#[cfg(windows)]
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

#[cfg(windows)]
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
        let reply = if answered {
            ask(client, replies, process, cancel, limits.reply, None)
        } else {
            let resend = Some(limits.resend);
            ask(client, replies, process, cancel, limits.first_reply, resend)
        };
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
#[cfg(windows)]
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

/// Sends `Storage.getCookies` and waits for its reply, at most `within`. With `resend`, sends
/// it again each time that long passes without a reply, in case a browser that isn't up yet
/// lost it (it's only read, so asking twice changes nothing): a reply to any of this wait's
/// commands counts, and one to an earlier wait's is skipped.
fn ask<W: Write>(
    client: &mut Client<W>,
    replies: &Receiver<Reply>,
    process: &impl Process,
    cancel: &AtomicBool,
    within: Duration,
    resend: Option<Duration>,
) -> Result<Result<serde_json::Value, String>, Stop> {
    let mut send = || {
        client
            .send(Method::GetCookies)
            .map_err(|_| Stop::PipeClosed)
    };
    let first = send()?;
    let deadline = Instant::now() + within;
    let mut again = Instant::now() + resend.unwrap_or_default();
    loop {
        check(process, cancel)?;
        let now = Instant::now();
        if now >= deadline {
            return Err(Stop::NoReply);
        }
        if let Some(every) = resend
            && now >= again
        {
            let id = send()?;
            log!("browser sign-in: no answer yet; asked again (#{id})");
            again = now + every;
        }
        match replies.recv_timeout(deadline.saturating_duration_since(now).min(SLICE)) {
            Ok(r) if r.id >= first => return Ok(r.result),
            // A late reply to an earlier wait's command.
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err(Stop::PipeClosed),
        }
    }
}

/// Waits `span` between reads, watching Cancel, the browser's process and the pipe.
#[cfg(windows)]
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
#[cfg(windows)]
struct Settle {
    settle: Duration,
    since: Option<Instant>,
    /// The header from the latest read with a session in it, while settling.
    last: Option<String>,
}

#[cfg(windows)]
#[derive(Debug, PartialEq)]
enum Seen {
    /// No session yet.
    NotYet,
    /// Signed in; read again after this.
    Settling(Duration),
    /// Signed in and settled: the header.
    Ready(String),
}

#[cfg(windows)]
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
