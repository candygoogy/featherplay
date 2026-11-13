//! Window operations on a window's `Handle` (its X11 window or Wayland surface here). Most
//! are stubs still: the watches are made but never fire, the queries find nothing, and the
//! title bar keeps the desktop's colours (`supports(Feature::TitleBarColour)` is false).
//!
//! On an X11 desktop the mini player is placed by the tray icon as on Windows: `pointer` and
//! `work_area_at` ask the X server (`x11`). Wayland lets an app neither place its windows
//! nor learn where anything is on screen, so there `work_area_at` finds nothing and the mini
//! player opens as an ordinary window (`supports(Feature::MiniPlayerPlacement)` is false).
//! `bring_forward` asks the X11 window manager to bring a window back; Wayland's compositor
//! decides that alone.

use std::sync::OnceLock;

use raw_window_handle::RawWindowHandle;

pub use crate::common::window::*;

use crate::{Feature, Unsupported};

mod x11;

/// Whether the desktop session is X11: `XDG_SESSION_TYPE` says so, or (it's unset or says
/// something else, as in WSL) there's an X display and no Wayland one. A Wayland session's
/// X server (XWayland) serves only the apps that use it: what it says of the pointer, the
/// screen or the keys pressed elsewhere is not the desktop's. Read once.
pub(crate) fn x11_session() -> bool {
    static X11: OnceLock<bool> = OnceLock::new();
    *X11.get_or_init(|| {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        is_x11(
            &session,
            var("WAYLAND_DISPLAY").is_some() || var("WAYLAND_SOCKET").is_some(),
            var("DISPLAY").is_some(),
        )
    })
}

/// `x11_session` for a session type and the displays there are.
fn is_x11(session: &str, wayland: bool, x_display: bool) -> bool {
    match session {
        "x11" => true,
        "wayland" => false,
        _ => x_display && !wayland,
    }
}

/// Where the pointer is on screen (physical pixels), on an X11 desktop; `None` elsewhere.
pub fn pointer() -> Option<(i32, i32)> {
    if !x11_session() {
        return None;
    }
    x11::pointer()
        .inspect_err(|e| log::warn!("pointer: {e}"))
        .ok()
}

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

/// Restores the window if it's minimised, raises it and gives it the focus, for an X11
/// window (XWayland's too: `x11::activate`). On Wayland the compositor alone decides: an app
/// can't un-minimise a window, and activating one takes a token from the user's action,
/// which a second launch would have to hand over; so there it does nothing.
pub fn bring_forward(h: Handle) {
    if let Window::X11(w) = h.0
        && let Err(e) = u32::try_from(w)
            .map_err(|_| "not an X11 window id".to_string())
            .and_then(x11::activate)
    {
        log::warn!("bring forward: {e}");
    }
}

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

/// A watch on presses anywhere on screen, which never fires here: there's no taskbar of the
/// app's to press, and a press on another window takes the focus from the mini player, which
/// closes it (winit's `Focused(false)`).
pub struct PressWatch {
    _none: (),
}

/// A watch that never fires (`PressWatch`).
pub fn watch_presses(_on_press: fn(&str)) -> Result<PressWatch, String> {
    Ok(PressWatch { _none: () })
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

/// On an X11 desktop, the work area (the monitor's, less the panels) of the monitor at
/// `(x, y)` (physical pixels), and the scale winit gives windows there; `None` on Wayland,
/// where nothing is known of the screen.
pub fn work_area_at(x: i32, y: i32) -> Option<(ScreenRect, f32)> {
    if !x11_session() {
        return None;
    }
    x11::work_area_at(x, y)
        .inspect_err(|e| log::warn!("work area: {e}"))
        .ok()
}

/// Nothing to do: X11 lets clicks through by the window's input shape alone, which winit
/// sets.
pub fn layered_opaque(_h: Handle) -> Result<(), String> {
    Ok(())
}

/// Nothing to do: the desktop draws its own border, if any.
pub fn popup_border(_h: Handle, _colour: Rgb) -> Result<(), String> {
    Ok(())
}

/// Animations on, as on Windows when the setting can't be read.
pub fn animations_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_type_decides_and_else_the_displays_there_are() {
        assert!(is_x11("x11", true, true));
        assert!(!is_x11("wayland", true, true));
        // WSL, and a desktop started by hand: no session type.
        assert!(
            !is_x11("", true, true),
            "a Wayland display makes it Wayland"
        );
        assert!(is_x11("", false, true));
        assert!(!is_x11("tty", false, false));
    }
}
