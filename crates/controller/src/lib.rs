//! featherplay's logic that needs no window (ARCHITECTURE.md, "Phase 3"). The app crate
//! keeps the pages, the UI bridge, the windows and startup, and calls in here.
//!
//! `task` has the detached task helpers everything here spawns with.

pub mod task;
