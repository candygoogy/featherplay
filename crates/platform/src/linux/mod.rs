//! Linux. What the app needs to start, play and sign in is here (fonts, clipboard, files,
//! opening links, thread priority, memory, the local time, the end of the session, the
//! Secret Service, the sign-in browsers), and so are MPRIS, notifications, the single-instance
//! guard and helpers that end with the app; the rest is still stubs, whose features `supports`
//! reports as missing and whose functions return `Unsupported`. One file per area, as for
//! Windows; there's no `webview` (WebView2 sign-in is Windows only).

pub mod browsers;
pub(crate) mod bus;
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
pub mod window;

// Only the performance harness needs these.
#[cfg(feature = "perf")]
pub mod perf;
