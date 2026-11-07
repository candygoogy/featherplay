//! Skip silence: drops silence from a voice's decoded samples on the decoder thread, before
//! they reach the ring. The decoder runs about 1.5 s ahead of playback, so nothing is
//! sought and nothing is fetched again; the reader is told how many frames went missing
//! where (`Skip`), so the position stays in track time.
//!
//! The samples go through in 10 ms blocks. A block is silent when its peak is under
//! −60 dBFS. A silent run keeps its first 0.5 s. Past that, its blocks are held back, up to
//! 1.5 s of them: sound coming back sends them on unchanged, and the end of the stream drops
//! them. In `Everywhere`, a run that reaches 2 s drops what it holds and every further block
//! until sound comes back. In `StartAndEnd`, a full hold sends its oldest block on, so only
//! the last 1.5 s of a trailing silence go. The silence a track starts with, from frame 0
//! after a load or a seek to 0 (not after a seek elsewhere), is dropped whole in both modes.

use std::collections::VecDeque;
use std::time::Duration;

use rodio::{ChannelCount, Sample, SampleRate};

/// The "Skip silence" setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkipSilence {
    #[default]
    Off,
    /// The silence before a track starts and after it ends.
    StartAndEnd,
    /// Also silences of 2 s or more inside a track, down to 0.5 s.
    Everywhere,
}

impl SkipSilence {
    pub(crate) fn to_u8(self) -> u8 {
        match self {
            SkipSilence::Off => 0,
            SkipSilence::StartAndEnd => 1,
            SkipSilence::Everywhere => 2,
        }
    }

    pub(crate) fn from_u8(v: u8) -> Self {
        match v {
            1 => SkipSilence::StartAndEnd,
            2 => SkipSilence::Everywhere,
            _ => SkipSilence::Off,
        }
    }
}

/// A block whose peak is under this is silent: −60 dBFS.
const THRESHOLD: Sample = 0.001;
const BLOCK: Duration = Duration::from_millis(10);
/// What a silent run keeps.
const KEEP: Duration = Duration::from_millis(500);
/// A silent run this long, inside a track, is shortened to `KEEP` (`Everywhere`).
const LONG: Duration = Duration::from_secs(2);

/// Frames dropped just before sample `at` of the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Skip {
    pub at: usize,
    pub frames: u64,
}

pub(crate) struct Trimmer {
    channels: usize,
    /// Samples per block.
    block_len: usize,
    /// `KEEP` and `LONG` in frames.
    keep: u64,
    long: u64,
    /// Most samples held back.
    hold_max: usize,
    /// The block being filled.
    block: Vec<Sample>,
    /// Silent blocks past the run's `KEEP`, not sent on yet.
    hold: VecDeque<Sample>,
    /// Nothing but silence since the load: the track's start.
    at_start: bool,
    /// Frames of the current silent run.
    run: u64,
    /// Frames dropped in the drop under way, not told yet.
    dropped: u64,
}

impl Trimmer {
    pub fn new(channels: ChannelCount, rate: SampleRate) -> Self {
        let channels = channels.get() as usize;
        let frames = |d: Duration| (d.as_secs_f64() * rate.get() as f64).round() as u64;
        let block_len = (frames(BLOCK).max(1) as usize) * channels;
        Self {
            channels,
            block_len,
            keep: frames(KEEP),
            long: frames(LONG),
            hold_max: (frames(LONG) - frames(KEEP)) as usize * channels,
            block: Vec::with_capacity(block_len),
            hold: VecDeque::new(),
            at_start: true,
            run: 0,
            dropped: 0,
        }
    }

    /// Nothing held back or part-filled: with the mode off, samples may skip the trimmer.
    pub fn is_idle(&self) -> bool {
        self.block.is_empty() && self.hold.is_empty() && self.dropped == 0
    }

    /// Samples went to the ring without the trimmer (the mode was off): the start is over.
    pub fn passed(&mut self) {
        self.at_start = false;
        self.run = 0;
    }

    /// The decoder moved to `frame`: what's held is from before. The silence at the target
    /// isn't the track's start, unless the target is 0 (Previous restarts a track so).
    pub fn seeked(&mut self, frame: u64) {
        self.block.clear();
        self.hold.clear();
        self.at_start = frame == 0;
        self.run = 0;
        self.dropped = 0;
    }

    /// Appends to `out` what of `input` is heard. `can_drop` is false while the reader
    /// hasn't taken the last `Skip`: no new drop starts then. Returns the drop that ended
    /// in this call, if any.
    pub fn feed(
        &mut self,
        mode: SkipSilence,
        can_drop: bool,
        input: &[Sample],
        out: &mut Vec<Sample>,
    ) -> Option<Skip> {
        let mut skip = None;
        if mode == SkipSilence::Off {
            // The part-block goes on now, so the decoder can pass the trimmer by from the
            // next batch.
            if !self.block.is_empty() {
                self.on_block(mode, can_drop, out, &mut skip);
            }
            if self.dropped > 0 {
                skip = Some(Skip {
                    at: out.len(),
                    frames: self.dropped,
                });
                self.dropped = 0;
            }
            out.extend(self.hold.drain(..));
            out.extend_from_slice(input);
            self.at_start = false;
            self.run = 0;
            return skip;
        }
        let mut rest = input;
        while !rest.is_empty() {
            let take = (self.block_len - self.block.len()).min(rest.len());
            self.block.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.block.len() == self.block_len {
                self.on_block(mode, can_drop, out, &mut skip);
            }
        }
        skip
    }

    /// The stream ended: the last part-block goes through, and the silence held back
    /// (the run reached the end) is dropped.
    pub fn finish(
        &mut self,
        mode: SkipSilence,
        can_drop: bool,
        out: &mut Vec<Sample>,
    ) -> Option<Skip> {
        let mut skip = None;
        if !self.block.is_empty() {
            self.on_block(mode, can_drop, out, &mut skip);
        }
        if mode == SkipSilence::Off {
            out.extend(self.hold.drain(..));
        }
        self.hold.clear();
        self.dropped = 0;
        skip
    }

    fn on_block(
        &mut self,
        mode: SkipSilence,
        can_drop: bool,
        out: &mut Vec<Sample>,
        skip: &mut Option<Skip>,
    ) {
        let block = std::mem::take(&mut self.block);
        let frames = (block.len() / self.channels) as u64;
        let peak = block.iter().fold(0.0, |m: Sample, s| m.max(s.abs()));
        if mode == SkipSilence::Off || peak >= THRESHOLD {
            if self.dropped > 0 {
                debug_assert!(skip.is_none(), "two drops in one call");
                *skip = Some(Skip {
                    at: out.len(),
                    frames: self.dropped,
                });
                self.dropped = 0;
            }
            out.extend(self.hold.drain(..));
            out.extend_from_slice(&block);
            self.at_start = false;
            self.run = 0;
        } else {
            self.run += frames;
            if self.dropped > 0 || (self.at_start && can_drop) {
                self.dropped += frames;
            } else if self.run <= self.keep {
                out.extend_from_slice(&block);
            } else {
                self.hold.extend(&block);
                if mode == SkipSilence::Everywhere && self.run >= self.long && can_drop {
                    self.dropped = (self.hold.len() / self.channels) as u64;
                    self.hold.clear();
                }
                while self.hold.len() > self.hold_max {
                    out.extend(self.hold.drain(..self.block_len.min(self.hold.len())));
                }
            }
        }
        self.block = block;
        self.block.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: usize = 48_000;

    fn trimmer() -> Trimmer {
        Trimmer::new(
            ChannelCount::new(2).unwrap(),
            SampleRate::new(RATE as u32).unwrap(),
        )
    }

    /// `secs` of stereo at `level` (a square wave, so every block peaks at it).
    fn signal(secs: f64, level: Sample) -> Vec<Sample> {
        let frames = (secs * RATE as f64).round() as usize;
        (0..frames)
            .flat_map(|i| {
                let s = if i % 2 == 0 { level } else { -level };
                [s, s]
            })
            .collect()
    }

    fn tone(secs: f64) -> Vec<Sample> {
        signal(secs, 0.5)
    }

    fn silence(secs: f64) -> Vec<Sample> {
        signal(secs, 0.0)
    }

    /// Feeds `input` in decoder-sized batches, then ends the stream.
    fn run(t: &mut Trimmer, mode: SkipSilence, input: &[Sample]) -> (Vec<Sample>, Vec<Skip>) {
        let mut out = Vec::new();
        let mut skips = Vec::new();
        for batch in input.chunks(4096) {
            skips.extend(t.feed(mode, true, batch, &mut out));
        }
        skips.extend(t.finish(mode, true, &mut out));
        (out, skips)
    }

    /// The longest stretch of silent frames in `out`, in seconds.
    fn longest_gap(out: &[Sample]) -> f64 {
        let (mut best, mut cur) = (0, 0);
        for f in out.chunks(2) {
            cur = if f[0] == 0.0 { cur + 1 } else { 0 };
            best = best.max(cur);
        }
        best as f64 / RATE as f64
    }

    /// `out == want`, told in lengths and gaps rather than a million samples.
    #[track_caller]
    fn same(out: &[Sample], want: &[Sample]) {
        assert!(
            out == want,
            "got {:.3} s (longest gap {:.3} s), want {:.3} s (longest gap {:.3} s)",
            out.len() as f64 / 2.0 / RATE as f64,
            longest_gap(out),
            want.len() as f64 / 2.0 / RATE as f64,
            longest_gap(want),
        );
    }

    #[test]
    fn the_silence_a_track_starts_with_is_dropped_and_told() {
        let mut t = trimmer();
        let input = [silence(1.0), tone(1.0)].concat();
        let (out, skips) = run(&mut t, SkipSilence::StartAndEnd, &input);
        same(&out, &tone(1.0));
        assert_eq!(
            skips,
            [Skip {
                at: 0,
                frames: 48_000
            }]
        );
    }

    #[test]
    fn everywhere_shortens_a_long_gap_inside_a_track_to_half_a_second() {
        let mut t = trimmer();
        let input = [tone(1.0), silence(3.0), tone(1.0)].concat();
        let (out, skips) = run(&mut t, SkipSilence::Everywhere, &input);
        same(&out, &[tone(1.0), silence(0.5), tone(1.0)].concat());
        // Dropped after 1.5 s of output: the run past its first 0.5 s, 2.5 s.
        assert_eq!(
            skips,
            [Skip {
                at: 144_000,
                frames: 120_000
            }]
        );
    }

    #[test]
    fn start_and_end_leaves_a_gap_inside_a_track_whole() {
        let mut t = trimmer();
        let input = [tone(1.0), silence(3.0), tone(1.0)].concat();
        let (out, skips) = run(&mut t, SkipSilence::StartAndEnd, &input);
        same(&out, &input);
        assert!(skips.is_empty());
    }

    #[test]
    fn everywhere_leaves_a_gap_under_two_seconds_whole() {
        let mut t = trimmer();
        let input = [tone(1.0), silence(1.9), tone(1.0)].concat();
        let (out, _) = run(&mut t, SkipSilence::Everywhere, &input);
        same(&out, &input);
    }

    #[test]
    fn a_quiet_passage_at_minus_50_dbfs_is_kept() {
        let mut t = trimmer();
        // −50 dBFS is 0.00316.
        let input = [signal(3.0, 0.003_16), tone(1.0), signal(3.0, 0.003_16)].concat();
        let (out, skips) = run(&mut t, SkipSilence::Everywhere, &input);
        same(&out, &input);
        assert!(skips.is_empty());
    }

    #[test]
    fn the_silence_a_track_ends_with_is_cut_to_half_a_second() {
        let mut t = trimmer();
        let input = [tone(1.0), silence(2.0)].concat();
        let (out, _) = run(&mut t, SkipSilence::StartAndEnd, &input);
        same(&out, &[tone(1.0), silence(0.5)].concat());
    }

    #[test]
    fn silence_at_a_seek_target_is_kept() {
        let mut t = trimmer();
        let mut out = Vec::new();
        t.feed(SkipSilence::Everywhere, true, &tone(0.1), &mut out);
        t.seeked(24_000);
        out.clear();
        let mut skips = Vec::new();
        for b in [silence(0.3), tone(0.1)].concat().chunks(4096) {
            skips.extend(t.feed(SkipSilence::Everywhere, true, b, &mut out));
        }
        same(&out, &[silence(0.3), tone(0.1)].concat());
        assert!(skips.is_empty());
    }

    #[test]
    fn a_restart_drops_the_start_again() {
        let mut t = trimmer();
        let mut out = Vec::new();
        t.feed(SkipSilence::StartAndEnd, true, &tone(0.1), &mut out);
        // Previous, or a seek to 0:00: the track starts over.
        t.seeked(0);
        out.clear();
        let mut skips = Vec::new();
        for b in [silence(1.0), tone(0.1)].concat().chunks(4096) {
            skips.extend(t.feed(SkipSilence::StartAndEnd, true, b, &mut out));
        }
        same(&out, &tone(0.1));
        assert_eq!(
            skips,
            [Skip {
                at: 0,
                frames: 48_000
            }]
        );
    }

    #[test]
    fn turned_off_it_holds_nothing_back() {
        let mut t = trimmer();
        let mut out = Vec::new();
        // Less than a block, then the mode goes off.
        t.feed(SkipSilence::Everywhere, true, &tone(0.001), &mut out);
        t.feed(SkipSilence::Off, true, &silence(0.001), &mut out);
        same(&out, &[tone(0.001), silence(0.001)].concat());
        assert!(t.is_idle());
    }

    #[test]
    fn no_drop_starts_while_the_reader_has_not_taken_the_last() {
        let mut t = trimmer();
        let mut out = Vec::new();
        t.feed(SkipSilence::Everywhere, true, &tone(0.1), &mut out);
        let gap = silence(3.0);
        let mut skips = Vec::new();
        for b in gap.chunks(4096) {
            skips.extend(t.feed(SkipSilence::Everywhere, false, b, &mut out));
        }
        for b in tone(0.1).chunks(4096) {
            skips.extend(t.feed(SkipSilence::Everywhere, false, b, &mut out));
        }
        same(&out, &[tone(0.1), silence(3.0), tone(0.1)].concat());
        assert!(skips.is_empty());
        assert!(longest_gap(&out) >= 3.0);
    }

    #[test]
    fn turning_it_off_mid_drop_tells_the_drop_and_passes_the_rest() {
        let mut t = trimmer();
        let mut out = Vec::new();
        let mut skips = Vec::new();
        skips.extend(t.feed(SkipSilence::Everywhere, true, &silence(0.5), &mut out));
        skips.extend(t.feed(SkipSilence::Off, true, &silence(0.2), &mut out));
        assert_eq!(
            skips,
            [Skip {
                at: 0,
                frames: 24_000
            }]
        );
        same(&out, &silence(0.2));
    }

    /// The decoder thread's cost in `Everywhere`: 60 s of 48 kHz stereo noise with a 3 s
    /// gap every 10 s. Run it in release:
    /// `cargo nextest run -p featherplay-audio --release --run-ignored only -E 'test(silence_cpu)' --no-capture`.
    #[test]
    #[ignore = "a measurement; run in release"]
    fn silence_cpu_everywhere() {
        let mut seed = 1_u32;
        let mut noise = |secs: f64| -> Vec<Sample> {
            (0..(secs * RATE as f64) as usize * 2)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 8) as Sample / (1 << 24) as Sample - 0.5
                })
                .collect()
        };
        let input: Vec<Sample> = (0..6)
            .flat_map(|_| [noise(7.0), silence(3.0)].concat())
            .collect();
        let mut t = trimmer();
        let mut out = Vec::with_capacity(8192);
        let start = std::time::Instant::now();
        for batch in input.chunks(4096) {
            out.clear();
            let _ = t.feed(SkipSilence::Everywhere, true, batch, &mut out);
        }
        let per_s = start.elapsed().as_secs_f64() * 1_000.0 / 60.0;
        println!("skip silence: {per_s:.3} ms per second of audio");
        assert!(per_s <= 0.5, "{per_s:.3} ms/s");
    }
}
