//! The window code of the tray's mini player (`mini`): a borderless window with no taskbar
//! button, placed by the tray icon like Windows 11's flyouts, opaque, with a hairline border,
//! that takes the activation (on Wayland, an ordinary window the desktop places: `placed`);
//! and the main window's `Theme` and `Player` copied into its own globals, its buttons going
//! through the main window's `Player` callbacks.

use std::cell::Cell;

use featherplay_platform::window::{self, Edge, Handle, ScreenRect, Taskbar};
use featherplay_platform::{Feature, supports};
use slint::winit_030::WinitWindowAccessor;
#[cfg(windows)]
use slint::winit_030::winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};
use slint::winit_030::winit::window::WindowAttributes;
use slint::{ComponentHandle, PhysicalPosition};

use crate::ui::{AppWindow, Appearance, Player, Theme};
use crate::ui_bridge::format_duration;
use crate::{bar, chrome, shell};

/// From the taskbar and the screen's edges, in logical pixels (Windows 11's flyouts keep 12).
const GAP: f32 = 12.0;

thread_local! {
    /// Set while a popup's window is created, for `window_attributes`.
    static CREATING: Cell<bool> = const { Cell::new(false) };
    /// Set while one is created that mustn't take the activation as it shows (the desktop
    /// lyrics' strip, over whatever app is in use).
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f`, which creates a popup's window, with `window_attributes` on for it. The window
/// adapter may be made on the first use of `window()` or inside `show`: both go in here.
pub fn creating<T>(f: impl FnOnce() -> T) -> T {
    CREATING.set(true);
    let made = f();
    CREATING.set(false);
    made
}

/// As `creating`, for a popup that shows without taking the activation from the app in
/// front.
pub fn creating_quiet<T>(f: impl FnOnce() -> T) -> T {
    QUIET.set(true);
    let made = creating(f);
    QUIET.set(false);
    made
}

/// The winit attributes of a window about to be created: for a popup, borderless with no
/// taskbar button, rounded with a shadow where Windows draws them (11); where the tray can't
/// place it (Wayland), in the desktop's frame (`placed`). Other windows keep theirs.
pub fn window_attributes(attrs: WindowAttributes) -> WindowAttributes {
    // Every window: the Wayland app id and X11 class the desktop matches to the desktop
    // entry (its name, icon and dock pin).
    #[cfg(target_os = "linux")]
    let attrs = {
        use slint::winit_030::winit::platform::x11::WindowAttributesExtX11;
        WindowAttributesExtX11::with_name(attrs, featherplay_platform::APP_ID, "featherplay")
    };
    if !CREATING.get() {
        return attrs;
    }
    let attrs = attrs
        .with_decorations(!placed())
        .with_resizable(false)
        .with_active(!QUIET.get());
    #[cfg(windows)]
    let attrs = attrs
        .with_skip_taskbar(true)
        .with_undecorated_shadow(true)
        .with_corner_preference(CornerPreference::Round);
    attrs
}

/// Whether popups are placed by the tray icon, borderless: everywhere but Wayland, which
/// lets no app place its windows or learn where the icon is.
pub fn placed() -> bool {
    supports(Feature::MiniPlayerPlacement)
}

/// Shows `window` where the desktop puts it, in its frame (Wayland: `placed` is false).
pub fn show_unplaced(window: &slint::Window) -> Result<(), String> {
    creating(|| window.show()).map_err(|e| format!("show: {e}"))
}

/// Shows `ui`'s window, `size` logical pixels, for a click on the tray icon at `(x, y)`
/// (physical screen pixels): by the click on the taskbar, else (an icon in the hidden icons'
/// flyout) by the taskbar's notification area, on its monitor (`place_from_tray`). Returns
/// where its top-left corner went. Once its window exists (`shell::when_handle`), `dress` it.
pub fn show_at<C: ComponentHandle + 'static>(
    ui: &C,
    (x, y): (i32, i32),
    size: (f32, f32),
) -> Result<(i32, i32), String> {
    let taskbar = window::taskbar();
    let (mut work, mut scale) = window::work_area_at(x, y).ok_or("no monitor at the click")?;
    if let Some(t) = taskbar.filter(|t| !on_taskbar((x, y), work, *t)) {
        let (l, top, r, b) = t.rect;
        if let Some(there) = window::work_area_at((l + r) / 2, (top + b) / 2) {
            (work, scale) = there;
        }
    }
    let physical = (
        (size.0 * scale).round() as i32,
        (size.1 * scale).round() as i32,
    );
    let gap = (GAP * scale).round() as i32;
    let (left, top) = place_from_tray((x, y), physical, work, gap, taskbar);
    let window = ui.window();
    window.set_position(PhysicalPosition::new(left, top));
    creating(|| window.show()).map_err(|e| format!("show: {e}"))?;
    // Placed again by the size the window really got, once it exists: right after `show`,
    // Slint's own size can still be the one at 100 %.
    shell::when_handle(
        ui,
        move |ui, _| {
            let real = ui.window().with_winit_window(|w| {
                let s = w.inner_size();
                (s.width as i32, s.height as i32)
            });
            let Some(real) = real.filter(|r| r.0 > 0 && *r != physical) else {
                return;
            };
            let (left, top) = place_from_tray((x, y), real, work, gap, taskbar);
            ui.window().set_position(PhysicalPosition::new(left, top));
            log!("popup: {}x{} px, moved to {left},{top}", real.0, real.1);
        },
        |_| {},
    );
    Ok((left, top))
}

/// Makes the shown popup's window `handle` opaque, with a hairline `border`, and gives it the
/// activation.
pub fn dress(handle: Handle, border: slint::Color) {
    if let Err(e) = window::opaque(handle) {
        log!("popup: opaque: {e}");
    }
    let _ = chrome::popup_border(handle, border);
    window::bring_forward(handle);
}

/// Where a popup opened from the tray icon goes, everything in physical pixels: for a click
/// at `point` on the `taskbar`, by the click (`place`); for one elsewhere (an icon in the
/// hidden icons' flyout), as Windows' own flyouts sit: in the corner of the work area at the
/// notification area's end of the taskbar, `gap` from it and from the screen's edge. `work`
/// is the work area of the taskbar's monitor then. A taskbar that hides itself lies over the
/// work area: the popup stays clear of it all the same. With no taskbar known, by the click.
pub fn place_from_tray(
    point: (i32, i32),
    size: (i32, i32),
    work: ScreenRect,
    gap: i32,
    taskbar: Option<Taskbar>,
) -> (i32, i32) {
    let Some(t) = taskbar else {
        return place(point, size, work, gap);
    };
    let work = clear_of(work, t);
    if on_taskbar(point, work, t) {
        return place(point, size, work, gap);
    }
    let ((w, h), (left, top, right, bottom)) = (size, work);
    match t.edge {
        Edge::Bottom | Edge::Right => (right - gap - w, bottom - gap - h),
        Edge::Top => (right - gap - w, top + gap),
        Edge::Left => (left + gap, bottom - gap - h),
    }
}

/// Whether `point` is on the taskbar `t`: on it, or outside the work area, where only a
/// taskbar is.
fn on_taskbar(point: (i32, i32), work: ScreenRect, t: Taskbar) -> bool {
    let inside =
        |(px, py): (i32, i32), (l, top, r, b): ScreenRect| px >= l && px < r && py >= top && py < b;
    inside(point, t.rect) || !inside(point, work)
}

/// `work` less the taskbar `t` where it lies over it (one that hides itself).
fn clear_of(work: ScreenRect, t: Taskbar) -> ScreenRect {
    let ((l, top, r, b), (tl, tt, tr, tb)) = (work, t.rect);
    if tl >= r || tr <= l || tt >= b || tb <= top {
        return work;
    }
    match t.edge {
        Edge::Bottom => (l, top, r, b.min(tt)),
        Edge::Top => (l, top.max(tb), r, b),
        Edge::Left => (l.max(tr), top, r, b),
        Edge::Right => (l, top, r.min(tl), b),
    }
}

/// Where a popup's top-left corner goes for a click at `point`, everything in physical
/// pixels: above a taskbar at the bottom, below one at the top, beside one on the left or
/// right, centred on the click along the taskbar; for a point inside the work area (the
/// bar's button where there's no tray), above the point, or below it when there's no room.
/// Always inside the work area, `gap` from its edges.
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
    to.set_preset_dark(from.get_preset_dark());
    to.set_preset_light(from.get_preset_light());
    sync_follow(from, to);
}

/// Copies the main window's cover colours into a popup's: the accent, the bar's tint, the
/// secondary text, and whether the accent is the neutral one (its glow and hover).
pub fn sync_follow(from: &Theme, to: &Theme) {
    to.set_follow_accent(from.get_follow_accent());
    to.set_follow_bar(from.get_follow_bar());
    to.set_follow_text2(from.get_follow_text2());
    to.set_follow_neutral(from.get_follow_neutral());
    to.set_follow(from.get_follow());
}

/// Copies where the main window's theme switch is (`follow::theme_changed`) into a popup
/// that follows each of its steps, the mini player.
pub fn sync_switch(from: &Theme, to: &Theme) {
    to.set_switch_shade(from.get_switch_shade());
    to.set_switching(from.get_switching());
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
    to.set_disliked(from.get_disliked());
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
    /// The same screen's work area with the taskbar on the top, left or right edge.
    const W_TOP: ScreenRect = (0, 48, 1920, 1080);
    const W_LEFT: ScreenRect = (48, 0, 1920, 1080);
    const W_RIGHT: ScreenRect = (0, 0, 1872, 1080);
    /// 150 % on a 3840 x 2160 screen, a 72 px taskbar.
    const W_150: ScreenRect = (0, 0, 3840, 2088);

    /// A 340 x 120 popup and a 12 px gap at `scale`, in physical pixels.
    fn size_and_gap(scale: f32) -> ((i32, i32), i32) {
        let px = |v: i32| (v as f32 * scale).round() as i32;
        ((px(340), px(120)), px(12))
    }

    #[test]
    fn a_popup_opens_by_the_point_and_stays_on_its_screen() {
        // The "bottom taskbar" row and the 125 %, 150 % and 200 % ones are the same click,
        // 300 px (logical) from the right above a bottom taskbar, at each scale.
        const W_125: ScreenRect = (0, 0, 2400, 1290);
        const W_200: ScreenRect = (0, 0, 3840, 2064);
        const W_WEST: ScreenRect = (-1920, 0, 0, 1032);
        // (case, scale, point, work, want)
        let cases = [
            ("bottom taskbar", 1.0, (1500, 1050), WORK, (1330, 900)),
            ("near the right edge", 1.0, (1900, 1050), WORK, (1568, 900)),
            ("top taskbar", 1.0, (1500, 20), W_TOP, (1330, 60)),
            ("left taskbar", 1.0, (20, 1000), W_LEFT, (60, 868)),
            ("right taskbar", 1.0, (1890, 1000), W_RIGHT, (1520, 868)),
            ("inside the work area", 1.0, (1500, 900), WORK, (1330, 768)),
            ("near its top", 1.0, (800, 50), WORK, (630, 62)),
            ("150 %, corner", 1.5, (3800, 2120), W_150, (3312, 1890)),
            ("125 %", 1.25, (1875, 1313), W_125, (1663, 1125)),
            ("150 %", 1.5, (3000, 2120), W_150, (2745, 1890)),
            ("200 %", 2.0, (3000, 2100), W_200, (2660, 1800)),
            ("left monitor", 1.0, (-100, 1050), W_WEST, (-352, 900)),
        ];
        for (case, scale, point, work, want) in cases {
            let (size, gap) = size_and_gap(scale);
            assert_eq!(place(point, size, work, gap), want, "{case}");
        }
    }

    const fn bar(edge: Edge, rect: ScreenRect) -> Option<Taskbar> {
        Some(Taskbar { edge, rect })
    }

    #[test]
    fn a_tray_popup_opens_by_the_click_or_in_the_flyouts_corner() {
        type Screen = (ScreenRect, Option<Taskbar>);
        // A 48 px taskbar on each edge of a 1920 x 1080 screen, with its work area.
        const BOTTOM: Screen = (WORK, bar(Edge::Bottom, (0, 1032, 1920, 1080)));
        const TOP: Screen = (W_TOP, bar(Edge::Top, (0, 0, 1920, 48)));
        const LEFT: Screen = (W_LEFT, bar(Edge::Left, (0, 0, 48, 1080)));
        const RIGHT: Screen = (W_RIGHT, bar(Edge::Right, (1872, 0, 1920, 1080)));
        const BOTTOM_150: Screen = (W_150, bar(Edge::Bottom, (0, 2088, 3840, 2160)));
        // A taskbar that hides itself: the work area is the whole screen, the taskbar shows
        // over its bottom 48 px.
        const HIDING: Screen = ((0, 0, 1920, 1080), BOTTOM.1);
        const NONE: Screen = (WORK, None);
        // A click on the taskbar opens by the click; one elsewhere (the overflow flyout,
        // which sits above the taskbar inside the work area) in the work area's corner.
        // (case, scale, point, screen, want)
        let cases = [
            ("bottom, on it", 1.0, (1500, 1050), BOTTOM, (1330, 900)),
            ("bottom, flyout", 1.0, (1500, 900), BOTTOM, (1568, 900)),
            ("150 %, flyout", 1.5, (3000, 1800), BOTTOM_150, (3312, 1890)),
            ("top, on it", 1.0, (1500, 20), TOP, (1330, 60)),
            ("top, flyout", 1.0, (1500, 200), TOP, (1568, 60)),
            ("left, on it", 1.0, (20, 1000), LEFT, (60, 868)),
            ("left, flyout", 1.0, (200, 900), LEFT, (60, 948)),
            ("right, on it", 1.0, (1890, 1000), RIGHT, (1520, 868)),
            ("right, flyout", 1.0, (1700, 900), RIGHT, (1520, 948)),
            ("hiding, on it", 1.0, (1500, 1050), HIDING, (1330, 900)),
            ("hiding, flyout", 1.0, (1500, 900), HIDING, (1568, 900)),
            ("no taskbar known", 1.0, (1500, 900), NONE, (1330, 768)),
        ];
        for (case, scale, point, (work, taskbar), want) in cases {
            let (size, gap) = size_and_gap(scale);
            let at = place_from_tray(point, size, work, gap, taskbar);
            assert_eq!(at, want, "{case}");
        }
    }
}
