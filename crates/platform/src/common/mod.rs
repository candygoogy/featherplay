//! What every OS's version of an area shares: the plain types its functions take and the
//! events they report, and logic with no OS calls in it. Each OS's area file re-exports its
//! part (`pub use crate::common::tray::*`), so it's defined once and named the same way
//! everywhere.

pub mod taskbar;
pub mod tray;
