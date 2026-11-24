//! Wheel and touchpad scrolling, for every list, grid and page at once: a filter on the
//! window's winit events, so no page or panel has its own.
//!
//! A mouse wheel notch eases: Slint's `Flickable` (and so every `ListView`) covers each notch
//! in 180 ms, slowing evenly to a stop (an ease-out), and a notch that arrives while one is
//! still running adds what was left of it, so a fast spin keeps a steady speed. The animation
//! ends with the distance and leaves nothing running. That holds as long as the notch reaches
//! Slint as a line delta in the "moved" phase, which is how winit reports `WM_MOUSEWHEEL`.
//!
//! Two cases mustn't ease, and this filter sends them straight through instead:
//! - Fractions of a notch: a precision touchpad, or a high-resolution wheel, reports many
//!   small steps that already follow the fingers. Easing each of them over 180 ms would make
//!   the list trail behind the hand.
//! - Windows' animation effects turned off (`Theme.motion`): every notch jumps at once.
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

use slint::platform::{Key, WindowEvent as SlintEvent};
use slint::winit_030::winit::event::{MouseScrollDelta, WindowEvent};
use slint::winit_030::winit::keyboard::{Key as WinitKey, NamedKey};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, LogicalPosition};

use crate::mouse_nav::{self, Press, Step};
use crate::ui::{AppWindow, Frame, Immersive, Nav, Theme};

/// Logical pixels per wheel line, as Slint's winit backend scales a line delta.
const LINE: f32 = 60.0;

/// What to do with one wheel event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Route {
    /// Leave it to Slint, which eases whole notches.
    Eased,
    /// Scroll by this many logical pixels at once.
    Direct { dx: f32, dy: f32 },
}

/// How a wheel event with `delta` scrolls. `motion`: Windows' animation effects are on.
pub fn route(delta: MouseScrollDelta, motion: bool) -> Route {
    match delta {
        MouseScrollDelta::LineDelta(x, y) if !motion || !whole(x) || !whole(y) => Route::Direct {
            dx: x * LINE,
            dy: y * LINE,
        },
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
                match (route(*delta, motion), pointer.get()) {
                    (Route::Direct { dx, dy }, Some(position)) => {
                        // No phase: the Flickable moves by the delta at once.
                        window.dispatch_event(SlintEvent::PointerScrolled {
                            position,
                            delta_x: dx,
                            delta_y: dy,
                        });
                        EventResult::PreventDefault
                    }
                    _ => EventResult::Propagate,
                }
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
    fn whole_notches_ease_and_the_rest_goes_straight_through() {
        let pixels = PixelDelta(PhysicalPosition::new(0.0, 12.0));
        // (case, delta, animation effects on, want)
        let cases = [
            ("a notch", LineDelta(0.0, -1.0), true, Route::Eased),
            ("notches", LineDelta(0.0, 3.0), true, Route::Eased),
            ("a tilt notch", LineDelta(1.0, 0.0), true, Route::Eased),
            // 15 of WHEEL_DELTA's 120: an eighth of a notch.
            ("a touchpad step", LineDelta(0.0, 0.125), true, direct(7.5)),
            ("past a notch", LineDelta(0.0, -1.5), true, direct(-90.0)),
            ("effects off", LineDelta(0.0, -1.0), false, direct(-60.0)),
            ("pixel deltas", pixels, true, Route::Eased),
        ];
        for (case, delta, motion, want) in cases {
            assert_eq!(route(delta, motion), want, "{case}");
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
