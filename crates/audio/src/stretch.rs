//! Playback speed with the pitch kept: a WSOLA time-stretcher (waveform-similarity
//! overlap-add), written for the audio thread.
//!
//! The output is built from 40 ms sequences of the input. Each one overlaps the previous by
//! 10 ms, under a Hann crossfade. Between sequences the input moves on by `speed` times what
//! the output does, so 2x skips half of the input and 0.5x plays parts of it twice. Where
//! exactly the next sequence starts is searched within ±12 ms of its nominal place: the
//! offset whose first 10 ms look most like the natural continuation of the sequence just
//! played, by normalised cross-correlation. The search runs on a mono downmix decimated 4x,
//! then refines ±4 frames at the full rate. This is the method SoundTouch's `TDStretch` uses,
//! re-implemented here: SoundTouch itself is LGPL-2.1.
//!
//! `Stretch::new` allocates every buffer once; `push`, `pull` and the rest never allocate,
//! lock or block. At exactly 1.0x the input passes through bit for bit.

/// The slowest speed `set_speed` accepts.
pub(crate) const MIN_SPEED: f32 = 0.5;
/// The fastest speed `set_speed` accepts.
pub(crate) const MAX_SPEED: f32 = 2.0;

const SEQUENCE_SECONDS: f64 = 0.040;
const OVERLAP_SECONDS: f64 = 0.010;
const SEEK_SECONDS: f64 = 0.012;
/// The coarse search compares every 4th frame, each the sum of 4.
const DECIMATION: usize = 4;
/// The fine search tries this many frames either side of the coarse winner.
const REFINE: usize = 4;
/// Samples below this (-300 dBFS) leave the stretched path as 0, so later stages never see a
/// subnormal.
const TINY: f32 = 1e-15;
/// Keeps silence from dividing by zero in the correlation.
const ENERGY_FLOOR: f32 = 1e-12;

/// A time-stretcher for interleaved `f32` audio of a fixed channel count and rate.
///
/// The audio thread `push`es source frames and `pull`s output frames, in blocks of any size.
/// Output comes in steps of 30 ms, each needing up to 64 ms of input buffered (sequence +
/// 2 x seek); the buffer holds twice that. At 1.0x no step is needed: `pull` hands out what
/// was pushed.
pub(crate) struct Stretch {
    channels: usize,
    /// Frames per sequence.
    sequence: usize,
    /// Frames of crossfade between sequences, a multiple of `DECIMATION`.
    overlap: usize,
    /// How far, in frames, the search may move a sequence from its nominal start.
    seek: usize,
    speed: f32,
    /// Interleaved input, `frames` of it held (zero padding included, after `finish`).
    input: Box<[f32]>,
    frames: usize,
    /// Frames of `input` that are real audio, not padding.
    real: usize,
    /// No more input comes: the rest of `real` is played out over zero padding.
    ending: bool,
    /// Where the next sequence nominally starts, in frames into `input`.
    pos: f64,
    /// The `overlap` frames that would follow the last sequence output: the template the
    /// next one is matched against and crossfaded from. Unset before the first sequence.
    tail: Box<[f32]>,
    has_tail: bool,
    /// Stretched output waiting for `pull`, `out[out_start..out_end]`.
    out: Box<[f32]>,
    out_start: usize,
    out_end: usize,
    /// The crossfade's rising half-Hann, `overlap` long; the falling side is `1 - fade_in`.
    fade_in: Box<[f32]>,
    /// Scratch for the search: the region's and the tail's mono downmix, and both decimated.
    mono: Box<[f32]>,
    decimated: Box<[f32]>,
    tail_mono: Box<[f32]>,
    tail_decimated: Box<[f32]>,
}

impl Stretch {
    /// A stretcher at 1.0x for `channels`-channel audio at `sample_rate`. The only place it
    /// allocates: about 50 KB for 48 kHz stereo.
    pub(crate) fn new(channels: u16, sample_rate: u32) -> Self {
        let channels = usize::from(channels.max(1));
        let rate = f64::from(sample_rate.max(8_000));
        let frames = |seconds: f64| (seconds * rate).round() as usize;
        let overlap = (frames(OVERLAP_SECONDS) / DECIMATION * DECIMATION).max(DECIMATION);
        let sequence = frames(SEQUENCE_SECONDS).max(3 * overlap);
        let seek = frames(SEEK_SECONDS);
        // A step reads at most `2 * seek + sequence + 1` frames into the buffer: after each
        // one the buffer starts no more than `seek + 1` frames before the next nominal start.
        // Twice that leaves room for whole blocks from the caller.
        let capacity = 2 * (2 * seek + sequence + 1);
        let fade_in = (0..overlap)
            .map(|i| {
                let x = (i as f64 + 0.5) / overlap as f64;
                (0.5 - 0.5 * (std::f64::consts::PI * x).cos()) as f32
            })
            .collect();
        let zeros = |n: usize| vec![0.0; n].into_boxed_slice();
        let region = 2 * seek + overlap;
        Self {
            channels,
            sequence,
            overlap,
            seek,
            speed: 1.0,
            input: zeros(capacity * channels),
            frames: 0,
            real: 0,
            ending: false,
            pos: 0.0,
            tail: zeros(overlap * channels),
            has_tail: false,
            out: zeros((sequence - overlap) * channels),
            out_start: 0,
            out_end: 0,
            fade_in,
            mono: zeros(region),
            decimated: zeros(region / DECIMATION),
            tail_mono: zeros(overlap),
            tail_decimated: zeros(overlap / DECIMATION),
        }
    }

    /// Sets the speed, clamped to `MIN_SPEED..=MAX_SPEED` (NaN means 1.0). It takes effect
    /// at the next 30 ms step, seamlessly: no `reset` is needed. Returning to exactly 1.0
    /// crossfades back into the plain input once, then passes it through bit for bit.
    pub(crate) fn set_speed(&mut self, speed: f32) {
        self.speed = if speed.is_nan() {
            1.0
        } else {
            speed.clamp(MIN_SPEED, MAX_SPEED)
        };
    }

    pub(crate) fn speed(&self) -> f32 {
        self.speed
    }

    /// Drops all buffered input and output, for a seek or after the input ran dry. The
    /// next output starts cleanly from the next input pushed. The speed is kept.
    pub(crate) fn reset(&mut self) {
        self.frames = 0;
        self.real = 0;
        self.ending = false;
        self.pos = 0.0;
        self.has_tail = false;
        self.out_start = 0;
        self.out_end = 0;
    }

    /// No more input follows (the end of the track): `pull` plays out what is buffered,
    /// over silence where a step reads past the end, then returns 0. `push` takes nothing
    /// until `reset`.
    pub(crate) fn finish(&mut self) {
        self.ending = true;
    }

    /// Takes as many whole frames of interleaved `input` as there is room for and returns
    /// the number of samples taken: 0 when full (pull first) or after `finish`.
    pub(crate) fn push(&mut self, input: &[f32]) -> usize {
        if self.ending {
            return 0;
        }
        let ch = self.channels;
        let room = self.input.len() / ch - self.frames;
        let n = (input.len() / ch).min(room) * ch;
        let at = self.frames * ch;
        self.input[at..at + n].copy_from_slice(&input[..n]);
        self.frames += n / ch;
        self.real = self.frames;
        n
    }

    /// Fills `out` with as many whole frames of output as are ready and returns the number
    /// of samples written. Fewer than `out.len()` means it needs more input (or, after
    /// `finish`, that it has played everything).
    pub(crate) fn pull(&mut self, out: &mut [f32]) -> usize {
        let want = out.len() / self.channels * self.channels;
        let mut done = 0;
        loop {
            let n = (self.out_end - self.out_start).min(want - done);
            out[done..done + n].copy_from_slice(&self.out[self.out_start..self.out_start + n]);
            self.out_start += n;
            done += n;
            if done == want {
                return done;
            }
            let more = if self.speed != 1.0 {
                self.step()
            } else if self.has_tail {
                self.rejoin()
            } else {
                return done + self.pass_through(&mut out[done..want]);
            };
            if !more {
                return done;
            }
        }
    }

    /// At 1.0x with nothing buffered: the caller may skip `push` and `pull` and use its
    /// input as it is, with nothing lost or reordered.
    pub(crate) fn is_idle(&self) -> bool {
        self.speed == 1.0 && !self.has_tail && self.frames == 0 && self.out_start == self.out_end
    }

    /// How many frames `push` has room for now, unless it is `finish`ed.
    pub(crate) fn room_frames(&self) -> usize {
        self.input.len() / self.channels - self.frames
    }

    /// Source frames pushed but not yet output: how far the playing position lags behind
    /// the frames read from the source. Exact at 1.0x; while stretching it counts from the
    /// nominal start of the next sequence, so it may be off by up to the seek window (12 ms).
    pub(crate) fn latency_frames(&self) -> usize {
        let queued = (self.out_end - self.out_start) / self.channels;
        let ahead = (self.real as f64 - self.pos).max(0.0);
        (ahead + queued as f64 * f64::from(self.speed)).round() as usize
    }

    /// 1.0x with nothing pending: copies the input straight out.
    fn pass_through(&mut self, out: &mut [f32]) -> usize {
        let frames = self.real.min(out.len() / self.channels);
        let n = frames * self.channels;
        out[..n].copy_from_slice(&self.input[..n]);
        self.discard(frames);
        // The next sequence, should the speed change, starts at the first frame not yet out.
        self.pos = 0.0;
        n
    }

    /// Outputs one sequence (less its overlap with the next) and moves the input on by
    /// `speed` times that. False when the input is too short for it.
    fn step(&mut self) -> bool {
        let (ch, sequence, overlap) = (self.channels, self.sequence, self.overlap);
        let start = self.pos.round() as usize;
        let (lo, hi) = self.search_range(start);
        if !self.ensure(start, hi + sequence) {
            return self.fade_out_tail();
        }
        let at = if self.has_tail {
            self.search(lo, hi)
        } else {
            start
        };
        let hop = sequence - overlap;
        let faded = if self.has_tail {
            self.crossfade(at);
            overlap * ch
        } else {
            0
        };
        let source = &self.input[at * ch..(at + sequence) * ch];
        self.out[faded..hop * ch].copy_from_slice(&source[faded..hop * ch]);
        self.tail.copy_from_slice(&source[hop * ch..]);
        flush_tiny(&mut self.out[..hop * ch]);
        self.has_tail = true;
        self.out_start = 0;
        self.out_end = hop * ch;
        self.pos += hop as f64 * f64::from(self.speed);
        let behind = (self.pos.floor() as usize).saturating_sub(self.seek);
        self.discard(behind.min(self.frames));
        true
    }

    /// Back at 1.0x after stretching: crossfades the tail into the best-matching place of
    /// the input once, then leaves the input from there to `pass_through`.
    fn rejoin(&mut self) -> bool {
        let start = self.pos.round() as usize;
        let (lo, hi) = self.search_range(start);
        if !self.ensure(start, hi + self.overlap) {
            return self.fade_out_tail();
        }
        let at = self.search(lo, hi);
        self.crossfade(at);
        flush_tiny(&mut self.out[..self.overlap * self.channels]);
        self.out_start = 0;
        self.out_end = self.overlap * self.channels;
        self.has_tail = false;
        self.discard(at + self.overlap);
        self.pos = 0.0;
        true
    }

    /// The first frames a sequence nominally starting at `start` may move to: only `start`
    /// itself for the first one, which has nothing to match.
    fn search_range(&self, start: usize) -> (usize, usize) {
        if self.has_tail {
            (start.saturating_sub(self.seek), start + self.seek)
        } else {
            (start, start)
        }
    }

    /// Whether `input` holds `need` frames for a sequence nominally at `start`. After
    /// `finish` it pads with silence while `start` is still in the real input; once it
    /// isn't, it drops the input, and only the tail is left to `fade_out_tail`.
    fn ensure(&mut self, start: usize, need: usize) -> bool {
        if !self.ending {
            return self.frames >= need;
        }
        if start >= self.real {
            self.frames = 0;
            self.real = 0;
            self.pos = 0.0;
            return false;
        }
        if self.frames < need {
            self.input[self.frames * self.channels..need * self.channels].fill(0.0);
            self.frames = need;
        }
        true
    }

    /// At the end of the input, outputs the tail (the audio that follows the last sequence)
    /// under the crossfade's falling half, so the stretched track ends without a click.
    /// False when there is nothing left: more input is needed, or it has all played out.
    fn fade_out_tail(&mut self) -> bool {
        if !(self.ending && self.has_tail) {
            return false;
        }
        let ch = self.channels;
        let frames = self
            .out
            .chunks_exact_mut(ch)
            .zip(self.tail.chunks_exact(ch))
            .zip(self.fade_in.iter());
        for ((out, tail), &rise) in frames {
            for (o, &a) in out.iter_mut().zip(tail) {
                *o = flush(a * (1.0 - rise));
            }
        }
        self.out_start = 0;
        self.out_end = self.overlap * ch;
        self.has_tail = false;
        true
    }

    /// Writes the crossfade from `tail` into the input at frame `at` to `out[..overlap]`.
    fn crossfade(&mut self, at: usize) {
        let ch = self.channels;
        let incoming = &self.input[at * ch..(at + self.overlap) * ch];
        let frames = self
            .out
            .chunks_exact_mut(ch)
            .zip(self.tail.chunks_exact(ch))
            .zip(incoming.chunks_exact(ch))
            .zip(self.fade_in.iter());
        for (((out, old), new), &rise) in frames {
            for ((o, &a), &b) in out.iter_mut().zip(old).zip(new) {
                *o = a * (1.0 - rise) + b * rise;
            }
        }
    }

    /// The start in `lo..=hi` whose first `overlap` frames correlate best with the tail.
    fn search(&mut self, lo: usize, hi: usize) -> usize {
        let ch = self.channels;
        let overlap = self.overlap;
        let len = hi - lo + overlap;
        downmix(&self.input[lo * ch..(lo + len) * ch], ch, &mut self.mono);
        downmix(&self.tail, ch, &mut self.tail_mono);
        let mono = &self.mono[..len];
        decimate(mono, &mut self.decimated);
        decimate(&self.tail_mono, &mut self.tail_decimated);

        let width = overlap / DECIMATION;
        let candidates = 0..=(hi - lo) / DECIMATION;
        let coarse = best_match(&self.tail_decimated, &self.decimated, candidates, width);
        let centre = lo + coarse * DECIMATION;
        let from = centre.saturating_sub(REFINE).max(lo) - lo;
        let to = (centre + REFINE).min(hi) - lo;
        lo + best_match(&self.tail_mono, mono, from..=to, overlap)
    }

    /// Drops the first `frames` frames of input.
    fn discard(&mut self, frames: usize) {
        if frames == 0 {
            return;
        }
        let ch = self.channels;
        self.input.copy_within(frames * ch..self.frames * ch, 0);
        self.frames -= frames;
        self.real = self.real.saturating_sub(frames);
        self.pos -= frames as f64;
    }
}

/// The offset in `offsets` where `signal[offset..offset + width]` has the highest normalised
/// correlation with `template`. Silence or NaN picks the first offset.
fn best_match(
    template: &[f32],
    signal: &[f32],
    offsets: std::ops::RangeInclusive<usize>,
    width: usize,
) -> usize {
    let mut best = (f32::NEG_INFINITY, *offsets.start());
    for offset in offsets {
        let (dot, energy) = dot_and_energy(template, &signal[offset..offset + width]);
        let score = dot / (energy + ENERGY_FLOOR).sqrt();
        if score > best.0 {
            best = (score, offset);
        }
    }
    best.1
}

/// `Σ a·b` and `Σ b²`, in four lanes so the compiler can vectorise them.
fn dot_and_energy(a: &[f32], b: &[f32]) -> (f32, f32) {
    let mut dot = [0.0_f32; 4];
    let mut energy = [0.0_f32; 4];
    let ((a4, a_rest), (b4, b_rest)) = (a.as_chunks::<4>(), b.as_chunks::<4>());
    for (x, y) in a4.iter().zip(b4) {
        for lane in 0..4 {
            dot[lane] += x[lane] * y[lane];
            energy[lane] += y[lane] * y[lane];
        }
    }
    let mut dot: f32 = dot.iter().sum();
    let mut energy: f32 = energy.iter().sum();
    for (x, y) in a_rest.iter().zip(b_rest) {
        dot += x * y;
        energy += y * y;
    }
    (dot, energy)
}

/// Sums each interleaved frame into one sample (the scale doesn't matter to a normalised
/// correlation), flushing tiny values.
fn downmix(interleaved: &[f32], channels: usize, mono: &mut [f32]) {
    for (m, frame) in mono.iter_mut().zip(interleaved.chunks_exact(channels)) {
        *m = flush(frame.iter().sum());
    }
}

/// Sums every `DECIMATION` samples into one: a box filter, enough to steer the coarse search.
fn decimate(signal: &[f32], decimated: &mut [f32]) {
    for (d, group) in decimated.iter_mut().zip(signal.as_chunks::<DECIMATION>().0) {
        *d = group.iter().sum();
    }
}

fn flush(x: f32) -> f32 {
    if x.abs() < TINY { 0.0 } else { x }
}

fn flush_tiny(samples: &mut [f32]) {
    for x in samples {
        *x = flush(*x);
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_SPEED, MIN_SPEED, Stretch};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::f32::consts::TAU;
    use std::time::Instant;

    const RATE: u32 = 48_000;
    /// One 40 ms sequence at 48 kHz, the length tolerance.
    const SEQUENCE: usize = 1_920;

    /// Counts the allocations of the current thread, so `no_allocation_after_new` sees only
    /// its own, whatever other test threads do.
    struct Counting;

    thread_local! {
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    fn count() {
        // `try_with`: a thread being torn down still allocates.
        let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
    }

    // SAFETY: every call goes straight to `System`, with the caller's layout and pointer.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            count();
            // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which is System's.
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            count();
            // SAFETY: as for `alloc`.
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: `ptr` came from `System` through this allocator, with this layout.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            count();
            // SAFETY: as for `dealloc`; the caller upholds `realloc`'s size rules.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    fn allocations() -> usize {
        ALLOCATIONS.with(Cell::get)
    }

    /// Interleaved `channels`-channel white noise in -0.5..0.5 (xorshift, fixed seed).
    fn noise(frames: usize, channels: usize) -> Vec<f32> {
        let mut state = 0x2545_f491_u32;
        (0..frames * channels)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32) - 0.5
            })
            .collect()
    }

    /// A stereo sine at half scale, the same in both channels.
    fn sine(hz: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let x = 0.5 * (TAU * hz * i as f32 / RATE as f32).sin();
                [x, x]
            })
            .collect()
    }

    /// Feeds `input` in `block`-sample pushes and pulls `block`-sample outputs, as the audio
    /// thread will, then finishes and drains what is left.
    fn run(stretch: &mut Stretch, input: &[f32], block: usize) -> Vec<f32> {
        let mut output = Vec::new();
        let mut buf = vec![0.0; block];
        let mut read = 0;
        loop {
            let n = stretch.pull(&mut buf);
            output.extend_from_slice(&buf[..n]);
            let pushed = stretch.push(&input[read..(read + block).min(input.len())]);
            read += pushed;
            if n == 0 && pushed == 0 {
                break;
            }
        }
        assert_eq!(read, input.len(), "the stretcher took all of the input");
        stretch.finish();
        loop {
            let n = stretch.pull(&mut buf);
            if n == 0 {
                break;
            }
            output.extend_from_slice(&buf[..n]);
        }
        output
    }

    fn left(interleaved: &[f32]) -> Vec<f32> {
        interleaved.iter().step_by(2).copied().collect()
    }

    /// The frequency of a mono tone from its rising zero crossings.
    fn pitch(mono: &[f32]) -> f64 {
        let rising = mono
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        rising as f64 / (mono.len() as f64 / f64::from(RATE))
    }

    /// The largest jump between neighbouring samples: a 440 Hz half-scale sine moves at most
    /// 0.029 per sample at 48 kHz, so a hard splice shows up well above that.
    fn largest_step(mono: &[f32]) -> f32 {
        mono.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn speed_one_passes_the_input_through_bit_for_bit() {
        let input = noise(RATE as usize, 2);
        let mut stretch = Stretch::new(2, RATE);
        let output = run(&mut stretch, &input, 1_000);
        assert!(output == input, "1.0x changed the samples");
    }

    #[test]
    fn output_length_follows_the_speed() {
        let input = sine(440.0, 3 * RATE as usize);
        for speed in [1.5, 0.75] {
            let mut stretch = Stretch::new(2, RATE);
            stretch.set_speed(speed);
            let frames = run(&mut stretch, &input, 512).len() / 2;
            let expected = (3.0 * RATE as f32 / speed) as usize;
            println!("{speed}x: {frames} frames, expected {expected}");
            assert!(
                frames.abs_diff(expected) <= SEQUENCE,
                "{speed}x: {frames} frames, expected {expected}"
            );
        }
    }

    #[test]
    fn a_sine_keeps_its_pitch_and_has_no_hard_splices() {
        let input = sine(440.0, 3 * RATE as usize);
        for speed in [MIN_SPEED, MAX_SPEED] {
            let mut stretch = Stretch::new(2, RATE);
            stretch.set_speed(speed);
            let output = left(&run(&mut stretch, &input, 480));
            let hz = pitch(&output);
            let step = largest_step(&output);
            println!("{speed}x: {hz:.2} Hz, largest step {step:.4}");
            assert!((hz - 440.0).abs() <= 4.4, "{speed}x: {hz} Hz");
            assert!(step < 0.06, "{speed}x: a jump of {step}");
        }
    }

    #[test]
    fn speed_changes_mid_stream_are_seamless() {
        let input = sine(440.0, 2 * RATE as usize);
        let mut stretch = Stretch::new(2, RATE);
        let mut output = Vec::new();
        let mut buf = vec![0.0; 960];
        for (i, block) in input.chunks(960).enumerate() {
            // 1x, then 1.5x, then back to 1x, then 0.75x.
            stretch.set_speed([1.0, 1.5, 1.0, 0.75][i * 4 / (input.len() / 960)]);
            assert_eq!(stretch.push(block), block.len());
            loop {
                let n = stretch.pull(&mut buf);
                if n == 0 {
                    break;
                }
                output.extend_from_slice(&buf[..n]);
            }
        }
        let step = largest_step(&left(&output));
        assert!(step < 0.06, "a jump of {step}");
        assert!(output.len() > input.len() / 2, "{} samples", output.len());
    }

    #[test]
    fn a_speed_change_after_long_pass_through_keeps_time() {
        let second = noise(RATE as usize, 2);
        let mut stretch = Stretch::new(2, RATE);
        let mut out = vec![0.0; 1_000];
        let mut frames = 0;
        for (i, block) in second.chunks(1_000).chain(second.chunks(1_000)).enumerate() {
            if i == second.len() / 1_000 {
                stretch.set_speed(1.5);
            }
            let mut read = 0;
            while read < block.len() {
                read += stretch.push(&block[read..]);
                frames += stretch.pull(&mut out) / 2;
                assert!(stretch.latency_frames() <= 2 * SEQUENCE, "latency ran away");
            }
        }
        // 1 s at 1x, then 1 s of input at 1.5x.
        let expected = RATE as usize + RATE as usize * 2 / 3;
        assert!(
            frames.abs_diff(expected) <= 2 * SEQUENCE,
            "{frames} frames, expected about {expected}"
        );
    }

    #[test]
    fn the_end_of_the_input_fades_out_instead_of_cutting() {
        for extra in (0..8).map(|k| k * 317) {
            let input = sine(440.0, RATE as usize + extra);
            for speed in [0.75, 1.5, 2.0] {
                let mut stretch = Stretch::new(2, RATE);
                stretch.set_speed(speed);
                let output = run(&mut stretch, &input, 512);
                let last = output.last().copied().unwrap_or_default();
                assert!(last.abs() < 0.01, "{speed}x, +{extra}: ends on {last}");
            }
        }
    }

    #[test]
    fn other_layouts_and_odd_blocks_keep_the_length() {
        for (channels, rate) in [(1_u16, 44_100_u32), (6, 8_000)] {
            let ch = usize::from(channels);
            let input = noise(2 * rate as usize, ch);
            for speed in [0.5, 1.0, 1.5, 2.0] {
                let mut stretch = Stretch::new(channels, rate);
                stretch.set_speed(speed);
                let frames = run(&mut stretch, &input, 37 * ch).len() / ch;
                let expected = (2.0 * rate as f32 / speed) as usize;
                let sequence = rate as usize / 25;
                assert!(
                    frames.abs_diff(expected) <= sequence,
                    "{channels} ch at {rate} Hz, {speed}x: {frames} frames, expected {expected}"
                );
            }
        }
    }

    #[test]
    fn set_speed_clamps_and_ignores_nan() {
        let mut stretch = Stretch::new(2, RATE);
        stretch.set_speed(4.0);
        assert_eq!(stretch.speed(), MAX_SPEED);
        stretch.set_speed(0.1);
        assert_eq!(stretch.speed(), MIN_SPEED);
        stretch.set_speed(f32::NAN);
        assert_eq!(stretch.speed(), 1.0);
    }

    #[test]
    fn reset_drops_what_is_buffered() {
        let mut stretch = Stretch::new(2, RATE);
        stretch.set_speed(1.5);
        assert_eq!(stretch.push(&noise(1_000, 2)), 2_000);
        assert_eq!(stretch.latency_frames(), 1_000);
        stretch.reset();
        assert_eq!(stretch.latency_frames(), 0);
        stretch.finish();
        assert_eq!(stretch.pull(&mut [0.0; 64]), 0);
    }

    #[test]
    fn no_allocation_after_new() {
        let input = noise(RATE as usize, 2);
        let mut out = vec![0.0; 1_024];
        let base = allocations();
        let mut stretch = Stretch::new(2, RATE);
        let before = allocations();
        assert!(before > base, "the counter missed new's buffers");
        let mut pulled = 0;
        for (i, block) in input.chunks(1_000).enumerate() {
            stretch.set_speed([0.5, 1.0, 1.25, 2.0, 0.75][i % 5]);
            stretch.push(block);
            pulled += stretch.pull(&mut out);
            if i % 17 == 16 {
                stretch.reset();
            }
        }
        stretch.finish();
        while stretch.pull(&mut out) > 0 {}
        assert_eq!(allocations() - before, 0, "allocated after new");
        assert!(pulled > 0, "nothing came out");
    }

    /// The CPU cost at 0.5x, 1.5x and 2x: 60 s of 48 kHz stereo noise. Run it in release:
    /// `cargo nextest run -p featherplay-audio --release --run-ignored only -E 'test(stretch_cpu)' --no-capture`.
    #[test]
    #[ignore = "a measurement; run in release"]
    fn stretch_cpu() {
        let seconds = 60;
        let input = noise(seconds * RATE as usize, 2);
        let mut out = vec![0.0; 1_024];
        for speed in [0.5, 1.5, 2.0] {
            let mut stretch = Stretch::new(2, RATE);
            stretch.set_speed(speed);
            let start = Instant::now();
            let mut produced = 0;
            for block in input.chunks(1_024) {
                let mut read = 0;
                while read < block.len() {
                    read += stretch.push(&block[read..]);
                    produced += stretch.pull(&mut out);
                }
            }
            let ms = start.elapsed().as_secs_f64() * 1_000.0;
            // Per second played, i.e. of output, is what the audio thread pays.
            let played = produced as f64 / 2.0 / f64::from(RATE);
            let per_played = ms / played;
            println!(
                "stretch at {speed}x: {per_played:.2} ms per second played, {:.2} ms per second of input",
                ms / seconds as f64
            );
            assert!(per_played <= 5.0, "{speed}x: {per_played:.2} ms/s");
        }
    }
}
