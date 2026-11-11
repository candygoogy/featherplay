//! System Media Transport Controls (the Win+A media card and media keys) through
//! souvlaki, bound to the app window's HWND. The playback rate, which souvlaki leaves out,
//! is set on the same controls directly.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use souvlaki::{MediaControlEvent, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};
use windows::Media::SystemMediaTransportControls;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;

pub use crate::common::media_controls::*;

pub struct MediaControls {
    controls: souvlaki::MediaControls,
    /// What the card shows, so Play and Pause only toggle when they'd change something.
    playing: Arc<AtomicBool>,
    /// The metadata last sent; the thumbnail is only re-set when the track changes.
    shown: Option<MediaState>,
    /// The window's controls again (the ones souvlaki drives), for the playback rate, and
    /// the rate they were given.
    rate: Option<SystemMediaTransportControls>,
    speed: f32,
}

impl MediaControls {
    /// Binds the media controls to `hwnd`. Call on the window's thread. `on_event` is called
    /// on a WinRT thread-pool thread.
    pub fn attach(
        hwnd: isize,
        on_event: impl Fn(MediaEvent) + Send + 'static,
    ) -> Result<MediaControls, String> {
        let mut controls = souvlaki::MediaControls::new(PlatformConfig {
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
                let event = match ev {
                    MediaControlEvent::Toggle => Some(MediaEvent::PlayPause),
                    MediaControlEvent::Play => (!is_playing).then_some(MediaEvent::PlayPause),
                    MediaControlEvent::Pause | MediaControlEvent::Stop => {
                        is_playing.then_some(MediaEvent::PlayPause)
                    }
                    MediaControlEvent::Next => Some(MediaEvent::Next),
                    MediaControlEvent::Previous => Some(MediaEvent::Previous),
                    MediaControlEvent::Raise => Some(MediaEvent::Show),
                    _ => None,
                };
                if let Some(event) = event {
                    on_event(event);
                }
            })
            .map_err(|e| format!("media controls attach: {e:?}"))?;
        Ok(MediaControls {
            controls,
            playing,
            shown: None,
            rate: window_controls(hwnd),
            speed: 1.0,
        })
    }

    /// Shows `np` on the media card.
    pub fn update(&mut self, np: MediaState) {
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
        if np.speed != self.speed
            && let Some(rate) = &self.rate
        {
            match rate.SetPlaybackRate(f64::from(np.speed)) {
                Ok(()) => self.speed = np.speed,
                Err(e) => log::warn!("smtc: set playback rate: {e:?}"),
            }
        }
        self.playing.store(np.playing, Ordering::Relaxed);
        self.shown = Some(np);
    }
}

/// The media controls of `hwnd`: the same object souvlaki was given.
fn window_controls(hwnd: isize) -> Option<SystemMediaTransportControls> {
    let interop = windows::core::factory::<
        SystemMediaTransportControls,
        ISystemMediaTransportControlsInterop,
    >()
    .ok()?;
    // SAFETY: `hwnd` is the live window souvlaki just attached to, on its thread.
    unsafe { interop.GetForWindow(HWND(hwnd as *mut c_void)) }.ok()
}

impl Drop for MediaControls {
    fn drop(&mut self) {
        let _ = self.controls.detach();
    }
}
