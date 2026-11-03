//! AAC-in-MP4 decoding that starts before the file is downloaded.
//!
//! symphonia's MP4 reader, given a seekable source, walks every top-level atom of the file
//! before it returns a single packet. The DASH m4a streams are fragmented (`ftyp`, `moov`,
//! `sidx`, then a `moof` + `mdat` pair per ~10 s), so that walk touched every fragment, and
//! the first sound waited for the whole download. Here the reader is opened over a source
//! that reports itself unseekable while the reader starts: it then stops at the first
//! fragment and reads the rest as playback reaches it. The source becomes seekable once the
//! reader exists, so packets and seeks still move freely.
//!
//! Seeks use the file's segment index (`sidx`): the reader is reopened over a view that
//! puts the target fragment straight after the `moov`, so a seek fetches the bytes at the
//! target (a `Range` request if they aren't downloaded) instead of walking every fragment
//! before it. Files without an index (local, unfragmented m4a) seek through the reader.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use featherplay_core::sync::LockExt;
use rodio::{ChannelCount, Sample, SampleRate};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder as Codec, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::media::Media;

/// One reference of a `sidx`: where a fragment starts in the file and in time.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fragment {
    offset: u64,
    start: Duration,
}

/// What the atoms before the first fragment say about the file.
#[derive(Debug, Default, PartialEq)]
struct Layout {
    /// Offset of the first `moof`/`mdat`: everything before it is the header.
    head_end: u64,
    /// From the `sidx`, in file order. Empty without one (or with a nested one).
    fragments: Vec<Fragment>,
}

/// A view of the media for one reader: the header (`0..head_end`), then the file from
/// `jump` on. `jump == head_end` is the file itself.
struct View {
    media: Arc<Mutex<Media>>,
    seekable: Arc<AtomicBool>,
    head_end: u64,
    jump: u64,
    len: u64,
    /// Position in the view.
    pos: u64,
}

impl View {
    fn physical(&self, pos: u64) -> u64 {
        if pos < self.head_end {
            pos
        } else {
            self.jump + (pos - self.head_end)
        }
    }

    fn view_len(&self) -> u64 {
        self.head_end + (self.len - self.jump)
    }
}

impl Read for View {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Never read across the seam between the header and the jump target.
        let want = if self.pos < self.head_end {
            buf.len().min((self.head_end - self.pos) as usize)
        } else {
            buf.len()
        };
        let at = self.physical(self.pos);
        let mut m = self.media.lock_or_recover();
        if m.position() != at {
            m.seek(SeekFrom::Start(at))?;
        }
        let n = m.read(&mut buf[..want])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for View {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => self.view_len().checked_add_signed(d),
        };
        let t = target.ok_or_else(|| io::Error::other("seek before the start"))?;
        // The media seeks lazily, on the next read.
        self.pos = t;
        Ok(t)
    }
}

impl MediaSource for View {
    fn is_seekable(&self) -> bool {
        self.seekable.load(Ordering::Relaxed)
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.view_len())
    }
}

pub(crate) struct Decoder {
    media: Arc<Mutex<Media>>,
    len: u64,
    hint: Hint,
    layout: Layout,
    format: Box<dyn FormatReader>,
    codec: Box<dyn Codec>,
    track: u32,
    buf: SampleBuffer<Sample>,
    idx: usize,
    channels: ChannelCount,
    rate: SampleRate,
    /// Samples still to drop after a seek, to land on the exact target.
    skip: u64,
    /// A seek went past the last packet: `next()` is `None` until the next seek.
    ended: bool,
}

impl Decoder {
    /// Opens `media` (`len` bytes) and decodes its first packet. Reads only the header and
    /// the first packets, so over HTTP it returns once those bytes arrived.
    pub fn open(media: Media, len: u64, mime: &str) -> Result<Decoder, String> {
        let media = Arc::new(Mutex::new(media));
        let mut hint = Hint::new();
        if let Some(ext) = crate::media::hint_for(mime) {
            hint.with_extension(ext);
            hint.mime_type(mime.split(';').next().unwrap_or_default().trim());
        }
        let layout = {
            let mut m = media.lock_or_recover();
            let layout =
                read_layout(&mut *m, len).map_err(|e| format!("reading the header: {e}"))?;
            m.seek(SeekFrom::Start(0))
                .map_err(|e| format!("reading the header: {e}"))?;
            layout
        };
        let head_end = layout.head_end;
        let mut format = match reader(&media, &hint, head_end, head_end, len, false) {
            Ok(f) => f,
            // Not streamable (the `moov` comes after the data): read it the slow way.
            Err(_) => reader(&media, &hint, head_end, head_end, len, true)
                .map_err(|e| format!("decoder: {e}"))?,
        };
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or("decoder: no supported audio track")?;
        let track_id = track.id;
        let mut codec = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| format!("decoder: {e}"))?;
        // The first packet gives the real channel count and rate.
        let buf = loop {
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(e) => return Err(format!("decoder: no audio ({e})")),
            };
            if packet.track_id() != track_id {
                continue;
            }
            match codec.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let mut buf = SampleBuffer::new(decoded.capacity() as u64, spec);
                    buf.copy_interleaved_ref(decoded);
                    break buf;
                }
                Err(Error::DecodeError(_)) => continue,
                Err(e) => return Err(format!("decoder: {e}")),
            }
        };
        let spec_channels = codec_channels(&*codec).unwrap_or(2);
        let spec_rate = codec.codec_params().sample_rate.unwrap_or(44_100);
        let channels = ChannelCount::new(spec_channels).ok_or("decoder: no channels")?;
        let rate = SampleRate::new(spec_rate).ok_or("decoder: no sample rate")?;
        Ok(Decoder {
            media,
            len,
            hint,
            layout,
            format,
            codec,
            track: track_id,
            buf,
            idx: 0,
            channels,
            rate,
            skip: 0,
            ended: false,
        })
    }

    pub fn channels(&self) -> ChannelCount {
        self.channels
    }

    pub fn sample_rate(&self) -> SampleRate {
        self.rate
    }

    /// Moves to `to`. With a segment index this reads only the fragment at the target;
    /// without one the reader walks there. Past the end, the next `next()` is `None`.
    pub fn try_seek(&mut self, to: Duration) -> Result<(), String> {
        let fragment = self.fragment_at(to);
        let within = match fragment {
            Some(f) => {
                let head_end = self.layout.head_end;
                self.format = reader(&self.media, &self.hint, head_end, f.offset, self.len, false)
                    .map_err(|e| e.to_string())?;
                to - f.start
            }
            None => to,
        };
        self.codec.reset();
        self.buf.clear();
        self.idx = 0;
        self.skip = 0;
        self.ended = false;
        if within.is_zero() {
            return Ok(());
        }
        let target = SeekTo::Time {
            time: within.into(),
            track_id: Some(self.track),
        };
        let seeked = match self.format.seek(SeekMode::Accurate, target) {
            Ok(s) => s,
            // Past the last fragment: the track is over (a network failure shows up as the
            // media's own error when the voice reads on).
            Err(_) if fragment.is_some() => {
                self.ended = true;
                return Ok(());
            }
            Err(e) => return Err(e.to_string()),
        };
        // The reader lands on the packet before the target; drop the samples up to it.
        if let Some(tb) = self.codec.codec_params().time_base {
            let early =
                Duration::from(tb.calc_time(seeked.required_ts.saturating_sub(seeked.actual_ts)));
            let frames = (early.as_secs_f64() * self.rate.get() as f64).round() as u64;
            self.skip = frames * self.channels.get() as u64;
        }
        Ok(())
    }

    /// The fragment holding `to`, when the file has an index.
    fn fragment_at(&self, to: Duration) -> Option<Fragment> {
        let frags = &self.layout.fragments;
        let i = frags.partition_point(|f| f.start <= to).checked_sub(1)?;
        Some(frags[i])
    }

    /// Decodes the next packet into `buf`. `false` at the end or on an error.
    fn refill(&mut self) -> bool {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(_) => return false,
            };
            if packet.track_id() != self.track {
                continue;
            }
            match self.codec.decode(&packet) {
                Ok(decoded) if decoded.frames() > 0 => {
                    let spec = *decoded.spec();
                    if self.buf.capacity() < decoded.capacity() {
                        self.buf = SampleBuffer::new(decoded.capacity() as u64, spec);
                    }
                    self.buf.copy_interleaved_ref(decoded);
                    self.idx = 0;
                    return true;
                }
                Ok(_) | Err(Error::DecodeError(_)) => continue,
                Err(_) => return false,
            }
        }
    }
}

impl Iterator for Decoder {
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Sample> {
        if self.ended {
            return None;
        }
        loop {
            if self.idx >= self.buf.len() && !self.refill() {
                return None;
            }
            let s = self.buf.samples()[self.idx];
            self.idx += 1;
            if self.skip == 0 {
                return Some(s);
            }
            self.skip -= 1;
        }
    }
}

fn codec_channels(codec: &dyn Codec) -> Option<u16> {
    codec
        .codec_params()
        .channels
        .map(|c| c.count() as u16)
        .filter(|&c| c > 0)
}

/// A format reader over the view `0..head_end` + `jump..len`. `seekable` false makes the
/// MP4 reader stop at the first fragment instead of walking the whole file; the view turns
/// seekable once the reader exists.
fn reader(
    media: &Arc<Mutex<Media>>,
    hint: &Hint,
    head_end: u64,
    jump: u64,
    len: u64,
    seekable: bool,
) -> Result<Box<dyn FormatReader>, Error> {
    let flag = Arc::new(AtomicBool::new(seekable));
    let view = View {
        media: media.clone(),
        seekable: flag.clone(),
        head_end,
        jump,
        len,
        pos: 0,
    };
    let mss = MediaSourceStream::new(Box::new(view), MediaSourceStreamOptions::default());
    let opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let probed =
        symphonia::default::get_probe().format(hint, mss, &opts, &MetadataOptions::default())?;
    flag.store(true, Ordering::Relaxed);
    Ok(probed.format)
}

/// Walks the top-level atoms up to the first fragment or `mdat`, reading the `sidx`.
fn read_layout<R: Read + Seek>(r: &mut R, len: u64) -> io::Result<Layout> {
    let mut pos = 0u64;
    let mut fragments = Vec::new();
    while pos + 8 <= len {
        r.seek(SeekFrom::Start(pos))?;
        let mut h = [0u8; 8];
        r.read_exact(&mut h)?;
        let mut size = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
        let kind = [h[4], h[5], h[6], h[7]];
        let mut header = 8;
        if size == 1 {
            let mut big = [0u8; 8];
            r.read_exact(&mut big)?;
            size = u64::from_be_bytes(big);
            header = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < header {
            break; // malformed; let the reader complain
        }
        match &kind {
            b"moof" | b"mdat" => break,
            b"sidx" if fragments.is_empty() && size <= 1 << 20 => {
                let mut body = vec![0u8; (size - header) as usize];
                r.read_exact(&mut body)?;
                fragments = parse_sidx(&body, pos + size).unwrap_or_default();
            }
            _ => {}
        }
        pos += size;
    }
    Ok(Layout {
        head_end: pos.min(len),
        fragments,
    })
}

/// The fragments of a flat `sidx` whose box ends at `end` (offsets count from there).
/// `None` for a malformed or nested index.
fn parse_sidx(b: &[u8], end: u64) -> Option<Vec<Fragment>> {
    let be32 = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
    let be64 = |i: usize| Some(u64::from_be_bytes(b.get(i..i + 8)?.try_into().ok()?));
    let version = *b.first()?;
    let timescale = be32(8)? as u64;
    if timescale == 0 {
        return None;
    }
    let (first_offset, mut i) = if version == 0 {
        (be32(16)? as u64, 20)
    } else {
        (be64(20)?, 28)
    };
    i += 2; // reserved
    let count = u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?) as usize;
    i += 2;
    let mut offset = end + first_offset;
    let mut ticks = 0u64;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let size = be32(i)?;
        let dur = be32(i + 4)? as u64;
        if size & 0x8000_0000 != 0 {
            return None; // references another sidx
        }
        out.push(Fragment {
            offset,
            start: Duration::from_secs_f64(ticks as f64 / timescale as f64),
        });
        offset += (size & 0x7fff_ffff) as u64;
        ticks += dur;
        i += 12;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidx_offsets_and_times() {
        // version 0, flags, reference_ID 1, timescale 1000, ept 0, first_offset 0,
        // reserved, count 2, then (size, duration, sap) twice.
        let mut b = vec![
            0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 3, 232, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2,
        ];
        for (size, dur) in [(100u32, 2000u32), (50, 1500)] {
            b.extend(size.to_be_bytes());
            b.extend(dur.to_be_bytes());
            b.extend(0x9000_0000u32.to_be_bytes());
        }
        let f = parse_sidx(&b, 500).unwrap();
        assert_eq!(
            f,
            vec![
                Fragment {
                    offset: 500,
                    start: Duration::ZERO
                },
                Fragment {
                    offset: 600,
                    start: Duration::from_secs(2)
                },
            ]
        );
    }
}
