//! The toast when a track starts (only when `settings.notifications` is on; the caller
//! checks). Plain notices from the app go to `featherplay_platform::notify::notify`.

use featherplay_platform::notify::notify;

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
