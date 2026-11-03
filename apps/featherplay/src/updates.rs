//! Whether a newer featherplay is out, from GitHub's latest release, and the open issue the
//! daily play check keeps when YouTube breaks playback. No account, no token.
//!
//! At most once a day: at startup, once the first page's loads have all arrived, if
//! Settings' "Check for updates" is on. The answer is kept in `{cache}/update-check.json`
//! with the time of the check, so the banner in About survives a restart without a request.
//! A failed request is logged once and counts as that day's check: no retry, no timer.
//! When songs keep failing to play after the helpers were updated, the controller asks
//! again at once (`help`), once a session.
//!
//! A newer release shows a toast once per version and a banner at the top of About. The
//! installed copy, with "Install updates automatically" on, first downloads and checks the
//! release's setup (`install`), once per version, and then offers "Restart to update" in
//! the toast and the banner, at every start until it's clicked.

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

pub mod install;

use install::{Origin, Ready};

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

/// A published release: its version ("0.2.0"), its page, and its files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub url: String,
    /// The files attached to it that are downloaded from `install::DOWNLOADS`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<Asset>,
}

/// A file attached to a release: its name and its download URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
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
    let assets = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let name = a["name"].as_str()?;
            let url = a["browser_download_url"].as_str()?;
            let downloadable = url.starts_with(install::DOWNLOADS) && ours(url);
            downloadable.then(|| Asset {
                name: name.to_string(),
                url: url.to_string(),
            })
        })
        .collect();
    Some(Release {
        version: format!("{major}.{minor}.{patch}"),
        url: url.to_string(),
        assets,
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
    /// The version whose setup was last downloaded, whatever came of it: each version is
    /// downloaded once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched: Option<String>,
    /// The setup downloaded and checked, waiting for "Restart to update".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready: Option<Ready>,
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

    /// The downloaded setup, if it installs the newer release (not an older one, nor the
    /// version running now).
    pub fn pending(&self, current: &str) -> Option<&Ready> {
        let newer = self.newer(current)?;
        self.ready.as_ref().filter(|r| r.version == newer.version)
    }

    /// The newer release, if its setup hasn't been downloaded (or tried) yet.
    pub fn to_fetch(&self, current: &str) -> Option<&Release> {
        self.newer(current)
            .filter(|r| self.fetched.as_deref() != Some(r.version.as_str()))
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

/// What a check found: the newer release, for About's banner, the one to announce, and its
/// downloaded setup, to offer "Restart to update" with instead of announcing it.
#[derive(Debug, Default)]
pub struct Found {
    pub newer: Option<Release>,
    pub announce: Option<Release>,
    pub ready: Option<Ready>,
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
        ready: None,
    }
}

/// The startup check, then with `install` (the setting is on) for the installed copy: the
/// setup already downloaded for the newer release, checked again, or else a download of it
/// if this version hasn't been tried. A failed or mismatched download is logged and the
/// release is only announced. Otherwise old setups are deleted.
pub async fn startup(cache: PathBuf, install: bool) -> Found {
    let path = stamp_path(&cache);
    let mut found = check(path.clone(), false).await;
    if !install || !install::installed() {
        return found;
    }
    let mut stamp = Stamp::load(&path);
    if let Some(ready) = stamp.pending(CURRENT).cloned() {
        if ready.verified(&cache) {
            install::clean(&cache, Some(&ready));
            found.announce = None;
            found.ready = Some(ready);
            return found;
        }
        log!(
            "update: the downloaded setup for {} is gone or changed",
            ready.version
        );
    }
    let stale = stamp.ready.take().is_some();
    install::clean(&cache, None);
    let plan = stamp.to_fetch(CURRENT).map(|r| {
        let plan = install::plan(r, std::env::consts::ARCH);
        if plan.is_none() {
            log!(
                "update: {} has no setup for {}; announcing it only",
                r.version,
                std::env::consts::ARCH
            );
        }
        (r.version.clone(), plan)
    });
    if let Some((version, plan)) = plan {
        stamp.fetched = Some(version);
        if let Some(plan) = plan {
            match install::fetch(&cache, &plan, &Origin::github()).await {
                Ok(ready) => {
                    log!("update: {} downloaded and checked", ready.version);
                    stamp.ready = Some(ready.clone());
                    found.announce = None;
                    found.ready = Some(ready);
                }
                Err(e) => log!("update download: {e}"),
            }
        }
    } else if !stale {
        return found;
    }
    if let Err(e) = stamp.save(&path) {
        log!("saving the update check: {}", e.kind());
    }
    found
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
    /// The setup "Restart to update" starts, once one is downloaded and checked.
    static READY: RefCell<Option<Ready>> = const { RefCell::new(None) };
    /// "Restart to update" was clicked: the setup is being started.
    static RESTARTING: Cell<bool> = const { Cell::new(false) };
}

/// The version "Restart to update" installs, if a setup is ready.
pub fn ready_version() -> Option<String> {
    READY.with(|r| r.borrow().as_ref().map(|r| r.version.clone()))
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
    let install = pages::settings::install_updates();
    ctx.load(
        move |_| startup(cache, install),
        |ctx, _ui, found| {
            READY.with(|r| *r.borrow_mut() = found.ready.clone());
            pages::about::show_update(ctx, found.newer.as_ref());
            if let Some(ready) = &found.ready {
                offer(ctx, ready);
            } else if let Some(r) = found.announce {
                announce(ctx, &r);
            }
        },
    );
}

/// The toast for a downloaded update: "Restart to update".
pub fn offer(ctx: &Ctx, ready: &Ready) {
    toast::show_with(
        ctx,
        &tr!("featherplay {} is ready to install", ready.version),
        tr!("Restart to update"),
        restart,
    );
}

/// "Restart to update": starts the downloaded setup (checked once more), then quits; the
/// setup starts the new version. If it can't be started, the release is offered as a page
/// to open instead.
pub fn restart(ctx: &Rc<Ctx>) {
    let (Some(ready), Some(cache)) = (
        READY.with(|r| r.borrow().clone()),
        CACHE.with(|c| c.borrow().clone()),
    ) else {
        return;
    };
    if RESTARTING.with(|r| r.replace(true)) {
        return;
    }
    log!("update: starting the setup for {}", ready.version);
    ctx.load(
        move |_| async move {
            tokio::task::spawn_blocking(move || install::launch(&cache, &ready))
                .await
                .map_err(|e| format!("setup task: {e}"))?
        },
        |ctx, _ui, started| match started {
            Ok(()) => {
                log!("update: the setup is running; quitting");
                ctx.quit();
            }
            Err(e) => {
                log!("update: {e}");
                RESTARTING.with(|r| r.set(false));
                READY.with(|r| r.borrow_mut().take());
                pages::about::refresh_update(ctx);
                ctx.toast(tr!("The update couldn't start"));
            }
        },
    );
}

/// `--snapshot about-update-ready`: a setup for 0.2.0 waiting, with its toast.
pub fn ready_for_snapshot(ctx: &Ctx) {
    let ready = Ready {
        version: "0.2.0".into(),
        file: "featherplay-setup-0.2.0.exe".into(),
        sha256: String::new(),
    };
    READY.with(|r| *r.borrow_mut() = Some(ready.clone()));
    offer(ctx, &ready);
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
