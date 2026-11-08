// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use std::cell::Cell;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use i_slint_core::timers::{Timer, TimerMode};

#[cfg(target_vendor = "apple")]
mod apple_display_link;

use crate::winitwindowadapter::WinitWindowAdapter;

pub fn create_frame_throttle(
    window_adapter: Weak<WinitWindowAdapter>,
    _winit_window: &winit::window::Window,
    _is_wayland: bool,
) -> Box<dyn FrameThrottle> {
    if _is_wayland {
        WinitBasedFrameThrottle::create()
    } else {
        #[cfg(target_vendor = "apple")]
        if let Some(throttle) =
            apple_display_link::try_create(window_adapter.clone(), _winit_window)
        {
            return throttle;
        }
        TimerBasedFrameThrottle::create(window_adapter)
    }
}

pub trait FrameThrottle {
    fn request_throttled_redraw(&self, winit_window: &winit::window::Window);
}

struct TimerBasedFrameThrottle {
    window_adapter: Weak<WinitWindowAdapter>,
    timer: Rc<Timer>,
    // featherplay patch: the frame interval, the monitor it was read for, and when.
    interval: std::cell::RefCell<Option<(winit::monitor::MonitorHandle, Duration, Instant)>>,
    // featherplay patch: when winit was last asked for a frame.
    last: Rc<Cell<Option<Instant>>>,
}

/// featherplay patch: how long a monitor's refresh rate is reused before it's read again.
const INTERVAL_KEPT: Duration = Duration::from_secs(1);

impl TimerBasedFrameThrottle {
    fn create(window_adapter: Weak<WinitWindowAdapter>) -> Box<dyn FrameThrottle> {
        Box::new(Self {
            window_adapter,
            timer: Rc::new(Timer::default()),
            interval: Default::default(),
            last: Default::default(),
        })
    }

    // featherplay patch: the refresh rate is read at most once a second per monitor. Upstream
    // read it for every redraw: on Windows a GetMonitorInfoW and an EnumDisplaySettingsExW
    // each time, a tenth of the UI thread's time while the playing mark steps (VENDORED.md).
    fn interval(&self, winit_window: &winit::window::Window) -> Duration {
        let monitor = winit_window.current_monitor();
        let now = Instant::now();
        if let (Some(m), Some((kept, interval, at))) = (&monitor, &*self.interval.borrow())
            && m == kept
            && now.duration_since(*at) < INTERVAL_KEPT
        {
            return *interval;
        }
        let refresh_interval_millihertz = monitor
            .as_ref()
            .and_then(|monitor| monitor.refresh_rate_millihertz())
            .unwrap_or(60000) as u64;
        let interval = Duration::from_millis((1000 * 1000) / refresh_interval_millihertz);
        *self.interval.borrow_mut() = monitor.map(|m| (m, interval, now));
        interval
    }
}

// featherplay patch: a redraw asked for a frame or more after the last is asked of winit at
// once; one asked for sooner waits, on a single shot, until a frame after the last, and is
// asked for then only if it's still pending. Frames stay at most one a refresh, as upstream.
// Upstream always waited a tick of a repeating timer, asked winit on every tick while a redraw
// was pending and once more on the tick after it was drawn, then stopped on the next: every
// redraw cost the event loop four turns and the window two WM_PAINTs, the second with
// nothing to draw (VENDORED.md).
impl FrameThrottle for TimerBasedFrameThrottle {
    fn request_throttled_redraw(&self, winit_window: &winit::window::Window) {
        if self.timer.running() {
            return;
        }
        let interval = self.interval(winit_window);
        let now = Instant::now();
        let due = self.last.get().map_or(now, |last| last + interval);
        if due <= now {
            self.last.set(Some(now));
            winit_window.request_redraw();
            return;
        }
        // Slint's timers count whole milliseconds: rounded up, so frames never come sooner.
        let wait = Duration::from_millis((due - now).as_micros().div_ceil(1000) as u64);
        let window_adapter = self.window_adapter.clone();
        let last = self.last.clone();
        self.timer.start(TimerMode::SingleShot, wait, move || {
            if window_adapter.upgrade().is_some_and(|adapter| adapter.pending_redraw()) {
                last.set(Some(Instant::now()));
                redraw_now(&window_adapter);
            }
        });
    }
}

fn redraw_now(window_adapter: &Weak<WinitWindowAdapter>) {
    let Some(winit_window) = window_adapter.upgrade().and_then(|adapter| adapter.winit_window())
    else {
        return;
    };
    winit_window.request_redraw();
}

struct WinitBasedFrameThrottle;

impl WinitBasedFrameThrottle {
    fn create() -> Box<dyn FrameThrottle> {
        Box::new(Self)
    }
}

impl FrameThrottle for WinitBasedFrameThrottle {
    fn request_throttled_redraw(&self, winit_window: &winit::window::Window) {
        winit_window.request_redraw();
    }
}
