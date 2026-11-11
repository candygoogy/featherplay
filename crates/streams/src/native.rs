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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use featherplay_core::sync::LockExt;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::expiry::{form_value, query_param};
use crate::js::{self, EJS_VERSION, Solver};
use crate::{Preprocessor, Quality, ResolvedStream, Via, expires_at_from_url};

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
}

impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skip(m) => write!(f, "not with the embedded client: {m}"),
            Self::Failed(m) | Self::NoUrl(m) => write!(f, "native resolve failed: {m}"),
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
    http: reqwest::Client,
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
    }
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
        let http = featherplay_proxy::client()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(5))
            // No idle timeout, so no reaper timer runs while paused: an idle connection
            // stays until the server closes it, and a click minutes later may reuse it.
            .pool_idle_timeout(None::<Duration>)
            .pool_max_idle_per_host(2)
            .build()
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
            Ok(_) | Err(NativeError::Skip(_)) => {
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
    /// answer, else the embedded client's. An embedded answer without a URL asks once more
    /// from a fresh page (~0.45 s): some visitors get none every time while new ones do
    /// (2026-11-09). Resolves at once that get none share the first one's fresh page.
    /// `counts`: a visionOS failure counts towards turning it off (`Health::visionos_answered`).
    async fn resolve(
        self: &Arc<Self>,
        video: &str,
        quality: Quality,
        counts: bool,
    ) -> (Result<ResolvedStream, NativeError>, Option<u64>) {
        let ready = match self.ready(video).await {
            Ok(r) => r,
            Err(e) => return (Err(e), None),
        };
        if self.health.lock_or_recover().visionos_on() {
            let answer = self.visionos(video, quality, &ready).await;
            self.health
                .lock_or_recover()
                .visionos_answered(video, &answer, counts);
            match answer {
                Ok(stream) => return (Ok(stream), Some(ready.page)),
                Err(NativeError::Skip(m) | NativeError::Failed(m) | NativeError::NoUrl(m)) => {
                    log::info!("resolve {video}: no visionOS stream ({m}), asking as the embed")
                }
            }
        }
        match self.answer(video, quality, &ready, true).await {
            // `answer` dropped the page.
            Err(NativeError::NoUrl(_)) => match self.ready(video).await {
                Ok(fresh) => (
                    self.answer(video, quality, &fresh, true).await,
                    Some(fresh.page),
                ),
                Err(e) => (Err(e), None),
            },
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

    /// The config and player to use: in memory, else on disk, else fetched.
    async fn ready(self: &Arc<Self>, video: &str) -> Result<Ready, NativeError> {
        let mut st = self.state.lock().await;
        // `drop_page` took it off the disk already.
        let stale = match self.forget.lock_or_recover().take() {
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

    /// Runs EJS on a player's JS, as `self.preprocessor` says. A child that can't be
    /// started (an antivirus may refuse it) leaves the work to this process.
    async fn preprocess(&self, js: String) -> Result<String, PreprocessError> {
        match &self.preprocessor {
            Preprocessor::Child(program) => {
                let mut cmd = crate::sidecars::hidden_command(program);
                cmd.arg(crate::PREPROCESS_ARG)
                    .stdin(std::process::Stdio::piped());
                match run_with_stdin(cmd, js.as_bytes(), PREPROCESS_TIMEOUT).await {
                    Err(PreprocessError::NotStarted(e)) => {
                        log::warn!("{e}; preprocessing the player in this process instead");
                        blocking(move || js::preprocess(&js)).await
                    }
                    result => result,
                }
            }
            Preprocessor::InProcess => blocking(move || js::preprocess(&js)).await,
            #[cfg(test)]
            Preprocessor::Stub(stub) => {
                let stub = stub.clone();
                blocking(move || stub(&js)).await
            }
        }
    }

    async fn get_text(
        &self,
        url: &str,
        referer: Option<&str>,
        timeout: Duration,
    ) -> Result<String, NativeError> {
        let mut req = self.http.get(url).timeout(timeout);
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

    /// The saved page's config, and the whole saved state.
    fn saved(&self) -> Option<(EmbedConfig, Value)> {
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(self.dir.join("state.json")).ok()?)
                .ok()?;
        let config = EmbedConfig {
            client_version: v["client_version"].as_str()?.to_string(),
            visitor_data: v["visitor_data"].as_str().map(str::to_string),
            host_flags: v["host_flags"].as_str().map(str::to_string),
            player: v["player"].as_str()?.to_string(),
        };
        Some((config, v))
    }

    fn load(&self) -> Option<Ready> {
        let (config, v) = self.saved()?;
        let player = installed(&self.dir.join("players"), v["active"].as_str()?)?;
        Some(Ready {
            config,
            player,
            fetched: crate::expiry::unix_secs(v["fetched"].as_u64()?)?,
            page: self.next_page(),
        })
    }

    fn save(&self, r: &Ready) {
        let v = json!({
            "client_version": r.config.client_version,
            "visitor_data": r.config.visitor_data,
            "host_flags": r.config.host_flags,
            "player": r.config.player,
            "active": r.player.id,
            "fetched": r.fetched.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        });
        let _ = std::fs::create_dir_all(&self.dir)
            .and_then(|()| write_atomic(&self.dir.join("state.json"), v.to_string().as_bytes()));
    }
}

/// The client config in an embed page (`https://www.youtube.com/embed/{id}?html5=1`).
pub(crate) fn parse_embed_page(html: &str) -> Option<EmbedConfig> {
    let player = html.split("/s/player/").skip(1).find_map(|rest| {
        let id = rest.split('/').next()?;
        valid_player_id(id).then(|| id.to_string())
    })?;
    let client_version = json_string(html, "INNERTUBE_CONTEXT_CLIENT_VERSION")
        .or_else(|| json_string(html, "INNERTUBE_CLIENT_VERSION"))
        .filter(|v| v.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
        .unwrap_or_else(|| CLIENT_VERSION.to_string());
    Some(EmbedConfig {
        client_version,
        visitor_data: json_string(html, "VISITOR_DATA"),
        host_flags: json_string(html, "encryptedHostFlags"),
        player,
    })
}

fn valid_player_id(id: &str) -> bool {
    (4..=16).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The first `"key":"value"` string in `text`, unescaped.
fn json_string(text: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\":\"");
    let start = text.find(&pattern)? + pattern.len();
    let rest = &text[start..];
    let mut escaped = false;
    let end = rest.char_indices().find_map(|(i, c)| {
        let close = c == '"' && !escaped;
        escaped = c == '\\' && !escaped;
        close.then_some(i)
    })?;
    let s: String = serde_json::from_str(&format!("\"{}\"", &rest[..end])).ok()?;
    (!s.is_empty()).then_some(s)
}

/// The player's `signatureTimestamp`, as yt-dlp finds it.
pub(crate) fn player_sts(js: &str) -> Option<u64> {
    ["signatureTimestamp", "sts"].iter().find_map(|key| {
        js.match_indices(key).find_map(|(i, _)| {
            let rest = js[i + key.len()..]
                .trim_start()
                .strip_prefix(':')?
                .trim_start();
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            (digits.len() == 5).then(|| digits.parse().ok()).flatten()
        })
    })
}

/// The player API request's body, as yt-dlp's `web_embedded` client sends it.
pub(crate) fn player_request_body(video: &str, c: &EmbedConfig, sts: u64) -> Value {
    let mut client = json!({
        "clientName": CLIENT_NAME,
        "clientVersion": c.client_version,
        "userAgent": USER_AGENT,
        "hl": "en",
        "timeZone": "UTC",
        "utcOffsetMinutes": 0,
    });
    if let Some(v) = &c.visitor_data {
        client["visitorData"] = json!(v);
    }
    let mut playback = json!({
        "html5Preference": "HTML5_PREF_WANTS",
        "signatureTimestamp": sts,
    });
    if let Some(f) = &c.host_flags {
        playback["encryptedHostFlags"] = json!(f);
    }
    json!({
        "context": {
            "client": client,
            "thirdParty": { "embedUrl": EMBED_URL },
        },
        "videoId": video,
        "playbackContext": { "contentPlaybackContext": playback },
        "contentCheckOk": true,
        "racyCheckOk": true,
    })
}

/// The client headers of the player API request (the body and origin aside).
pub(crate) fn player_headers(c: &EmbedConfig) -> Vec<(&'static str, String)> {
    let mut h = vec![
        ("X-YouTube-Client-Name", CLIENT_ID.to_string()),
        ("X-YouTube-Client-Version", c.client_version.clone()),
    ];
    if let Some(v) = &c.visitor_data {
        h.push(("X-Goog-Visitor-Id", v.clone()));
    }
    h
}

/// The player API request's body as yt-dlp's `visionos` client sends it: no player, so no
/// `signatureTimestamp`.
pub(crate) fn visionos_request_body(video: &str, visitor: &str) -> Value {
    json!({
        "context": {
            "client": {
                "clientName": VISIONOS,
                "clientVersion": VISIONOS_VERSION,
                "deviceMake": "Apple",
                "deviceModel": "RealityDevice17,1",
                "osName": "visionOS",
                "osVersion": "26.5.23O471",
                "userAgent": VISIONOS_USER_AGENT,
                "hl": "en",
                "timeZone": "UTC",
                "utcOffsetMinutes": 0,
                "visitorData": visitor,
            },
        },
        "videoId": video,
        "playbackContext": {
            "contentPlaybackContext": { "html5Preference": "HTML5_PREF_WANTS" },
        },
        "contentCheckOk": true,
        "racyCheckOk": true,
    })
}

/// The visionOS client's headers of the player API request (the body and origin aside).
pub(crate) fn visionos_headers(visitor: &str) -> Vec<(&'static str, String)> {
    vec![
        ("User-Agent", VISIONOS_USER_AGENT.to_string()),
        ("X-YouTube-Client-Name", VISIONOS_ID.to_string()),
        ("X-YouTube-Client-Version", VISIONOS_VERSION.to_string()),
        ("X-Goog-Visitor-Id", visitor.to_string()),
    ]
}

/// Reads a player API answer: the URL and challenges of the first format in `wanted` that
/// it has (and isn't DRM-protected), and the metadata.
pub(crate) fn parse_player_response(
    body: &str,
    video: &str,
    wanted: &[u32],
) -> Result<Pending, NativeError> {
    let v: Value =
        serde_json::from_str(body).map_err(|e| failed(format!("player API JSON: {e}")))?;
    let status = v["playabilityStatus"]["status"].as_str().unwrap_or("");
    if status != "OK" {
        let reason = v["playabilityStatus"]["reason"]
            .as_str()
            .unwrap_or("no reason");
        let m = format!("{status}: {reason}");
        // `UNPLAYABLE` is a video that can't be embedded, the others a sign-in or age check.
        // `ERROR` ("This video is unavailable") is also what a request with a bad config
        // gets, so yt-dlp's own embedded run decides then.
        return Err(match status {
            "UNPLAYABLE"
            | "LOGIN_REQUIRED"
            | "AGE_CHECK_REQUIRED"
            | "AGE_VERIFICATION_REQUIRED"
            | "CONTENT_CHECK_REQUIRED" => NativeError::Skip(m),
            _ => failed(m),
        });
    }
    if v["videoDetails"]["videoId"].as_str() != Some(video) {
        return Err(failed("the answer is for another video"));
    }
    let gate = preroll_gate(&v)?;
    let formats = v["streamingData"]["adaptiveFormats"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let there: Vec<(u32, &Value)> = wanted
        .iter()
        .filter_map(|&itag| {
            let f = formats
                .iter()
                .find(|f| f["itag"].as_u64() == Some(u64::from(itag)))?;
            Some((itag, f))
        })
        .collect();
    if there.is_empty() {
        let names: Vec<String> = wanted.iter().map(u32::to_string).collect();
        return Err(NativeError::Skip(format!(
            "no format {}",
            names.join(" or ")
        )));
    }
    let (itag, f) = there
        .into_iter()
        .find(|(_, f)| f.get("drmFamilies").is_none())
        .ok_or_else(|| NativeError::Skip("the stream is DRM-protected".into()))?;
    let (url, sig) = match f["url"].as_str() {
        Some(u) => (u.to_string(), None),
        None => {
            let sc = f["signatureCipher"]
                .as_str()
                .ok_or_else(|| NativeError::NoUrl(format!("format {itag} has no URL")))?;
            let url = form_value(sc, "url").ok_or_else(|| failed("a cipher without a URL"))?;
            let s = form_value(sc, "s").ok_or_else(|| failed("a cipher without s"))?;
            let sp = form_value(sc, "sp").unwrap_or_else(|| "signature".into());
            (url, Some((sp, s)))
        }
    };
    if !url.starts_with("https://") {
        return Err(failed(format!("format {itag}'s URL isn't https")));
    }
    let mime = query_param(&url, "mime")
        .filter(|m| m.contains('/'))
        .or_else(|| {
            f["mimeType"]
                .as_str()
                .map(|m| m.split(';').next().unwrap_or(m).trim().to_string())
        })
        .unwrap_or_else(|| "audio/mp4".into());
    let num = |x: &Value| x.as_u64().or_else(|| x.as_str()?.parse().ok());
    let kbps = num(&f["averageBitrate"])
        .or_else(|| num(&f["bitrate"]))
        .unwrap_or(0);
    let d = &v["videoDetails"];
    Ok(Pending {
        itag,
        n: query_param(&url, "n").filter(|n| !n.is_empty()),
        url,
        sig,
        mime,
        bitrate_kbps: ((kbps + 500) / 1000) as u32,
        content_length: num(&f["contentLength"]),
        duration: num(&d["lengthSeconds"])
            .filter(|s| *s > 0)
            .map(Duration::from_secs)
            .or_else(|| num(&f["approxDurationMs"]).map(Duration::from_millis)),
        video_title: text(&d["title"]),
        first_tag: text(&d["keywords"][0]),
        // Per format, as the web player reads it; else the one for the whole video.
        loudness_db: f["loudnessDb"]
            .as_f64()
            .or_else(|| v["playerConfig"]["audioConfig"]["loudnessDb"].as_f64())
            .map(|db| db as f32),
        gate,
    })
}

/// How long the URL of an answer with ads before the song is refused: each ad until it
/// could be skipped, else its length, added up (as yt-dlp reads them). The embedded client
/// sends them as `adSlots` since 2026-11-05; `adPlacements` is the older form. An ad before
/// the song whose length isn't given is an error: yt-dlp gets the resolve.
fn preroll_gate(v: &Value) -> Result<Option<Duration>, NativeError> {
    let mut ads: Vec<&Value> = Vec::new();
    let mut unreadable = false;
    for p in v["adPlacements"].as_array().into_iter().flatten() {
        let p = &p["adPlacementRenderer"];
        if p["config"]["adPlacementConfig"]["kind"] == "AD_PLACEMENT_KIND_START" {
            let ad = &p["renderer"]["instreamVideoAdRenderer"];
            if ad.is_object() {
                ads.push(ad);
            } else {
                unreadable = true;
            }
        }
    }
    for s in v["adSlots"].as_array().into_iter().flatten() {
        let s = &s["adSlotRenderer"];
        if s["adSlotMetadata"]["triggerEvent"] != "SLOT_TRIGGER_EVENT_BEFORE_CONTENT" {
            continue;
        }
        let content = &s["fulfillmentContent"]["fulfilledLayout"]["playerBytesAdLayoutRenderer"]["renderingContent"];
        let pod = content["playerBytesSequentialLayoutRenderer"]["sequentialLayouts"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| &l["playerBytesAdLayoutRenderer"]["renderingContent"]);
        let before = ads.len();
        ads.extend(
            std::iter::once(content)
                .chain(pod)
                .map(|c| &c["instreamVideoAdRenderer"])
                .filter(|ad| ad.is_object()),
        );
        unreadable |= ads.len() == before;
    }
    let mut gate = Duration::ZERO;
    for ad in ads {
        let skip = ad["skipOffsetMilliseconds"].as_f64().map(|ms| ms / 1000.0);
        let length = ad["playerVars"]
            .as_str()
            .and_then(|q| form_value(q, "length_seconds"))
            .and_then(|s| s.parse::<f64>().ok());
        match skip
            .or(length)
            .and_then(|s| Duration::try_from_secs_f64(s).ok())
        {
            Some(wait) => gate = gate.saturating_add(wait),
            None => unreadable = true,
        }
    }
    if unreadable {
        return Err(failed("an ad before the song, of unknown length"));
    }
    Ok((!gate.is_zero()).then_some(gate))
}

fn text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The solver's answers, by challenge.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Answers {
    pub n: HashMap<String, String>,
    pub sig: HashMap<String, String>,
}

/// The stream, with the answered challenges put in its URL.
pub(crate) fn finish(
    p: Pending,
    a: &Answers,
    now: SystemTime,
) -> Result<ResolvedStream, NativeError> {
    let mut url = p.url;
    if let Some(n) = &p.n {
        let answer = a.n.get(n).ok_or_else(|| failed("no answer for n"))?;
        url = with_param(&url, "n", answer);
    }
    if let Some((sp, s)) = &p.sig {
        let answer = a.sig.get(s).ok_or_else(|| failed("no answer for sig"))?;
        url = with_param(&url, sp, answer);
    }
    // Counted from the next whole second, as yt-dlp does (its URLs played so, 2026-11-07).
    // A gate past what the clock can hold (a damaged answer) is left out.
    let available_at = p.gate.and_then(|gate| {
        let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
        let whole = since.as_secs() + u64::from(since.subsec_nanos() > 0);
        crate::expiry::unix_secs(whole)?.checked_add(gate)
    });
    Ok(ResolvedStream {
        expires_at: expires_at_from_url(&url, now),
        url,
        mime: p.mime,
        itag: p.itag,
        bitrate_kbps: p.bitrate_kbps,
        content_length: p.content_length,
        duration: p.duration,
        available_at,
        video_title: p.video_title,
        first_tag: p.first_tag,
        loudness_db: p.loudness_db,
        via: Via::NativeEmbedded,
    })
}

/// `url` with `key` set to `value` (replaced in place, or added at the end).
pub(crate) fn with_param(url: &str, key: &str, value: &str) -> String {
    let encoded = percent_encode(value);
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    let mut found = false;
    let mut pairs: Vec<String> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            if p.split('=').next() == Some(key) && !found {
                found = true;
                format!("{key}={encoded}")
            } else {
                p.to_string()
            }
        })
        .collect();
    if !found {
        pairs.push(format!("{key}={encoded}"));
    }
    format!("{base}?{}", pairs.join("&"))
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The installed player `id`, if it was preprocessed by these EJS scripts.
pub(crate) fn installed(players: &Path, id: &str) -> Option<Player> {
    if !valid_player_id(id) {
        return None;
    }
    let path = players.join(format!("{id}.js"));
    let mut head = [0u8; 128];
    let n = std::io::Read::read(&mut std::fs::File::open(&path).ok()?, &mut head).ok()?;
    let line = std::str::from_utf8(&head[..n]).ok()?.lines().next()?;
    // `// featherplay player {id} sts {sts} ejs {version}`
    let mut w = line.strip_prefix("// featherplay player ")?.split(' ');
    let (pid, _, sts, _, ejs) = (w.next()?, w.next()?, w.next()?, w.next()?, w.next()?);
    let sts = sts
        .parse()
        .ok()
        .filter(|_| pid == id && ejs == EJS_VERSION)?;
    Some(Player {
        id: id.to_string(),
        sts,
        path,
    })
}

/// The files in `players` with this extension, oldest first.
fn files(players: &Path, extension: &str) -> Vec<(SystemTime, PathBuf)> {
    let mut files: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(players)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let p = e.ok()?.path();
            let ok = p.extension().is_some_and(|x| x == extension);
            ok.then(|| Some((p.metadata().ok()?.modified().ok()?, p)))?
        })
        .collect();
    files.sort();
    files
}

/// The newest installed player that `good` takes: what answers while a page's own player
/// is being installed.
fn newest(players: &Path, good: impl Fn(&str) -> bool) -> Option<Player> {
    files(players, "js")
        .into_iter()
        .rev()
        .filter_map(|(_, p)| installed(players, p.file_stem()?.to_str()?))
        .find(|p| good(&p.id))
}

/// Saves a preprocessed player and drops all but the newest `KEEP_PLAYERS`, each with its
/// bytecode (`js`).
pub(crate) fn install(players: &Path, id: &str, sts: u64, pre: &str) -> std::io::Result<Player> {
    if !valid_player_id(id) {
        return Err(std::io::Error::other("bad player id"));
    }
    std::fs::create_dir_all(players)?;
    let path = players.join(format!("{id}.js"));
    let body = format!("// featherplay player {id} sts {sts} ejs {EJS_VERSION}\n{pre}");
    write_atomic(&path, body.as_bytes())?;
    // An earlier file's bytecode isn't this one's.
    let _ = std::fs::remove_file(js::bytecode_path(&path));
    let others = files(players, "js").into_iter().filter(|(_, p)| *p != path);
    let others: Vec<_> = others.collect();
    let drop_n = (others.len() + 1).saturating_sub(KEEP_PLAYERS);
    for (_, p) in others.into_iter().take(drop_n) {
        let _ = std::fs::remove_file(js::bytecode_path(&p));
        let _ = std::fs::remove_file(p);
    }
    // A save cut short (a crash, the power) left its temporary file.
    for (written, part) in files(players, "part") {
        if written.elapsed().is_ok_and(|age| age > STRAY_AFTER) {
            let _ = std::fs::remove_file(part);
        }
    }
    Ok(Player {
        id: id.to_string(),
        sts,
        path,
    })
}

/// Numbers the temporary files of `write_atomic`, so two writes of one file at once (two
/// installs of a player, or two saves of the state) each rename their own.
static WRITES: AtomicU64 = AtomicU64::new(0);

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let part = path.with_extension(format!(
        "{}-{}.part",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

/// Loads `player` once, which saves its bytecode: the first song it answers then loads it
/// in ~15 ms instead of compiling 4 MB of JS (~190 ms).
async fn warm(player: &Player) {
    let path = player.path.clone();
    let warmed = tokio::task::spawn_blocking(move || js::warm(&path)).await;
    if let Err(e) = warmed.map_err(|e| e.to_string()).and_then(|w| w) {
        log::warn!("player {} didn't answer once loaded: {e}", player.id);
    }
}

/// Why a player couldn't be preprocessed.
enum PreprocessError {
    /// The work didn't start, so nothing is known about the player.
    NotStarted(NativeError),
    /// EJS ran on the player and failed, or ran past its own time limit, and said so.
    Unsolved(NativeError),
    /// The child preprocessing it ended before it said how it went (killed, crashed, hung
    /// past its own limit): nothing is known about the player, and it's tried again later.
    Interrupted(NativeError),
}

/// `work` on the blocking pool: it waits for a QuickJS thread, which an async thread
/// never does.
async fn blocking(
    work: impl FnOnce() -> Result<String, String> + Send + 'static,
) -> Result<String, PreprocessError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| PreprocessError::NotStarted(failed(e)))?
        .map_err(|e| PreprocessError::Unsolved(failed(e)))
}

/// Runs the process that preprocesses a player (`crate::preprocess_stdio`): the player's
/// JS goes to its stdin, the preprocessed player comes from its stdout. Killed at
/// `timeout`, or when the future is dropped.
async fn run_with_stdin(
    mut cmd: tokio::process::Command,
    stdin: &[u8],
    timeout: Duration,
) -> Result<String, PreprocessError> {
    let not_started =
        |e: String| PreprocessError::NotStarted(failed(format!("starting the preprocessor: {e}")));
    // Starting a process blocks for as long as an antivirus checks the exe, and a 30 MB one
    // can take a few hundred ms: not on an async thread, where resolves run.
    let mut child = tokio::task::spawn_blocking(move || cmd.spawn())
        .await
        .map_err(|e| not_started(e.to_string()))?
        .map_err(|e| not_started(e.to_string()))?;
    // Held for the run: if the app exits or crashes, the job's handle closes and the child
    // (~205 MB) ends with it instead of running out its own time limit.
    #[cfg(windows)]
    let process = child.raw_handle();
    #[cfg(not(windows))]
    let process = child.id();
    let _job = match process {
        Some(process) => featherplay_platform::process::kill_on_close(process)
            .inspect_err(|e| log::warn!("the preprocessor isn't tied to the app: {e}"))
            .ok(),
        None => None,
    };
    let unsolved = |m: String| PreprocessError::Unsolved(failed(m));
    let mut pipe = child
        .stdin
        .take()
        .expect("the caller's command pipes stdin");
    let run = async {
        // The child reads all of it before it writes, so nothing waits on a full pipe.
        pipe.write_all(stdin).await?;
        drop(pipe);
        child.wait_with_output().await
    };
    // A child that hangs past its own limit, or whose pipes broke, says nothing of the
    // player either.
    let interrupted = |m: String| PreprocessError::Interrupted(failed(m));
    let out = tokio::time::timeout(timeout, run)
        .await
        .map_err(|_| interrupted("the preprocessor timed out".into()))?
        .map_err(|e| interrupted(format!("the preprocessor: {e}")))?;
    if !out.status.success() {
        return Err(child_failed(
            out.status.code(),
            &String::from_utf8_lossy(&out.stderr),
        ));
    }
    String::from_utf8(out.stdout)
        .ok()
        .filter(|player| !player.is_empty())
        .ok_or_else(|| unsolved("the preprocessor gave no player".into()))
}

/// Why a preprocessing child that exited with `code` and `stderr` failed: EJS failed when
/// it exits with 1 and says why (`preprocess_stdio`); one that ended without a reason was
/// killed or crashed, which says nothing of the player.
fn child_failed(code: Option<i32>, stderr: &str) -> PreprocessError {
    let e = failed(format!(
        "the preprocessor exited with {}: {}",
        code.unwrap_or(-1),
        crate::sidecars::tail(stderr, 300)
    ));
    if code == Some(1) && !stderr.trim().is_empty() {
        PreprocessError::Unsolved(e)
    } else {
        PreprocessError::Interrupted(e)
    }
}

/// A one-thread HTTP/1.1 server for tests: answers each request with
/// `route(method, path, body)` and logs `"METHOD path"` of each.
#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    pub(crate) type Log = Arc<Mutex<Vec<String>>>;

    /// One thread per connection, so a client that keeps its connection open doesn't hold
    /// up another (requests at once).
    pub(crate) fn serve(
        route: impl Fn(&str, &str, &str) -> (u16, String) + Send + Sync + 'static,
    ) -> (String, Log) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log: Log = Arc::default();
        let log2 = log.clone();
        let route = Arc::new(route);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let (log, route) = (log2.clone(), route.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            break;
                        }
                        let mut parts = line.split_whitespace();
                        let method = parts.next().unwrap_or("").to_string();
                        let path = parts.next().unwrap_or("").to_string();
                        let mut len = 0;
                        loop {
                            let mut h = String::new();
                            if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                                break;
                            }
                            if let Some((k, v)) = h.split_once(':')
                                && k.eq_ignore_ascii_case("content-length")
                            {
                                len = v.trim().parse().unwrap_or(0);
                            }
                        }
                        let mut body = vec![0; len];
                        if reader.read_exact(&mut body).is_err() {
                            break;
                        }
                        log.lock().unwrap().push(format!("{method} {path}"));
                        let (status, answer) =
                            route(&method, &path, &String::from_utf8_lossy(&body));
                        let head = format!(
                            "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n",
                            answer.len()
                        );
                        if stream.write_all(head.as_bytes()).is_err()
                            || stream.write_all(answer.as_bytes()).is_err()
                        {
                            break;
                        }
                    }
                });
            }
        });
        (base, log)
    }

    /// Like `serve`, with binary answers, and requests read as GETs.
    pub(crate) fn serve_bytes(
        route: impl Fn(&str) -> (u16, Vec<u8>) + Send + Sync + 'static,
    ) -> (String, Log) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log: Log = Arc::default();
        let log2 = log.clone();
        let route = Arc::new(route);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let (log, route) = (log2.clone(), route.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            break;
                        }
                        let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                        loop {
                            let mut h = String::new();
                            if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                                break;
                            }
                        }
                        log.lock().unwrap().push(format!("GET {path}"));
                        let (status, answer) = route(&path);
                        let head = format!(
                            "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n\r\n",
                            answer.len()
                        );
                        if stream.write_all(head.as_bytes()).is_err()
                            || stream.write_all(&answer).is_err()
                        {
                            break;
                        }
                    }
                });
            }
        });
        (base, log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMBED: &str = include_str!("../tests/fixtures/native/embed.html");
    /// A recorded `player` answer for `h46YpWrg55Q` without cookies (2026-10-28), cut to
    /// formats 140 and 251, with the CDN host, request ids and signatures scrubbed.
    const PLAYER: &str = include_str!("../tests/fixtures/native/player-embedded.json");
    /// The visionOS client's answer for the same song (2026-11-10), cut and scrubbed the
    /// same way: its URLs have no `n` and no cipher.
    const VISIONOS_PLAYER: &str = include_str!("../tests/fixtures/native/player-visionos.json");
    const VIDEO: &str = "h46YpWrg55Q";
    /// Stands in for a preprocessed player: `n` reversed, `sig` without its first two
    /// characters and reversed.
    const FAKE_PLAYER: &str = r#"_result.n = (n) => n.split("").reverse().join("");
_result.sig = (s) => s.slice(2).split("").reverse().join("");"#;

    fn home(name: &str) -> PathBuf {
        let base = std::env::var("FEATHERPLAY_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"));
        let dir = base
            .join("test-streams")
            .join(format!("native-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn config() -> EmbedConfig {
        parse_embed_page(EMBED).unwrap()
    }

    #[test]
    fn the_embed_page_gives_the_client_config() {
        assert_eq!(
            config(),
            EmbedConfig {
                client_version: "2.20261028.01.00".into(),
                visitor_data: Some("CgtTY3J1YmJlZA%3D%3D".into()),
                host_flags: Some("AEZHscrubbed==".into()),
                player: "096efeec".into(),
            }
        );
        // A page with only the player: yt-dlp's client version, nothing else.
        let bare = parse_embed_page(r#"<script src="/s/player/abcd1234/base.js">"#).unwrap();
        assert_eq!(bare.client_version, CLIENT_VERSION);
        assert_eq!((bare.visitor_data, bare.host_flags), (None, None));
        assert_eq!(parse_embed_page("<html>no player</html>"), None);
        assert_eq!(
            parse_embed_page("/s/player/../../x/base.js"),
            None,
            "not an id"
        );
    }

    #[test]
    fn the_sts_comes_from_the_player() {
        assert_eq!(
            player_sts("var a={signatureTimestamp:20752,b:1}"),
            Some(20752)
        );
        assert_eq!(player_sts("x.sts : 20391;"), Some(20391));
        assert_eq!(player_sts("sts:1; signatureTimestamp:123"), None);
    }

    #[test]
    fn the_request_is_the_embedded_clients() {
        let body = player_request_body(VIDEO, &config(), 20752);
        let client = &body["context"]["client"];
        assert_eq!(client["clientName"], "WEB_EMBEDDED_PLAYER");
        assert_eq!(client["clientVersion"], "2.20261028.01.00");
        assert_eq!(client["visitorData"], "CgtTY3J1YmJlZA%3D%3D");
        assert_eq!(body["context"]["thirdParty"]["embedUrl"], EMBED_URL);
        assert_eq!(body["videoId"], VIDEO);
        let pb = &body["playbackContext"]["contentPlaybackContext"];
        assert_eq!(pb["signatureTimestamp"], 20752);
        assert_eq!(pb["encryptedHostFlags"], "AEZHscrubbed==");
        assert_eq!(body["contentCheckOk"], true);
        assert_eq!(body["racyCheckOk"], true);
        let h = player_headers(&config());
        assert!(h.contains(&("X-YouTube-Client-Name", "56".into())));
        assert!(h.contains(&("X-YouTube-Client-Version", "2.20261028.01.00".into())));
        assert!(h.contains(&("X-Goog-Visitor-Id", "CgtTY3J1YmJlZA%3D%3D".into())));
        // No visitor data or host flags: neither the fields nor the header.
        let bare = EmbedConfig {
            visitor_data: None,
            host_flags: None,
            ..config()
        };
        let body = player_request_body(VIDEO, &bare, 1);
        assert!(body["context"]["client"].get("visitorData").is_none());
        let pb = &body["playbackContext"]["contentPlaybackContext"];
        assert!(pb.get("encryptedHostFlags").is_none());
        assert_eq!(player_headers(&bare).len(), 2);
    }

    #[test]
    fn the_visionos_request_is_yt_dlps() {
        let body = visionos_request_body(VIDEO, "visitor");
        let client = &body["context"]["client"];
        assert_eq!(client["clientName"], "VISIONOS");
        assert_eq!(client["clientVersion"], "1.02");
        assert_eq!(client["deviceModel"], "RealityDevice17,1");
        assert_eq!(client["visitorData"], "visitor");
        assert!(body["context"].get("thirdParty").is_none());
        let pb = &body["playbackContext"]["contentPlaybackContext"];
        assert!(pb.get("signatureTimestamp").is_none(), "no player");
        assert_eq!(body["videoId"], VIDEO);
        let h = visionos_headers("visitor");
        assert!(h.contains(&("X-YouTube-Client-Name", "101".into())));
        assert!(h.contains(&("X-Goog-Visitor-Id", "visitor".into())));
        assert!(h.contains(&("User-Agent", VISIONOS_USER_AGENT.into())));
    }

    #[test]
    fn a_recorded_visionos_answer_has_a_url_without_challenges() {
        let p = parse_player_response(VISIONOS_PLAYER, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!((p.itag, p.mime.as_str()), (251, "audio/webm"));
        assert_eq!(query_param(&p.url, "c").as_deref(), Some("VISIONOS"));
        assert_eq!((p.n, p.sig, p.gate), (None, None, None));
        assert_eq!(p.loudness_db, Some(7.17));
        assert_eq!(p.video_title.as_deref(), Some("affection"));
    }

    #[test]
    fn a_recorded_answer_gives_format_140_and_its_challenges() {
        let p = parse_player_response(PLAYER, VIDEO, Quality::Standard.itags()).unwrap();
        assert!(
            p.url
                .starts_with("https://rr1---sn-scrubbed.googlevideo.com/videoplayback?")
        );
        assert_eq!(query_param(&p.url, "itag").as_deref(), Some("140"));
        assert_eq!(p.n.as_deref(), Some("tbgZJm7jUNULHGjSAEd"));
        let (sp, s) = p.sig.clone().unwrap();
        assert_eq!((sp.as_str(), s.len()), ("sig", 107));
        assert_eq!(p.mime, "audio/mp4");
        assert_eq!(p.bitrate_kbps, 130);
        assert_eq!(p.content_length, Some(3_809_688));
        assert_eq!(p.duration, Some(Duration::from_secs(235)));
        assert_eq!(p.video_title.as_deref(), Some("affection"));
        assert!(p.first_tag.is_some());

        let answers = Answers {
            n: HashMap::from([("tbgZJm7jUNULHGjSAEd".into(), "solved-n".into())]),
            sig: HashMap::from([(s.clone(), "AB=C/D".into())]),
        };
        let now = SystemTime::now();
        let stream = finish(p.clone(), &answers, now).unwrap();
        assert_eq!(query_param(&stream.url, "n").as_deref(), Some("solved-n"));
        assert_eq!(query_param(&stream.url, "sig").as_deref(), Some("AB=C/D"));
        assert!(stream.url.contains("&sig=AB%3DC%2FD"));
        assert_eq!(stream.url.matches("&n=").count(), 1);
        assert_eq!(stream.itag, 140);
        assert_eq!(stream.available_at, None);
        assert_eq!(
            stream.expires_at,
            UNIX_EPOCH + Duration::from_secs(1_793_234_740)
        );
        // An unanswered challenge is an error, not a URL that fails later.
        assert!(finish(p, &Answers::default(), now).is_err());
    }

    #[test]
    fn the_answer_gives_the_songs_loudness() {
        let p = parse_player_response(PLAYER, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.loudness_db, Some(7.17));
        let (_, s) = p.sig.clone().unwrap();
        let answers = Answers {
            n: HashMap::from([("tbgZJm7jUNULHGjSAEd".into(), "solved-n".into())]),
            sig: HashMap::from([(s, "solved-sig".into())]),
        };
        let stream = finish(p, &answers, SystemTime::now()).unwrap();
        assert_eq!(stream.loudness_db, Some(7.17));

        // Without one per format, the player's own (`playerConfig.audioConfig`); else none.
        let bare = PLAYER.replace("\"loudnessDb\": 7.17,", "");
        let p = parse_player_response(&bare, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.loudness_db, None);
        let config = bare.replacen(
            '{',
            r#"{ "playerConfig": { "audioConfig": { "loudnessDb": -2.5 } },"#,
            1,
        );
        let p = parse_player_response(&config, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.loudness_db, Some(-2.5));
    }

    #[test]
    fn high_quality_picks_format_251_as_webm() {
        let p = parse_player_response(PLAYER, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(query_param(&p.url, "itag").as_deref(), Some("251"));
        assert_eq!(p.mime, "audio/webm");
        assert_eq!(p.content_length, Some(3_856_919));
        let (_, s) = p.sig.clone().unwrap();
        let answers = Answers {
            n: HashMap::from([("tbgZJm7jUNULHGjSAEd".into(), "solved-n".into())]),
            sig: HashMap::from([(s, "solved-sig".into())]),
        };
        let stream = finish(p, &answers, SystemTime::now()).unwrap();
        assert_eq!(stream.itag, 251);
        assert_eq!(stream.mime, "audio/webm");
    }

    #[test]
    fn high_quality_falls_back_to_format_140_without_251() {
        let body = PLAYER.replace("\"itag\": 251", "\"itag\": 250");
        let p = parse_player_response(&body, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(query_param(&p.url, "itag").as_deref(), Some("140"));
        assert_eq!(p.mime, "audio/mp4");
        assert_eq!(p.itag, 140);
    }

    #[test]
    fn a_missing_format_names_the_ones_wanted() {
        let skip = |body: &str, q: Quality| match parse_player_response(body, VIDEO, q.itags()) {
            Err(NativeError::Skip(m)) => m,
            other => panic!("{other:?}"),
        };
        let neither = PLAYER
            .replace("\"itag\": 251", "\"itag\": 250")
            .replace("\"itag\": 140", "\"itag\": 139");
        assert_eq!(skip(&neither, Quality::High), "no format 251 or 140");
        let no_140 = PLAYER.replace("\"itag\": 140", "\"itag\": 139");
        assert_eq!(skip(&no_140, Quality::Standard), "no format 140");
    }

    #[test]
    fn answers_that_mean_another_client_or_yt_dlp() {
        let skip = |body: &str| {
            matches!(
                parse_player_response(body, VIDEO, Quality::Standard.itags()),
                Err(NativeError::Skip(_))
            )
        };
        let fail = |body: &str| {
            matches!(
                parse_player_response(body, VIDEO, Quality::Standard.itags()),
                Err(NativeError::Failed(_))
            )
        };
        assert!(skip(
            r#"{"playabilityStatus":{"status":"UNPLAYABLE","reason":"Playback on other websites has been disabled by the video owner"}}"#
        ));
        assert!(skip(
            r#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm your age"}}"#
        ));
        // What a request with a bad config gets too.
        assert!(fail(
            r#"{"playabilityStatus":{"status":"ERROR","reason":"This video is unavailable"}}"#
        ));
        assert!(fail("not json"));
        assert!(
            fail(&PLAYER.replace(VIDEO, "aaaaaaaaaaa")),
            "another video's answer"
        );
        assert!(
            skip(&PLAYER.replace("\"itag\": 140", "\"itag\": 139")),
            "no format 140"
        );
        let preroll = PLAYER.replacen(
            '{',
            r#"{"adPlacements":[{"adPlacementRenderer":{"config":{"adPlacementConfig":{"kind":"AD_PLACEMENT_KIND_START"}}}}],"#,
            1,
        );
        assert!(
            fail(&preroll),
            "a pre-roll of unknown length is left to yt-dlp"
        );
    }

    /// `PLAYER` with `ads` (a JSON member) in front.
    fn with_ads(ads: &str) -> String {
        PLAYER.replacen('{', &format!("{{{ads},"), 1)
    }

    /// A pre-roll ad slot as the embedded client got them from 2026-11-05: the URL answers
    /// 403 until the ad could be skipped (seen live: 403 at once, 206 after the 5 s).
    const AD_SLOT: &str = r#""adSlots":[{"adSlotRenderer":{"adSlotMetadata":{"triggerEvent":"SLOT_TRIGGER_EVENT_BEFORE_CONTENT"},"fulfillmentContent":{"fulfilledLayout":{"playerBytesAdLayoutRenderer":{"renderingContent":{"instreamVideoAdRenderer":{"skipOffsetMilliseconds":5000,"playerVars":"length_seconds=15&x=1"}}}}}}},{"adSlotRenderer":{"adSlotMetadata":{"triggerEvent":"SLOT_TRIGGER_EVENT_LAYOUT_ID_ENTERED"}}}]"#;

    #[test]
    fn a_pre_roll_ad_slot_gates_the_url_until_it_could_be_skipped() {
        let p = parse_player_response(&with_ads(AD_SLOT), VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.gate, Some(Duration::from_secs(5)));
        let (_, s) = p.sig.clone().unwrap();
        let answers = Answers {
            n: HashMap::from([("tbgZJm7jUNULHGjSAEd".into(), "solved-n".into())]),
            sig: HashMap::from([(s, "solved-sig".into())]),
        };
        // As yt-dlp counts it: from the next whole second.
        let now = UNIX_EPOCH + Duration::from_millis(1_794_043_690_250);
        let stream = finish(p, &answers, now).unwrap();
        assert_eq!(
            stream.available_at,
            Some(UNIX_EPOCH + Duration::from_secs(1_794_043_696))
        );
    }

    /// A gate past what the clock can hold (a damaged answer) is left out, without a panic.
    #[test]
    fn a_gate_too_far_ahead_is_left_out() {
        let mut p =
            parse_player_response(&with_ads(AD_SLOT), VIDEO, Quality::High.itags()).unwrap();
        p.gate = Some(Duration::MAX);
        let (_, s) = p.sig.clone().unwrap();
        let answers = Answers {
            n: HashMap::from([("tbgZJm7jUNULHGjSAEd".into(), "solved-n".into())]),
            sig: HashMap::from([(s, "solved-sig".into())]),
        };
        let stream = finish(p, &answers, SystemTime::now()).unwrap();
        assert_eq!(stream.available_at, None);
    }

    #[test]
    fn an_ad_that_cant_be_skipped_gates_the_url_for_its_length() {
        let ads = AD_SLOT.replace(r#""skipOffsetMilliseconds":5000,"#, "");
        let p = parse_player_response(&with_ads(&ads), VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.gate, Some(Duration::from_secs(15)));
    }

    /// A pod of two ads before the song, and an older-style placement: the waits add up.
    #[test]
    fn the_waits_of_every_ad_before_the_song_add_up() {
        let pod = r#""adSlots":[{"adSlotRenderer":{"adSlotMetadata":{"triggerEvent":"SLOT_TRIGGER_EVENT_BEFORE_CONTENT"},"fulfillmentContent":{"fulfilledLayout":{"playerBytesAdLayoutRenderer":{"renderingContent":{"playerBytesSequentialLayoutRenderer":{"sequentialLayouts":[{"playerBytesAdLayoutRenderer":{"renderingContent":{"instreamVideoAdRenderer":{"skipOffsetMilliseconds":5000}}}},{"playerBytesAdLayoutRenderer":{"renderingContent":{"instreamVideoAdRenderer":{"playerVars":"length_seconds=6"}}}}]}}}}}}}],"adPlacements":[{"adPlacementRenderer":{"config":{"adPlacementConfig":{"kind":"AD_PLACEMENT_KIND_START"}},"renderer":{"instreamVideoAdRenderer":{"skipOffsetMilliseconds":2500}}}},{"adPlacementRenderer":{"config":{"adPlacementConfig":{"kind":"AD_PLACEMENT_KIND_END"}},"renderer":{"instreamVideoAdRenderer":{"skipOffsetMilliseconds":9000}}}}]"#;
        let p = parse_player_response(&with_ads(pod), VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.gate, Some(Duration::from_millis(13_500)));
    }

    #[test]
    fn an_answer_without_ads_before_the_song_is_not_gated() {
        let after = AD_SLOT.replace(
            "SLOT_TRIGGER_EVENT_BEFORE_CONTENT",
            "SLOT_TRIGGER_EVENT_LAYOUT_ID_ENTERED",
        );
        let p = parse_player_response(&with_ads(&after), VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.gate, None);
        let p = parse_player_response(PLAYER, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.gate, None);
    }

    #[test]
    fn params_are_replaced_or_added() {
        assert_eq!(
            with_param("https://h/p?a=1&n=x&b=2", "n", "y z"),
            "https://h/p?a=1&n=y%20z&b=2"
        );
        assert_eq!(
            with_param("https://h/p?a=1", "sig", "s"),
            "https://h/p?a=1&sig=s"
        );
        assert_eq!(
            with_param("https://h/p?an=1", "n", "2"),
            "https://h/p?an=1&n=2"
        );
    }

    #[test]
    fn the_transform_cache_keeps_the_newest_players() {
        let root = home("cache");
        let dir = root.join("players");
        assert_eq!(installed(&dir, "aaaa0001"), None);
        let p = install(&dir, "aaaa0001", 20000, FAKE_PLAYER).unwrap();
        assert_eq!(installed(&dir, "aaaa0001"), Some(p.clone()));
        assert_eq!(p.sts, 20000);
        let text = std::fs::read_to_string(&p.path).unwrap();
        assert!(text.ends_with(FAKE_PLAYER), "the player follows the header");
        let bytecode = js::bytecode_path(&p.path);
        std::fs::write(&bytecode, b"its bytecode").unwrap();
        for (i, id) in ["aaaa0002", "aaaa0003", "aaaa0004"].iter().enumerate() {
            std::thread::sleep(Duration::from_millis(20));
            install(&dir, id, 20001 + i as u64, FAKE_PLAYER).unwrap();
        }
        assert_eq!(installed(&dir, "aaaa0001"), None, "the oldest went");
        assert!(!bytecode.exists(), "and its bytecode with it");
        assert_eq!(newest(&dir, |_| true).unwrap().id, "aaaa0004");
        for id in ["aaaa0002", "aaaa0003", "aaaa0004"] {
            assert!(installed(&dir, id).is_some(), "{id}");
        }
        // Made by other EJS scripts: made again.
        let other = format!("// featherplay player bbbb0001 sts 1 ejs 0.0.1\n{FAKE_PLAYER}");
        std::fs::write(dir.join("bbbb0001.js"), other).unwrap();
        assert_eq!(installed(&dir, "bbbb0001"), None);
        assert!(install(&dir, "../evil", 1, "").is_err());
        assert_eq!(installed(&dir, "../aaaa0002"), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A resolver over `base` whose state and player are already on disk, asking as the
    /// embed alone: these tests are about that client.
    fn seeded(name: &str, base: &str) -> (NativeResolver, PathBuf) {
        let (r, dir) = seeded_with_visionos(name, base);
        embedded_only(&r);
        (r, dir)
    }

    /// Resolves ask the embedded client alone, as after repeated visionOS failures.
    fn embedded_only(r: &NativeResolver) {
        r.inner.health.lock().unwrap().visionos_off_until =
            Some(Instant::now() + Duration::from_secs(3600));
    }

    /// A resolver over `base` whose state and player are already on disk.
    fn seeded_with_visionos(name: &str, base: &str) -> (NativeResolver, PathBuf) {
        let dir = home(name);
        let player = install(&dir.join("players"), "096efeec", 20752, FAKE_PLAYER).unwrap();
        // Its bytecode now, so no save runs beside the resolves and the folder can go.
        js::warm(&player.path).unwrap();
        let r = NativeResolver::with_dirs(dir.clone(), base).unwrap();
        r.inner.save(&Ready {
            config: config(),
            player,
            fetched: SystemTime::now(),
            page: 0,
        });
        (r, dir)
    }

    #[tokio::test]
    async fn a_native_resolve_is_one_request() {
        let (base, log) = test_server::serve(|method, path, body| {
            match (method, path.split('?').next().unwrap()) {
                ("POST", "/youtubei/v1/player") if body.contains(VIDEO) => {
                    (200, PLAYER.to_string())
                }
                _ => (404, String::new()),
            }
        });
        let (r, dir) = seeded("resolve", &base);
        let s = r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(
            *log.lock().unwrap(),
            ["POST /youtubei/v1/player?prettyPrint=false"]
        );
        assert_eq!(
            query_param(&s.url, "n").as_deref(),
            Some("dEASjGHLUNUj7mJZgbt")
        );
        let s_in = parse_player_response(PLAYER, VIDEO, Quality::High.itags())
            .unwrap()
            .sig
            .unwrap()
            .1;
        let want: String = s_in[2..].chars().rev().collect();
        assert_eq!(query_param(&s.url, "sig"), Some(want));
        assert_eq!((s.itag, s.mime.as_str()), (251, "audio/webm"));
        assert_eq!(s.video_title.as_deref(), Some("affection"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A server for both clients: `visionos` answers the visionOS client's requests that
    /// come with the kept visitor, the recorded embedded answer the others.
    fn two_clients(visionos: String) -> (String, test_server::Log) {
        let kept = config().visitor_data.unwrap();
        test_server::serve(move |method, path, body| {
            match (method, path.split('?').next().unwrap()) {
                ("POST", "/youtubei/v1/player") if body.contains(r#""VISIONOS""#) => {
                    if body.contains(&kept) {
                        (200, visionos.clone())
                    } else {
                        (400, String::new())
                    }
                }
                ("POST", "/youtubei/v1/player") => (200, PLAYER.to_string()),
                _ => (404, String::new()),
            }
        })
    }

    fn client(s: &ResolvedStream) -> String {
        query_param(&s.url, "c").unwrap_or_default()
    }

    /// The visionOS client is asked first, with the kept visitor: one request, and its URL
    /// as it came.
    #[tokio::test]
    async fn a_resolve_asks_as_visionos_first() {
        let (base, log) = two_clients(VISIONOS_PLAYER.to_string());
        let (r, dir) = seeded_with_visionos("visionos", &base);
        let s = r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(log.lock().unwrap().len(), 1, "one request");
        assert_eq!((client(&s).as_str(), s.itag), ("VISIONOS", 251));
        let want = parse_player_response(VISIONOS_PLAYER, VIDEO, Quality::High.itags());
        assert_eq!(s.url, want.unwrap().url);
        assert_eq!(s.available_at, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// No stream from the visionOS client (here a sign-in it asks for): the embedded one
    /// answers. Three in a row leave resolves to the embedded client alone for a while,
    /// then one resolve tries it, and one more failure turns it off again.
    #[tokio::test]
    async fn without_a_visionos_stream_the_embedded_client_answers() {
        let bot = r#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm you're not a bot"}}"#;
        let (base, log) = two_clients(bot.to_string());
        let (r, dir) = seeded_with_visionos("no-visionos", &base);
        for asked in [2, 4, 6] {
            let s = r.resolve(VIDEO, Quality::High).await.unwrap();
            assert_eq!(client(&s), "WEB_EMBEDDED_PLAYER");
            assert_eq!(log.lock().unwrap().len(), asked, "visionOS, then the embed");
        }
        r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(log.lock().unwrap().len(), 7, "the embed alone after three");
        r.inner.health.lock().unwrap().visionos_off_until = Some(Instant::now());
        r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(
            log.lock().unwrap().len(),
            9,
            "tried once after the time off"
        );
        r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(log.lock().unwrap().len(), 10, "off again after one failure");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A visionOS answer with ads before the song would wait as long as an embedded one,
    /// which may come without: the embedded client answers.
    #[tokio::test]
    async fn a_visionos_answer_with_ads_goes_to_the_embedded_client() {
        let gated = VISIONOS_PLAYER.replacen('{', &format!("{{{AD_SLOT},"), 1);
        let (base, log) = two_clients(gated);
        let (r, dir) = seeded_with_visionos("visionos-ads", &base);
        let s = r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(client(&s), "WEB_EMBEDDED_PLAYER");
        assert_eq!(log.lock().unwrap().len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// What the visionOS client won't give for the song's sake (an age check, a song that's
    /// unavailable) doesn't turn it off, nor do songs prepared aside: after as many as would
    /// turn it off, the next click still asks it first.
    #[tokio::test]
    async fn song_refusals_and_resolves_aside_leave_visionos_on() {
        let age = r#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm your age"}}"#;
        let gone =
            r#"{"playabilityStatus":{"status":"ERROR","reason":"This video is unavailable"}}"#;
        let bot = r#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm you're not a bot"}}"#;
        for (name, answer, aside) in [
            ("age", age, false),
            ("gone", gone, false),
            ("aside", bot, true),
        ] {
            let (base, log) = two_clients(answer.to_string());
            let (r, dir) = seeded_with_visionos(&format!("visionos-{name}"), &base);
            for _ in 0..VISIONOS_MAX_FAILURES {
                let s = if aside {
                    r.resolve_aside(VIDEO, Quality::High).await
                } else {
                    r.resolve(VIDEO, Quality::High).await
                };
                assert_eq!(client(&s.unwrap()), "WEB_EMBEDDED_PLAYER", "{name}");
            }
            let n = log.lock().unwrap().len();
            r.resolve(VIDEO, Quality::High).await.unwrap();
            assert_eq!(
                log.lock().unwrap().len(),
                n + 2,
                "{name}: visionOS wasn't asked"
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// "This video is unavailable" for one song is the song's, but for different songs in a
    /// row it's how YouTube answers a client config it no longer takes: that turns it off.
    #[test]
    fn unavailable_for_different_songs_in_a_row_turns_visionos_off() {
        let gone = Err(failed("ERROR: This video is unavailable"));
        let mut h = Health::default();
        for _ in 0..VISIONOS_MAX_FAILURES {
            h.visionos_answered("aaaaaaaaaaa", &gone, true);
        }
        assert!(h.visionos_on(), "off for one song");
        h.visionos_answered("bbbbbbbbbbb", &gone, true);
        assert!(h.visionos_on(), "off for two songs");
        h.visionos_answered("ccccccccccc", &gone, true);
        assert!(!h.visionos_on(), "still on after three songs");
    }

    /// A refused visionOS URL leaves resolves to the embedded client for a while, at once.
    #[tokio::test]
    async fn a_refused_visionos_url_leaves_resolves_to_the_embedded_client() {
        let (base, log) = two_clients(VISIONOS_PLAYER.to_string());
        let (r, dir) = seeded_with_visionos("visionos-refused", &base);
        r.visionos_refused();
        let s = r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(client(&s), "WEB_EMBEDDED_PLAYER");
        assert_eq!(log.lock().unwrap().len(), 1, "no visionOS request");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A fresh answer asks with a new embed page's visitor, and leaves the kept page to
    /// the resolves after it.
    #[tokio::test]
    async fn a_fresh_resolve_uses_a_new_embed_page_once() {
        let kept = config().visitor_data.unwrap();
        let page = EMBED.replace(&kept, "fresh-visitor");
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let seen = bodies.clone();
        let (base, log) = test_server::serve(move |method, path, body| {
            match (method, path.split('?').next().unwrap()) {
                ("GET", p) if p == format!("/embed/{VIDEO}") => (200, page.clone()),
                ("POST", "/youtubei/v1/player") => {
                    seen.lock().unwrap().push(body.to_string());
                    (200, PLAYER.to_string())
                }
                _ => (404, String::new()),
            }
        });
        let (r, dir) = seeded("fresh", &base);
        r.resolve_fresh(VIDEO, Quality::High).await.unwrap();
        r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(
            *log.lock().unwrap(),
            [
                format!("GET /embed/{VIDEO}?html5=1"),
                "POST /youtubei/v1/player?prettyPrint=false".into(),
                "POST /youtubei/v1/player?prettyPrint=false".into(),
            ]
        );
        let bodies = bodies.lock().unwrap();
        assert!(bodies[0].contains("fresh-visitor"), "{}", bodies[0]);
        assert!(bodies[1].contains(&kept), "{}", bodies[1]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_failed_request_forgets_the_config_and_three_turn_it_off() {
        let (base, log) = test_server::serve(|_, _, _| (500, String::new()));
        let (r, dir) = seeded("fails", &base);
        let e = r.resolve(VIDEO, Quality::High).await.unwrap_err();
        assert!(matches!(e, NativeError::Failed(_)), "{e:?}");
        assert!(
            r.inner.forget.lock().unwrap().is_some(),
            "the next resolve fetches the embed page"
        );
        for _ in 0..2 {
            let _ = r.resolve(VIDEO, Quality::High).await;
        }
        let n = log.lock().unwrap().len();
        assert!(r.resolve(VIDEO, Quality::High).await.is_err());
        assert_eq!(
            log.lock().unwrap().len(),
            n,
            "off: no request after three failures"
        );
        // Once the time off is over, one resolve tries; failing, it turns it off again.
        r.inner.health.lock().unwrap().off_until = Some(Instant::now());
        assert!(r.resolve(VIDEO, Quality::High).await.is_err());
        let n = log.lock().unwrap().len();
        assert!(r.resolve(VIDEO, Quality::High).await.is_err());
        assert_eq!(log.lock().unwrap().len(), n, "off again after one failure");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Songs prepared in case they're clicked fail without counting: a page of them can't
    /// turn the step off for the clicks.
    #[tokio::test]
    async fn failures_of_songs_prepared_aside_do_not_turn_it_off() {
        let (base, log) = test_server::serve(|_, _, _| (500, String::new()));
        let (r, dir) = seeded("aside", &base);
        for _ in 0..MAX_FAILURES {
            assert!(r.resolve_aside(VIDEO, Quality::High).await.is_err());
        }
        let n = log.lock().unwrap().len();
        assert!(r.resolve(VIDEO, Quality::High).await.is_err());
        assert!(log.lock().unwrap().len() > n, "still on: the click asks");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Some new visitors get answers without URLs, every time, while the kept one and other
    /// new ones get URLs for the same song (2026-11-09). An answer without a URL drops the
    /// kept page from disk at once, so a start after a short session doesn't load it again.
    #[tokio::test]
    async fn an_answer_without_a_url_drops_the_kept_page_for_the_next_start() {
        let answer = without_urls();
        let page = EMBED.replace(&config().visitor_data.unwrap(), "fresh-visitor");
        let (base, _log) = test_server::serve(move |_, path, _| {
            if path.starts_with("/embed/") {
                (200, page.clone())
            } else {
                (200, answer.clone())
            }
        });
        let (r, dir) = seeded("no-url", &base);
        let e = r.resolve(VIDEO, Quality::High).await.unwrap_err();
        assert!(matches!(e, NativeError::NoUrl(_)), "{e:?}");
        let next_start = NativeResolver::with_dirs(dir.clone(), &base).unwrap();
        assert!(
            next_start.inner.load().is_none(),
            "the next start fetches a fresh embed page"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A kept state whose time is past what the clock can hold (damaged) is read as none,
    /// without a panic: the next start fetches the page again.
    #[test]
    fn a_kept_state_fetched_too_far_ahead_is_not_loaded() {
        let (_r, dir) = seeded("far-fetched", "http://127.0.0.1:9");
        let path = dir.join("state.json");
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["fetched"] = u64::MAX.into();
        std::fs::write(&path, v.to_string()).unwrap();
        let next_start = NativeResolver::with_dirs(dir.clone(), "http://127.0.0.1:9").unwrap();
        assert!(next_start.inner.load().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `PLAYER` with its formats' URLs taken out (a SABR-only answer).
    fn without_urls() -> String {
        let mut answer: Value = serde_json::from_str(PLAYER).unwrap();
        for f in answer["streamingData"]["adaptiveFormats"]
            .as_array_mut()
            .unwrap()
        {
            let f = f.as_object_mut().unwrap();
            f.remove("url");
            f.remove("signatureCipher");
        }
        answer.to_string()
    }

    /// The three resolves a start fires at once all got answers without URLs from the kept
    /// visitor, which turned the native step off for 30 min (2026-11-09 13:42), while
    /// yt-dlp's fresh visitors got URLs. Each asks once more, from one fresh page they share.
    #[tokio::test]
    async fn answers_without_urls_from_the_kept_page_ask_again_from_one_fresh_page() {
        let kept = config().visitor_data.unwrap();
        let page = EMBED.replace(&kept, "fresh-visitor");
        let bad = without_urls();
        let (base, log) = test_server::serve(move |method, path, body| {
            match (method, path.split('?').next().unwrap()) {
                ("GET", p) if p.starts_with("/embed/") => (200, page.clone()),
                ("POST", "/youtubei/v1/player") if body.contains(&kept) => (200, bad.clone()),
                ("POST", "/youtubei/v1/player") => (200, PLAYER.to_string()),
                _ => (404, String::new()),
            }
        });
        let (r, dir) = seeded("no-url-again", &base);
        let (a, b, c) = tokio::join!(
            r.resolve(VIDEO, Quality::High),
            r.resolve(VIDEO, Quality::High),
            r.resolve(VIDEO, Quality::High),
        );
        for s in [a, b, c] {
            assert_eq!(s.unwrap().itag, 251);
        }
        let pages = log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("GET /embed/"))
            .count();
        assert_eq!(pages, 1, "one fresh page for the three");
        assert_eq!(r.inner.health.lock().unwrap().failures, 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Answers without URLs from fresh pages too: the failures of one page count once, so
    /// a start's three resolves at once don't turn the native step off.
    #[tokio::test]
    async fn the_failures_of_one_page_count_once() {
        let kept = config().visitor_data.unwrap();
        let fetched = std::sync::atomic::AtomicUsize::new(0);
        let bad = without_urls();
        let (base, log) = test_server::serve(move |_, path, _| {
            if path.starts_with("/embed/") {
                let n = fetched.fetch_add(1, Ordering::Relaxed);
                (200, EMBED.replace(&kept, &format!("visitor-{n}")))
            } else {
                (200, bad.clone())
            }
        });
        let (r, dir) = seeded("no-url-once", &base);
        let (a, b, c) = tokio::join!(
            r.resolve(VIDEO, Quality::High),
            r.resolve(VIDEO, Quality::High),
            r.resolve(VIDEO, Quality::High),
        );
        for e in [a, b, c] {
            assert!(matches!(e, Err(NativeError::NoUrl(_))), "{e:?}");
        }
        let n = log.lock().unwrap().len();
        let _ = r.resolve(VIDEO, Quality::High).await;
        assert!(
            log.lock().unwrap().len() > n,
            "still on: the next resolve asks"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An embed page that names player `abcd1234`, that player's JS (which has an `sts`),
    /// and the player API's recorded answer.
    fn new_player_server() -> (String, test_server::Log) {
        test_server::serve(|method, path, _| {
            if path.starts_with("/embed/") {
                (200, EMBED.replace("096efeec", "abcd1234"))
            } else if method == "POST" {
                (200, PLAYER.to_string())
            } else {
                (200, "var x={signatureTimestamp:20760};".into())
            }
        })
    }

    /// Stands in for the preprocessing: gives `player` for any JS.
    fn stub(player: &'static str) -> Preprocessor {
        Preprocessor::Stub(Arc::new(move |_: &str| Ok(player.to_string())))
    }

    /// YouTube moves on to a new player a few times a week, and preprocessing one takes
    /// seconds: the song is answered with the older player meanwhile, and the new one
    /// takes over once it's installed.
    #[tokio::test]
    async fn a_new_player_is_installed_behind_the_older_ones_answer() {
        /// `n` in upper case: not what the older player answers.
        const NEW_PLAYER: &str = "_result.n = (n) => n.toUpperCase(); _result.sig = (s) => s;";
        let (base, log) = new_player_server();
        let (mut r, dir) = seeded("new-player", &base);
        // Preprocessing ends when the test lets it.
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        r.set_preprocessor(Preprocessor::Stub(Arc::new(move |_: &str| {
            let _ = wait.lock().unwrap().recv();
            Ok(NEW_PLAYER.to_string())
        })));
        // The kept page goes: the next one names `abcd1234`.
        r.refused();
        let s = r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(
            query_param(&s.url, "n").as_deref(),
            Some("dEASjGHLUNUj7mJZgbt"),
            "the older player's answer, while the new one is being made"
        );
        go.send(()).unwrap();
        for _ in 0..500 {
            let st = r.inner.state.lock().await;
            if st.as_ref().is_some_and(|st| st.player.id == "abcd1234") {
                break;
            }
            drop(st);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let s = r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(
            query_param(&s.url, "n").as_deref(),
            Some("TBGZJM7JUNULHGJSAED")
        );
        let new = installed(&dir.join("players"), "abcd1234").unwrap();
        assert!(
            js::bytecode_path(&new.path).is_file(),
            "the install saved its bytecode"
        );
        let log = log.lock().unwrap();
        let fetched = log.iter().filter(|l| l.contains("/s/player/"));
        assert_eq!(fetched.count(), 1);
        drop(log);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A player whose challenges couldn't be solved isn't the one answering from then on,
    /// though the kept state names it: the next resolve fetches a page and answers with the
    /// player that page names, installed now.
    #[tokio::test]
    async fn a_player_that_couldnt_be_solved_gives_way_to_the_next_one() {
        let (base, log) = new_player_server();
        let (mut r, dir) = seeded("bad-player", &base);
        r.set_preprocessor(stub(FAKE_PLAYER));
        r.inner.health.lock().unwrap().bad.insert("096efeec".into());
        r.resolve(VIDEO, Quality::High).await.unwrap();
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("/s/player/abcd1234/")),
            "the page's player wasn't installed: {:?}",
            log.lock().unwrap()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A player whose challenges throw or run past the solver's limit can't be solved; one
    /// the solver once didn't answer for (a busy machine) or that stopped (a thread that
    /// couldn't start) is tried again, but not after it didn't answer again.
    #[test]
    fn a_player_is_set_aside_for_its_own_failures_and_repeated_timeouts() {
        let bad = |errors: &[&str]| {
            let mut h = Health::default();
            for e in errors {
                h.solve_failed("p", e);
            }
            h.bad.contains("p")
        };
        assert!(bad(&["n: TypeError: not a function"]));
        assert!(bad(&[js::OVER_LIMIT]), "a run past the limit");
        assert!(!bad(&[js::STOPPED, js::STOPPED, js::STOPPED]));
        assert!(!bad(&[js::TIMED_OUT]), "one timeout");
        assert!(bad(&[js::TIMED_OUT, js::TIMED_OUT]), "timed out again");
    }

    /// A preprocessing child that ended without a reason (killed, crashed) says nothing of
    /// the player; one that failed says why on stderr.
    #[test]
    fn a_preprocessor_that_ends_without_a_reason_leaves_the_player_good() {
        let unsolved =
            |code, err: &str| matches!(child_failed(code, err), PreprocessError::Unsolved(_));
        assert!(unsolved(Some(1), "EJS: no n function"));
        assert!(!unsolved(Some(1), ""), "killed (TerminateProcess's code 1)");
        assert!(!unsolved(Some(-1_073_741_819), ""), "crashed");
        assert!(!unsolved(None, ""));
    }

    /// A first run has no player. The app's start fetches a page and installs its player
    /// (`prepare`), so the first song is one request, as any other.
    #[tokio::test]
    async fn a_first_run_has_its_player_before_the_first_song() {
        let (base, log) = new_player_server();
        let dir = home("first-run");
        let mut r = NativeResolver::with_dirs(dir.clone(), &base).unwrap();
        embedded_only(&r);
        r.set_preprocessor(stub(FAKE_PLAYER));
        r.prepare().await;
        assert_eq!(
            *log.lock().unwrap(),
            [
                "GET /embed/?html5=1",
                "GET /s/player/abcd1234/player_ias.vflset/en_US/base.js"
            ]
        );
        r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(
            log.lock().unwrap().len(),
            3,
            "the song asked the player API"
        );
        // With a player there, a start asks nothing.
        let next_start = NativeResolver::with_dirs(dir.clone(), &base).unwrap();
        next_start.prepare().await;
        assert_eq!(log.lock().unwrap().len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A child that can't be started (an antivirus may refuse it) leaves the preprocessing
    /// to this process: EJS itself answers, here that the stand-in player has nothing to
    /// find.
    #[tokio::test]
    async fn a_preprocessor_that_cannot_start_leaves_the_work_to_this_process() {
        let (base, _log) = new_player_server();
        let dir = home("no-child");
        let mut r = NativeResolver::with_dirs(dir.clone(), &base).unwrap();
        r.set_preprocessor(Preprocessor::Child(dir.join("no-such-program.exe")));
        let e = r.resolve(VIDEO, Quality::High).await.unwrap_err();
        assert!(e.to_string().contains("EJS"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// This process's private bytes, in MB.
    fn private_mb() -> f64 {
        let ask = format!(
            "(Get-Process -Id {}).PrivateMemorySize64",
            std::process::id()
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ask])
            .output()
            .unwrap();
        let bytes: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
        bytes / 1_048_576.0
    }

    /// A measurement on a real player, against a local server that answers at once, so a
    /// resolve's time is the solver's: 50 resolves and the memory after them, then a page
    /// that names a new player. `MEASURE_PLAYER`: a preprocessed player (a file of
    /// `{home}/resolver/players`); `MEASURE_BASE`: a player's JS as YouTube serves it;
    /// `MEASURE_PREPROCESSOR`: featherplay.exe, to preprocess in a child as the app does.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "a measurement: set MEASURE_PLAYER and MEASURE_BASE"]
    async fn measure_resolves_and_a_new_player() {
        let pre = std::fs::read_to_string(std::env::var("MEASURE_PLAYER").unwrap()).unwrap();
        let (head, body) = pre.split_once('\n').unwrap();
        let sts: u64 = head.split(' ').nth(5).unwrap().parse().unwrap();
        let base_js = std::fs::read_to_string(std::env::var("MEASURE_BASE").unwrap()).unwrap();
        let page = EMBED.replace("096efeec", "abcd1234");
        let (base, _log) = test_server::serve(move |method, path, _| {
            match (method, path.split('?').next().unwrap()) {
                ("POST", "/youtubei/v1/player") => (200, PLAYER.to_string()),
                ("GET", p) if p.starts_with("/embed/") => (200, page.clone()),
                ("GET", p) if p.starts_with("/s/player/abcd1234/") => (200, base_js.clone()),
                _ => (404, String::new()),
            }
        });
        let dir = home("measure");
        let player = install(&dir.join("players"), "096efeec", sts, body).unwrap();
        let mut r = NativeResolver::with_dirs(dir.clone(), &base).unwrap();
        embedded_only(&r);
        if let Ok(exe) = std::env::var("MEASURE_PREPROCESSOR") {
            r.set_preprocessor(Preprocessor::Child(exe.into()));
        }
        r.inner.save(&Ready {
            config: config(),
            player,
            fetched: SystemTime::now(),
            page: 0,
        });
        println!("private at start: {:.1} MB", private_mb());
        let mut times = Vec::new();
        for i in 0..50 {
            let t = Instant::now();
            let s = r.resolve(VIDEO, Quality::High).await.unwrap();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            if i == 0 {
                println!("n = {:?}", query_param(&s.url, "n"));
            }
            if i % 10 == 9 {
                println!("private after {} resolves: {:.1} MB", i + 1, private_mb());
            }
        }
        let (first, second) = (times[0], times[1]);
        times.sort_by(f64::total_cmp);
        println!(
            "50 resolves: first {first:.1} ms (from the source), second {second:.1} ms, \
             median {:.1} ms, min {:.1} ms, max {:.1} ms",
            times[25], times[0], times[49]
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
        println!("private after 50 resolves: {:.1} MB", private_mb());
        // A new player appears: the kept page is dropped, the next page names `abcd1234`.
        r.refused();
        let appeared = Instant::now();
        let mut peak = private_mb();
        for i in 0.. {
            let t = Instant::now();
            r.resolve(VIDEO, Quality::High).await.unwrap();
            let took = t.elapsed().as_secs_f64() * 1000.0;
            let player = r
                .inner
                .state
                .lock()
                .await
                .as_ref()
                .unwrap()
                .player
                .id
                .clone();
            peak = peak.max(private_mb());
            println!(
                "new player, resolve {i} at {:.1} s: {took:.1} ms (player {player}), private {peak:.1} MB at most",
                appeared.elapsed().as_secs_f64()
            );
            if player == "abcd1234" || appeared.elapsed() > Duration::from_secs(60) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        println!("private at the end: {:.1} MB", private_mb());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
