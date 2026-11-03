//! The window code of the tray's mini player (`mini`): a borderless window with no taskbar
//! button, placed by the tray icon like Windows 11's flyouts, opaque, with a hairline border,
//! that takes the activation; and the main window's `Theme` and `Player` copied into its own
//! globals, its buttons going through the main window's `Player` callbacks.

use std::cell::Cell;

use slint::winit_030::winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};
use slint::winit_030::winit::window::WindowAttributes;
use slint::{ComponentHandle, PhysicalPosition};

use crate::bar;
use crate::native::{self, ScreenRect};
use crate::shell;
use crate::ui::{AppWindow, Appearance, Player, Theme};
use crate::ui_bridge::format_duration;

/// From the taskbar and the screen's edges, in logical pixels (Windows 11's flyouts keep 12).
const GAP: f32 = 12.0;

thread_local! {
    /// Set while a popup's window is created, for `window_attributes`.
    static CREATING: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f`, which creates a popup's window, with `window_attributes` on for it. The window
/// adapter may be made on the first use of `window()` or inside `show`: both go in here.
pub fn creating<T>(f: impl FnOnce() -> T) -> T {
    CREATING.set(true);
    let made = f();
    CREATING.set(false);
    made
}

/// The winit attributes of a window about to be created: for a popup, borderless with no
/// taskbar button, rounded with a shadow where Windows draws them (11). Other windows keep
/// theirs.
pub fn window_attributes(attrs: WindowAttributes) -> WindowAttributes {
    if !CREATING.get() {
        return attrs;
    }
    attrs
        .with_decorations(false)
        .with_resizable(false)
        .with_skip_taskbar(true)
        .with_undecorated_shadow(true)
        .with_corner_preference(CornerPreference::Round)
}

/// Shows `window`, `size` logical pixels, for a click on the tray icon at `(x, y)` (physical
/// screen pixels), and gives it the activation. Returns where its top-left corner went.
pub fn show_at(
    window: &slint::Window,
    (x, y): (i32, i32),
    size: (f32, f32),
    border: slint::Color,
) -> Result<(i32, i32), String> {
    let (work, scale) = native::work_area_at(x, y).ok_or("no monitor at the click")?;
    let physical = (
        (size.0 * scale).round() as i32,
        (size.1 * scale).round() as i32,
    );
    let gap = (GAP * scale).round() as i32;
    let (left, top) = place((x, y), physical, work, gap);
    window.set_position(PhysicalPosition::new(left, top));
    creating(|| window.show()).map_err(|e| format!("show: {e}"))?;
    // The monitor's scale may differ from the one the window got; place it by its own size.
    let real = window.size();
    let real = (real.width as i32, real.height as i32);
    let (mut left, mut top) = (left, top);
    if real != physical && real.0 > 0 {
        (left, top) = place((x, y), real, work, gap);
        window.set_position(PhysicalPosition::new(left, top));
    }
    if let Some(hwnd) = shell::hwnd(window) {
        if let Err(e) = native::opaque(hwnd) {
            log!("popup: opaque: {e}");
        }
        let _ = native::popup_border(hwnd, border);
        native::bring_forward(hwnd);
    }
    Ok((left, top))
}

/// Where a popup's top-left corner goes for a click at `point`, everything in physical
/// pixels: above a taskbar at the bottom, below one at the top, beside one on the left or
/// right, centred on the click along the taskbar; for a click inside the work area (the
/// hidden icons' flyout), above the point, or below it when there's no room. Always inside
/// the work area, `gap` from its edges.
pub fn place(point: (i32, i32), size: (i32, i32), work: ScreenRect, gap: i32) -> (i32, i32) {
    let ((px, py), (w, h), (left, top, right, bottom)) = (point, size, work);
    let x = if px < left {
        left + gap
    } else if px >= right {
        right - gap - w
    } else {
        px - w / 2
    };
    let y = if py >= bottom {
        bottom - gap - h
    } else if py < top {
        top + gap
    } else if py - gap - h >= top + gap {
        py - gap - h
    } else {
        py + gap
    };
    let clamp = |v: i32, lo: i32, hi: i32| v.min(hi).max(lo);
    (
        clamp(x, left + gap, right - gap - w),
        clamp(y, top + gap, bottom - gap - h),
    )
}

/// The border colour for a popup in `theme`: a step from its ground towards the text.
pub fn border(theme: &Theme) -> slint::Color {
    theme.get_bar().mix(&theme.get_text1(), 0.86)
}

/// Copies the main window's theme into a popup's.
pub fn sync_theme(from: &Theme, to: &Theme, appearance: Appearance) {
    to.set_motion(from.get_motion());
    to.set_still(from.get_still());
    to.invoke_apply(appearance);
    to.set_follow_accent(from.get_follow_accent());
    to.set_preset_dark(from.get_preset_dark());
    to.set_preset_light(from.get_preset_light());
    to.set_follow_bar(from.get_follow_bar());
    to.set_follow_text2(from.get_follow_text2());
    to.set_follow(from.get_follow());
}

/// Copies the main window's player into a popup's.
pub fn sync_player(from: &Player, to: &Player) {
    to.set_has_track(from.get_has_track());
    to.set_track_id(from.get_track_id());
    to.set_title(from.get_title());
    to.set_artist(from.get_artist());
    to.set_playing(from.get_playing());
    to.set_loading(from.get_loading());
    to.set_liked(from.get_liked());
    to.set_duration_s(from.get_duration_s());
    to.set_total(from.get_total());
    // The clock rather than the bar's properties, which stand still while it's minimized.
    let at = bar::position().unwrap_or_default();
    to.set_position_s(at.as_secs_f32());
    to.set_elapsed(format_duration(at).into());
}

/// A popup's transport goes through the main window's `Player` callbacks, the bar's path.
/// `liked` runs after a like is toggled, so the popup can show it.
pub fn wire_player(to: &Player, main: &AppWindow, liked: fn()) {
    let m = main.as_weak();
    to.on_toggle(move || {
        if let Some(main) = m.upgrade() {
            main.global::<Player>().invoke_toggle();
        }
    });
    let m = main.as_weak();
    to.on_next(move || {
        if let Some(main) = m.upgrade() {
            main.global::<Player>().invoke_next();
        }
    });
    let m = main.as_weak();
    to.on_previous(move || {
        if let Some(main) = m.upgrade() {
            main.global::<Player>().invoke_previous();
        }
    });
    let m = main.as_weak();
    to.on_toggle_like(move || {
        let Some(main) = m.upgrade() else { return };
        // The bar's handler flips the heart at once and sends the rating.
        main.global::<Player>().invoke_toggle_like();
        liked();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1920 x 1080 screen with a 48 px taskbar at the bottom.
    const WORK: ScreenRect = (0, 0, 1920, 1032);
    const SIZE: (i32, i32) = (340, 120);

    #[test]
    fn bottom_taskbar_opens_above_it_centred_on_the_click() {
        assert_eq!(place((1500, 1050), SIZE, WORK, 12), (1330, 900));
    }

    #[test]
    fn near_the_right_edge_it_stays_on_screen() {
        assert_eq!(place((1900, 1050), SIZE, WORK, 12), (1568, 900));
    }

    #[test]
    fn top_taskbar_opens_below_it() {
        let work = (0, 48, 1920, 1080);
        assert_eq!(place((1500, 20), SIZE, work, 12), (1330, 60));
    }

    #[test]
    fn left_taskbar_opens_beside_it() {
        let work = (48, 0, 1920, 1080);
        assert_eq!(place((20, 1000), SIZE, work, 12), (60, 868));
    }

    #[test]
    fn right_taskbar_flips_to_the_left_of_it() {
        let work = (0, 0, 1872, 1080);
        assert_eq!(place((1890, 1000), SIZE, work, 12), (1520, 868));
    }

    #[test]
    fn overflow_flyout_click_opens_above_the_point() {
        assert_eq!(place((1500, 900), SIZE, WORK, 12), (1330, 768));
    }

    #[test]
    fn click_near_the_top_of_the_work_area_opens_below_the_point() {
        assert_eq!(place((800, 50), SIZE, WORK, 12), (630, 62));
    }

    #[test]
    fn scaled_screen_keeps_the_gap_in_physical_pixels() {
        // 150 %: 510 x 180 physical, an 18 px gap, a 72 px taskbar.
        let work = (0, 0, 3840, 2088);
        assert_eq!(place((3800, 2120), (510, 180), work, 18), (3312, 1890));
    }

    #[test]
    fn second_monitor_to_the_left_uses_its_own_work_area() {
        let work = (-1920, 0, 0, 1032);
        assert_eq!(place((-100, 1050), SIZE, work, 12), (-352, 900));
    }
}
