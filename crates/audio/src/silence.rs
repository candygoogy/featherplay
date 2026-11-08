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
use std::ops::Range;
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
pub(crate) const THRESHOLD: Sample = 0.001;
pub(crate) const BLOCK: Duration = Duration::from_millis(10);
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

/// The sizes above at one sample rate, in frames.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sizes {
    pub block: u64,
    keep: u64,
    long: u64,
    /// Most frames held back.
    hold_max: u64,
}

impl Sizes {
    pub fn new(rate: SampleRate) -> Self {
        let frames = |d: Duration| (d.as_secs_f64() * f64::from(rate.get())).round() as u64;
        Sizes {
            block: frames(BLOCK).max(1),
            keep: frames(KEEP),
            long: frames(LONG),
            hold_max: frames(LONG) - frames(KEEP),
        }
    }
}

/// What a block does (`Gate::block`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    /// Sound: what's held goes on, then the block. `told` frames were dropped just before it.
    Sound { told: u64 },
    /// Silence that's heard.
    Pass,
    /// Silence that's dropped.
    Drop,
    /// Held back; then `release` frames from the front of the hold go on.
    Hold { release: u64 },
    /// Held back, then the whole hold, this block with it, is dropped.
    DropHeld,
}

/// Skip silence's decisions, one block at a time, from the block's length and whether it's
/// silent alone. The trimmer moves samples by them, and `Silences::drops` replays them over
/// a track's silent runs, so the marks on the bar are what playback drops.
#[derive(Clone, Debug)]
struct Gate {
    sizes: Sizes,
    /// Nothing but silence since the load: the track's start.
    at_start: bool,
    /// Frames of the current silent run.
    run: u64,
    /// Frames dropped in the drop under way, not told yet.
    dropped: u64,
    /// Frames held back.
    held: u64,
}

impl Gate {
    fn new(sizes: Sizes) -> Self {
        Gate {
            sizes,
            at_start: true,
            run: 0,
            dropped: 0,
            held: 0,
        }
    }

    fn block(&mut self, mode: SkipSilence, can_drop: bool, frames: u64, silent: bool) -> Act {
        if mode == SkipSilence::Off || !silent {
            let told = std::mem::take(&mut self.dropped);
            self.held = 0;
            self.at_start = false;
            self.run = 0;
            return Act::Sound { told };
        }
        self.run += frames;
        if self.dropped > 0 || (self.at_start && can_drop) {
            self.dropped += frames;
            return Act::Drop;
        }
        if self.run <= self.sizes.keep {
            return Act::Pass;
        }
        self.held += frames;
        if mode == SkipSilence::Everywhere && self.run >= self.sizes.long && can_drop {
            self.dropped = std::mem::take(&mut self.held);
            return Act::DropHeld;
        }
        let mut release = 0;
        while self.held > self.sizes.hold_max {
            let r = self.sizes.block.min(self.held);
            self.held -= r;
            release += r;
        }
        Act::Hold { release }
    }

    /// The mode went off: what's held goes on. Returns the frames dropped not told yet.
    fn off(&mut self) -> u64 {
        self.held = 0;
        self.at_start = false;
        self.run = 0;
        std::mem::take(&mut self.dropped)
    }

    /// The end of the stream: what's held, or being dropped, goes. Returns how much.
    fn finish(&mut self) -> u64 {
        std::mem::take(&mut self.held) + std::mem::take(&mut self.dropped)
    }
}

pub(crate) struct Trimmer {
    channels: usize,
    /// Samples per block.
    block_len: usize,
    gate: Gate,
    /// The block being filled.
    block: Vec<Sample>,
    /// Silent blocks past the run's `KEEP`, not sent on yet: `gate.held` frames.
    hold: VecDeque<Sample>,
}

impl Trimmer {
    pub fn new(channels: ChannelCount, rate: SampleRate) -> Self {
        let channels = channels.get() as usize;
        let sizes = Sizes::new(rate);
        let block_len = sizes.block as usize * channels;
        Self {
            channels,
            block_len,
            gate: Gate::new(sizes),
            block: Vec::with_capacity(block_len),
            hold: VecDeque::new(),
        }
    }

    /// Nothing held back or part-filled: with the mode off, samples may skip the trimmer.
    pub fn is_idle(&self) -> bool {
        self.block.is_empty() && self.hold.is_empty() && self.gate.dropped == 0
    }

    /// Samples went to the ring without the trimmer (the mode was off): the start is over.
    pub fn passed(&mut self) {
        self.gate.at_start = false;
        self.gate.run = 0;
    }

    /// The decoder moved to `frame`: what's held is from before. The silence at the target
    /// isn't the track's start, unless the target is 0 (Previous restarts a track so).
    pub fn seeked(&mut self, frame: u64) {
        self.block.clear();
        self.hold.clear();
        self.gate = Gate {
            at_start: frame == 0,
            ..Gate::new(self.gate.sizes)
        };
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
            let told = self.gate.off();
            if told > 0 {
                skip = Some(Skip {
                    at: out.len(),
                    frames: told,
                });
            }
            out.extend(self.hold.drain(..));
            out.extend_from_slice(input);
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
        self.gate.finish();
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
        match self.gate.block(mode, can_drop, frames, is_silent(&block)) {
            Act::Sound { told } => {
                if told > 0 {
                    debug_assert!(skip.is_none(), "two drops in one call");
                    *skip = Some(Skip {
                        at: out.len(),
                        frames: told,
                    });
                }
                out.extend(self.hold.drain(..));
                out.extend_from_slice(&block);
            }
            Act::Pass => out.extend_from_slice(&block),
            Act::Drop => {}
            Act::Hold { release } => {
                self.hold.extend(&block);
                out.extend(self.hold.drain(..release as usize * self.channels));
            }
            Act::DropHeld => self.hold.clear(),
        }
        self.block = block;
        self.block.clear();
    }
}

/// A block whose peak is under `THRESHOLD`.
fn is_silent(block: &[Sample]) -> bool {
    block.iter().fold(0.0, |m: Sample, s| m.max(s.abs())) < THRESHOLD
}

/// Where a track is silent, from a scan of all of it (`Scan`): its runs of silent blocks,
/// whatever the mode. `spans` gives what a mode drops.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Silences {
    /// The rate the frames count at.
    pub rate: u32,
    /// The track's length, in frames.
    pub frames: u64,
    /// The silent runs, as (first frame, frames): in order, apart, each a whole number of
    /// blocks from the track's start (the last may end with a part-block).
    pub runs: Vec<(u64, u64)>,
}

impl Silences {
    /// What a scan could have found: a rate of 8 to 384 kHz, at most 12 hours, the runs in
    /// order, apart and within the track. Anything else (a damaged cache) has no spans.
    pub fn is_plausible(&self) -> bool {
        const MOST: u64 = 12 * 3600;
        if !(8_000..=384_000).contains(&self.rate) || self.frames > u64::from(self.rate) * MOST {
            return false;
        }
        let mut pos = 0;
        self.runs.iter().all(|&(start, len)| {
            let ok = start >= pos && len > 0 && start.saturating_add(len) <= self.frames;
            pos = start.saturating_add(len);
            ok
        })
    }

    /// What `mode` drops from the track played from its start, in track time. Playback
    /// drops the same, give or take a few blocks after a seek or right after another drop
    /// (the trimmer holds a new drop until the last one is heard).
    pub fn spans(&self, mode: SkipSilence) -> Vec<Range<Duration>> {
        let time = |f: u64| Duration::from_secs_f64(f as f64 / f64::from(self.rate));
        self.drops(mode)
            .into_iter()
            .map(|r| time(r.start)..time(r.end))
            .collect()
    }

    /// What `mode` drops from the track played from its start, in frames: the gate's
    /// decisions replayed over the runs, a block at a time, with the sound between them
    /// as one block (any sound block leaves the gate the same). Nothing unless
    /// `is_plausible`.
    pub(crate) fn drops(&self, mode: SkipSilence) -> Vec<Range<u64>> {
        let mut drops = Vec::new();
        let Some(rate) = SampleRate::new(self.rate) else {
            return drops;
        };
        if mode == SkipSilence::Off || !self.is_plausible() {
            return drops;
        }
        let sizes = Sizes::new(rate);
        let mut gate = Gate::new(sizes);
        // Frames gone through the gate, and where the drop under way began.
        let mut pos = 0;
        let mut open: Option<u64> = None;
        let mut sound = |gate: &mut Gate, pos: &mut u64, to: u64, open: &mut Option<u64>| {
            if to > *pos {
                gate.block(mode, true, to - *pos, false);
                if let Some(from) = open.take() {
                    drops.push(from..*pos);
                }
                *pos = to;
            }
        };
        for &(start, len) in &self.runs {
            let start = start.clamp(pos, self.frames);
            let end = start.saturating_add(len).min(self.frames);
            sound(&mut gate, &mut pos, start, &mut open);
            while pos < end {
                let n = sizes.block.min(end - pos);
                match gate.block(mode, true, n, true) {
                    Act::Drop => {
                        open.get_or_insert(pos);
                    }
                    Act::DropHeld => {
                        open.get_or_insert((pos + n).saturating_sub(gate.dropped));
                    }
                    Act::Sound { .. } | Act::Pass | Act::Hold { .. } => {}
                }
                pos += n;
            }
        }
        sound(&mut gate, &mut pos, self.frames, &mut open);
        // The end of the stream drops what's held, or the drop under way.
        let tail = gate.finish();
        if tail > 0 {
            let from = open.unwrap_or(self.frames.saturating_sub(tail));
            drops.push(from..self.frames);
        }
        drops
    }
}

/// Finds a track's silent runs, block by block as the trimmer sees them, from its decoded
/// samples fed in order from the start. Holds no samples.
pub(crate) struct Scan {
    channels: usize,
    rate: u32,
    /// Samples per block.
    block_len: usize,
    /// Samples of the block being read, and its peak so far.
    filled: usize,
    peak: Sample,
    /// Frames read in whole blocks.
    pos: u64,
    /// Where the silent run under way began.
    run: Option<u64>,
    runs: Vec<(u64, u64)>,
}

impl Scan {
    pub fn new(channels: ChannelCount, rate: SampleRate) -> Self {
        let channels = channels.get() as usize;
        Scan {
            channels,
            rate: rate.get(),
            block_len: Sizes::new(rate).block as usize * channels,
            filled: 0,
            peak: 0.0,
            pos: 0,
            run: None,
            runs: Vec::new(),
        }
    }

    pub fn feed(&mut self, samples: &[Sample]) {
        let mut rest = samples;
        while !rest.is_empty() {
            let take = (self.block_len - self.filled).min(rest.len());
            self.peak = rest[..take]
                .iter()
                .fold(self.peak, |m: Sample, s| m.max(s.abs()));
            self.filled += take;
            rest = &rest[take..];
            if self.filled == self.block_len {
                self.end_block();
            }
        }
    }

    pub fn finish(mut self) -> Silences {
        if self.filled > 0 {
            self.end_block();
        }
        if let Some(start) = self.run.take() {
            self.runs.push((start, self.pos - start));
        }
        Silences {
            rate: self.rate,
            frames: self.pos,
            runs: self.runs,
        }
    }

    fn end_block(&mut self) {
        if self.peak < THRESHOLD {
            self.run.get_or_insert(self.pos);
        } else if let Some(start) = self.run.take() {
            self.runs.push((start, self.pos - start));
        }
        self.pos += (self.filled / self.channels) as u64;
        self.filled = 0;
        self.peak = 0.0;
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

    /// What the trimmer drops from `input` played from its start, as frame ranges: the
    /// skips it tells, and what the end of the stream drops untold.
    fn trimmed(t: &mut Trimmer, mode: SkipSilence, input: &[Sample]) -> Vec<Range<u64>> {
        let (out, skips) = run(t, mode, input);
        let mut drops = Vec::new();
        let mut dropped = 0;
        for s in skips {
            let from = (s.at / 2) as u64 + dropped;
            drops.push(from..from + s.frames);
            dropped += s.frames;
        }
        let total = (input.len() / 2) as u64;
        let tail = total - (out.len() / 2) as u64 - dropped;
        if tail > 0 {
            drops.push(total - tail..total);
        }
        drops
    }

    fn scanned(input: &[Sample], rate: u32) -> Silences {
        let mut scan = Scan::new(
            ChannelCount::new(2).unwrap(),
            SampleRate::new(rate).unwrap(),
        );
        for batch in input.chunks(4096) {
            scan.feed(batch);
        }
        scan.finish()
    }

    /// `secs` of stereo at `level` at `rate`, a square wave as `signal` makes.
    fn at_rate(rate: u32, secs: f64, level: Sample) -> Vec<Sample> {
        let frames = (secs * f64::from(rate)).round() as usize;
        (0..frames)
            .flat_map(|i| {
                let s = if i % 2 == 0 { level } else { -level };
                [s, s]
            })
            .collect()
    }

    #[test]
    fn the_scan_marks_what_the_trimmer_drops() {
        // A part-block at the end, and 22.05 kHz, whose 10 ms block (221 frames) doesn't
        // divide the kept 0.5 s: the hold lets a block go before a long run drops.
        for rate in [48_000, 22_050] {
            let (s, t) = (
                |secs| at_rate(rate, secs, 0.0),
                |secs| at_rate(rate, secs, 0.5),
            );
            let tracks = [
                [s(1.0), t(1.0), s(3.0), t(1.0), s(2.0)].concat(),
                [t(1.0), s(1.9), t(0.5), s(2.5), t(0.3), s(1.0)].concat(),
                [t(0.5), s(5.003_7)].concat(),
                [s(0.7)].concat(),
                [t(0.2), s(2.0), t(0.004), s(0.9)].concat(),
            ];
            for mode in [SkipSilence::StartAndEnd, SkipSilence::Everywhere] {
                for (i, track) in tracks.iter().enumerate() {
                    let mut trimmer = Trimmer::new(
                        ChannelCount::new(2).unwrap(),
                        SampleRate::new(rate).unwrap(),
                    );
                    let want = trimmed(&mut trimmer, mode, track);
                    assert!(!want.is_empty(), "track {i} drops nothing in {mode:?}");
                    assert_eq!(
                        scanned(track, rate).drops(mode),
                        want,
                        "track {i} at {rate} Hz in {mode:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_mode_change_derives_the_marks_again_without_a_rescan() {
        let input = [
            silence(1.0),
            tone(1.0),
            silence(3.0),
            tone(1.0),
            silence(2.0),
        ]
        .concat();
        let found = scanned(&input, RATE as u32);
        let secs = |a: f64, b: f64| Duration::from_secs_f64(a)..Duration::from_secs_f64(b);
        // The start whole; the last 1.5 s of the end.
        assert_eq!(
            found.spans(SkipSilence::StartAndEnd),
            [secs(0.0, 1.0), secs(6.5, 8.0)]
        );
        // And the inner gap past its first 0.5 s; all of the end past its first 0.5 s.
        assert_eq!(
            found.spans(SkipSilence::Everywhere),
            [secs(0.0, 1.0), secs(2.5, 5.0), secs(6.5, 8.0)]
        );
        assert!(found.spans(SkipSilence::Off).is_empty());
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
