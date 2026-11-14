//! The main window's place between runs, with no window: where a saved place opens on the
//! monitors there are now (`opening`), where it is then if it opens maximised (`maximised`),
//! and the place to save for bounds the window reports (`place_of`). The app reads and sets the window (`apps/featherplay/src/window_place.rs`).
//!
//! A saved place opens at its size, cut to the work area of the monitor it lands on and never
//! below the window's minimum, and at its position when its title bar shows on a monitor
//! there now; otherwise (that monitor is gone, or the layout changed) it opens centred on the
//! primary monitor. The size is kept in logical pixels, so after a change of scale between
//! runs the window looks as big as before; the position is the screen's, in physical pixels.
//! Where no monitor is known (Wayland, which places windows itself), only the size counts.

use featherplay_platform::window::{Monitor, ScreenRect};
use featherplay_store::WindowPlace;

/// How tall a band at the window's top, its title bar, must show for the window to count as
/// on screen (logical pixels), half of it at least.
const TITLE_BAR: f64 = 32.0;
/// How much of that band's width must show, logical pixels (all of it for a narrower window).
const REACH: f64 = 120.0;

/// The frame round a window's client area at scale 1 (left, top, right, bottom), logical
/// pixels: `platform::window::frame_insets(1.0)`, none where the desktop draws it apart.
pub type Frame = (f64, f64, f64, f64);

/// How the window opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opening {
    /// The inner (client area) size, logical pixels.
    pub size: (f64, f64),
    /// The outer top-left corner, physical pixels; `None` leaves it to the desktop.
    pub position: Option<(i32, i32)>,
    pub maximised: bool,
}

/// Where a window that opens maximised is made (Windows), as `maximised` finds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Maximised {
    /// The inner (client area) size, logical pixels.
    pub size: (f64, f64),
    /// The outer top-left corner, physical pixels.
    pub position: (i32, i32),
    /// The outer rectangle restoring it brings back, physical pixels: the opening's.
    pub normal: ScreenRect,
}

/// Where `saved` opens on `monitors`, for a window whose least inner size is `min` (logical
/// pixels) with `frame` round it; `None` for a place whose numbers can't be used, which opens
/// the window as if nothing was saved.
pub fn opening(
    saved: &WindowPlace,
    monitors: &[Monitor],
    min: (f64, f64),
    frame: Frame,
) -> Option<Opening> {
    if !saved.usable() {
        return None;
    }
    let wanted = (saved.width.max(min.0), saved.height.max(min.1));
    let maximised = saved.maximised;
    let at = saved
        .position
        .map(|[x, y]| (to_px(x, saved.scale), to_px(y, saved.scale)));
    if let Some(at) = at {
        let shows = |m: &&Monitor| visible(outer(at, wanted, frame, m.scale), m.scale, m.work);
        // Its own monitor, if it's still there; else the one it shows most on.
        let landed = monitors
            .iter()
            .filter(|m| saved.monitor == Some(corners(m.rect)))
            .find(shows)
            .or_else(|| {
                monitors
                    .iter()
                    .filter(shows)
                    .max_by_key(|m| area(outer(at, wanted, frame, m.scale), m.work))
            });
        if let Some(m) = landed {
            let size = fit(wanted, m, min, frame);
            // Cut to the work area, it starts at the work area's edge, where it then fits.
            let x = if size.0 < wanted.0 { m.work.0 } else { at.0 };
            let y = if size.1 < wanted.1 { m.work.1 } else { at.1 };
            return Some(Opening {
                size,
                position: Some((x, y)),
                maximised,
            });
        }
    }
    let Some(primary) = monitors.iter().find(|m| m.primary).or(monitors.first()) else {
        return Some(Opening {
            size: wanted,
            position: None,
            maximised,
        });
    };
    let size = fit(wanted, primary, min, frame);
    let (l, t, r, b) = outer((0, 0), size, frame, primary.scale);
    let (width, height) = (r - l, b - t);
    let work = primary.work;
    Some(Opening {
        size,
        position: Some((
            work.0 + (work.2 - work.0 - width).max(0) / 2,
            work.1 + (work.3 - work.1 - height).max(0) / 2,
        )),
        maximised,
    })
}

/// Where a window that opens as `open` is once maximised (Windows): on the monitor its
/// restored bounds overlap most, over that monitor's work area with the frame's sizing border
/// off screen on every side and the title bar inside, as Windows maximises a window. `insets`
/// gives the frame round the client area at a scale (left, top, right, bottom, physical
/// pixels: `platform::window::frame_insets`). `None` where no monitor is known.
pub fn maximised(
    open: &Opening,
    monitors: &[Monitor],
    frame: Frame,
    insets: impl Fn(f32) -> ScreenRect,
) -> Option<Maximised> {
    let at = open.position?;
    let m = monitors
        .iter()
        .max_by_key(|m| area(outer(at, open.size, frame, m.scale), m.rect))?;
    let (l, t, r, b) = insets(m.scale);
    let scale = f64::from(m.scale);
    let px = |logical: f64| to_px(logical, scale);
    let normal = (
        at.0,
        at.1,
        at.0 + px(open.size.0) + l + r,
        at.1 + px(open.size.1) + t + b,
    );
    // The sizing border is the frame's left side; the top has the title bar too.
    let (wl, wt, wr, wb) = m.work;
    let outer = (wl - l, wt - l, wr + l, wb + l);
    let inner = |from: i32, to: i32| f64::from(to - from) / scale;
    Some(Maximised {
        size: (
            inner(outer.0 + l, outer.2 - r),
            inner(outer.1 + t, outer.3 - b),
        ),
        position: (outer.0, outer.1),
        normal,
    })
}

/// The place to save for a window whose restored bounds are `size` (inner, physical pixels)
/// at `position` (its outer top-left corner, physical pixels, where known) on a monitor of
/// `scale`. Logical values keep two decimals, which come back to the same pixels.
pub fn place_of(
    position: Option<(i32, i32)>,
    size: (u32, u32),
    scale: f32,
    maximised: bool,
    monitors: &[Monitor],
) -> WindowPlace {
    let scale = f64::from(scale);
    let logical = |px: f64| (px / scale * 100.0).round() / 100.0;
    let monitor = position.and_then(|(x, y)| {
        let half = |v: u32| i32::try_from(v / 2).unwrap_or(i32::MAX);
        let centre = (
            x.saturating_add(half(size.0)),
            y.saturating_add(half(size.1)),
        );
        monitors
            .iter()
            .find(|m| holds(m.rect, centre))
            .map(|m| corners(m.rect))
    });
    WindowPlace {
        width: logical(f64::from(size.0)),
        height: logical(f64::from(size.1)),
        position: position.map(|(x, y)| [logical(f64::from(x)), logical(f64::from(y))]),
        scale,
        maximised,
        monitor,
    }
}

/// `logical` at `scale`, in whole physical pixels.
fn to_px(logical: f64, scale: f64) -> i32 {
    (logical * scale)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// The window's outer rectangle at `at` with an inner `size` (logical) and `frame` round it,
/// on a monitor of `scale`.
fn outer(at: (i32, i32), size: (f64, f64), frame: Frame, scale: f32) -> ScreenRect {
    let scale = f64::from(scale);
    let width = to_px(size.0 + frame.0 + frame.2, scale);
    let height = to_px(size.1 + frame.1 + frame.3, scale);
    (
        at.0,
        at.1,
        at.0.saturating_add(width),
        at.1.saturating_add(height),
    )
}

/// The inner size `wanted` cut to what fits in `m`'s work area with the frame, but never
/// below `min`.
fn fit(wanted: (f64, f64), m: &Monitor, min: (f64, f64), frame: Frame) -> (f64, f64) {
    let scale = f64::from(m.scale);
    let (l, t, r, b) = m.work;
    let room = (
        f64::from(r - l) / scale - frame.0 - frame.2,
        f64::from(b - t) / scale - frame.1 - frame.3,
    );
    (
        wanted.0.min(room.0).max(min.0),
        wanted.1.min(room.1).max(min.1),
    )
}

/// Whether a window at `outer` (physical pixels) on a monitor of `scale` can be reached on
/// the work area `work`: enough of its title bar shows there to drag it by.
fn visible(outer: ScreenRect, scale: f32, work: ScreenRect) -> bool {
    let scale = f64::from(scale);
    let band = (
        outer.0,
        outer.1,
        outer.2,
        outer.1.saturating_add(to_px(TITLE_BAR, scale)),
    );
    let (width, height) = overlap(band, work);
    let reach = (REACH * scale).min(f64::from(outer.2 - outer.0));
    f64::from(width) >= reach && f64::from(height) >= TITLE_BAR * scale / 2.0
}

/// The width and height of what `a` and `b` share (zero when they don't meet).
fn overlap(a: ScreenRect, b: ScreenRect) -> (i32, i32) {
    let width = a.2.min(b.2) - a.0.max(b.0);
    let height = a.3.min(b.3) - a.1.max(b.1);
    (width.max(0), height.max(0))
}

fn area(a: ScreenRect, b: ScreenRect) -> i64 {
    let (width, height) = overlap(a, b);
    i64::from(width) * i64::from(height)
}

fn holds((l, t, r, b): ScreenRect, (x, y): (i32, i32)) -> bool {
    (l..r).contains(&x) && (t..b).contains(&y)
}

fn corners((l, t, r, b): ScreenRect) -> [i32; 4] {
    [l, t, r, b]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows 11's frame at 100 %: invisible borders and a 31 px title bar.
    const FRAME: Frame = (8.0, 31.0, 8.0, 8.0);
    const MIN: (f64, f64) = (480.0, 600.0);

    fn monitor(rect: ScreenRect, scale: f32, primary: bool) -> Monitor {
        // A 48 px taskbar at the bottom.
        let work = (rect.0, rect.1, rect.2, rect.3 - 48);
        Monitor {
            rect,
            work,
            scale,
            primary,
        }
    }

    fn saved(width: f64, height: f64, position: [f64; 2], scale: f64) -> WindowPlace {
        WindowPlace {
            width,
            height,
            position: Some(position),
            scale,
            maximised: false,
            monitor: None,
        }
    }

    #[test]
    fn a_saved_place_opens_where_it_can() {
        let primary = monitor((0, 0, 1920, 1080), 1.0, true);
        let left = monitor((-2560, 0, 0, 1440), 1.5, false);
        let both = [primary, left];
        let at = |size: (f64, f64), position| Opening {
            size,
            position: Some(position),
            maximised: false,
        };
        let cases: [(&str, WindowPlace, &[Monitor], Opening); 7] = [
            (
                "on screen: as it was",
                saved(1280.0, 800.0, [100.0, 60.0], 1.0),
                &both,
                at((1280.0, 800.0), (100, 60)),
            ),
            (
                "on the other monitor, at its scale",
                saved(1000.0, 700.0, [-1600.0, 100.0], 1.5),
                &both,
                at((1000.0, 700.0), (-2400, 150)),
            ),
            (
                "off every screen: centred on the primary",
                saved(1280.0, 800.0, [4000.0, 60.0], 1.0),
                &both,
                at((1280.0, 800.0), (312, 96)),
            ),
            (
                "its monitor is gone: centred on the primary",
                saved(1000.0, 700.0, [-1600.0, 100.0], 1.5),
                &[primary],
                at((1000.0, 700.0), (452, 146)),
            ),
            (
                "only the title bar hidden above the screen: centred",
                saved(1280.0, 800.0, [100.0, -40.0], 1.0),
                &[primary],
                at((1280.0, 800.0), (312, 96)),
            ),
            (
                "bigger than the screen: cut to the work area, from its corner",
                saved(2400.0, 1400.0, [200.0, 10.0], 1.0),
                &[primary],
                at((1904.0, 993.0), (0, 0)),
            ),
            (
                "below the minimum: the minimum",
                saved(300.0, 200.0, [100.0, 60.0], 1.0),
                &[primary],
                at(MIN, (100, 60)),
            ),
        ];
        for (case, place, monitors, want) in cases {
            assert_eq!(opening(&place, monitors, MIN, FRAME), Some(want), "{case}");
        }
    }

    #[test]
    fn maximised_it_covers_the_work_area_of_its_monitor() {
        let main = monitor((0, 0, 3840, 2160), 1.5, true);
        let left = monitor((-1920, 0, 0, 1080), 1.0, false);
        // Windows 11's frame at 150 %.
        let insets = |scale: f32| {
            if scale == 1.5 {
                (11, 45, 11, 11)
            } else {
                (8, 31, 8, 8)
            }
        };
        let open = |position| Opening {
            size: (1000.0, 700.0),
            position: Some(position),
            maximised: true,
        };
        let on_main = Maximised {
            size: (2560.0, 2078.0 / 1.5),
            position: (-11, -11),
            normal: (300, 200, 1822, 1306),
        };
        let on_left = Maximised {
            size: (1920.0, 1009.0),
            position: (-1928, -8),
            normal: (-1500, 100, -484, 839),
        };
        let monitors = [main, left];
        assert_eq!(
            maximised(&open((300, 200)), &monitors, FRAME, insets),
            Some(on_main)
        );
        assert_eq!(
            maximised(&open((-1500, 100)), &monitors, FRAME, insets),
            Some(on_left)
        );
    }

    #[test]
    fn with_no_monitor_known_only_the_size_counts() {
        let place = WindowPlace {
            maximised: true,
            ..saved(1280.0, 800.0, [4000.0, 60.0], 1.0)
        };
        let want = Opening {
            size: (1280.0, 800.0),
            position: None,
            maximised: true,
        };
        assert_eq!(opening(&place, &[], MIN, FRAME), Some(want));
    }

    #[test]
    fn numbers_that_cant_be_used_open_as_if_nothing_was_saved() {
        let place = saved(f64::NAN, 800.0, [0.0, 0.0], 1.0);
        let primary = monitor((0, 0, 1920, 1080), 1.0, true);
        assert_eq!(opening(&place, &[primary], MIN, FRAME), None);
    }

    #[test]
    fn a_saved_place_comes_back_to_the_same_pixels() {
        let left = monitor((-2560, 0, 0, 1440), 1.5, false);
        let place = place_of(Some((-2399, 151)), (1201, 1001), 1.5, true, &[left]);
        assert_eq!(place.monitor, Some([-2560, 0, 0, 1440]));
        assert_eq!(place.position, Some([-1599.33, 100.67]));
        let opened = opening(&place, &[left], MIN, (0.0, 0.0, 0.0, 0.0)).expect("opens");
        assert_eq!(opened.position, Some((-2399, 151)));
        assert_eq!(
            (to_px(opened.size.0, 1.5), to_px(opened.size.1, 1.5)),
            (1201, 1001)
        );
        assert!(opened.maximised);
    }
}
