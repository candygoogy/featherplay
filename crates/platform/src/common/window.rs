//! What the window functions take and give on every OS: a plain colour, a screen
//! rectangle, the window in front, and the entry of an `.ico` for a size. The window
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
