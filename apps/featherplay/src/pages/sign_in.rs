//! Sign in page (ui/pages/sign_in.slint). Shown on first run (no session), after a
//! sign-out, and on "Session expired, sign in again"; after a sign-in it goes Home.
//!
//! The account menu's "Switch Google account" opens the same window on Google's account
//! chooser (`switch_account`), from any page; its news then shows as toasts.
//!
//! Three ways in: the sign-in window (`webview`, a separate WebView2 window on its own
//! thread, the default), Microsoft Edge with a temporary profile (`edge`), or a pasted
//! cookie header, checked here for shape and by the controller with one read. The two
//! windows build the same header (`session`). Every way, the controller stores the header
//! and signs in without a restart. The header is never logged.

mod edge;
mod session;
mod webview;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use featherplay_core::tr;
use featherplay_store::Paths;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::{AccountAction, AccountEvent, Action, Cookies, PlayerState};
use crate::pages::settings;
use crate::router::Page;
use crate::ui::SignInData;

/// Once at startup: connect `SignInData`'s callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    let c = ctx.clone();
    data.on_open_window(move || open_window(&c, session::SIGN_IN_URL));
    let c = ctx.clone();
    data.on_open_edge(move || open_edge(&c));
    data.set_edge_blocked(edge::pipe_blocked());
    data.on_cancel_edge(cancel_edge);
    let c = ctx.clone();
    data.on_submit(move |text| submit(&c, &text));
}

/// At startup: deletes the temporary Edge profiles a crash or a kill left in `cache`, on a
/// thread of its own (a profile can hold thousands of files).
pub fn sweep_edge_profiles(cache: &Path) {
    let cache = cache.to_path_buf();
    let swept = std::thread::Builder::new()
        .name("featherplay-signin-sweep".into())
        .spawn(move || {
            let removed = edge::sweep(&cache);
            if removed > 0 {
                log!("removed {removed} leftover Edge sign-in profile(s)");
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
        // The policy may have changed since the app started.
        data.set_edge_blocked(edge::pipe_blocked());
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
        data.set_error("".into());
        data.set_expired(false);
        data.set_cookies("".into());
        data.set_paste_open(false);
        if ctx.current_page() == Page::SignIn {
            ctx.navigate(Page::Home);
        }
    } else {
        // An Edge sign-in still running keeps the page busy until it ends.
        if EDGE.with(|e| e.borrow().is_none()) {
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
        AccountEvent::SignInFailed(why) => {
            data.set_busy(false);
            data.set_status("".into());
            // From the account menu, on another page: the page's error line isn't shown.
            if ctx.current_page() != Page::SignIn {
                ctx.toast(&why);
            }
            data.set_error(why.into());
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

fn open_window(ctx: &Rc<Ctx>, start: &'static str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    if data.get_busy() {
        return;
    }
    data.set_busy(true);
    data.set_error("".into());
    data.set_status(tr!("Finish signing in in the other window. It closes by itself.").into());
    if ctx.current_page() != Page::SignIn {
        ctx.toast(tr!("Pick or add a Google account in the sign-in window"));
    }
    let profile = Paths::resolve().cache.join("webview2");
    let opened = webview::open(profile, start, |outcome| {
        let _ = slint::invoke_from_event_loop(move || window_closed(outcome));
    });
    if let Err(e) = opened {
        log!("sign-in window: {e}");
        data.set_busy(false);
        data.set_status("".into());
        if ctx.current_page() != Page::SignIn {
            ctx.toast(&e);
        }
        data.set_error(e.into());
    }
}

/// On the UI thread, once the sign-in window has closed.
fn window_closed(outcome: webview::Outcome) {
    let Some(ctx) = Ctx::get() else { return };
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    match outcome {
        webview::Outcome::SignedIn(header) => {
            data.set_status("Checking the session…".into());
            ctx.send(Action::Account(AccountAction::SignIn(Cookies(header))));
        }
        webview::Outcome::NotSignedIn => {
            data.set_busy(false);
            data.set_status("".into());
            let why = tr!("The sign-in window closed before you signed in.");
            if ctx.current_page() != Page::SignIn {
                ctx.toast(why);
            }
            data.set_error(why.into());
        }
        webview::Outcome::Cancelled => {
            data.set_busy(false);
            data.set_status("".into());
        }
        webview::Outcome::Failed(e) => {
            log!("sign-in window failed: {e}");
            data.set_busy(false);
            data.set_status("".into());
            let why = tr!("The sign-in window didn't work: {}", e);
            if ctx.current_page() != Page::SignIn {
                ctx.toast(&why);
            }
            data.set_error(why.into());
        }
    }
}

thread_local! {
    /// The running Edge sign-in, for Cancel.
    static EDGE: RefCell<Option<edge::Cancel>> = const { RefCell::new(None) };
}

/// "Use Microsoft Edge": Google's sign-in in Edge, with a temporary profile.
fn open_edge(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    if data.get_busy() || EDGE.with(|e| e.borrow().is_some()) {
        return;
    }
    data.set_busy(true);
    data.set_edge(true);
    data.set_error("".into());
    data.set_status(
        tr!("Finish signing in in Microsoft Edge. featherplay closes it once you're in.").into(),
    );
    let started = edge::start(Paths::resolve().cache, |outcome| {
        let _ = slint::invoke_from_event_loop(move || edge_done(outcome));
    });
    match started {
        Ok(cancel) => EDGE.with(|e| *e.borrow_mut() = Some(cancel)),
        Err(e) => {
            log!("{e}");
            edge_done(edge::Outcome::DidntStart(e));
        }
    }
}

/// Cancel while Edge is open: Edge closes and its profile is deleted; `edge_done` follows.
fn cancel_edge() {
    let running = EDGE.with(|e| e.borrow().as_ref().map(edge::Cancel::cancel).is_some());
    if running
        && let Some(ctx) = Ctx::get()
        && let Some(ui) = ctx.window()
    {
        ui.global::<SignInData>()
            .set_status(tr!("Closing Edge…").into());
    }
}

/// On the UI thread, once Edge is gone and its profile deleted.
fn edge_done(outcome: edge::Outcome) {
    use edge::Outcome;
    EDGE.with(|e| e.borrow_mut().take());
    let Some(ctx) = Ctx::get() else { return };
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    data.set_edge(false);
    let why = match outcome {
        Outcome::SignedIn(header) => {
            data.set_status("Checking the session…".into());
            ctx.send(Action::Account(AccountAction::SignIn(Cookies(header))));
            return;
        }
        Outcome::Cancelled => None,
        Outcome::Closed => Some(tr!("Edge closed before you signed in.")),
        Outcome::TimedOut => Some(tr!(
            "You didn't sign in within 10 minutes, so featherplay closed Edge."
        )),
        Outcome::NotFound => Some(tr!(
            "featherplay can't find Microsoft Edge on this PC. Use Sign in with Google instead."
        )),
        Outcome::DidntStart(e) => {
            log!("edge sign-in: didn't start: {e}");
            Some(tr!(
                "Microsoft Edge didn't start. Use Sign in with Google instead."
            ))
        }
        Outcome::Blocked => Some(tr!(
            "Edge didn't let featherplay read the sign-in. A policy on this PC may block it. Use Sign in with Google instead."
        )),
        Outcome::PolicyOff => {
            data.set_edge_blocked(true);
            None
        }
        Outcome::Lost(e) => {
            log!("edge sign-in: lost Edge: {e}");
            Some(tr!(
                "featherplay lost the connection to Edge. Try again, or use Sign in with Google."
            ))
        }
    };
    data.set_busy(false);
    data.set_status("".into());
    if let Some(why) = why {
        data.set_error(why.into());
    }
}

fn submit(ctx: &Rc<Ctx>, text: &str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SignInData>();
    if data.get_busy() {
        return;
    }
    match check_cookie_header(text) {
        Ok(header) => {
            data.set_busy(true);
            data.set_error("".into());
            data.set_status("Checking the session…".into());
            ctx.send(Action::Account(AccountAction::SignIn(Cookies(header))));
        }
        Err(why) => data.set_error(why.into()),
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
