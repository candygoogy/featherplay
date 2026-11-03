//! Whether a newer featherplay is out, from GitHub's latest release, and the open issue the
//! daily play check keeps when YouTube breaks playback. No account, no token, nothing
//! downloaded.
//!
//! At most once a day: at startup, once the first page's loads have all arrived, if
//! Settings' "Check for updates" is on. The answer is kept in `{cache}/update-check.json`
//! with the time of the check, so the banner in About survives a restart without a request.
//! A failed request (the repository is private until launch, so 404 is normal) is logged
//! once and counts as that day's check: no retry, no timer. When songs keep failing to play
//! after the helpers were updated, the controller asks again at once (`help`), once a
//! session.
//!
//! A newer release shows a toast once per version and a banner at the top of About.

use std::cell::{Cell, RefCell};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use featherplay_core::tr;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::Ctx;
use crate::{busy, pages, toast};

/// The project's pages on GitHub; every URL opened from a reply must start with it.
const PAGES: &str = "https://github.com/candygoogy/featherplay/";
const LATEST: &str = "https://api.github.com/repos/candygoogy/featherplay/releases/latest";
const SEARCH: &str = "https://api.github.com/search/issues";
/// The title `.github/workflows/daily-play.yml` gives its issue.
const ISSUE_TITLE: &str = "Daily play check failed";
const TIMEOUT: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
const STAMP: &str = "update-check.json";
/// The running version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// A published release: its version ("0.2.0") and its page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub url: String,
}

/// A link the "can't play" toast offers: the newer release, or the open issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Help {
    /// The toast's button: "Get it" or "Details".
    pub label: &'static str,
    pub url: String,
}

/// `x.y.z`, `v` in front or not; `-…` marks a pre-release; `+…` (build data) is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Version {
    core: [u64; 3],
    pre: bool,
}

fn version(text: &str) -> Option<Version> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let text = text.split_once('+').map_or(text, |(v, _)| v);
    let (core, pre) = match text.split_once('-') {
        Some((_, "")) => return None,
        Some((core, _)) => (core, true),
        None => (text, false),
    };
    let mut parts = core.split('.');
    let mut out = [0; 3];
    for slot in &mut out {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(Version { core: out, pre })
}

/// Whether the release tagged `tag` is newer than `current`, as semver orders them. A
/// pre-release tag, or a tag or version that isn't `x.y.z`, is never newer.
pub fn is_newer(tag: &str, current: &str) -> bool {
    let (Some(latest), Some(current)) = (version(tag), version(current)) else {
        return false;
    };
    !latest.pre && (latest.core > current.core || (latest.core == current.core && current.pre))
}

/// A URL of the project's own on GitHub: only those are opened.
fn ours(url: &str) -> bool {
    url.starts_with(PAGES) && url.is_ascii() && !url.contains(|c: char| c.is_ascii_whitespace())
}

/// GitHub's `releases/latest` reply: the release, unless it's a draft or a pre-release or
/// isn't the project's.
pub fn parse_release(json: &str) -> Option<Release> {
    let v: Value = serde_json::from_str(json).ok()?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let tag = version(v["tag_name"].as_str()?)?;
    if tag.pre {
        return None;
    }
    let url = v["html_url"].as_str().filter(|u| ours(u))?;
    let [major, minor, patch] = tag.core;
    Some(Release {
        version: format!("{major}.{minor}.{patch}"),
        url: url.to_string(),
    })
}

/// GitHub's issue search reply: the first issue's page.
pub fn parse_issue(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    let url = v["items"][0]["html_url"].as_str().filter(|u| ours(u))?;
    Some(url.to_string())
}

/// The last check, kept in the cache.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stamp {
    /// When, in seconds since 1970; 0 for never.
    pub checked: u64,
    /// The latest release GitHub gave, newer or not.
    pub latest: Option<Release>,
    /// The version the toast last announced.
    pub announced: Option<String>,
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Stamp {
    /// The stamp at `path`; a missing or broken file is no check yet.
    pub fn load(path: &Path) -> Stamp {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string(self).map_err(io::Error::other)?;
        std::fs::write(path, text)
    }

    /// Whether a day has passed since the last check (or the clock went back past it).
    pub fn due(&self, now: SystemTime) -> bool {
        let now = unix(now);
        self.checked == 0 || now < self.checked || now - self.checked >= CHECK_EVERY.as_secs()
    }

    /// A check at `now`. A failed one keeps the last answer, and still waits a day.
    pub fn record(&mut self, result: Result<Option<Release>, String>, now: SystemTime) {
        self.checked = unix(now);
        if let Ok(latest) = result {
            self.latest = latest;
        }
    }

    /// The latest release, if it's newer than `current`.
    pub fn newer(&self, current: &str) -> Option<&Release> {
        self.latest
            .as_ref()
            .filter(|r| is_newer(&r.version, current))
    }

    /// The newer release, if the toast hasn't announced it yet.
    pub fn to_announce(&self, current: &str) -> Option<Release> {
        self.newer(current)
            .filter(|r| self.announced.as_deref() != Some(r.version.as_str()))
            .cloned()
    }

    pub fn announced(&mut self, version: &str) {
        self.announced = Some(version.to_string());
    }
}

/// The stamp file in `cache`.
pub fn stamp_path(cache: &Path) -> PathBuf {
    cache.join(STAMP)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("featherplay/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| format!("http client: {e}"))
}

/// One GET to GitHub's API, read-only and unauthenticated.
async fn get(url: reqwest::Url) -> Result<String, String> {
    let resp = client()?
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("request: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    resp.text()
        .await
        .map_err(|e| format!("reading the reply: {e}"))
}

/// The latest release on GitHub.
async fn fetch_latest() -> Result<Option<Release>, String> {
    let url = reqwest::Url::parse(LATEST).map_err(|e| e.to_string())?;
    Ok(parse_release(&get(url).await?))
}

/// The page of the open issue the daily play check keeps, if there is one.
async fn find_issue() -> Result<Option<String>, String> {
    let query = format!("repo:candygoogy/featherplay is:issue is:open in:title \"{ISSUE_TITLE}\"");
    let url = reqwest::Url::parse_with_params(SEARCH, [("q", query.as_str()), ("per_page", "1")])
        .map_err(|e| e.to_string())?;
    Ok(parse_issue(&get(url).await?))
}

/// What a check found: the newer release, for About's banner, and the one to announce.
#[derive(Debug, Default)]
pub struct Found {
    pub newer: Option<Release>,
    pub announce: Option<Release>,
}

/// Checks GitHub if the last check is a day old, or at once with `force`; else reads the
/// last answer. Marks the release it returns to announce as announced. A failure is logged.
pub async fn check(path: PathBuf, force: bool) -> Found {
    let now = SystemTime::now();
    let mut stamp = Stamp::load(&path);
    let mut changed = false;
    if force || stamp.due(now) {
        let result = fetch_latest().await;
        if let Err(e) = &result {
            log!("update check: {e}");
        }
        stamp.record(result, now);
        changed = true;
    }
    let announce = stamp.to_announce(CURRENT);
    if let Some(r) = &announce {
        stamp.announced(&r.version);
        changed = true;
    }
    if changed && let Err(e) = stamp.save(&path) {
        log!("saving the update check: {}", e.kind());
    }
    Found {
        newer: stamp.newer(CURRENT).cloned(),
        announce,
    }
}

/// When songs keep failing after the helpers were updated: a newer release (a fresh
/// check, whatever the stamp says), else the open daily play issue. At most two requests.
pub async fn help(cache: PathBuf) -> (Option<Help>, Option<Release>) {
    let found = check(stamp_path(&cache), true).await;
    if let Some(r) = found.newer {
        let help = Help {
            label: tr!("Get it"),
            url: r.url.clone(),
        };
        return (Some(help), Some(r));
    }
    match find_issue().await {
        Ok(issue) => (
            issue.map(|url| Help {
                label: tr!("Details"),
                url,
            }),
            None,
        ),
        Err(e) => {
            log!("issue search: {e}");
            (None, None)
        }
    }
}

thread_local! {
    /// The startup check is still to run (the real app only: never in tests or snapshots).
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static CACHE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// The app's window is up: the startup check runs once the first page has loaded.
pub fn arm(cache: PathBuf) {
    CACHE.with(|c| *c.borrow_mut() = Some(cache));
    ARMED.with(|a| a.set(true));
}

/// A load arrived, or the session became known: once nothing is loading any more, the
/// first page is up, and the startup check runs (once).
pub fn page_settled(ctx: &Rc<Ctx>) {
    if !ARMED.with(Cell::get) || ctx.session_pending() || busy::pending() > 0 {
        return;
    }
    ARMED.with(|a| a.set(false));
    if !pages::settings::check_for_updates() {
        log!("update check: off in Settings");
        return;
    }
    let Some(cache) = CACHE.with(|c| c.borrow().clone()) else {
        return;
    };
    ctx.load(
        move |_| check(stamp_path(&cache), false),
        |ctx, _ui, found| {
            pages::about::show_update(ctx, found.newer.as_ref());
            if let Some(r) = found.announce {
                announce(ctx, &r);
            }
        },
    );
}

/// The toast for a newer release.
pub fn announce(ctx: &Ctx, release: &Release) {
    let url = release.url.clone();
    toast::show_with(
        ctx,
        &tr!("featherplay {} is out", release.version),
        tr!("Get it"),
        move |c| open(c, &url),
    );
}

/// Opens one of the project's pages in the browser, off the UI thread.
pub fn open(ctx: &Ctx, url: &str) {
    if !ours(url) {
        return;
    }
    let url = url.to_string();
    ctx.catalog.runtime().spawn_blocking(move || {
        if let Err(e) = crate::controller::open_url(&url) {
            log!("opening the release page: {e}");
        }
    });
}
