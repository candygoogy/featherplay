//! The decoder thread: opens the stream, decodes ahead into the ring, and does the seeks and
//! the scans, so the audio thread never waits.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::time::Instant;

use featherplay_core::sync::{CondvarExt, LockExt};
use rodio::Sample;
use tokio::runtime::Runtime;

use super::{BATCH, Controls, UNKNOWN, VoiceShared};
use crate::StreamSource;
use crate::decoder::Decoder;
use crate::engine::Msg;
use crate::head::HeadCache;
use crate::http::ErrorSlot;
use crate::media::{Media, Opened};
use crate::silence::{Skip, SkipSilence, Trimmer};

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
