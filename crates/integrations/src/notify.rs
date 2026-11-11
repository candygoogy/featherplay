//! Toasts: one when a track starts (only when `settings.notifications` is on; the caller
//! checks), and plain notices from the app (`notify`).

use tauri_winrt_notification::{Duration, Sound, Toast};

use featherplay_platform::identity;

use crate::NowPlaying;

/// Shows the title over "artist, album". Blocks for a few milliseconds of WinRT calls, so call
/// it off the UI thread. Errors are logged, not returned: a missing toast is not worth
/// interrupting playback for.
pub fn notify_track(np: &NowPlaying) {
    let detail = match &np.album {
        Some(album) if !album.is_empty() => format!("{}, {album}", np.artist),
        _ => np.artist.clone(),
    };
    notify(&np.title, &detail);
}

/// Shows a silent, short toast with a title and one line of text. Like `notify_track`, it
/// blocks briefly, so call it off the UI thread.
pub fn notify(title: &str, text: &str) {
    let res = Toast::new(app_id(identity::registered()))
        .title(title)
        .text1(text)
        .sound(None::<Sound>)
        .duration(Duration::Short)
        .show();
    if let Err(e) = res {
        log::warn!("notify: {e}");
    }
}

/// The AppUserModelID a toast goes out under. Windows drops toasts from an AUMID it has no
/// registration for, so featherplay's own is used only once this run has registered it
/// (`identity::registered`), which gives the toast featherplay's name and icon. Until then
/// (dev runs, `FEATHERPLAY_HOME`, a failed registration, or a toast that beats the
/// registration thread at startup) it borrows PowerShell's, as tauri-winrt-notification
/// suggests for an unpackaged exe.
fn app_id(registered: bool) -> &'static str {
    if registered {
        identity::AUMID
    } else {
        Toast::POWERSHELL_APP_ID
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_app_toasts_as_itself() {
        assert_eq!(app_id(true), identity::AUMID);
    }

    #[test]
    fn unregistered_app_borrows_powershells_id() {
        assert_eq!(app_id(false), Toast::POWERSHELL_APP_ID);
    }

    #[test]
    fn a_run_starts_unregistered() {
        // Nothing in this test process registers, so its toasts keep the fallback.
        assert!(!identity::registered());
    }
}
