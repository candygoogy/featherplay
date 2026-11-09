//! Stream URLs for tracks: one InnerTube request made here (`native`), else the yt-dlp sidecar.
#![cfg_attr(
    test,
    allow(
        clippy::print_stderr,
        clippy::print_stdout,
        reason = "tests report skips and measurements"
    )
)]

mod chain;
mod expiry;
mod native;
mod sidecars;
mod ytdlp;

use std::fmt;
use std::future::Future;
use std::time::{Duration, SystemTime};

use featherplay_core::failure::HelperFault;
use featherplay_core::model::TrackId;

pub use expiry::expires_at_from_url;
pub use sidecars::{Downloads, Sidecars};
pub use ytdlp::{Probe, YtDlpResolver, classify_failure, parse_ytdlp_json};

/// A directly fetchable audio stream. Fetch it with 1 MiB `Range` requests: plain GETs are throttled.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStream {
    pub url: String,
    pub mime: String,
    pub itag: u32,
    pub bitrate_kbps: u32,
    pub content_length: Option<u64>,
    pub expires_at: SystemTime,
    pub duration: Option<Duration>,
    /// The URL returns 403 before this time (YouTube's ad gate on the TV clients).
    /// `YtDlpResolver::resolve` waits it out, so a URL it returns is already usable.
    pub available_at: Option<SystemTime>,
    /// The video's own title and first tag, from the player data: what the scrobbler's
    /// "alternative titles" and "alternative artists" use. `None` when missing or blank.
    pub video_title: Option<String>,
    pub first_tag: Option<String>,
    /// How many dB YouTube measured the song above its reference level (the player data's
    /// `loudnessDb`), for "Match song volumes". The native resolver gives it; yt-dlp's
    /// answer doesn't carry it, so `None` there.
    pub loudness_db: Option<f32>,
}

/// How soon the player may ask for a song resolved ahead (`YtDlpResolver::resolve_ahead`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ahead {
    /// A row or card under the pointer, a page's first row, Home's featured song, the
    /// restored track, a refused preload: a click may come any moment.
    Soon,
    /// The queue's tracks after the next one: minutes away.
    Later,
}

/// Which audio format a resolve asks for (`YtDlpResolver::set_quality`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    /// Format 251 (Opus in WebM, ~130-160 kbps) when the video has it, else 140.
    #[default]
    High,
    /// Format 140 (AAC in m4a, ~129 kbps) only.
    Standard,
}

impl Quality {
    /// The formats wanted, best first.
    pub(crate) fn itags(self) -> &'static [u32] {
        match self {
            Quality::High => &[251, 140],
            Quality::Standard => &[140],
        }
    }

    /// The wanted formats as text, joined by `sep`: `251/140` is yt-dlp's `-f` for High.
    pub(crate) fn formats(self, sep: &str) -> String {
        let itags: Vec<String> = self.itags().iter().map(u32::to_string).collect();
        itags.join(sep)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// The track can't be played (deleted, private, region-locked, none of the formats
    /// `Quality` asks for).
    Unavailable(String),
    /// The yt-dlp or deno sidecar isn't installed.
    ToolMissing,
    /// yt-dlp failed for another reason. Updating yt-dlp may fix it.
    ToolFailed { code: i32, stderr_tail: String },
    /// Downloading or verifying a sidecar failed (network, checksum mismatch, bad archive).
    Sidecar(SidecarError),
}

/// Why and where downloading the sidecars failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidecarError {
    pub fault: HelperFault,
    /// For the log: the step and the error (`https://github.com/…: connection refused`).
    pub message: String,
    /// The server a download asked.
    pub host: Option<String>,
    /// The HTTP status it answered with, when it answered.
    pub status: Option<u16>,
    /// The system's error number for a file step (5: access denied, 32: in use, 112: disk full).
    pub os_error: Option<i32>,
}

impl SidecarError {
    /// One line for Copy diagnostics: the kind, the host, the HTTP status, the os error,
    /// those known. No paths.
    pub fn summary(&self) -> String {
        let mut parts = vec![self.fault.name().to_string()];
        parts.extend(self.host.clone());
        parts.extend(self.status.map(|s| format!("HTTP {s}")));
        parts.extend(self.os_error.map(|n| format!("os error {n}")));
        parts.join(", ")
    }
}

/// For the log: the exit code and stderr tail in full.
impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::Unavailable(m) => write!(f, "unavailable: {m}"),
            ResolveError::ToolMissing => f.write_str("yt-dlp or deno is missing"),
            ResolveError::ToolFailed { code, stderr_tail } => {
                write!(f, "yt-dlp exited with code {code}: {stderr_tail}")
            }
            ResolveError::Sidecar(e) => write!(f, "sidecar ({}): {}", e.fault.name(), e.message),
        }
    }
}

pub trait StreamResolver: Send + Sync {
    fn resolve(
        &self,
        id: &TrackId,
    ) -> impl Future<Output = Result<ResolvedStream, ResolveError>> + Send;
}
