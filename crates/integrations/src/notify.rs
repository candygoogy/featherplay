//! Toasts: one when a track starts (only when `settings.notifications` is on; the caller
//! checks), and plain notices from the app (`notify`).

use tauri_winrt_notification::{Duration, Sound, Toast};

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
    // An unpackaged exe has no registered AppUserModelID of its own, so the toast borrows
    // PowerShell's, as tauri-winrt-notification suggests.
    let res = Toast::new(Toast::POWERSHELL_APP_ID)
        .title(title)
        .text1(text)
        .sound(None::<Sound>)
        .duration(Duration::Short)
        .show();
    if let Err(e) = res {
        log::warn!("notify: {e}");
    }
}
