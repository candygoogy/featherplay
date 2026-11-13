//! What a second launch's request to the running featherplay carries, on every OS.

/// What a second launch asks of the running featherplay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Show the main window (an ordinary launch).
    Show,
    /// Open the quick search box (`featherplay --quick-search`, a desktop's own shortcut
    /// where the app can't hear keys pressed in other apps).
    QuickSearch,
}

/// A second launch asking this featherplay to show its window or open the quick search
/// (`instance`). The window answers `message` with `SHOWN` (on Windows) when `on_show`
/// returns true, and with 0 otherwise.
pub struct Show {
    /// The registered window message that asks.
    pub message: u32,
    /// Runs on the window's thread with what was asked; returns false if it can't be done
    /// (the app is closing).
    pub on_show: Box<dyn Fn(Ask) -> bool + Send>,
}
