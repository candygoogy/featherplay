//! The process: its CPU, threads and memory, as the system and the allocator count them.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use featherplay_platform::perf::{thread_cycles, thread_name};
use serde_json::{Map, Value, json};

use super::report::round;

// Windows charges a thread's time a clock tick at a time; the spells also report cycles
// (`featherplay_platform::perf` says why).

/// This process's cycles so far, its ended threads' included, or 0.
fn process_cycles() -> u64 {
    featherplay_platform::thread::process_cycles().unwrap_or(0)
}

/// The time-stamp counter, the clock the cycle counts are in on x86-64. Elsewhere (ARM64
/// counts cycles in other units) 0, and the spells report no cycles.
fn tsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: rdtsc has no preconditions on x86-64.
    return unsafe { core::arch::x86_64::_rdtsc() };
    #[cfg(not(target_arch = "x86_64"))]
    return 0;
}

/// One look at the process from the system's process list.
#[derive(Clone, Default)]
pub(super) struct Sample {
    pub(super) cpu_ms: f64,
    pub(super) private_mb: f64,
    pub(super) working_set_mb: f64,
    /// Each thread's id, its context switches so far (each one is the thread waking) and its
    /// CPU time so far (100 ns units).
    switches: Vec<(u64, u64, u64)>,
    pub(super) threads: u32,
    /// Each thread's id and cycles so far; the process's cycles; the TSC.
    cycles: Vec<(u64, u64)>,
    pub(super) proc_cycles: u64,
    pub(super) tsc: u64,
}

impl Sample {
    /// The context switches from `self` to `later`, of the threads alive at `later` (a thread
    /// that ended in between is lost; one that began counts from zero).
    pub(super) fn switches_until(&self, later: &Sample) -> u64 {
        later
            .switches
            .iter()
            .map(|(tid, n, _)| {
                let before = self
                    .switches
                    .iter()
                    .find(|(t, _, _)| t == tid)
                    .map_or(0, |(_, n, _)| *n);
                n.saturating_sub(before)
            })
            .sum()
    }

    /// The threads that used the most cycles from `self` to `later`: name (or id), CPU ms
    /// as Windows charges it, CPU ms from the cycles, and wakeups per second over `secs`.
    pub(super) fn busiest_until(&self, later: &Sample, secs: f64) -> Vec<Value> {
        // No rate without both TSC readings (a failed sample, an ARM64 build): no cycles.
        let tsc_per_ms = if self.tsc > 0 {
            later.tsc.saturating_sub(self.tsc) as f64 / (secs * 1000.0)
        } else {
            0.0
        };
        let cycles = |s: &Sample, tid: u64| {
            s.cycles
                .iter()
                .find(|(t, _)| *t == tid)
                .map_or(0, |(_, c)| *c)
        };
        let mut rows: Vec<(u64, u64, u64, f64)> = later
            .switches
            .iter()
            .map(|&(tid, n, cpu)| {
                let (n0, cpu0) = self
                    .switches
                    .iter()
                    .find(|(t, _, _)| *t == tid)
                    .map_or((0, 0), |(_, n, c)| (*n, *c));
                let ran = cycles(later, tid).saturating_sub(cycles(self, tid)) as f64;
                let cycle_ms = if tsc_per_ms > 0.0 {
                    ran / tsc_per_ms
                } else {
                    0.0
                };
                (
                    tid,
                    cpu.saturating_sub(cpu0),
                    n.saturating_sub(n0),
                    cycle_ms,
                )
            })
            .collect();
        rows.sort_by(|a, b| b.3.total_cmp(&a.3).then(b.1.cmp(&a.1)));
        rows.into_iter()
            .take(5)
            .filter(|(_, cpu, n, cycle_ms)| *cpu > 0 || *n > 0 || *cycle_ms > 0.0)
            .map(|(tid, cpu, n, cycle_ms)| {
                let name = thread_name(tid);
                json!({
                    "thread": if name.is_empty() { tid.to_string() } else { name },
                    "cpu_ms": round(cpu as f64 / 1e4),
                    "cycle_ms": round(cycle_ms),
                    "wakeups_per_s": round(n as f64 / secs),
                })
            })
            .collect()
    }
}

pub(super) fn sample() -> Sample {
    let Some(s) = featherplay_platform::perf::process_sample() else {
        return Sample::default();
    };
    let switches: Vec<(u64, u64, u64)> = s
        .threads
        .iter()
        .map(|t| (t.id, t.switches, t.cpu))
        .collect();
    let cycles = switches
        .iter()
        .map(|&(tid, _, _)| (tid, thread_cycles(tid)))
        .collect();
    Sample {
        cpu_ms: s.cpu as f64 / 1e4,
        private_mb: s.private_bytes as f64 / 1e6,
        working_set_mb: s.working_set_bytes as f64 / 1e6,
        threads: s.threads.len() as u32,
        switches,
        cycles,
        proc_cycles: process_cycles(),
        tsc: tsc(),
    }
}

pub(super) fn logical_cpus() -> f64 {
    std::thread::available_parallelism().map_or(1.0, |n| n.get() as f64)
}

/// The Rust heap's live bytes: every allocation through the global allocator, counted.
/// What the C and C++ libraries (skia, the system's DLLs) allocate isn't in it.
struct Counting;

static RUST_LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call goes to `System` as it came; only a counter is kept beside it.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's layout, passed on.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            RUST_LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as `alloc`.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            RUST_LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        // SAFETY: a block this allocator handed out, with its layout.
        unsafe { System.dealloc(p, layout) };
        RUST_LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: as `dealloc`, with the new size the caller asked for.
        let q = unsafe { System.realloc(p, layout, size) };
        if !q.is_null() {
            RUST_LIVE.fetch_add(size, Ordering::Relaxed);
            RUST_LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        q
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Into `m`, each name after `prefix`: Rust's live bytes, the heaps' parts, the committed
/// private regions and the decoded covers in the in-memory cache.
pub(super) fn heap_parts(m: &mut Map<String, Value>, prefix: &str, covers: &crate::covers::Covers) {
    let h = featherplay_platform::perf::heap_walk();
    let (private, private_big) = featherplay_platform::perf::private_regions();
    let rust = RUST_LIVE.load(Ordering::Relaxed) as u64;
    for (name, bytes) in [
        ("rust", rust),
        ("heap_busy", h.busy),
        ("heap_committed", h.committed),
        ("heap_large", h.large),
        ("regions_private", private),
        ("regions_private_1mb", private_big),
        ("covers_cache", covers.cached_bytes() as u64),
    ] {
        m.insert(
            format!("{prefix}{name}_mb"),
            json!(round(bytes as f64 / 1e6)),
        );
    }
}
