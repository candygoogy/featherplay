//! `RingSource`, the mixer side of a voice, and its gains.

use std::sync::Arc;
use std::sync::atomic::{Ordering, fence};
use std::sync::mpsc::Sender;
use std::time::Duration;

use rodio::{ChannelCount, Sample, SampleRate, Source};

use super::{Controls, DECLICK, FULL, LEVEL_RAMP, NO_SKIP, SILENCE_FRAMES, TAKE, VoiceShared};
use crate::engine::Msg;
use crate::stretch::Stretch;

impl VoiceShared {
    /// The level a source plays at now: the song's own while "Match song volumes" is on.
    fn level_for(&self, controls: &Controls) -> f32 {
        if controls.match_volumes.load(Ordering::Relaxed) {
            f32::from_bits(self.level.load(Ordering::Relaxed))
        } else {
            1.0
        }
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

    use super::super::Fade;
    use super::super::tests::{playing, voice};
    use super::*;

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
