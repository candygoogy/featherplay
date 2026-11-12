//! Thread priority and thread CPU time. Priority is best effort: Linux lets an ordinary user
//! lower a thread's priority, but raising one, or the real-time class, only goes as far as
//! the limits allow (`RLIMIT_RTPRIO` and `RLIMIT_NICE`, which many desktops grant the
//! audio group). Where they don't, the thread stays as it was.

use std::time::Duration;

/// The output thread in the real-time class (`SCHED_FIFO`). It isn't undone: the thread is
/// cpal's for its stream, and ends with it.
pub struct ProAudio {}

/// The real-time priority asked for: low in the range, as audio servers' clients take it
/// (PipeWire gives its own threads 88 and clients less).
const AUDIO_RT_PRIORITY: libc::c_int = 10;

/// Puts the calling thread in the real-time class for audio, at a priority the process's
/// `RLIMIT_RTPRIO` allows. `None` where none is allowed, and the thread stays as it was.
pub fn pro_audio() -> Option<ProAudio> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: a valid out pointer to a local.
    if unsafe { libc::getrlimit(libc::RLIMIT_RTPRIO, &mut limit) } != 0 || limit.rlim_cur == 0 {
        return None;
    }
    let priority = AUDIO_RT_PRIORITY.min(libc::c_int::try_from(limit.rlim_cur).unwrap_or(1));
    let param = libc::sched_param {
        sched_priority: priority,
    };
    // SAFETY: this thread's own handle and a valid parameter.
    let set =
        unsafe { libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, &param) };
    (set == 0).then_some(ProAudio {})
}

/// The nice value for this thread (Linux keeps one per thread).
fn set_nice(nice: libc::c_int) {
    // SAFETY: no arguments.
    let tid = unsafe { libc::gettid() };
    // `PRIO_PROCESS` with a thread id names that thread. A refusal leaves it as it was.
    // SAFETY: plain values; the thread is this one.
    let _ = unsafe { libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, nice) };
}

/// Raises the calling thread above ordinary threads (nice -5), where `RLIMIT_NICE` allows.
/// For the decoders.
pub fn raise_priority() {
    set_nice(-5);
}

/// Lowers the calling thread below ordinary threads (nice 10): background work (skip
/// silence's scan) that every ordinary thread, the UI's included, goes ahead of.
pub fn lower_priority() {
    set_nice(10);
}

/// This thread's user and system CPU time so far (`getrusage(RUSAGE_THREAD)`). `None` if
/// the kernel didn't say. For the frame log (`FEATHERPLAY_FRAME_LOG`).
pub fn user_and_kernel_time() -> Option<(Duration, Duration)> {
    // SAFETY: an all-zero `rusage` is a valid value of the plain C struct.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer to a local.
    if unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut usage) } != 0 {
        return None;
    }
    let time = |t: libc::timeval| {
        Duration::from_secs(t.tv_sec.max(0) as u64) + Duration::from_micros(t.tv_usec.max(0) as u64)
    };
    Some((time(usage.ru_utime), time(usage.ru_stime)))
}

/// This thread's CPU time so far, from the scheduler's count (`/proc/thread-self/schedstat`,
/// in nanoseconds). `None` where the kernel doesn't keep it.
#[cfg(feature = "perf")]
pub fn cpu_time() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    let nanos = stat.split_whitespace().next()?.parse().ok()?;
    Some(Duration::from_nanos(nanos))
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

/// Would raise this whole process's priority. Leaves it as it is.
#[cfg(feature = "perf")]
pub fn raise_process_priority() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_threads_cpu_time_grows_and_a_lowered_thread_says_so() {
        let worker = std::thread::spawn(|| {
            lower_priority();
            // SAFETY: plain values; reads this thread's nice value.
            let nice =
                unsafe { libc::getpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t) };
            let before = user_and_kernel_time().expect("a time");
            let t = std::time::Instant::now();
            let mut x = 0u64;
            while t.elapsed() < Duration::from_millis(30) {
                x = std::hint::black_box(x.wrapping_add(1));
            }
            let after = user_and_kernel_time().expect("a time");
            (nice, before, after)
        });
        let (nice, before, after) = worker.join().unwrap();
        assert_eq!(nice, 10);
        assert!(
            after.0 + after.1 > before.0 + before.1,
            "{before:?} -> {after:?}"
        );
        // Allowed or not, asking doesn't fail loudly; on a thread of its own, which ends.
        std::thread::spawn(|| {
            let _ = pro_audio();
            raise_priority();
        })
        .join()
        .unwrap();
    }
}
