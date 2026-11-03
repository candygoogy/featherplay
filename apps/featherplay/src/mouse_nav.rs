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
    fn button_4_released_goes_back() {
        assert_eq!(
            press(MouseButton::Back, ElementState::Released),
            Some(Press::Go(Step::Back))
        );
    }

    #[test]
    fn button_5_released_goes_forward() {
        assert_eq!(
            press(MouseButton::Forward, ElementState::Released),
            Some(Press::Go(Step::Forward))
        );
    }

    #[test]
    fn a_side_button_going_down_only_holds() {
        assert_eq!(
            press(MouseButton::Back, ElementState::Pressed),
            Some(Press::Held)
        );
    }

    #[test]
    fn other_buttons_are_left_to_slint() {
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            assert_eq!(press(button, ElementState::Released), None);
        }
    }
}
