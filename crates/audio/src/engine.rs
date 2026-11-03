//! The engine thread: owns the output device and the voices, and turns their signals into
//! `AudioEvent`s. API calls send it messages; play, pause and volume are atomics the audio
//! thread reads directly, so they take effect without a round trip.
//!
//! A running output wakes its device thread every few milliseconds even when it only plays
//! silence, so `STOP_AFTER` into a pause the engine stops the output stream; `play()` starts
//! it again. The device stays open, so resuming is as quick as from a short pause.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use featherplay_core::sync::LockExt;
use rodio::cpal;
use tokio::runtime::Runtime;

use crate::devices::{self, Output, is_device_lost};
use crate::head::{self, HeadCache};
use crate::voice::{self, Controls, VoiceShared};
use crate::{AudioEvent, StreamSource, perceptual_gain};

/// How long the ring must stay empty before `Buffering(true)` goes out. Shorter gaps (a seek
/// within downloaded data) would only make the UI flicker.
const BUFFERING_DELAY: Duration = Duration::from_millis(150);
/// How long a paused output keeps running before its stream stops: enough to play out the
/// device buffer as silence, so a resume never replays audio from before the pause.
const STOP_AFTER: Duration = Duration::from_secs(1);

pub(crate) enum Msg {
    Load {
        src: StreamSource,
        start_at: Duration,
        seq: u64,
    },
    Preload(StreamSource),
    Prefetch(String),
    Seek {
        to: Duration,
        seq: u64,
    },
    SetDevice(Option<String>),
    /// Sent by `play()`: start the output, reopening it if a device loss left none.
    EnsureOutput,
    /// Sent by `pause()` and `load()`: stop the output stream after `STOP_AFTER`.
    Paused,
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
    // From the audio thread.
    Started {
        voice: u64,
    },
    Ended {
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
    /// (pass `{cache}/tmp`), created on first use. Fails only when no output device opens.
    /// The engine starts paused at full volume.
    pub fn new(device: Option<&str>, temp_dir: impl Into<PathBuf>) -> Result<AudioEngine, String> {
        Self::start(device, temp_dir.into(), false, crate::http::READ_TIMEOUT)
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
        Self::start(None, temp_dir.into(), true, read_timeout).expect("null engine")
    }

    fn start(
        device: Option<&str>,
        tmp_dir: PathBuf,
        null: bool,
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
                    if let Err(e) = engine.open_output(device) {
                        let _ = ready_tx.send(Err(e));
                        return;
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
    /// of the same URL starts without waiting for the network. Cheap to call again: a URL
    /// already fetched or on its way is left alone. The 8 newest are kept.
    pub fn prefetch(&self, src: &StreamSource) {
        if src.url.starts_with("https://") || src.url.starts_with("http://") {
            self.send(Msg::Prefetch(src.url.clone()));
        }
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
        self.controls.set_gain(perceptual_gain(slider));
    }

    /// Moves playback to the named device (`None` or unknown: the default device), keeping
    /// the current track and position.
    pub fn set_device(&self, name: Option<&str>) {
        self.send(Msg::SetDevice(name.map(str::to_owned)));
    }

    /// Position in the current track; zero when nothing is loaded.
    pub fn position(&self) -> Duration {
        let s = self.status.lock_or_recover();
        match (&s.pending, &s.current) {
            (Some((_, to)), _) => *to,
            (None, Some(v)) => v.position(),
            (None, None) => Duration::ZERO,
        }
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

struct Voice {
    shared: Arc<VoiceShared>,
    /// The decoder has opened the stream and knows the format.
    ready: bool,
    /// Attach to the mixer only after this load seek finished, so no audio from before
    /// `start_at` is heard.
    attach_after: Option<u64>,
}

struct Engine {
    tx: Sender<Msg>,
    controls: Arc<Controls>,
    status: Arc<Mutex<Status>>,
    rt: Arc<Runtime>,
    client: reqwest::Client,
    tmp_dir: PathBuf,
    heads: HeadCache,
    sink: Option<Output>,
    sink_gen: u64,
    device: Option<String>,
    current: Option<Voice>,
    preloaded: Option<Voice>,
    next_voice: u64,
    subscribers: Vec<Sender<AudioEvent>>,
    /// When the current voice's ring ran dry, until `Buffering(true)` is sent.
    starved_since: Option<Instant>,
    buffering_shown: bool,
    /// When the output stream stops, set by `Msg::Paused` and cleared by `play()`.
    stop_at: Option<Instant>,
    /// Tests: a null output instead of a device.
    #[cfg_attr(not(test), allow(dead_code))]
    null: bool,
}

impl Engine {
    fn new(
        tx: Sender<Msg>,
        controls: Arc<Controls>,
        status: Arc<Mutex<Status>>,
        tmp_dir: PathBuf,
        null: bool,
        read_timeout: Duration,
    ) -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("featherplay-audio-net")
            .enable_all()
            .build()
            .map_err(|e| format!("audio runtime: {e}"))?;
        let client =
            crate::http::stream_client(read_timeout).map_err(|e| format!("http client: {e}"))?;
        Ok(Engine {
            tx,
            controls,
            status,
            rt: Arc::new(rt),
            client,
            tmp_dir,
            heads: HeadCache::default(),
            sink: None,
            sink_gen: 0,
            device: None,
            current: None,
            preloaded: None,
            next_voice: 0,
            subscribers: Vec::new(),
            starved_since: None,
            buffering_shown: false,
            stop_at: None,
            null,
        })
    }

    fn run(&mut self, rx: Receiver<Msg>) {
        loop {
            let deadline = self
                .starved_since
                .filter(|_| !self.buffering_shown)
                .map(|t| t + BUFFERING_DELAY);
            let wake = match (deadline, self.stop_at) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let msg = match wake {
                Some(d) => match rx.recv_timeout(d.saturating_duration_since(Instant::now())) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match rx.recv() {
                    Ok(m) => Some(m),
                    Err(_) => break,
                },
            };
            match msg {
                Some(Msg::Shutdown) => break,
                Some(m) => self.handle(m),
                None => {}
            }
            if let Some(d) = deadline
                && Instant::now() >= d
                && !self.buffering_shown
                && self.current.as_ref().is_some_and(|v| v.shared.is_starved())
            {
                self.buffering_shown = true;
                self.emit(AudioEvent::Buffering(true));
            }
            if self.stop_at.is_some_and(|t| Instant::now() >= t) {
                self.stop_at = None;
                if self.controls.paused.load(Ordering::SeqCst)
                    && let Some(sink) = &mut self.sink
                {
                    // Fails only if the stream's thread is gone, which reports its own error.
                    let _ = sink.set_playing(false);
                }
            }
        }
        for v in [self.current.take(), self.preloaded.take()]
            .into_iter()
            .flatten()
        {
            v.shared.stop();
        }
        self.status.lock_or_recover().current = None;
        self.sink = None;
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Load { src, start_at, seq } => self.load(src, start_at, seq),
            Msg::Preload(src) => {
                if self
                    .preloaded
                    .as_ref()
                    .is_some_and(|p| p.shared.url == src.url)
                {
                    return;
                }
                if let Some(old) = self.preloaded.take() {
                    old.shared.stop();
                }
                self.preloaded = Some(self.spawn_voice(src));
            }
            Msg::Prefetch(url) => {
                // A voice for the URL is downloading it already.
                let playing = [&self.current, &self.preloaded]
                    .into_iter()
                    .flatten()
                    .any(|v| v.shared.url == url);
                if !playing {
                    head::prefetch(&self.heads, self.rt.handle(), &self.client, url);
                }
            }
            Msg::Seek { to, seq } => match &mut self.current {
                Some(v) => {
                    // This seek replaces an unfinished load seek; attach after it instead.
                    if v.attach_after.is_some() {
                        v.attach_after = Some(seq);
                    }
                    v.shared.request_seek(to, seq);
                }
                None => self.settle(seq),
            },
            Msg::SetDevice(name) => {
                if let Err(e) = self.open_output(name) {
                    self.emit(AudioEvent::Error(e));
                }
            }
            Msg::EnsureOutput => {
                self.stop_at = None;
                let started = self
                    .sink
                    .as_mut()
                    .is_some_and(|s| s.set_playing(true).is_ok());
                if !started && let Err(e) = self.open_output(self.device.clone()) {
                    self.emit(AudioEvent::Error(e));
                }
            }
            Msg::Paused => {
                self.stop_at
                    .get_or_insert_with(|| Instant::now() + STOP_AFTER);
            }
            Msg::Subscribe(tx) => self.subscribers.push(tx),
            Msg::Shutdown => {}
            Msg::Ready { voice } => {
                for v in [&mut self.current, &mut self.preloaded]
                    .into_iter()
                    .flatten()
                {
                    if v.shared.id == voice {
                        v.ready = true;
                    }
                }
                if self.is_current(voice) {
                    self.attach_current();
                }
            }
            Msg::SeekDone { voice, seq, error } => {
                if !self.is_current(voice) {
                    return;
                }
                self.settle(seq);
                if let Some(e) = error {
                    self.emit(AudioEvent::Error(e));
                }
                // `is_current` held above; a let-else rather than a panic on the engine
                // thread, which would silence playback for good.
                let Some(v) = self.current.as_mut() else {
                    return;
                };
                let after_load = v.attach_after == Some(seq);
                if after_load {
                    v.attach_after = None;
                }
                // A seek after the end revives the voice; its old source left the mixer.
                if after_load || v.shared.take_ended() {
                    self.attach_current();
                }
            }
            Msg::VoiceError { voice, msg } => {
                if self.is_current(voice) {
                    self.status.lock_or_recover().pending = None;
                    self.emit(AudioEvent::Error(msg));
                } else if self
                    .preloaded
                    .as_ref()
                    .is_some_and(|p| p.shared.id == voice)
                {
                    // Dropped quietly; a later load of the URL tries again and reports.
                    self.preloaded = None;
                }
            }
            Msg::Started { voice } => {
                if self.is_current(voice) {
                    self.emit(AudioEvent::Started);
                }
            }
            Msg::Ended { voice } => {
                if self.is_current(voice) {
                    self.clear_buffering();
                    self.emit(AudioEvent::Ended);
                }
            }
            Msg::Starved { voice, on } => {
                if !self.is_current(voice) {
                    return;
                }
                if on {
                    self.starved_since.get_or_insert_with(Instant::now);
                } else {
                    self.clear_buffering();
                }
            }
            Msg::DeviceError { sink, lost } => {
                if !lost || sink != self.sink_gen || self.sink.is_none() {
                    return;
                }
                // Pause, then reopen: the same device if it's back, else the default one.
                self.controls.paused.store(true, Ordering::SeqCst);
                self.sink = None;
                let _ = self.open_output(self.device.clone());
                self.emit(AudioEvent::DeviceLost);
            }
        }
    }

    fn load(&mut self, src: StreamSource, start_at: Duration, seq: u64) {
        self.clear_buffering();
        if let Some(old) = self.current.take() {
            old.shared.stop();
        }
        let mut voice = match self.preloaded.take() {
            Some(p) if p.shared.url == src.url => p,
            other => {
                self.preloaded = other;
                self.spawn_voice(src)
            }
        };
        if start_at.is_zero() {
            // A new or preloaded voice is already at the start.
            self.settle(seq);
        } else {
            voice.attach_after = Some(seq);
            voice.shared.request_seek(start_at, seq);
        }
        self.status.lock_or_recover().current = Some(voice.shared.clone());
        self.current = Some(voice);
        self.attach_current();
    }

    fn spawn_voice(&mut self, src: StreamSource) -> Voice {
        self.next_voice += 1;
        let shared = voice::spawn(
            self.next_voice,
            src,
            self.rt.clone(),
            self.client.clone(),
            self.tmp_dir.clone(),
            self.heads.clone(),
            self.tx.clone(),
        );
        Voice {
            shared,
            ready: false,
            attach_after: None,
        }
    }

    /// Opens the output (see `devices::open_output`), started unless paused, and moves the
    /// current voice onto it.
    fn open_output(&mut self, name: Option<String>) -> Result<(), String> {
        let gen_ = self.sink_gen + 1;
        let tx = self.tx.clone();
        let on_error = move |e: cpal::Error| {
            let _ = tx.send(Msg::DeviceError {
                sink: gen_,
                lost: is_device_lost(&e),
            });
        };
        #[cfg(test)]
        let mut sink = if self.null {
            devices::open_null()
        } else {
            devices::open_output(name.as_deref(), on_error)?
        };
        #[cfg(not(test))]
        let mut sink = devices::open_output(name.as_deref(), on_error)?;
        if !self.controls.paused.load(Ordering::SeqCst) {
            sink.set_playing(true)?;
        }
        self.sink_gen = gen_;
        self.sink = Some(sink);
        self.device = name;
        self.attach_current();
        Ok(())
    }

    /// Adds a source for the current voice to the mixer if it can play, retiring any older one.
    fn attach_current(&mut self) {
        let (Some(sink), Some(v)) = (&self.sink, &self.current) else {
            return;
        };
        if !v.ready || v.attach_after.is_some() {
            return;
        }
        if let Some(src) = v.shared.source(self.controls.clone(), self.tx.clone()) {
            sink.mixer().add(src);
        }
    }

    fn is_current(&self, voice: u64) -> bool {
        self.current.as_ref().is_some_and(|v| v.shared.id == voice)
    }

    /// The load or seek `seq` is done: `position()` reads the voice again.
    fn settle(&self, seq: u64) {
        let mut s = self.status.lock_or_recover();
        if s.pending.is_some_and(|(p, _)| p == seq) {
            s.pending = None;
        }
    }

    fn clear_buffering(&mut self) {
        self.starved_since = None;
        if std::mem::take(&mut self.buffering_shown) {
            self.emit(AudioEvent::Buffering(false));
        }
    }

    fn emit(&mut self, ev: AudioEvent) {
        self.subscribers.retain(|s| s.send(ev.clone()).is_ok());
    }
}
