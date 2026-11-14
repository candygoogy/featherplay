//! Where the main window was left, for the next start: the `[main_window]` table of the
//! settings. A file without it (the first run, or one from before 0.4.1) has none, and the
//! window opens as it always did. A table this build can't read is dropped as a whole.

use serde::{Deserialize, Serialize};

/// The main window as it was left: its restored (normal) bounds, also while it's maximised,
/// and whether it was maximised. Never minimised: a minimised window isn't saved.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowPlace {
    /// The inner (client area) size, logical pixels.
    pub width: f64,
    pub height: f64,
    /// The outer top-left corner, logical pixels at `scale` (so `position × scale` is where it
    /// was on the screen, physical pixels); `None` where the desktop doesn't say (Wayland).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 2]>,
    /// The scale of the monitor it was on.
    pub scale: f64,
    #[serde(default)]
    pub maximised: bool,
    /// That monitor's rectangle (left, top, right, bottom), physical pixels, where known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<[i32; 4]>,
}

impl WindowPlace {
    /// Whether the numbers can be used: a size and a scale above zero, all of them finite.
    pub fn usable(&self) -> bool {
        let position = self.position.unwrap_or_default();
        [self.width, self.height, self.scale]
            .iter()
            .all(|v| v.is_finite() && *v > 0.0)
            && position.iter().all(|v| v.is_finite())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_finite_sizes_and_scales_above_zero_are_usable() {
        let place = WindowPlace {
            width: 1280.0,
            height: 800.0,
            position: Some([-1900.0, 40.0]),
            scale: 1.5,
            maximised: false,
            monitor: None,
        };
        assert!(place.usable());
        assert!(
            !WindowPlace {
                scale: 0.0,
                ..place
            }
            .usable()
        );
        assert!(
            !WindowPlace {
                width: f64::NAN,
                ..place
            }
            .usable()
        );
        assert!(
            !WindowPlace {
                position: Some([f64::INFINITY, 0.0]),
                ..place
            }
            .usable()
        );
    }
}
