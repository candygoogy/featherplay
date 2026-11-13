//! featherplay: a light native player for the YouTube Music catalogue, in Rust and Slint.
//!
//! Threads: Slint owns the main thread (`app`, the window, the integrations in `shell`).
//! The controller runs on `featherplay-controller` and shares one tokio runtime with the
//! UI's background loads (the controller crate's `catalog`, `covers`).
//!
//! The window: `app` (context, startup), `router` (pages and history), `pages` (one module
//! per page), `sidebar`, `top_bar` (search), `bar`, `queue_panel`, `ui_bridge` (model to Slint
//! rows). `mini` is the tray's mini player and `quick_search` the quick search box, windows of
//! their own (`popup` places the mini player); the tray
//! icon's hover text and right-click menu are Windows' own (`shell`). The Slint side is in `ui/`;
//! `ui/README.md` says which file owns what.

/// `log!` writes a line to the log file (the controller crate's `diagnostics`), in every
/// module here.
#[macro_use]
extern crate featherplay_controller;

/// The Slint components, globals and structs (`ui/app.slint`).
pub mod ui {
    slint::include_modules!();
}

pub mod account_menu;
pub mod account_picker;
pub mod album_actions;
pub mod app;
pub mod autostart;
pub mod bar;
pub mod busy;
pub mod chrome;
pub mod close;
pub mod covers;
pub mod desktop_lyrics;
pub mod follow;
pub mod fonts;
pub mod frame_log;
pub mod identity;
pub mod instance;
pub mod language;
pub mod library_list;
pub mod link;
pub mod live_resize;
pub mod lyrics;
pub mod memory;
pub mod mini;
pub mod mouse_nav;
pub mod not_for_me;
pub mod offscreen;
pub mod pages;
#[cfg(feature = "perf-harness")]
pub mod perf;
pub mod playlist_menu;
pub mod popup;
pub mod prepare;
pub mod queue_panel;
pub mod quick_search;
pub mod refused;
pub mod repaint;
pub mod router;
pub mod shell;
pub mod sidebar;
pub mod sleep;
pub mod snapshot;
pub mod song_menu;
pub mod suggest;
pub mod toast;
pub mod top_bar;
pub mod ui_bridge;
pub mod updates;
pub mod wheel;

/// The controller crate's interface (`Action`, `UiEvent`, `PlayerState`, `Options`, `UiLink`),
/// under the name the pages have always used.
pub use featherplay_controller::controller;
