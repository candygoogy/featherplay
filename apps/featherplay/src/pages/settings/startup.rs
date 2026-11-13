//! Settings' Window section, its first rows: "Start when you sign in to Windows" (Linux:
//! "…when you log in") and, while that's on, "Start in the tray" ("Start minimised" without
//! a tray). The first is the system's entry (`featherplay_platform::autostart`), read when
//! the page opens, so a change in Windows' Startup apps shows; the second is a setting.
//!
//! Tests and snapshots (the fixtures) never touch the real entry: they point the page at
//! one of their own (`use_entry`), or the switch only moves.

use std::cell::RefCell;

use featherplay_core::tr;
use featherplay_platform::autostart::{Autostart, State, launch_path};

use crate::app::Ctx;
use crate::ui::SettingsData;

thread_local! {
    /// The entry the page reads and writes instead of the user's (`use_entry`).
    static ENTRY: RefCell<Option<Autostart>> = const { RefCell::new(None) };
}

/// Points the page at `entry` instead of the user's own, for tests.
#[doc(hidden)]
pub fn use_entry(entry: Autostart) {
    ENTRY.with(|e| *e.borrow_mut() = Some(entry));
}

/// The entry the page works on: the one `use_entry` set, else the user's, except with the
/// fixtures (tests and snapshots), which get none.
fn entry(ctx: &Ctx) -> Option<Autostart> {
    if let Some(e) = ENTRY.with(|e| e.borrow().clone()) {
        return Some(e);
    }
    if ctx.catalog.fixtures().is_some() {
        return None;
    }
    match Autostart::for_user() {
        Ok(a) => Some(a),
        Err(e) => {
            log!("start with the system: {e}");
            None
        }
    }
}

/// The page opens: the switch shows whether featherplay will start at sign-in, as the system
/// sees it now. Off when Windows' Startup apps (or the desktop's settings) turned it off.
pub(super) fn show(ctx: &Ctx, data: &SettingsData) {
    if let Some(a) = entry(ctx) {
        let state = a.state();
        if state == State::TurnedOffBySystem {
            log!("start with the system: the entry is there, turned off by the system");
        }
        data.set_start_with_system(state.on());
    }
}

/// The switch was flipped: the entry is made (pointing at this copy) or taken away. If that
/// fails, the switch goes back and a toast says so.
pub(super) fn toggled(ctx: &Ctx, data: &SettingsData, on: bool) {
    data.set_start_with_system(on);
    let Some(a) = entry(ctx) else { return };
    let done = if on {
        launch_path().and_then(|exe| a.enable(&exe))
    } else {
        a.disable()
    };
    match done {
        Ok(()) => log!("start with the system: {}", if on { "on" } else { "off" }),
        Err(e) => {
            log!("start with the system: {e}");
            data.set_start_with_system(a.state().on());
            ctx.toast_error(if on {
                tr!("featherplay couldn't add itself to the apps that start when you sign in.")
            } else {
                tr!("featherplay couldn't remove itself from the apps that start when you sign in.")
            });
        }
    }
}
