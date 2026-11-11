//! Private bytes and heap compaction.

/// The process's commit charge now and at its peak, in bytes. `None` if Windows didn't say.
#[cfg(feature = "perf")]
pub fn commit_charge() -> Option<(usize, usize)> {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut c = PROCESS_MEMORY_COUNTERS::default();
    // SAFETY: the pseudo-handle of the current process is always valid, and `c` is a live
    // local of the size passed.
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut c,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    }
    .ok()?;
    Some((c.PagefileUsage, c.PeakPagefileUsage))
}
