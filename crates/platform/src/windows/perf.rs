//! The performance harness's process times, system queries, heap walks and window messages.
//!
//! Windows charges a thread's time (GetProcessTimes, the system's process list, Task Manager)
//! a whole clock tick (15.6 ms) at a time, to the thread running when the tick comes. A
//! thread that wakes 100 times a second for 40 µs, as the audio output's does, then reads 0
//! for a while, then a tick per wake while its wakes line up with the tick: up to 4 % of a
//! core for 10-20 s, every minute or two, as the sound device's clock drifts against the
//! system's, though its real cost stays ~0.4 %. Cycles are counted at every switch, so the
//! harness's spells also report them ([`thread_cycles`], `thread::process_cycles`), at the
//! TSC's rate (QueryThreadCycleTime counts in it).

use std::time::Duration;

use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN, SW_MINIMIZE, SW_RESTORE, ShowWindow,
};

use crate::window::Handle;

// Declared here as the harness always had them, rather than turning on more `windows`
// features for a build that's never published.
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> isize;
    fn GetCurrentProcessId() -> u32;
    fn GetProcessTimes(
        process: isize,
        creation: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
    fn GetSystemTimePreciseAsFileTime(now: *mut u64);
    fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> isize;
    fn GetThreadDescription(thread: isize, description: *mut *mut u16) -> i32;
    fn LocalFree(memory: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    fn CloseHandle(handle: isize) -> i32;
    fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(class: u32, info: *mut u8, len: u32, returned: *mut u32) -> i32;
}

const THREAD_QUERY_LIMITED_INFORMATION: u32 = 0x0800;

/// A thread's cycles so far, or 0. `tid` is a thread of this process.
pub fn thread_cycles(tid: u64) -> u64 {
    // SAFETY: a thread id of this process; a null handle is checked before use.
    let h = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, tid as u32) };
    if h == 0 {
        return 0;
    }
    let mut c = 0u64;
    // SAFETY: a live thread handle and a valid out pointer.
    unsafe { QueryThreadCycleTime(h, &mut c) };
    // SAFETY: the handle opened above.
    unsafe { CloseHandle(h) };
    c
}

/// A thread's name (`GetThreadDescription`: Rust's thread names, tokio's workers), or "".
pub fn thread_name(tid: u64) -> String {
    // SAFETY: a thread id of this process; a null handle is checked before use.
    let h = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, tid as u32) };
    if h == 0 {
        return String::new();
    }
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: a live thread handle and a valid out pointer; the string is freed below.
    let ok = unsafe { GetThreadDescription(h, &mut text) } >= 0;
    let mut name = String::new();
    if ok && !text.is_null() {
        // SAFETY: a NUL-terminated wide string from the system, read up to its NUL.
        let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
        // SAFETY: `len` wide characters before the NUL.
        name = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
        // SAFETY: allocated by GetThreadDescription with LocalAlloc.
        unsafe { LocalFree(text.cast()) };
    }
    // SAFETY: the handle opened above.
    unsafe { CloseHandle(h) };
    name
}

/// This process's CPU time so far, in ms.
pub fn cpu_ms() -> f64 {
    let (mut creation, mut exit, mut kernel, mut user) = (0, 0, 0, 0);
    // SAFETY: the process pseudo-handle and four valid, aligned out pointers (a FILETIME is a
    // little-endian u64 in two halves).
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    (kernel + user) as f64 / 1e4
}

/// How long ago the process was created.
pub fn since_creation() -> Duration {
    let (mut creation, mut exit, mut kernel, mut user, mut now) = (0u64, 0, 0, 0, 0u64);
    // SAFETY: as in `cpu_ms`; the clock writes one FILETIME.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        );
        GetSystemTimePreciseAsFileTime(&mut now);
    }
    Duration::from_nanos(now.saturating_sub(creation) * 100)
}

/// One thread in [`ProcessSample`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ThreadSample {
    pub id: u64,
    /// Its context switches so far: each one is the thread waking.
    pub switches: u64,
    /// Its CPU time so far, in 100 ns units, as Windows charges it.
    pub cpu: u64,
}

/// One look at this process in the system's process list.
#[derive(Clone, Debug, Default)]
pub struct ProcessSample {
    /// The CPU time so far, in 100 ns units, as Windows charges it.
    pub cpu: u64,
    /// The commit charge (private bytes).
    pub private_bytes: u64,
    pub working_set_bytes: u64,
    /// Each thread, in the list's order.
    pub threads: Vec<ThreadSample>,
}

/// `SYSTEM_PROCESS_INFORMATION` (x64): the offsets read here.
const SPI_THREADS: usize = 4;
const SPI_USER: usize = 40;
const SPI_KERNEL: usize = 48;
const SPI_PID: usize = 80;
const SPI_WORKING_SET: usize = 144;
const SPI_PAGEFILE: usize = 184;
const SPI_SIZE: usize = 256;
/// `SYSTEM_THREAD_INFORMATION` (x64): its size and the context switches' offset.
const STI_SIZE: usize = 80;
const STI_SWITCHES: usize = 64;
/// Its `ClientId.UniqueThread`.
const STI_THREAD_ID: usize = 48;
/// Its kernel and user times.
const STI_KERNEL: usize = 0;
const STI_USER: usize = 8;

/// This process, from the system's process list. `None` if the list couldn't be read or
/// this process isn't in it.
pub fn process_sample() -> Option<ProcessSample> {
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let mut returned = 0u32;
        // SAFETY: a writable buffer of the length given; class 5 is SystemProcessInformation.
        let status = unsafe {
            NtQuerySystemInformation(5, buf.as_mut_ptr(), buf.len() as u32, &mut returned)
        };
        if status == 0xC000_0004_u32 as i32 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if status != 0 {
            return None;
        }
        break;
    }
    let u32_at =
        |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap_or_default());
    let u64_at =
        |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or_default());
    // SAFETY: no arguments.
    let me = u64::from(unsafe { GetCurrentProcessId() });
    let mut at = 0usize;
    loop {
        let entry = &buf[at..];
        if u64_at(entry, SPI_PID) == me {
            let threads = u32_at(entry, SPI_THREADS);
            let threads = (0..threads as usize)
                .map(|i| {
                    let t = SPI_SIZE + i * STI_SIZE;
                    ThreadSample {
                        id: u64_at(entry, t + STI_THREAD_ID),
                        switches: u64::from(u32_at(entry, t + STI_SWITCHES)),
                        cpu: u64_at(entry, t + STI_KERNEL) + u64_at(entry, t + STI_USER),
                    }
                })
                .collect();
            return Some(ProcessSample {
                cpu: u64_at(entry, SPI_USER) + u64_at(entry, SPI_KERNEL),
                private_bytes: u64_at(entry, SPI_PAGEFILE),
                working_set_bytes: u64_at(entry, SPI_WORKING_SET),
                threads,
            });
        }
        let next = u32_at(entry, 0) as usize;
        if next == 0 {
            return None;
        }
        at += next;
    }
}

/// What the process's heaps hold, in bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Heaps {
    /// Blocks in use, Rust's and C/C++'s alike (a low-fragmentation bucket counts whole).
    pub busy: u64,
    /// The heaps' segments, committed: blocks in use and free blocks alike.
    pub committed: u64,
    /// Blocks in use too big for a segment, each its own allocation.
    pub large: u64,
}

/// The process's heaps walked.
pub fn heap_walk() -> Heaps {
    /// `PROCESS_HEAP_ENTRY` (x64); for a region, `rest[0]` holds its committed size in its
    /// low half.
    #[repr(C)]
    struct Entry {
        data: *mut u8,
        size: u32,
        overhead: u8,
        region: u8,
        flags: u16,
        rest: [usize; 3],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetProcessHeaps(count: u32, heaps: *mut isize) -> u32;
        fn HeapLock(heap: isize) -> i32;
        fn HeapUnlock(heap: isize) -> i32;
        fn HeapWalk(heap: isize, entry: *mut Entry) -> i32;
    }
    const REGION: u16 = 0x1;
    const BUSY: u16 = 0x4;
    /// A block this big is no segment's on x64: Windows allocates it on its own.
    const LARGE: u32 = 1016 * 1024;
    let mut heaps = [0isize; 64];
    // SAFETY: a buffer of `heaps.len()` handles; the call fills at most that many.
    let n = unsafe { GetProcessHeaps(heaps.len() as u32, heaps.as_mut_ptr()) } as usize;
    let mut out = Heaps::default();
    for &heap in &heaps[..n.min(heaps.len())] {
        // SAFETY: a heap of this process, locked while it's walked; the walk starts from a
        // null `data` and is handed back the entry it filled.
        unsafe {
            if HeapLock(heap) == 0 {
                continue;
            }
            let mut e = Entry {
                data: std::ptr::null_mut(),
                size: 0,
                overhead: 0,
                region: 0,
                flags: 0,
                rest: [0; 3],
            };
            while HeapWalk(heap, &mut e) != 0 {
                if e.flags & REGION != 0 {
                    out.committed += (e.rest[0] & 0xFFFF_FFFF) as u64;
                } else if e.flags & BUSY != 0 {
                    out.busy += u64::from(e.size);
                    if e.size >= LARGE {
                        out.large += u64::from(e.size);
                    }
                }
            }
            HeapUnlock(heap);
        }
    }
    out
}

/// The process's committed private memory, from its address space: all of it, and the
/// part in regions of 1 MB or more.
pub fn private_regions() -> (u64, u64) {
    /// `MEMORY_BASIC_INFORMATION` (x64).
    #[repr(C)]
    #[derive(Default)]
    struct Info {
        base: usize,
        allocation_base: usize,
        allocation_protect: u32,
        partition: u32,
        size: usize,
        state: u32,
        protect: u32,
        kind: u32,
        pad: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualQuery(at: usize, info: *mut Info, len: usize) -> usize;
    }
    const COMMIT: u32 = 0x1000;
    const PRIVATE: u32 = 0x2_0000;
    let (mut all, mut big) = (0u64, 0u64);
    let mut at = 0usize;
    loop {
        let mut info = Info::default();
        // SAFETY: an address of this process and a struct of the size given.
        let got = unsafe { VirtualQuery(at, &mut info, std::mem::size_of::<Info>()) };
        if got == 0 || info.size == 0 {
            break;
        }
        if info.state == COMMIT && info.kind == PRIVATE {
            all += info.size as u64;
            if info.size >= 1 << 20 {
                big += info.size as u64;
            }
        }
        let Some(next) = info.base.checked_add(info.size) else {
            break;
        };
        at = next;
    }
    (all, big)
}

/// Minimises the window (`minimise`) or restores it, directly (ShowWindow), not as its
/// title bar's buttons do. `h` is a live window of the calling thread.
pub fn show_minimised(h: Handle, minimise: bool) {
    let how = if minimise { SW_MINIMIZE } else { SW_RESTORE };
    // SAFETY: the window's live handle, on its own thread.
    let _ = unsafe { ShowWindow(h.hwnd(), how) };
}

/// The primary screen's size, in physical pixels.
pub fn screen_size() -> (i32, i32) {
    // SAFETY: no arguments that could be wrong.
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}
