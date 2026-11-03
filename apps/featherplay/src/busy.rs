//! Counts background work whose result the UI is still waiting for (page loads, covers),
//! so `--snapshot` can wait until everything has arrived before it renders.

use std::sync::atomic::{AtomicUsize, Ordering};

static PENDING: AtomicUsize = AtomicUsize::new(0);

/// Held from the start of a load until its result is applied on the UI thread.
pub struct Busy(());

impl Busy {
    pub fn start() -> Busy {
        PENDING.fetch_add(1, Ordering::SeqCst);
        Busy(())
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        PENDING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Loads started and not applied yet.
pub fn pending() -> usize {
    PENDING.load(Ordering::SeqCst)
}
