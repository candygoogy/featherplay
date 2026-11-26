//! The display's refresh, for an animation the app paces itself: not known here, so a frame
//! comes as soon as it's asked for.

use std::time::{Duration, Instant};

/// Runs `then` at once.
pub fn after_composition(then: impl FnOnce() + Send + 'static) {
    then();
}

/// Not known: `None`.
pub fn next_refresh(_ahead: Duration) -> Option<Instant> {
    None
}
