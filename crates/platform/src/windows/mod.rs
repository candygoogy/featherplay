//! Windows: the Win32, WinRT and COM code, moved here from the other crates. One file per
//! area; each area's functions live in its own file.

pub mod browsers;
pub mod clipboard;
pub mod credentials;
pub mod files;
pub mod fonts;
pub mod heap;
pub mod identity;
pub mod installer;
pub mod instance;
pub mod media_controls;
pub mod memory;
pub mod notify;
pub mod open;
pub mod process;
pub mod proxy;
pub mod session_end;
pub mod system;
pub mod taskbar;
pub mod thread;
pub mod tray;
pub mod webview;
pub mod window;

// Only the performance harness needs these.
#[cfg(feature = "perf")]
pub mod perf;
