//! The engine thread: owns the output device and the voices, and turns their signals into
//! `AudioEvent`s. API calls send it messages; play, pause and volume are atomics the audio
//! thread reads directly, so they take effect without a round trip.
//!
//! A crossfade keeps the track being left playing, as a `fading` voice that ramps down
//! while the new current one ramps up (`voice::Fade`); the mixer sums them. It stops once
//! silent. A load, a seek or another device ends the fade at once.
//!
//! A running output wakes its device thread every few milliseconds even when it only plays
//! silence, so `STOP_AFTER` into a pause the engine stops the output stream; `play()` starts
//! it again. The device stays open, so resuming is as quick as from a short pause, unless
//! Windows has let the device go idle meanwhile: then `wake()` starts it ahead of a play.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use featherplay_core::sync::LockExt;

#[cfg(test)]
use crate::glitches::Counts;
use crate::voice::{Controls, Stages, VoiceShared};
use crate::{AudioEvent, Compressor, Playhead, SkipSilence, StreamSource, perceptual_gain};

mod thread;

use thread::Engine;

pub(crate) enum Msg {
    Load {
        src: StreamSource,
        start_at: Duration,
        seq: u64,
    },
    Preload(StreamSource),
    Crossfade {
        src: StreamSource,
        fade: Duration,
        seq: u64,
    },
    FadeOut(Duration),
    Prefetch(String),
    Connect(String),
    Seek {
        to: Duration,
        seq: u64,
    },
    SetDevice(Option<String>),
    /// Sent by `play()`: start the output, reopening it if a device loss left none.
    EnsureOutput,
    /// Sent by `pause()` and `load()`: stop the output stream after `STOP_AFTER`.
    Paused,
    /// Sent by `wake()`.
    Wake,
    Subscribe(Sender<AudioEvent>),
    Shutdown,
    // From decoder threads.
    Ready {
        voice: u64,
    },
    SeekDone {
        voice: u64,
        seq: u64,
        error: Option<String>,
    },
    VoiceError {
        voice: u64,
        msg: String,
    },
    /// From a head prefetch (`head::prefetch`): its URL was refused.
    HeadRefused(String),
    /// The app has this stream's silences: its voice doesn't scan for them.
    SilencesKnown(String),
    /// From a scan thread (`scan`): the voice's silences are there.
    Silences {
        voice: u64,
    },
    // From the audio thread.
    Started {
        voice: u64,
    },
    /// Playback reached silence the decoder dropped: the position jumped ahead.
    Skipped {
        voice: u64,
    },
    Ended {
        voice: u64,
    },
    /// A voice ramped down to silence (`voice::Fade::Out`) and left the mixer.
    FadedOut {
        voice: u64,
    },
    Starved {
        voice: u64,
        on: bool,
    },
    DeviceError {
        sink: u64,
        lost: bool,
    },
}

/// What `position()` reads without asking the engine thread.
#[derive(Default)]
struct Status {
    current: Option<Arc<VoiceShared>>,
    /// The target of a `load` or `seek` the engine hasn't finished, with its sequence number.
    pending: Option<(u64, Duration)>,
    /// The latest track's way to its first sound. For tests.
    #[cfg(test)]
    first_sound: Option<FirstSound>,
}

/// How a track's first sound came after its load, step by step; logged once per load.
/// It counts from the load, or from `play()` if that came later. Each step counts from the
/// end of the one before, or from that start if it was done before it (a preload; a
/// preloaded track loaded at a position counts its seek in `output`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FirstSound {
    pub total: Duration,
    /// Until the stream's start was there to read: its first response's headers, or
    /// without a request of its own (`at_hand`): a local file, a head in memory or the head
    /// on its way.
    pub open: Duration,
    pub at_hand: bool,
    /// The decoder reading the header and the first packet.
    pub decoder: Duration,
    /// Decoding (and a load at a position seeking) until the first samples are in the ring.
    pub fill: Duration,
    /// From there until the output took them.
    pub output: Duration,
}

impl FirstSound {
    /// The steps of a voice loaded at `loaded` whose first sound the output took at `now`.
    fn of(stages: &Stages, loaded: Instant, now: Instant) -> FirstSound {
        let mut last = loaded;
        let mut step = |at: Option<Instant>| {
            let at = at.unwrap_or(last).max(last);
            let d = at - last;
            last = at;
            d
        };
        let opened = stages.opened.get();
        FirstSound {
            total: now.saturating_duration_since(loaded),
            open: step(opened.map(|(t, _)| *t)),
            at_hand: opened.is_some_and(|(_, h)| *h),
            decoder: step(stages.ready.get().copied()),
            fill: step(stages.filled.get().copied()),
            output: step(Some(now)),
        }
    }
}

impl std::fmt::Display for FirstSound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ms = |d: Duration| d.as_millis();
        write!(
            f,
            "first sound {} ms after the load: stream {} ms ({}), decoder {} ms, \
             first samples {} ms, output {} ms",
            ms(self.total),
            ms(self.open),
            if self.at_hand {
                "no request"
            } else {
                "a request"
            },
            ms(self.decoder),
            ms(self.fill),
            ms(self.output),
        )
    }
}

/// Audio playback on a dedicated thread. See the crate docs for the event contract.
pub struct AudioEngine {
    tx: Sender<Msg>,
    controls: Arc<Controls>,
    status: Arc<Mutex<Status>>,
    seq: AtomicU64,
    thread: Option<JoinHandle<()>>,
}

impl AudioEngine {
    /// Opens the output device called `device` (from `output_devices()`); `None` or an unknown
    /// name opens the default device. HTTP streams are buffered in temp files under `temp_dir`
    /// (pass `{cache}/tmp`), created on first use. With no output device at all it starts
    /// without one, and `play()` opens one once it's there (`AudioEvent::NoDevice` until
    /// then); a device that doesn't open is tried again the same way, and `play()` reports
    /// its error. The engine starts paused at full volume.
    pub fn new(device: Option<&str>, temp_dir: impl Into<PathBuf>) -> Result<AudioEngine, String> {
        Self::start(device, temp_dir.into(), None, crate::http::READ_TIMEOUT)
    }

    /// An engine whose output is a null sink: no device opens, the mix is consumed in real
    /// time. For tests.
    #[cfg(test)]
    pub(crate) fn new_null(temp_dir: impl Into<PathBuf>) -> AudioEngine {
        Self::new_null_with_read_timeout(temp_dir, crate::http::READ_TIMEOUT)
    }

    /// `new_null` with another stream read timeout (tests use a short one).
    #[cfg(test)]
    pub(crate) fn new_null_with_read_timeout(
        temp_dir: impl Into<PathBuf>,
        read_timeout: Duration,
    ) -> AudioEngine {
        let plugged = Arc::new(AtomicBool::new(true));
        Self::start(None, temp_dir.into(), Some(plugged), read_timeout).expect("null engine")
    }

    /// An engine whose only device is a null sink that is there while `plugged` is set,
    /// and missing otherwise. For tests.
    #[cfg(test)]
    pub(crate) fn new_fake_device(
        temp_dir: impl Into<PathBuf>,
        plugged: Arc<AtomicBool>,
    ) -> Result<AudioEngine, String> {
        Self::start(
            None,
            temp_dir.into(),
            Some(plugged),
            crate::http::READ_TIMEOUT,
        )
    }

    fn start(
        device: Option<&str>,
        tmp_dir: PathBuf,
        null: Option<Arc<AtomicBool>>,
        read_timeout: Duration,
    ) -> Result<AudioEngine, String> {
        let (tx, rx) = mpsc::channel();
        let controls = Arc::new(Controls::new());
        let status = Arc::new(Mutex::new(Status::default()));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let device = device.map(str::to_owned);
        let thread = std::thread::Builder::new()
            .name("featherplay-audio".into())
            .spawn({
                let (tx, controls, status) = (tx.clone(), controls.clone(), status.clone());
                move || {
                    let mut engine =
                        match Engine::new(tx, controls, status, tmp_dir, null, read_timeout) {
                            Ok(e) => e,
                            Err(e) => {
                                let _ = ready_tx.send(Err(e));
                                return;
                            }
                        };
                    // Nothing to play on yet (the speakers are off), or a device that doesn't
                    // open now (a broken driver, the audio service stopped, a virtual
                    // machine's audio): start with no output, as after a lost device.
                    // `play()` opens one once it can and reports what's wrong until then, so
                    // the app still starts.
                    if engine.open_output(device.clone()).is_err() {
                        engine.device = device;
                    }
                    let _ = ready_tx.send(Ok(()));
                    engine.run(rx);
                }
            })
            .map_err(|e| format!("audio thread: {e}"))?;
        ready_rx
            .recv()
            .map_err(|_| "audio thread exited".to_string())??;
        Ok(AudioEngine {
            tx,
            controls,
            status,
            seq: AtomicU64::new(0),
            thread: Some(thread),
        })
    }

    /// Makes `src` the current track at `start_at` and pauses; call `play()` to hear it.
    /// The previous track stops. If `src.url` was preloaded, the buffered data is reused.
    pub fn load(&self, src: StreamSource, start_at: Duration) {
        self.controls.paused.store(true, Ordering::SeqCst);
        let seq = self.pending(start_at);
        self.send(Msg::Load { src, start_at, seq });
        self.send(Msg::Paused);
    }

    /// Starts downloading and decoding `src` in the background, so that the next `load()` of
    /// the same URL starts at once. Replaces an earlier preload.
    pub fn preload(&self, src: StreamSource) {
        self.send(Msg::Preload(src));
    }

    /// Fetches the first 256 KB of `src` into memory, so a later `load()` or `preload()`
    /// of the same URL starts without waiting for the network; one that comes while the
    /// fetch is on its way takes its request over. Cheap to call again: a URL
    /// already fetched or on its way is left alone. The 8 newest are kept. A refused URL
    /// is told (`AudioEvent::PreloadRefused`).
    pub fn prefetch(&self, src: &StreamSource) {
        if src.url.starts_with("https://") || src.url.starts_with("http://") {
            self.send(Msg::Prefetch(src.url.clone()));
        }
    }

    /// Opens a connection to the server of `url` (`https://host` is enough), ahead of a
    /// stream that will be loaded in a moment and likely comes from there: its first request
    /// finds the connection open, and its answer comes ~100 ms sooner. Nothing is fetched.
    /// With a connection to that server waiting already none is opened, and a call while
    /// another is on its way does nothing. Left unused, the connection closes like any
    /// other, 90 s later.
    pub fn connect(&self, url: &str) {
        if url.starts_with("https://") || url.starts_with("http://") {
            self.send(Msg::Connect(url.to_owned()));
        }
    }

    /// Makes `src` the current track from its start without pausing, and plays: the track
    /// playing now fades out over `fade` while `src` fades in (equal power), and stops once
    /// silent. A preloaded `src` sounds at once; another fades in once it has loaded. A
    /// `load`, a `seek` or another device ends the fade, the old track at once.
    pub fn crossfade_to(&self, src: StreamSource, fade: Duration) {
        let seq = self.pending(Duration::ZERO);
        self.send(Msg::Crossfade { src, fade, seq });
        self.play();
    }

    /// The current track fades out over `fade`, then stops; nothing is current until the
    /// next `load` or `crossfade_to`.
    pub fn fade_out(&self, fade: Duration) {
        self.send(Msg::FadeOut(fade));
    }

    /// A play may come in a moment (the pointer rests on a song): while paused, an output
    /// stopped `WAKE_AFTER` or more starts now, playing silence, so the play doesn't wait
    /// ~85 ms for the device to wake. It stops again `STOP_AFTER` later unless something
    /// plays. Paused or stopped a short while, it does nothing.
    pub fn wake(&self) {
        self.send(Msg::Wake);
    }

    pub fn play(&self) {
        self.controls.paused.store(false, Ordering::SeqCst);
        self.send(Msg::EnsureOutput);
    }

    pub fn pause(&self) {
        self.controls.paused.store(true, Ordering::SeqCst);
        self.send(Msg::Paused);
    }

    /// Seeks the current track. `position()` reports `to` at once. A seek past the downloaded
    /// data fetches the chunk at the target; meanwhile the output is silent and, if it takes
    /// a while, `Buffering(true)` and then `Buffering(false)` are sent.
    pub fn seek(&self, to: Duration) {
        let seq = self.pending(to);
        self.send(Msg::Seek { to, seq });
    }

    /// `slider` is the 0..=1 volume control; see `perceptual_gain`.
    pub fn set_volume(&self, slider: f32) {
        self.controls.bus.set_gain(perceptual_gain(slider));
    }

    /// The equalizer's five bands in dB (60 Hz, 250 Hz, 1 kHz, 4 kHz, 12 kHz), each within
    /// ±12; `None` turns it off. Boosts lower the overall level by the largest one, so
    /// nothing clips. Heard within one device buffer (~50 ms).
    pub fn set_equalizer(&self, bands: Option<[f32; 5]>) {
        self.controls.bus.set_equalizer(bands);
    }

    /// "Skip silence": from the next silence the decoder reaches, about 1.5 s ahead of
    /// what plays. A skip that's heard sends `AudioEvent::Skipped`.
    pub fn set_skip_silences(&self, mode: SkipSilence) {
        self.controls
            .skip_silence
            .store(mode.to_u8(), Ordering::Relaxed);
    }

    /// The app keeps the silences of the stream at `url` (from an earlier
    /// `AudioEvent::Silences`): a `load`, `preload` or `crossfade_to` of it after this
    /// doesn't scan it again. Remembered for the few URLs told last.
    pub fn silences_known(&self, url: &str) {
        self.send(Msg::SilencesKnown(url.to_owned()));
    }

    /// "Even out loudness". Heard within one device buffer (~50 ms).
    pub fn set_compressor(&self, c: Compressor) {
        self.controls.bus.set_compressor(c);
    }

    /// The playback speed, with the pitch kept: 0.5 to 2.0 (clamped; NaN means 1.0). Heard
    /// within ~30 ms, on every voice, a crossfade's two included. `position()` stays in the
    /// track's time.
    pub fn set_speed(&self, speed: f32) {
        self.controls.set_speed(speed);
    }

    /// Moves playback to the named device (`None` or unknown: the default device), keeping
    /// the current track and position.
    pub fn set_device(&self, name: Option<&str>) {
        self.send(Msg::SetDevice(name.map(str::to_owned)));
    }

    /// Crossfade on: each track's decoder thread also reads the last 20 s of its stream
    /// once, to find where the music ends (`Playhead::music_end`).
    pub fn set_crossfade(&self, on: bool) {
        self.controls.crossfade.store(on, Ordering::Relaxed);
    }

    /// "Match song volumes": each song louder than YouTube's reference level plays turned
    /// down to it (`StreamSource::loudness_db`), quieter ones as they are; off, all play
    /// unchanged. The playing songs move to their new level over ~150 ms.
    pub fn set_match_volumes(&self, on: bool) {
        self.controls.match_volumes.store(on, Ordering::Relaxed);
    }

    /// The position in the current track, and where its music ends if known; read
    /// together, so both are the same track's.
    pub fn playhead(&self) -> Playhead {
        let s = self.status.lock_or_recover();
        match (&s.pending, &s.current) {
            (Some((_, to)), _) => Playhead {
                position: *to,
                music_end: None,
            },
            (None, Some(v)) => Playhead {
                position: v.position(),
                music_end: v.music_end(),
            },
            (None, None) => Playhead::default(),
        }
    }

    /// Position in the current track; zero when nothing is loaded.
    pub fn position(&self) -> Duration {
        self.playhead().position
    }

    /// How the latest track's first sound came. For tests.
    #[cfg(test)]
    pub(crate) fn first_sound(&self) -> Option<FirstSound> {
        self.status.lock_or_recover().first_sound
    }

    /// The glitch counters so far. For tests.
    #[cfg(test)]
    pub(crate) fn glitches(&self) -> Counts {
        self.controls.glitches.snapshot()
    }

    /// How many times the output started. For tests.
    #[cfg(test)]
    pub(crate) fn output_starts(&self) -> u32 {
        self.controls.glitches.starts()
    }

    /// Sources in the mixer now. For tests.
    #[cfg(test)]
    pub(crate) fn sources(&self) -> usize {
        self.controls.sources.load(Ordering::SeqCst)
    }

    /// Starts the longest-gap counter over. For tests.
    #[cfg(test)]
    pub(crate) fn reset_max_gap(&self) {
        self.controls.glitches.reset_max_gap();
    }

    /// A new receiver for the events from now on. Each call gets its own copy of every event.
    pub fn events(&self) -> Receiver<AudioEvent> {
        let (tx, rx) = mpsc::channel();
        self.send(Msg::Subscribe(tx));
        rx
    }

    fn pending(&self, to: Duration) -> u64 {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.status.lock_or_recover().pending = Some((seq, to));
        seq
    }

    fn send(&self, m: Msg) {
        // Fails only if the engine thread is gone, and then there is nothing to tell.
        let _ = self.tx.send(m);
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.send(Msg::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// Stages done `opened`, `ready` and `filled` after `t0`.
    fn stages(t0: Instant, opened: u32, ready: u32, filled: u32) -> Stages {
        let s = Stages::default();
        let _ = s.opened.set((t0 + MS * opened, false));
        let _ = s.ready.set(t0 + MS * ready);
        let _ = s.filled.set(t0 + MS * filled);
        s
    }

    #[test]
    fn a_first_sound_splits_into_the_steps_after_the_load() {
        let t0 = Instant::now();
        let steps = FirstSound::of(&stages(t0, 150, 160, 162), t0, t0 + MS * 170);
        let want = FirstSound {
            total: MS * 170,
            open: MS * 150,
            at_hand: false,
            decoder: MS * 10,
            fill: MS * 2,
            output: MS * 8,
        };
        assert_eq!(steps, want);
    }

    #[test]
    fn a_preloaded_track_counts_only_what_came_after_its_load() {
        let t0 = Instant::now();
        let loaded = t0 + MS * 500;
        let steps = FirstSound::of(&stages(t0, 150, 160, 162), loaded, loaded + MS * 4);
        assert_eq!(
            (
                steps.total,
                steps.open,
                steps.decoder,
                steps.fill,
                steps.output
            ),
            (
                MS * 4,
                Duration::ZERO,
                Duration::ZERO,
                Duration::ZERO,
                MS * 4
            )
        );
    }
}
