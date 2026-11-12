use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use featherplay_core::model::TrackId;
use featherplay_core::sync::LockExt;
use serde_json::Value;
use tokio::sync::Notify;

use crate::chain::{Chain, Check, Steps};
use crate::expiry::query_param;
use crate::native::{NativeError, NativeResolver, VISIONOS};
use crate::sidecars::{Installer, tail};
use crate::{
    Ahead, Downloads, Preprocessor, Quality, ResolveError, ResolvedStream, Sidecars,
    StreamResolver, Via, expires_at_from_url,
};
use featherplay_proxy::Proxy;

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(45);

/// Reads the `-J` output of `yt-dlp -f 251/140` (or `-f 140`): yt-dlp copies the selected
/// format's fields to the top level. Format 251 is Opus in WebM, which the `opus-decoder`
/// crate in crates/audio decodes; 140 is AAC in m4a.
pub fn parse_ytdlp_json(json: &str, now: SystemTime) -> Result<ResolvedStream, ResolveError> {
    let v: Value = serde_json::from_str(json).map_err(|e| ResolveError::ToolFailed {
        code: 0,
        stderr_tail: format!("yt-dlp printed invalid JSON: {e}"),
    })?;
    if v.is_null() {
        return Err(ResolveError::Unavailable(
            "yt-dlp returned no video info".into(),
        ));
    }
    if v["has_drm"].as_bool() == Some(true) {
        return Err(ResolveError::Unavailable(
            "the stream is DRM-protected".into(),
        ));
    }
    let url = v["url"]
        .as_str()
        .filter(|u| u.starts_with("https://"))
        .ok_or_else(|| ResolveError::Unavailable("no stream URL for the selected format".into()))?;
    let itag = v["format_id"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| query_param(url, "itag")?.parse().ok())
        .ok_or_else(|| ResolveError::Unavailable("the selected format has no itag".into()))?;
    let mime = query_param(url, "mime")
        .filter(|m| m.contains('/'))
        .unwrap_or_else(|| {
            match v["ext"].as_str() {
                Some("m4a" | "mp4") => "audio/mp4",
                Some("webm") => "audio/webm",
                _ => "application/octet-stream",
            }
            .to_string()
        });
    let kbps = v["abr"]
        .as_f64()
        .or_else(|| v["tbr"].as_f64())
        .filter(|k| k.is_finite() && *k > 0.0);
    Ok(ResolvedStream {
        url: url.to_string(),
        mime,
        itag,
        bitrate_kbps: kbps.map_or(0, |k| k.round() as u32),
        content_length: v["filesize"]
            .as_u64()
            .or_else(|| query_param(url, "clen")?.parse().ok()),
        expires_at: expires_at_from_url(url, now),
        duration: v["duration"]
            .as_f64()
            .filter(|d| d.is_finite() && *d > 0.0)
            .map(Duration::from_secs_f64),
        available_at: v["available_at"]
            .as_u64()
            .and_then(crate::expiry::unix_secs),
        video_title: text(&v["title"]),
        first_tag: text(&v["tags"][0]),
        // yt-dlp's formats don't carry YouTube's `loudnessDb`.
        loudness_db: None,
        // The chain names the embedded client's runs (`Chain::embedded_answer`).
        via: Via::YtDlpDefault,
    })
}

/// A non-blank string field, trimmed.
fn text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Maps a failed yt-dlp run to `Unavailable` when its `ERROR:` line says the video can't be played,
/// else to `ToolFailed` (extraction broke, network, bot check), which an update may fix.
pub fn classify_failure(code: i32, stderr: &str) -> ResolveError {
    const UNPLAYABLE: &[&str] = &[
        "unavailable",
        "not available",
        "private video",
        "has been removed",
        "account associated with this video has been terminated",
        "copyright",
        "confirm your age",
        "members-only",
        "members only",
        "premium",
    ];
    if let Some(line) = stderr.lines().rev().find(|l| l.starts_with("ERROR:")) {
        let l = line.to_ascii_lowercase();
        // A missing format (251 or 140) usually means signature solving broke, not that the
        // track has none: yt-dlp says "Requested format is not available" for either.
        let tool_side = l.contains("http error") || l.contains("requested format");
        if !tool_side && UNPLAYABLE.iter().any(|p| l.contains(p)) {
            return ResolveError::Unavailable(line.trim_start_matches("ERROR:").trim().to_string());
        }
    }
    ResolveError::ToolFailed {
        code,
        stderr_tail: tail(stderr, 600),
    }
}

/// What `YtDlpResolver::on_helpers_wait` is told.
type Told = Box<dyn Fn(&TrackId) + Send + Sync>;

/// The embedded player's client: no PO token, and its URL plays at once unless the answer
/// had ads before the song (often since 2026-11-05: then 5 s later, which the chain waits
/// out). Only for videos that allow embedding. The webpage and the `next` call are skipped
/// (the title and tags come from the player response), and so are the HLS manifests:
/// ~2.5 s in all instead of ~4 s plus the ~5 s gate of the TV clients.
const EMBEDDED_ARGS: &str =
    "youtube:player_client=web_embedded;player_skip=webpage,initial_data;skip=hls";
/// Resolves streams, without cookies unless a cookie header is given (cookies made
/// resolving ~2x slower with the same result, measured 2026-10-26). The steps, in the
/// order and with the rules of `Chain::resolve`:
///
/// 1. The embedded player's client, asked from Rust (`native`: one request, ~0.3 s once
///    the player is cached). Not with cookies. It needs no helper: the player's challenges
///    run in the QuickJS built in.
/// 2. The same client through yt-dlp (`EMBEDDED_ARGS`, ~2.6 s).
/// 3. yt-dlp's default clients (the TV clients, which gate their URLs behind a pre-roll ad
///    until `available_at`).
///
/// Steps 2 and 3 need the helpers (yt-dlp, and deno as its JavaScript runtime): a resolve
/// waits for their download only when it gets to one of them (`helpers`).
pub struct YtDlpResolver {
    sidecars: Sidecars,
    cookie_header: Option<String>,
    chain: Chain,
    native: Option<NativeResolver>,
    /// The songs whose resolve waits for the helpers' download now.
    waiting: Mutex<Vec<TrackId>>,
    /// Told when a song's resolve starts to wait for it (`on_helpers_wait`).
    on_wait: Option<Told>,
    /// Told when a resolve ahead starts to wait out an ad gate (`on_gate_wait`).
    on_gate: Option<Told>,
    /// yt-dlp was forced up to date after a failure this session (`update_helpers`).
    forced: AtomicBool,
    /// Held while an update runs, so resolves failing together run one `yt-dlp -U`.
    updating: tokio::sync::Mutex<()>,
    /// Updates run so far, so a caller that waited for another's update can tell.
    updates: AtomicU64,
    /// Downloads the helpers when they're missing (`with_downloads`).
    installer: Option<Installer>,
    /// For `first_byte`, made at the first check.
    checks: OnceLock<Option<reqwest::Client>>,
    /// The `Quality` resolves ask for (`set_quality`), as `quality_to_u8`.
    quality: AtomicU8,
    /// yt-dlp's `--proxy`: the proxy the HTTP clients take for YouTube Music, asked at each
    /// run, as a proxy script's or auto-detection's answer comes after the start.
    proxy: fn() -> Option<Proxy>,
}

impl YtDlpResolver {
    pub fn new(sidecars: Sidecars, cookie_header: Option<String>) -> Self {
        let native = NativeResolver::new(sidecars.home().join("resolver"));
        Self {
            proxy: || featherplay_proxy::for_url(featherplay_proxy::YOUTUBE),
            ..Self::with_native(sidecars, cookie_header, native)
        }
    }

    fn with_native(
        sidecars: Sidecars,
        cookie_header: Option<String>,
        native: Option<NativeResolver>,
    ) -> Self {
        if cookie_header.is_some() {
            CookieFile::sweep(&sidecars.home().join("tmp"));
        }
        Self {
            sidecars,
            cookie_header,
            chain: Chain::default(),
            native,
            waiting: Mutex::default(),
            on_wait: None,
            on_gate: None,
            forced: AtomicBool::new(false),
            updating: tokio::sync::Mutex::new(()),
            updates: AtomicU64::new(0),
            installer: None,
            checks: OnceLock::new(),
            quality: AtomicU8::new(quality_to_u8(Quality::default())),
            proxy: || None,
        }
    }

    /// The format later resolves ask for; one running keeps the one it started with.
    pub fn set_quality(&self, q: Quality) {
        self.quality.store(quality_to_u8(q), Ordering::Relaxed);
    }

    fn quality(&self) -> Quality {
        match self.quality.load(Ordering::Relaxed) {
            1 => Quality::Standard,
            _ => Quality::High,
        }
    }

    /// Downloads yt-dlp and deno from `downloads` when they're missing: a resolve that
    /// gets to a yt-dlp step waits for them there (`helpers`).
    pub fn with_downloads(mut self, downloads: Downloads) -> Self {
        self.installer = Some(Installer::new(downloads));
        self
    }

    /// Where the native step preprocesses a new player (in this process unless set).
    pub fn with_preprocessor(mut self, preprocessor: Preprocessor) -> Self {
        if let Some(native) = &mut self.native {
            native.set_preprocessor(preprocessor);
        }
        self
    }

    /// `told(id)` runs when `id`'s resolve starts to wait for the helpers' download: its
    /// native step didn't give a stream, and yt-dlp isn't there yet.
    pub fn on_helpers_wait(mut self, told: impl Fn(&TrackId) + Send + Sync + 'static) -> Self {
        self.on_wait = Some(Box::new(told));
        self
    }

    /// `told(id)` runs when a resolve ahead of a click for `id` gets a native URL with ads
    /// before the song and starts to wait them out (~5-6 s, no request meanwhile).
    pub fn on_gate_wait(mut self, told: impl Fn(&TrackId) + Send + Sync + 'static) -> Self {
        self.on_gate = Some(Box::new(told));
        self
    }

    /// `id`'s resolve is waiting for the helpers' download now.
    pub fn waits_for_helpers(&self, id: &TrackId) -> bool {
        self.waiting.lock_or_recover().contains(id)
    }

    /// For the app's start: has the native step ready for the first song (a first run
    /// fetches and preprocesses its player now, `NativeResolver::prepare`). Nothing to do
    /// with cookies, which the native step doesn't take.
    pub async fn prepare(&self) {
        if self.cookie_header.is_none()
            && let Some(native) = &self.native
        {
            native.prepare().await;
        }
    }

    /// Downloads yt-dlp and deno if either is missing, or waits for the download running.
    /// The app starts it at startup, in the background: only the yt-dlp steps need them.
    /// Without `with_downloads`, a missing helper is `ToolMissing`.
    pub async fn install_helpers(&self) -> Result<(), ResolveError> {
        match &self.installer {
            Some(installer) => installer.install(&self.sidecars).await,
            None if self.sidecars.installed() => Ok(()),
            None => Err(ResolveError::ToolMissing),
        }
    }

    /// yt-dlp and deno are both there, so a resolve won't wait for a download.
    pub fn helpers_installed(&self) -> bool {
        self.sidecars.installed()
    }

    /// Brings yt-dlp up to date after a failure: at once the first time in a session, then
    /// only when the last update is a day old, so failing songs can't loop on `yt-dlp -U`.
    /// True when an update ran during the call (this one's, or one it waited for), so
    /// trying again may help; a failed update counts, the error being transient or not.
    pub async fn update_helpers(&self) -> bool {
        let before = self.updates.load(Ordering::SeqCst);
        let _held = self.updating.lock().await;
        if self.updates.load(Ordering::SeqCst) != before {
            return true;
        }
        let force = !self.forced.swap(true, Ordering::SeqCst);
        if !force && self.sidecars.updated_recently() {
            return false;
        }
        if let Err(e) = self.sidecars.update(force).await {
            log::warn!("yt-dlp -U failed: {e}");
        }
        self.updates.fetch_add(1, Ordering::SeqCst);
        true
    }

    /// The daily `yt-dlp -U`, when the helpers are there and the last update (or the first
    /// download) is a day old; never beside another update. The app asks at startup and when
    /// a song starts a day later. True when it ran (a failed run too: offline, it waits for
    /// the next ask). It doesn't count as the update after a failure (`update_helpers`).
    pub async fn update_daily(&self) -> bool {
        if !self.sidecars.installed() || self.sidecars.updated_recently() {
            return false;
        }
        let _held = self.updating.lock().await;
        if self.sidecars.updated_recently() {
            return false;
        }
        match self.sidecars.update_if_stale().await {
            Ok(()) => log::info!("yt-dlp -U (daily) done"),
            Err(e) => log::warn!("yt-dlp -U (daily) failed: {e}"),
        }
        self.updates.fetch_add(1, Ordering::SeqCst);
        true
    }

    /// `update_helpers` has run this session: another helpers update won't help now.
    pub fn helpers_tried(&self) -> bool {
        self.forced.load(Ordering::SeqCst)
    }

    /// Why `id`'s last resolve went to yt-dlp instead of the native request, once.
    pub fn take_fallback_reason(&self, id: &TrackId) -> Option<String> {
        self.chain.take_fallback_reason(id)
    }

    /// The stream URL this resolver gave for `id` was refused (HTTP 403) before it
    /// expired: see `Chain::refused`. The native step starts from a fresh embed page next
    /// time, checked; refused twice, `id` skips it for a while. A refused visionOS URL
    /// leaves the native step to the embedded client for a while
    /// (`NativeResolver::visionos_refused`), whichever step gave it (yt-dlp's default
    /// clients include visionOS).
    pub fn refused(&self, id: &TrackId, url: &str) {
        if query_param(url, "c").as_deref() == Some(VISIONOS) {
            if let Some(n) = &self.native {
                n.visionos_refused();
            }
        } else if self.chain.refused(id, url)
            && let Some(n) = &self.native
        {
            n.refused();
        }
    }

    /// `resolve`, ahead of a click: its yt-dlp steps wait for those of other resolves
    /// ahead, so one yt-dlp runs ahead at a time. Notify `urgent` when the player asks for
    /// `id` meanwhile: the resolve then stops waiting. `ahead`: how soon that may be.
    pub async fn resolve_ahead(
        &self,
        id: &TrackId,
        urgent: &Notify,
        ahead: Ahead,
    ) -> Result<ResolvedStream, ResolveError> {
        self.run_chain(id, Some((urgent, ahead))).await
    }

    /// One step of `resolve` on its own, without cookies: no fallback, no `yt-dlp -U`, no
    /// wait for `available_at`. For the play probe (`crates/audio/tests/it/play_probe.rs`),
    /// which measures which steps get through from where. The error is the step's own text
    /// (the native refusal, or yt-dlp's exit code and stderr tail).
    pub async fn probe(&self, id: &TrackId, step: Probe<'_>) -> Result<ResolvedStream, String> {
        let quality = self.quality();
        let args = match step {
            Probe::Native => {
                let native = self
                    .native
                    .as_ref()
                    .ok_or("no HTTP client for the native step")?;
                return native
                    .resolve(&id.0, quality)
                    .await
                    .map_err(|e| e.to_string());
            }
            Probe::Embedded => Some(EMBEDDED_ARGS),
            Probe::Default => None,
            Probe::Args(a) => Some(a),
        };
        self.run(id, None, args, quality)
            .await
            .map_err(|e| e.to_string())
    }

    async fn run(
        &self,
        id: &TrackId,
        cookies: Option<&CookieFile>,
        extractor_args: Option<&str>,
        quality: Quality,
    ) -> Result<ResolvedStream, ResolveError> {
        if !self.sidecars.ytdlp().is_file() || !self.sidecars.deno().is_file() {
            return Err(ResolveError::ToolMissing);
        }
        let mut cmd = self.command(id, cookies, extractor_args, quality);
        let out = match tokio::time::timeout(RESOLVE_TIMEOUT, cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ResolveError::ToolMissing);
            }
            Ok(Err(e)) => {
                return Err(ResolveError::ToolFailed {
                    code: -1,
                    stderr_tail: format!("spawning yt-dlp: {e}"),
                });
            }
            Err(_) => {
                return Err(ResolveError::ToolFailed {
                    code: -1,
                    stderr_tail: "yt-dlp timed out".into(),
                });
            }
        };
        if !out.status.success() {
            return Err(classify_failure(
                out.status.code().unwrap_or(-1),
                &String::from_utf8_lossy(&out.stderr),
            ));
        }
        parse_ytdlp_json(&String::from_utf8_lossy(&out.stdout), SystemTime::now())
    }

    /// One yt-dlp run for `quality`'s formats of `id`, printing its JSON. Warnings stay on
    /// (stderr): when the format is missing, they say why (a challenge not solved, formats skipped
    /// for a PO token or SABR), and a failure's log line ends with them.
    fn command(
        &self,
        id: &TrackId,
        cookies: Option<&CookieFile>,
        extractor_args: Option<&str>,
        quality: Quality,
    ) -> tokio::process::Command {
        let mut cmd = self.sidecars.command(&self.sidecars.ytdlp());
        cmd.args(["--ignore-config", "--no-playlist", "-J", "-f"])
            .arg(quality.formats("/"))
            .arg("--cache-dir")
            .arg(self.sidecars.home().join("yt-dlp-cache"))
            .arg("--js-runtimes")
            .arg(format!("deno:{}", self.sidecars.deno().display()));
        if let Some(c) = cookies {
            cmd.arg("--cookies").arg(&c.path);
        }
        if let Some(a) = extractor_args {
            cmd.arg("--extractor-args").arg(a);
        }
        if let Some(proxy) = (self.proxy)()
            && let Some(p) = proxy.argument()
        {
            cmd.arg("--proxy").arg(p);
        }
        cmd.arg(format!("https://music.youtube.com/watch?v={}", id.0));
        cmd
    }
}

impl StreamResolver for YtDlpResolver {
    async fn resolve(&self, id: &TrackId) -> Result<ResolvedStream, ResolveError> {
        self.run_chain(id, None).await
    }
}

impl YtDlpResolver {
    async fn run_chain(
        &self,
        id: &TrackId,
        ahead: Option<(&Notify, Ahead)>,
    ) -> Result<ResolvedStream, ResolveError> {
        if id.0.is_empty()
            || !id
                .0
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(ResolveError::Unavailable(format!(
                "not a video id: {:?}",
                id.0
            )));
        }
        let quality = self.quality();
        // Lives for every step; dropping it deletes the file, on every return path.
        let cookies = match &self.cookie_header {
            Some(header) => Some(
                CookieFile::write(&self.sidecars.home().join("tmp"), header).map_err(|e| {
                    ResolveError::ToolFailed {
                        code: -1,
                        stderr_tail: format!("writing the cookie file: {e}"),
                    }
                })?,
            ),
            None => None,
        };
        let steps = Live {
            r: self,
            cookies: cookies.as_ref(),
            quality,
            aside: matches!(ahead, Some((_, Ahead::Maybe))),
        };
        match ahead {
            Some((urgent, ahead)) => self.chain.resolve_ahead(&steps, id, urgent, ahead).await,
            None => self.chain.resolve(&steps, id).await,
        }
    }

    /// Before a yt-dlp step of `id`'s resolve: waits for the helpers when they aren't
    /// there (a first run whose native step gave no stream), or for the download already
    /// running for them, and tells `on_wait` so. Without `with_downloads` nothing is
    /// waited for: the step answers `ToolMissing` itself.
    async fn helpers(&self, id: &TrackId) -> Result<(), ResolveError> {
        let Some(installer) = &self.installer else {
            return Ok(());
        };
        if self.sidecars.installed() {
            return Ok(());
        }
        log::info!("resolve {}: waiting for yt-dlp and deno to download", id.0);
        self.waiting.lock_or_recover().push(id.clone());
        // Off the list on every way out, a cancelled resolve's too.
        let _waiting = Waiting(&self.waiting, id);
        if let Some(told) = &self.on_wait {
            told(id);
        }
        installer.install(&self.sidecars).await
    }
}

/// A song on `YtDlpResolver::waiting`, taken off when dropped.
struct Waiting<'a>(&'a Mutex<Vec<TrackId>>, &'a TrackId);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut waiting = self.0.lock_or_recover();
        if let Some(i) = waiting.iter().position(|id| id == self.1) {
            waiting.swap_remove(i);
        }
    }
}

/// The steps as `resolve` runs them, for `Chain::resolve`.
struct Live<'a> {
    r: &'a YtDlpResolver,
    cookies: Option<&'a CookieFile>,
    /// Read once when the resolve starts.
    quality: Quality,
    /// A song prepared in case it's clicked (`Ahead::Maybe`): its native failure doesn't
    /// count towards turning the native step off.
    aside: bool,
}

impl Steps for Live<'_> {
    async fn native(&self, id: &TrackId) -> Option<Result<ResolvedStream, NativeError>> {
        if self.cookies.is_some() {
            return None;
        }
        let native = self.r.native.as_ref()?;
        Some(if self.aside {
            native.resolve_aside(&id.0, self.quality).await
        } else {
            native.resolve(&id.0, self.quality).await
        })
    }

    async fn native_fresh(&self, id: &TrackId) -> Option<Result<ResolvedStream, NativeError>> {
        if self.cookies.is_some() {
            return None;
        }
        Some(
            self.r
                .native
                .as_ref()?
                .resolve_fresh(&id.0, self.quality)
                .await,
        )
    }

    fn helpers_ready(&self) -> bool {
        self.r.installer.is_none() || self.r.sidecars.installed()
    }

    async fn helpers_downloaded(&self) {
        match &self.r.installer {
            Some(installer) if installer.install(&self.r.sidecars).await.is_ok() => {}
            _ => std::future::pending().await,
        }
    }

    async fn embedded(&self, id: &TrackId) -> Result<ResolvedStream, ResolveError> {
        self.r.helpers(id).await?;
        self.r
            .run(id, self.cookies, Some(EMBEDDED_ARGS), self.quality)
            .await
    }

    async fn default_clients(&self, id: &TrackId) -> Result<ResolvedStream, ResolveError> {
        self.r.helpers(id).await?;
        self.r.run(id, self.cookies, None, self.quality).await
    }

    async fn update(&self) -> bool {
        self.r.update_helpers().await
    }

    async fn check(&self, url: &str) -> Check {
        let check = match self.r.checks.get_or_init(check_client) {
            Some(http) => first_byte(http, url).await,
            None => Check::Unknown,
        };
        if check == Check::Refused
            && query_param(url, "c").as_deref() == Some(VISIONOS)
            && let Some(n) = &self.r.native
        {
            n.visionos_refused();
        }
        check
    }

    fn waits_at_gate(&self, id: &TrackId) {
        if let Some(told) = &self.r.on_gate {
            told(id);
        }
    }
}

/// A check that takes longer is no answer (a normal one takes ~0.2 s).
const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// The client for `first_byte`: like the engine's (no User-Agent), and keeping no
/// connection idle, so no reaper timer runs while paused.
fn check_client() -> Option<reqwest::Client> {
    featherplay_proxy::client()
        .connect_timeout(CHECK_TIMEOUT)
        .pool_max_idle_per_host(0)
        .build()
        .ok()
}

/// One `Range: bytes=0-0` request for `url`: the engine's first request, for one byte.
async fn first_byte(http: &reqwest::Client, url: &str) -> Check {
    let answer = http
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .timeout(CHECK_TIMEOUT)
        .send()
        .await;
    match answer {
        Ok(r) if r.status().is_success() => Check::Plays,
        Ok(r) if r.status() == reqwest::StatusCode::FORBIDDEN => Check::Refused,
        _ => Check::Unknown,
    }
}

fn quality_to_u8(q: Quality) -> u8 {
    match q {
        Quality::High => 0,
        Quality::Standard => 1,
    }
}

/// A step for `YtDlpResolver::probe`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe<'a> {
    /// Step 1: the embedded client asked from Rust.
    Native,
    /// Step 2: the embedded client through yt-dlp, with the app's `--extractor-args`.
    Embedded,
    /// Step 3: yt-dlp's default clients.
    Default,
    /// yt-dlp with these `--extractor-args`, such as `youtube:player_client=tv`.
    Args(&'a str),
}

/// A Netscape cookie file for yt-dlp's `--cookies`: plaintext, so it lives only for one
/// resolve and is deleted on drop. The app never passes cookies to yt-dlp.
struct CookieFile {
    path: PathBuf,
}

impl CookieFile {
    fn write(dir: &Path, header: &str) -> std::io::Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        std::fs::create_dir_all(dir)?;
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        // The guard exists before the write, so a partial file is removed too.
        let file = Self {
            path: dir.join(format!("cookies-{}-{nanos}-{n}.txt", std::process::id())),
        };
        std::fs::write(&file.path, netscape_cookies(header))?;
        Ok(file)
    }
}

impl CookieFile {
    /// Deletes cookie files that other (killed) processes left in `dir`.
    fn sweep(dir: &Path) {
        let ours = format!("cookies-{}-", std::process::id());
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with("cookies-") && name.ends_with(".txt") && !name.starts_with(&ours) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

impl Drop for CookieFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `name=value; name2=value2` as `.youtube.com` cookies, valid for a year.
fn netscape_cookies(header: &str) -> String {
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + 365 * 24 * 3600;
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for pair in header.split(';') {
        let Some((name, value)) = pair.trim().split_once('=') else {
            continue;
        };
        let clean = |s: &str| s.trim().replace(['\t', '\r', '\n'], "");
        let (name, value) = (clean(name), clean(value));
        if !name.is_empty() {
            out.push_str(&format!(
                ".youtube.com\tTRUE\t/\tTRUE\t{expiry}\t{name}\t{value}\n"
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests;
