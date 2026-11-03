//! One loaded track. A decoder thread does all blocking work (network reads, demuxer seeks,
//! decoding) and fills a ring of samples; `RingSource` on the cpal callback thread only
//! copies from the ring, so a slow network never stalls the audio callback.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use featherplay_core::sync::{CondvarExt, LockExt};
use rodio::{ChannelCount, Sample, SampleRate, Source};
use tokio::runtime::Runtime;

use crate::StreamSource;
use crate::decoder::Decoder;
use crate::engine::Msg;
use crate::head::HeadCache;
use crate::http::ErrorSlot;
use crate::media::{Cancel, Media, Opened};

/// Samples the decoder thread decodes between ring updates.
const BATCH: usize = 4096;
/// Most samples the audio thread takes from the ring at once (~23 ms of 44.1 kHz stereo).
const TAKE: usize = 2048;
/// Frames of silence output per step while paused or starved.
const SILENCE_FRAMES: usize = 256;

/// Play state shared by every voice of one engine.
pub(crate) struct Controls {
    pub paused: AtomicBool,
    gain: AtomicU32,
}

impl Controls {
    pub fn new() -> Self {
        Self {
            paused: AtomicBool::new(true),
            gain: AtomicU32::new(1.0f32.to_bits()),
        }
    }
    pub fn set_gain(&self, g: f32) {
        self.gain.store(g.to_bits(), Ordering::Relaxed);
    }
    fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }
}

/// The samples between the decoder thread and the audio thread. Locked with
/// `lock_or_recover`: no section that holds the lock can leave it half-updated. Each one
/// sets single fields, or moves samples and then advances the frame counters with nothing
/// that can panic in between (no indexing, channel counts are non-zero, `drain` stays
/// within `len`), and the
/// decoding itself runs with the lock released. So a lock poisoned by a panic elsewhere
/// still holds a consistent ring, and the other thread can go on or wind down normally.
struct Ring {
    buf: VecDeque<Sample>,
    /// Frame index (from the start of the track) of the next sample the audio thread takes.
    front_frame: u64,
    /// Frame index of the chunk the audio thread is playing now; this is `position()`.
    played_frame: u64,
    /// The decoder reached the end of the stream.
    eof: bool,
    /// The stream ended with an error; the source ends without `Ended`.
    failed: bool,
    /// A seek for the decoder thread: target and the engine's sequence number.
    seek: Option<(Duration, u64)>,
}

pub(crate) struct VoiceShared {
    pub id: u64,
    pub url: String,
    ring: Mutex<Ring>,
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
    pub fn position(&self) -> Duration {
        let Some((_, rate)) = self.format.get() else {
            return Duration::ZERO;
        };
        let frame = self.ring.lock_or_recover().played_frame;
        Duration::from_secs_f64(frame as f64 / rate.get() as f64)
    }

    pub fn request_seek(&self, to: Duration, seq: u64) {
        self.ring.lock_or_recover().seek = Some((to, seq));
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
}

/// Starts a voice: its decoder thread opens `src`, then sends `Msg::Ready` and decodes ahead
/// until about 1 s is buffered. The thread holds `rt` so a cancelled download can still wind
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
    let shared = Arc::new(VoiceShared {
        id,
        url: src.url.clone(),
        ring: Mutex::new(Ring {
            buf: VecDeque::new(),
            front_frame: 0,
            played_frame: 0,
            eof: false,
            failed: false,
            seek: None,
        }),
        cv: Condvar::new(),
        stopped: AtomicBool::new(false),
        attach: AtomicU64::new(0),
        format: OnceLock::new(),
        started: AtomicBool::new(false),
        ended: AtomicBool::new(false),
        starved: AtomicBool::new(false),
        cancel: Mutex::new(None),
    });
    let v = shared.clone();
    let err_tx = tx.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("featherplay-decode-{id}"))
        .spawn(move || {
            if let Err(e) = decode(&v, &src, &rt, &client, &tmp_dir, &heads, &tx) {
                v.ring.lock_or_recover().failed = true;
                let _ = tx.send(Msg::VoiceError {
                    voice: v.id,
                    msg: e,
                });
            }
        });
    if let Err(e) = spawned {
        shared.ring.lock_or_recover().failed = true;
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
    let _ = v.format.set((channels, rate));
    let cap = rate.get() as usize * channels.get() as usize; // ~1 s
    let _ = tx.send(Msg::Ready { voice: v.id });

    let mut batch: Vec<Sample> = Vec::with_capacity(BATCH);
    loop {
        // Wait for a seek, room in the ring, or stop.
        let seek = {
            let mut g = v.ring.lock_or_recover();
            loop {
                if v.stopped.load(Ordering::SeqCst) {
                    return Ok(());
                }
                if let Some(s) = g.seek.take() {
                    break Some(s);
                }
                if !g.eof && g.buf.len() < cap {
                    break None;
                }
                g = v.cv.wait_or_recover(g);
            }
        };
        if let Some((to, seq)) = seek {
            // May block on the network for a cold seek; the ring keeps playing what it has
            // until the clear below, then the source outputs silence (starved).
            let res = dec.try_seek(to);
            let mut g = v.ring.lock_or_recover();
            match res {
                Ok(()) => {
                    let frame = (to.as_secs_f64() * rate.get() as f64).round() as u64;
                    g.buf.clear();
                    g.front_frame = frame;
                    g.played_frame = frame;
                    g.eof = false;
                    g.failed = false;
                    drop(g);
                    let _ = tx.send(Msg::SeekDone {
                        voice: v.id,
                        seq,
                        error: None,
                    });
                }
                Err(e) => {
                    drop(g);
                    let msg = take_error().unwrap_or_else(|| {
                        format!("seek to {:.1} s failed: {e}", to.as_secs_f64())
                    });
                    let _ = tx.send(Msg::SeekDone {
                        voice: v.id,
                        seq,
                        error: Some(msg),
                    });
                }
            }
            continue;
        }

        batch.clear();
        let mut end = false;
        while batch.len() < BATCH {
            match dec.next() {
                Some(s) => batch.push(s),
                None => {
                    end = true;
                    break;
                }
            }
        }
        let error = if end { take_error() } else { None };
        let mut g = v.ring.lock_or_recover();
        if g.seek.is_some() {
            continue; // decoded from the old position; the seek clears the ring anyway
        }
        g.buf.extend(batch.iter().copied());
        if end {
            g.eof = true;
            g.failed = error.is_some();
        }
        drop(g);
        if let Some(msg) = error {
            let _ = tx.send(Msg::VoiceError { voice: v.id, msg });
        }
    }
}

/// The mixer side of a voice. Runs on the cpal callback thread: no I/O, no blocking beyond
/// the short ring lock, and messages only on state changes.
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
        let ch = self.channels.get() as usize;
        if self.controls.paused.load(Ordering::Relaxed) {
            self.local.resize(SILENCE_FRAMES * ch, 0.0);
            return Some(());
        }
        let mut g = v.ring.lock_or_recover();
        let n = g.buf.len().min(TAKE / ch * ch);
        if n == 0 {
            if g.eof {
                // Under the ring lock, so a seek (which clears `eof` under it) can't miss
                // that this source is gone and needs replacing.
                let first = !v.ended.swap(true, Ordering::SeqCst);
                let failed = g.failed;
                drop(g);
                if first && !failed {
                    let _ = self.tx.send(Msg::Ended { voice: v.id });
                }
                return None;
            }
            drop(g);
            if v.started.load(Ordering::Relaxed) && !v.starved.swap(true, Ordering::Relaxed) {
                let _ = self.tx.send(Msg::Starved {
                    voice: v.id,
                    on: true,
                });
            }
            self.local.resize(SILENCE_FRAMES * ch, 0.0);
            return Some(());
        }
        self.local.extend(g.buf.drain(..n));
        g.played_frame = g.front_frame;
        g.front_frame += (n / ch) as u64;
        let low = g.buf.len() < (self.rate.get() as usize * ch) / 2;
        drop(g);
        if low {
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
