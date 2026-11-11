//! Everything in featherplay that talks to the operating system, behind plain functions and
//! types. One module per OS, picked at compile time and re-exported here, so callers write
//! `featherplay_platform::tray::…` on every OS. `windows` holds the Windows code; `linux` is
//! stubs until phase 5 (ARCHITECTURE.md, "Target structure"). A `macos` module would go
//! beside them, picked the same way.
//!
//! A feature an OS lacks says so: [`supports`] returns false for it, and its functions
//! return [`Unsupported`] rather than doing nothing.
//!
//! What an area's versions share (its plain types and events, logic with no OS calls) is in
//! `common`, which each OS's area file re-exports.
#![cfg_attr(test, allow(clippy::print_stderr, reason = "tests report skips"))]

mod common;
mod feature;

pub use crate::feature::{Feature, Unsupported, supports};

// `crate::` paths, because a bare `windows` here would also name the `windows` crate.
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use crate::windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use crate::linux::*;

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!(
    "featherplay-platform has no module for this OS: add one beside windows/ and linux/"
);
