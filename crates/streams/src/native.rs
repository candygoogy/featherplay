//! Resolves a stream with one InnerTube `player` request made from Rust, as the embedded
//! player's client (`WEB_EMBEDDED_PLAYER`, yt-dlp's `web_embedded`), instead of a yt-dlp run.
//! A yt-dlp run takes ~2.6 s: 0.6 s to start, the embed page, the player JS, the player API,
//! 0.6 s to parse the whole player in deno, 0.35 s to exit.
//!
//! - The embed page gives the client version, the visitor data, the host flags and the
//!   player's version. They're kept in `{home}/resolver/state.json` and fetched again after
//!   `REFRESH_AFTER`, in the background, after a resolve that used the old ones.
//! - Each player version is downloaded and preprocessed once, with yt-dlp's EJS solver
//!   scripts (`ejs/`) in QuickJS (`js`), into `{home}/resolver/players/{id}.js`: the cached
//!   transform, with its bytecode beside it. The last `KEEP_PLAYERS` are kept.
//!   Preprocessing takes seconds, so it never sits in front of a song: a page that names
//!   a new player is answered with an older one while the new one is made in the
//!   background, and a first run makes its player at startup (`prepare`).
//! - A resolve then loads that player in QuickJS on a thread of its own (~20 ms from the
//!   bytecode) while the player API request runs, and hands it the `n` and `sig`
//!   challenges once the answer is in (~1 ms).
//! - Before that, each resolve asks as the visionOS app (`VISIONOS`, yt-dlp's `visionos`),
//!   with the kept page's visitor. Its answers come without ads before the song, and its
//!   URLs need no challenges, so it needs the page but not its player (it still waits for
//!   a resolve's page and player, as the embedded client does). Any answer it gives no
//!   stream with goes on to the embedded client.
//!
//! `YtDlpResolver` falls back to yt-dlp on any error here.

mod embed;
mod players;
mod request;
/// A one-thread HTTP/1.1 server for tests: answers each request with
/// `route(method, path, body)` and logs `"METHOD path"` of each.
#[cfg(test)]
pub(crate) mod test_server;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use featherplay_core::sync::LockExt;
use serde_json::Value;

use crate::js::{self, Solver};
use crate::{Preprocessor, Quality, ResolvedStream, Via};
use embed::parse_embed_page;
use players::{PreprocessError, install, installed, newest, player_sts, warm};
pub(crate) use request::Answers;
use request::{
    finish, parse_player_response, player_headers, player_request_body, visionos_headers,
    visionos_request_body,
};

const ORIGIN: &str = "https://www.youtube.com";
const CLIENT_NAME: &str = "WEB_EMBEDDED_PLAYER";
const CLIENT_ID: &str = "56";
/// yt-dlp's (2026.10.26), for an embed page that has none.
const CLIENT_VERSION: &str = "2.20260708.00.00";
/// yt-dlp's for this client.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15,gzip(gfe)";
/// The page the player is "embedded" in: any page outside YouTube (yt-dlp's choice).
const EMBED_URL: &str = "https://www.reddit.com/";
/// The client asked first: yt-dlp's `visionos` (one of its default clients since
/// 2026.08.19) and NewPipe's only one. On 2026-11-10 none of its 43 answers had ads before
/// the song (the embedded client's: 12 of 22), and its URLs played to the end from their
/// first byte, with no PO token. It wants a visitor: without one it asks for a sign-in
/// ("confirm you're not a bot"). ANDROID_VR and TVHTML5_SIMPLY answered without ads too,
/// but without a PO token their URLs were refused after the first MiB.
pub(crate) const VISIONOS: &str = "VISIONOS";
const VISIONOS_ID: &str = "101";
/// yt-dlp's (2026.10.26).
const VISIONOS_VERSION: &str = "1.02";
const VISIONOS_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
/// After this many visionOS answers in a row without a stream, resolves ask the embedded
/// client alone for `OFF_FOR`, and then one more failure turns it off again, so a client
/// YouTube changed (or a kept visitor it won't answer) costs no request per song. Songs it
/// can't give (made for kids, no such format) don't count.
const VISIONOS_MAX_FAILURES: u32 = 3;
/// Its answers took 91-154 ms (2026-11-10): one that takes longer goes on to the embedded
/// client rather than hold the song up to `REQUEST_TIMEOUT`.
const VISIONOS_TIMEOUT: Duration = Duration::from_secs(3);
/// How long resolves ask the embedded client alone after a visionOS URL was refused: the
/// way YouTube refused the other clients' URLs without a PO token (after the first MiB).
const VISIONOS_REFUSED_FOR: Duration = Duration::from_secs(30 * 60);
const REFRESH_AFTER: Duration = Duration::from_secs(12 * 3600);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);
/// When the process preprocessing a player is killed. Its own limit (`js`, 30 s) ends it
/// first and says why; this is for one that hangs.
const PREPROCESS_TIMEOUT: Duration = Duration::from_secs(40);
const KEEP_PLAYERS: usize = 3;
/// A temporary file in the players' folder this old belongs to no save still running.
const STRAY_AFTER: Duration = Duration::from_secs(600);
/// After this many failures in a row, resolves go straight to yt-dlp for `OFF_FOR`, and
/// then one failure turns it off again. Resolves at once from one page that fail count
/// once: on 2026-11-09 a start's three resolves failed together on one bad visitor, and
/// 30 min off cost every song 2.6-10.8 s, while each fresh visitor got URLs.
const MAX_FAILURES: u32 = 3;
const OFF_FOR: Duration = Duration::from_secs(5 * 60);
/// How often the solver may not answer for one player before it counts as one that can't
/// be solved: each time holds a resolve up to its watchdog (`Solver::solve`, ~30 s).
const SOLVER_TIMEOUTS: u32 = 2;

/// Why a native resolve didn't give a stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NativeError {
    /// The embedded client won't give this video (not embeddable, sign-in, none of the
    /// formats wanted):
    /// yt-dlp's embedded run would fail the same way, so its other clients go next.
    Skip(String),
    /// Something on our side or the network failed: yt-dlp gets the whole resolve.
    Failed(String),
    /// The format wanted came without a URL (a SABR-only answer). yt-dlp gets the whole
    /// resolve, as for `Failed`; the same song tends to get such answers again.
    NoUrl(String),
    /// visionOS and the embedded client both said the song isn't available (`VISIONOS_REFUSAL`):
    /// YouTube won't give it to any client but its own music web player now. visionOS's
    /// words.
    NotAvailable(String),
}

impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skip(m) => write!(f, "not with the embedded client: {m}"),
            Self::Failed(m) | Self::NoUrl(m) => write!(f, "native resolve failed: {m}"),
            Self::NotAvailable(m) => write!(f, "not available to visionOS nor the embed: {m}"),
        }
    }
}

fn failed(m: impl std::fmt::Display) -> NativeError {
    NativeError::Failed(m.to_string())
}

/// What the embed page tells about the embedded client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EmbedConfig {
    pub client_version: String,
    pub visitor_data: Option<String>,
    /// `encryptedHostFlags`, which the embedded player sends in its playback context.
    pub host_flags: Option<String>,
    /// The player's version, as in `/s/player/{player}/`.
    pub player: String,
}

/// A player preprocessed by EJS, on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Player {
    pub id: String,
    /// `signatureTimestamp`: the `sig` challenges the server sends are made for this player.
    pub sts: u64,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
struct Ready {
    config: EmbedConfig,
    /// Usually `config.player`; an older one while the new one is being preprocessed, or
    /// when it couldn't be.
    player: Player,
    fetched: SystemTime,
    /// Which page this is in this run (`Inner::pages`): a newer one has a higher number.
    page: u64,
}

/// The `n`/`sig` resolve's fields before the challenges are answered.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pending {
    /// The format picked: the first one wanted that the answer has.
    pub itag: u32,
    /// The format's URL, with the `n` challenge in it and no signature yet.
    pub url: String,
    pub n: Option<String>,
    /// The query parameter the signature goes in, and the `s` challenge.
    pub sig: Option<(String, String)>,
    pub mime: String,
    pub bitrate_kbps: u32,
    pub content_length: Option<u64>,
    pub duration: Option<Duration>,
    pub video_title: Option<String>,
    pub first_tag: Option<String>,
    pub loudness_db: Option<f32>,
    /// How long the URL is refused for (HTTP 403) after the answer: the ads before the
    /// song, until each could be skipped.
    pub gate: Option<Duration>,
}

pub(crate) struct NativeResolver {
    inner: Arc<Inner>,
}

struct Inner {
    /// Made again when "Look up YouTube directly" changes: it keeps its connections open
    /// with no idle timeout, and one to the old route would go on being used.
    http: featherplay_proxy::Fresh,
    /// `{home}/resolver`
    dir: PathBuf,
    base: String,
    preprocessor: Preprocessor,
    /// The players being installed in the background, so each is installed once at a time.
    installing: Mutex<HashSet<String>>,
    state: tokio::sync::Mutex<Option<Ready>>,
    /// The page to drop from `state` before the next resolve (a URL was refused, or the
    /// request failed).
    forget: Mutex<Option<Forget>>,
    /// The last page number given (`Ready::page`).
    pages: AtomicU64,
    refreshing: AtomicBool,
    health: Mutex<Health>,
}

/// Which kept page the next resolve drops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Forget {
    /// Whichever is kept: a URL it gave was refused.
    Any,
    /// This page (`Ready::page`) or an older one: an answer from it failed. Resolves at
    /// once from one page fail one after the other, and the first may have kept a fresh
    /// page meanwhile.
    Page(u64),
}

#[derive(Default)]
struct Health {
    failures: u32,
    /// The page of the last answer without a URL: another one from it doesn't count.
    failed_page: Option<u64>,
    off_until: Option<Instant>,
    /// Players whose challenges couldn't be solved.
    bad: HashSet<String>,
    /// How often the solver didn't answer for each player (`solve_failed`).
    solver_timeouts: HashMap<String, u32>,
    /// visionOS answers in a row without a stream.
    visionos_failures: u32,
    visionos_off_until: Option<Instant>,
    /// The songs visionOS said were unavailable since its last stream, the last
    /// `VISIONOS_MAX_FAILURES` at most.
    visionos_unavailable: Vec<String>,
}

impl Health {
    fn visionos_on(&self) -> bool {
        self.visionos_off_until
            .is_none_or(|until| Instant::now() >= until)
    }

    /// Counts visionOS's answer for `video`; enough in a row without a stream turn the
    /// client off. "This video is unavailable" counts once it's said of different songs
    /// in a row: for one song it's the song's, for several it's what a client config
    /// YouTube no longer takes gets. `counts`: not for a song prepared aside
    /// (`resolve_aside`), so a page of them can't turn it off for the clicks.
    fn visionos_answered(
        &mut self,
        video: &str,
        answer: &Result<ResolvedStream, NativeError>,
        counts: bool,
    ) {
        match answer {
            Ok(_) => {
                self.visionos_failures = 0;
                self.visionos_unavailable.clear();
            }
            Err(_) if !counts => {}
            Err(NativeError::Failed(m)) if m.starts_with("ERROR:") => {
                let songs = &mut self.visionos_unavailable;
                if !songs.iter().any(|v| v == video) {
                    songs.push(video.to_string());
                    if songs.len() > VISIONOS_MAX_FAILURES as usize {
                        songs.remove(0);
                    }
                }
                if songs.len() >= VISIONOS_MAX_FAILURES as usize {
                    self.visionos_failures = VISIONOS_MAX_FAILURES - 1;
                    self.visionos_off(OFF_FOR);
                }
            }
            Err(e) if song_not_client(e) => {}
            Err(_) => {
                self.visionos_failures += 1;
                if self.visionos_failures >= VISIONOS_MAX_FAILURES {
                    // The first resolve after the time off tries once.
                    self.visionos_failures = VISIONOS_MAX_FAILURES - 1;
                    self.visionos_off(OFF_FOR);
                }
            }
        }
    }
}

/// Whether a visionOS answer without a stream is the song's doing rather than the client's:
/// made for kids, no such format, DRM, an age check, or "This video is unavailable" (for
/// one song: `Health::visionos_answered`). A sign-in it asks for otherwise ("confirm you're
/// not a bot") is the client's.
fn song_not_client(e: &NativeError) -> bool {
    match e {
        NativeError::Skip(m) => {
            !m.starts_with("LOGIN_REQUIRED")
                || m.contains("your age")
                || m.contains("inappropriate")
        }
        NativeError::Failed(m) => m.starts_with("ERROR:"),
        NativeError::NoUrl(_) => false,
        NativeError::NotAvailable(_) => true,
    }
}

/// How visionOS refuses a song itself, in its `UNPLAYABLE` reason: "This video is not
/// available". Since 2026-11-11 YouTube answers so for some songs to every client but its
/// music web player, which needs PO tokens (`NativeError::NotAvailable`).
const VISIONOS_REFUSAL: &[&str] = &["this video is not available"];
/// How the embedded client refuses the same songs: "Video unavailable". It says so of a
/// video that can't be embedded too, so it counts only after visionOS's refusal.
const EMBED_REFUSAL: &[&str] = &["not available", "unavailable"];

/// Whether `e` is an `UNPLAYABLE` answer whose reason has one of `words` (lower case).
fn unplayable(e: &NativeError, words: &[&str]) -> bool {
    let NativeError::Skip(m) = e else {
        return false;
    };
    let low = m.to_ascii_lowercase();
    low.starts_with("unplayable") && words.iter().any(|w| low.contains(w))
}

impl Health {
    /// `player`'s solve failed with `e`: it can't be solved (`bad`) when its challenges
    /// threw or ran past the solver's own limit, or when the solver didn't answer for it
    /// `SOLVER_TIMEOUTS` times (once may be a busy machine). A solver that stopped (its
    /// thread couldn't start) says nothing of the player.
    fn solve_failed(&mut self, player: &str, e: &str) {
        let bad = match e {
            js::STOPPED => false,
            js::TIMED_OUT => {
                let n = self.solver_timeouts.entry(player.to_string()).or_default();
                *n += 1;
                *n >= SOLVER_TIMEOUTS
            }
            _ => true,
        };
        if bad {
            self.bad.insert(player.to_string());
        }
    }

    /// visionOS off for `time`, or for longer if it's off for longer already.
    fn visionos_off(&mut self, time: Duration) {
        let until = Instant::now() + time;
        self.visionos_off_until = Some(self.visionos_off_until.map_or(until, |t| t.max(until)));
    }
}

impl NativeResolver {
    /// `dir`: where the state and the players go (`{home}/resolver`). `None` when no HTTP
    /// client can be built (then yt-dlp does every resolve).
    pub(crate) fn new(dir: PathBuf) -> Option<Self> {
        Self::with_dirs(dir, ORIGIN)
    }

    /// `base`: `https://www.youtube.com`, or a test server.
    pub(crate) fn with_dirs(dir: PathBuf, base: &str) -> Option<Self> {
        let http = featherplay_proxy::Fresh::new(|| {
            featherplay_proxy::client()
                .user_agent(USER_AGENT)
                .connect_timeout(Duration::from_secs(5))
                // No idle timeout, so no reaper timer runs while paused: an idle connection
                // stays until the server closes it, and a click minutes later may reuse it.
                .pool_idle_timeout(None::<Duration>)
                .pool_max_idle_per_host(2)
                .build()
        })
        .ok()?;
        Some(Self {
            inner: Arc::new(Inner {
                http,
                dir,
                base: base.trim_end_matches('/').to_string(),
                preprocessor: Preprocessor::InProcess,
                installing: Mutex::default(),
                state: tokio::sync::Mutex::new(None),
                forget: Mutex::new(None),
                pages: AtomicU64::new(0),
                refreshing: AtomicBool::new(false),
                health: Mutex::new(Health::default()),
            }),
        })
    }

    /// How new players are preprocessed, set before the first resolve.
    pub(crate) fn set_preprocessor(&mut self, preprocessor: Preprocessor) {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.preprocessor = preprocessor;
        }
    }

    /// A URL this client gave was refused: the next resolve starts from a fresh embed page.
    pub(crate) fn refused(&self) {
        self.inner.drop_page(None);
    }

    /// A visionOS URL was refused: resolves ask the embedded client alone for
    /// `VISIONOS_REFUSED_FOR`.
    pub(crate) fn visionos_refused(&self) {
        log::info!(
            "a visionOS URL was refused: the embedded client alone for {} min",
            VISIONOS_REFUSED_FOR.as_secs() / 60
        );
        let mut h = self.inner.health.lock_or_recover();
        h.visionos_failures = VISIONOS_MAX_FAILURES - 1;
        h.visionos_off(VISIONOS_REFUSED_FOR);
    }

    /// For the app's start. With no player to answer with (a first run, or players made by
    /// other EJS scripts), fetches an embed page and installs its player now, so the first
    /// song doesn't wait the seconds that takes; a resolve that comes meanwhile waits for
    /// this install instead of starting its own. With a player, nothing is asked: the first
    /// resolve does what it did.
    pub(crate) async fn prepare(&self) {
        let inner = &self.inner;
        // What deno ran until 0.2.1.
        let _ = std::fs::remove_file(inner.dir.join("solve.js"));
        let mut st = inner.state.lock().await;
        if st.is_some() {
            return;
        }
        if let Some(player) = newest(&inner.dir.join("players"), |_| true) {
            drop(st);
            // An update left its bytecode to another build: made again now, not in front
            // of the first song (~0.2 s).
            let path = player.path.clone();
            let stale = tokio::task::spawn_blocking(move || !js::has_bytecode(&path));
            if stale.await.unwrap_or(true) {
                warm(&player).await;
            }
            return;
        }
        let started = Instant::now();
        match inner.fetch_ready("", None).await {
            Ok(ready) => {
                log::info!(
                    "player {} ready {:.1}s after the start",
                    ready.player.id,
                    started.elapsed().as_secs_f64()
                );
                inner.save(&ready);
                *st = Some(ready);
            }
            Err(e) => log::warn!("no player yet, the first song's resolve tries again: {e}"),
        }
    }

    pub(crate) async fn resolve(
        &self,
        video: &str,
        quality: Quality,
    ) -> Result<ResolvedStream, NativeError> {
        self.resolve_counting(video, quality, true).await
    }

    /// `resolve` for a song prepared in case it's clicked (`Ahead::Maybe`): its failure
    /// doesn't count towards turning the step off, so a page of rows prepared ahead can't
    /// turn it off for the clicks.
    pub(crate) async fn resolve_aside(
        &self,
        video: &str,
        quality: Quality,
    ) -> Result<ResolvedStream, NativeError> {
        self.resolve_counting(video, quality, false).await
    }

    async fn resolve_counting(
        &self,
        video: &str,
        quality: Quality,
        counts: bool,
    ) -> Result<ResolvedStream, NativeError> {
        let inner = &self.inner;
        if inner
            .health
            .lock_or_recover()
            .off_until
            .is_some_and(|t| Instant::now() < t)
        {
            return Err(failed("off for a while after repeated failures"));
        }
        let (result, page) = inner.resolve(video, quality, counts).await;
        let mut h = inner.health.lock_or_recover();
        match &result {
            Ok(_) | Err(NativeError::Skip(_) | NativeError::NotAvailable(_)) => {
                h.failures = 0;
                h.failed_page = None;
            }
            Err(_) if !counts => {}
            Err(e @ (NativeError::Failed(_) | NativeError::NoUrl(_))) => {
                // Other failures (a request, a player's challenges) count each time.
                let page = page.filter(|_| matches!(e, NativeError::NoUrl(_)));
                if page.is_none() || page != h.failed_page {
                    h.failures += 1;
                    h.failed_page = page;
                    if h.failures >= MAX_FAILURES {
                        // The first resolve after the time off tries once.
                        h.failures = MAX_FAILURES - 1;
                        h.off_until = Some(Instant::now() + OFF_FOR);
                    }
                }
            }
        }
        drop(h);
        if result.is_ok() {
            self.refresh_if_stale(video);
        }
        result
    }

    /// `resolve` from a fresh embed page (a new visitor), for this answer only: the kept
    /// page, the failure count and the next resolves stay as they were. For a gated answer
    /// the player waits for (`Chain`): the page's player is the kept one unless YouTube
    /// moved on, then the kept one answers and the new one is installed in the background.
    /// Its URL refused later drops the kept page as any refused URL does (`refused`): rare,
    /// and a fresh page is cheap.
    pub(crate) async fn resolve_fresh(
        &self,
        video: &str,
        quality: Quality,
    ) -> Result<ResolvedStream, NativeError> {
        let inner = &self.inner;
        if inner
            .health
            .lock_or_recover()
            .off_until
            .is_some_and(|t| Instant::now() < t)
        {
            return Err(failed("off for a while after repeated failures"));
        }
        // Not waiting for a resolve that holds the state while it fetches a page.
        let kept = inner
            .state
            .try_lock()
            .ok()
            .and_then(|st| st.as_ref().map(|r| r.player.clone()));
        let ready = inner.fetch_ready(video, kept).await?;
        inner.answer(video, quality, &ready, false).await
    }

    /// Fetches the embed page again in the background once the one in use is old.
    fn refresh_if_stale(&self, video: &str) {
        let inner = self.inner.clone();
        let stale = match inner.state.try_lock() {
            Ok(st) => st
                .as_ref()
                .is_some_and(|r| r.fetched.elapsed().unwrap_or_default() > REFRESH_AFTER),
            Err(_) => false,
        };
        if !stale || inner.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let video = video.to_string();
        tokio::spawn(async move {
            let old = inner.state.lock().await.as_ref().map(|r| r.player.clone());
            if let Ok(mut ready) = inner.fetch_ready(&video, old).await {
                let mut st = inner.state.lock().await;
                // Its page's player may have been installed meanwhile (`install_behind`).
                if let Some(own) = installed(&inner.dir.join("players"), &ready.config.player) {
                    ready.player = own;
                }
                inner.save(&ready);
                *st = Some(ready);
            }
            inner.refreshing.store(false, Ordering::Release);
        });
    }
}

impl Inner {
    /// The next resolve starts from a fresh embed page, unless `page` (`None`: whichever is
    /// kept) is one another resolve already replaced. The saved page goes at once: some
    /// new visitors get answers without URLs every time (2026-11-09), and a session that
    /// ends before its next resolve would load it again at the next start.
    fn drop_page(&self, page: Option<&Ready>) {
        if page.is_none_or(|r| self.saved().is_some_and(|(c, _)| c == r.config)) {
            let _ = std::fs::remove_file(self.dir.join("state.json"));
        }
        let mut forget = self.forget.lock_or_recover();
        *forget = Some(match (*forget, page) {
            (Some(Forget::Any), _) | (_, None) => Forget::Any,
            (Some(Forget::Page(old)), Some(r)) => Forget::Page(old.max(r.page)),
            (None, Some(r)) => Forget::Page(r.page),
        });
    }

    /// A resolve from the kept page, and that page (`Ready::page`): the visionOS client's
    /// answer, else the embedded client's; both refusing the song (`VISIONOS_REFUSAL`) is
    /// `NotAvailable`. An embedded answer without a URL asks once more
    /// from a fresh page (~0.45 s): some visitors get none every time while new ones do
    /// (2026-11-09). Resolves at once that get none share the first one's fresh page.
    /// `counts`: a visionOS failure counts towards turning it off (`Health::visionos_answered`).
    async fn resolve(
        self: &Arc<Self>,
        video: &str,
        quality: Quality,
        counts: bool,
    ) -> (Result<ResolvedStream, NativeError>, Option<u64>) {
        let ready = match self.ready(video, None).await {
            Ok(r) => r,
            Err(e) => return (Err(e), None),
        };
        // What visionOS said, when it refused the song.
        let mut refused: Option<String> = None;
        if self.health.lock_or_recover().visionos_on() {
            let answer = self.visionos(video, quality, &ready).await;
            self.health
                .lock_or_recover()
                .visionos_answered(video, &answer, counts);
            match answer {
                Ok(stream) => return (Ok(stream), Some(ready.page)),
                Err(e) => {
                    let (NativeError::Skip(m)
                    | NativeError::Failed(m)
                    | NativeError::NoUrl(m)
                    | NativeError::NotAvailable(m)) = &e;
                    log::info!("resolve {video}: no visionOS stream ({m}), asking as the embed");
                    if unplayable(&e, VISIONOS_REFUSAL) {
                        refused = Some(m.clone());
                    }
                }
            }
        }
        match self.answer(video, quality, &ready, true).await {
            // `answer` dropped the page.
            Err(NativeError::NoUrl(_)) => match self.ready(video, Some(ready.page)).await {
                Ok(fresh) => (
                    self.answer(video, quality, &fresh, true).await,
                    Some(fresh.page),
                ),
                Err(e) => (Err(e), None),
            },
            Err(e) if unplayable(&e, EMBED_REFUSAL) && refused.is_some() => {
                let m = refused.unwrap_or_default();
                (Err(NativeError::NotAvailable(m)), Some(ready.page))
            }
            result => (result, Some(ready.page)),
        }
    }

    fn next_page(&self) -> u64 {
        self.pages.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// One resolve from `ready`. `kept`: `ready` is the state resolves share, which a
    /// failed request or a bad answer drops.
    async fn answer(
        &self,
        video: &str,
        quality: Quality,
        ready: &Ready,
        kept: bool,
    ) -> Result<ResolvedStream, NativeError> {
        let player = ready.player.clone();
        let forget = || {
            if kept {
                self.drop_page(Some(ready));
            }
        };
        // Loads the player while the request runs.
        let solver =
            Solver::start(&player.path).map_err(|e| failed(format!("starting the solver: {e}")))?;
        let body = match self.player_request(video, ready).await {
            Ok(b) => b,
            Err(e) => {
                forget();
                return Err(e);
            }
        };
        // A gate counts from the answer, not from the solved challenges.
        let answered = SystemTime::now();
        let pending = parse_player_response(&body, video, quality.itags()).inspect_err(|e| {
            if matches!(e, NativeError::Failed(_) | NativeError::NoUrl(_)) {
                forget();
            }
        })?;
        let n: Vec<String> = pending.n.iter().cloned().collect();
        let sig: Vec<String> = pending.sig.iter().map(|(_, s)| s.clone()).collect();
        let answers = if n.is_empty() && sig.is_empty() {
            Answers::default()
        } else {
            match solver.solve(&n, &sig).await {
                Ok(a) => a,
                Err(e) => {
                    self.health.lock_or_recover().solve_failed(&player.id, &e);
                    forget();
                    return Err(failed(format!("player {}: {e}", player.id)));
                }
            }
        };
        finish(pending, &answers, answered)
    }

    /// One resolve as the visionOS app, with `ready`'s visitor. An answer with ads before
    /// the song is no stream here: the embedded client's may come without.
    async fn visionos(
        &self,
        video: &str,
        quality: Quality,
        ready: &Ready,
    ) -> Result<ResolvedStream, NativeError> {
        let visitor = ready
            .config
            .visitor_data
            .as_deref()
            .ok_or_else(|| failed("no visitor"))?;
        let body = self
            .post_player(
                &visionos_request_body(video, visitor),
                &visionos_headers(visitor),
                VISIONOS_TIMEOUT,
            )
            .await?;
        let answered = SystemTime::now();
        let pending = parse_player_response(&body, video, quality.itags())?;
        if pending.n.is_some() || pending.sig.is_some() {
            return Err(failed("its URL has a challenge"));
        }
        if let Some(gate) = pending.gate {
            return Err(failed(format!(
                "ads before the song, {:.1}s",
                gate.as_secs_f64()
            )));
        }
        let stream = finish(pending, &Answers::default(), answered)?;
        Ok(ResolvedStream {
            via: Via::NativeVisionOs,
            ..stream
        })
    }

    /// The config and player to use: in memory, else on disk, else fetched. `after`: the
    /// page an answer without a URL came from; a newer one kept meanwhile answers, even
    /// one whose own answer failed since, so resolves at once share one fresh page (and its
    /// failures count once) however late each one's first answer came. What to drop stays
    /// for the next resolve.
    async fn ready(
        self: &Arc<Self>,
        video: &str,
        after: Option<u64>,
    ) -> Result<Ready, NativeError> {
        let mut st = self.state.lock().await;
        let newer = after.is_some_and(|p| st.as_ref().is_some_and(|r| r.page > p));
        // `drop_page` took it off the disk already.
        let stale = !newer
            && match self.forget.lock_or_recover().take() {
                Some(Forget::Any) => true,
                Some(Forget::Page(page)) => st.as_ref().is_some_and(|r| r.page <= page),
                None => false,
            };
        if stale {
            *st = None;
        }
        if st.is_none() {
            *st = self.load();
            // Saved while its page's player was still being installed: once more, once.
            if let Some(r) = st.as_ref().filter(|r| r.player.id != r.config.player) {
                self.install_behind(&r.config.player);
            }
        }
        // A player whose challenges couldn't be solved gives way: the state that names it
        // goes, from the disk too, and a fresh page's player answers (an older good one
        // while it's installed), so a new player YouTube moved on to is found.
        if let Some(r) = st
            .as_ref()
            .filter(|r| self.health.lock_or_recover().bad.contains(&r.player.id))
        {
            log::info!("player {} couldn't be solved: a fresh page", r.player.id);
            let _ = std::fs::remove_file(self.dir.join("state.json"));
            *st = None;
        }
        let ready = match &*st {
            Some(ready) => ready.clone(),
            None => {
                let ready = self.fetch_ready(video, None).await?;
                self.save(&ready);
                *st = Some(ready.clone());
                ready
            }
        };
        Ok(ready)
    }

    /// A fresh embed page (`video`'s, or with `""` the one of no video, which names the
    /// same things) and a player to answer with. When the page's player isn't installed,
    /// an older one answers now (`fallback`, else the newest installed: YouTube accepts an
    /// older player's `sts` for a while) and the page's is installed in the background,
    /// which takes seconds. Only with no player at all is the install waited for.
    async fn fetch_ready(
        self: &Arc<Self>,
        video: &str,
        fallback: Option<Player>,
    ) -> Result<Ready, NativeError> {
        let page = self
            .get_text(
                &format!("{}/embed/{video}?html5=1", self.base),
                Some(EMBED_URL),
                REQUEST_TIMEOUT,
            )
            .await?;
        let config =
            parse_embed_page(&page).ok_or_else(|| failed("no player in the embed page"))?;
        let players = self.dir.join("players");
        let good = |id: &str| !self.health.lock_or_recover().bad.contains(id);
        let player = match installed(&players, &config.player).filter(|p| good(&p.id)) {
            Some(player) => player,
            None => {
                let older = fallback
                    .filter(|old| old.path.is_file() && good(&old.id))
                    .or_else(|| newest(&players, good));
                match older {
                    Some(old) => {
                        self.install_behind(&config.player);
                        old
                    }
                    None => self.install_player(&config.player).await?,
                }
            }
        };
        Ok(Ready {
            config,
            player,
            fetched: SystemTime::now(),
            page: self.next_page(),
        })
    }

    /// Installs player `id` in the background, once at a time, and hands it to the kept
    /// page that names it. A failure waits for the next page that names the player.
    fn install_behind(self: &Arc<Self>, id: &str) {
        if self.health.lock_or_recover().bad.contains(id)
            || !self.installing.lock_or_recover().insert(id.to_string())
        {
            return;
        }
        let (inner, id) = (self.clone(), id.to_string());
        tokio::spawn(async move {
            let started = Instant::now();
            match inner.install_player(&id).await {
                Ok(player) => {
                    log::info!(
                        "player {id} installed in the background in {:.1}s",
                        started.elapsed().as_secs_f64()
                    );
                    let mut st = inner.state.lock().await;
                    if let Some(r) = st.as_mut().filter(|r| r.config.player == id) {
                        r.player = player;
                        inner.save(r);
                    }
                }
                Err(e) => log::warn!("installing player {id} in the background failed: {e}"),
            }
            inner.installing.lock_or_recover().remove(&id);
        });
    }

    async fn install_player(&self, id: &str) -> Result<Player, NativeError> {
        let players = self.dir.join("players");
        // Installed or not, a player that couldn't be solved doesn't answer.
        if self.health.lock_or_recover().bad.contains(id) {
            return Err(failed(format!("player {id} couldn't be solved")));
        }
        if let Some(p) = installed(&players, id) {
            return Ok(p);
        }
        let js = self
            .get_text(
                &format!(
                    "{}/s/player/{id}/player_ias.vflset/en_US/base.js",
                    self.base
                ),
                None,
                DOWNLOAD_TIMEOUT,
            )
            .await?;
        let sts = player_sts(&js).ok_or_else(|| failed(format!("player {id} has no sts")))?;
        let pre = self.preprocess(js).await.map_err(|e| match e {
            // Says nothing about the player: it's tried again next time.
            PreprocessError::NotStarted(e) | PreprocessError::Interrupted(e) => e,
            PreprocessError::Unsolved(e) => {
                self.health.lock_or_recover().bad.insert(id.to_string());
                e
            }
        })?;
        let player = install(&players, id, sts, &pre)
            .map_err(|e| failed(format!("saving player {id}: {e}")))?;
        warm(&player).await;
        Ok(player)
    }

    async fn get_text(
        &self,
        url: &str,
        referer: Option<&str>,
        timeout: Duration,
    ) -> Result<String, NativeError> {
        let mut req = self.http.client().get(url).timeout(timeout);
        if let Some(r) = referer {
            req = req.header("Referer", r);
        }
        let resp = req.send().await.map_err(|e| failed(format!("GET: {e}")))?;
        if !resp.status().is_success() {
            return Err(failed(format!("GET answered {}", resp.status())));
        }
        resp.text()
            .await
            .map_err(|e| failed(format!("GET body: {e}")))
    }

    async fn player_request(&self, video: &str, ready: &Ready) -> Result<String, NativeError> {
        self.post_player(
            &player_request_body(video, &ready.config, ready.player.sts),
            &player_headers(&ready.config),
            REQUEST_TIMEOUT,
        )
        .await
    }

    /// The player API's answer to `body`, sent with the client's `headers`.
    async fn post_player(
        &self,
        body: &Value,
        headers: &[(&'static str, String)],
        timeout: Duration,
    ) -> Result<String, NativeError> {
        let mut req = self
            .http
            .client()
            .post(format!(
                "{}/youtubei/v1/player?prettyPrint=false",
                self.base
            ))
            .timeout(timeout)
            .header("Content-Type", "application/json")
            .header("Origin", ORIGIN);
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        let resp = req
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| failed(format!("player API: {e}")))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| failed(format!("player API body: {e}")))?;
        if !status.is_success() {
            return Err(failed(format!("player API answered {status}")));
        }
        Ok(body)
    }
}
