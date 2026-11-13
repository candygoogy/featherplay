//! The engine thread itself: `Engine` and the voices it holds (the module docs of
//! `engine`).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use featherplay_core::sync::LockExt;
use rodio::cpal;
use tokio::runtime::Runtime;

use super::{FirstSound, Msg, Status};
use crate::devices::{self, OpenError, Output, is_device_lost};
use crate::glitches::Counts;
use crate::head::{self, HeadCache};
use crate::voice::{self, Controls, DECLICK, Fade, VoiceShared};
use crate::{AudioEvent, StreamSource};

/// How long the ring must stay empty before `Buffering(true)` goes out. Shorter gaps (a seek
/// within downloaded data) would only make the UI flicker.
const BUFFERING_DELAY: Duration = Duration::from_millis(150);
/// How long a paused output keeps running before its stream stops: enough to play out the
/// device buffer as silence, so a resume never replays audio from before the pause.
const STOP_AFTER: Duration = Duration::from_secs(1);
/// An output stopped this long may have let its device go idle (it did after 8-15 s,
/// 2026-11-10), so `wake()` starts it ahead of a play. Stopped for less, a play starts it
/// at once anyway.
#[cfg(not(test))]
const WAKE_AFTER: Duration = Duration::from_secs(5);
#[cfg(test)]
const WAKE_AFTER: Duration = Duration::from_millis(300);
/// Whether an output stopped for `WAKE_AFTER` or more is opened afresh rather than started
/// again. On Linux, ALSA's stream through PulseAudio, paused that long, took 5-11 s to give
/// its first callback when resumed (WSLg, 2026-11-12), where a new one starts in ~30 ms.
const REOPEN_IDLE: bool = cfg!(target_os = "linux");
/// Tracks fading out at once at most: a third (skips in a row, each fading) stops the
/// oldest, which is the quietest.
const MAX_FADING: usize = 2;
/// URLs `silences_known` remembers: the current track, the next, and a skip or two.
const KNOWN: usize = 4;

struct Voice {
    shared: Arc<VoiceShared>,
    /// The decoder has opened the stream and knows the format.
    ready: bool,
    /// Attach to the mixer only after this load seek finished, so no audio from before
    /// `start_at` is heard.
    attach_after: Option<u64>,
    /// When it was loaded (or crossfaded to), until its first sound is logged.
    loaded: Option<Instant>,
}

pub(super) struct Engine {
    tx: Sender<Msg>,
    controls: Arc<Controls>,
    status: Arc<Mutex<Status>>,
    rt: Arc<Runtime>,
    /// Made again when "Look up YouTube directly" changes, so no connection kept from the
    /// old route is taken again (`featherplay_proxy::Fresh`).
    http: featherplay_proxy::Fresh,
    tmp_dir: PathBuf,
    heads: HeadCache,
    /// The connection being opened ahead of a stream (`AudioEngine::connect`), if any.
    connecting: Option<tokio::task::JoinHandle<()>>,
    sink: Option<Output>,
    sink_gen: u64,
    pub(super) device: Option<String>,
    current: Option<Voice>,
    preloaded: Option<Voice>,
    /// Tracks fading out after a crossfade, oldest first; `MAX_FADING` at most.
    fading: Vec<Voice>,
    /// A preload asked for during a fade, started once the fade is over, so a fade never
    /// holds a third ring.
    deferred_preload: Option<StreamSource>,
    next_voice: u64,
    subscribers: Vec<Sender<AudioEvent>>,
    /// When the current voice's ring ran dry, until `Buffering(true)` is sent.
    starved_since: Option<Instant>,
    buffering_shown: bool,
    /// When the output stream stops, set by `Msg::Paused` and cleared by `play()`.
    stop_at: Option<Instant>,
    /// The latest `play()`: a track loaded while paused counts its first sound from there.
    played: Option<Instant>,
    /// The glitch counters at the last report.
    reported: Counts,
    /// The URLs `silences_known` was told last, newest last; `KNOWN` at most.
    known: VecDeque<String>,
    /// Tests: a null output instead of a device, there while the flag is set. `None` opens
    /// the real devices.
    #[cfg_attr(not(test), allow(dead_code))]
    null: Option<Arc<AtomicBool>>,
}

impl Engine {
    pub(super) fn new(
        tx: Sender<Msg>,
        controls: Arc<Controls>,
        status: Arc<Mutex<Status>>,
        tmp_dir: PathBuf,
        null: Option<Arc<AtomicBool>>,
        read_timeout: Duration,
    ) -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("featherplay-audio-net")
            .enable_all()
            .build()
            .map_err(|e| format!("audio runtime: {e}"))?;
        // With no client to be had, the engine still starts: a stream fails as with no
        // network, and local files play.
        let http =
            featherplay_proxy::Fresh::or_offline(move || crate::http::stream_client(read_timeout));
        Ok(Engine {
            tx,
            controls,
            status,
            rt: Arc::new(rt),
            http,
            tmp_dir,
            heads: HeadCache::default(),
            connecting: None,
            sink: None,
            sink_gen: 0,
            device: None,
            current: None,
            preloaded: None,
            fading: Vec::new(),
            deferred_preload: None,
            next_voice: 0,
            subscribers: Vec::new(),
            starved_since: None,
            buffering_shown: false,
            stop_at: None,
            played: None,
            reported: Counts::default(),
            known: VecDeque::new(),
            null,
        })
    }

    pub(super) fn run(&mut self, rx: Receiver<Msg>) {
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
                if self.controls.paused.load(Ordering::SeqCst) {
                    self.stop_output();
                }
            }
        }
        for v in [self.current.take(), self.preloaded.take()]
            .into_iter()
            .flatten()
            .chain(self.fading.drain(..))
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
                if !self.fading.is_empty() {
                    self.deferred_preload = Some(src);
                } else {
                    self.preload(src);
                }
            }
            Msg::Crossfade { src, fade, seq } => self.crossfade(src, fade, seq),
            Msg::FadeOut(fade) => {
                if let Some(old) = self.current.take() {
                    self.clear_buffering();
                    self.status.lock_or_recover().current = None;
                    self.fade_away(old, fade);
                }
            }
            Msg::Prefetch(url) => {
                // A voice for the URL is downloading it already.
                let playing = [&self.current, &self.preloaded]
                    .into_iter()
                    .flatten()
                    .chain(&self.fading)
                    .any(|v| v.shared.url == url);
                if !playing {
                    let tx = self.tx.clone();
                    let refused = move |url| {
                        // Fails only if the engine thread is gone.
                        let _ = tx.send(Msg::HeadRefused(url));
                    };
                    head::prefetch(
                        &self.heads,
                        self.rt.handle(),
                        &self.http.client(),
                        url,
                        refused,
                    );
                }
            }
            Msg::Connect(url) => {
                // One at a time: a second one beside it would open a second connection.
                if self.connecting.as_ref().is_some_and(|c| !c.is_finished()) {
                    return;
                }
                let client = self.http.client();
                self.connecting = Some(self.rt.spawn(async move {
                    let t = Instant::now();
                    match crate::http::connect(&client, &url).await {
                        Ok(()) => log::info!(
                            "audio: connected ahead of the stream in {} ms",
                            t.elapsed().as_millis()
                        ),
                        Err(e) => log::info!("audio: no connection ahead of the stream: {e}"),
                    }
                }));
            }
            // Told like a refused preload: the app resolves the stream again.
            Msg::HeadRefused(url) => self.emit(AudioEvent::PreloadRefused(url)),
            Msg::SilencesKnown(url) => {
                for v in [&self.current, &self.preloaded].into_iter().flatten() {
                    if v.shared.url == url {
                        v.shared.silences_known();
                    }
                }
                self.known.retain(|u| *u != url);
                if self.known.len() >= KNOWN {
                    self.known.pop_front();
                }
                self.known.push_back(url);
            }
            Msg::Silences { voice } => {
                if self.is_current(voice) {
                    self.tell_silences();
                }
            }
            Msg::Seek { to, seq } => {
                if !self.fading.is_empty() {
                    // A seek ends the fade: the old track goes, the new one comes up.
                    self.end_fades();
                    if let Some(v) = &self.current {
                        v.shared.fade(Fade::In, DECLICK);
                    }
                }
                match &mut self.current {
                    Some(v) => {
                        // This seek replaces an unfinished load seek; attach after it instead.
                        if v.attach_after.is_some() {
                            v.attach_after = Some(seq);
                        }
                        v.shared.request_seek(to, seq);
                    }
                    None => self.settle(seq),
                }
            }
            Msg::SetDevice(name) => {
                if let Err(e) = self.open_output(name) {
                    self.emit(AudioEvent::Error(e.to_string()));
                }
            }
            Msg::EnsureOutput => {
                self.stop_at = None;
                self.played = Some(Instant::now());
                let reopen = REOPEN_IDLE && self.stopped_long();
                let started = !reopen
                    && self
                        .sink
                        .as_mut()
                        .is_some_and(|s| s.set_playing(true).is_ok());
                if started {
                    return;
                }
                match self.open_output(self.device.clone()) {
                    Ok(()) => {}
                    Err(OpenError::NoDevice) => {
                        // Nothing to hear it on, so it doesn't play on silently.
                        self.controls.paused.store(true, Ordering::SeqCst);
                        self.emit(AudioEvent::NoDevice);
                    }
                    Err(e) => {
                        // A new one wouldn't open: the old one, late, is better than none.
                        let resumed = reopen
                            && self
                                .sink
                                .as_mut()
                                .is_some_and(|s| s.set_playing(true).is_ok());
                        if !resumed {
                            self.emit(AudioEvent::Error(e.to_string()));
                        }
                    }
                }
            }
            Msg::Paused => {
                self.report_glitches();
                self.stop_at
                    .get_or_insert_with(|| Instant::now() + STOP_AFTER);
            }
            Msg::Wake => {
                let idle = self.stopped_long();
                let paused = self.controls.paused.load(Ordering::SeqCst);
                if REOPEN_IDLE {
                    // A new stream, stopped while paused, starts at once on the play.
                    if idle && paused {
                        let _ = self.open_output(self.device.clone());
                    }
                } else if idle
                    && paused
                    && let Some(sink) = &mut self.sink
                    && sink.set_playing(true).is_ok()
                {
                    self.stop_at = Some(Instant::now() + STOP_AFTER);
                }
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
                if self.drop_fading(voice) {
                    return;
                }
                if self.is_current(voice) {
                    self.status.lock_or_recover().pending = None;
                    self.emit(AudioEvent::Error(msg));
                } else if self
                    .preloaded
                    .as_ref()
                    .is_some_and(|p| p.shared.id == voice)
                {
                    // Dropped; a later load of the URL tries again and reports. A refused
                    // URL is told now, so the app can resolve it again before that load.
                    if let Some(p) = self.preloaded.take()
                        && msg.contains("HTTP 403 at byte 0")
                    {
                        self.emit(AudioEvent::PreloadRefused(p.shared.url.clone()));
                    }
                }
            }
            Msg::Started { voice } => {
                if self.is_current(voice) {
                    self.log_first_sound();
                    self.emit(AudioEvent::Started);
                }
            }
            Msg::Skipped { voice } => {
                if self.is_current(voice) {
                    self.emit(AudioEvent::Skipped);
                }
            }
            Msg::Ended { voice } => {
                if self.is_current(voice) {
                    self.clear_buffering();
                    self.report_glitches();
                    self.emit(AudioEvent::Ended);
                } else {
                    // A fading track's stream ran out before its fade did.
                    self.drop_fading(voice);
                }
            }
            Msg::FadedOut { voice } => {
                self.drop_fading(voice);
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
                self.end_fades();
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
        self.end_fades();
        let mut voice = match self.preloaded.take() {
            Some(p) if p.shared.url == src.url => p,
            other => {
                self.preloaded = other;
                self.spawn_voice(src)
            }
        };
        voice.loaded = Some(Instant::now());
        if start_at.is_zero() {
            // A new or preloaded voice is already at the start.
            self.settle(seq);
        } else {
            voice.shared.ramp_in_first();
            voice.attach_after = Some(seq);
            voice.shared.request_seek(start_at, seq);
        }
        self.status.lock_or_recover().current = Some(voice.shared.clone());
        self.current = Some(voice);
        self.attach_current();
        self.tell_silences();
    }

    fn preload(&mut self, src: StreamSource) {
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

    /// `src` becomes current from its start, fading in over `fade`, while the current track
    /// fades out. The preloaded voice is used if it's `src`.
    fn crossfade(&mut self, src: StreamSource, fade: Duration, seq: u64) {
        self.clear_buffering();
        if let Some(old) = self.current.take() {
            self.fade_away(old, fade);
        }
        // A preload waiting for the fade to end may be this track (Fade on skip): it's
        // spawned here instead.
        if self
            .deferred_preload
            .as_ref()
            .is_some_and(|d| d.url == src.url)
        {
            self.deferred_preload = None;
        }
        let mut voice = match self.preloaded.take() {
            Some(p) if p.shared.url == src.url => p,
            other => {
                self.preloaded = other;
                self.spawn_voice(src)
            }
        };
        voice.loaded = Some(Instant::now());
        // Not attached yet (a preloaded voice never is), so it starts from silence.
        voice.shared.start_silent();
        voice.shared.fade(Fade::In, fade);
        self.settle(seq);
        self.status.lock_or_recover().current = Some(voice.shared.clone());
        self.current = Some(voice);
        self.attach_current();
        self.tell_silences();
    }

    /// The current voice's silences, once, when its scan has found them (a preloaded
    /// voice's may have before it became current).
    fn tell_silences(&mut self) {
        let Some(v) = &self.current else { return };
        if let Some(silences) = v.shared.take_silences() {
            let ev = AudioEvent::Silences {
                url: v.shared.url.clone(),
                silences: silences.clone(),
            };
            self.emit(ev);
        }
    }

    /// `v` is no longer current: it fades out over `fade` from where it is, or stops now if
    /// it isn't sounding. Tracks already fading are gone within `fade` too.
    fn fade_away(&mut self, v: Voice, fade: Duration) {
        let sounding =
            self.sink.is_some() && v.ready && v.attach_after.is_none() && !v.shared.has_ended();
        if !sounding {
            v.shared.stop();
            return;
        }
        if self.fading.len() >= MAX_FADING {
            // Stopping ramps it out over a few ms.
            self.fading.remove(0).shared.stop();
        }
        for f in &self.fading {
            f.shared.fade_out_within(fade);
        }
        v.shared.fade(Fade::Out, fade);
        self.fading.push(v);
    }

    /// Stops the tracks fading out (each ramps out over a few ms), then starts a preload
    /// that waited for them.
    fn end_fades(&mut self) {
        for v in self.fading.drain(..) {
            v.shared.stop();
        }
        if let Some(src) = self.deferred_preload.take() {
            self.preload(src);
        }
    }

    /// Stops and forgets the fading voice `id`; false if it isn't one.
    fn drop_fading(&mut self, id: u64) -> bool {
        let Some(i) = self.fading.iter().position(|v| v.shared.id == id) else {
            return false;
        };
        let v = self.fading.remove(i);
        v.shared.stop();
        // Its start is kept in memory, as for any track left (the app's `prefetch` skipped
        // it while it played), so Previous back to it starts at once.
        let url = v.shared.url.clone();
        if url.starts_with("https://") || url.starts_with("http://") {
            self.handle(Msg::Prefetch(url));
        }
        if self.fading.is_empty()
            && let Some(src) = self.deferred_preload.take()
        {
            self.preload(src);
        }
        true
    }

    fn spawn_voice(&mut self, src: StreamSource) -> Voice {
        self.next_voice += 1;
        let known = self.known.contains(&src.url);
        let shared = voice::spawn(
            self.next_voice,
            src,
            voice::Fetch {
                rt: self.rt.clone(),
                client: self.http.client(),
                tmp_dir: self.tmp_dir.clone(),
                heads: self.heads.clone(),
            },
            self.controls.clone(),
            self.tx.clone(),
            known,
        );
        Voice {
            shared,
            ready: false,
            attach_after: None,
            loaded: None,
        }
    }

    /// Stops the paused output's stream. On Linux it's replaced by a new one that isn't started,
    /// which waits on nothing: a stream paused with cpal's ALSA pause (`snd_pcm_pause`) keeps
    /// its thread polling the device, and where the sound server had stalled the stream
    /// (WSLg's, 2026-11-13) that poll spun at 15 % of a core for as long as the pause lasted.
    /// Through a server that doesn't stall it stayed idle either way (the sound rig). The new
    /// stream starts on the play as any does, as one opened after `WAKE_AFTER` already did;
    /// failing to open one, the old one is paused as elsewhere.
    fn stop_output(&mut self) {
        if REOPEN_IDLE && self.sink.is_some() && self.open_output(self.device.clone()).is_ok() {
            return;
        }
        if let Some(sink) = &mut self.sink {
            // Fails only if the stream's thread is gone, which reports its own error.
            let _ = sink.set_playing(false);
        }
    }

    /// The output has been stopped for `WAKE_AFTER` or more.
    fn stopped_long(&self) -> bool {
        self.sink
            .as_ref()
            .and_then(Output::stopped_for)
            .is_some_and(|d| d >= WAKE_AFTER)
    }

    /// Opens the output (see `devices::open_output`), started unless paused, and moves the
    /// current voice onto it.
    pub(super) fn open_output(&mut self, name: Option<String>) -> Result<(), OpenError> {
        let gen_ = self.sink_gen + 1;
        let tx = self.tx.clone();
        let on_error = move |e: cpal::Error| {
            let _ = tx.send(Msg::DeviceError {
                sink: gen_,
                lost: is_device_lost(&e),
            });
        };
        #[cfg(test)]
        let mut sink = match &self.null {
            Some(plugged) if plugged.load(Ordering::SeqCst) => devices::open_null(&self.controls),
            Some(_) => return Err(OpenError::NoDevice),
            None => devices::open_output(name.as_deref(), on_error, &self.controls)?,
        };
        #[cfg(not(test))]
        let mut sink = devices::open_output(name.as_deref(), on_error, &self.controls)?;
        // The fading tracks' sources were on the old output.
        self.end_fades();
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

    /// Logs how the current track's first sound came, once per load.
    fn log_first_sound(&mut self) {
        let Some(v) = self.current.as_mut() else {
            return;
        };
        let Some(loaded) = v.loaded.take() else {
            return;
        };
        let from = self.played.map_or(loaded, |p| p.max(loaded));
        let steps = FirstSound::of(&v.shared.stages, from, Instant::now());
        log::info!("audio: {steps}");
        #[cfg(test)]
        {
            self.status.lock_or_recover().first_sound = Some(steps);
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

    /// Logs the late callbacks and dry rings since the last report, if there were any. On
    /// a pause and at a track's end, so a glitch shows up in the log without a timer.
    fn report_glitches(&mut self) {
        let g = &self.controls.glitches;
        let now = g.snapshot();
        let late = now.late - self.reported.late;
        let starved = now.starved - self.reported.starved;
        if late > 0 || starved > 0 {
            log::info!(
                "audio: {late} late output callback(s), longest gap {} ms; ring ran dry {starved} time(s)",
                now.max_gap.as_millis()
            );
        }
        self.reported = now;
        g.reset_max_gap();
    }

    fn emit(&mut self, ev: AudioEvent) {
        self.subscribers.retain(|s| s.send(ev.clone()).is_ok());
    }
}
