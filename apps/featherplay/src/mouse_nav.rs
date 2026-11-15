//! The mouse's side buttons (4 and 5) go back and forward, as Alt+Left and Alt+Right do.
//! Called from the window's winit event filter (`wheel.rs`), the only one a window has.

use slint::winit_030::winit::event::{ElementState, MouseButton};

/// Where a side button goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Back,
    Forward,
}

/// What a mouse button event does here: `None` for buttons this leaves to Slint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// A side button goes down: nothing yet, and Slint doesn't see it.
    Held,
    /// A side button comes up: go. On release, as browsers do, so a held button doesn't
    /// repeat and a press that started elsewhere still counts once.
    Go(Step),
}

/// How a mouse button event is handled.
pub fn press(button: MouseButton, state: ElementState) -> Option<Press> {
    let step = match button {
        MouseButton::Back => Step::Back,
        MouseButton::Forward => Step::Forward,
        _ => return None,
    };
    Some(match state {
        ElementState::Pressed => Press::Held,
        ElementState::Released => Press::Go(step),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_buttons_hold_then_go_and_the_rest_is_left_to_slint() {
        use ElementState::{Pressed, Released};
        use MouseButton::{Back, Forward, Left, Middle, Right};
        use Press::{Go, Held};
        // (case, button, state, want): a side button going down only holds, coming up goes;
        // other buttons are left to Slint.
        let cases = [
            ("button 4 up", Back, Released, Some(Go(Step::Back))),
            ("button 5 up", Forward, Released, Some(Go(Step::Forward))),
            ("button 4 down", Back, Pressed, Some(Held)),
            ("left", Left, Released, None),
            ("right", Right, Released, None),
            ("middle", Middle, Released, None),
        ];
        for (case, button, state, want) in cases {
            assert_eq!(press(button, state), want, "{case}");
        }
    }
}
