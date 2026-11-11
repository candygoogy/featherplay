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
mod js;
mod native;
mod sidecars;
mod ytdlp;

use std::fmt;
use std::future::Future;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use featherplay_core::failure::HelperFault;
use featherplay_core::model::TrackId;

pub use expiry::{expires_at_from_url, unix_secs};
pub use sidecars::{Downloads, Sidecars};
pub use ytdlp::{Probe, YtDlpResolver, classify_failure, parse_ytdlp_json};

/// The JavaScript engine built in for YouTube's player challenges, with its version:
/// `QuickJS-NG 0.16.2`.
pub fn js_engine() -> String {
    js::engine()
}

/// Where the native resolver preprocesses a new player's JS: seconds of work and ~220 MB at
/// its peak, once per player version (`YtDlpResolver::with_preprocessor`).
#[derive(Clone)]
pub enum Preprocessor {
    /// In this process, on a thread of its own. For tests and tools.
    InProcess,
    /// In a short-lived child process, so the memory peak and a crash stay out of the app:
    /// this program, started with [`PREPROCESS_ARG`] alone, which must then run
    /// [`preprocess_stdio`] and nothing else (featherplay's own exe does).
    Child(PathBuf),
    /// Stands in for the work: the preprocessed player for a player's JS.
    #[cfg(test)]
    Stub(Stub),
}

/// Stands in for the preprocessing in tests.
#[cfg(test)]
pub(crate) type Stub = std::sync::Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// The one argument of the process that preprocesses a player ([`Preprocessor::Child`]).
pub const PREPROCESS_ARG: &str = "--preprocess-player";

/// The whole run of the process started with [`PREPROCESS_ARG`]: reads a player's JS from
/// stdin and writes the preprocessed player to stdout. False when it failed, with the
/// reason on stderr. It reads no file and asks nothing of the network.
pub fn preprocess_stdio() -> bool {
    let mut js = String::new();
    let done = std::io::stdin()
        .read_to_string(&mut js)
        .map_err(|e| format!("reading the player: {e}"))
        .and_then(|_| js::preprocess(&js))
        .and_then(|player| {
            let mut out = std::io::stdout().lock();
            out.write_all(player.as_bytes())
                .and_then(|()| out.flush())
                .map_err(|e| format!("writing the player: {e}"))
        });
    match done {
        Ok(()) => true,
        Err(e) => {
            let _ = std::io::stderr().write_all(e.as_bytes());
            false
        }
    }
}

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
    /// Which step of the resolve gave the URL, for the log and Copy diagnostics. Not kept
    /// in the streams saved for the next session: those come back as `Via::Saved`.
    pub via: Via,
}

/// Where a stream came from (`ResolvedStream::via`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// The native request as the visionOS app (a URL without challenges).
    NativeVisionOs,
    /// The native request as the embedded client, its challenges solved in QuickJS.
    NativeEmbedded,
    /// yt-dlp with the embedded client (step 2).
    YtDlpEmbedded,
    /// yt-dlp's default clients (step 3).
    YtDlpDefault,
    /// A stream kept from the last session (`{cache}/streams.json`).
    Saved,
    /// A local file standing in for one (the performance harness's fixtures).
    Fixture,
}

impl Via {
    /// A short, stable name for the log and Copy diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Via::NativeVisionOs => "native visionOS",
            Via::NativeEmbedded => "native embedded (QuickJS)",
            Via::YtDlpEmbedded => "yt-dlp embedded",
            Via::YtDlpDefault => "yt-dlp default",
            Via::Saved => "saved",
            Via::Fixture => "local file",
        }
    }
}

/// How soon the player may ask for a song resolved ahead (`YtDlpResolver::resolve_ahead`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ahead {
    /// A row or card under the pointer, a page's first row, Home's featured song, the
    /// restored track, a refused preload: a click may come any moment.
    Soon,
    /// The queue's tracks after the next one: minutes away.
    Later,
    /// A song row on screen: it may be clicked, or not. Only the native step runs for it
    /// (`ResolveError::Deferred` past it), its failures don't count towards turning that
    /// step off, and an ad gate is waited out as for `Later`. Once the player asks, it goes
    /// on as any resolve does.
    Maybe,
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
    /// A resolve ahead of a click that may not come (`Ahead::Maybe`) stopped where it
    /// would have needed yt-dlp: the click resolves the song.
    Deferred,
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
            ResolveError::Deferred => f.write_str("not natively, left for a click"),
        }
    }
}

pub trait StreamResolver: Send + Sync {
    fn resolve(
        &self,
        id: &TrackId,
    ) -> impl Future<Output = Result<ResolvedStream, ResolveError>> + Send;
}
