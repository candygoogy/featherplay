//! Starting with the system (Settings > Window, "Start when you sign in to Windows" and
//! "Start in the tray"). The entry itself is `featherplay_platform::autostart`'s: the HKCU
//! Run value on Windows, the XDG autostart file on Linux. Whether featherplay starts at
//! sign-in is read from there, never kept in the settings file, so Windows' Startup apps and
//! the desktop's own settings stay the truth.
//!
//! A start at sign-in carries `--autostart`; nothing else makes one. It restores the session
//! paused, as every start does, and:
//! - with "Start in the tray" on, shows no window: only the tray icon (the media controls and
//!   the tray's menu know the song). The first page isn't opened, so Home isn't loaded, until
//!   the window is first shown (`window_shown`). With no tray to go to, the window starts
//!   minimised instead.
//! - with it off, the window opens as usual but doesn't take the focus from what the user
//!   opened first (`window_attributes`).

use std::cell::Cell;

use featherplay_platform::autostart::{Autostart, launch_path};
use featherplay_platform::identity;
use slint::winit_030::winit::window::WindowAttributes;

use crate::app::Ctx;

/// How this start shows the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Shown, as when the user starts featherplay.
    Window,
    /// Shown, without taking the focus: a start at sign-in.
    Quiet,
    /// Not shown, only the tray icon: a start at sign-in with "Start in the tray" on.
    Tray,
}

impl Start {
    /// How a start with `--autostart` (or without) shows the window, "Start in the tray"
    /// being `in_tray`.
    pub fn new(autostart: bool, in_tray: bool) -> Start {
        match (autostart, in_tray) {
            (false, _) => Start::Window,
            (true, false) => Start::Quiet,
            (true, true) => Start::Tray,
        }
    }
}

thread_local! {
    /// The next window made shows without taking the focus (`Start::Quiet`): set just
    /// before the main window is made, used once.
    static QUIET: Cell<bool> = const { Cell::new(false) };
    /// The window hasn't been shown yet in a start in the tray: its first page opens with it.
    static PAGE_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Before the main window is made: how this start shows it.
pub fn prepare(start: Start) {
    QUIET.set(start == Start::Quiet);
    PAGE_PENDING.set(start == Start::Tray);
}

/// The winit attributes of a window about to be made: the main window of a quiet start
/// shows without taking the focus (Windows: shown with SW_SHOWNOACTIVATE the first time).
pub fn window_attributes(attrs: WindowAttributes) -> WindowAttributes {
    if QUIET.replace(false) {
        attrs.with_active(false)
    } else {
        attrs
    }
}

/// The window is about to show: in a start in the tray, the first time, its page opens now.
pub fn window_shown(ctx: &std::rc::Rc<Ctx>) {
    if PAGE_PENDING.replace(false) {
        log!("first show after a start in the tray: opening the page");
        ctx.show_current();
    }
}

/// Whether the window's first page waits for its first show (a start in the tray).
pub fn page_pending() -> bool {
    PAGE_PENDING.get()
}

/// At every start of a normal install (`identity::registration_allowed_here`: not a dev
/// build, no `FEATHERPLAY_HOME`): an entry that's there is pointed at this copy if it names
/// another path. One registry read (or file read) when nothing moved; nothing when the
/// setting is off.
pub fn refresh_entry() {
    if !identity::registration_allowed_here() {
        return;
    }
    let refreshed = Autostart::for_user().and_then(|a| a.refresh(&launch_path()?));
    match refreshed {
        Ok(true) => log!("start with the system: the entry now points at this copy"),
        Ok(false) => {}
        Err(e) => log!("start with the system: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_flag_makes_a_start_at_sign_in() {
        assert_eq!(Start::new(false, false), Start::Window);
        assert_eq!(Start::new(false, true), Start::Window);
        assert_eq!(Start::new(true, false), Start::Quiet);
        assert_eq!(Start::new(true, true), Start::Tray);
    }
}
