//! Counts background work whose result the UI is still waiting for (page loads, covers),
//! so `--snapshot` can wait until everything has arrived before it renders.

use std::sync::atomic::{AtomicUsize, Ordering};

static PENDING: AtomicUsize = AtomicUsize::new(0);
/// Of those, the ones waiting on the UI thread's timers rather than on other threads.
static ON_TIMERS: AtomicUsize = AtomicUsize::new(0);

/// Held from the start of a load until its result is applied on the UI thread.
pub struct Busy {
    on_timers: bool,
}

impl Busy {
    pub fn start() -> Busy {
        PENDING.fetch_add(1, Ordering::SeqCst);
        Busy { on_timers: false }
    }

    /// For work that a timer on the UI thread moves on (a lookup's wait, lines handed over a
    /// few a tick), not another thread: the tests' frozen clock has to move for it to end.
    pub fn on_timers() -> Busy {
        ON_TIMERS.fetch_add(1, Ordering::SeqCst);
        PENDING.fetch_add(1, Ordering::SeqCst);
        Busy { on_timers: true }
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        let _was = PENDING.fetch_sub(1, Ordering::SeqCst);
        if self.on_timers {
            ON_TIMERS.fetch_sub(1, Ordering::SeqCst);
        }
        #[cfg(feature = "perf-harness")]
        if _was == 1 && crate::perf::on() {
            crate::perf::idle();
        }
    }
}

/// Loads started and not applied yet.
pub fn pending() -> usize {
    PENDING.load(Ordering::SeqCst)
}

/// Loads still waiting on other threads (pages, covers): the ones a frozen clock doesn't move.
pub fn on_threads() -> usize {
    PENDING
        .load(Ordering::SeqCst)
        .saturating_sub(ON_TIMERS.load(Ordering::SeqCst))
}
