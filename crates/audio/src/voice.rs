//! One loaded track. A decoder thread does all blocking work (network reads, demuxer seeks,
//! decoding) and fills a ring of samples; `RingSource` on the audio thread only copies
//! from the ring, so a slow network never stalls the audio callback.
//!
//! The ring has one writer (the decoder thread) and one reader (the audio thread) and no
//! lock between them: the audio thread runs in the real-time range (`priority.rs`), and a
//! lock it shared with a normal thread would let that thread, preempted while holding it,
//! hold up the output as long as the scheduler keeps it waiting. Indices count samples
//! from the voice's start and never wrap; a slot is `index % capacity`.
//!
//! A seek can't empty the ring from the decoder's side, since the reader owns the read
//! index. The decoder publishes where the new position's samples start instead (a seqlock:
//! `seek_seq` odd while it writes, even once done), and from then on treats everything
//! before that as free space. The reader jumps there on its next refill. Should the decoder
//! overwrite samples the reader is copying at that moment, the reader sees `seek_seq` move
//! once it's done and drops what it copied.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use featherplay_core::sync::{CondvarExt, LockExt};
use rodio::{ChannelCount, Sample, SampleRate, Source};
use tokio::runtime::Runtime;

use crate::StreamSource;
use crate::decoder::Decoder;
use crate::engine::Msg;
use crate::glitches::Glitches;
use crate::head::HeadCache;
use crate::http::ErrorSlot;
use crate::media::{Cancel, Media, Opened};

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

/// Play state shared by every voice of one engine.
pub(crate) struct Controls {
    pub paused: AtomicBool,
    gain: AtomicU32,
    pub glitches: Arc<Glitches>,
}

impl Controls {
    pub fn new() -> Self {
        Self {
            paused: AtomicBool::new(true),
            gain: AtomicU32::new(1.0f32.to_bits()),
            glitches: Arc::default(),
        }
    }
    pub fn set_gain(&self, g: f32) {
        self.gain.store(g.to_bits(), Ordering::Relaxed);
    }
    fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }
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

pub(crate) struct VoiceShared {
    pub id: u64,
    pub url: String,
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
        Some(RingSource {
            voice: self.clone(),
            controls,
            tx,
            attach,
            channels,
            rate,
            local: Vec::with_capacity(TAKE.max(SILENCE_FRAMES * channels.get() as usize)),
            idx: 0,
            gain: 0.0,
        })
    }

    // The decoder thread's side.

    /// Sets the format and makes the ring, ~`RING` of it.
    fn set_format(&self, channels: ChannelCount, rate: SampleRate) {
        let frames = (RING.as_secs_f64() * rate.get() as f64) as usize;
        let _ = self.slots.set(Slots::new(frames * channels.get() as usize));
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
        self.end.store(NO_END, Ordering::Relaxed);
        self.failed.store(false, Ordering::Relaxed);
        // SeqCst: `RingSource::finish` relies on its order against `ended`.
        self.seek_seq.store(seq + 2, Ordering::SeqCst);
    }
}

/// Starts a voice: its decoder thread opens `src`, then sends `Msg::Ready` and decodes ahead
/// until the ring is full. The thread holds `rt` so a cancelled download can still wind
/// down after the engine is gone.
pub(crate) fn spawn(
    id: u64,
    src: StreamSource,
    rt: Arc<Runtime>,
    client: reqwest::Client,
    tmp_dir: PathBuf,
    heads: HeadCache,
    tx: Sender<Msg>,
) -> Arc<VoiceShared> {
    let shared = Arc::new(VoiceShared::new(id, src.url.clone()));
    let v = shared.clone();
    let err_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("featherplay-decode-{id}"))
        .spawn(move || {
            crate::priority::raise_decoder();
            if let Err(e) = decode(&v, &src, &rt, &client, &tmp_dir, &heads, &tx) {
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
    v: &VoiceShared,
    src: &StreamSource,
    rt: &Runtime,
    client: &reqwest::Client,
    tmp_dir: &std::path::Path,
    heads: &HeadCache,
    tx: &Sender<Msg>,
) -> Result<(), String> {
    let error = ErrorSlot::default();
    let Opened { media, len, cancel } =
        Media::open(src, rt.handle(), client, tmp_dir, heads, error.clone())?;
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
    let _ = tx.send(Msg::Ready { voice: v.id });

    let mut batch: Vec<Sample> = Vec::with_capacity(BATCH);
    // The decoder reached the end of the stream (until the next seek).
    let mut at_end = false;
    loop {
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
                if !at_end && v.room(slots) >= BATCH as u64 {
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
                        v.publish_seek((to.as_secs_f64() * rate.get() as f64).round() as u64);
                        at_end = false;
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
        v.push(slots, &batch);
        if at_end {
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
pub(crate) struct RingSource {
    voice: Arc<VoiceShared>,
    controls: Arc<Controls>,
    tx: Sender<Msg>,
    attach: u64,
    channels: ChannelCount,
    rate: SampleRate,
    local: Vec<Sample>,
    idx: usize,
    gain: f32,
}

impl RingSource {
    /// Refills `local` at a frame boundary. `None` ends the source.
    fn refill(&mut self) -> Option<()> {
        let v = &*self.voice;
        if v.stopped.load(Ordering::Relaxed) || v.attach.load(Ordering::Relaxed) != self.attach {
            return None;
        }
        self.gain = self.controls.gain();
        self.local.clear();
        self.idx = 0;
        if self.controls.paused.load(Ordering::Relaxed) {
            return self.silence();
        }
        let slots = v.slots.get()?;
        let ch = self.channels.get() as u64;

        // Move to the latest seek.
        let seq = v.seek_seq.load(Ordering::SeqCst);
        if seq % 2 == 1 {
            return self.silence(); // being published; the next refill reads it
        }
        if seq != v.seek_seen.load(Ordering::Relaxed) {
            let start = v.seek_start.load(Ordering::Relaxed);
            let frame = v.seek_frame.load(Ordering::Relaxed);
            fence(Ordering::Acquire);
            if v.seek_seq.load(Ordering::Relaxed) != seq {
                return self.silence();
            }
            v.read.store(start, Ordering::Release);
            v.front_frame.store(frame, Ordering::Relaxed);
            v.played_frame.store(frame, Ordering::Relaxed);
            v.seek_seen.store(seq, Ordering::Release);
            // The ring is free again from `start` on; the decoder may be waiting for room.
            v.cv.notify_one();
        }

        let read = v.read.load(Ordering::Relaxed);
        let write = v.write.load(Ordering::Acquire);
        let n = (write - read).min(TAKE as u64 / ch * ch);
        if n == 0 {
            if v.end.load(Ordering::Acquire) == read {
                return self.finish(seq);
            }
            if v.started.load(Ordering::Relaxed) && !v.starved.swap(true, Ordering::Relaxed) {
                self.controls.glitches.starved();
                let _ = self.tx.send(Msg::Starved {
                    voice: v.id,
                    on: true,
                });
            }
            v.cv.notify_one();
            return self.silence();
        }
        self.local
            .extend((read..read + n).map(|i| f32::from_bits(slots.at(i).load(Ordering::Relaxed))));
        // A seek published while copying may have overwritten what was copied.
        fence(Ordering::Acquire);
        if v.seek_seq.load(Ordering::Relaxed) != seq {
            self.local.clear();
            return self.silence();
        }
        v.read.store(read + n, Ordering::Release);
        let front = v.front_frame.load(Ordering::Relaxed);
        v.played_frame.store(front, Ordering::Relaxed);
        v.front_frame.store(front + n / ch, Ordering::Relaxed);
        if write - (read + n) < slots.len() / 2 {
            v.cv.notify_one();
        }
        if !v.started.swap(true, Ordering::Relaxed) {
            let _ = self.tx.send(Msg::Started { voice: v.id });
        }
        if v.starved.swap(false, Ordering::Relaxed) {
            let _ = self.tx.send(Msg::Starved {
                voice: v.id,
                on: false,
            });
        }
        Some(())
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

impl Iterator for RingSource {
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Sample> {
        if self.idx >= self.local.len() {
            self.refill()?;
        }
        let s = self.local[self.idx];
        self.idx += 1;
        Some(s * self.gain)
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
}
