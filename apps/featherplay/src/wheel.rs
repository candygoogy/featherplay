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
//! pointer, or a button, lifts it before Slint sees that event. Nothing is timed.
//!
//! The same filter, the only one a window can have, sends the mouse's side buttons to
//! `Nav.back` and `Nav.forward` (`mouse_nav`), and tells `window_place` when the window moves
//! or changes size.

use std::cell::Cell;

use slint::platform::WindowEvent as SlintEvent;
use slint::winit_030::winit::event::{MouseScrollDelta, WindowEvent};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, LogicalPosition};

use crate::mouse_nav::{self, Press, Step};
use crate::ui::{AppWindow, Frame, Nav, Theme};

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

/// Sets `Frame.scrolling`, if it isn't already.
fn set_scrolling(ui: &AppWindow, on: bool) {
    let frame = ui.global::<Frame>();
    if frame.get_scrolling() != on {
        frame.set_scrolling(on);
    }
}

/// Installs the filter on the app's window (a winit window; anything else is left alone).
pub fn install(ui: &AppWindow) {
    let weak = ui.as_weak();
    // Where the pointer is, in logical pixels: winit's wheel events don't say.
    let pointer: Cell<Option<LogicalPosition>> = Cell::new(None);
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
}
