//! System Media Transport Controls (the Win+A media card and media keys) through
//! souvlaki, bound to the app window's HWND.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};

use crate::{NowPlaying, Sender, UiCommand};

pub struct Smtc {
    controls: MediaControls,
    /// What the card shows, so Play and Pause only toggle when they'd change something.
    playing: Arc<AtomicBool>,
    /// The metadata last sent; the thumbnail is only re-set when the track changes.
    shown: Option<NowPlaying>,
}

impl Smtc {
    /// Binds the media controls to `hwnd`. Call on the window's thread.
    pub fn attach(hwnd: isize, tx: Sender<UiCommand>) -> Result<Smtc, String> {
        let mut controls = MediaControls::new(PlatformConfig {
            display_name: "featherplay",
            dbus_name: "featherplay",
            hwnd: Some(hwnd as *mut c_void),
        })
        .map_err(|e| format!("media controls: {e:?}"))?;
        let playing = Arc::new(AtomicBool::new(false));
        let state = playing.clone();
        // Runs on a WinRT thread-pool thread.
        controls
            .attach(move |ev: MediaControlEvent| {
                let is_playing = state.load(Ordering::Relaxed);
                let cmd = match ev {
                    MediaControlEvent::Toggle => Some(UiCommand::PlayPause),
                    MediaControlEvent::Play => (!is_playing).then_some(UiCommand::PlayPause),
                    MediaControlEvent::Pause | MediaControlEvent::Stop => {
                        is_playing.then_some(UiCommand::PlayPause)
                    }
                    MediaControlEvent::Next => Some(UiCommand::Next),
                    MediaControlEvent::Previous => Some(UiCommand::Previous),
                    MediaControlEvent::Raise => Some(UiCommand::Show),
                    _ => None,
                };
                if let Some(cmd) = cmd {
                    let _ = tx.send(cmd);
                }
            })
            .map_err(|e| format!("media controls attach: {e:?}"))?;
        Ok(Smtc {
            controls,
            playing,
            shown: None,
        })
    }

    pub fn update(&mut self, np: &NowPlaying) {
        let same_track = self.shown.as_ref().is_some_and(|s| {
            s.title == np.title
                && s.artist == np.artist
                && s.album == np.album
                && s.art_url == np.art_url
                && s.duration == np.duration
        });
        if !same_track {
            let res = self.controls.set_metadata(MediaMetadata {
                title: Some(&np.title),
                artist: Some(&np.artist),
                album: np.album.as_deref(),
                cover_url: np.art_url.as_deref(),
                duration: np.duration,
            });
            if let Err(e) = res {
                log::warn!("smtc: set_metadata: {e:?}");
            }
        }
        let progress = Some(MediaPosition(np.position));
        let playback = if np.playing {
            MediaPlayback::Playing { progress }
        } else {
            MediaPlayback::Paused { progress }
        };
        if let Err(e) = self.controls.set_playback(playback) {
            log::warn!("smtc: set_playback: {e:?}");
        }
        self.playing.store(np.playing, Ordering::Relaxed);
        self.shown = Some(np.clone());
    }
}

impl Drop for Smtc {
    fn drop(&mut self) {
        let _ = self.controls.detach();
    }
}
