//! One loaded track. A decoder thread does all blocking work (network reads, demuxer seeks,
//! decoding) and fills a ring of samples; `RingSource` on the audio thread only copies
//! from the ring, so a slow network never stalls the audio callback.
//!
//! The ring has one writer (the decoder thread) and one reader (the audio thread) and no
//! lock between them: the audio thread runs in the real-time range
//! (`featherplay_platform::thread`), and a lock it shared with a normal thread would let
//! that thread, preempted while holding it, hold up the output as long as the scheduler
//! keeps it waiting. Indices count samples from the voice's start and never wrap; a slot is
//! `index % capacity`.
//!
//! A seek can't empty the ring from the decoder's side, since the reader owns the read
//! index. The decoder publishes where the new position's samples start instead (a seqlock:
//! `seek_seq` odd while it writes, even once done), and from then on treats everything
//! before that as free space. The reader jumps there on its next refill. Should the decoder
//! overwrite samples the reader is copying at that moment, the reader sees `seek_seq` move
//! once it's done and drops what it copied.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use featherplay_core::sync::LockExt;
use rodio::{ChannelCount, SampleRate};

use crate::dsp::BusControls;
use crate::glitches::Glitches;
use crate::media::Cancel;
use crate::silence::Silences;
use crate::stretch::{MAX_SPEED, MIN_SPEED};

mod decode;
mod ring;
mod source;

pub(crate) use decode::{Fetch, spawn};
use ring::Slots;
#[cfg(test)]
pub(crate) use ring::rings;

/// Samples the decoder thread decodes between ring updates.
const BATCH: usize = 4096;
/// Most samples the audio thread takes from the ring at once (~23 ms of 44.1 kHz stereo).
const TAKE: usize = 2048;
/// Frames of silence output per step while paused or starved.
const SILENCE_FRAMES: usize = 256;
/// How much the ring holds. The decoder fills it, and the audio thread wakes it once it's
/// half empty: a decoder held up for up to ~0.75 s (a busy machine) still doesn't starve
/// the output.
const RING: Duration = Duration::from_millis(1500);
/// `end` while the decoder hasn't reached the end of the stream.
const NO_END: u64 = u64::MAX;
/// `skip_at` while no skip waits for the reader.
const NO_SKIP: u64 = u64::MAX;
/// `music_end_ms` until the tail scan has found it.
const UNKNOWN: u64 = u64::MAX;
/// How long a cut takes: a pause, a stop or a skip ramps the sound out over this instead of
/// clicking, and a resume ramps it back in (`measurements/r74-crossfade-reference.md`).
pub(crate) const DECLICK: Duration = Duration::from_millis(30);
/// How long a voice takes to reach a new level when "Match song volumes" is switched.
const LEVEL_RAMP: Duration = Duration::from_millis(150);
/// The crossfade phase of a voice at full volume; its gain is `sin(phase)`.
const FULL: f64 = std::f64::consts::FRAC_PI_2;

/// Which way a crossfade ramp goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fade {
    /// Up to full volume.
    In,
    /// Down to silence; the source then ends and sends `Msg::FadedOut`.
    Out,
}

/// Play state shared by every voice of one engine, and the output's master bus.
pub(crate) struct Controls {
    pub paused: AtomicBool,
    /// Crossfade is on: each decoder thread finds where its track's music ends (`tail`).
    pub crossfade: AtomicBool,
    /// "Skip silence" (`SkipSilence::to_u8`), which decoder threads read once per batch.
    pub skip_silence: AtomicU8,
    /// "Match song volumes": each voice plays at its song's level (`VoiceShared::set_level`),
    /// which every source reads once per refill and ramps to over `LEVEL_RAMP`.
    pub match_volumes: AtomicBool,
    /// The playback speed (`f32` bits), which every source reads once per refill: at 1.0 it
    /// copies from the ring, otherwise it plays the ring through its `Stretch`.
    speed: AtomicU32,
    /// The volume, the equalizer and the compressor, which the output's bus applies to the
    /// mix (`dsp`).
    pub bus: BusControls,
    pub glitches: Arc<Glitches>,
    /// Sources in the mixer now. For tests.
    #[cfg(test)]
    pub sources: std::sync::atomic::AtomicUsize,
}

impl Controls {
    pub fn new() -> Self {
        Self {
            paused: AtomicBool::new(true),
            crossfade: AtomicBool::new(false),
            skip_silence: AtomicU8::new(0),
            match_volumes: AtomicBool::new(true),
            speed: AtomicU32::new(1.0_f32.to_bits()),
            bus: BusControls::default(),
            glitches: Arc::default(),
            #[cfg(test)]
            sources: Default::default(),
        }
    }

    /// Sets the playback speed, clamped to `MIN_SPEED..=MAX_SPEED`; NaN means 1.0.
    pub fn set_speed(&self, speed: f32) {
        let speed = if speed.is_nan() {
            1.0
        } else {
            speed.clamp(MIN_SPEED, MAX_SPEED)
        };
        self.speed.store(speed.to_bits(), Ordering::Relaxed);
    }

    pub fn speed(&self) -> f32 {
        f32::from_bits(self.speed.load(Ordering::Relaxed))
    }
}

/// When a voice's start-up steps were done, for the first-sound log line (`engine.rs`).
#[derive(Default)]
pub(crate) struct Stages {
    /// The stream's start is there to read, and whether it came without a request (a
    /// local file, a head in memory).
    pub opened: OnceLock<(Instant, bool)>,
    /// The decoder has read the header and its first packet.
    pub ready: OnceLock<Instant>,
    /// The first samples went into the ring.
    pub filled: OnceLock<Instant>,
}

pub(crate) struct VoiceShared {
    pub id: u64,
    pub url: String,
    pub stages: Stages,
    /// Set once with `format`, before the first source exists.
    slots: OnceLock<Slots>,
    /// Samples written so far. Only the decoder thread stores it.
    write: AtomicU64,
    /// The next sample the audio thread takes. Only the audio thread stores it.
    read: AtomicU64,
    /// `write` when the decoder reached the end of the stream; `NO_END` before.
    end: AtomicU64,
    /// The stream ended with an error; the source ends without `Ended`.
    failed: AtomicBool,
    /// The seqlock over `seek_start` and `seek_frame`; see the module docs.
    seek_seq: AtomicU64,
    /// The index the samples of the latest seek start at.
    seek_start: AtomicU64,
    /// The frame (from the start of the track) the latest seek went to.
    seek_frame: AtomicU64,
    /// The `seek_seq` the audio thread has applied.
    seek_seen: AtomicU64,
    /// Frame index of the sample at `read`. Only the audio thread stores it.
    front_frame: AtomicU64,
    /// Frame index of the chunk the audio thread is playing now; this is `position()`.
    played_frame: AtomicU64,
    /// Skipped silence (`silence`), one at a time: the decoder dropped `skip_frames` frames
    /// just before the sample at `skip_at`. The decoder sets both, `skip_at` last; the audio
    /// thread adds the frames to `front_frame` when it gets there and sets `NO_SKIP`. A
    /// seek clears it.
    skip_at: AtomicU64,
    skip_frames: AtomicU64,
    /// A crossfade ramp for the audio thread: `fade_out` and `fade_ms` (a whole sweep, from
    /// silence to full), then `fade_seq` moves on. The ramp starts from where the gain is.
    fade_seq: AtomicU32,
    fade_out: AtomicBool,
    fade_ms: AtomicU32,
    /// The crossfade phase (`f64` bits; the gain is `sin(phase)`). The audio thread keeps it
    /// up to date while a ramp runs, so a new source (another device) goes on from there.
    fade_phase: AtomicU64,
    /// Where the track's music ends, in ms (`tail`); `UNKNOWN` before the decoder thread
    /// has read the end of the stream.
    music_end_ms: AtomicU64,
    /// Skip silence's scan of the whole track (`scan`), once it's done.
    silences: OnceLock<Silences>,
    /// The app has the track's silences already (`AudioEngine::silences_known`): no scan.
    silences_known: AtomicBool,
    /// The engine has told the scan (`take_silences`).
    silences_told: AtomicBool,
    /// The first source ramps in (`ramp_in_first`).
    ramp_in: AtomicBool,
    /// The song's level while "Match song volumes" is on (`f32` bits, 0..=1; `loudness_gain`).
    level: AtomicU32,
    /// A seek for the decoder thread: target and the engine's sequence number. The engine
    /// and the decoder thread lock it; the audio thread never does.
    seek: Mutex<Option<(Duration, u64)>>,
    /// Wakes the decoder thread: a seek, a stop, or room in the ring.
    cv: Condvar,
    stopped: AtomicBool,
    /// Id of the one `RingSource` allowed to read; attaching a new one retires the old.
    attach: AtomicU64,
    format: OnceLock<(ChannelCount, SampleRate)>,
    started: AtomicBool,
    ended: AtomicBool,
    starved: AtomicBool,
    cancel: Mutex<Option<Cancel>>,
}

impl VoiceShared {
    fn new(id: u64, url: String) -> Self {
        Self {
            id,
            url,
            stages: Stages::default(),
            slots: OnceLock::new(),
            write: AtomicU64::new(0),
            read: AtomicU64::new(0),
            end: AtomicU64::new(NO_END),
            failed: AtomicBool::new(false),
            seek_seq: AtomicU64::new(0),
            seek_start: AtomicU64::new(0),
            seek_frame: AtomicU64::new(0),
            seek_seen: AtomicU64::new(0),
            front_frame: AtomicU64::new(0),
            played_frame: AtomicU64::new(0),
            skip_at: AtomicU64::new(NO_SKIP),
            skip_frames: AtomicU64::new(0),
            fade_seq: AtomicU32::new(0),
            fade_out: AtomicBool::new(false),
            fade_ms: AtomicU32::new(0),
            fade_phase: AtomicU64::new(FULL.to_bits()),
            music_end_ms: AtomicU64::new(UNKNOWN),
            silences: OnceLock::new(),
            silences_known: AtomicBool::new(false),
            silences_told: AtomicBool::new(false),
            ramp_in: AtomicBool::new(false),
            level: AtomicU32::new(1.0_f32.to_bits()),
            seek: Mutex::new(None),
            cv: Condvar::new(),
            stopped: AtomicBool::new(false),
            attach: AtomicU64::new(0),
            format: OnceLock::new(),
            started: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            starved: AtomicBool::new(false),
            cancel: Mutex::new(None),
        }
    }

    pub fn position(&self) -> Duration {
        let Some((_, rate)) = self.format.get() else {
            return Duration::ZERO;
        };
        // A seek the audio thread hasn't reached yet (it doesn't run while the output is
        // stopped) is where playback stands.
        let seq = self.seek_seq.load(Ordering::Acquire);
        let frame = if seq == self.seek_seen.load(Ordering::Acquire) {
            self.played_frame.load(Ordering::Relaxed)
        } else {
            self.seek_frame.load(Ordering::Relaxed)
        };
        Duration::from_secs_f64(frame as f64 / rate.get() as f64)
    }

    pub fn request_seek(&self, to: Duration, seq: u64) {
        *self.seek.lock_or_recover() = Some((to, seq));
        self.cv.notify_all();
    }

    /// Ends the voice: the source leaves the mixer, the download is cancelled (waking a
    /// decoder blocked on the network), and the decoder thread exits, deleting the temp file.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(cancel) = self.cancel.lock_or_recover().as_ref() {
            cancel();
        }
        self.cv.notify_all();
    }

    /// Ramps the voice in or out over `over` (equal power), from where its gain is now.
    pub fn fade(&self, way: Fade, over: Duration) {
        let ms = u32::try_from(over.as_millis()).unwrap_or(u32::MAX).max(1);
        self.fade_out.store(way == Fade::Out, Ordering::Relaxed);
        self.fade_ms.store(ms, Ordering::Relaxed);
        self.fade_seq.fetch_add(1, Ordering::Release);
    }

    /// The first sound ramps in (`DECLICK`): the voice starts mid-song, after a load at a
    /// position, so it would click in.
    pub fn ramp_in_first(&self) {
        self.ramp_in.store(true, Ordering::Relaxed);
    }

    /// A fading voice goes out within `over`: sooner if its ramp is longer, never later.
    pub fn fade_out_within(&self, over: Duration) {
        let ms = u32::try_from(over.as_millis()).unwrap_or(u32::MAX).max(1);
        if !self.fade_out.load(Ordering::Relaxed) || self.fade_ms.load(Ordering::Relaxed) > ms {
            self.fade(Fade::Out, over);
        }
    }

    /// The song's level while "Match song volumes" is on: an amplitude gain, clamped to
    /// 0..=1 (NaN is 1). Set before the first source, so it plays at that level from the start.
    pub fn set_level(&self, gain: f32) {
        let gain = if gain.is_nan() {
            1.0
        } else {
            gain.clamp(0.0, 1.0)
        };
        self.level.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// The voice starts silent, for a fade in. Before its first source only.
    pub fn start_silent(&self) {
        self.fade_phase.store(0.0_f64.to_bits(), Ordering::Relaxed);
    }

    /// Where the track's music ends, once the decoder thread has found it.
    pub fn music_end(&self) -> Option<Duration> {
        let ms = self.music_end_ms.load(Ordering::Relaxed);
        (ms != UNKNOWN).then(|| Duration::from_millis(ms))
    }

    /// The track's silent runs, the first time it's asked once the scan has found them.
    pub fn take_silences(&self) -> Option<&Silences> {
        let found = self.silences.get()?;
        (!self.silences_told.swap(true, Ordering::Relaxed)).then_some(found)
    }

    /// From the scan thread.
    pub fn found_silences(&self, found: Silences) {
        let _ = self.silences.set(found);
    }

    /// The app has this track's silences: the decoder thread doesn't scan for them.
    pub fn silences_known(&self) {
        self.silences_known.store(true, Ordering::Relaxed);
    }

    /// Set once the voice stops; the scan thread ends on it.
    pub fn stopped_flag(&self) -> &AtomicBool {
        &self.stopped
    }

    /// The audio source ran to the end and left the mixer (not cleared).
    pub fn has_ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }

    /// The audio source ran to the end and left the mixer; clears the flag.
    pub fn take_ended(&self) -> bool {
        self.ended.swap(false, Ordering::SeqCst)
    }

    pub fn is_starved(&self) -> bool {
        self.starved.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A voice at 44.1 kHz stereo, with no decoder thread: the tests play its part.
    pub(super) fn voice() -> Arc<VoiceShared> {
        let v = Arc::new(VoiceShared::new(1, "test".into()));
        v.set_format(
            ChannelCount::new(2).unwrap(),
            SampleRate::new(44_100).unwrap(),
        );
        v
    }

    pub(super) fn playing() -> Arc<Controls> {
        let c = Arc::new(Controls::new());
        c.paused.store(false, Ordering::SeqCst);
        c
    }
}
