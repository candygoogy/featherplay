//! The performance harness's process and system queries. Stubs until the harness runs on
//! Linux: every count reads 0 and every list is empty, as on Windows when it doesn't say.

use std::time::Duration;

/// A thread's cycles so far. 0: not counted on Linux.
pub fn thread_cycles(_tid: u64) -> u64 {
    0
}

/// A thread's name. "" for now.
pub fn thread_name(_tid: u64) -> String {
    String::new()
}

/// This process's CPU time so far, in ms. 0 for now.
pub fn cpu_ms() -> f64 {
    0.0
}

/// How long ago the process was created. Zero for now.
pub fn since_creation() -> Duration {
    Duration::ZERO
}

/// One thread in [`ProcessSample`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ThreadSample {
    pub id: u64,
    /// Its context switches so far: each one is the thread waking.
    pub switches: u64,
    /// Its CPU time so far, in 100 ns units.
    pub cpu: u64,
}

/// One look at this process.
#[derive(Clone, Debug, Default)]
pub struct ProcessSample {
    /// The CPU time so far, in 100 ns units.
    pub cpu: u64,
    /// The private bytes.
    pub private_bytes: u64,
    pub working_set_bytes: u64,
    /// Each thread.
    pub threads: Vec<ThreadSample>,
}

/// This process's counts. `None` for now.
pub fn process_sample() -> Option<ProcessSample> {
    None
}

/// What the process's heaps hold, in bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Heaps {
    /// Blocks in use.
    pub busy: u64,
    /// The heaps' memory, committed.
    pub committed: u64,
    /// Blocks in use too big for the heap's own segments.
    pub large: u64,
}

/// The process's heaps. Nothing counted for now.
pub fn heap_walk() -> Heaps {
    Heaps::default()
}

/// The process's committed private memory, all of it and the part in regions of 1 MB or
/// more. Nothing counted for now.
pub fn private_regions() -> (u64, u64) {
    (0, 0)
}

/// Would minimise or restore the window. Does nothing yet.
pub fn show_minimised(_h: isize, _minimise: bool) {}

/// The primary screen's size, in physical pixels. Unknown for now: (0, 0).
pub fn screen_size() -> (i32, i32) {
    (0, 0)
}
