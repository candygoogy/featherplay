//! Window operations on a window's `Handle` (its X11 window or Wayland surface here): stubs
//! until phase 5. The watches are made but never fire, the queries find nothing, and the
//! title bar keeps the desktop's colours (`supports(Feature::TitleBarColour)` is false).

use raw_window_handle::RawWindowHandle;

pub use crate::common::window::*;

use crate::{Feature, Unsupported};

/// A window, as the window functions take it: here its X11 window or its Wayland surface.
/// Made from the windowing library's raw handle (`from_raw`); its `Debug` prints it in hex.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Handle(Window);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Window {
    /// The X11 window id (Xlib or XCB).
    X11(u64),
    /// The `wl_surface` pointer's address.
    Wayland(usize),
}

impl Handle {
    /// The window `raw` names, if it's an X11 window or a Wayland surface.
    pub fn from_raw(raw: RawWindowHandle) -> Option<Handle> {
        match raw {
            RawWindowHandle::Xlib(h) => Some(Handle(Window::X11(h.window))),
            RawWindowHandle::Xcb(h) => Some(Handle(Window::X11(u64::from(h.window.get())))),
            RawWindowHandle::Wayland(h) => {
                Some(Handle(Window::Wayland(h.surface.as_ptr() as usize)))
            }
            _ => None,
        }
    }
}

impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Window::X11(w) => write!(f, "X11 {w:#x}"),
            Window::Wayland(s) => write!(f, "Wayland {s:#x}"),
        }
    }
}

/// Always an error: `Feature::TitleBarColour` isn't supported here.
pub fn title_bar(_h: Handle, _dark: bool, _caption: Rgb, _text: Rgb) -> Result<(), String> {
    Err(Unsupported(Feature::TitleBarColour).to_string())
}

/// Nothing to do: the window is opaque already.
pub fn opaque(_h: Handle) -> Result<(), String> {
    Ok(())
}

/// Never calls back yet; each paint runs as it would without the watch.
pub fn repaint_when_invalidated(
    _h: Handle,
    _invalidated: impl Fn() + 'static,
    _around_paint: impl Fn(bool, &mut dyn FnMut()) + 'static,
    _wheel: impl Fn(i16) + 'static,
) -> Result<(), String> {
    Ok(())
}

/// Leaves the icon to the desktop entry.
pub fn set_icon(_h: Handle, _scale: f32, _ico: &[u8]) -> Result<(), String> {
    Ok(())
}

/// Never calls back yet.
pub fn when_colours_change(_h: Handle, _changed: impl Fn() + 'static) -> Result<(), String> {
    Ok(())
}

/// Never calls back yet.
pub fn when_minimised(_h: Handle, _changed: impl Fn(bool) + 'static) -> Result<(), String> {
    Ok(())
}

/// Never calls back yet.
pub fn when_displays_change(_h: Handle, _changed: impl Fn() + 'static) -> Result<(), String> {
    Ok(())
}

/// Never calls back yet.
pub fn when_sizing(_h: Handle, _changed: impl Fn(bool) + 'static) -> Result<(), String> {
    Ok(())
}

/// Does nothing yet.
pub fn bring_forward(_h: Handle) {}

/// Does nothing yet.
pub fn minimise(_h: Handle) {}

/// Never calls back yet.
pub fn when_pressed(_h: Handle, _pressed: impl Fn(bool) + 'static) -> Result<(), String> {
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
pub fn is_minimised(_h: Handle) -> bool {
    false
}

/// Not known yet.
pub fn client_origin(_h: Handle) -> Option<(i32, i32)> {
    None
}

/// Not known yet: no frame.
pub fn frame_insets(_scale: f32) -> ScreenRect {
    (0, 0, 0, 0)
}

/// Never calls back yet.
pub fn when_taskbar_button(_h: Handle, _created: impl Fn() + 'static) -> Result<(), String> {
    Ok(())
}

/// Not known yet.
pub fn work_area_at(_x: i32, _y: i32) -> Option<(ScreenRect, f32)> {
    None
}

/// Nothing to do: the desktop draws its own border, if any.
pub fn popup_border(_h: Handle, _colour: Rgb) -> Result<(), String> {
    Ok(())
}

/// Animations on, as on Windows when the setting can't be read.
pub fn animations_enabled() -> bool {
    true
}
