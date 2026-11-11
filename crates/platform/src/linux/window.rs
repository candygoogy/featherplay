//! Window operations on a plain handle: stubs until phase 5. The watches are made but never
//! fire, the queries find nothing, and the title bar keeps the desktop's colours
//! (`supports(Feature::TitleBarColour)` is false).

pub use crate::common::window::*;

use crate::{Feature, Unsupported};

/// Always an error: `Feature::TitleBarColour` isn't supported here.
pub fn title_bar(_h: isize, _dark: bool, _caption: Rgb, _text: Rgb) -> Result<(), String> {
    Err(Unsupported(Feature::TitleBarColour).to_string())
}

/// Nothing to do: the window is opaque already.
pub fn opaque(_h: isize) -> Result<(), String> {
    Ok(())
}

/// Never calls back yet; each paint runs as it would without the watch.
pub fn repaint_when_invalidated(
    _h: isize,
    _invalidated: impl Fn() + 'static,
    _around_paint: impl Fn(bool, &mut dyn FnMut()) + 'static,
    _wheel: impl Fn(i16) + 'static,
) -> Result<(), String> {
    Ok(())
}

/// Leaves the icon to the desktop entry.
pub fn set_icon(_h: isize, _scale: f32, _ico: &[u8]) -> Result<(), String> {
    Ok(())
}

/// Never calls back yet.
pub fn when_colours_change(_h: isize, _changed: impl Fn() + 'static) -> Result<(), String> {
    Ok(())
}

/// Never calls back yet.
pub fn when_minimised(_h: isize, _changed: impl Fn(bool) + 'static) -> Result<(), String> {
    Ok(())
}

/// Never calls back yet.
pub fn when_displays_change(_h: isize, _changed: impl Fn() + 'static) -> Result<(), String> {
    Ok(())
}

/// Never calls back yet.
pub fn when_sizing(_h: isize, _changed: impl Fn(bool) + 'static) -> Result<(), String> {
    Ok(())
}

/// Does nothing yet.
pub fn bring_forward(_h: isize) {}

/// Does nothing yet.
pub fn minimise(_h: isize) {}

/// Never calls back yet.
pub fn when_pressed(_h: isize, _pressed: impl Fn(bool) + 'static) -> Result<(), String> {
    Ok(())
}

/// No window is known to be in front yet.
pub fn foreground() -> Option<Foreground> {
    None
}

/// A watch on presses anywhere on screen, which this OS doesn't have yet.
pub struct PressWatch {
    _none: (),
}

/// Always an error: `Feature::MiniPlayerPlacement` (which closes the mini player on a press
/// elsewhere) isn't supported here.
pub fn watch_presses(_on_press: fn(&str)) -> Result<PressWatch, String> {
    Err(Unsupported(Feature::MiniPlayerPlacement).to_string())
}

/// Not known yet: never minimised.
pub fn is_minimised(_h: isize) -> bool {
    false
}

/// Not known yet.
pub fn client_origin(_h: isize) -> Option<(i32, i32)> {
    None
}

/// Not known yet: no frame.
pub fn frame_insets(_scale: f32) -> ScreenRect {
    (0, 0, 0, 0)
}

/// Never calls back yet.
pub fn when_taskbar_button(_h: isize, _created: impl Fn() + 'static) -> Result<(), String> {
    Ok(())
}

/// Not known yet.
pub fn work_area_at(_x: i32, _y: i32) -> Option<(ScreenRect, f32)> {
    None
}

/// Nothing to do: the desktop draws its own border, if any.
pub fn popup_border(_h: isize, _colour: Rgb) -> Result<(), String> {
    Ok(())
}

/// Animations on, as on Windows when the setting can't be read.
pub fn animations_enabled() -> bool {
    true
}
