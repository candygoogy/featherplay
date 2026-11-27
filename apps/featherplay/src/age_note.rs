//! A song YouTube marks for adults was passed over (`UiEvent::AgeRestricted`): the first
//! time in a run a dialog says why and offers the fix (ui/age_note.slint): Sign in, or, for
//! an account that isn't age-verified, Google's age verification in the browser. After it,
//! and while the window isn't on screen (the tray, the mini player), a toast says it.

use std::cell::Cell;
use std::rc::Rc;

use featherplay_core::failure::age_restricted;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::toast;
use crate::ui::{AccountPicker, AgeNote, CloseChoice, Frame};

/// Google's age verification, where its help on age-restricted content sends a viewer
/// (support.google.com/accounts?p=verify_age redirects here).
const VERIFY: &str = "https://myaccount.google.com/age-verification";

thread_local! {
    /// The dialog has been shown this run.
    static ASKED: Cell<bool> = const { Cell::new(false) };
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let c = ctx.clone();
    ui.global::<AgeNote>().on_verify(move || {
        featherplay_controller::task::blocking_detached(c.catalog.runtime(), || {
            if let Err(e) = featherplay_platform::open::url(VERIFY) {
                log!("opening age verification: {e}");
            }
        });
    });
}

/// An age-restricted song that was to play was skipped; `signed_in`: the account isn't
/// age-verified.
pub fn skipped(ctx: &Ctx, signed_in: bool) {
    let Some(ui) = ctx.window() else { return };
    let note = ui.global::<AgeNote>();
    // The dialog on screen already says it.
    if note.get_open() {
        return;
    }
    let w = ui.window();
    let on_screen = w.is_visible() && !w.is_minimized() && !ui.global::<Frame>().get_hidden();
    // Another dialog holds the window: a toast, and the dialog waits for a later skip.
    let other = ui.global::<CloseChoice>().get_open() || ui.global::<AccountPicker>().get_open();
    if ASKED.get() || !on_screen || other {
        toast::show(ctx, age_restricted(signed_in));
        return;
    }
    ASKED.set(true);
    log!("age-restricted song skipped: the dialog explains it");
    note.set_signed_in(signed_in);
    note.set_open(true);
}

/// `--snapshot age-dialog-*`: the dialog for that kind.
pub fn show_for_snapshot(ctx: &Ctx, signed_in: bool) {
    let Some(ui) = ctx.window() else { return };
    let note = ui.global::<AgeNote>();
    note.set_signed_in(signed_in);
    note.set_open(true);
}
