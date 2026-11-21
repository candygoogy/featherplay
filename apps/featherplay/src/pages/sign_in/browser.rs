//! "Use Microsoft Edge" (or Google Chrome, Brave or Vivaldi, and Chromium and Firefox on
//! Linux: `featherplay_platform::browsers`): Google's sign-in in a real browser window (its
//! own address bar, padlock, passkeys and Windows Hello), for people who'd rather not type
//! their password into a window of the app. On Windows the WebView2 window (`webview`) stays
//! the default; Linux has no such window, so this is its way in, with the pasted cookie
//! header.
//!
//! Which browser: the page offers one of those found on this PC (`offers`) and starts the
//! sign-in with it (`start`).
//!
//! How it goes, on its own thread:
//! 1. Find the browser's exe (`browsers`: on Windows App Paths, then where its installer puts
//!    it; on Linux `PATH`, then where its package puts it, then its Snap or Flatpak). On
//!    Windows a policy that turns the pipe off means it isn't started (`Outcome::Unusable`).
//! 2. Make a temporary profile (`profile`): `{cache}/signin-{pid}-{random}`, or, for a Snap or
//!    a Flatpak, `featherplay-signin-{pid}-{random}` in the folder its sandbox lets it reach
//!    (`Sandbox::profiles`). The user's own browser profile is never touched.
//! 3. Start the browser plainly on the profile, on Google's sign-in page, in a kill-on-close
//!    job (Linux: a process group of its own) (`browsers::launch_plain`): no pipe, no switch
//!    about automation, so Google's sign-in sees the browser as the user would start it
//!    (with the pipe, Chromium tells every page it's automated, `navigator.webdriver`, and
//!    Google may refuse such a browser: "This browser or app may not be secure"). The user
//!    signs in and closes it; closing it writes the profile's cookies to its folder. It's
//!    closed once its process has exited and, on Linux, the profile's lock is let go
//!    (`browsers::ProfileLock`), which a launcher that hands over and exits doesn't. Left
//!    open for 15 minutes, it's asked to quit as closing it would (`Running::quit`), and
//!    ended 5 s later if it's still there; the read follows all the same, so a sign-in that
//!    happened isn't lost.
//! 4. Read the cookies it kept. Windows: start it again on that profile, headless and on a
//!    blank page, with the pipe (`browsers::launch`, which documents how the pipe reaches
//!    Chromium; `browsers::HEADLESS`), so no Google page is open while the pipe is. A pipe
//!    only: never `--remote-debugging-port`, so no other program can connect. Ask for the
//!    cookies once (again every 5 s until the first answer, see `first_reply`), then
//!    `Browser.close` (the app can send only those two, `cdp`), wait up to 5 s for it to
//!    exit, then end it. Linux: read the profile's cookie database from disk
//!    (`browsers::cookies`), with nothing started.
//! 5. Signed in = the YouTube session cookies (`session::signed_in`, the rule the WebView2
//!    path and the controller use), and the header both sign-ins build from them
//!    (`session::music_cookies`; every other site's cookies are dropped). None means the
//!    browser was closed before the sign-in finished (or, at the time limit, `TimedOut`).
//!    Then the profile is deleted and the header handed over, as the WebView2 window does.
//!
//! It ends early, ending the browser and deleting the profile, when the browser exits within
//! 3 s (it didn't start), when the read gets no answer (a policy may block DevTools) or the
//! pipe breaks, when the cookies can't be read (on Linux, a Chromium that kept them under the
//! keyring's key: `Outcome::Blocked`), or on Cancel. Profiles a crash left behind are deleted
//! at the next start (`sweep`).
//!
//! "Switch Google account" stays on the WebView2 window: it relies on that window's
//! profile remembering earlier logins for Google's account chooser, and this profile is
//! thrown away after every sign-in. On Linux it opens the Sign in page instead.
//!
//! Cookie values are never logged, printed or written here.

#[cfg(windows)]
mod cdp;
mod profile;

use std::ffi::OsString;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
#[cfg(windows)]
use {
    cdp::{Client, Method, Reply},
    std::io::Write,
    std::sync::mpsc::{Receiver, RecvTimeoutError},
};

use super::session::{self, Cookie};
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
    /// The browser never answered: a policy may block DevTools. On Linux: it kept the
    /// cookies under the keyring's key, which featherplay never asks for.
    Blocked,
    /// The browser can't be used (its `RemoteDebuggingAllowed` policy is off), so it wasn't
    /// started.
    #[cfg(windows)]
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
            #[cfg(windows)]
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
    /// Why the sign-in can't use it (a policy on this PC turns off its DevTools pipe), if it
    /// can't. Never on Linux.
    pub blocked: Option<Block>,
}

/// The known browsers installed on this PC, in the order the page lists them, which one it
/// offers (`None` with none found): the one used last (`remembered`, its id), else the
/// system's default browser, else the first; a blocked one only when every one is blocked.
/// And whether the system's default browser is one the sign-in can't open, while there's one
/// to offer instead and none has been picked or used yet. Read from the registry (Windows)
/// and the disk each time (only read, never written).
pub fn offers(remembered: Option<&str>) -> (Vec<Offer>, Option<usize>, bool) {
    use browsers::Lookup;
    let pc = browsers::ThisPc;
    let found = browsers::found(&pc);
    // The default browser only matters between two or more.
    let default = (found.len() > 1).then(|| pc.https_handler()).flatten();
    let pick = browsers::pick(&found, remembered, default.as_deref());
    let default_unusable =
        remembered.is_none() && !found.is_empty() && browsers::default_elsewhere(&pc, &found);
    let offers = found
        .iter()
        .map(|f| Offer {
            browser: f.browser,
            blocked: f.blocked,
        })
        .collect();
    (offers, pick, default_unusable)
}

/// Deletes the temporary profiles that earlier runs left in `cache` (a crash, a kill), and on
/// Linux in the folders of the Snaps and Flatpaks.
pub fn sweep(cache: &Path) -> usize {
    profile::sweep(cache)
}

/// The waits and limits of a sign-in.
#[derive(Clone, Debug)]
struct Limits {
    /// For the first reply, which comes once the browser has started; none means it doesn't
    /// allow the pipe.
    #[cfg(windows)]
    first_reply: Duration,
    /// Until the first reply, how often the cookies are asked for again (`ask`).
    #[cfg(windows)]
    resend: Duration,
    /// For the sign-in window.
    timeout: Duration,
    /// The sign-in window exiting sooner than this after the start didn't start; later, the
    /// user closed it.
    early_exit: Duration,
    /// For the browser to exit after `Browser.close`.
    close: Duration,
}

impl Limits {
    fn real() -> Self {
        Self {
            #[cfg(windows)]
            first_reply: Duration::from_secs(15),
            #[cfg(windows)]
            resend: Duration::from_secs(5),
            timeout: timeout(),
            early_exit: Duration::from_secs(3),
            close: Duration::from_secs(5),
        }
    }
}

/// 15 minutes: the time runs until the user closes the window rather than until the session
/// shows, and a slow 2-step verification shouldn't run into it. A debug build takes
/// `FEATHERPLAY_BROWSER_TIMEOUT_SECS` instead, to try the timeout by hand.
fn timeout() -> Duration {
    let dev = cfg!(debug_assertions)
        .then(|| std::env::var("FEATHERPLAY_BROWSER_TIMEOUT_SECS").ok())
        .flatten()
        .and_then(|s| s.trim().parse().ok());
    Duration::from_secs(dev.unwrap_or(15 * 60))
}

/// What the sign-in watches besides the pipe: the browser's own process.
trait Process {
    fn exited(&self) -> bool;
    fn wait(&self, limit: Duration) -> bool;
    /// Ends it and everything it started.
    fn kill(&self);
    /// Asks it to quit, as closing its window would.
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
    #[cfg(windows)]
    let Some(exe) = browser.find(pc) else {
        log!("browser sign-in: {} not found", browser.name);
        return Outcome::NotFound;
    };
    // The browser would open and never answer: don't start it at all.
    #[cfg(windows)]
    if let Some(why) = browser.blocked(pc) {
        log!(
            "browser sign-in: {}'s RemoteDebuggingAllowed policy is off",
            browser.name
        );
        return Outcome::Unusable(why);
    }
    #[cfg(windows)]
    let profile = profile::create(cache, profile::PREFIX);
    #[cfg(target_os = "linux")]
    let (exe, profile) = {
        let Some(found) = browser.locate(pc) else {
            log!("browser sign-in: {} not found", browser.name);
            return Outcome::NotFound;
        };
        let (dir, prefix) = profile_place(&found.sandbox, pc, cache);
        (found.exe, profile::create(&dir, prefix))
    };
    let profile = match profile {
        Ok(p) => TempProfile(p),
        Err(e) => return Outcome::DidntStart(format!("temporary profile: {}", e.kind())),
    };
    if let Some(prefs) = browser.preferences
        && let Err(e) = seed(browser, &profile.0, prefs)
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

/// Where the profile goes, and its name's prefix: in the app's cache, or for a Snap or a
/// Flatpak, in the folder its sandbox lets it reach, named for featherplay.
#[cfg(target_os = "linux")]
fn profile_place(
    sandbox: &browsers::Sandbox,
    pc: &impl browsers::Lookup,
    cache: &Path,
) -> (PathBuf, &'static str) {
    match sandbox.profiles(pc) {
        Some(dir) => (dir, profile::SANDBOX_PREFIX),
        None => (cache.to_path_buf(), profile::PREFIX),
    }
}

/// Writes the browser's `preferences` into the new profile: Firefox's `user.js`, a Chromium
/// one's `Default/Preferences`.
#[cfg(target_os = "linux")]
fn seed(browser: &Browser, profile: &Path, prefs: &str) -> std::io::Result<()> {
    if browser.kind == browsers::Kind::Firefox {
        return std::fs::write(profile.join("user.js"), prefs);
    }
    profile::seed(profile, prefs)
}

/// A Chromium one's `Default/Preferences`.
#[cfg(windows)]
fn seed(_: &Browser, profile: &Path, prefs: &str) -> std::io::Result<()> {
    profile::seed(profile, prefs)
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

/// The sign-in window with no pipe, then, once the user has closed it, the read of its
/// profile: headless over the pipe (Windows), from disk (Linux).
fn launch_and_watch(browser: &Browser, exe: &Path, profile: &Path, cancel: &AtomicBool) -> Outcome {
    let limits = Limits::real();
    let url = session::SIGN_IN_URL;
    let ended = match sign_in_window(browser, exe, profile, url, cancel, &limits) {
        Ok(ended) => ended,
        Err(outcome) => return outcome,
    };
    #[cfg(windows)]
    let read = read_profile(browser, exe, profile, cancel, &limits);
    #[cfg(target_os = "linux")]
    let read = read_from_disk(browser, profile);
    match read {
        Ok(cookies) => session_or(&cookies, wall_now(), ended),
        Err(outcome) => outcome,
    }
}

/// The cookies the closed browser kept in its profile, from its cookie database.
#[cfg(target_os = "linux")]
fn read_from_disk(browser: &Browser, profile: &Path) -> Result<Vec<Cookie>, Outcome> {
    use browsers::cookies::{self, Unreadable};
    cookies::read(profile, browser.kind).map_err(|e| match e {
        Unreadable::Keyring => {
            log!(
                "browser sign-in: {} kept its cookies under the keyring's key \
                 (--password-store=basic didn't take); the keyring isn't asked",
                browser.name
            );
            Outcome::Blocked
        }
        Unreadable::Store(e) => {
            log!("browser sign-in: reading the profile's cookies failed: {e}");
            Outcome::Lost(e)
        }
    })
}

/// How the sign-in window went away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    /// The user closed it.
    Closed,
    /// featherplay closed it at the time limit.
    TimeLimit,
}

/// The sign-in window: the temporary profile, its own switches, the page. No pipe, and
/// nothing that marks the browser as automated or hides that mark.
fn window_args(b: &Browser, profile: &Path, url: &str) -> Vec<OsString> {
    #[cfg(target_os = "linux")]
    let mut out = match b.kind {
        browsers::Kind::Chromium => vec![user_data_dir(profile)],
        browsers::Kind::Firefox => vec!["--profile".into(), profile.into()],
    };
    #[cfg(windows)]
    let mut out = vec![user_data_dir(profile)];
    out.extend(b.switches.iter().map(OsString::from));
    out.push(url.into());
    out
}

/// The read of that profile: the profile, the pipe, headless, its own switches, a blank
/// page.
#[cfg(windows)]
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
fn sign_in_window(
    browser: &Browser,
    exe: &Path,
    profile: &Path,
    url: &str,
    cancel: &AtomicBool,
    limits: &Limits,
) -> Result<Ended, Outcome> {
    #[cfg(windows)]
    let process = browsers::launch_plain(exe, &window_args(browser, profile, url));
    #[cfg(target_os = "linux")]
    let process = {
        let lock = browsers::ProfileLock::new(profile, browser.kind);
        browsers::launch_plain(exe, &window_args(browser, profile, url), Some(lock))
    };
    let process = process.map_err(|e| {
        log!("browser sign-in: {} didn't start: {e}", browser.name);
        Outcome::DidntStart(e.to_string())
    })?;
    log!(
        "browser sign-in: {} started (pid {}), no pipe",
        browser.name,
        process.pid
    );
    let closed = wait_closed(&process, cancel, limits);
    // Ending its job (Linux: its group) ends any helper that's left, before the read starts on
    // the profile.
    drop(process);
    closed
}

/// Waits for the browser's process to exit (on Linux, and to let go of its profile): the user
/// closed it. At `limits.timeout` it's asked to quit, as closing it would, so it writes its
/// cookies, and ended if it hasn't within `limits.close`; either way its profile is read next. Ended on Cancel; one that exits within
/// `limits.early_exit` didn't start.
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
            Err(_) if started.elapsed() < limits.early_exit => {
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
#[cfg(windows)]
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
#[cfg(windows)]
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

/// Why waiting stopped.
enum Stop {
    Cancelled,
    /// The browser's process exited.
    Exited,
    /// The pipe closed or broke.
    #[cfg(windows)]
    PipeClosed,
    /// No reply in time.
    #[cfg(windows)]
    NoReply,
}

/// How often waits look at Cancel and the browser's process.
const SLICE: Duration = Duration::from_millis(100);

/// Sends `Storage.getCookies` and waits for its reply, at most `within`. With `resend`, sends
/// it again each time that long passes without a reply, in case a browser that isn't up yet
/// lost it (it's only read, so asking twice changes nothing): a reply to any of this wait's
/// commands counts, and one to an earlier wait's is skipped.
#[cfg(windows)]
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

fn check(process: &impl Process, cancel: &AtomicBool) -> Result<(), Stop> {
    if cancel.load(Ordering::Relaxed) {
        Err(Stop::Cancelled)
    } else if process.exited() {
        Err(Stop::Exited)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
