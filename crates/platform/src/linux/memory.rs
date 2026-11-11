//! Private bytes and heap compaction. Stubs until phase 5.

/// This process's private bytes. 0 until phase 5 reads them, as on Windows when it
/// doesn't say.
pub fn private_bytes() -> u64 {
    0
}

/// Does nothing yet.
pub fn compact_heaps() {}

/// The process's commit charge now and at its peak. `None` until phase 5.
#[cfg(feature = "perf")]
pub fn commit_charge() -> Option<(usize, usize)> {
    None
}

/// The heaps' allocated and committed bytes. Nothing counted until phase 5.
#[cfg(feature = "perf")]
pub fn heap_summary() -> (usize, usize) {
    (0, 0)
}

/// The committed private bytes of each allocation. None listed until phase 5.
#[cfg(feature = "perf")]
pub fn private_allocations() -> Vec<usize> {
    Vec::new()
}
