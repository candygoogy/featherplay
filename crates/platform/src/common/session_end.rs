//! What a second launch's request to show the window carries, on every OS.

/// A second launch asking this featherplay to show its window (`instance`). The window
/// answers `message` with `SHOWN` (on Windows) when `on_show` returns true, and with 0
/// otherwise.
pub struct Show {
    /// The registered window message that asks.
    pub message: u32,
    /// Runs on the window's thread; returns false if it can't show the window (the app is
    /// closing).
    pub on_show: Box<dyn Fn() -> bool + Send>,
}
