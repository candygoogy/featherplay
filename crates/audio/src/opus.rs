//! Opus (YouTube's format 251, in WebM) behind symphonia's `Decoder` trait. symphonia 0.5
//! demuxes the WebM but has no Opus decoder; this wraps the pure-Rust `opus-decoder`
//! (vendored with a fast FFT, `vendor/opus-decoder/VENDORED.md`).

use opus_decoder::OpusDecoder;
use symphonia::core::audio::{
    AsAudioBufferRef, AudioBuffer, AudioBufferRef, Channels, Signal, SignalSpec,
};
use symphonia::core::codecs::{
    CODEC_TYPE_OPUS, CodecDescriptor, CodecParameters, Decoder, DecoderOptions, FinalizeResult,
};
use symphonia::core::errors::{Error, Result, decode_error, unsupported_error};
use symphonia::core::formats::Packet;
use symphonia::core::support_codec;

/// The output rate: Opus's own, which its pre-skip and granule positions count in (RFC 7845 §4).
pub(crate) const RATE: u32 = 48_000;

/// What the `OpusHead` (RFC 7845 §5.1, a WebM track's `CodecPrivate`) says.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Head {
    channels: u8,
    /// Frames at 48 kHz to drop from the start of the decoded stream.
    pre_skip: u16,
}

impl Head {
    fn parse(extra: &[u8]) -> Option<Head> {
        if extra.len() < 19 || !extra.starts_with(b"OpusHead") {
            return None;
        }
        Some(Head {
            channels: extra[9],
            pre_skip: u16::from_le_bytes([extra[10], extra[11]]),
        })
    }
}

/// The frames to drop from the start of an Opus track (its pre-skip); 0 for another codec.
pub(crate) fn pre_skip(params: &CodecParameters) -> u64 {
    if params.codec != CODEC_TYPE_OPUS {
        return 0;
    }
    params
        .extra_data
        .as_deref()
        .and_then(Head::parse)
        .map_or(0, |h| u64::from(h.pre_skip))
}

pub(crate) struct OpusCodec {
    params: CodecParameters,
    decoder: OpusDecoder,
    channels: usize,
    /// Interleaved, as the decoder writes it.
    pcm: Vec<f32>,
    buf: AudioBuffer<f32>,
}

impl Decoder for OpusCodec {
    fn try_new(params: &CodecParameters, _options: &DecoderOptions) -> Result<Self> {
        let head = params
            .extra_data
            .as_deref()
            .and_then(Head::parse)
            .ok_or(Error::DecodeError("opus: no OpusHead"))?;
        // Mono and stereo are one stream (mapping family 0); more channels are multistream.
        let layout = match head.channels {
            1 => Channels::FRONT_LEFT,
            2 => Channels::FRONT_LEFT | Channels::FRONT_RIGHT,
            _ => return unsupported_error("opus: more than two channels"),
        };
        let channels = usize::from(head.channels);
        let decoder = OpusDecoder::new(RATE, channels)
            .map_err(|_| Error::Unsupported("opus: decoder setup"))?;
        let max = decoder.max_frame_size_per_channel();
        let mut params = params.clone();
        params.with_sample_rate(RATE).with_channels(layout);
        Ok(OpusCodec {
            params,
            decoder,
            channels,
            pcm: vec![0.0; max * channels],
            buf: AudioBuffer::new(max as u64, SignalSpec::new(RATE, layout)),
        })
    }

    fn supported_codecs() -> &'static [CodecDescriptor] {
        &[support_codec!(CODEC_TYPE_OPUS, "opus", "Opus")]
    }

    fn reset(&mut self) {
        self.decoder.reset();
    }

    fn codec_params(&self) -> &CodecParameters {
        &self.params
    }

    fn decode(&mut self, packet: &Packet) -> Result<AudioBufferRef<'_>> {
        self.buf.clear();
        let frames = match self
            .decoder
            .decode_float(packet.buf(), &mut self.pcm, false)
        {
            Ok(n) => n,
            Err(_) => return decode_error("opus: invalid packet"),
        };
        self.buf.render_reserved(Some(frames));
        for ch in 0..self.channels {
            let plane = self.buf.chan_mut(ch);
            for (i, s) in plane.iter_mut().enumerate() {
                *s = self.pcm[i * self.channels + ch];
            }
        }
        Ok(self.buf.as_audio_buffer_ref())
    }

    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }

    fn last_decoded(&self) -> AudioBufferRef<'_> {
        self.buf.as_audio_buffer_ref()
    }
}
