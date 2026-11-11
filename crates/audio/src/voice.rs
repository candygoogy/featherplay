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

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering, fence};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use featherplay_core::sync::{CondvarExt, LockExt};
use rodio::{ChannelCount, Sample, SampleRate, Source};
use tokio::runtime::Runtime;

use crate::StreamSource;
use crate::decoder::Decoder;
use crate::dsp::BusControls;
use crate::engine::Msg;
use crate::glitches::Glitches;
use crate::head::HeadCache;
use crate::http::ErrorSlot;
use crate::media::{Cancel, Media, Opened};
use crate::silence::{Silences, Skip, SkipSilence, Trimmer};
use crate::stretch::{MAX_SPEED, MIN_SPEED, Stretch};

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

/// Rings allocated and not freed yet, in this process. For tests.
#[cfg(test)]
static RINGS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many voices hold a ring now. For tests.
#[cfg(test)]
pub(crate) fn rings() -> usize {
    RINGS.load(Ordering::SeqCst)
}

/// The ring's slots: `f32` bits in atomics, so neither side needs `unsafe` to share them.
struct Slots(Box<[AtomicU32]>);

impl Slots {
    fn new(len: usize) -> Self {
        Self((0..len.max(1)).map(|_| AtomicU32::new(0)).collect())
    }
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    /// The slot of sample `index`.
    fn at(&self, index: u64) -> &AtomicU32 {
        // In range: `index % len` < len.
        &self.0[(index % self.len()) as usize]
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

    /// The level a source plays at now: the song's own while "Match song volumes" is on.
    fn level_for(&self, controls: &Controls) -> f32 {
        if controls.match_volumes.load(Ordering::Relaxed) {
            f32::from_bits(self.level.load(Ordering::Relaxed))
        } else {
            1.0
        }
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

    /// A new source for the mixer that replaces any earlier one. `None` until the format is known.
    pub fn source(
        self: &Arc<Self>,
        controls: Arc<Controls>,
        tx: Sender<Msg>,
    ) -> Option<RingSource> {
        let (channels, rate) = *self.format.get()?;
        let attach = self.attach.fetch_add(1, Ordering::SeqCst) + 1;
        #[cfg(test)]
        controls.sources.fetch_add(1, Ordering::SeqCst);
        let ch = channels.get() as usize;
        let declick = (DECLICK.as_secs_f64() * f64::from(rate.get()))
            .round()
            .max(1.0) as u64;
        // A source for a voice already heard (another device, a seek after the end) comes
        // in mid-song, as does one loaded at a position: it ramps in.
        let mid_song = self.started.load(Ordering::Relaxed) || self.ramp_in.load(Ordering::Relaxed);
        let level = self.level_for(&controls);
        let level_frames = (LEVEL_RAMP.as_secs_f64() * f64::from(rate.get())).max(1.0) as f32;
        Some(RingSource {
            voice: self.clone(),
            controls,
            tx,
            attach,
            channels,
            rate,
            // Room for any refill, so the audio thread never allocates.
            local: Vec::with_capacity(TAKE.max(SILENCE_FRAMES * ch).max(declick as usize * ch)),
            idx: 0,
            stretch: Stretch::new(channels.get(), rate.get()),
            input: Vec::with_capacity(TAKE),
            phase: f64::from_bits(self.fade_phase.load(Ordering::Relaxed)),
            phase_step: 0.0,
            fade_seen: 0,
            click: if mid_song { 0.0 } else { 1.0 },
            click_step: 0.0,
            declick,
            level,
            level_to: level,
            level_step: 0.0,
            level_frames,
            live: false,
            ending: false,
            faded_out: false,
        })
    }

    // The decoder thread's side.

    /// Sets the format and makes the ring, ~`RING` of it.
    fn set_format(&self, channels: ChannelCount, rate: SampleRate) {
        let frames = (RING.as_secs_f64() * rate.get() as f64) as usize;
        let _ = self.slots.set(Slots::new(frames * channels.get() as usize));
        #[cfg(test)]
        RINGS.fetch_add(1, Ordering::SeqCst);
        let _ = self.format.set((channels, rate));
    }

    /// Samples the ring has room for, counting those before the latest seek as free.
    fn room(&self, slots: &Slots) -> u64 {
        let read = self
            .read
            .load(Ordering::Acquire)
            .max(self.seek_start.load(Ordering::Relaxed));
        slots.len() - (self.write.load(Ordering::Relaxed) - read)
    }

    /// Appends `samples`; the caller checked `room`.
    fn push(&self, slots: &Slots, samples: &[Sample]) {
        let w = self.write.load(Ordering::Relaxed);
        for (i, s) in samples.iter().enumerate() {
            slots.at(w + i as u64).store(s.to_bits(), Ordering::Relaxed);
        }
        self.write
            .store(w + samples.len() as u64, Ordering::Release);
    }

    /// The reader has taken the last skip, so the decoder may drop again.
    fn can_skip(&self) -> bool {
        self.skip_at.load(Ordering::Acquire) == NO_SKIP
    }

    /// `frames` were dropped just before the next sample pushed; the caller checked
    /// `can_skip`.
    fn mark_skip(&self, frames: u64) {
        self.skip_frames.store(frames, Ordering::Relaxed);
        self.skip_at
            .store(self.write.load(Ordering::Relaxed), Ordering::Release);
    }

    /// The stream ends after the samples written so far.
    fn mark_end(&self, failed: bool) {
        self.failed.store(failed, Ordering::Relaxed);
        self.end
            .store(self.write.load(Ordering::Relaxed), Ordering::Release);
    }

    /// The decoder moved to `frame`: what's in the ring is from before it.
    fn publish_seek(&self, frame: u64) {
        let seq = self.seek_seq.load(Ordering::Relaxed);
        self.seek_seq.store(seq + 1, Ordering::Relaxed);
        fence(Ordering::Release);
        self.seek_start
            .store(self.write.load(Ordering::Relaxed), Ordering::Relaxed);
        self.seek_frame.store(frame, Ordering::Relaxed);
        self.skip_at.store(NO_SKIP, Ordering::Relaxed);
        self.end.store(NO_END, Ordering::Relaxed);
        self.failed.store(false, Ordering::Relaxed);
        // SeqCst: `RingSource::finish` relies on its order against `ended`.
        self.seek_seq.store(seq + 2, Ordering::SeqCst);
    }
}

#[cfg(test)]
impl Drop for VoiceShared {
    fn drop(&mut self) {
        if self.slots.get().is_some() {
            RINGS.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// What a decoder thread opens its stream with.
pub(crate) struct Fetch {
    pub rt: Arc<Runtime>,
    pub client: reqwest::Client,
    pub tmp_dir: PathBuf,
    pub heads: HeadCache,
}

/// Starts a voice: its decoder thread opens `src`, then sends `Msg::Ready` and decodes ahead
/// until the ring is full. The thread holds `fetch.rt` so a cancelled download can still
/// wind down after the engine is gone. `silences_known`: the app has the track's silences,
/// so it isn't scanned for them.
pub(crate) fn spawn(
    id: u64,
    src: StreamSource,
    fetch: Fetch,
    controls: Arc<Controls>,
    tx: Sender<Msg>,
    silences_known: bool,
) -> Arc<VoiceShared> {
    let shared = Arc::new(VoiceShared::new(id, src.url.clone()));
    if silences_known {
        shared.silences_known();
    }
    shared.set_level(crate::loudness_gain(src.loudness_db));
    let v = shared.clone();
    let err_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("featherplay-decode-{id}"))
        .spawn(move || {
            featherplay_platform::thread::raise_priority();
            if let Err(e) = decode(&v, &src, &fetch, &controls, &tx) {
                v.failed.store(true, Ordering::Relaxed);
                let _ = tx.send(Msg::VoiceError {
                    voice: v.id,
                    msg: e,
                });
            }
        });
    if let Err(e) = spawned {
        shared.failed.store(true, Ordering::Relaxed);
        let _ = err_tx.send(Msg::VoiceError {
            voice: id,
            msg: format!("decoder thread: {e}"),
        });
    }
    shared
}

fn decode(
    v: &Arc<VoiceShared>,
    src: &StreamSource,
    fetch: &Fetch,
    controls: &Controls,
    tx: &Sender<Msg>,
) -> Result<(), String> {
    let error = ErrorSlot::default();
    let Fetch {
        rt,
        client,
        tmp_dir,
        heads,
    } = fetch;
    let Opened {
        media,
        len,
        cancel,
        at_hand,
    } = Media::open(src, rt.handle(), client, tmp_dir, heads, error.clone())?;
    let _ = v.stages.opened.set((Instant::now(), at_hand));
    // For the tail scan, while crossfade is on; done once the stream is known not to have
    // one (no length) or after one try with the whole file there.
    let viewer = media.viewer();
    let mut scanned = src.duration.is_none();
    // Skip silence's whole-track scan (`scan`): started once, while the mode is on.
    let mut silences_started = false;
    *v.cancel.lock_or_recover() = cancel;
    if v.stopped.load(Ordering::SeqCst) {
        return Ok(());
    }
    let take_error = || error.lock_or_recover().take();
    let mut dec = Decoder::open(media, len, &src.mime).map_err(|e| take_error().unwrap_or(e))?;
    let (channels, rate) = (dec.channels(), dec.sample_rate());
    v.set_format(channels, rate);
    let Some(slots) = v.slots.get() else {
        return Err("decoder: no ring".into());
    };
    let _ = v.stages.ready.set(Instant::now());
    let _ = tx.send(Msg::Ready { voice: v.id });

    let mut batch: Vec<Sample> = Vec::with_capacity(BATCH);
    // What goes into the ring next (the batch after skip silence), and how much of it has.
    let mut out: Vec<Sample> = Vec::with_capacity(BATCH);
    let mut sent = 0;
    // Silence dropped just before `out[skip.at]`.
    let mut skip: Option<Skip> = None;
    let mut trimmer = Trimmer::new(channels, rate);
    let ch = channels.get() as usize;
    // The decoder reached the end of the stream (until the next seek).
    let mut at_end = false;
    loop {
        // About to wait with the ring full (or the stream read): time for the tail scan,
        // which never waits on the network. Seeks wait meanwhile (~0.1 s).
        let idle = v.room(slots) < BATCH as u64 || (at_end && sent == out.len());
        if !scanned
            && idle
            && controls.crossfade.load(Ordering::Relaxed)
            && let Some(tail_media) = viewer.open(len)
        {
            scanned = true;
            let length = src.duration.unwrap_or_default();
            if let Some(end) =
                crate::tail::music_end(tail_media, len, &src.mime, length, &v.stopped)
            {
                let ms = u64::try_from(end.as_millis()).unwrap_or(UNKNOWN - 1);
                v.music_end_ms.store(ms, Ordering::Relaxed);
            }
        }
        // Then skip silence's scan, on a thread of its own.
        if !silences_started
            && idle
            && controls.skip_silence.load(Ordering::Relaxed) != SkipSilence::Off.to_u8()
            && !v.silences_known.load(Ordering::Relaxed)
            && let Some(scan_media) = viewer.open(len)
        {
            silences_started = true;
            crate::scan::spawn(v.clone(), scan_media, len, src.mime.clone(), tx.clone());
        }
        // Wait for a seek, room in the ring, or stop.
        let seek = {
            let mut g = v.seek.lock_or_recover();
            loop {
                if v.stopped.load(Ordering::SeqCst) {
                    return Ok(());
                }
                if let Some(s) = g.take() {
                    break Some(s);
                }
                if (sent < out.len() || !at_end) && v.room(slots) >= BATCH as u64 {
                    break None;
                }
                g = v.cv.wait_or_recover(g);
            }
        };
        if let Some((to, seq)) = seek {
            // May block on the network for a cold seek; the ring keeps playing what it has
            // until the audio thread reaches the new position, then the source outputs
            // silence (starved) until the samples arrive.
            let error =
                match dec.try_seek(to) {
                    Ok(()) => {
                        let to_frame = (to.as_secs_f64() * rate.get() as f64).round() as u64;
                        v.publish_seek(to_frame);
                        at_end = false;
                        // What's waiting is from before the seek.
                        out.clear();
                        sent = 0;
                        skip = None;
                        trimmer.seeked(to_frame);
                        None
                    }
                    Err(e) => Some(take_error().unwrap_or_else(|| {
                        format!("seek to {:.1} s failed: {e}", to.as_secs_f64())
                    })),
                };
            let _ = tx.send(Msg::SeekDone {
                voice: v.id,
                seq,
                error,
            });
            continue;
        }

        if sent == out.len() {
            batch.clear();
            while batch.len() < BATCH {
                match dec.next() {
                    Some(s) => batch.push(s),
                    None => {
                        at_end = true;
                        break;
                    }
                }
            }
            let mode = SkipSilence::from_u8(controls.skip_silence.load(Ordering::Relaxed));
            sent = 0;
            if mode == SkipSilence::Off && trimmer.is_idle() {
                std::mem::swap(&mut batch, &mut out);
                trimmer.passed();
            } else {
                out.clear();
                skip = trimmer.feed(mode, v.can_skip(), &batch, &mut out);
                if at_end {
                    skip = skip.or(trimmer.finish(mode, v.can_skip(), &mut out));
                }
            }
        }
        // As much as fits, in whole frames; a skip is told just before its sample goes in.
        let room = v.room(slots) as usize;
        let upto = (sent + room / ch * ch).min(out.len());
        if let Some(s) = skip.filter(|s| s.at <= upto) {
            v.push(slots, &out[sent..s.at]);
            v.mark_skip(s.frames);
            sent = s.at;
            skip = None;
        }
        v.push(slots, &out[sent..upto]);
        if upto > sent && v.stages.filled.get().is_none() {
            let _ = v.stages.filled.set(Instant::now());
        }
        sent = upto;
        if at_end && sent == out.len() {
            let error = take_error();
            v.mark_end(error.is_some());
            if let Some(msg) = error {
                let _ = tx.send(Msg::VoiceError { voice: v.id, msg });
            }
        }
    }
}

/// The mixer side of a voice. Runs on the audio thread: no I/O, no locks, no allocation,
/// and messages only on state changes.
///
/// Three gains shape what it plays, per frame and only while one of them moves or is below
/// full:
/// - the song's level, "Match song volumes" (`VoiceShared::set_level`), which ramps over
///   `LEVEL_RAMP` when the switch changes; it comes before the master bus, so the
///   compressor and the limiter see each song levelled;
/// - the crossfade, `sin(phase)`, ramping between silence and full over seconds
///   (`VoiceShared::fade`);
/// - the de-click, which ramps out over `DECLICK` on a pause or a stop, and back in on a
///   resume or after a dry ring, so none of them clicks.
///
/// None of them moves while the source plays silence (paused or starved): a pause holds a fade.
pub(crate) struct RingSource {
    voice: Arc<VoiceShared>,
    controls: Arc<Controls>,
    tx: Sender<Msg>,
    attach: u64,
    channels: ChannelCount,
    rate: SampleRate,
    local: Vec<Sample>,
    idx: usize,
    /// Plays the ring at the engine's speed with the pitch kept, from `input`, a chunk of the
    /// ring. Bypassed at 1.0 once it holds nothing (`Stretch::is_idle`): then the ring is
    /// copied into `local` directly.
    stretch: Stretch,
    input: Vec<Sample>,
    /// The crossfade phase, and how far it moves per frame (0 while it holds).
    phase: f64,
    phase_step: f64,
    /// The latest `fade_seq` taken.
    fade_seen: u32,
    /// The de-click gain, 0..=1, and how far it moves per frame.
    click: f32,
    click_step: f32,
    /// `DECLICK` in frames.
    declick: u64,
    /// The song's level ("Match song volumes"), where it's headed, how far it moves per
    /// frame, and `LEVEL_RAMP` in frames.
    level: f32,
    level_to: f32,
    level_step: f32,
    level_frames: f32,
    /// The last chunk was sound, not silence: a pause or a stop ramps it out.
    live: bool,
    /// The next refill ends the source: a stop's tail has played, or a fade out reached
    /// silence (`faded_out`, told then).
    ending: bool,
    faded_out: bool,
}

/// What a refill found in the ring.
enum Read {
    Sound,
    /// Nothing to play yet: a dry ring, or a seek being published.
    Silence,
    /// The end of the stream, at this seek sequence.
    End(u64),
}

impl RingSource {
    /// Refills `local` at a frame boundary. `None` ends the source.
    fn refill(&mut self) -> Option<()> {
        if self.ending || self.voice.attach.load(Ordering::Relaxed) != self.attach {
            if std::mem::take(&mut self.faded_out) {
                let _ = self.tx.send(Msg::FadedOut {
                    voice: self.voice.id,
                });
            }
            return None;
        }
        self.local.clear();
        self.idx = 0;
        self.take_fade();
        self.take_level();
        let stopped = self.voice.stopped.load(Ordering::Relaxed);
        let leaving = stopped || self.controls.paused.load(Ordering::Relaxed);
        let ch = u64::from(self.channels.get());
        let frames = if leaving {
            if !self.live {
                return if stopped { None } else { self.silence() };
            }
            // Out over the next few ms rather than with a click.
            self.ending = stopped;
            self.declick
        } else {
            TAKE as u64 / ch
        };
        self.stretch.set_speed(self.controls.speed());
        let read = if self.stretch.is_idle() {
            self.read(frames, false)?
        } else {
            self.read_stretched(frames)?
        };
        match read {
            Read::Sound => {}
            Read::Silence if stopped => return None,
            Read::Silence => {
                if std::mem::take(&mut self.live) {
                    // The ring ran dry: the sound comes back with a ramp.
                    self.click = 0.0;
                    self.click_step = 0.0;
                }
                return self.silence();
            }
            Read::End(seq) => return self.finish(seq),
        }
        if leaving {
            self.live = false;
            let n = (self.local.len() / ch as usize).max(1);
            self.click_step = -self.click / n as f32;
        } else if !std::mem::replace(&mut self.live, true) && self.click < 1.0 {
            self.click_step = 1.0 / self.declick as f32;
        }
        self.apply_gains();
        Some(())
    }

    /// Takes a new crossfade ramp, if the engine set one.
    fn take_fade(&mut self) {
        let v = &*self.voice;
        let seq = v.fade_seq.load(Ordering::Acquire);
        if seq == self.fade_seen {
            return;
        }
        self.fade_seen = seq;
        let frames = (f64::from(v.fade_ms.load(Ordering::Relaxed)) / 1000.0
            * f64::from(self.rate.get()))
        .max(1.0);
        let step = FULL / frames;
        self.phase_step = if v.fade_out.load(Ordering::Relaxed) {
            -step
        } else {
            step
        };
    }

    /// Heads for the level "Match song volumes" wants now, if it changed.
    fn take_level(&mut self) {
        let to = self.voice.level_for(&self.controls);
        if to != self.level_to {
            self.level_to = to;
            self.level_step = (to - self.level) / self.level_frames;
        }
    }

    /// Applies the level, the crossfade and the de-click to `local`, frame by frame, while
    /// any of them moves or is below full.
    fn apply_gains(&mut self) {
        if self.phase_step == 0.0
            && self.click_step == 0.0
            && self.level_step == 0.0
            && self.phase >= FULL
            && self.click >= 1.0
            && self.level >= 1.0
        {
            return;
        }
        let ch = self.channels.get() as usize;
        for frame in self.local.chunks_exact_mut(ch) {
            let fade = if self.phase >= FULL {
                1.0
            } else {
                (self.phase as f32).sin()
            };
            let g = self.level * self.click * fade;
            for s in frame {
                *s *= g;
            }
            self.click = (self.click + self.click_step).clamp(0.0, 1.0);
            self.phase = (self.phase + self.phase_step).clamp(0.0, FULL);
            if self.level_step != 0.0 {
                // Snapped within a step, so a step too small for `f32` still ends.
                if (self.level_to - self.level).abs() <= self.level_step.abs() {
                    self.level = self.level_to;
                    self.level_step = 0.0;
                } else {
                    self.level += self.level_step;
                }
            }
        }
        if self.click <= 0.0 || self.click >= 1.0 {
            self.click_step = 0.0;
        }
        if self.phase <= 0.0 && self.phase_step < 0.0 {
            // Faded out: the source ends at the next refill.
            self.ending = true;
            self.faded_out = true;
        }
        if self.phase <= 0.0 || self.phase >= FULL {
            self.phase_step = 0.0;
        }
        self.voice
            .fade_phase
            .store(self.phase.to_bits(), Ordering::Relaxed);
    }

    /// Fills `local` with up to `frames` frames of the ring played through the stretcher,
    /// reading as much of the ring as that takes, and sets the position from what the
    /// stretcher still holds. `None` ends the source.
    fn read_stretched(&mut self, frames: u64) -> Option<Read> {
        let ch = usize::from(self.channels.get());
        let want = frames as usize * ch;
        let v = &*self.voice;
        if v.seek_seq.load(Ordering::Acquire) != v.seek_seen.load(Ordering::Relaxed) {
            // A seek: what the stretcher holds is from before it, so none of it plays.
            self.stretch.reset();
        }
        let mut end = None;
        loop {
            self.local.resize(want, 0.0);
            let got = self.stretch.pull(&mut self.local);
            self.local.truncate(got);
            if got > 0 {
                // The frames read from the ring, less those the stretcher holds and those
                // about to play: where the chunk playing now starts, in track time.
                let speed = f64::from(self.stretch.speed());
                let behind = self.stretch.latency_frames() as f64 + (got / ch) as f64 * speed;
                let front = self.voice.front_frame.load(Ordering::Relaxed);
                let played = front.saturating_sub(behind.round() as u64);
                self.voice.played_frame.store(played, Ordering::Relaxed);
                return Some(Read::Sound);
            }
            if let Some(seq) = end {
                return Some(Read::End(seq));
            }
            let room = self.stretch.room_frames().min(TAKE / ch) as u64;
            if room == 0 {
                // Full yet nothing out: can't happen, but never read what wouldn't fit.
                return Some(Read::Silence);
            }
            self.input.clear();
            match self.read(room, true)? {
                Read::Sound => {
                    self.stretch.push(&self.input);
                }
                Read::Silence => return Some(Read::Silence),
                Read::End(seq) => {
                    // The stretcher plays out what it holds, then the source ends.
                    self.stretch.finish();
                    end = Some(seq);
                }
            }
        }
    }

    /// Takes up to `frames` from the ring, following seeks and skips: into `local`, or for
    /// the stretcher into `input`, leaving the position to `read_stretched`. `None` ends the
    /// source.
    fn read(&mut self, frames: u64, stretching: bool) -> Option<Read> {
        let v = &*self.voice;
        let slots = v.slots.get()?;
        let ch = self.channels.get() as u64;

        // Move to the latest seek.
        let seq = v.seek_seq.load(Ordering::SeqCst);
        if seq % 2 == 1 {
            return Some(Read::Silence); // being published; the next refill reads it
        }
        if seq != v.seek_seen.load(Ordering::Relaxed) {
            let start = v.seek_start.load(Ordering::Relaxed);
            let frame = v.seek_frame.load(Ordering::Relaxed);
            fence(Ordering::Acquire);
            if v.seek_seq.load(Ordering::Relaxed) != seq {
                return Some(Read::Silence);
            }
            v.read.store(start, Ordering::Release);
            v.front_frame.store(frame, Ordering::Relaxed);
            v.played_frame.store(frame, Ordering::Relaxed);
            v.seek_seen.store(seq, Ordering::Release);
            // What the stretcher holds is from before the seek.
            self.stretch.reset();
            // The ring is free again from `start` on; the decoder may be waiting for room.
            v.cv.notify_one();
        }

        let read = v.read.load(Ordering::Relaxed);
        let write = v.write.load(Ordering::Acquire);
        let skip_at = v.skip_at.load(Ordering::Acquire);
        let mut n = (write - read).min(frames * ch);
        if skip_at > read && skip_at < read + n {
            // A chunk ends at a skip, so the position moves when that sample plays.
            n = skip_at - read;
        }
        if n == 0 {
            if v.end.load(Ordering::Acquire) == read {
                return Some(Read::End(seq));
            }
            if v.started.load(Ordering::Relaxed) && !v.starved.swap(true, Ordering::Relaxed) {
                self.controls.glitches.starved();
                let _ = self.tx.send(Msg::Starved {
                    voice: v.id,
                    on: true,
                });
            }
            v.cv.notify_one();
            return Some(Read::Silence);
        }
        let to = if stretching {
            &mut self.input
        } else {
            &mut self.local
        };
        to.extend((read..read + n).map(|i| f32::from_bits(slots.at(i).load(Ordering::Relaxed))));
        // A seek published while copying may have overwritten what was copied.
        fence(Ordering::Acquire);
        if v.seek_seq.load(Ordering::Relaxed) != seq {
            to.clear();
            return Some(Read::Silence);
        }
        v.read.store(read + n, Ordering::Release);
        let mut front = v.front_frame.load(Ordering::Relaxed);
        let mut skipped = false;
        if skip_at == read {
            let frames = v.skip_frames.load(Ordering::Relaxed);
            // Fails only if a seek cleared it meanwhile; the next refill moves there.
            if v.skip_at
                .compare_exchange(read, NO_SKIP, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                front += frames;
                skipped = true;
            }
        }
        if !stretching {
            v.played_frame.store(front, Ordering::Relaxed);
        }
        v.front_frame.store(front + n / ch, Ordering::Relaxed);
        if write - (read + n) < slots.len() / 2 {
            v.cv.notify_one();
        }
        if !v.started.swap(true, Ordering::Relaxed) {
            let _ = self.tx.send(Msg::Started { voice: v.id });
        }
        if skipped {
            let _ = self.tx.send(Msg::Skipped { voice: v.id });
        }
        if v.starved.swap(false, Ordering::Relaxed) {
            let _ = self.tx.send(Msg::Starved {
                voice: v.id,
                on: false,
            });
        }
        Some(Read::Sound)
    }

    /// A step of silence (paused, starved, or waiting for a seek).
    fn silence(&mut self) -> Option<()> {
        self.local
            .resize(SILENCE_FRAMES * self.channels.get() as usize, 0.0);
        Some(())
    }

    /// Played to the end: ends the source, unless a seek came in meanwhile. A seek after
    /// the end revives the voice: the engine, on `SeekDone`, attaches a new source if this
    /// one has set `ended`. Both sides use SeqCst, so either this sees the seek, or the
    /// engine sees `ended`.
    fn finish(&mut self, seq: u64) -> Option<()> {
        let v = &*self.voice;
        let first = !v.ended.swap(true, Ordering::SeqCst);
        if v.seek_seq.load(Ordering::SeqCst) != seq {
            if v.ended.swap(false, Ordering::SeqCst) {
                // The engine hasn't taken `ended`: play on from the seek.
                return self.silence();
            }
            return None; // it has, and attached a new source
        }
        if first && !v.failed.load(Ordering::Relaxed) {
            let _ = self.tx.send(Msg::Ended { voice: v.id });
        }
        None
    }
}

#[cfg(test)]
impl Drop for RingSource {
    fn drop(&mut self) {
        self.controls.sources.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Iterator for RingSource {
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Sample> {
        if self.idx >= self.local.len() {
            self.refill()?;
        }
        let s = self.local[self.idx];
        self.idx += 1;
        Some(s)
    }
}

impl Source for RingSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    /// A voice at 44.1 kHz stereo, with no decoder thread: the tests play its part.
    fn voice() -> Arc<VoiceShared> {
        let v = Arc::new(VoiceShared::new(1, "test".into()));
        v.set_format(
            ChannelCount::new(2).unwrap(),
            SampleRate::new(44_100).unwrap(),
        );
        v
    }

    fn playing() -> Arc<Controls> {
        let c = Arc::new(Controls::new());
        c.paused.store(false, Ordering::SeqCst);
        c
    }

    fn ramp(from: usize, n: usize) -> Vec<Sample> {
        (from..from + n).map(|i| i as Sample).collect()
    }

    #[test]
    fn the_audio_thread_reads_on_while_the_decoder_side_holds_its_lock() {
        let v = voice();
        let slots = v.slots.get().unwrap();
        v.push(slots, &ramp(0, 8192));
        let (tx, _rx) = mpsc::channel();
        let mut src = v.source(playing(), tx).unwrap();
        // The decoder thread (or the engine, asking for a seek) holds the voice's lock, as
        // when it's preempted on a busy machine.
        let held = v.seek.lock_or_recover();
        let (done, got) = mpsc::channel();
        std::thread::spawn(move || {
            let samples: Vec<Sample> = src.by_ref().take(8192).collect();
            let _ = done.send(samples);
        });
        let samples = got
            .recv_timeout(Duration::from_secs(5))
            .expect("the audio thread waited for the decoder's lock");
        drop(held);
        assert_eq!(samples, ramp(0, 8192));
    }

    #[test]
    fn a_seek_skips_what_was_decoded_before_it() {
        let v = voice();
        let slots = v.slots.get().unwrap();
        v.push(slots, &[-1.0; 4096]);
        v.publish_seek(22_050);
        v.push(slots, &ramp(0, 4096));
        // Before the audio thread gets there (it doesn't run while the output is stopped).
        assert_eq!(v.position(), Duration::from_millis(500));
        let (tx, _rx) = mpsc::channel();
        let src = v.source(playing(), tx).unwrap();
        // The ramp's first sample is 0.0, as silence is.
        let heard: Vec<Sample> = src.filter(|&s| s != 0.0).take(4095).collect();
        assert_eq!(heard, ramp(1, 4095));
    }

    #[test]
    fn a_seek_frees_the_whole_ring_for_the_new_position() {
        let v = voice();
        let slots = v.slots.get().unwrap();
        while v.room(slots) >= BATCH as u64 {
            v.push(slots, &[0.5; BATCH]);
        }
        v.publish_seek(0);
        assert_eq!(v.room(slots), slots.len());
    }

    #[test]
    fn a_skip_moves_the_position_when_playback_reaches_it_not_before() {
        let v = voice();
        let slots = v.slots.get().unwrap();
        v.push(slots, &ramp(0, 3000));
        // A second of silence was dropped here (`silence`).
        v.mark_skip(44_100);
        v.push(slots, &ramp(3000, 3000));
        let (tx, rx) = mpsc::channel();
        let mut src = v.source(playing(), tx).unwrap();
        let frames = |f: u64| Duration::from_secs_f64(f as f64 / 44_100.0);
        let heard: Vec<Sample> = src.by_ref().take(3000).collect();
        assert_eq!(heard, ramp(0, 3000));
        // The chunk playing now is the one just before the skip.
        assert_eq!(v.position(), frames(1024));
        assert!(!rx.try_iter().any(|m| matches!(m, Msg::Skipped { .. })));

        assert_eq!(src.next(), Some(3000.0));
        assert_eq!(v.position(), frames(1500 + 44_100));
        let skipped = rx
            .try_iter()
            .filter(|m| matches!(m, Msg::Skipped { voice: 1 }))
            .count();
        assert_eq!(skipped, 1);
    }

    /// A voice holding `secs` of a constant 0.5 on both channels.
    fn steady(secs: f64) -> Arc<VoiceShared> {
        let v = voice();
        let slots = v.slots.get().unwrap();
        let n = (secs * 44_100.0) as usize * 2;
        v.push(slots, &vec![0.5; n]);
        v
    }

    /// The gain of each frame `src` plays next, for `frames` frames (the voices hold 0.5).
    fn gains(src: &mut RingSource, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|_| {
                let l = src.next().unwrap_or(0.0);
                let _ = src.next();
                l / 0.5
            })
            .collect()
    }

    #[test]
    fn a_crossfade_keeps_the_power_steady_and_is_even_at_the_midpoint() {
        let controls = playing();
        let (tx, _rx) = mpsc::channel();
        let (old, new) = (steady(2.0), steady(2.0));
        let mut out = old.source(controls.clone(), tx.clone()).unwrap();
        // Whole chunks, so both ramps start on the same frame.
        let _ = gains(&mut out, 4096);
        let fade = Duration::from_secs(1);
        old.fade(Fade::Out, fade);
        new.start_silent();
        new.fade(Fade::In, fade);
        let mut inc = new.source(controls, tx).unwrap();
        let a = gains(&mut out, 44_100);
        let b = gains(&mut inc, 44_100);
        for (i, (a, b)) in a.iter().zip(&b).enumerate() {
            let power = a * a + b * b;
            assert!(
                (power - 1.0).abs() < 1e-3,
                "frame {i}: {a}² + {b}² = {power}"
            );
        }
        let mid = 22_050;
        assert!((a[mid] - b[mid]).abs() < 1e-3, "{} vs {}", a[mid], b[mid]);
        assert!((a[mid] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
    }

    #[test]
    fn a_pause_holds_a_fade_where_it_is() {
        let controls = playing();
        let (tx, _rx) = mpsc::channel();
        let v = steady(3.0);
        v.start_silent();
        v.fade(Fade::In, Duration::from_secs(1));
        let mut src = v.source(controls.clone(), tx).unwrap();
        let before = gains(&mut src, 22_050);
        controls.paused.store(true, Ordering::SeqCst);
        // What was taken before the pause plays out, then a short tail, then silence.
        let _ = gains(&mut src, 4410);
        let held = f64::from_bits(v.fade_phase.load(Ordering::Relaxed));
        let silent = gains(&mut src, 44_100);
        assert!(silent.iter().all(|&g| g == 0.0));
        assert_eq!(f64::from_bits(v.fade_phase.load(Ordering::Relaxed)), held);
        controls.paused.store(false, Ordering::SeqCst);
        let after = gains(&mut src, 4410);
        let last = before[before.len() - 1];
        // Back up past where it paused (after the short ramp in), and still below full.
        assert!(
            after[4409] > last && after[4409] < 0.9,
            "{last} then {}",
            after[4409]
        );
    }

    #[test]
    fn a_pause_ramps_out_instead_of_cutting() {
        let controls = playing();
        let (tx, _rx) = mpsc::channel();
        let v = steady(2.0);
        let mut src = v.source(controls.clone(), tx).unwrap();
        let _ = gains(&mut src, 4410);
        controls.paused.store(true, Ordering::SeqCst);
        let g = gains(&mut src, 8820);
        let biggest = g
            .windows(2)
            .map(|w| (w[0] - w[1]).abs())
            .fold(0.0, f32::max);
        assert!(biggest < 0.01, "a step of {biggest} between frames");
        let ramp = g.iter().filter(|&&x| x > 0.0 && x < 1.0).count();
        assert!((1000..=1400).contains(&ramp), "{ramp} frames of ramp");
        assert_eq!(g[8819], 0.0);
    }

    #[test]
    fn a_resume_ramps_in() {
        let controls = playing();
        let (tx, _rx) = mpsc::channel();
        let v = steady(2.0);
        let mut src = v.source(controls.clone(), tx).unwrap();
        let _ = gains(&mut src, 4410);
        controls.paused.store(true, Ordering::SeqCst);
        let _ = gains(&mut src, 8820);
        controls.paused.store(false, Ordering::SeqCst);
        let g = gains(&mut src, 4410);
        let biggest = g
            .windows(2)
            .map(|w| (w[0] - w[1]).abs())
            .fold(0.0, f32::max);
        assert!(biggest < 0.01, "a step of {biggest} between frames");
        assert_eq!(g[4409], 1.0);
    }

    #[test]
    fn a_stopped_voice_ramps_out_then_ends() {
        let controls = playing();
        let (tx, _rx) = mpsc::channel();
        let v = steady(2.0);
        let mut src = v.source(controls, tx).unwrap();
        let _ = gains(&mut src, 4410);
        v.stop();
        let tail: Vec<f32> = src.by_ref().take(88_200 * 2).map(|s| s / 0.5).collect();
        assert!(
            tail.len() / 2 <= 2048 + 1323,
            "{} frames after the stop",
            tail.len() / 2
        );
        let biggest = tail
            .windows(2)
            .map(|w| (w[0] - w[1]).abs())
            .fold(0.0, f32::max);
        assert!(biggest < 0.01, "a step of {biggest}");
        assert!(tail[tail.len() - 1] < 0.01);
    }

    #[test]
    fn a_new_source_for_a_voice_already_heard_ramps_in() {
        let controls = playing();
        let (tx, _rx) = mpsc::channel();
        let v = steady(2.0);
        let mut first = v.source(controls.clone(), tx.clone()).unwrap();
        let _ = gains(&mut first, 4096);
        // Another device: a new source takes over mid-song.
        let mut second = v.source(controls, tx).unwrap();
        let g = gains(&mut second, 2048);
        assert!(g[0] < 0.01, "starts at {}", g[0]);
        assert_eq!(g[2047], 1.0);
    }

    #[test]
    fn a_voice_loaded_mid_song_ramps_in() {
        let controls = playing();
        let (tx, _rx) = mpsc::channel();
        let v = steady(2.0);
        v.ramp_in_first();
        let mut src = v.source(controls, tx).unwrap();
        let g = gains(&mut src, 2048);
        assert!(g[0] < 0.01, "starts at {}", g[0]);
        assert_eq!(g[2047], 1.0);
    }

    #[test]
    fn a_loud_song_plays_at_its_own_level_from_the_first_frame() {
        let (tx, _rx) = mpsc::channel();
        let v = steady(2.0);
        v.set_level(0.25);
        let mut src = v.source(playing(), tx).unwrap();
        let g = gains(&mut src, 8192);
        assert!(g.iter().all(|&x| x == 0.25), "{} .. {}", g[0], g[8191]);
    }

    #[test]
    fn match_song_volumes_turned_off_ramps_the_song_back_up_without_a_step() {
        let controls = playing();
        let (tx, _rx) = mpsc::channel();
        let v = steady(2.0);
        v.set_level(0.25);
        let mut src = v.source(controls.clone(), tx).unwrap();
        let _ = gains(&mut src, 4096);
        controls.match_volumes.store(false, Ordering::Relaxed);
        let g = gains(&mut src, 8192);
        let biggest = g
            .windows(2)
            .map(|w| (w[0] - w[1]).abs())
            .fold(0.0, f32::max);
        assert!(biggest < 0.01, "a step of {biggest} between frames");
        assert_eq!(g[8191], 1.0);
        controls.match_volumes.store(true, Ordering::Relaxed);
        let back = gains(&mut src, 8192);
        assert_eq!(back[8191], 0.25);
    }

    #[test]
    fn a_pause_at_48_khz_ramps_out_without_allocating() {
        let controls = playing();
        let (tx, _rx) = mpsc::channel();
        let v = Arc::new(VoiceShared::new(1, "test".into()));
        v.set_format(
            ChannelCount::new(2).unwrap(),
            SampleRate::new(48_000).unwrap(),
        );
        v.push(v.slots.get().unwrap(), &vec![0.5; 96_000]);
        let mut src = v.source(controls.clone(), tx).unwrap();
        let capacity = src.local.capacity();
        let _ = src.by_ref().take(4096).count();
        controls.paused.store(true, Ordering::SeqCst);
        let _ = src.by_ref().take(8192).count();
        assert_eq!(src.local.capacity(), capacity, "the audio thread allocated");
    }

    #[test]
    fn a_voice_faded_out_ends_and_says_so() {
        let controls = playing();
        let (tx, rx) = mpsc::channel();
        let v = steady(2.0);
        let mut src = v.source(controls, tx).unwrap();
        v.fade(Fade::Out, Duration::from_millis(100));
        // Bounded: a source that never ends would hang the test.
        let heard = src.by_ref().take(88_200 * 2).count() / 2;
        assert!(heard < 4410 + 2048, "{heard} frames");
        let faded = rx
            .try_iter()
            .filter(|m| matches!(m, Msg::FadedOut { voice: 1 }))
            .count();
        assert_eq!(faded, 1);
    }

    #[test]
    fn a_seek_clears_a_skip_not_yet_reached() {
        let v = voice();
        let slots = v.slots.get().unwrap();
        v.push(slots, &ramp(0, 3000));
        v.mark_skip(44_100);
        v.push(slots, &ramp(3000, 3000));
        v.publish_seek(0);
        assert!(v.can_skip());
    }

    /// Controls playing at `speed`.
    fn playing_at(speed: f32) -> Arc<Controls> {
        let c = playing();
        c.set_speed(speed);
        c
    }

    /// Plays `frames` frames from `src`, topping the ring up with a 440 Hz tone (peak 0.4)
    /// as a decoder would; `next` counts the frames written so far.
    fn play_fed(
        v: &VoiceShared,
        src: &mut RingSource,
        next: &mut usize,
        frames: usize,
    ) -> Vec<Sample> {
        let slots = v.slots.get().unwrap();
        let mut heard = Vec::with_capacity(frames * 2);
        while heard.len() < frames * 2 {
            while v.room(slots) >= 2048 {
                let tone: Vec<Sample> = (*next..*next + 1024)
                    .flat_map(|i| {
                        let x = 0.4 * (std::f32::consts::TAU * 440.0 * i as f32 / 44_100.0).sin();
                        [x, x]
                    })
                    .collect();
                v.push(slots, &tone);
                *next += 1024;
            }
            heard.extend(src.by_ref().take(1024.min(frames * 2 - heard.len())));
        }
        heard
    }

    #[test]
    fn at_2x_the_position_moves_two_seconds_per_second_played() {
        let v = voice();
        let (tx, _rx) = mpsc::channel();
        let mut src = v.source(playing_at(2.0), tx).unwrap();
        let mut next = 0;
        let _ = play_fed(&v, &mut src, &mut next, 22_050);
        let from = v.position();
        let _ = play_fed(&v, &mut src, &mut next, 44_100);
        let moved = v.position() - from;
        assert!(
            (moved.as_secs_f64() - 2.0).abs() < 0.06,
            "{moved:?} in a second played"
        );
    }

    #[test]
    fn a_seek_while_stretching_lands_on_its_target() {
        let v = voice();
        let slots = v.slots.get().unwrap();
        v.push(slots, &[-0.5; 44_100]);
        let (tx, _rx) = mpsc::channel();
        let mut src = v.source(playing_at(1.5), tx).unwrap();
        // The stretcher holds some of the old place when the seek comes.
        let _ = src.by_ref().take(4096).count();
        v.publish_seek(441_000);
        assert_eq!(v.position(), Duration::from_secs(10));
        let mut next = 0;
        let heard = play_fed(&v, &mut src, &mut next, 22_050);
        // What was already taken before the seek may finish; nothing of it after that.
        assert!(
            heard[TAKE..].iter().all(|&s| s > -0.45),
            "the old place played after the seek"
        );
        // Half a second played at 1.5x: 0.75 s of the track, less up to a chunk.
        let at = v.position().as_secs_f64();
        assert!((10.68..=10.76).contains(&at), "at {at} s");
    }

    #[test]
    fn a_crossfade_at_2x_keeps_its_length_in_output_time() {
        let controls = playing_at(2.0);
        let (tx, _rx) = mpsc::channel();
        let (old, new) = (steady(3.0), steady(3.0));
        let mut out = old.source(controls.clone(), tx.clone()).unwrap();
        let _ = gains(&mut out, 4096);
        let fade = Duration::from_secs(1);
        old.fade(Fade::Out, fade);
        new.start_silent();
        new.fade(Fade::In, fade);
        let mut inc = new.source(controls, tx).unwrap();
        let a = gains(&mut out, 46_000);
        let b = gains(&mut inc, 46_000);
        // Halfway through the second, not the half second the track's time would take.
        let (mid, half) = (22_050, std::f32::consts::FRAC_1_SQRT_2);
        assert!((a[mid] - half).abs() < 0.04, "the old voice at {}", a[mid]);
        assert!((b[mid] - half).abs() < 0.04, "the new voice at {}", b[mid]);
        assert!(a[45_999] < 0.01, "the old voice still at {}", a[45_999]);
        assert!(b[45_999] > 0.99, "the new voice at {}", b[45_999]);
        // And the track it fades into plays at 2x meanwhile.
        let at = new.position().as_secs_f64();
        assert!((1.95..=2.1).contains(&at), "the new track at {at} s");
    }

    #[test]
    fn a_skip_while_stretching_moves_the_position() {
        let v = voice();
        let slots = v.slots.get().unwrap();
        v.push(slots, &[0.5; 6000]);
        v.mark_skip(44_100);
        v.push(slots, &[0.5; 6000]);
        let (tx, rx) = mpsc::channel();
        let mut src = v.source(playing_at(2.0), tx).unwrap();
        let _ = src.by_ref().take(4000).count();
        let at = v.position().as_secs_f64();
        assert!(at > 1.0, "at {at} s");
        assert!(
            rx.try_iter()
                .any(|m| matches!(m, Msg::Skipped { voice: 1 }))
        );
    }

    #[test]
    fn a_ring_that_trickles_in_while_stretching_loses_nothing() {
        let v = voice();
        let slots = v.slots.get().unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut src = v.source(playing_at(2.0), tx).unwrap();
        // 2 s arriving 500 frames at a time, less than a stretch step needs: the ring runs
        // dry between pieces, as with a slow download.
        let mut heard = 0;
        for _ in 0..176 {
            v.push(slots, &[0.5; 1000]);
            heard += src.by_ref().take(4000).filter(|&s| s > 0.0).count() / 2;
        }
        // 1 s of output at 2x, less what the stretcher still holds.
        assert!(heard > 40_000, "{heard} frames heard of about 44100");
    }

    #[test]
    fn back_at_1x_the_ring_is_read_directly_again() {
        let v = voice();
        let (tx, _rx) = mpsc::channel();
        let controls = playing_at(2.0);
        let mut src = v.source(controls.clone(), tx).unwrap();
        let mut next = 0;
        let _ = play_fed(&v, &mut src, &mut next, 22_050);
        controls.set_speed(1.0);
        // The stretcher crossfades back and drains within a chunk or two.
        let _ = play_fed(&v, &mut src, &mut next, 4410);
        assert!(src.stretch.is_idle(), "still stretching at 1x");
    }
}
