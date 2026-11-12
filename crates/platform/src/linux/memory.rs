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

/// The heaps' allocated and committed bytes. Nothing counted on Linux.
#[cfg(feature = "perf")]
pub fn heap_summary() -> (usize, usize) {
    (0, 0)
}

/// The committed private bytes of each allocation. None listed on Linux.
#[cfg(feature = "perf")]
pub fn private_allocations() -> Vec<usize> {
    Vec::new()
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
