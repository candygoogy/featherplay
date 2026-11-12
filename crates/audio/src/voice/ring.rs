//! The ring's slots and the decoder thread's side of them, with the seqlock a seek publishes
//! (the module docs of `voice`).

use std::sync::atomic::{AtomicU32, Ordering, fence};

use rodio::{ChannelCount, Sample, SampleRate};

use super::{NO_END, NO_SKIP, RING, VoiceShared};

/// Rings allocated and not freed yet, in this process. For tests.
#[cfg(test)]
static RINGS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many voices hold a ring now. For tests.
#[cfg(test)]
pub(crate) fn rings() -> usize {
    RINGS.load(Ordering::SeqCst)
}

/// The ring's slots: `f32` bits in atomics, so neither side needs `unsafe` to share them.
pub(super) struct Slots(Box<[AtomicU32]>);

impl Slots {
    fn new(len: usize) -> Self {
        Self((0..len.max(1)).map(|_| AtomicU32::new(0)).collect())
    }
    pub(super) fn len(&self) -> u64 {
        self.0.len() as u64
    }
    /// The slot of sample `index`.
    pub(super) fn at(&self, index: u64) -> &AtomicU32 {
        // In range: `index % len` < len.
        &self.0[(index % self.len()) as usize]
    }
}

impl VoiceShared {
    // The decoder thread's side.

    /// Sets the format and makes the ring, ~`RING` of it.
    pub(super) fn set_format(&self, channels: ChannelCount, rate: SampleRate) {
        let frames = (RING.as_secs_f64() * rate.get() as f64) as usize;
        let _ = self.slots.set(Slots::new(frames * channels.get() as usize));
        #[cfg(test)]
        RINGS.fetch_add(1, Ordering::SeqCst);
        let _ = self.format.set((channels, rate));
    }

    /// Samples the ring has room for, counting those before the latest seek as free.
    pub(super) fn room(&self, slots: &Slots) -> u64 {
        let read = self
            .read
            .load(Ordering::Acquire)
            .max(self.seek_start.load(Ordering::Relaxed));
        slots.len() - (self.write.load(Ordering::Relaxed) - read)
    }

    /// Appends `samples`; the caller checked `room`.
    pub(super) fn push(&self, slots: &Slots, samples: &[Sample]) {
        let w = self.write.load(Ordering::Relaxed);
        for (i, s) in samples.iter().enumerate() {
            slots.at(w + i as u64).store(s.to_bits(), Ordering::Relaxed);
        }
        self.write
            .store(w + samples.len() as u64, Ordering::Release);
    }

    /// The reader has taken the last skip, so the decoder may drop again.
    pub(super) fn can_skip(&self) -> bool {
        self.skip_at.load(Ordering::Acquire) == NO_SKIP
    }

    /// `frames` were dropped just before the next sample pushed; the caller checked
    /// `can_skip`.
    pub(super) fn mark_skip(&self, frames: u64) {
        self.skip_frames.store(frames, Ordering::Relaxed);
        self.skip_at
            .store(self.write.load(Ordering::Relaxed), Ordering::Release);
    }

    /// The stream ends after the samples written so far.
    pub(super) fn mark_end(&self, failed: bool) {
        self.failed.store(failed, Ordering::Relaxed);
        self.end
            .store(self.write.load(Ordering::Relaxed), Ordering::Release);
    }

    /// The decoder moved to `frame`: what's in the ring is from before it.
    pub(super) fn publish_seek(&self, frame: u64) {
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use featherplay_core::sync::LockExt;

    use super::super::BATCH;
    use super::super::tests::{playing, voice};
    use super::*;
    use crate::engine::Msg;

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
}
