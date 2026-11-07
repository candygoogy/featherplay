//! Where a stream's music ends, for crossfade. A fade of up to 12 s must start that long
//! before the music ends, but the decoder runs only 1.5 s ahead of playback, and Skip
//! silence learns of a silent outro only when it gets there. So, once per track, a second
//! decoder reads the stream's last `TAIL` over the same downloaded bytes, and Skip
//! silence's test (10 ms blocks, silent under −60 dBFS) finds the last sound in it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rodio::Sample;

use crate::decoder::Decoder;
use crate::media::Media;
use crate::silence::{BLOCK, THRESHOLD};

/// How much of the end of a stream is read: longer than any trailing silence but a
/// hidden track's.
const TAIL: Duration = Duration::from_secs(20);

/// Where the music of `media` (`len` bytes, a stream `length` long) ends, counted from the
/// track's start: the end of the last 10 ms block whose peak reaches −60 dBFS within its
/// last `TAIL`, or the start of that window if it's all silent. `None` if the stream can't
/// be read there, or once `stop` is set.
pub(crate) fn music_end(
    media: Media,
    len: u64,
    mime: &str,
    length: Duration,
    stop: &AtomicBool,
) -> Option<Duration> {
    let mut dec = Decoder::open(media, len, mime).ok()?;
    let from = length.saturating_sub(TAIL);
    dec.try_seek(from).ok()?;
    let rate = f64::from(dec.sample_rate().get());
    let channels = dec.channels().get() as usize;
    let block = ((BLOCK.as_secs_f64() * rate).round() as usize).max(1) * channels;
    // Samples read since `from`, and up to the end of the last block with sound.
    let (mut read, mut sound) = (0_usize, 0_usize);
    let mut peak: Sample = 0.0;
    for s in dec.by_ref() {
        peak = peak.max(s.abs());
        read += 1;
        if read % block == 0 {
            if peak >= THRESHOLD {
                sound = read;
            }
            peak = 0.0;
            if stop.load(Ordering::Relaxed) {
                return None;
            }
        }
    }
    if peak >= THRESHOLD {
        sound = read;
    }
    if read == 0 {
        return None;
    }
    Some(from + Duration::from_secs_f64((sound / channels) as f64 / rate))
}
