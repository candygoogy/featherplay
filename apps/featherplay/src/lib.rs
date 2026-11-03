//! featherplay: a light native player for the YouTube Music catalogue, in Rust and Slint.
//!
//! Threads: Slint owns the main thread (`app`, the window, the integrations in `shell`).
//! The controller runs on `featherplay-controller` and shares one tokio runtime with the
//! UI's background loads (`catalog`, `covers`).
//!
//! The window: `app` (context, startup), `router` (pages and history), `pages` (one module
//! per page), `sidebar`, `top_bar` (search), `bar`, `queue_panel`, `ui_bridge` (model to Slint
//! rows). `mini` is the tray's mini player, a window of its own (`popup` places it); the tray
//! icon's hover text and right-click menu are Windows' own (`shell`). The Slint side is in `ui/`;
//! `ui/README.md` says which file owns what.

/// Writes a line to the log file (`diagnostics`), prefixed with the time and the seconds
/// since start; debug builds print it to stderr too.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::diagnostics::write(format_args!($($arg)*))
    };
}

/// The Slint components, globals and structs (`ui/app.slint`).
pub mod ui {
    slint::include_modules!();
}

pub mod account_menu;
pub mod app;
pub mod bar;
pub mod busy;
pub mod catalog;
pub mod close;
pub mod controller;
pub mod covers;
pub mod diagnostics;
pub mod endsession;
pub mod fixtures;
pub mod follow;
pub mod fonts;
pub mod identity;
pub mod instance;
pub mod language;
pub mod library_list;
pub mod link;
pub mod lyrics;
pub mod memory;
pub mod mini;
pub mod mouse_nav;
pub mod native;
pub mod not_for_me;
pub mod offscreen;
pub mod pages;
pub mod playlist_menu;
pub mod popup;
pub mod presence;
pub mod queue_panel;
pub mod repaint;
pub mod router;
pub mod scrobbler;
pub mod shell;
pub mod sidebar;
pub mod snapshot;
pub mod song_menu;
pub mod toast;
pub mod top_bar;
pub mod ui_bridge;
pub mod updates;
pub mod wheel;
