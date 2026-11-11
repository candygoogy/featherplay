//! Thread priority and thread CPU time. Stubs until phase 5: no thread's priority changes.

/// The calling thread's real-time audio registration. Never made here yet.
pub struct ProAudio {}

/// Would put the calling thread in the real-time range for audio. `None`: the thread stays
/// as it was.
pub fn pro_audio() -> Option<ProAudio> {
    None
}

/// Would raise the calling thread above ordinary threads. Leaves it as it is.
pub fn raise_priority() {}

/// Would lower the calling thread below ordinary threads. Leaves it as it is.
pub fn lower_priority() {}

/// This thread's user and kernel CPU time so far. `None`: not read here yet (phase 5).
pub fn user_and_kernel_time() -> Option<(std::time::Duration, std::time::Duration)> {
    None
}

/// This thread's CPU time so far, from the scheduler's count (`/proc/thread-self/schedstat`,
/// in nanoseconds). `None` where the kernel doesn't keep it.
#[cfg(feature = "perf")]
pub fn cpu_time() -> Option<std::time::Duration> {
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    let nanos = stat.split_whitespace().next()?.parse().ok()?;
    Some(std::time::Duration::from_nanos(nanos))
}

/// The cycles this thread has run so far. `None`: not read on Linux.
#[cfg(feature = "perf")]
pub fn cycles() -> Option<u64> {
    None
}

/// The cycles this process has run so far. `None`: not read on Linux.
#[cfg(feature = "perf")]
pub fn process_cycles() -> Option<u64> {
    None
}

/// Would raise this whole process's priority. Leaves it as it is for now.
#[cfg(feature = "perf")]
pub fn raise_process_priority() {}
