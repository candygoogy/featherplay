//! Private bytes and heap compaction.

/// This process's private bytes (what Task Manager calls the commit size).
pub fn private_bytes() -> u64 {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_nonpaged: usize,
        quota_nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
    }
    let mut c = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: this process's pseudo-handle and a counters struct of the size it says.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    if ok == 0 { 0 } else { c.private as u64 }
}

/// Hands the heaps' free pages back to Windows. What was just let go is mostly free blocks
/// inside the heaps (a cover is under the heap's 512 KB threshold for a block of its own), and
/// a heap keeps free pages committed, so they'd still count as private memory.
pub fn compact_heaps() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetProcessHeaps(count: u32, heaps: *mut isize) -> u32;
        fn HeapCompact(heap: isize, flags: u32) -> usize;
    }
    let mut heaps = [0isize; 32];
    // SAFETY: a buffer of `heaps.len()` handles; the call fills at most that many.
    let n = unsafe { GetProcessHeaps(heaps.len() as u32, heaps.as_mut_ptr()) } as usize;
    for &heap in &heaps[..n.min(heaps.len())] {
        // SAFETY: a heap of this process, from GetProcessHeaps.
        unsafe { HeapCompact(heap, 0) };
    }
}

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
