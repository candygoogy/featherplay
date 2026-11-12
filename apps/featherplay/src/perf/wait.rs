//! Waiting: the futures the script's steps await, woken by what the app reports.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use super::{SEEN, TICK};

/// Ready with `true` once `cond` holds (checked whenever the app reports something), or with
/// `false` at `deadline`.
pub(super) struct Until<F> {
    cond: F,
    deadline: Instant,
    timer: Option<slint::Timer>,
    /// Checked again at least this often, for conditions that time alone makes true.
    tick: Option<Duration>,
    /// Checked again whenever the app reports something. Off for a plain wait (`sleep`),
    /// which time alone ends: woken by every paint, the script would cost an idle spell's
    /// frames a posted message and a turn of the event loop each.
    on_reports: bool,
}

impl<F: FnMut() -> bool + Unpin> Future for Until<F> {
    type Output = bool;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<bool> {
        if (self.cond)() {
            return Poll::Ready(true);
        }
        let now = Instant::now();
        if now >= self.deadline {
            return Poll::Ready(false);
        }
        // Once: each wake polls the script again, and each poll lands here. Queued on every
        // poll, a copy was never dropped and every timer wake added one, so over a run every
        // paint came to post hundreds of wakes to the UI thread (half its time in the tour).
        SEEN.with_borrow_mut(|s| {
            if self.on_reports && !s.wakers.iter().any(|w| w.will_wake(cx.waker())) {
                s.wakers.push(cx.waker().clone());
            }
        });
        // Slint's timers count whole milliseconds and may fire a little early: armed again
        // for what's left whenever it isn't running.
        if !self.timer.as_ref().is_some_and(slint::Timer::running) {
            let waker = cx.waker().clone();
            let left = self.deadline - now + Duration::from_millis(1);
            let left = self.tick.map_or(left, |t| t.min(left));
            let timer = self.timer.get_or_insert_with(slint::Timer::default);
            timer.start(slint::TimerMode::SingleShot, left, move || {
                waker.wake_by_ref()
            });
        }
        Poll::Pending
    }
}

pub(super) fn until(
    timeout: Duration,
    cond: impl FnMut() -> bool + Unpin,
) -> Until<impl FnMut() -> bool + Unpin> {
    Until {
        cond,
        deadline: Instant::now() + timeout,
        timer: None,
        tick: None,
        on_reports: true,
    }
}

/// `until`, for a condition that may come true with nothing reported (a quiet spell): checked
/// every `TICK` too. Never used for `idle`, whose wakeups are measured.
pub(super) fn ticking(
    timeout: Duration,
    cond: impl FnMut() -> bool + Unpin,
) -> Until<impl FnMut() -> bool + Unpin> {
    Until {
        tick: Some(TICK),
        ..until(timeout, cond)
    }
}

pub(super) async fn sleep(d: Duration) {
    Until {
        on_reports: false,
        ..until(d, || false)
    }
    .await;
}

/// Lets the event loop run once (input dispatched, a frame asked for).
pub(super) async fn turn() {
    sleep(Duration::from_millis(1)).await;
}

#[cfg(test)]
mod tests {
    use std::task::Waker;

    use super::*;

    #[test]
    fn a_wait_keeps_one_waker_however_often_it_is_polled() {
        struct Task;
        #[expect(
            clippy::manual_noop_waker,
            reason = "Waker::noop() doesn't say its copies wake the same task"
        )]
        impl std::task::Wake for Task {
            fn wake(self: std::sync::Arc<Self>) {}
        }
        // One task's waker, as the script's.
        let waker = Waker::from(std::sync::Arc::new(Task));
        let mut cx = Context::from_waker(&waker);
        let mut wait = until(Duration::from_secs(60), || false);
        for _ in 0..5 {
            assert!(Pin::new(&mut wait).poll(&mut cx).is_pending());
        }
        assert_eq!(SEEN.with_borrow(|s| s.wakers.len()), 1);
    }
}
