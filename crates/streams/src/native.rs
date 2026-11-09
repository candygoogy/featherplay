//! Resolves a stream with one InnerTube `player` request made from Rust, as the embedded
//! player's client (`WEB_EMBEDDED_PLAYER`, yt-dlp's `web_embedded`), instead of a yt-dlp run.
//! A yt-dlp run takes ~2.6 s: 0.6 s to start, the embed page, the player JS, the player API,
//! 0.6 s to parse the whole player in deno, 0.35 s to exit.
//!
//! - The embed page gives the client version, the visitor data, the host flags and the
//!   player's version. They're kept in `{home}/resolver/state.json` and fetched again after
//!   `REFRESH_AFTER`, in the background, after a resolve that used the old ones.
//! - Each player version is downloaded and preprocessed once, with yt-dlp's EJS solver
//!   scripts (`ejs/`) in deno, into `{home}/resolver/players/{id}.js`: the cached transform.
//!   The last `KEEP_PLAYERS` are kept.
//! - A resolve then starts deno on that file (it loads in ~0.2 s) while the player API
//!   request runs, and hands it the `n` and `sig` challenges once the answer is in.
//!
//! `YtDlpResolver` falls back to yt-dlp on any error here.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use featherplay_core::sync::LockExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::expiry::{form_value, query_param};
use crate::{Quality, ResolvedStream, Sidecars, expires_at_from_url};

/// yt-dlp's EJS challenge solver, v0.8.0 (Unlicense; the lib bundles meriyah, ISC, and
/// astring, MIT). Only preprocessing a new player needs them. The lib is deflated by
/// `build.rs` ([`ejs_lib`]).
const EJS_LIB_DEFLATED: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/yt.solver.lib.min.js.deflate"));
const EJS_CORE: &str = include_str!("../ejs/yt.solver.core.min.js");
/// Preprocessed players carry it; one made by other scripts is made again.
const EJS_VERSION: &str = "0.8.0";
/// Runs per resolve: loads a preprocessed player, answers the challenges from stdin.
const SOLVE_JS: &str = include_str!("../ejs/solve.js");

const ORIGIN: &str = "https://www.youtube.com";
const CLIENT_NAME: &str = "WEB_EMBEDDED_PLAYER";
const CLIENT_ID: &str = "56";
/// yt-dlp's (2026.10.26), for an embed page that has none.
const CLIENT_VERSION: &str = "2.20260708.00.00";
/// yt-dlp's for this client.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15,gzip(gfe)";
/// The page the player is "embedded" in: any page outside YouTube (yt-dlp's choice).
const EMBED_URL: &str = "https://www.reddit.com/";
const REFRESH_AFTER: Duration = Duration::from_secs(12 * 3600);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);
const SOLVE_TIMEOUT: Duration = Duration::from_secs(10);
const PREPROCESS_TIMEOUT: Duration = Duration::from_secs(30);
const KEEP_PLAYERS: usize = 3;
/// After this many failures in a row, resolves go straight to yt-dlp for `OFF_FOR`, and
/// then one failure turns it off again. Resolves at once from one page that fail count
/// once: on 2026-11-09 a start's three resolves failed together on one bad visitor, and
/// 30 min off cost every song 2.6-10.8 s, while each fresh visitor got URLs.
const MAX_FAILURES: u32 = 3;
const OFF_FOR: Duration = Duration::from_secs(5 * 60);

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
    /// Usually `config.player`; an older one when the new one couldn't be preprocessed.
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
    sidecars: Sidecars,
    /// `{home}/resolver`
    dir: PathBuf,
    base: String,
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
}

impl NativeResolver {
    /// `None` when no HTTP client can be built (then yt-dlp does every resolve).
    pub(crate) fn new(sidecars_dir: PathBuf) -> Option<Self> {
        Self::with_dirs(sidecars_dir, None, ORIGIN)
    }

    /// `dir`: where the state and the players go (`{home}/resolver` by default); `base`:
    /// `https://www.youtube.com`, or a test server.
    pub(crate) fn with_dirs(
        sidecars_dir: PathBuf,
        dir: Option<PathBuf>,
        base: &str,
    ) -> Option<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(5))
            // No idle timeout, so no reaper timer runs while paused: an idle connection
            // stays until the server closes it, and a click minutes later may reuse it.
            .pool_idle_timeout(None::<Duration>)
            .pool_max_idle_per_host(2)
            .build()
            .ok()?;
        let sidecars = Sidecars { dir: sidecars_dir };
        let dir = dir.unwrap_or_else(|| sidecars.home().join("resolver"));
        Some(Self {
            inner: Arc::new(Inner {
                http,
                sidecars,
                dir,
                base: base.trim_end_matches('/').to_string(),
                state: tokio::sync::Mutex::new(None),
                forget: Mutex::new(None),
                pages: AtomicU64::new(0),
                refreshing: AtomicBool::new(false),
                health: Mutex::new(Health::default()),
            }),
        })
    }

    /// A URL this client gave was refused: the next resolve starts from a fresh embed page.
    pub(crate) fn refused(&self) {
        self.inner.drop_page(None);
    }

    pub(crate) async fn resolve(
        &self,
        video: &str,
        quality: Quality,
    ) -> Result<ResolvedStream, NativeError> {
        let inner = &self.inner;
        if !inner.sidecars.deno().is_file() {
            // Not installed yet (a first run): no request is worth it, and it's no failure
            // of this path, so it doesn't count towards turning it off.
            return Err(failed("deno is missing"));
        }
        if inner
            .health
            .lock_or_recover()
            .off_until
            .is_some_and(|t| Instant::now() < t)
        {
            return Err(failed("off for a while after repeated failures"));
        }
        let (result, page) = inner.resolve(video, quality).await;
        let mut h = inner.health.lock_or_recover();
        match &result {
            Ok(_) | Err(NativeError::Skip(_)) => {
                h.failures = 0;
                h.failed_page = None;
            }
            Err(e @ (NativeError::Failed(_) | NativeError::NoUrl(_))) => {
                // Other failures (deno, a player's challenges) count each time.
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
    /// moved on, then it's installed as for a refresh. Its URL refused later drops the kept
    /// page as any refused URL does (`refused`): rare, and a fresh page is cheap.
    pub(crate) async fn resolve_fresh(
        &self,
        video: &str,
        quality: Quality,
    ) -> Result<ResolvedStream, NativeError> {
        let inner = &self.inner;
        if !inner.sidecars.deno().is_file() {
            return Err(failed("deno is missing"));
        }
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
            if let Ok(ready) = inner.fetch_ready(&video, old).await {
                inner.save(&ready);
                *inner.state.lock().await = Some(ready);
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

    /// A resolve from the kept page, and that page (`Ready::page`). An answer without a
    /// URL asks once more from a fresh page (~0.45 s): some visitors get none every time
    /// while new ones do (2026-11-09). Resolves at once that get none share the first
    /// one's fresh page.
    async fn resolve(
        &self,
        video: &str,
        quality: Quality,
    ) -> (Result<ResolvedStream, NativeError>, Option<u64>) {
        let ready = match self.ready(video).await {
            Ok(r) => r,
            Err(e) => return (Err(e), None),
        };
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
        let solver = Solver::start(&self.sidecars, &self.solve_script()?, &player.path)
            .map_err(|e| failed(format!("starting deno: {e}")))?;
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
                    self.health.lock_or_recover().bad.insert(player.id.clone());
                    forget();
                    return Err(failed(format!("player {}: {e}", player.id)));
                }
            }
        };
        finish(pending, &answers, answered)
    }

    /// The config and player to use: in memory, else on disk, else fetched.
    async fn ready(&self, video: &str) -> Result<Ready, NativeError> {
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
        if self.health.lock_or_recover().bad.contains(&ready.player.id) {
            *st = None;
            return Err(failed(format!(
                "player {} couldn't be solved",
                ready.player.id
            )));
        }
        Ok(ready)
    }

    /// A fresh embed page and its player. `fallback` is used if that player can't be
    /// installed (YouTube accepts an older player's `sts` for a while).
    async fn fetch_ready(
        &self,
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
        let player = match self.install_player(&config.player).await {
            Ok(p) => p,
            Err(e) => match fallback {
                Some(old) if old.path.is_file() => old,
                _ => return Err(e),
            },
        };
        Ok(Ready {
            config,
            player,
            fetched: SystemTime::now(),
            page: self.next_page(),
        })
    }

    async fn install_player(&self, id: &str) -> Result<Player, NativeError> {
        let players = self.dir.join("players");
        if let Some(p) = installed(&players, id) {
            return Ok(p);
        }
        if self.health.lock_or_recover().bad.contains(id) {
            return Err(failed(format!("player {id} couldn't be solved")));
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
        let pre = preprocess(&self.sidecars, &js).await.map_err(|e| match e {
            // Says nothing about the player: it's tried again next time.
            PreprocessError::NotStarted(e) => e,
            PreprocessError::Unsolved(e) => {
                self.health.lock_or_recover().bad.insert(id.to_string());
                e
            }
        })?;
        install(&players, id, sts, &pre).map_err(|e| failed(format!("saving player {id}: {e}")))
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
        let mut req = self
            .http
            .post(format!(
                "{}/youtubei/v1/player?prettyPrint=false",
                self.base
            ))
            .timeout(REQUEST_TIMEOUT)
            .header("Content-Type", "application/json")
            .header("Origin", ORIGIN);
        for (k, v) in player_headers(&ready.config) {
            req = req.header(k, v);
        }
        let resp = req
            .body(player_request_body(video, &ready.config, ready.player.sts).to_string())
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

    /// `{dir}/solve.js`, written when missing or out of date.
    fn solve_script(&self) -> Result<PathBuf, NativeError> {
        let path = self.dir.join("solve.js");
        if std::fs::read_to_string(&path).ok().as_deref() != Some(SOLVE_JS) {
            std::fs::create_dir_all(&self.dir)
                .and_then(|()| write_atomic(&path, SOLVE_JS.as_bytes()))
                .map_err(|e| failed(format!("writing solve.js: {e}")))?;
        }
        Ok(path)
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
            fetched: UNIX_EPOCH + Duration::from_secs(v["fetched"].as_u64()?),
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
    let available_at = p.gate.map(|gate| {
        let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
        let whole = since.as_secs() + u64::from(since.subsec_nanos() > 0);
        UNIX_EPOCH + Duration::from_secs(whole) + gate
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

/// Saves a preprocessed player and drops all but the newest `KEEP_PLAYERS`.
pub(crate) fn install(players: &Path, id: &str, sts: u64, pre: &str) -> std::io::Result<Player> {
    if !valid_player_id(id) {
        return Err(std::io::Error::other("bad player id"));
    }
    std::fs::create_dir_all(players)?;
    let path = players.join(format!("{id}.js"));
    let body = format!("// featherplay player {id} sts {sts} ejs {EJS_VERSION}\n{pre}");
    write_atomic(&path, body.as_bytes())?;
    let mut others: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(players)?
        .filter_map(|e| {
            let p = e.ok()?.path();
            let ok = p != path && p.extension().is_some_and(|x| x == "js");
            ok.then(|| Some((p.metadata().ok()?.modified().ok()?, p)))?
        })
        .collect();
    others.sort();
    let drop_n = (others.len() + 1).saturating_sub(KEEP_PLAYERS);
    for (_, p) in others.into_iter().take(drop_n) {
        let _ = std::fs::remove_file(p);
    }
    Ok(Player {
        id: id.to_string(),
        sts,
        path,
    })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let part = path.with_extension("part");
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, path)
}

/// deno with no network, no prompts and no project files.
fn deno(sidecars: &Sidecars) -> tokio::process::Command {
    let mut cmd = sidecars.command(&sidecars.deno());
    cmd.args([
        "run",
        "--no-prompt",
        "--no-remote",
        "--no-lock",
        "--no-config",
        "--no-npm",
        "--node-modules-dir=none",
        "--cached-only",
    ])
    .stdin(Stdio::piped());
    cmd
}

/// Why a player couldn't be preprocessed.
enum PreprocessError {
    /// deno didn't start, so nothing is known about the player.
    NotStarted(NativeError),
    /// EJS ran on the player and failed, or took too long.
    Unsolved(NativeError),
}

/// The EJS library's source, inflated from what `build.rs` deflated.
fn ejs_lib() -> String {
    let mut js = String::new();
    flate2::read::DeflateDecoder::new(EJS_LIB_DEFLATED)
        .read_to_string(&mut js)
        .expect("build.rs deflates the EJS library from a UTF-8 file");
    js
}

/// Runs EJS on a player's JS once: the preprocessed player, which sets `_result.n` and
/// `_result.sig` when run as `Function("_result", code)`. Checked on a test challenge each.
async fn preprocess(sidecars: &Sidecars, js: &str) -> Result<String, PreprocessError> {
    let sig_test: String = (0u8..100).map(|b| (b'0' + b % 75) as char).collect();
    let input = json!({
        "type": "player",
        "player": js,
        "requests": [
            { "type": "n", "challenges": ["featherplayTest0"] },
            { "type": "sig", "challenges": [sig_test] },
        ],
        "output_preprocessed": true,
    });
    let ejs = ejs_lib();
    let script = format!(
        "{ejs}\nObject.assign(globalThis, lib);\n{EJS_CORE}\nconsole.log(JSON.stringify(jsc({input})));\n"
    );
    let mut cmd = deno(sidecars);
    cmd.args(["--ext=js", "--no-code-cache", "-"]);
    let out = run_with_stdin(cmd, script.as_bytes(), PREPROCESS_TIMEOUT).await?;
    let unsolved = |m: String| PreprocessError::Unsolved(failed(m));
    let v: Value =
        serde_json::from_str(out.trim()).map_err(|e| unsolved(format!("EJS output: {e}")))?;
    for r in v["responses"].as_array().into_iter().flatten() {
        if r["type"] != "result" {
            return Err(unsolved(format!(
                "EJS: {}",
                r["error"].as_str().unwrap_or("error")
            )));
        }
    }
    v["preprocessed_player"]
        .as_str()
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .ok_or_else(|| unsolved("EJS gave no preprocessed player".into()))
}

async fn run_with_stdin(
    mut cmd: tokio::process::Command,
    stdin: &[u8],
    timeout: Duration,
) -> Result<String, PreprocessError> {
    let mut child = cmd
        .spawn()
        .map_err(|e| PreprocessError::NotStarted(failed(format!("starting deno: {e}"))))?;
    let unsolved = |m: String| PreprocessError::Unsolved(failed(m));
    let mut pipe = child
        .stdin
        .take()
        .expect("callers pass a `deno()` command, which pipes stdin");
    let run = async {
        pipe.write_all(stdin).await?;
        drop(pipe);
        child.wait_with_output().await
    };
    let out = tokio::time::timeout(timeout, run)
        .await
        .map_err(|_| unsolved("deno timed out".into()))?
        .map_err(|e| unsolved(format!("deno: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(unsolved(format!(
            "deno exited with {}: {}",
            out.status.code().unwrap_or(-1),
            crate::sidecars::tail(&err, 300)
        )));
    }
    String::from_utf8(out.stdout).map_err(|_| unsolved("deno printed non-UTF-8".into()))
}

/// A deno process loading a preprocessed player, waiting for its challenges on stdin.
/// Killed if dropped before it answers.
pub(crate) struct Solver {
    child: tokio::process::Child,
}

impl Solver {
    pub(crate) fn start(
        sidecars: &Sidecars,
        script: &Path,
        player: &Path,
    ) -> std::io::Result<Self> {
        let mut cmd = deno(sidecars);
        let dir = player.parent().unwrap_or(player);
        cmd.arg(format!("--allow-read={}", dir.display()))
            .arg(script)
            .arg(player);
        Ok(Self {
            child: cmd.spawn()?,
        })
    }

    pub(crate) async fn solve(mut self, n: &[String], sig: &[String]) -> Result<Answers, String> {
        let input = json!({ "n": n, "sig": sig }).to_string();
        let mut pipe = self
            .child
            .stdin
            .take()
            .expect("`deno()` pipes stdin, taken once here");
        let mut stdout = self
            .child
            .stdout
            .take()
            .expect("`Sidecars::command` pipes stdout");
        let mut stderr = self
            .child
            .stderr
            .take()
            .expect("`Sidecars::command` pipes stderr");
        let run = async {
            pipe.write_all(input.as_bytes()).await?;
            drop(pipe);
            let (mut out, mut err) = (String::new(), String::new());
            let (a, b) = tokio::join!(
                stdout.read_to_string(&mut out),
                stderr.read_to_string(&mut err)
            );
            a?;
            b?;
            let status = self.child.wait().await?;
            Ok::<_, std::io::Error>((status, out, err))
        };
        let (status, out, err) = tokio::time::timeout(SOLVE_TIMEOUT, run)
            .await
            .map_err(|_| "the solver timed out".to_string())?
            .map_err(|e| format!("the solver: {e}"))?;
        if !status.success() {
            return Err(format!(
                "the solver failed: {}",
                crate::sidecars::tail(&err, 300)
            ));
        }
        let v: Value =
            serde_json::from_str(out.trim()).map_err(|e| format!("solver output: {e}"))?;
        let map = |k: &str| -> HashMap<String, String> {
            v[k].as_object()
                .into_iter()
                .flatten()
                .filter_map(|(c, a)| Some((c.clone(), a.as_str()?.to_string())))
                .collect()
        };
        Ok(Answers {
            n: map("n"),
            sig: map("sig"),
        })
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

    /// The sidecar dir with deno in it (`FEATHERPLAY_SIDECARS_DIR`), if there is one.
    fn deno_sidecars() -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var("FEATHERPLAY_SIDECARS_DIR").ok()?);
        dir.join("deno.exe").is_file().then_some(dir)
    }

    fn config() -> EmbedConfig {
        parse_embed_page(EMBED).unwrap()
    }

    #[test]
    fn the_ejs_library_inflates_to_its_source() {
        assert_eq!(ejs_lib(), include_str!("../ejs/yt.solver.lib.min.js"));
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
        for (i, id) in ["aaaa0002", "aaaa0003", "aaaa0004"].iter().enumerate() {
            std::thread::sleep(Duration::from_millis(20));
            install(&dir, id, 20001 + i as u64, FAKE_PLAYER).unwrap();
        }
        assert_eq!(installed(&dir, "aaaa0001"), None, "the oldest went");
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

    /// A resolver over `base` whose state and player are already on disk.
    fn seeded(name: &str, sidecars: PathBuf, base: &str) -> (NativeResolver, PathBuf) {
        let dir = home(name);
        let player = install(&dir.join("players"), "096efeec", 20752, FAKE_PLAYER).unwrap();
        let r = NativeResolver::with_dirs(sidecars, Some(dir.clone()), base).unwrap();
        r.inner.save(&Ready {
            config: config(),
            player,
            fetched: SystemTime::now(),
            page: 0,
        });
        (r, dir)
    }

    #[tokio::test]
    async fn the_solver_answers_from_a_cached_player() {
        let Some(sidecars) = deno_sidecars() else {
            eprintln!("skipped: no deno in FEATHERPLAY_SIDECARS_DIR");
            return;
        };
        let dir = home("solver");
        let p = install(&dir.join("players"), "096efeec", 20752, FAKE_PLAYER).unwrap();
        std::fs::write(dir.join("solve.js"), SOLVE_JS).unwrap();
        let s = Sidecars { dir: sidecars };
        let a = Solver::start(&s, &dir.join("solve.js"), &p.path)
            .unwrap()
            .solve(&["abc".into()], &["xy123".into()])
            .await
            .unwrap();
        assert_eq!(a.n["abc"], "cba");
        assert_eq!(a.sig["xy123"], "321");
        // A player without the functions fails, and says so.
        let bad = install(&dir.join("players"), "badd0001", 1, "_result.x = 1;").unwrap();
        let e = Solver::start(&s, &dir.join("solve.js"), &bad.path)
            .unwrap()
            .solve(&["abc".into()], &[])
            .await
            .unwrap_err();
        assert!(e.contains("no n function"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_native_resolve_is_one_request() {
        let Some(sidecars) = deno_sidecars() else {
            eprintln!("skipped: no deno in FEATHERPLAY_SIDECARS_DIR");
            return;
        };
        let (base, log) = test_server::serve(|method, path, body| {
            match (method, path.split('?').next().unwrap()) {
                ("POST", "/youtubei/v1/player") if body.contains(VIDEO) => {
                    (200, PLAYER.to_string())
                }
                _ => (404, String::new()),
            }
        });
        let (r, dir) = seeded("resolve", sidecars, &base);
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

    /// A fresh answer asks with a new embed page's visitor, and leaves the kept page to
    /// the resolves after it.
    #[tokio::test]
    async fn a_fresh_resolve_uses_a_new_embed_page_once() {
        let Some(sidecars) = deno_sidecars() else {
            eprintln!("skipped: no deno in FEATHERPLAY_SIDECARS_DIR");
            return;
        };
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
        let (r, dir) = seeded("fresh", sidecars, &base);
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
        let Some(sidecars) = deno_sidecars() else {
            eprintln!("skipped: no deno in FEATHERPLAY_SIDECARS_DIR");
            return;
        };
        let (base, log) = test_server::serve(|_, _, _| (500, String::new()));
        let (r, dir) = seeded("fails", sidecars, &base);
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

    /// Some new visitors get answers without URLs, every time, while the kept one and other
    /// new ones get URLs for the same song (2026-11-09). An answer without a URL drops the
    /// kept page from disk at once, so a start after a short session doesn't load it again.
    #[tokio::test]
    async fn an_answer_without_a_url_drops_the_kept_page_for_the_next_start() {
        let Some(sidecars) = deno_sidecars() else {
            eprintln!("skipped: no deno in FEATHERPLAY_SIDECARS_DIR");
            return;
        };
        let answer = without_urls();
        let page = EMBED.replace(&config().visitor_data.unwrap(), "fresh-visitor");
        let (base, _log) = test_server::serve(move |_, path, _| {
            if path.starts_with("/embed/") {
                (200, page.clone())
            } else {
                (200, answer.clone())
            }
        });
        let (r, dir) = seeded("no-url", sidecars.clone(), &base);
        let e = r.resolve(VIDEO, Quality::High).await.unwrap_err();
        assert!(matches!(e, NativeError::NoUrl(_)), "{e:?}");
        let next_start = NativeResolver::with_dirs(sidecars, Some(dir.clone()), &base).unwrap();
        assert!(
            next_start.inner.load().is_none(),
            "the next start fetches a fresh embed page"
        );
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
        let Some(sidecars) = deno_sidecars() else {
            eprintln!("skipped: no deno in FEATHERPLAY_SIDECARS_DIR");
            return;
        };
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
        let (r, dir) = seeded("no-url-again", sidecars, &base);
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
        let Some(sidecars) = deno_sidecars() else {
            eprintln!("skipped: no deno in FEATHERPLAY_SIDECARS_DIR");
            return;
        };
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
        let (r, dir) = seeded("no-url-once", sidecars, &base);
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

    /// The embed page with player `abcd1234`, and that player (which has an `sts`).
    fn embed_and_player() -> (String, test_server::Log) {
        test_server::serve(|_, path, _| {
            if path.starts_with("/embed/") {
                (200, r#"<script src="/s/player/abcd1234/base.js">"#.into())
            } else {
                (200, "var x={signatureTimestamp:20752};".into())
            }
        })
    }

    /// A first run resolves before deno is installed (v0.1.0, 2026-11-03): no request is
    /// made, and it counts as no failure, so the native path is still on once deno is in.
    #[tokio::test]
    async fn without_deno_nothing_is_asked_and_nothing_counts_as_a_failure() {
        let dir = home("no-deno");
        let sidecars = dir.join("sidecars");
        std::fs::create_dir_all(&sidecars).unwrap();
        let (base, log) = embed_and_player();
        let r = NativeResolver::with_dirs(sidecars, Some(dir.join("resolver")), &base).unwrap();
        for _ in 0..=MAX_FAILURES {
            let e = r.resolve(VIDEO, Quality::High).await.unwrap_err();
            assert_eq!(e, failed("deno is missing"));
        }
        assert!(log.lock().unwrap().is_empty(), "{:?}", log.lock().unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// deno that doesn't start says nothing about the player: it isn't marked as one that
    /// can't be solved (which lasted the whole session in v0.1.0), and is tried again.
    #[tokio::test]
    async fn a_player_is_not_marked_unsolvable_when_deno_did_not_start() {
        let dir = home("deno-wont-start");
        let sidecars = dir.join("sidecars");
        std::fs::create_dir_all(&sidecars).unwrap();
        std::fs::write(sidecars.join("deno.exe"), b"not a program").unwrap();
        let (base, log) = embed_and_player();
        let r = NativeResolver::with_dirs(sidecars, Some(dir.join("resolver")), &base).unwrap();
        for _ in 0..2 {
            let e = r
                .resolve(VIDEO, Quality::High)
                .await
                .unwrap_err()
                .to_string();
            assert!(e.contains("starting deno"), "{e}");
        }
        let players = log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("/s/player/abcd1234/"))
            .count();
        assert_eq!(players, 2, "the player was fetched and tried again");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
