//! Private bytes and heap compaction.

/// The process's commit charge now and at its peak. `None` until phase 5.
#[cfg(feature = "perf")]
pub fn commit_charge() -> Option<(usize, usize)> {
    None
}
