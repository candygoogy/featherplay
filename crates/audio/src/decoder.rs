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
//!
//! WebM (Opus, itag 251) has none of the MP4 layout: its reader opens on the header alone
//! and seeks with the file's `Cues`, which YouTube puts before the first `Cluster`.
//!
//! Each track plays exactly its own samples, so the songs of a gapless album join without
//! a gap: the codec's delay at the start and the encoder's padding at the end are dropped,
//! which symphonia 0.5 doesn't do. An MP4 says both in its edit list (`elst`: where the
//! track starts in the media, 1600 frames into YouTube's, and its length). A WebM's
//! `OpusHead` has the start (the pre-skip, 312), and its last block the end
//! (`DiscardPadding`, which symphonia's reader skips, so it's read from the file's last
//! `Cluster` here).

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use featherplay_core::sync::LockExt;
use rodio::{ChannelCount, Sample, SampleRate};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, CodecRegistry, Decoder as Codec, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, Packet, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::codecs::AacDecoder;

use crate::media::Media;
use crate::opus::OpusCodec;

/// The decoders: symphonia's AAC one and ours for Opus, which symphonia 0.5 lacks.
static CODECS: LazyLock<CodecRegistry> = LazyLock::new(|| {
    let mut codecs = CodecRegistry::new();
    codecs.register_all::<AacDecoder>();
    codecs.register_all::<OpusCodec>();
    codecs
});

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
    /// The `sidx`'s whole length.
    indexed: Duration,
    /// The edit list's one edit, if the file has one.
    edit: Option<Edit>,
}

/// An edit list's single edit: where the track starts in the media and how long it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Edit {
    /// In the media's timescale (`mdhd`): the codec's delay.
    start: u64,
    media_scale: u32,
    /// In the movie's timescale (`mvhd`); 0 when not given.
    length: u64,
    movie_scale: u32,
}

impl Edit {
    /// The codec's delay in frames at `rate`.
    fn lead(&self, rate: u32) -> u64 {
        if self.media_scale == 0 {
            return 0;
        }
        (u128::from(self.start) * u128::from(rate) / u128::from(self.media_scale)) as u64
    }

    fn length(&self) -> Option<Duration> {
        (self.length > 0 && self.movie_scale > 0)
            .then(|| Duration::from_secs_f64(self.length as f64 / f64::from(self.movie_scale)))
    }
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
    /// Samples of `buf` to give: fewer than it holds in a WebM's last packet, whose end is
    /// the encoder's padding.
    stop: usize,
    /// Samples given since the start of the track (after `lead`), counting a seek's target.
    given: u64,
    /// The track's length in samples, when the file says (an MP4's edit list): the
    /// encoder's padding after it isn't given.
    length: Option<u64>,
    /// A WebM's packet after the one in `buf`, read ahead: the last one ends with padding.
    peeked: Option<Packet>,
    /// A seek went past the last packet: `next()` is `None` until the next seek.
    ended: bool,
    /// A WebM file: no MP4 layout, and seeks go through `seek_webm`.
    webm: bool,
    /// Frames the codec puts before the stream's start (Opus's pre-skip, an MP4's edit),
    /// dropped from the start of the track.
    lead: u64,
}

/// How long Opus decodes before a seek target to converge (RFC 7845 §4.6).
const PRE_ROLL: Duration = Duration::from_millis(80);

/// `d` in frames at `rate`.
fn frames_in(d: Duration, rate: u32) -> u64 {
    (d.as_secs_f64() * f64::from(rate)).round() as u64
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
        let webm = crate::media::hint_for(mime) == Some("webm");
        let layout = if webm {
            // The identity view: a header of 0 bytes, then the file.
            Layout::default()
        } else {
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
        let pre_skip = crate::opus::pre_skip(&track.codec_params);
        let mut codec = CODECS
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
        let ch = u64::from(channels.get());
        let edit = layout.edit.filter(|_| !webm);
        let lead = edit.map_or(pre_skip, |e| e.lead(spec_rate));
        let length = edit
            .and_then(|e| e.length())
            .map(|d| frames_in(d, spec_rate) * ch);
        let stop = buf.len();
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
            skip: lead * ch,
            stop,
            given: 0,
            length,
            peeked: None,
            ended: false,
            webm,
            lead,
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
        if self.webm {
            return self.seek_webm(to);
        }
        // The track's time `to` is `lead` later in the media. YouTube's `sidx` counts in
        // the track's time (its length is the edit's), ffmpeg's in the media's.
        let lead = Duration::from_secs_f64(self.lead as f64 / f64::from(self.rate.get()));
        let in_track = self.layout.edit.and_then(|e| e.length()) == Some(self.layout.indexed);
        let indexed = if in_track { to } else { to + lead };
        let fragment = self.fragment_at(indexed);
        let within = match fragment {
            Some(f) => {
                let head_end = self.layout.head_end;
                self.format = reader(&self.media, &self.hint, head_end, f.offset, self.len, false)
                    .map_err(|e| e.to_string())?;
                // The reader counts from the fragment's start in the media, and the first
                // fragment's is `lead` before the track's.
                let first = in_track && f.start.is_zero();
                indexed - f.start + if first { lead } else { Duration::ZERO }
            }
            None => to + lead,
        };
        self.codec.reset();
        self.buf.clear();
        self.idx = 0;
        self.stop = 0;
        self.skip = 0;
        self.given = frames_in(to, self.rate.get()) * u64::from(self.channels.get());
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

    /// Moves to `to` in a WebM file. The reader is reopened (its Cues come before the first
    /// Cluster, so this reads only the header again), which also drops any frame a previous
    /// seek left buffered in it. Opus needs about 80 ms of decoding before the target to
    /// converge (RFC 7845 §4.6): the reader lands that much earlier, and the samples up to
    /// the target are dropped. Positions count from after the pre-skip.
    fn seek_webm(&mut self, to: Duration) -> Result<(), String> {
        self.format =
            reader(&self.media, &self.hint, 0, 0, self.len, false).map_err(|e| e.to_string())?;
        self.codec.reset();
        self.buf.clear();
        self.idx = 0;
        self.stop = 0;
        self.peeked = None;
        self.given = frames_in(to, self.rate.get()) * u64::from(self.channels.get());
        self.ended = false;
        let rate = self.rate.get();
        // In the stream's own frames, which start with the pre-skip.
        let target = frames_in(to, rate) + self.lead;
        let from = target.saturating_sub(frames_in(PRE_ROLL, rate));
        let landed = if from == 0 {
            0
        } else {
            let at = SeekTo::Time {
                time: Duration::from_secs_f64(from as f64 / f64::from(rate)).into(),
                track_id: Some(self.track),
            };
            let seeked = match self.format.seek(SeekMode::Accurate, at) {
                Ok(s) => s,
                // Past the last block: the track is over (a network failure shows up as the
                // media's own error when the voice reads on).
                Err(_) => {
                    self.ended = true;
                    return Ok(());
                }
            };
            match self.codec.codec_params().time_base {
                Some(tb) => frames_in(tb.calc_time(seeked.actual_ts).into(), rate),
                None => from,
            }
        };
        self.skip = target.saturating_sub(landed) * self.channels.get() as u64;
        Ok(())
    }

    /// The fragment holding `to`, when the file has an index.
    fn fragment_at(&self, to: Duration) -> Option<Fragment> {
        let frags = &self.layout.fragments;
        let i = frags.partition_point(|f| f.start <= to).checked_sub(1)?;
        Some(frags[i])
    }

    /// The track's next packet; `None` at the end or on an error.
    fn next_packet(&mut self) -> Option<Packet> {
        if let Some(p) = self.peeked.take() {
            return Some(p);
        }
        loop {
            match self.format.next_packet() {
                Ok(p) if p.track_id() == self.track => return Some(p),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }

    /// Decodes the next packet into `buf`. `false` at the end or on an error.
    fn refill(&mut self) -> bool {
        loop {
            let Some(packet) = self.next_packet() else {
                return false;
            };
            match self.codec.decode(&packet) {
                Ok(decoded) if decoded.frames() > 0 => {
                    let spec = *decoded.spec();
                    if self.buf.capacity() < decoded.capacity() {
                        self.buf = SampleBuffer::new(decoded.capacity() as u64, spec);
                    }
                    self.buf.copy_interleaved_ref(decoded);
                    self.idx = 0;
                    self.stop = self.buf.len();
                    if self.webm {
                        self.peeked = self.next_packet();
                        if self.peeked.is_none() {
                            // The last packet: it ends with the encoder's padding.
                            let pad = self.end_padding() * usize::from(self.channels.get());
                            self.stop = self.stop.saturating_sub(pad);
                        }
                    }
                    return true;
                }
                Ok(_) | Err(Error::DecodeError(_)) => continue,
                Err(_) => return false,
            }
        }
    }

    /// The next run of the track's samples, at most `max` (more than 0) of them, straight
    /// from the decoded packet; `None` at the end or on an error.
    pub fn run(&mut self, max: usize) -> Option<&[Sample]> {
        debug_assert!(max > 0, "a run of no samples");
        loop {
            if self.ended {
                return None;
            }
            let left = self
                .length
                .map_or(usize::MAX, |n| n.saturating_sub(self.given) as usize);
            if left == 0 || (self.idx >= self.stop && !self.refill()) {
                return None;
            }
            let held = self.stop - self.idx;
            if self.skip > 0 {
                let n = held.min(usize::try_from(self.skip).unwrap_or(usize::MAX));
                self.idx += n;
                self.skip -= n as u64;
                continue;
            }
            let n = held.min(max).min(left);
            if n == 0 {
                continue;
            }
            let from = self.idx;
            self.idx += n;
            self.given += n as u64;
            return Some(&self.buf.samples()[from..from + n]);
        }
    }

    /// Appends the next samples to `out` until it holds `upto`. False if the track ended
    /// first.
    pub fn fill(&mut self, out: &mut Vec<Sample>, upto: usize) -> bool {
        while out.len() < upto {
            match self.run(upto - out.len()) {
                Some(run) => out.extend_from_slice(run),
                None => return false,
            }
        }
        true
    }

    /// The frames of padding at the end of a WebM, from its last block; 0 if it has none
    /// or the end can't be read.
    fn end_padding(&self) -> usize {
        let mut m = self.media.lock_or_recover();
        let ns = webm_end_padding(&mut *m, self.len).unwrap_or(0);
        // The reader's view seeks the media again before its next read.
        frames_in(Duration::from_nanos(ns), self.rate.get()) as usize
    }
}

/// A sample at a time, for tests.
#[cfg(test)]
impl Iterator for Decoder {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        self.run(1).map(|run| run[0])
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
    let mut indexed = Duration::ZERO;
    let mut edit = None;
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
                (fragments, indexed) = parse_sidx(&body, pos + size).unwrap_or_default();
            }
            b"moov" if size <= 1 << 20 => {
                let mut body = vec![0u8; (size - header) as usize];
                r.read_exact(&mut body)?;
                edit = parse_edit(&body);
            }
            _ => {}
        }
        pos += size;
    }
    Ok(Layout {
        head_end: pos.min(len),
        fragments,
        indexed,
        edit,
    })
}

/// The boxes in `b`, a box's body, as (type, body).
fn boxes(mut b: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        let size = u32::from_be_bytes(b.get(0..4)?.try_into().ok()?) as usize;
        let kind: [u8; 4] = b.get(4..8)?.try_into().ok()?;
        let (size, header) = match size {
            0 => (b.len(), 8),
            1 => (
                usize::try_from(u64::from_be_bytes(b.get(8..16)?.try_into().ok()?)).ok()?,
                16,
            ),
            n => (n, 8),
        };
        let body = b.get(header..size)?;
        b = &b[size..];
        Some((kind, body))
    })
}

/// A full box's timescale, at 12 in version 0 and 20 in version 1 (`mvhd`, `mdhd`).
fn timescale(b: &[u8]) -> Option<u32> {
    let at = if *b.first()? == 1 { 20 } else { 12 };
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The edit of the `moov` body's first track, if its edit list has a single edit of the
/// media. An empty edit (a delay before the media) or several aren't followed: the track
/// plays as it's coded.
fn parse_edit(moov: &[u8]) -> Option<Edit> {
    let movie_scale = boxes(moov)
        .find(|(k, _)| k == b"mvhd")
        .and_then(|(_, b)| timescale(b))?;
    let (_, trak) = boxes(moov).find(|(k, _)| k == b"trak")?;
    let media_scale = boxes(trak)
        .find(|(k, _)| k == b"mdia")
        .and_then(|(_, m)| boxes(m).find(|(k, _)| k == b"mdhd"))
        .and_then(|(_, b)| timescale(b))?;
    let (_, edts) = boxes(trak).find(|(k, _)| k == b"edts")?;
    let (_, elst) = boxes(edts).find(|(k, _)| k == b"elst")?;
    if u32::from_be_bytes(elst.get(4..8)?.try_into().ok()?) != 1 {
        return None;
    }
    let (length, start) = if *elst.first()? == 1 {
        (
            u64::from_be_bytes(elst.get(8..16)?.try_into().ok()?),
            i64::from_be_bytes(elst.get(16..24)?.try_into().ok()?),
        )
    } else {
        (
            u64::from(u32::from_be_bytes(elst.get(8..12)?.try_into().ok()?)),
            i64::from(i32::from_be_bytes(elst.get(12..16)?.try_into().ok()?)),
        )
    };
    Some(Edit {
        start: u64::try_from(start).ok()?,
        media_scale,
        length,
        movie_scale,
    })
}

/// How much of a WebM's end is searched for its last `Cluster`: YouTube's hold up to ~10 s,
/// some 170 KB.
const WEBM_TAIL: u64 = 512 << 10;

/// The `DiscardPadding` of a WebM's last block, in ns: what the encoder added after the
/// end. `None` if its last `Cluster` can't be read from the end of the file.
fn webm_end_padding<R: Read + Seek>(r: &mut R, len: u64) -> Option<u64> {
    const CLUSTER: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
    let from = len.saturating_sub(WEBM_TAIL);
    r.seek(SeekFrom::Start(from)).ok()?;
    let mut tail = vec![0u8; usize::try_from(len - from).ok()?];
    r.read_exact(&mut tail).ok()?;
    // The last `Cluster` whose children run exactly to the end; an id that's really part
    // of a block's data fails that.
    (0..tail.len().saturating_sub(3))
        .rev()
        .filter(|&i| tail[i..i + 4] == CLUSTER)
        .find_map(|i| last_block_padding(&tail[i + 4..]))
}

/// An EBML size at the start of `b`, and how many bytes it took; `None` for an unknown size.
fn ebml_size(b: &[u8]) -> Option<(u64, usize)> {
    let first = *b.first()?;
    let n = first.leading_zeros() as usize + 1;
    if n > 8 {
        return None;
    }
    let mut v = u64::from(first) & (0xFF >> n);
    for byte in b.get(1..n)? {
        v = v << 8 | u64::from(*byte);
    }
    (v != (1 << (7 * n)) - 1).then_some((v, n))
}

/// The EBML element at the start of `b`: its id, its data, and what follows it.
fn ebml_element(b: &[u8]) -> Option<(u32, &[u8], &[u8])> {
    let n = b.first()?.leading_zeros() as usize + 1;
    if n > 4 {
        return None;
    }
    let id = b.get(..n)?.iter().fold(0u32, |v, x| v << 8 | u32::from(*x));
    let (size, m) = ebml_size(&b[n..])?;
    let start = n + m;
    let end = start.checked_add(usize::try_from(size).ok()?)?;
    Some((id, b.get(start..end)?, &b[end..]))
}

/// The padding of the last block of a `Cluster` whose size and data are `b`, and which ends
/// where `b` does: its `BlockGroup`'s `DiscardPadding`, or 0.
fn last_block_padding(b: &[u8]) -> Option<u64> {
    const SIMPLE_BLOCK: u32 = 0xA3;
    const BLOCK_GROUP: u32 = 0xA0;
    const DISCARD_PADDING: u32 = 0x75A2;
    let (size, n) = ebml_size(b)?;
    let mut rest = b.get(n..)?;
    if rest.len() as u64 != size {
        return None;
    }
    let mut padding = None;
    while !rest.is_empty() {
        let (id, data, after) = ebml_element(rest)?;
        match id {
            SIMPLE_BLOCK => padding = Some(0),
            BLOCK_GROUP => {
                let mut inner = data;
                let mut pad = 0;
                while !inner.is_empty() {
                    let (id, data, after) = ebml_element(inner)?;
                    if id == DISCARD_PADDING && (1..=8).contains(&data.len()) {
                        // A signed integer; a negative one would pad the start instead.
                        let bits = 64 - 8 * data.len() as u32;
                        let v = (data.iter().fold(0u64, |v, x| v << 8 | u64::from(*x)) << bits)
                            as i64
                            >> bits;
                        pad = u64::try_from(v).unwrap_or(0);
                    }
                    inner = after;
                }
                padding = Some(pad);
            }
            _ => {}
        }
        rest = after;
    }
    padding
}

/// The fragments of a flat `sidx` whose box ends at `end` (offsets count from there), and
/// their whole length. `None` for a malformed or nested index.
fn parse_sidx(b: &[u8], end: u64) -> Option<(Vec<Fragment>, Duration)> {
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
    let length = Duration::from_secs_f64(ticks as f64 / timescale as f64);
    Some((out, length))
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
        let (f, length) = parse_sidx(&b, 500).unwrap();
        assert_eq!(length, Duration::from_millis(3500));
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

#[cfg(test)]
mod padding {
    use proptest::prelude::*;

    use super::*;

    /// An EBML element: `id` (its bytes), an 8-byte size, `data`.
    fn element(id: &[u8], data: &[u8]) -> Vec<u8> {
        let mut e = id.to_vec();
        e.push(0x01);
        e.extend(&(data.len() as u64).to_be_bytes()[1..]);
        e.extend(data);
        e
    }

    proptest! {
        /// Behind any bytes, a last `Cluster` of blocks (whose data may hold the
        /// `Cluster` id), the last a `BlockGroup` with `DiscardPadding`, gives that padding.
        #[test]
        fn the_last_blocks_padding_is_found(
            before in proptest::collection::vec(any::<u8>(), 0..300),
            blocks in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..64), 0..6),
            pad in 0u32..=120_000_000,
        ) {
            let mut cluster = element(&[0xE7], &[0]);
            for b in &blocks {
                let mut data = b.clone();
                data.extend([0x1F, 0x43, 0xB6, 0x75]);
                cluster.extend(element(&[0xA3], &data));
            }
            let group = [element(&[0xA1], &[1, 2, 3]), element(&[0x75, 0xA2], &pad.to_be_bytes())].concat();
            cluster.extend(element(&[0xA0], &group));
            let file = [before, element(&[0x1F, 0x43, 0xB6, 0x75], &cluster)].concat();
            let len = file.len() as u64;
            let got = webm_end_padding(&mut io::Cursor::new(file), len);
            prop_assert_eq!(got, Some(u64::from(pad)));
        }

        /// Any bytes: no panic, and a moov parse that never panics either.
        #[test]
        fn any_tail_or_moov_reads_without_a_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
            let len = bytes.len() as u64;
            let _ = parse_edit(&bytes);
            let _ = webm_end_padding(&mut io::Cursor::new(bytes), len);
        }
    }
}
