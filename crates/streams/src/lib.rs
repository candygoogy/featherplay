//! Stream URLs for tracks: one InnerTube request made here (`native`), else the yt-dlp sidecar.
#![cfg_attr(
    test,
    allow(
        clippy::print_stderr,
        clippy::print_stdout,
        reason = "tests report skips and measurements"
    )
)]

mod expiry;
mod native;
mod sidecars;
mod ytdlp;

use std::fmt;
use std::future::Future;
use std::time::{Duration, SystemTime};

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// The track can't be played (deleted, private, region-locked, no format 140).
    Unavailable(String),
    /// The yt-dlp or deno sidecar isn't installed.
    ToolMissing,
    /// yt-dlp failed for another reason. Updating yt-dlp may fix it.
    ToolFailed { code: i32, stderr_tail: String },
    /// Downloading or verifying a sidecar failed (network, checksum mismatch, bad archive).
    Sidecar(String),
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
            ResolveError::Sidecar(m) => write!(f, "sidecar: {m}"),
        }
    }
}

pub trait StreamResolver: Send + Sync {
    fn resolve(
        &self,
        id: &TrackId,
    ) -> impl Future<Output = Result<ResolvedStream, ResolveError>> + Send;
}
