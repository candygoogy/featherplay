//! Notifications: none yet (phase 5). `supports(Feature::Notifications)` says so, and
//! `notify` shows nothing, as a missing toast on Windows only logs.

/// Shows nothing yet.
pub fn notify(_title: &str, _text: &str) {}
