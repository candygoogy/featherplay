//! Thread priority (MMCSS "Pro Audio", the decoders, the scans) and thread CPU time.
//!
//! cpal runs its WASAPI thread at normal priority (its `realtime` feature, off here, would
//! pull in another crate for this), so with every core busy the output callback waited
//! its turn among the other normal threads and came after the ~50 ms device buffer had
//! run dry. The output thread joins MMCSS's "Pro Audio" task, as audio apps do: Windows
//! runs it in the real-time range, ahead of ordinary work. The decoder threads, which
//! decode ~1.5 s ahead in bursts, run at the highest normal priority.

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, GetCurrentThread,
    SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_HIGHEST,
};
use windows::core::w;

/// The calling thread's MMCSS registration; dropping it leaves the task.
pub struct ProAudio {
    /// The MMCSS handle's value: a `HANDLE` isn't `Send`, and cpal moves the callback that
    /// holds this onto its thread before the first call.
    handle: usize,
}

/// Registers the calling thread with MMCSS's "Pro Audio" task. `None` if MMCSS refused (its
/// service is off), and the thread stays as it was.
pub fn pro_audio() -> Option<ProAudio> {
    let mut task = 0u32;
    // SAFETY: the task name is a static NUL-terminated string and `task` outlives the
    // call; the handle returned is owned by `ProAudio` and released once in `drop`.
    let handle = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task) }.ok()?;
    Some(ProAudio {
        handle: handle.0 as usize,
    })
}

impl Drop for ProAudio {
    fn drop(&mut self) {
        // SAFETY: the handle came from `AvSetMmThreadCharacteristicsW` and is released
        // only here. Fails only if the thread already left the task, which is harmless.
        let _ = unsafe { AvRevertMmThreadCharacteristics(HANDLE(self.handle as *mut _)) };
    }
}

/// Raises the calling thread to the highest priority of the normal range: above every
/// ordinary thread, below the real-time range (MMCSS, the output thread). For the decoders.
pub fn raise_priority() {
    // SAFETY: `GetCurrentThread` is a pseudo-handle for the calling thread, valid for
    // the call. A failure leaves the thread at normal priority, as before.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) };
}

/// Lowers the calling thread below normal priority: background work (skip silence's scan)
/// that every ordinary thread, the UI's included, goes ahead of.
pub fn lower_priority() {
    // SAFETY: `GetCurrentThread` is a pseudo-handle for the calling thread, valid for
    // the call. A failure leaves the thread at normal priority.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
}

/// This thread's CPU time so far, in the 15.6 ms steps Windows charges it in. `None` if
/// Windows didn't say.
#[cfg(feature = "perf")]
pub fn cpu_time() -> Option<std::time::Duration> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::GetThreadTimes;

    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    // SAFETY: the pseudo-handle of the current thread is always valid, and the four
    // pointers are to live locals.
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    ok.ok()?;
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    // 100 ns ticks.
    Some(std::time::Duration::from_nanos(
        (ticks(kernel) + ticks(user)) * 100,
    ))
}

/// The cycles this thread has run so far. `None` if Windows didn't say.
#[cfg(feature = "perf")]
pub fn cycles() -> Option<u64> {
    use windows::Win32::System::WindowsProgramming::QueryThreadCycleTime;
    let mut c = 0u64;
    // SAFETY: the pseudo-handle of the current thread is always valid, and the pointer
    // is to a live local.
    unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut c) }.ok()?;
    Some(c)
}

/// The cycles this process has run so far, its ended threads' included. `None` if Windows
/// didn't say.
#[cfg(feature = "perf")]
pub fn process_cycles() -> Option<u64> {
    use windows::Win32::System::Threading::GetCurrentProcess;
    use windows::Win32::System::WindowsProgramming::QueryProcessCycleTime;
    let mut c = 0u64;
    // SAFETY: the pseudo-handle of the current process is always valid, and the pointer is
    // to a live local.
    unsafe { QueryProcessCycleTime(GetCurrentProcess(), &mut c) }.ok()?;
    Some(c)
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::Threading::{GetCurrentThread, GetThreadPriority};

    fn priority() -> i32 {
        // SAFETY: the pseudo-handle of the calling thread, valid for the call.
        unsafe { GetThreadPriority(GetCurrentThread()) }
    }

    #[test]
    fn pro_audio_lifts_the_thread_into_the_real_time_range_until_dropped() {
        std::thread::spawn(|| {
            let before = priority();
            let task = super::pro_audio().expect("MMCSS registration");
            // MMCSS raises the thread's actual priority, which `GetThreadPriority` reports
            // as THREAD_PRIORITY_TIME_CRITICAL (15) in a normal-priority process.
            assert!(priority() > before, "priority {} after joining", priority());
            drop(task);
            assert_eq!(priority(), before);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_decoder_thread_runs_above_normal_threads() {
        std::thread::spawn(|| {
            super::raise_priority();
            assert_eq!(priority(), 2); // THREAD_PRIORITY_HIGHEST
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_scan_thread_runs_below_normal_threads() {
        std::thread::spawn(|| {
            super::lower_priority();
            assert_eq!(priority(), -1); // THREAD_PRIORITY_BELOW_NORMAL
        })
        .join()
        .unwrap();
    }
}
