//! What the window's close button does (Settings > Window, "When I close the window"):
//! - **Ask**, the default: the close dialog (ui/close_dialog.slint) asks whether to close
//!   featherplay or keep playing in the tray. With "Remember my choice" checked (it starts
//!   checked) the answer becomes the setting; unchecked, it's for this once.
//! - **Quit**: stops playback and exits, the way the tray's Quit does.
//! - **Tray**: minimises the window into the taskbar, hides it once that has played, and
//!   keeps playing; the first time, a notification says where featherplay went.
//!
//! Without a tray icon there would be no way back, so closing quits then.

use std::rc::{Rc, Weak};
use std::time::Duration;

use featherplay_core::tr;
use featherplay_store::CloseAction;
use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};

use crate::app::{self, Ctx};
use crate::pages::settings;
use crate::shell;
use crate::ui::{AppWindow, CloseChoice, Frame};

/// How long a window going to the tray stays minimised on the taskbar before it hides, so
/// Windows' minimise animation into its button plays out.
///
/// Nothing says when that animation ends. IsIconic and WM_SIZE's SIZE_MINIMIZED both turn
/// true inside the minimise call, as the animation starts, so they only say the minimise
/// took; DWM then plays it on its own for about a quarter of a second. Hidden before that,
/// the window and its button go and the animation is cut. A little longer is harmless: the
/// window is already off screen, only its taskbar button stays a moment more.
pub const MINIMISE_WAIT: Duration = Duration::from_millis(350);

thread_local! {
    /// The hide that follows a minimise to the tray (`to_tray`). Single-shot: once it has
    /// run, or `cancel_hide` stopped it, no timer runs.
    static HIDE: Timer = Timer::default();
}

/// Connects the window's close button and the close dialog. `has_tray` says whether the
/// tray icon exists (`shell::has_tray` in the app; the offscreen tests have none and pass
/// their own).
pub fn install(ctx: &Rc<Ctx>, ui: &AppWindow, has_tray: impl Fn() -> bool + 'static) {
    let choice = ui.global::<CloseChoice>();
    let c = ctx.clone();
    choice.on_close_app(move || chosen(&c, CloseAction::Quit));
    let c = ctx.clone();
    choice.on_keep_playing(move || chosen(&c, CloseAction::Tray));
    let c = ctx.clone();
    choice.on_cancel(move || {
        if let Some(ui) = c.window() {
            ui.global::<CloseChoice>().set_open(false);
        }
        log!("close cancelled");
    });
    let c = ctx.clone();
    ui.window()
        .on_close_requested(move || requested(&c, has_tray()));
}

/// The close button was clicked (or Alt+F4, or the taskbar's Close).
fn requested(ctx: &Rc<Ctx>, has_tray: bool) -> CloseRequestResponse {
    match (settings::close_action(), has_tray) {
        (CloseAction::Quit, _) | (_, false) => {
            quit(ctx);
            CloseRequestResponse::HideWindow
        }
        (CloseAction::Tray, true) => {
            // Shown: `to_tray` hides it once the minimise has played.
            to_tray(ctx);
            CloseRequestResponse::KeepWindowShown
        }
        (CloseAction::Ask, true) => {
            // Closed from the taskbar while minimized or hidden: the dialog needs the
            // window on screen.
            app::show_window(ctx);
            open(ctx);
            CloseRequestResponse::KeepWindowShown
        }
    }
}

/// Opens the dialog, with "Remember my choice" checked and the keyboard on Close
/// featherplay.
fn open(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let choice = ui.global::<CloseChoice>();
    // Asked again while it's open (the title bar's close): keep what the user has set.
    if choice.get_open() {
        return;
    }
    choice.set_remember(true);
    choice.set_focused(0);
    choice.set_open(true);
}

/// The dialog's answer: saved when "Remember my choice" is checked, then carried out.
fn chosen(ctx: &Rc<Ctx>, action: CloseAction) {
    let Some(ui) = ctx.window() else { return };
    let choice = ui.global::<CloseChoice>();
    choice.set_open(false);
    let remember = choice.get_remember();
    log!(
        "close: {action:?}{}",
        if remember { ", remembered" } else { "" }
    );
    // Before quitting: the controller saves it ahead of the quit that follows.
    if remember {
        settings::set_close_action(ctx, action);
    }
    // The close request kept the window shown for the dialog. To the tray it's minimised
    // first, and hidden once that has played (`to_tray`).
    if action == CloseAction::Tray {
        to_tray(ctx);
        return;
    }
    if let Err(e) = ui.hide() {
        log!("hide window: {e}");
    }
    quit(ctx);
}

/// Stops playback and exits, after the settings sent before it are saved.
fn quit(ctx: &Ctx) {
    log!("closing featherplay");
    ctx.quit();
}

/// Sends the shown window to the tray, where the music keeps playing. It's minimised first,
/// so Windows plays its minimise animation into the taskbar, and hidden once that has played
/// (`MINIMISE_WAIT`); at once with Windows' animations off, if it was minimised already
/// (closed from its taskbar button) or if it didn't minimise. It stays minimised in the
/// tray, for the restore animation when it comes back (`app::show_window`, which calls off
/// a hide still waiting).
fn to_tray(ctx: &Rc<Ctx>) {
    let window = ctx.native();
    let wait = window.as_ref().is_some_and(|w| {
        let animates = w.animations_enabled() && !w.is_minimised();
        w.minimise();
        animates && w.is_minimised()
    });
    if !wait {
        hide(ctx);
        return;
    }
    let ctx = Rc::downgrade(ctx);
    HIDE.with(|t| {
        t.start(TimerMode::SingleShot, MINIMISE_WAIT, move || {
            minimise_played(&ctx);
        });
    });
}

/// Calls off a hide to the tray still waiting for the minimise: the window is being shown.
pub fn cancel_hide() {
    HIDE.with(Timer::stop);
}

/// The minimise to the tray has played: the window hides, unless featherplay is quitting or
/// the window was restored meanwhile (its taskbar button was still there to click).
fn minimise_played(ctx: &Weak<Ctx>) {
    let Some(ctx) = ctx.upgrade() else { return };
    if app::is_closing() || !ctx.native().is_some_and(|w| w.is_minimised()) {
        return;
    }
    hide(&ctx);
}

/// Hides the window to the tray. The first time, a notification says so; hidden, nothing in
/// the window may animate (the playing mark's clock stops).
fn hide(ctx: &Ctx) {
    log!("window hidden to the tray");
    if let Some(ui) = ctx.window() {
        ui.global::<Frame>().set_hidden(true);
        if let Err(e) = ui.hide() {
            log!("hide window: {e}");
        }
    }
    crate::memory::hidden(ctx, true);
    // Only with the real tray icon: it's what the notice points at.
    if shell::has_tray() && !settings::tray_notice_shown() {
        settings::mark_tray_notice_shown(ctx);
        let playing = ctx.player().is_some_and(|p| p.playing);
        let title = if playing {
            tr!("featherplay is still playing in the tray")
        } else {
            tr!("featherplay is still running in the tray")
        };
        // A few milliseconds of WinRT calls: off the UI thread.
        crate::app::blocking_detached(ctx.catalog.runtime(), move || {
            featherplay_integrations::notify(
                title,
                tr!("Right-click the tray icon to show the window or quit."),
            );
        });
    }
}

/// `--snapshot close-dialog`: the dialog as a close shows it.
pub fn show_for_snapshot(ctx: &Ctx) {
    open(ctx);
}
