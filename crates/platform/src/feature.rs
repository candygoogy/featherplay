//! The features an OS may lack, and which ones this build has. Every OS's answer is in
//! [`supports`], so its gaps are listed in one place; the UI can hide a setting whose feature
//! is missing. A function for a missing feature returns [`Unsupported`] rather than doing
//! nothing.

use std::fmt;

/// A feature that some OS doesn't have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    /// Previous, play/pause and next on the taskbar button's thumbnail.
    TaskbarButtons,
    /// The tray icon and its menu.
    Tray,
    /// The mini player placed by the tray icon.
    MiniPlayerPlacement,
    /// Hotkeys that work while another app has the focus.
    GlobalHotkeys,
    /// The "now playing" notification.
    Notifications,
    /// The system's media controls (the media card, the media keys).
    MediaControls,
    /// Signing in inside a window of the app's own (WebView2), not only in a browser.
    WebViewSignIn,
    /// An update that installs over the running copy.
    InPlaceUpdate,
    /// The title bar recoloured to the theme.
    TitleBarColour,
    /// Secrets kept in the system's credential store.
    SecretStorage,
}

impl Feature {
    /// Every feature, in declaration order.
    pub const ALL: [Feature; 10] = [
        Feature::TaskbarButtons,
        Feature::Tray,
        Feature::MiniPlayerPlacement,
        Feature::GlobalHotkeys,
        Feature::Notifications,
        Feature::MediaControls,
        Feature::WebViewSignIn,
        Feature::InPlaceUpdate,
        Feature::TitleBarColour,
        Feature::SecretStorage,
    ];
}

/// Whether this build's OS has `feature`. Windows has them all.
#[cfg(windows)]
pub fn supports(feature: Feature) -> bool {
    match feature {
        Feature::TaskbarButtons
        | Feature::Tray
        | Feature::MiniPlayerPlacement
        | Feature::GlobalHotkeys
        | Feature::Notifications
        | Feature::MediaControls
        | Feature::WebViewSignIn
        | Feature::InPlaceUpdate
        | Feature::TitleBarColour
        | Feature::SecretStorage => true,
    }
}

/// Whether this build's OS has `feature`. Linux is stubs until phase 5, so none yet.
#[cfg(target_os = "linux")]
pub fn supports(feature: Feature) -> bool {
    match feature {
        Feature::TaskbarButtons
        | Feature::Tray
        | Feature::MiniPlayerPlacement
        | Feature::GlobalHotkeys
        | Feature::Notifications
        | Feature::MediaControls
        | Feature::WebViewSignIn
        | Feature::InPlaceUpdate
        | Feature::TitleBarColour
        | Feature::SecretStorage => false,
    }
}

/// A call for a feature this OS doesn't have (see [`supports`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsupported(pub Feature);

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} isn't supported on this system", self.0)
    }
}

impl std::error::Error for Unsupported {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_platform_answers_for_every_feature() {
        for feature in Feature::ALL {
            assert_eq!(supports(feature), cfg!(windows), "{feature:?}");
        }
        assert_eq!(
            Unsupported(Feature::Tray).to_string(),
            "Tray isn't supported on this system"
        );
    }
}
