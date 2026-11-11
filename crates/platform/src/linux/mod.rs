//! Linux: stubs until phase 5. Each area's functions return `Unsupported` until then.
//! One file per area, as for Windows; there's no `webview` (WebView2 sign-in is Windows only).

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
pub mod window;

// Only the performance harness needs these.
#[cfg(feature = "perf")]
pub mod perf;
