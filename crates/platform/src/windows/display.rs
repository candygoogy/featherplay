//! The display's refresh, for an animation the app paces itself (the wheel's glide): DWM's
//! composition clock. GDI's copy to the window has no vsync of its own, so frames drawn as
//! fast as they come land on the refreshes unevenly (two in one, none in the next).

use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DWM_TIMING_INFO, DwmFlush, DwmGetCompositionTimingInfo};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

type Job = Box<dyn FnOnce() + Send>;

/// Runs `then`, on a thread of its own, once DWM has composed its next frame (DwmFlush): the
/// start of a refresh, so a frame started then has the whole refresh to be drawn in time.
pub fn after_composition(then: impl FnOnce() + Send + 'static) {
    static CLOCK: OnceLock<Sender<Job>> = OnceLock::new();
    let clock = CLOCK.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        // If the thread can't start, `rx` goes with it and every job runs at once below.
        let _ = std::thread::Builder::new()
            .name("featherplay-frame-clock".into())
            .spawn(move || {
                for job in rx {
                    // SAFETY: no arguments. It fails when DWM isn't composing (a locked
                    // screen): the job runs at once then.
                    let _ = unsafe { DwmFlush() };
                    job();
                }
            });
        tx
    });
    if let Err(job) = clock.send(Box::new(then)) {
        (job.0)();
    }
}

/// DWM's composition clock, in performance-counter counts: the last vertical blank, the
/// refresh period and the counter now, with the counter's counts a second. `None` when DWM
/// or the counter doesn't say.
pub(crate) fn clock() -> Option<(i64, i64, i64, i64)> {
    let mut info = DWM_TIMING_INFO {
        cbSize: size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    let (mut now, mut freq) = (0i64, 0i64);
    // SAFETY: valid out pointers; a null window asks for the desktop's composition timing.
    unsafe {
        DwmGetCompositionTimingInfo(HWND::default(), &mut info).ok()?;
        QueryPerformanceCounter(&mut now).ok()?;
        QueryPerformanceFrequency(&mut freq).ok()?;
    }
    Some((
        info.qpcVBlank as i64,
        info.qpcRefreshPeriod as i64,
        now,
        freq,
    ))
}

/// The first refresh of the display that comes at least `ahead` from now, by DWM's clock
/// (the last vertical blank and the refresh period), or `None` when DWM doesn't say.
pub fn next_refresh(ahead: Duration) -> Option<Instant> {
    let (vblank, period, now, freq) = clock()?;
    if period <= 0 || freq <= 0 || vblank <= 0 {
        return None;
    }
    let soonest = now + (ahead.as_secs_f64() * freq as f64) as i64;
    let refreshes = ((soonest - vblank).max(0) as u64)
        .div_ceil(period as u64)
        .max(1) as i64;
    let next = vblank + refreshes * period;
    Some(Instant::now() + Duration::from_secs_f64((next - now) as f64 / freq as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The job runs (after a composition, or at once where DWM isn't composing), and the next
    /// refresh, where DWM tells it, is ahead and within a refresh or two (a display of 24 Hz
    /// or faster).
    #[test]
    fn a_frame_waits_for_a_composition_and_aims_at_the_next_refresh() {
        let (tx, rx) = channel();
        after_composition(move || {
            let _ = tx.send(());
        });
        assert!(
            rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "the job ran"
        );
        let ahead = Duration::from_millis(2);
        let asked = Instant::now();
        if let Some(next) = next_refresh(ahead) {
            assert!(next >= asked + ahead, "{:?}", next - asked);
            assert!(
                next <= asked + ahead + Duration::from_millis(84),
                "{:?}",
                next - asked
            );
        }
    }
}
