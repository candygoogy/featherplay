//! Sign in page (ui/pages/sign_in.slint). Shown on first run (no session), after a
//! sign-out, and on "Session expired, sign in again"; after a sign-in it goes Home.
//!
//! The account menu's "Switch Google account" opens the same window on Google's account
//! chooser (`switch_account`), from any page; its news then shows as toasts.
//!
//! Three ways in: the sign-in window (the default: a slim browser frame, `frame`, round
//! Google's page in WebView2 on a thread of its own, `webview`, its address from the
//! webview's `Source`, `address`), the user's own browser with a temporary profile
//! (`browser`: Microsoft Edge, Google Chrome, Brave or Vivaldi), or a pasted cookie header,
//! checked here for shape and by the controller with one read. The two windows build the
//! same header (`session`). Every way, the controller stores the header and signs in
//! without a restart. The header is never logged.
//!
//! The browser row offers one browser, named in its title: the one used last (kept in the
//! settings), else the system's default browser when it's a known one, else the first found
//! in the order Edge, Chrome, Brave, Vivaldi (Linux: Chrome, Chromium, Edge, Brave, Vivaldi);
//! one whose policy turns the connection off (or, on Linux, a Snap or a Flatpak) gives way to
//! one that works. With more than one found, "Change" lists them.
//!
//! Linux has no sign-in window (`Feature::WebViewSignIn`): the page hides that row, the
//! browser row leads, and "Switch Google account" opens this page. The card's last line says
//! where the session is kept: in Credential Manager, in the desktop's keyring, or, with no
//! keyring running (`Feature::SecretStorage`), for this run only.

mod address;
mod browser;
mod frame;
mod session;
mod webview;

pub use frame::SIZE as WINDOW_SIZE;

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

use featherplay_core::tr;
use featherplay_platform::{Feature, supports};
use featherplay_store::Paths;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::{AccountAction, AccountEvent, Action, Cookies, PlayerState};
use crate::pages::settings;
use crate::router::Page;
use crate::ui::{BrowserBlock, BrowserChoice, SessionStore, SignInData, SignInWay};
use crate::ui_bridge;
use browser::{Block, Browser};

/// Once at startup: connect `SignInData`'s callbacks. The browsers are looked for when the
/// page is shown (`open`), not here.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    data.set_window_way(supports(Feature::WebViewSignIn));
    show_session_store(&ui);
    // Until the page looks: the row names the first browser (Edge; Linux: Chrome), as with
    // none found.
    offer(&data, Vec::new(), 0);
    let c = ctx.clone();
    data.on_open_window(move || open_window(&c, session::SIGN_IN_URL));
    let c = ctx.clone();
    data.on_open_browser(move || open_browser(&c));
    data.on_cancel_browser(cancel_browser);
    let c = ctx.clone();
    data.on_pick_browser(move |index| pick_browser(&c, index));
    let c = ctx.clone();
    data.on_submit(move |text| submit(&c, &text));
}

/// The card's last line: where the session is kept. Windows always has Credential Manager;
/// on Linux, whether a Secret Service answers is asked off the UI thread (a D-Bus round
/// trip, longer while one starts up), and the line says the keyring until it's known.
fn show_session_store(ui: &crate::ui::AppWindow) {
    let data = ui.global::<SignInData>();
    if cfg!(windows) {
        data.set_session_store(SessionStore::CredentialManager);
        return;
    }
    data.set_session_store(SessionStore::Keyring);
    let weak = ui.as_weak();
    let asked = std::thread::Builder::new()
        .name("featherplay-keyring-check".into())
        .spawn(move || {
            if !supports(Feature::SecretStorage) {
                let _ = weak.upgrade_in_event_loop(|ui| {
                    ui.global::<SignInData>()
                        .set_session_store(SessionStore::None);
                });
            }
        });
    if let Err(e) = asked {
        log!("keyring check: {e}");
    }
}

/// The sentences that point to another way in when one fails: the sign-in window where
/// there is one, else a pasted cookie header.
fn other_way() -> (&'static str, &'static str) {
    if supports(Feature::WebViewSignIn) {
        (
            tr!("Try again, or use Sign in with Google."),
            tr!("Sign in with Google works without it."),
        )
    } else {
        (
            tr!("Try again, or paste a cookie header."),
            tr!("Pasting a cookie header works without it."),
        )
    }
}

/// At startup: deletes the temporary browser profiles a crash or a kill left in `cache`, on
/// a thread of its own (a profile can hold thousands of files).
pub fn sweep_browser_profiles(cache: &Path) {
    let cache = cache.to_path_buf();
    let swept = std::thread::Builder::new()
        .name("featherplay-signin-sweep".into())
        .spawn(move || {
            let removed = browser::sweep(&cache);
            if removed > 0 {
                log!("removed {removed} leftover sign-in profile(s)");
            }
        });
    if let Err(e) = swept {
        log!("sign-in profile sweep: {e}");
    }
}

/// The page is being shown.
pub fn open(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    if !data.get_busy() {
        data.set_status("".into());
        // A failure told on another page (a toast) is old news here.
        clear_problem(&data);
        // What's installed, the default browser and the policies may have changed since.
        look_for_browsers(&data);
    }
}

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(_ctx: &Rc<Ctx>, _state: &PlayerState) {}

/// Signed in or out. Signed out, the app works as a visitor: only a session that ended
/// (`expired`, told just before) brings the window to this page; signed in from it, it goes
/// Home.
pub fn account_changed(ctx: &Rc<Ctx>, signed_in: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    if signed_in {
        data.set_busy(false);
        data.set_browser_running(false);
        data.set_status("".into());
        clear_problem(&data);
        data.set_expired(false);
        data.set_cookies("".into());
        data.set_paste_open(false);
        if ctx.current_page() == Page::SignIn {
            ctx.navigate(Page::Home);
        }
    } else {
        // A browser sign-in still running keeps the page busy until it ends.
        if RUNNING.with(|r| r.borrow().is_none()) && !frame::is_open() {
            data.set_busy(false);
        }
        if data.get_expired() && ctx.current_page() != Page::SignIn {
            ctx.navigate(Page::SignIn);
        }
    }
}

/// The controller's account news: sign-in failures, the expired session, Last.fm.
pub fn notice(ctx: &Rc<Ctx>, ev: AccountEvent) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    match ev {
        // The session the way in brought back doesn't work: `why` says so in plain words.
        AccountEvent::SignInFailed(why) => {
            problem(ctx, tr!("That sign-in didn't work"), &why, LAST_WAY.get())
        }
        AccountEvent::SessionExpired => data.set_expired(true),
        AccountEvent::Lastfm(status, message) => settings::lastfm_changed(ctx, status, message),
        AccountEvent::LastfmKeys { custom } => settings::lastfm_keys_changed(ctx, custom),
        AccountEvent::ListenBrainz(status, message) => {
            settings::listenbrainz_changed(ctx, status, message)
        }
    }
}

/// The account menu's "Switch Google account": the sign-in window on Google's account
/// chooser, to add a login or pick another one. Once it closes, the app signs in with the
/// new cookies (the session's own identity), as after a first sign-in. With no sign-in
/// window (Linux), this page, where a browser sign-in (whose profile is new each time) or a
/// pasted header brings in the other account.
pub fn switch_account(ctx: &Rc<Ctx>) {
    if supports(Feature::WebViewSignIn) {
        open_window(ctx, webview::CHOOSER_URL);
    } else {
        ctx.navigate(Page::SignIn);
    }
}

thread_local! {
    /// The way in last tried, for Try again after the session check fails.
    static LAST_WAY: Cell<SignInWay> = const { Cell::new(SignInWay::None) };
}

/// A way in failed: the page says what happened (`title`) and what to do (`line`), with Try
/// again for `retry`; on another page, a toast says what happened. The technical detail is
/// for the log, never here.
fn problem(ctx: &Rc<Ctx>, title: &str, line: &str, retry: SignInWay) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    data.set_busy(false);
    data.set_status("".into());
    data.set_error_title(title.into());
    data.set_error(line.into());
    data.set_retry(retry);
    if ctx.current_page() != Page::SignIn {
        ctx.toast_error(title);
    }
}

fn clear_problem(data: &SignInData) {
    data.set_error_title("".into());
    data.set_error("".into());
    data.set_retry(SignInWay::None);
    data.set_paste_error("".into());
}

/// The sign-in window didn't open (`why`, for the log): the page says so and offers Try
/// again. On the UI thread.
pub fn didnt_open(why: &str) {
    log!("sign-in window: {why}");
    if let Some(ctx) = Ctx::get() {
        problem(
            &ctx,
            tr!("The sign-in window didn't open"),
            tr!("Try again, or use another way below."),
            SignInWay::Window,
        );
    }
}

fn open_window(ctx: &Rc<Ctx>, start: &'static str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    if data.get_busy() {
        return;
    }
    LAST_WAY.set(SignInWay::Window);
    data.set_busy(true);
    // The list of browsers would unfold again once this way ends.
    data.set_choosing(false);
    clear_problem(&data);
    data.set_status(tr!("Finish signing in in the other window. It closes by itself.").into());
    if ctx.current_page() != Page::SignIn {
        ctx.toast(tr!("Pick or add a Google account in the sign-in window"));
    }
    let profile = Paths::resolve().cache.join("webview2");
    let opened = frame::open(ctx, profile, start, |outcome| {
        let _ = slint::invoke_from_event_loop(move || window_closed(outcome));
    });
    if let Err(why) = opened {
        didnt_open(&why);
    }
}

/// The sign-in window's frame showing `source` as its address, as `--snapshot` renders it.
pub fn frame_for_snapshot(ctx: &Rc<Ctx>, source: &str) -> Result<crate::ui::SignInWindow, String> {
    let ui = frame::create(ctx)?;
    frame::show_address(&ui, &address::address(source));
    ui.set_can_back(true);
    Ok(ui)
}

/// On the UI thread, once the sign-in window has closed.
fn window_closed(outcome: webview::Outcome) {
    // The page thread has ended, so its parent can go.
    frame::closed();
    let Some(ctx) = Ctx::get() else { return };
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    match outcome {
        webview::Outcome::SignedIn(header) => {
            data.set_status(tr!("Checking the session…").into());
            ctx.send(Action::Account(AccountAction::SignIn(Cookies(header))));
        }
        webview::Outcome::NotSignedIn => problem(
            &ctx,
            tr!("The sign-in window closed"),
            tr!("It closed before you finished signing in. Try again when you're ready."),
            SignInWay::Window,
        ),
        webview::Outcome::Cancelled => {
            data.set_busy(false);
            data.set_status("".into());
        }
        webview::Outcome::Failed(e) => {
            log!("sign-in window failed: {e}");
            problem(
                &ctx,
                tr!("The sign-in window stopped"),
                tr!("Something went wrong inside it. Try again, or use another way below."),
                SignInWay::Window,
            );
        }
    }
}

/// The browser row's state: the known browsers found on this PC, and the one it offers.
struct Offers {
    found: Vec<browser::Offer>,
    /// The one the row offers, an index into `found`.
    current: usize,
}

impl Offers {
    /// The browser the row names. With none found that's the first known one (Edge; on
    /// Linux, Chrome), which says it isn't on this PC when it's tried.
    fn browser(&self) -> &'static Browser {
        self.found
            .get(self.current)
            .map_or(browser::ALL[0], |o| o.browser)
    }
}

/// How the page tells why a browser can't be used.
fn block_kind(why: Block) -> BrowserBlock {
    match why {
        Block::Policy => BrowserBlock::Policy,
        Block::Snap => BrowserBlock::Snap,
        Block::Flatpak => BrowserBlock::Flatpak,
    }
}

thread_local! {
    static OFFERS: RefCell<Offers> = const {
        RefCell::new(Offers { found: Vec::new(), current: 0 })
    };
    /// The running browser sign-in: its Cancel, and the browser it opened.
    static RUNNING: RefCell<Option<(browser::Cancel, &'static Browser)>> =
        const { RefCell::new(None) };
}

/// Looks for the known browsers on this PC and shows the one the row offers.
fn look_for_browsers(data: &SignInData) {
    let (found, pick) = browser::offers(settings::sign_in_browser().as_deref());
    offer(data, found, pick.unwrap_or(0));
}

/// The row offers `found[current]`, and "Change" lists `found` when there's a choice.
fn offer(data: &SignInData, found: Vec<browser::Offer>, current: usize) {
    let offers = Offers { found, current };
    let browser = offers.browser();
    data.set_browser_name(browser.name.into());
    data.set_browser_short(browser.short.into());
    let blocked = offers.found.get(current).and_then(|o| o.blocked);
    data.set_browser_blocked(blocked.is_some());
    data.set_browser_block(blocked.map_or(BrowserBlock::Policy, block_kind));
    data.set_browser_current(i32::try_from(current).unwrap_or(0));
    // One browser is no choice: the list is for two or more.
    let choices: Vec<BrowserChoice> = offers
        .found
        .iter()
        .filter(|_| offers.found.len() > 1)
        .map(|o| BrowserChoice {
            name: o.browser.name.into(),
            short: o.browser.short.into(),
            blocked: o.blocked.is_some(),
            block: o.blocked.map_or(BrowserBlock::Policy, block_kind),
        })
        .collect();
    data.set_browsers(ui_bridge::model(choices).1);
    data.set_choosing(false);
    OFFERS.with(|o| *o.borrow_mut() = offers);
}

/// The browser row as on a PC with these known browsers installed (by id, in the page's
/// order; `true` for one whose policy turns the connection off), offering `current`. For
/// `--snapshot` and the tests, which mustn't show the PC they run on.
pub fn offer_for_snapshot(ctx: &Rc<Ctx>, installed: &[(&str, bool)], current: &str) {
    let Some(ui) = ctx.window() else { return };
    let found: Vec<browser::Offer> = installed
        .iter()
        .filter_map(|&(id, blocked)| {
            let browser = browser::by_id(id)?;
            let blocked = blocked.then_some(Block::Policy);
            Some(browser::Offer { browser, blocked })
        })
        .collect();
    let current = found.iter().position(|o| o.browser.id == current);
    offer(&ui.global::<SignInData>(), found, current.unwrap_or(0));
}

/// "Change": the row offers the browser at `index` of the list from now on, and the next
/// start does too.
fn pick_browser(ctx: &Rc<Ctx>, index: i32) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    if data.get_busy() {
        return;
    }
    let picked = OFFERS.with(|o| {
        let o = o.borrow();
        let index = usize::try_from(index).ok()?;
        let offer = o.found.get(index).filter(|offer| offer.blocked.is_none())?;
        Some((o.found.clone(), index, offer.browser))
    });
    let Some((found, index, browser)) = picked else {
        return;
    };
    settings::set_sign_in_browser(ctx, browser.id);
    // A note on a blocked one shuts with the list, and Try again on the browser before
    // would open this one.
    data.set_why_open(false);
    if data.get_retry() == SignInWay::Browser {
        clear_problem(&data);
    }
    offer(&data, found, index);
}

/// "Use Microsoft Edge" (or the browser the row names): Google's sign-in in that browser,
/// with a temporary profile.
fn open_browser(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    if data.get_busy() || RUNNING.with(|r| r.borrow().is_some()) {
        return;
    }
    let (browser, found) = OFFERS.with(|o| {
        let o = o.borrow();
        (o.browser(), !o.found.is_empty())
    });
    // The one used last is the one offered next time.
    if found {
        settings::set_sign_in_browser(ctx, browser.id);
    }
    LAST_WAY.set(SignInWay::Browser);
    show_running(&data, browser);
    let started = browser::start(browser, Paths::resolve().cache, move |outcome| {
        let _ = slint::invoke_from_event_loop(move || browser_done(browser, outcome));
    });
    match started {
        Ok(cancel) => RUNNING.with(|r| *r.borrow_mut() = Some((cancel, browser))),
        Err(e) => {
            log!("{e}");
            browser_done(browser, browser::Outcome::DidntStart(e));
        }
    }
}

/// The page while `browser` is open for the sign-in: its row waits, with Cancel.
fn show_running(data: &SignInData, browser: &Browser) {
    data.set_busy(true);
    data.set_browser_running(true);
    data.set_choosing(false);
    clear_problem(data);
    // Linux reads the sign-in once the browser is closed (`browser`), Windows while it's open.
    let status = if cfg!(target_os = "linux") {
        tr!(
            "Sign in in {}, then close that window. featherplay picks up the session once it's closed.",
            browser.name
        )
    } else {
        tr!(
            "Finish signing in in {}. featherplay closes it once you're in.",
            browser.name
        )
    };
    data.set_status(status.into());
}

/// The page as while the browser its row offers is open for the sign-in, without opening
/// it: for `--snapshot`.
pub fn running_for_snapshot(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let browser = OFFERS.with(|o| o.borrow().browser());
    show_running(&ui.global::<SignInData>(), browser);
}

/// Cancel while the browser is open: it closes and its profile is deleted; `browser_done`
/// follows.
fn cancel_browser() {
    let running = RUNNING.with(|r| {
        let r = r.borrow();
        let (cancel, browser) = r.as_ref()?;
        cancel.cancel();
        Some(*browser)
    });
    if let Some(browser) = running
        && let Some(ctx) = Ctx::get()
        && let Some(ui) = ctx.window()
    {
        ui.global::<SignInData>()
            .set_status(tr!("Closing {}…", browser.short).into());
    }
}

/// On the UI thread, once `browser` is gone and its profile deleted.
fn browser_done(browser: &'static Browser, outcome: browser::Outcome) {
    use browser::Outcome;
    RUNNING.with(|r| r.borrow_mut().take());
    let Some(ctx) = Ctx::get() else { return };
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    data.set_browser_running(false);
    let (name, short) = (browser.name, browser.short);
    let not_yet = tr!("It closed before you finished signing in. Try again when you're ready.");
    let (instead, without_it) = other_way();
    let (title, line, retry) = match outcome {
        Outcome::SignedIn(header) => {
            data.set_status(tr!("Checking the session…").into());
            ctx.send(Action::Account(AccountAction::SignIn(Cookies(header))));
            return;
        }
        Outcome::Cancelled => {
            data.set_busy(false);
            data.set_status("".into());
            return;
        }
        // Its policy was turned off since the page looked (or it's a Snap or a Flatpak now):
        // the row says so now, or offers another browser.
        Outcome::Unusable(_) => {
            data.set_busy(false);
            data.set_status("".into());
            look_for_browsers(&data);
            return;
        }
        Outcome::Closed => (tr!("{} closed", short), not_yet, SignInWay::Browser),
        // Linux gives the window 15 minutes (`browser::timeout`), and reads it even then.
        Outcome::TimedOut if cfg!(target_os = "linux") => (
            tr!("{} closed after 15 minutes", short),
            tr!(
                "featherplay closes it when it stays open that long, and found no sign-in in it. Try again when you're ready."
            ),
            SignInWay::Browser,
        ),
        Outcome::TimedOut => (
            tr!("{} closed after 10 minutes", short),
            tr!(
                "featherplay closes it when the sign-in takes that long. Try again when you're ready."
            ),
            SignInWay::Browser,
        ),
        Outcome::NotFound => {
            // It was removed since the page looked.
            look_for_browsers(&data);
            (
                tr!("{} isn't on this PC", name),
                without_it,
                SignInWay::None,
            )
        }
        Outcome::DidntStart(e) => {
            log!("browser sign-in: {name} didn't start: {e}");
            (tr!("{} didn't start", name), instead, SignInWay::Browser)
        }
        Outcome::Blocked => (
            tr!("{} didn't let featherplay in", short),
            if supports(Feature::WebViewSignIn) {
                tr!("A setting on this PC may block it. Sign in with Google works without it.")
            } else {
                tr!("A setting on this PC may block it. Pasting a cookie header works without it.")
            },
            SignInWay::None,
        ),
        Outcome::Lost(e) => {
            log!("browser sign-in: lost {name}: {e}");
            (
                tr!("featherplay lost touch with {}", short),
                instead,
                SignInWay::Browser,
            )
        }
    };
    problem(&ctx, &title, line, retry);
}

fn submit(ctx: &Rc<Ctx>, text: &str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    if data.get_busy() {
        return;
    }
    match check_cookie_header(text) {
        Ok(header) => {
            LAST_WAY.set(SignInWay::Paste);
            data.set_busy(true);
            data.set_choosing(false);
            clear_problem(&data);
            data.set_status(tr!("Checking the session…").into());
            ctx.send(Action::Account(AccountAction::SignIn(Cookies(header))));
        }
        // Said under the field, where the text is fixed.
        Err(why) => {
            data.set_paste_open(true);
            data.set_paste_error(why.into());
        }
    }
}

/// Tidies a pasted cookie header: drops a leading `cookie:` and line breaks, and checks
/// that it has `name=value` pairs and a session cookie. Says why when it can't be used.
pub fn check_cookie_header(text: &str) -> Result<String, &'static str> {
    let mut t = text.trim();
    if let Some((name, rest)) = t.split_once(':')
        && name.trim().eq_ignore_ascii_case("cookie")
    {
        t = rest.trim();
    }
    // Copied from a wrapped view: the breaks aren't part of the header.
    let joined: String = t.lines().map(str::trim).collect();
    let pairs: Vec<&str> = joined
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if pairs.is_empty() {
        return Err(tr!("Paste the cookie header first."));
    }
    if pairs.iter().any(|p| !p.contains('=') || p.starts_with('=')) {
        return Err(tr!(
            "That doesn't look like a cookie header: it should be name=value pairs separated by semicolons."
        ));
    }
    let signed_in = pairs
        .iter()
        .any(|p| session::is_session_cookie(p.split('=').next().unwrap_or_default()));
    if !signed_in {
        return Err(tr!(
            "That cookie header has no SAPISID, so it isn't signed in. Copy it from a tab that is signed in."
        ));
    }
    Ok(pairs.join("; "))
}

#[cfg(test)]
mod tests {
    use super::check_cookie_header;

    #[test]
    fn a_pasted_header_is_tidied() {
        let pasted = "cookie: SID=abc; HSID=d:e;\n  SAPISID=xyz/1;  PREF=f6=4000 ";
        assert_eq!(
            check_cookie_header(pasted).unwrap(),
            "SID=abc; HSID=d:e; SAPISID=xyz/1; PREF=f6=4000"
        );
        assert_eq!(
            check_cookie_header("__Secure-3PAPISID=q").unwrap(),
            "__Secure-3PAPISID=q"
        );
    }

    #[test]
    fn unusable_text_says_why() {
        assert!(check_cookie_header("   ").unwrap_err().contains("first"));
        assert!(
            check_cookie_header("hello world")
                .unwrap_err()
                .contains("name=value")
        );
        assert!(
            check_cookie_header("SID=abc; HSID=def")
                .unwrap_err()
                .contains("SAPISID")
        );
        // A value named like the session cookie doesn't count.
        assert!(check_cookie_header("X=SAPISID").is_err());
    }
}
