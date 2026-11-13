//! Starting featherplay when the user signs in (Settings > Window, "Start when you sign in to
//! Windows" / "Start when you log in"): what each OS's version shares.

/// The flag the entry starts featherplay with, so a start at sign-in can tell itself apart
/// from one by hand (it may stay in the tray, and never takes the focus).
pub const FLAG: &str = "--autostart";

/// Whether featherplay starts when the user signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No entry.
    Off,
    /// The entry is there and the system runs it.
    On,
    /// The entry is there, but the system's own list of startup apps (Windows' Startup apps,
    /// the desktop's autostart settings) has it turned off. The app reads this as off and
    /// leaves the system's choice alone.
    TurnedOffBySystem,
}

impl State {
    /// Whether featherplay will start at the next sign-in.
    pub fn on(self) -> bool {
        self == State::On
    }
}
