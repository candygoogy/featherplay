//! Audio playback for featherplay: an `AudioEngine` on its own thread, built on rodio 0.23
//! (cpal output, symphonia AAC/MP4 decode).
//!
//! HTTP streams are fetched in 1 MiB `Range` requests into a temp file (`download.rs`),
//! because a plain GET is throttled to about 2x realtime. `file://` URLs play directly.
//! Decoding starts on the first fragment, before the download is done (`decoder.rs`), and
//! `prefetch` keeps the first 256 KB of streams that may play soon in memory (`head.rs`).
//! Each track gets a decoder thread that does all blocking I/O and decodes about 1.5 s ahead,
//! so the audio callback never waits on the network.
//!
//! Events, in the order a track produces them:
//! - `Started` once per `load`, when its first audio reaches the device after `play()`.
//! - `Buffering(true)` when the output has been starved for 150 ms (a stalled download, a
//!   seek past the downloaded data), then `Buffering(false)` when audio resumes.
//! - `Skipped` each time playback reaches silence that skip silence dropped: the position
//!   jumped ahead.
//! - `Silences` while skip silence is on, once the scan of the whole downloaded track has
//!   found where it's silent (`scan.rs`).
//! - `Ended` when the track played to its end. A seek after that plays again.
//! - `Error(msg)` when the stream can't be opened, decoded or downloaded; no `Ended` follows.
//!   An expired URL shows up here as `HTTP 403`.
//! - `DeviceLost` when the output device disappeared: playback is paused and moved to the
//!   default device (or back to the same one if it reappeared).
//! - `NoDevice` when `play()` finds no output device at all (the speakers are off): the
//!   engine stays paused, and the next `play()` looks again. No `Error` follows.
#![cfg_attr(
    test,
    allow(
        clippy::print_stderr,
        clippy::print_stdout,
        reason = "tests report skips and measurements"
    )
)]

use std::time::Duration;

mod decoder;
mod devices;
mod download;
mod dsp;
mod engine;
mod glitches;
mod head;
mod http;
mod media;
mod opus;
mod priority;
mod scan;
mod silence;
#[cfg(test)]
mod stream_tests;
mod stretch;
mod tail;
#[cfg(test)]
mod timing;
mod tmp;
#[cfg(test)]
mod under_load;
mod voice;
mod volume;

pub use devices::output_devices;
pub use dsp::Compressor;
pub use engine::AudioEngine;
pub use silence::{Silences, SkipSilence};
pub use tmp::clear_stale_temp;
pub use volume::perceptual_gain;

/// What to play: an `https://` stream URL (from the resolver) or a `file://` URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamSource {
    pub url: String,
    /// Picks the demuxer, e.g. `audio/mp4`.
    pub mime: String,
    /// Used when the server doesn't report the total size.
    pub content_length: Option<u64>,
    /// The stream's length, when known. While crossfade is on, the decoder reads the last
    /// 20 s from here to find where the music ends (`AudioEngine::playhead`).
    pub duration: Option<Duration>,
}

/// Where playback stands in the current track.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Playhead {
    pub position: Duration,
    /// Where the track's music ends, before any trailing silence, once the decoder has
    /// read its last 20 s (only while crossfade is on; `AudioEngine::set_crossfade`).
    pub music_end: Option<Duration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioEvent {
    Started,
    /// Silence was skipped (`AudioEngine::set_skip_silences`): `position()` jumped ahead,
    /// as after a seek.
    Skipped,
    Ended,
    Buffering(bool),
    DeviceLost,
    NoDevice,
    Error(String),
    /// The URL of a stream `preload` or `prefetch` was given was refused (HTTP 403 on its
    /// first request). Other preload failures stay quiet: a later `load` of the URL tries
    /// again and reports.
    PreloadRefused(String),
    /// While skip silence is on, the current track's silent runs, found by a scan of the
    /// whole stream once its download is complete (`Silences::spans` gives what a mode
    /// drops). Once per track, unless `AudioEngine::silences_known` was told its URL.
    Silences {
        url: String,
        silences: Silences,
    },
}
