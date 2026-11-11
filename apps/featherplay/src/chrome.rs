//! The window chrome the platform draws in the app's look: the Tri-vane icon for the title
//! bar and taskbar, which the build script makes, and the theme's colours for the title bar
//! and a popup's border, handed to `featherplay_platform::window` as plain RGB.

use featherplay_platform::window::{self, Handle, Rgb};
use slint::Color;

/// The Tri-vane logo, 16 to 256 px, from `build.rs`, drawn for dark grounds: the play sits
/// in a clearance ring of the ground's colour. The exe's embedded icon is this one, and
/// `identity` writes it out for the media card's registration.
pub(crate) const ICON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/featherplay.ico"));

/// The light variant, for a light taskbar: the ring round the play is a deep crimson, so
/// the white play doesn't read as a hole on white.
const ICON_LIGHT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/featherplay-light.ico"));

/// The icon for the taskbar's theme.
fn icon_for(light_taskbar: bool) -> &'static [u8] {
    if light_taskbar { ICON_LIGHT } else { ICON }
}

/// A theme colour as the platform takes it.
fn rgb(c: Color) -> Rgb {
    Rgb {
        red: c.red(),
        green: c.green(),
        blue: c.blue(),
    }
}

/// The title bar in the app's own colours: dark or light caption buttons, the window's
/// ground as the caption colour and the main text colour for the title, so it matches the
/// app even when Windows' app mode differs. No Mica: its wallpaper tint didn't match the
/// opaque client area below it. Caption and text colours need Windows 11; older versions
/// keep the plain dark or light title bar.
pub fn title_bar(h: Handle, dark: bool, caption: Color, text: Color) -> Result<(), String> {
    window::title_bar(h, dark, rgb(caption), rgb(text))
}

/// Sets the title bar (small) and taskbar (big) icons, sized for `scale`, in the drawing for
/// the taskbar's theme. Call it again when that theme changes (`window::when_colours_change`).
pub fn set_icon(h: Handle, scale: f32) -> Result<(), String> {
    let ico = icon_for(featherplay_platform::taskbar::light_taskbar());
    window::set_icon(h, scale, ico)
}

/// The border Windows 11 draws around a rounded popup, in the app's hairline colour rather
/// than the system's accent-or-grey one. Older versions have no such border.
pub fn popup_border(h: Handle, colour: Color) -> Result<(), String> {
    window::popup_border(h, rgb(colour))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_every_size() {
        for ico in [ICON, ICON_LIGHT] {
            for px in [16, 20, 24, 32, 48, 256] {
                let png = window::ico_entry(ico, px).expect("entry");
                assert_eq!(&png[1..4], b"PNG");
                // Width from the IHDR chunk.
                let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
                assert_eq!(w, px);
            }
            let w = |png: &[u8]| u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
            assert_eq!(w(window::ico_entry(ico, 22).unwrap()), 24);
            assert_eq!(w(window::ico_entry(ico, 512).unwrap()), 256);
        }
    }

    #[test]
    fn light_taskbar_gets_the_light_icon() {
        assert_ne!(ICON, ICON_LIGHT);
        assert_eq!(icon_for(true), ICON_LIGHT);
        assert_eq!(icon_for(false), ICON);
    }
}
