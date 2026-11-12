//! Private bytes and heap compaction.

/// This process's private bytes: its resident anonymous memory, swapped-out pages included
/// (`RssAnon` + `VmSwap` in `/proc/self/status`), the nearest Linux has to Windows' private
/// bytes. 0 if the kernel didn't say, as on Windows.
pub fn private_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .map(|status| private_from_status(&status))
        .unwrap_or(0)
}

fn private_from_status(status: &str) -> u64 {
    let kb = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| {
                rest.trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
            .unwrap_or(0)
    };
    (kb("RssAnon:") + kb("VmSwap:")) * 1024
}

/// The size from which glibc gives a block a mapping of its own: its starting value, kept.
const OWN_MAPPING_FROM: libc::c_int = 128 * 1024;

/// Keeps glibc from holding on to the big blocks the app lets go of. Call it once, first
/// thing, before other threads start.
///
/// glibc maps a block of 128 KiB or more on its own, and unmaps it when it's freed, but each
/// time it frees one it raises that size to the block's (up to 32 MiB). Soon the decoded
/// covers and the renderer's buffers come from its arenas instead, which keep their pages
/// once they're freed: at 4K, 480 MB private with 125 MB in use (the memory test, measured
/// 2026-11-12). Setting the size keeps it where it starts, so those blocks go back to the
/// system as they're freed, as Windows' heap does with its big blocks: 120 MB.
pub fn tune_allocator() {
    // SAFETY: glibc's own call with one of its parameters; safe at any time.
    unsafe { libc::mallopt(libc::M_MMAP_THRESHOLD, OWN_MAPPING_FROM) };
}

/// Gives the allocator's free pages back to the system (`malloc_trim`), as the app does
/// when its window is hidden.
pub fn compact_heaps() {
    // SAFETY: glibc's own call, safe at any time; 0 keeps no padding.
    unsafe { libc::malloc_trim(0) };
}

/// The process's commit charge now and at its peak. `None`: Linux has no such count.
#[cfg(feature = "perf")]
pub fn commit_charge() -> Option<(usize, usize)> {
    None
}

/// The allocator's allocated bytes and the bytes it holds from the system (glibc's
/// `mallinfo2`: in use, and its arenas' size, each with the blocks it maps on their own).
#[cfg(feature = "perf")]
pub fn heap_summary() -> (usize, usize) {
    // SAFETY: glibc's own call, no arguments; it only reads the allocator's counts.
    let m = unsafe { libc::mallinfo2() };
    (m.uordblks + m.hblkhd, m.arena + m.hblkhd)
}

/// The resident anonymous bytes of each mapping (`Anonymous` in `/proc/self/smaps`), in
/// address order; the mappings with none are left out.
#[cfg(feature = "perf")]
pub fn private_allocations() -> Vec<usize> {
    std::fs::read_to_string("/proc/self/smaps")
        .map(|smaps| anonymous_from_smaps(&smaps))
        .unwrap_or_default()
}

#[cfg(feature = "perf")]
fn anonymous_from_smaps(smaps: &str) -> Vec<usize> {
    smaps
        .lines()
        .filter_map(|line| line.strip_prefix("Anonymous:"))
        .filter_map(|rest| {
            rest.trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<usize>()
                .ok()
        })
        .filter(|&kb| kb > 0)
        .map(|kb| kb * 1024)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_bytes_are_read_from_the_status_file() {
        let status = "Name:\tx\nRssAnon:\t   2048 kB\nRssFile:\t 100 kB\nVmSwap:\t      16 kB\n";
        assert_eq!(private_from_status(status), (2048 + 16) * 1024);
        assert!(private_bytes() > 0);
        compact_heaps();
    }
}
