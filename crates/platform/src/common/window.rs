//! What the window functions take and give on every OS: a plain colour, a screen
//! rectangle, the taskbar, the window in front, the monitors, a window's restored bounds, the
//! entry of an `.ico` for a size, and what a point of the window's own title bar is. The window
//! itself, `Handle`, is each OS's own.

use crate::window::Handle;

/// A colour as the window functions take it: red, green and blue, no alpha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// A screen rectangle in physical pixels: left, top, right, bottom.
pub type ScreenRect = (i32, i32, i32, i32);

/// The edge of its monitor a taskbar sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

/// The taskbar, as `taskbar` reads it: its edge and where it is (physical pixels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Taskbar {
    pub edge: Edge,
    pub rect: ScreenRect,
}

/// A monitor, as `monitors` lists them: its rectangle and its work area (the rectangle less
/// the taskbar or the panels), physical pixels, and the scale windows get there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub rect: ScreenRect,
    pub work: ScreenRect,
    pub scale: f32,
    /// The primary monitor, where a window with nowhere else to go opens.
    pub primary: bool,
}

/// A window's restored (normal) bounds as the system keeps them, also while it's maximised,
/// minimised or hidden: what `restored_bounds` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Restored {
    /// The outer top-left corner (its frame's, invisible borders included), physical pixels.
    pub position: (i32, i32),
    /// The inner (client area) size, physical pixels.
    pub size: (u32, u32),
    /// The scale of the monitor those bounds are on.
    pub scale: f32,
    /// It's maximised, or minimised from maximised (it comes back maximised).
    pub maximised: bool,
    /// It's minimised (hidden or not).
    pub minimised: bool,
}

/// The window in front: what `foreground` reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Foreground {
    pub window: Handle,
    /// Its window class, as `GetClassNameW` gives it (`Shell_TrayWnd`).
    pub class: String,
}

/// The PNG of `ico`'s smallest entry at least `px` wide (else its largest).
pub fn ico_entry(ico: &[u8], px: u32) -> Option<&[u8]> {
    let count = u16::from_le_bytes([*ico.get(4)?, *ico.get(5)?]) as usize;
    let mut best: Option<(u32, &[u8])> = None;
    for i in 0..count {
        let e = ico.get(6 + 16 * i..6 + 16 * (i + 1))?;
        let size = if e[0] == 0 { 256 } else { e[0] as u32 };
        let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let off = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        let data = ico.get(off..off + len)?;
        let better = match best {
            None => true,
            Some((b, _)) => (size >= px && (b < px || size < b)) || (b < px && size > b),
        };
        if better {
            best = Some((size, data));
        }
    }
    best.map(|(_, d)| d)
}

/// One of the window buttons the app draws in its own title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionButton {
    Minimise,
    Maximise,
    Close,
}

/// What a point in the window is, for the system, while the app draws the title bar
/// (`title_hit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleHit {
    /// The app's own: a control in the bar, or anything below it.
    Client,
    /// The bar's free space: drags the window, double-click maximises, right-click opens the
    /// window menu.
    Drag,
    /// The band along the top edge that still resizes the window.
    Top,
    Button(CaptionButton),
}

/// The app's title bar as the hit test reads it, in physical pixels from the client area's top
/// left: its height, where the three window buttons start (they run to the right edge, each
/// `button_w` wide) and the controls in it, each a span `(left, right)` across the bar's
/// height. Everything else in the bar drags the window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TitleBar {
    pub height: i32,
    pub buttons_x: i32,
    pub button_w: i32,
    pub controls: Vec<(i32, i32)>,
}

/// What the point `(x, y)` of the client area is, `edge` the height of the band along the top
/// that resizes (0 for a maximised window, which has nothing to resize).
pub fn title_hit(bar: &TitleBar, x: i32, y: i32, edge: i32) -> TitleHit {
    if y < 0 || y >= bar.height {
        return TitleHit::Client;
    }
    if y < edge {
        return TitleHit::Top;
    }
    if x >= bar.buttons_x && bar.button_w > 0 {
        return TitleHit::Button(match (x - bar.buttons_x) / bar.button_w {
            0 => CaptionButton::Minimise,
            1 => CaptionButton::Maximise,
            _ => CaptionButton::Close,
        });
    }
    if bar.controls.iter().any(|&(l, r)| x >= l && x < r) {
        TitleHit::Client
    } else {
        TitleHit::Drag
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1100 px window at 150 %: the bar 72 px, the buttons 69 px each from 893, Back and the
    /// search field in it.
    fn bar() -> TitleBar {
        TitleBar {
            height: 72,
            buttons_x: 1100 - 3 * 69,
            button_w: 69,
            controls: vec![(234, 288), (420, 840)],
        }
    }

    #[test]
    fn controls_are_the_apps_and_the_rest_of_the_bar_drags() {
        let b = bar();
        assert_eq!(title_hit(&b, 10, 30, 6), TitleHit::Drag);
        assert_eq!(title_hit(&b, 234, 30, 6), TitleHit::Client);
        assert_eq!(title_hit(&b, 287, 30, 6), TitleHit::Client);
        assert_eq!(title_hit(&b, 288, 30, 6), TitleHit::Drag);
        assert_eq!(title_hit(&b, 839, 71, 6), TitleHit::Client);
        assert_eq!(title_hit(&b, 840, 71, 6), TitleHit::Drag);
        // Below the bar, all of it is the app's.
        assert_eq!(title_hit(&b, 10, 72, 6), TitleHit::Client);
        assert_eq!(title_hit(&b, 1099, 500, 6), TitleHit::Client);
    }

    #[test]
    fn the_window_buttons_split_their_strip_and_close_takes_the_corner() {
        let b = bar();
        let at = |x| title_hit(&b, x, 30, 6);
        assert_eq!(at(893), TitleHit::Button(CaptionButton::Minimise));
        assert_eq!(at(961), TitleHit::Button(CaptionButton::Minimise));
        assert_eq!(at(962), TitleHit::Button(CaptionButton::Maximise));
        assert_eq!(at(1031), TitleHit::Button(CaptionButton::Close));
        assert_eq!(at(1099), TitleHit::Button(CaptionButton::Close));
        // Past the edge (a rounding pixel): still close.
        assert_eq!(at(1110), TitleHit::Button(CaptionButton::Close));
        assert_eq!(at(892), TitleHit::Drag);
    }

    #[test]
    fn the_top_band_resizes_over_everything_unless_maximised() {
        let b = bar();
        for x in [10, 250, 900, 1099] {
            assert_eq!(title_hit(&b, x, 0, 6), TitleHit::Top);
            assert_eq!(title_hit(&b, x, 5, 6), TitleHit::Top);
        }
        assert_eq!(
            title_hit(&b, 1099, 0, 0),
            TitleHit::Button(CaptionButton::Close)
        );
        assert_eq!(title_hit(&b, 10, 0, 0), TitleHit::Drag);
    }

    #[test]
    fn a_bar_without_buttons_is_controls_and_drag_only() {
        let b = TitleBar {
            buttons_x: 0,
            button_w: 0,
            ..bar()
        };
        assert_eq!(title_hit(&b, 1099, 30, 6), TitleHit::Drag);
        assert_eq!(title_hit(&b, 250, 30, 6), TitleHit::Client);
    }
}
