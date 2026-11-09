//! Sign in page (ui/pages/sign_in.slint). Shown on first run (no session), after a
//! sign-out, and on "Session expired, sign in again"; after a sign-in it goes Home.
//!
//! The account menu's "Switch Google account" opens the same window on Google's account
//! chooser (`switch_account`), from any page; its news then shows as toasts.
//!
//! Three ways in: the sign-in window (the default: a slim browser frame, `frame`, round
//! Google's page in WebView2 on a thread of its own, `webview`, its address from the
//! webview's `Source`, `address`), Microsoft Edge with a temporary profile (`browser`), or a
//! pasted cookie header, checked here for shape and by the controller with one read. The
//! two windows build the same header (`session`). Every way, the controller stores the
//! header and signs in without a restart. The header is never logged.

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
use featherplay_store::Paths;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::{AccountAction, AccountEvent, Action, Cookies, PlayerState};
use crate::pages::settings;
use crate::router::Page;
use crate::ui::{SignInData, SignInWay};

/// Once at startup: connect `SignInData`'s callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    let c = ctx.clone();
    data.on_open_window(move || open_window(&c, session::SIGN_IN_URL));
    let c = ctx.clone();
    data.on_open_edge(move || open_edge(&c));
    data.set_edge_blocked(browser::pipe_blocked(&browser::EDGE));
    data.on_cancel_edge(cancel_edge);
    let c = ctx.clone();
    data.on_submit(move |text| submit(&c, &text));
}

/// At startup: deletes the temporary Edge profiles a crash or a kill left in `cache`, on a
/// thread of its own (a profile can hold thousands of files).
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
        // The policy may have changed since the app started.
        data.set_edge_blocked(browser::pipe_blocked(&browser::EDGE));
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
        data.set_edge(false);
        data.set_status("".into());
        clear_problem(&data);
        data.set_expired(false);
        data.set_cookies("".into());
        data.set_paste_open(false);
        if ctx.current_page() == Page::SignIn {
            ctx.navigate(Page::Home);
        }
    } else {
        // An Edge sign-in still running keeps the page busy until it ends.
        if RUNNING.with(|e| e.borrow().is_none()) && !frame::is_open() {
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
/// new cookies (the session's own identity), as after a first sign-in.
pub fn switch_account(ctx: &Rc<Ctx>) {
    open_window(ctx, webview::CHOOSER_URL);
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

thread_local! {
    /// The running Edge sign-in, for Cancel.
    static RUNNING: RefCell<Option<browser::Cancel>> = const { RefCell::new(None) };
}

/// "Use Microsoft Edge": Google's sign-in in Edge, with a temporary profile.
fn open_edge(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    if data.get_busy() || RUNNING.with(|e| e.borrow().is_some()) {
        return;
    }
    LAST_WAY.set(SignInWay::Edge);
    data.set_busy(true);
    data.set_edge(true);
    clear_problem(&data);
    data.set_status(
        tr!("Finish signing in in Microsoft Edge. featherplay closes it once you're in.").into(),
    );
    let started = browser::start(&browser::EDGE, Paths::resolve().cache, |outcome| {
        let _ = slint::invoke_from_event_loop(move || edge_done(outcome));
    });
    match started {
        Ok(cancel) => RUNNING.with(|e| *e.borrow_mut() = Some(cancel)),
        Err(e) => {
            log!("{e}");
            edge_done(browser::Outcome::DidntStart(e));
        }
    }
}

/// Cancel while Edge is open: Edge closes and its profile is deleted; `edge_done` follows.
fn cancel_edge() {
    let running = RUNNING.with(|e| e.borrow().as_ref().map(browser::Cancel::cancel).is_some());
    if running
        && let Some(ctx) = Ctx::get()
        && let Some(ui) = ctx.window()
    {
        ui.global::<SignInData>()
            .set_status(tr!("Closing Edge…").into());
    }
}

/// On the UI thread, once Edge is gone and its profile deleted.
fn edge_done(outcome: browser::Outcome) {
    use browser::Outcome;
    RUNNING.with(|e| e.borrow_mut().take());
    let Some(ctx) = Ctx::get() else { return };
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    data.set_edge(false);
    let not_yet = tr!("It closed before you finished signing in. Try again when you're ready.");
    let instead = tr!("Try again, or use Sign in with Google.");
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
        Outcome::PolicyOff => {
            data.set_edge_blocked(true);
            data.set_busy(false);
            data.set_status("".into());
            return;
        }
        Outcome::Closed => (tr!("Edge closed"), not_yet, SignInWay::Edge),
        Outcome::TimedOut => (
            tr!("Edge closed after 10 minutes"),
            tr!(
                "featherplay closes it when the sign-in takes that long. Try again when you're ready."
            ),
            SignInWay::Edge,
        ),
        Outcome::NotFound => (
            tr!("Microsoft Edge isn't on this PC"),
            tr!("Sign in with Google works without it."),
            SignInWay::None,
        ),
        Outcome::DidntStart(e) => {
            log!("browser sign-in: didn't start: {e}");
            (tr!("Microsoft Edge didn't start"), instead, SignInWay::Edge)
        }
        Outcome::Blocked => (
            tr!("Edge didn't let featherplay in"),
            tr!("A setting on this PC may block it. Sign in with Google works without it."),
            SignInWay::None,
        ),
        Outcome::Lost(e) => {
            log!("browser sign-in: lost Edge: {e}");
            (
                tr!("featherplay lost touch with Edge"),
                instead,
                SignInWay::Edge,
            )
        }
    };
    problem(&ctx, title, line, retry);
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
