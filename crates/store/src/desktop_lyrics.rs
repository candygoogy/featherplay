//! Desktop lyrics (Settings > Lyrics): the strip that shows the line being sung above other
//! apps. Stored as the `[desktop_lyrics]` table of the settings; missing keys take their
//! defaults, and a value this build doesn't know takes the default for that key only.

use serde::{Deserialize, Serialize};

/// The desktop lyrics' setting and how the strip was left.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopLyrics {
    /// The strip shows (off by default), while lyrics are on and the desktop allows it.
    pub on: bool,
    /// Clicks, the wheel and hover go through it to the app beneath (off at first, so it
    /// can be dragged into place).
    pub locked: bool,
    #[serde(deserialize_with = "lenient")]
    pub size: StripSize,
    /// Where it was dragged to; `None` puts it above the taskbar of the main window's screen.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub place: Option<StripPlace>,
    /// The first opening's line saying it can be dragged and locked has been shown.
    pub hinted: bool,
}

/// The strip's size, from its A− and A+.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StripSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl StripSize {
    /// Smallest first.
    pub const ALL: [StripSize; 3] = [StripSize::Small, StripSize::Medium, StripSize::Large];

    /// The next size up (`step` 1) or down (-1), if there is one.
    pub fn step(self, step: i32) -> Option<StripSize> {
        let at = Self::ALL.iter().position(|s| *s == self)? as i32 + step;
        usize::try_from(at)
            .ok()
            .and_then(|i| Self::ALL.get(i))
            .copied()
    }
}

/// Where the strip was left, in physical screen pixels: the middle of its bottom edge (it
/// grows upward and changes width about it), and the work area of the screen it was on
/// (left, top, right, bottom). Once that screen is gone or changed, it goes back to the
/// default place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StripPlace {
    pub x: i32,
    pub bottom: i32,
    pub screen: [i32; 4],
}

/// Reads a value, or takes the default when it isn't one this build knows.
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let value = toml::Value::deserialize(d)?;
    Ok(T::deserialize(value).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sizes_step_and_stop_at_the_ends() {
        assert_eq!(StripSize::Medium.step(1), Some(StripSize::Large));
        assert_eq!(StripSize::Medium.step(-1), Some(StripSize::Small));
        assert_eq!(StripSize::Large.step(1), None);
        assert_eq!(StripSize::Small.step(-1), None);
    }

    #[test]
    fn a_value_this_build_doesnt_know_takes_its_default_only() {
        let d: DesktopLyrics =
            toml::from_str("on = true\nsize = \"huge\"\nplace = \"left\"").expect("loads");
        assert_eq!(
            d,
            DesktopLyrics {
                on: true,
                ..DesktopLyrics::default()
            }
        );
    }

    #[test]
    fn the_place_comes_back_as_it_was_left() {
        let d = DesktopLyrics {
            on: true,
            locked: true,
            size: StripSize::Large,
            place: Some(StripPlace {
                x: 960,
                bottom: 990,
                screen: [0, 0, 1920, 1032],
            }),
            hinted: true,
        };
        let text = toml::to_string(&d).expect("saves");
        assert_eq!(toml::from_str::<DesktopLyrics>(&text).expect("loads"), d);
    }
}
