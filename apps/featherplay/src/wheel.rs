//! Wheel and touchpad scrolling, for every list, grid and page at once: a filter on the
//! window's winit events, so no page or panel has its own.
//!
//! The filter eases the wheel itself (`Glide`), as browsers do: every delta joins one
//! glide toward where the wheel has asked to go, and each frame, before Slint draws it
//! (`RedrawRequested`), the filter hands that frame's step to Slint as a scroll without a
//! phase, which moves a `Flickable` (and so every `ListView`, and a shelf's rail) at once;
//! then it asks for the next frame, until the glide has arrived. A notch from rest takes
//! 180 ms, slowing evenly to a stop (`glide_at`: d·(1 − (1 − t)²), Slint 1.18's wheel
//! curve). A delta that comes while the glide runs adds to what's left of it and carries on
//! at the speed the list has then, from where it is then, so a fast spin keeps a steady
//! speed with no kick at each notch and no frame that stands still. (Slint 1.19's own
//! curve, `AndroidFlick`, d·(1 − (1 − t)^2.36), starts each notch harder and crawls longer.)
//! Fractions of a notch, from a precision touchpad or a high-resolution wheel, glide the
//! same way over 60 ms (`FINE_TIME`): they come at their own pace, 125 to 500 a second,
//! and handed over as they came they moved the list one, two or no steps a frame; the list
//! trails a steady touchpad drag by about 40 ms. A `scroll-event` handler gets a notch in
//! steps too, adding up to the notch: the volume lands on the value one notch gave it; a
//! menu closes on the first.
//!
//! With Windows' animation effects turned off (`Theme.motion`) nothing eases: every delta
//! jumps at once.
//!
//! While the wheel scrolls under a still pointer, `Frame.scrolling` lays a bare touch area
//! over the window that holds the hover (`app.slint`), so rows passing under the pointer
//! don't light up and fade one after another, as browsers do. The first real move of the
//! pointer, or a button, lifts it before Slint sees that event. Nothing is timed, but for a
//! shelf's rail: once it comes to rest under the still pointer, `rested` lifts the layer and
//! gives the hover to what's under the pointer now.
//!
//! Shift and the vertical wheel move a shelf's rail, and Slint knows Shift only from key
//! events. On Wayland winit sends none for a Shift already held when the window takes the
//! keyboard focus (X11 and Windows send a synthetic press), only the modifiers, so the
//! filter hands Slint the press or release it missed (`ShiftSync`).
//!
//! The same filter, the only one a window can have, sends the mouse's side buttons to
//! `Nav.back` and `Nav.forward` (`mouse_nav`), and tells `window_place` when the window moves
//! or changes size.

use std::cell::Cell;
use std::time::{Duration, Instant};

use slint::platform::{Key, WindowEvent as SlintEvent};
use slint::winit_030::winit::event::{MouseScrollDelta, WindowEvent};
use slint::winit_030::winit::keyboard::{Key as WinitKey, NamedKey};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, LogicalPosition};
use std::rc::Rc;

use crate::mouse_nav::{self, Press, Step};
use crate::ui::{AppWindow, Frame, Immersive, Nav, Theme};
use featherplay_platform::display;

/// Logical pixels per wheel line, as Slint's winit backend scales a line delta.
const LINE: f32 = 60.0;

/// The time a notch takes to cover (Slint's `WHEEL_SCROLL_DURATION`, the same in 1.18 and
/// 1.19).
const NOTCH_TIME: Duration = Duration::from_millis(180);

/// The time a fraction of a notch takes: long enough to even out wheel messages that come
/// at their own pace (a high-resolution wheel's 125 to 500 a second against the frames'),
/// short enough that a touchpad's list stays close behind the fingers (~40 ms on a steady
/// drag).
const FINE_TIME: Duration = Duration::from_millis(60);

/// About as long as a frame takes to draw at 4K: a frame that starts closer to the next
/// refresh than this shows at the one after.
const DRAW_TIME: Duration = Duration::from_millis(2);

/// How long after a fraction of a notch whole notches are taken as part of the same fine
/// stream: a high-resolution wheel sends whole notches too, when its steps add up to one.
const FINE_HOLD: Duration = Duration::from_millis(500);

/// Where a glide of `distance` that started at `speed` (pixels a second) is, and how fast it
/// goes, `elapsed` into its `time`: a cubic that starts at that speed and ends at rest on the
/// distance. Started at 2·distance/time, it is Slint 1.18's wheel curve,
/// d·(1 − (1 − t)²), a constant deceleration to a stop.
pub fn glide_at(distance: f32, speed: f32, time: Duration, elapsed: Duration) -> (f32, f32) {
    let time = time.as_secs_f32();
    let s = (elapsed.as_secs_f32() / time).clamp(0.0, 1.0);
    if s >= 1.0 {
        return (distance, 0.0);
    }
    let lead = speed * time;
    let at = lead * (s * s * s - 2.0 * s * s + s) + distance * (3.0 * s * s - 2.0 * s * s * s);
    let speed = (lead * (3.0 * s * s - 4.0 * s + 1.0) + distance * (6.0 * s - 6.0 * s * s)) / time;
    (at, speed)
}

/// The speed a glide of `distance` over `time` starts at, carrying on from `speed`: the
/// same speed, so that a notch arriving mid-glide doesn't kick the list on (each notch's
/// kick made a fast spin pulse); from rest, or turning back, 1.18's 2·distance/time. Never
/// over 3·distance/time, past which the curve would overshoot and come back.
fn start_speed(distance: f32, speed: Option<f32>, time: Duration) -> f32 {
    let time = time.as_secs_f32();
    match speed {
        Some(v) if v * distance > 0.0 => {
            v.abs().min(3.0 * distance.abs() / time) * distance.signum()
        }
        _ => 2.0 * distance / time,
    }
}

/// Wheel motion being covered at one pointer position (`glide_at`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glide {
    pub at: LogicalPosition,
    started: Instant,
    time: Duration,
    /// The distance from where the curve started, its start speed, and what's been handed
    /// to Slint of it, in logical pixels (each for x and y). What's handed starts below
    /// zero when the glide before owed some of its way at the time this one took over.
    total: (f32, f32),
    speed: (f32, f32),
    done: (f32, f32),
}

impl Glide {
    /// A delta of `dx`, `dy` at `at`, `now`, covered in `time`, on what's left of `before`
    /// there: the glide that covers both from now, carrying on at the speed `before` had
    /// then. What's left of a glide somewhere else is returned, to be handed over at once.
    pub fn notch(
        before: Option<Glide>,
        at: LogicalPosition,
        (dx, dy): (f32, f32),
        time: Duration,
        now: Instant,
    ) -> (Glide, Option<(LogicalPosition, f32, f32)>) {
        let mut glide = Glide {
            at,
            started: now,
            time,
            total: (dx, dy),
            speed: (0.0, 0.0),
            done: (0.0, 0.0),
        };
        let mut speed = (None, None);
        let mut rest = None;
        if let Some(g) = before {
            if g.at == at {
                // From where the glide is now, not where the last frame left it, so the time
                // since that frame isn't lost (the frame after each notch stood still).
                let elapsed = now.saturating_duration_since(g.started);
                let (x, vx) = glide_at(g.total.0, g.speed.0, g.time, elapsed);
                let (y, vy) = glide_at(g.total.1, g.speed.1, g.time, elapsed);
                glide.total = (dx + g.total.0 - x, dy + g.total.1 - y);
                glide.done = (g.done.0 - x, g.done.1 - y);
                speed = (Some(vx), Some(vy));
            } else {
                let left = (g.total.0 - g.done.0, g.total.1 - g.done.1);
                if left != (0.0, 0.0) {
                    rest = Some((g.at, left.0, left.1));
                }
            }
        }
        glide.speed = (
            start_speed(glide.total.0, speed.0, time),
            start_speed(glide.total.1, speed.1, time),
        );
        (glide, rest)
    }

    /// The step to hand over at `now`, and whether the glide has covered its distance. The
    /// last step ends on it exactly.
    pub fn step(&mut self, now: Instant) -> (f32, f32, bool) {
        let elapsed = now.saturating_duration_since(self.started);
        let over = elapsed >= self.time;
        let to = if over {
            self.total
        } else {
            (
                glide_at(self.total.0, self.speed.0, self.time, elapsed).0,
                glide_at(self.total.1, self.speed.1, self.time, elapsed).0,
            )
        };
        let step = (to.0 - self.done.0, to.1 - self.done.1);
        self.done = to;
        (step.0, step.1, over)
    }
}

/// What to do with one wheel event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Route {
    /// Whole notches, eased by the filter over `NOTCH_TIME` (`Glide`); pixel deltas, left
    /// to Slint.
    Eased,
    /// A fraction of a notch, eased by the filter over `FINE_TIME`.
    Fine,
    /// Scroll by this many logical pixels at once.
    Direct { dx: f32, dy: f32 },
}

/// How a wheel event with `delta` scrolls. `motion`: Windows' animation effects are on.
pub fn route(delta: MouseScrollDelta, motion: bool) -> Route {
    match delta {
        MouseScrollDelta::LineDelta(x, y) if !motion => Route::Direct {
            dx: x * LINE,
            dy: y * LINE,
        },
        MouseScrollDelta::LineDelta(x, y) if !whole(x) || !whole(y) => Route::Fine,
        // Whole notches, and pixel deltas (Windows has none), which Slint takes as they come.
        MouseScrollDelta::LineDelta(..) | MouseScrollDelta::PixelDelta(_) => Route::Eased,
    }
}

/// A whole number of notches: `WM_MOUSEWHEEL` in multiples of `WHEEL_DELTA` (120).
fn whole(lines: f32) -> bool {
    (lines - lines.round()).abs() < 1e-3
}

/// Shift as Slint has it, kept in step with the modifiers winit reports.
#[derive(Debug, Default)]
pub struct ShiftSync {
    down: bool,
}

impl ShiftSync {
    /// A Shift key event, which Slint takes as it comes.
    pub fn key(&mut self, pressed: bool) {
        self.down = pressed;
    }

    /// The window lost the focus: Slint drops its modifiers.
    pub fn focus_lost(&mut self) {
        self.down = false;
    }

    /// The modifiers now have Shift `shift`: the press (`true`) or release Slint missed.
    pub fn modifiers(&mut self, shift: bool) -> Option<bool> {
        (shift != self.down).then(|| {
            self.down = shift;
            shift
        })
    }
}

/// Whether the window is a Wayland surface (Slint picks X11 under WSL, whatever the session).
#[cfg(target_os = "linux")]
fn on_wayland(window: &slint::Window) -> bool {
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .with_winit_window(|w| {
            matches!(
                w.window_handle().map(|h| h.as_raw()),
                Ok(RawWindowHandle::Wayland(_))
            )
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn on_wayland(_: &slint::Window) -> bool {
    false
}

/// Sets `Frame.scrolling`, if it isn't already.
fn set_scrolling(ui: &AppWindow, on: bool) {
    let frame = ui.global::<Frame>();
    if frame.get_scrolling() != on {
        frame.set_scrolling(on);
    }
}

/// A rail has come to rest with the pointer at `at` (`Frame.rested`): the hover-holding layer
/// goes and the hover goes to what's under the pointer now, as a move there would give it.
/// Slint works hover out only when the pointer moves (and once after a wheel event it takes,
/// before an eased rail has moved). Any window: a test's offscreen one too.
pub fn rested(ui: &AppWindow, at: LogicalPosition) {
    set_scrolling(ui, false);
    ui.window()
        .dispatch_event(SlintEvent::PointerMoved { position: at });
}

/// Installs the filter on the app's window (a winit window; anything else is left alone).
pub fn install(ui: &AppWindow) {
    let cursor = ui.as_weak();
    ui.global::<Immersive>().on_pointer_visible(move |visible| {
        if let Some(ui) = cursor.upgrade() {
            let _ = ui
                .window()
                .with_winit_window(|window| window.set_cursor_visible(visible));
        }
    });
    let weak = ui.as_weak();
    // Where the pointer is, in logical pixels: winit's wheel events don't say.
    let pointer: Cell<Option<LogicalPosition>> = Cell::new(None);
    let mut shift = ShiftSync::default();
    let mut wayland: Option<bool> = None;
    let glide: Rc<Cell<Option<Glide>>> = Rc::default();
    // Until when whole notches are taken as a high-resolution wheel's (`FINE_HOLD`).
    let fine_until: Cell<Option<Instant>> = Cell::new(None);
    // The next frame, asked for after this one's step, once the display has taken this one
    // (`display::after_composition`): each frame then starts as a refresh begins and has the
    // refresh to be drawn in. (A request made while Slint handles `RedrawRequested` would be
    // cleared by its draw; this one comes after.)
    let next_frame = {
        let weak = weak.clone();
        move || {
            let weak = weak.clone();
            display::after_composition(move || {
                let _ = weak.upgrade_in_event_loop(|ui| ui.window().request_redraw());
            });
        }
    };
    ui.window()
        .on_winit_window_event(move |window, event| match event {
            WindowEvent::CursorMoved { position, .. } => {
                let scale = f64::from(window.scale_factor());
                let at =
                    LogicalPosition::new((position.x / scale) as f32, (position.y / scale) as f32);
                // A move to where the pointer already was isn't one.
                if pointer.replace(Some(at)) != Some(at)
                    && let Some(ui) = weak.upgrade()
                {
                    set_scrolling(&ui, false);
                    // Full-screen lyrics reveal their controls on real movement, including
                    // movement over child touch areas that handle the event themselves.
                    ui.global::<Immersive>().invoke_reveal();
                }
                EventResult::Propagate
            }
            WindowEvent::CursorLeft { .. } | WindowEvent::MouseInput { .. } => {
                if matches!(event, WindowEvent::CursorLeft { .. }) {
                    pointer.set(None);
                } else {
                    // A press ends a glide where it is, as it stops Slint's own.
                    glide.set(None);
                }
                let Some(ui) = weak.upgrade() else {
                    return EventResult::Propagate;
                };
                set_scrolling(&ui, false);
                // The side buttons navigate, and the pages never see them.
                if let WindowEvent::MouseInput { button, state, .. } = event
                    && let Some(press) = mouse_nav::press(*button, *state)
                {
                    if let Press::Go(step) = press {
                        let nav = ui.global::<Nav>();
                        match step {
                            Step::Back => nav.invoke_back(),
                            Step::Forward => nav.invoke_forward(),
                        }
                    }
                    return EventResult::PreventDefault;
                }
                EventResult::Propagate
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let Some(ui) = weak.upgrade() else {
                    return EventResult::Propagate;
                };
                set_scrolling(&ui, true);
                let motion = ui.global::<Theme>().get_motion();
                let at = pointer.get();
                // The performance harness posts wheel messages with the real pointer elsewhere.
                #[cfg(feature = "perf-harness")]
                let at = at.or_else(crate::perf::pointer);
                let now = Instant::now();
                let route = match route(*delta, motion) {
                    Route::Fine => {
                        fine_until.set(Some(now + FINE_HOLD));
                        Route::Fine
                    }
                    Route::Eased if fine_until.get().is_some_and(|t| now < t) => Route::Fine,
                    other => other,
                };
                match (route, delta, at) {
                    (Route::Direct { dx, dy }, _, Some(position)) => {
                        // No phase: the Flickable moves by the delta at once.
                        window.dispatch_event(SlintEvent::PointerScrolled {
                            position,
                            delta_x: dx,
                            delta_y: dy,
                        });
                        EventResult::PreventDefault
                    }
                    (
                        Route::Eased | Route::Fine,
                        MouseScrollDelta::LineDelta(x, y),
                        Some(position),
                    ) => {
                        let time = if route == Route::Fine {
                            FINE_TIME
                        } else {
                            NOTCH_TIME
                        };
                        let (next, rest) =
                            Glide::notch(glide.get(), position, (x * LINE, y * LINE), time, now);
                        if let Some((position, delta_x, delta_y)) = rest {
                            window.dispatch_event(SlintEvent::PointerScrolled {
                                position,
                                delta_x,
                                delta_y,
                            });
                        }
                        // A running glide has asked for its next frame already: one asked
                        // for here came at once, a frame early, that barely moved.
                        if glide.replace(Some(next)).is_none() {
                            window.request_redraw();
                        }
                        EventResult::PreventDefault
                    }
                    _ => EventResult::Propagate,
                }
            }
            // A glide's step for this frame, before Slint draws it.
            WindowEvent::RedrawRequested => {
                if let Some(mut g) = glide.get() {
                    // Where the glide is when this frame shows: the refresh after it's drawn,
                    // so the list moves by the time between refreshes, not between the
                    // moments frames happened to start.
                    let shown = display::next_refresh(DRAW_TIME).unwrap_or_else(Instant::now);
                    let (delta_x, delta_y, over) = g.step(shown);
                    glide.set((!over).then_some(g));
                    if delta_x != 0.0 || delta_y != 0.0 {
                        window.dispatch_event(SlintEvent::PointerScrolled {
                            position: g.at,
                            delta_x,
                            delta_y,
                        });
                    }
                    if !over {
                        next_frame();
                    }
                }
                EventResult::Propagate
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.logical_key == WinitKey::Named(NamedKey::Shift) =>
            {
                shift.key(event.state.is_pressed());
                EventResult::Propagate
            }
            WindowEvent::Focused(false) => {
                shift.focus_lost();
                EventResult::Propagate
            }
            WindowEvent::ModifiersChanged(m) => {
                if let Some(down) = shift.modifiers(m.state().shift_key())
                    && *wayland.get_or_insert_with(|| on_wayland(window))
                {
                    let text = Key::Shift.into();
                    window.dispatch_event(if down {
                        SlintEvent::KeyPressed { text }
                    } else {
                        SlintEvent::KeyReleased { text }
                    });
                }
                EventResult::Propagate
            }
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                crate::window_place::changed();
                EventResult::Propagate
            }
            _ => EventResult::Propagate,
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use MouseScrollDelta::{LineDelta, PixelDelta};
    use slint::winit_030::winit::dpi::PhysicalPosition;

    const fn direct(dy: f32) -> Route {
        Route::Direct { dx: 0.0, dy }
    }

    #[test]
    fn notches_ease_fractions_ease_quicker_and_effects_off_jump() {
        let pixels = PixelDelta(PhysicalPosition::new(0.0, 12.0));
        // (case, delta, animation effects on, want)
        let cases = [
            ("a notch", LineDelta(0.0, -1.0), true, Route::Eased),
            ("notches", LineDelta(0.0, 3.0), true, Route::Eased),
            ("a tilt notch", LineDelta(1.0, 0.0), true, Route::Eased),
            // 15 of WHEEL_DELTA's 120: an eighth of a notch.
            ("a touchpad step", LineDelta(0.0, 0.125), true, Route::Fine),
            ("past a notch", LineDelta(0.0, -1.5), true, Route::Fine),
            ("effects off", LineDelta(0.0, -1.0), false, direct(-60.0)),
            ("pixel deltas", pixels, true, Route::Eased),
        ];
        for (case, delta, motion, want) in cases {
            assert_eq!(route(delta, motion), want, "{case}");
        }
    }

    /// A notch follows 1.18's curve: half of it in the first 53 ms, all of it at 180 ms, a
    /// constant deceleration from 2d/T (667 px/s for a 60 px notch) to a stop.
    #[test]
    fn a_notch_glides_on_1_18s_curve() {
        let ms = Duration::from_millis;
        let origin = Instant::now();
        let at_micros = |us| {
            let (mut g, _) = Glide::notch(
                None,
                LogicalPosition::default(),
                (0.0, 60.0),
                NOTCH_TIME,
                origin,
            );
            g.step(origin + Duration::from_micros(us)).1
        };
        let at = |t| glide_at(60.0, 2.0 * 60.0 / 0.18, NOTCH_TIME, ms(t)).0;
        assert_eq!(at(0), 0.0);
        assert!((at(45) - 26.25).abs() < 1e-3, "{}", at(45));
        assert!((at(90) - 45.0).abs() < 1e-3, "{}", at(90));
        assert!((at(135) - 56.25).abs() < 1e-3, "{}", at(135));
        assert_eq!(at(180), 60.0);
        assert_eq!(at(400), 60.0);
        assert!(at(52) < 30.0 && at(53) > 30.0);
        // The start speed: 2d/T.
        let speed = (at_micros(100) / 0.0001).round();
        assert!((speed - 667.0).abs() <= 1.0, "{speed}");
    }

    #[test]
    fn notches_hand_over_steps_that_add_up_to_them() {
        let origin = Instant::now();
        let at = LogicalPosition::new(400.0, 300.0);
        let t = |ms| origin + Duration::from_millis(ms);
        // A notch down, frames every 7 ms.
        let (mut g, rest) = Glide::notch(None, at, (0.0, -60.0), NOTCH_TIME, t(0));
        assert_eq!(rest, None);
        let mut steps = Vec::new();
        for ms in (7..).step_by(7) {
            let (_, dy, over) = g.step(t(ms));
            steps.push(dy);
            if over {
                break;
            }
        }
        assert_eq!(steps.len(), 26, "one a frame until 180 ms");
        assert_eq!(steps.iter().sum::<f32>(), -60.0);
        assert!(steps.windows(2).all(|w| w[0] <= w[1]), "slowing: {steps:?}");

        // A second notch 90 ms into the first adds the 15 px it had left, from then.
        let (mut g, _) = Glide::notch(None, at, (0.0, -60.0), NOTCH_TIME, t(0));
        g.step(t(90));
        let (mut g, rest) = Glide::notch(Some(g), at, (0.0, -60.0), NOTCH_TIME, t(90));
        assert_eq!(rest, None);
        assert_eq!(g.step(t(270)), (0.0, -75.0, true));
        // Elsewhere, what was left goes there at once.
        let (mut g, _) = Glide::notch(None, at, (0.0, -60.0), NOTCH_TIME, t(0));
        g.step(t(90));
        let there = LogicalPosition::new(10.0, 10.0);
        let (_, rest) = Glide::notch(Some(g), there, (0.0, 60.0), NOTCH_TIME, t(90));
        assert_eq!(rest, Some((at, 0.0, -15.0)));
    }

    /// A fast spin (a notch every 30 ms, frames every 4.4 ms, out of step with the notches):
    /// every frame moves the list about as far as the frames beside it. The glide used to
    /// start again from where the last frame left it, so the frame after each notch stood
    /// nearly still, and each notch kicked the speed up again.
    #[test]
    fn a_fast_spin_moves_evenly_from_frame_to_frame() {
        for (every, time, notch) in [(30, NOTCH_TIME, -60.0), (4, FINE_TIME, -7.5)] {
            let origin = Instant::now();
            let at = LogicalPosition::new(400.0, 300.0);
            let notches: Vec<u64> = (0..40).map(|i| 3_000 + i * every * 1_000).collect();
            let mut glide: Option<Glide> = None;
            let (mut steps, mut sum, mut next) = (Vec::new(), 0.0, 0);
            let mut frame = 0;
            while frame < 2_000_000 {
                frame += 4_400;
                while next < notches.len() && notches[next] <= frame {
                    let when = origin + Duration::from_micros(notches[next]);
                    glide = Some(Glide::notch(glide, at, (0.0, notch), time, when).0);
                    next += 1;
                }
                if let Some(mut g) = glide {
                    let (_, dy, over) = g.step(origin + Duration::from_micros(frame));
                    glide = (!over).then_some(g);
                    sum += dy;
                    steps.push((frame, dy));
                }
            }
            assert!(
                (sum - notch * 40.0).abs() < 1e-2,
                "every notch covered: {sum}"
            );
            // Between the first notches' start and the last notch.
            let mid: Vec<f32> = steps
                .iter()
                .filter(|(t, _)| *t > notches[4] && *t < notches[39])
                .map(|s| s.1)
                .collect();
            for w in mid.windows(3) {
                let beside = (w[0] + w[2]) / 2.0;
                assert!(
                    (w[1] - beside).abs() <= 0.05 * beside.abs(),
                    "every {every} ms: a step of {} between {} and {}",
                    w[1],
                    w[0],
                    w[2]
                );
            }
        }
    }

    /// The volume's wheel handler (`bottom_bar.slint`, `wheel`) takes each step as it comes:
    /// a notch handed over in steps lands where the notch in one went.
    #[test]
    fn the_volume_lands_where_one_notch_put_it() {
        let wheel = |volume: f32, dy: f32| (volume + dy / 60.0 * 5.0 / 100.0).clamp(0.0, 1.0);
        let origin = Instant::now();
        let at = LogicalPosition::new(0.0, 0.0);
        for (start, notch) in [(0.5, 60.0), (0.5, -60.0), (0.98, 60.0), (0.02, -60.0)] {
            let (mut g, _) = Glide::notch(None, at, (0.0, notch), NOTCH_TIME, origin);
            let mut volume = start;
            for ms in (16..).step_by(16) {
                let (_, dy, over) = g.step(origin + Duration::from_millis(ms));
                volume = wheel(volume, dy);
                if over {
                    break;
                }
            }
            let once = wheel(start, notch);
            assert!(
                (volume - once).abs() < 1e-5,
                "{start} {notch}: {volume} vs {once}"
            );
            assert_eq!((volume * 100.0).round(), (once * 100.0).round());
        }
    }

    #[test]
    fn slint_gets_the_shift_its_key_events_missed() {
        // A press and release Slint saw: the modifiers that follow agree.
        let mut s = ShiftSync::default();
        s.key(true);
        assert_eq!(s.modifiers(true), None);
        s.key(false);
        assert_eq!(s.modifiers(false), None);
        // Shift held as the focus arrives (Wayland: the modifiers, no key event), then let go
        // with no key event either.
        assert_eq!(s.modifiers(true), Some(true));
        assert_eq!(s.modifiers(true), None);
        assert_eq!(s.modifiers(false), Some(false));
        // Held as the focus arrives, let go as a key event.
        assert_eq!(s.modifiers(true), Some(true));
        s.key(false);
        assert_eq!(s.modifiers(false), None);
        // Both Shifts down, the left let go: Slint drops Shift with it, the right still holds it.
        s.key(true);
        s.key(true);
        s.key(false);
        assert_eq!(s.modifiers(true), Some(true));
        // The focus goes, and Slint's modifiers with it.
        s.focus_lost();
        assert_eq!(s.modifiers(false), None);
    }
}
