//! Whether a newer featherplay is out, from GitHub's latest release, and the open issue the
//! maintainer opens when YouTube breaks playback. No account, no token.
//!
//! One runner (`run`) serves every trigger, one run at a time (`Running`):
//! - startup, once the first page's loads have all arrived;
//! - in a long session, when playback goes from paused to playing, the window comes back
//!   from the tray or the taskbar, or "Check for updates" is turned on, if a check is due
//!   (`maybe_run`: one comparison with `NEXT_DUE`, no timer);
//! - About's "Check for updates" button, which works even with the setting off: a click is
//!   consent for one request;
//! - songs that keep failing to play after the helpers were updated (`help`).
//!
//! GitHub is asked at most once a day, or an hour after a failed check, and never within a
//! minute of the last request or before a rate limit's reset (`may_request`). The answer is
//! kept in `{cache}/update-check.json`, so About survives a restart without a request.
//!
//! A newer release shows a toast once per version and a banner at the top of About. The
//! installed copy, with "Install updates automatically" on, first downloads and checks the
//! release's setup (`install`); a failed download is tried again an hour later, three times
//! at most, a setup that doesn't match its checksum never. It then offers "Restart to
//! update" in the toast and the banner, at every start until it's clicked. If the update
//! doesn't install (the setup fails silently), the next start says so. A copy Scoop
//! installed is told the Scoop command instead; Scoop updates it.
//!
//! The triggers, the toasts and "Restart to update" are the app's (its `updates` module,
//! which re-exports this one); About's banner and row are redrawn through the callback it
//! gives (`on_change`).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use featherplay_core::tr;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod install;

use install::{FetchError, Keys, Origin, Ready};

/// The project's pages on GitHub; every URL opened from a reply must start with it.
const PAGES: &str = "https://github.com/candygoogy/featherplay/";
/// The website, opened instead of the release page when a release couldn't be verified:
/// the page users know to download from, deployed apart from GitHub's releases. Its
/// download links still lead to GitHub's files, for the version it was last deployed with.
pub const SITE: &str = "https://featherplay.candygoogy.dev/";
const LATEST: &str = "https://api.github.com/repos/candygoogy/featherplay/releases/latest";
const SEARCH: &str = "https://api.github.com/search/issues";
/// The title of the issue the maintainer opens by hand once playback is confirmed broken for
/// everyone (RELEASING.md, "When playback breaks"). No workflow opens it: the daily yt-dlp
/// watch keeps its own issue under another title, which never reaches users.
const ISSUE_TITLE: &str = "Playback broken";
/// Who may open that issue: the owner and the people with write access to the repository.
const MAINTAINERS: &[&str] = &["OWNER", "MEMBER", "COLLABORATOR"];
/// Search results read for it: anyone can open an issue with those words in its title.
const ISSUE_RESULTS: &str = "10";
const TIMEOUT: Duration = Duration::from_secs(10);
const CHECK_EVERY: u64 = 24 * 3600;
/// A failed check is due again after this.
const CHECK_RETRY: u64 = 3600;
/// No request within this of the last one, whatever asks.
pub const COOLDOWN: u64 = 60;
/// A rate limit's reset further away than this is taken as this.
const LIMIT_AT_MOST: u64 = 3600;
/// A failed download is tried again after this, at most `FETCH_ATTEMPTS` times in all.
const FETCH_RETRY: u64 = 3600;
const FETCH_ATTEMPTS: u32 = 3;
/// The lines of the setup's log copied to featherplay's own after an update didn't install.
const INSTALL_LOG_LINES: usize = 20;
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
    /// The toast's button: "Get it", "Restart to update" or "Details".
    pub label: &'static str,
    pub url: String,
    /// The button starts the downloaded setup ("Restart to update") instead of opening `url`.
    pub restart: bool,
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
pub fn ours(url: &str) -> bool {
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

/// GitHub's issue search reply: the page of the first issue titled exactly `ISSUE_TITLE`
/// that a maintainer opened. The search matches the words anywhere in any title.
pub fn parse_issue(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    v["items"].as_array()?.iter().find_map(|item| {
        let by_maintainer = item["author_association"]
            .as_str()
            .is_some_and(|a| MAINTAINERS.contains(&a));
        if !by_maintainer || item["title"].as_str() != Some(ISSUE_TITLE) {
            return None;
        }
        item["html_url"]
            .as_str()
            .filter(|u| ours(u))
            .map(str::to_string)
    })
}

/// Why a check got no answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckError {
    /// No connection, or no reply within the timeout.
    Offline,
    /// GitHub's rate limit: no request before `until` (seconds since 1970).
    Limited { until: u64 },
    /// Any other HTTP status.
    Http(u16),
    /// A reply that isn't JSON.
    Unreadable,
}

/// What a failed reply's status and headers mean: GitHub's rate limit (403 with no requests
/// left, or with `retry-after`; any 429) until its reset, or a plain HTTP error. The reset is
/// taken as an hour at most, whatever the headers say.
pub fn classify(status: u16, headers: &HeaderMap, now: SystemTime) -> CheckError {
    let number = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
    };
    let now = unix(now);
    let retry_after = number("retry-after").map(|secs| now.saturating_add(secs));
    let exhausted = number("x-ratelimit-remaining") == Some(0);
    if status == 429 || (status == 403 && (exhausted || retry_after.is_some())) {
        let until = retry_after
            .or_else(|| number("x-ratelimit-reset"))
            .unwrap_or(now + COOLDOWN);
        return CheckError::Limited {
            until: until.clamp(now, now + LIMIT_AT_MOST),
        };
    }
    CheckError::Http(status)
}

/// Whether GitHub may be asked now: not within `COOLDOWN` of the last request (`last`,
/// seconds since 1970; 0 for none; a clock set back allows it), nor before a rate limit's
/// end.
pub fn may_request(last: u64, now: SystemTime, limited_until: Option<u64>) -> bool {
    let now = unix(now);
    let cooled = last == 0 || now < last || now - last >= COOLDOWN;
    cooled && limited_until.is_none_or(|until| now >= until)
}

/// The download of a newer release's setup: which version, how many times it was tried and
/// when last, and whether it's refused for good (a checksum that didn't match, no setup for
/// this PC), because its checksums weren't signed for it with featherplay's keys or not.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fetch {
    pub version: String,
    pub attempts: u32,
    pub last: u64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub refused: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unverified: bool,
}

/// The last check, kept in the cache.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stamp {
    /// When GitHub was last asked, answered or not, in seconds since 1970; 0 for never.
    pub checked: u64,
    /// When it last answered ("Checked 2 hours ago"); 0 for never.
    #[serde(skip_serializing_if = "is_zero")]
    pub answered: u64,
    /// Why the last check got no answer; none if it got one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CheckError>,
    /// The latest release GitHub gave, newer or not.
    pub latest: Option<Release>,
    /// The version the toast last announced.
    pub announced: Option<String>,
    /// The newer release's setup download, whatever came of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetch: Option<Fetch>,
    /// The setup downloaded and checked, waiting for "Restart to update".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready: Option<Ready>,
    /// The version whose setup "Restart to update" last started. Running an older version
    /// than it afterwards means that update didn't install.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launched: Option<String>,
    /// Before 0.2.0: the version whose setup was downloaded, once, whatever came of it.
    #[serde(skip_serializing, rename = "fetched")]
    old_fetched: Option<String>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Seconds since 1970, now.
pub fn now_unix() -> u64 {
    unix(SystemTime::now())
}

impl Stamp {
    /// The stamp at `path`; a missing or broken file is no check yet. A stamp from before
    /// 0.2.0 reads as if its download was tried three times, and its last check answered.
    pub fn load(path: &Path) -> Stamp {
        let mut stamp: Stamp = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        if let Some(version) = stamp.old_fetched.take()
            && stamp.fetch.is_none()
        {
            stamp.fetch = Some(Fetch {
                version,
                attempts: FETCH_ATTEMPTS,
                last: stamp.checked,
                ..Fetch::default()
            });
        }
        if stamp.answered == 0 && stamp.error.is_none() {
            stamp.answered = stamp.checked;
        }
        stamp
    }

    /// Written whole or not at all: to a temporary file, then renamed over the stamp.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string(self).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }

    /// When the next check is due: a day after the last, an hour after a failed one, and
    /// not before a rate limit's end. 0 for at once.
    pub fn check_due_at(&self) -> u64 {
        if self.checked == 0 {
            return 0;
        }
        let wait = if self.error.is_some() {
            CHECK_RETRY
        } else {
            CHECK_EVERY
        };
        (self.checked + wait).max(self.limited_until().unwrap_or(0))
    }

    /// Whether a check is due (`check_due_at`), or the clock went back past the last one.
    pub fn due(&self, now: SystemTime) -> bool {
        let now = unix(now);
        now < self.checked || now >= self.check_due_at()
    }

    /// The end of GitHub's rate limit, if the last check met it.
    pub fn limited_until(&self) -> Option<u64> {
        match self.error {
            Some(CheckError::Limited { until }) => Some(until),
            _ => None,
        }
    }

    /// A check at `now`. A failed one keeps the last answer.
    pub fn record(&mut self, result: Result<Option<Release>, CheckError>, now: SystemTime) {
        self.checked = unix(now);
        match result {
            Ok(latest) => {
                self.latest = latest;
                self.answered = self.checked;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
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
    /// version running now), and its checksums were signed (`Ready::signed`).
    pub fn pending(&self, current: &str) -> Option<&Ready> {
        let newer = self.newer(current)?;
        self.ready
            .as_ref()
            .filter(|r| r.version == newer.version && r.signed)
    }

    /// The newer release, if its setup is to be downloaded now: never tried; or tried and
    /// failed fewer than `FETCH_ATTEMPTS` times, the last over an hour ago; or, asked for by
    /// a click (`manual`), failed any number of times. Never once it's refused, except one
    /// refused as unverified: its signature is looked for again a day later, in case it was
    /// uploaded late (and checked as ever).
    pub fn to_fetch(&self, current: &str, now: SystemTime, manual: bool) -> Option<&Release> {
        let newer = self.newer(current)?;
        let Some(f) = self.fetch.as_ref().filter(|f| f.version == newer.version) else {
            return Some(newer);
        };
        let now = unix(now);
        let waited = |secs| now < f.last || now - f.last >= secs;
        let again = if f.unverified {
            waited(CHECK_EVERY)
        } else {
            !f.refused && (manual || (f.attempts < FETCH_ATTEMPTS && waited(FETCH_RETRY)))
        };
        again.then_some(newer)
    }

    /// A download of `version`'s setup at `now`, counted whatever came of it; `refused`: it
    /// can never work.
    pub fn fetch_tried(&mut self, version: &str, now: SystemTime, refused: bool) {
        let attempts = self
            .fetch
            .as_ref()
            .filter(|f| f.version == version)
            .map_or(0, |f| f.attempts);
        self.fetch = Some(Fetch {
            version: version.to_string(),
            attempts: attempts + 1,
            last: unix(now),
            refused,
            unverified: false,
        });
    }

    /// A download of `version`'s setup at `now` that its checksums' signature refused. It's
    /// looked at again only a day later (`to_fetch`), and About says why.
    pub fn fetch_unverified(&mut self, version: &str, now: SystemTime) {
        self.fetch_tried(version, now, true);
        if let Some(f) = &mut self.fetch {
            f.unverified = true;
        }
    }

    /// When a failed download of the newer release is to be tried again, if it is.
    pub fn fetch_due_at(&self, current: &str) -> Option<u64> {
        let newer = self.newer(current)?;
        let f = self.fetch.as_ref().filter(|f| f.version == newer.version)?;
        let again = !f.refused && f.attempts < FETCH_ATTEMPTS && self.pending(current).is_none();
        again.then_some(f.last + FETCH_RETRY)
    }

    /// Whether the newer release's setup was refused as unverified (`fetch_unverified`).
    pub fn unverified(&self, current: &str) -> bool {
        let Some(newer) = self.newer(current) else {
            return false;
        };
        self.fetch
            .as_ref()
            .is_some_and(|f| f.version == newer.version && f.unverified)
    }

    /// The version "Restart to update" started whose update didn't install: the setup ran,
    /// and `current` is still older than it.
    pub fn failed_install(&self, current: &str) -> Option<&str> {
        self.launched.as_deref().filter(|v| is_newer(v, current))
    }
}

/// The stamp file in `cache`.
pub fn stamp_path(cache: &Path) -> PathBuf {
    cache.join(STAMP)
}

/// Where a run asks: GitHub, or in tests a server on 127.0.0.1; the keys a release's
/// checksums must be signed with; and whether this is the installed copy, which downloads
/// setups.
pub struct Remote {
    latest: String,
    origin: Origin,
    keys: Keys,
    installed: bool,
    /// Tests: the release files' URLs (`install::DOWNLOADS…`) are fetched from here instead.
    #[cfg(test)]
    files: Option<String>,
}

impl Remote {
    /// GitHub, with featherplay's own keys, for this copy (`installed`: it's the installed
    /// copy).
    pub fn github(installed: bool) -> Remote {
        Remote {
            latest: LATEST.to_string(),
            origin: Origin::github(),
            keys: Keys::release(),
            installed,
            #[cfg(test)]
            files: None,
        }
    }

    #[cfg(test)]
    fn local(base: &str, installed: bool) -> Remote {
        Remote {
            latest: format!("{base}/latest"),
            origin: Origin::local(),
            keys: install::tests::test_keys(),
            installed,
            files: Some(base.to_string()),
        }
    }

    /// The download plan for this remote: GitHub's URLs as they are, or the test server's.
    fn localise(&self, plan: install::Plan) -> install::Plan {
        #[cfg(test)]
        if let Some(base) = &self.files {
            let mut plan = plan;
            for asset in [&mut plan.setup, &mut plan.sums]
                .into_iter()
                .chain(plan.sig.as_mut())
            {
                asset.url = format!("{base}/{}", asset.name);
            }
            return plan;
        }
        plan
    }
}

/// The latest release on GitHub. A request that fails or times out is `Offline`.
async fn fetch_latest(remote: &Remote) -> Result<Option<Release>, CheckError> {
    let http = remote.origin.api_client(TIMEOUT).map_err(|e| {
        log!("update check: {e}");
        CheckError::Offline
    })?;
    let resp = http
        .get(&remote.latest)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| {
            log!("update check: request: {e}");
            CheckError::Offline
        })?;
    let status = resp.status();
    if !status.is_success() {
        let left = resp
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?")
            .to_string();
        log!(
            "update check: HTTP {} ({left} requests left)",
            status.as_u16()
        );
        return Err(classify(status.as_u16(), resp.headers(), SystemTime::now()));
    }
    let body = resp.text().await.map_err(|e| {
        log!("update check: reading the reply: {e}");
        CheckError::Offline
    })?;
    if serde_json::from_str::<Value>(&body).is_err() {
        log!("update check: the reply isn't JSON");
        return Err(CheckError::Unreadable);
    }
    Ok(parse_release(&body))
}

/// The page of the open "Playback broken" issue, if the maintainer opened one.
async fn find_issue() -> Result<Option<String>, String> {
    let query = format!("repo:candygoogy/featherplay is:issue is:open in:title \"{ISSUE_TITLE}\"");
    let params = [("q", query.as_str()), ("per_page", ISSUE_RESULTS)];
    let url = reqwest::Url::parse_with_params(SEARCH, params).map_err(|e| e.to_string())?;
    let resp = Origin::github()
        .api_client(TIMEOUT)?
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("request: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("reading the reply: {e}"))?;
    Ok(parse_issue(&body))
}

/// What asked for a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// The first page is up.
    Startup,
    /// Playback resumed, the window came back, or the setting was turned on.
    Resume,
    /// About's "Check for updates" (or "Try again" after a failed check).
    Manual,
    /// The banner's "Try again" after a failed download: no check, the download again.
    Retry,
    /// Songs keep failing to play.
    Help,
}

impl Trigger {
    /// A click asked: the setting is no bar, and a failed download is tried again whatever
    /// its count.
    fn clicked(self) -> bool {
        matches!(self, Trigger::Manual | Trigger::Retry)
    }
}

/// How this copy is updated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// A portable exe or a dev build: told about a newer release, with its page.
    #[default]
    Portable,
    /// The installer's copy (winget's too): downloads the setup, restarts into it.
    Installed,
    /// Scoop's copy: told the Scoop command.
    Scoop,
}

impl Kind {
    /// This process's: the installer's uninstall key, or Scoop's folder.
    pub fn detect() -> Kind {
        if install::installed() {
            Kind::Installed
        } else if install::scoop() {
            Kind::Scoop
        } else {
            Kind::Portable
        }
    }
}

/// What About shows of the updater, shared by the runs (on the runtime, or the controller's
/// thread for `help`) and the UI thread.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// The stamp as the last run left it.
    pub stamp: Stamp,
    /// A run is going (`Running`).
    pub running: bool,
    /// It's asking GitHub (S1, "Checking…").
    pub checking: bool,
    /// It's downloading the setup (B2).
    pub downloading: bool,
    /// "Restart to update" was clicked: the setup is being started (B8).
    pub restarting: bool,
    /// The setup couldn't be started: it's offered as a page to open instead (B5).
    pub launch_failed: bool,
    pub kind: Kind,
}

/// The banner at the top of About (the board's B1-B8), or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Banner {
    None,
    /// B1: "Get it" opens the release page.
    Get,
    /// B2: the setup is downloading.
    Downloading,
    /// B3: "Restart to update".
    Ready,
    /// B4: the download didn't finish: "Try again".
    DownloadFailed,
    /// B5: it can't be installed from here (checksum, no setup for this PC, a setup that
    /// wouldn't start): "Get it".
    CannotInstall,
    /// B6: the last update didn't install: "Try again" starts the setup once more.
    InstallFailed,
    /// B7: Scoop's copy: the command.
    Scoop,
    /// B8: "Restart to update" clicked, before featherplay quits.
    Starting,
    /// B9: the release's checksums weren't signed for it with featherplay's keys: nothing
    /// downloaded; "Get it" opens the website, not the release page.
    Unverified,
}

impl Status {
    /// The setup "Restart to update" starts: downloaded and checked for the newer release,
    /// on the installed copy with "Install updates automatically" on.
    pub fn ready(&self, install_on: bool) -> Option<&Ready> {
        let usable = self.kind == Kind::Installed && install_on && !self.launch_failed;
        self.stamp.pending(CURRENT).filter(|_| usable)
    }

    pub fn banner(&self, install_on: bool) -> Banner {
        let Some(newer) = self.stamp.newer(CURRENT) else {
            return Banner::None;
        };
        if self.restarting {
            return Banner::Starting;
        }
        let ready = self.ready(install_on).is_some();
        if ready && self.stamp.failed_install(CURRENT) == Some(newer.version.as_str()) {
            return Banner::InstallFailed;
        }
        if self.kind == Kind::Scoop {
            return Banner::Scoop;
        }
        if ready {
            return Banner::Ready;
        }
        if self.launch_failed {
            return Banner::CannotInstall;
        }
        if self.kind != Kind::Installed || !install_on {
            return Banner::Get;
        }
        if self.downloading {
            return Banner::Downloading;
        }
        if self.stamp.unverified(CURRENT) {
            return Banner::Unverified;
        }
        match self.stamp.fetch.as_ref() {
            Some(f) if f.version == newer.version && f.refused => Banner::CannotInstall,
            Some(f) if f.version == newer.version => Banner::DownloadFailed,
            _ => Banner::Get,
        }
    }
}

static STATUS: LazyLock<Mutex<Status>> = LazyLock::new(Mutex::default);

fn lock() -> MutexGuard<'static, Status> {
    STATUS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The updater's state now, for About.
pub fn status() -> Status {
    lock().clone()
}

/// Changes the updater's state as `f` does: the app's triggers (which copy this is, a
/// "Restart to update" under way) and About's snapshots.
pub fn edit_status<R>(f: impl FnOnce(&mut Status) -> R) -> R {
    f(&mut lock())
}

/// One run at a time: held from a run's start to its end. Dropping it ends the run.
#[derive(Debug)]
pub struct Running(());

impl Running {
    /// The run's token, unless one is going; `checking` shows "Checking…" until the request
    /// is answered (or found not needed).
    pub fn begin(checking: bool) -> Option<Running> {
        let mut s = lock();
        if s.running {
            return None;
        }
        s.running = true;
        s.checking = checking;
        Some(Running(()))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let mut s = lock();
        s.running = false;
        s.checking = false;
        s.downloading = false;
    }
}

/// What a run found: the newer release, for About's banner; the one to announce; its
/// downloaded setup, to offer "Restart to update" with instead; the update that didn't
/// install; and when the next run is due.
#[derive(Debug, Default)]
pub struct Found {
    pub newer: Option<Release>,
    pub announce: Option<Release>,
    pub ready: Option<Ready>,
    /// `ready` was downloaded by this run (not at an earlier one).
    pub downloaded: bool,
    /// This run refused the newer release's setup: its checksums weren't signed for it.
    pub unverified: bool,
    pub failed_install: Option<String>,
    /// Seconds since 1970; 0 for at once.
    pub next_due: u64,
}

/// The app's redraw of About's banner and row (`on_change`).
static CHANGED: OnceLock<fn()> = OnceLock::new();

/// Gives the runs the app's redraw of About's banner and row, for when they change them off
/// the UI thread. The first call sets it; until then, nothing is redrawn.
pub fn on_change(redraw: fn()) {
    let _ = CHANGED.set(redraw);
}

/// About's banner and row may have changed off the UI thread: the app redraws them there.
fn changed_elsewhere() {
    if let Some(redraw) = CHANGED.get() {
        redraw();
    }
}

/// The run behind every trigger. With `check_on` (the setting; ignored when a click asked)
/// it asks GitHub if a check is due and allowed (`may_request`); then, on the installed copy
/// with `install_on`, it checks a setup downloaded earlier, or downloads the newer
/// release's if that's due (`Stamp::to_fetch`). Setups no longer pending are deleted. A
/// release is marked announced; it's returned to announce only at startup or on resume, as
/// a click is answered in About and `help` makes its own toast.
pub async fn run(
    running: Running,
    cache: PathBuf,
    trigger: Trigger,
    check_on: bool,
    install_on: bool,
    remote: Remote,
) -> Found {
    let path = stamp_path(&cache);
    let now = SystemTime::now();
    let mut stamp = Stamp::load(&path);
    let mut changed = false;
    // The last "Restart to update": installed (this is that version or newer) or not.
    if let Some(v) = stamp.launched.clone() {
        if !is_newer(&v, CURRENT) {
            stamp.launched = None;
            changed = true;
        } else if trigger == Trigger::Startup {
            log!("update: the update to {v} didn't install; the setup's log ends:");
            for line in install::install_log_tail(&cache, INSTALL_LOG_LINES) {
                log!("  {line}");
            }
        }
    }
    let allowed = may_request(stamp.checked, now, stamp.limited_until());
    let ask = match trigger {
        Trigger::Startup | Trigger::Resume => check_on && stamp.due(now) && allowed,
        Trigger::Manual | Trigger::Help => allowed,
        Trigger::Retry => false,
    };
    if ask {
        // S1 shows only while GitHub is really being asked.
        if !std::mem::replace(&mut lock().checking, true) {
            changed_elsewhere();
        }
        let result = fetch_latest(&remote).await;
        stamp.record(result, now);
        changed = true;
    }
    {
        let mut s = lock();
        s.checking = false;
        s.stamp = stamp.clone();
    }
    let mut announce = None;
    if let Some(r) = stamp.to_announce(CURRENT) {
        stamp.announced(&r.version);
        changed = true;
        if matches!(trigger, Trigger::Startup | Trigger::Resume) {
            announce = Some(r);
        }
    }
    let mut ready = None;
    let mut downloaded = false;
    let mut unverified = false;
    if install_on && remote.installed && (check_on || trigger.clicked()) {
        if let Some(r) = stamp.pending(CURRENT).cloned() {
            if r.verified(&cache) {
                ready = Some(r);
            } else {
                log!(
                    "update: the downloaded setup for {} is gone or changed",
                    r.version
                );
                stamp.ready = None;
                changed = true;
            }
        }
        let due = stamp.to_fetch(CURRENT, now, trigger.clicked()).cloned();
        if let (None, Some(release)) = (&ready, due) {
            changed = true;
            match install::plan(&release, std::env::consts::ARCH) {
                None => {
                    log!(
                        "update: {} has no setup for {}; announcing it only",
                        release.version,
                        std::env::consts::ARCH
                    );
                    stamp.fetch_tried(&release.version, now, true);
                }
                Some(plan) => {
                    lock().downloading = true;
                    changed_elsewhere();
                    let plan = remote.localise(plan);
                    let began = std::time::Instant::now();
                    match install::fetch(&cache, &plan, &remote.origin, &remote.keys).await {
                        Ok(r) => {
                            log!(
                                "update: {} downloaded and checked in {:.1} s",
                                r.version,
                                began.elapsed().as_secs_f32()
                            );
                            stamp.fetch_tried(&release.version, now, false);
                            stamp.ready = Some(r.clone());
                            ready = Some(r);
                            downloaded = true;
                        }
                        Err(FetchError::Unverified(e)) => {
                            log!(
                                "update: refusing {}, it couldn't be verified: {e}",
                                release.version
                            );
                            stamp.fetch_unverified(&release.version, now);
                            unverified = true;
                        }
                        Err(e) => {
                            log!("update download: {e}");
                            let refused = matches!(e, FetchError::Refused(_));
                            stamp.fetch_tried(&release.version, now, refused);
                        }
                    }
                }
            }
        }
    }
    if ready.is_some() {
        announce = None;
    }
    // Nothing waits to install: the folder keeps no setup, whatever the switches say.
    let pending = stamp.pending(CURRENT).cloned();
    if pending.is_none() && stamp.ready.take().is_some() {
        changed = true;
    }
    install::clean(&cache, pending.as_ref());
    if changed && let Err(e) = stamp.save(&path) {
        log!("saving the update check: {}", e.kind());
    }
    let failed_install = stamp
        .failed_install(CURRENT)
        .filter(|v| ready.as_ref().is_some_and(|r| r.version == *v))
        .map(str::to_string);
    let mut next_due = stamp.check_due_at();
    if install_on
        && remote.installed
        && let Some(at) = stamp.fetch_due_at(CURRENT)
    {
        next_due = next_due.min(at);
    }
    let found = Found {
        newer: stamp.newer(CURRENT).cloned(),
        announce,
        ready,
        downloaded,
        unverified,
        failed_install,
        next_due,
    };
    {
        let mut s = lock();
        s.stamp = stamp;
        // A setup downloaded since one wouldn't start is offered again.
        s.launch_failed &= !downloaded;
    }
    drop(running);
    found
}

/// When songs keep failing after the helpers were updated: a newer release (a fresh check,
/// unless one was made within a minute; on the installed copy, its setup downloaded as at
/// startup), else the open "Playback broken" issue. If a run is going, it waits for that
/// run's answer, which then needs no request of its own (`COOLDOWN`).
pub async fn help(cache: PathBuf, install_on: bool) -> (Option<Help>, Option<Release>) {
    let running = loop {
        if let Some(running) = Running::begin(false) {
            break running;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    let remote = Remote::github(install::installed());
    let found = run(running, cache, Trigger::Help, true, install_on, remote).await;
    if let Some(r) = found.newer {
        // A setup downloaded and checked: the banner's "Restart to update". One that
        // couldn't be verified: the website, not the release page.
        let restart = found.ready.is_some();
        let unverified = status().stamp.unverified(CURRENT);
        let help = Help {
            label: if restart {
                tr!("Restart to update")
            } else {
                tr!("Get it")
            },
            url: if unverified {
                SITE.into()
            } else {
                r.url.clone()
            },
            restart,
        };
        return (Some(help), Some(r));
    }
    match find_issue().await {
        Ok(issue) => (
            issue.map(|url| Help {
                label: tr!("Details"),
                url,
                restart: false,
            }),
            None,
        ),
        Err(e) => {
            log!("issue search: {e}");
            (None, None)
        }
    }
}

/// A toast a run has for the user, held while the window is in the tray or minimised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// "featherplay 0.2.0 is out", with "Get it" (or, for Scoop's copy, "View": About).
    Out(Release),
    /// "featherplay 0.2.0 is ready to install", with "Restart to update".
    Ready(String),
    /// "The update to 0.2.0 didn't install", with "Try again".
    InstallFailed(String),
    /// "The update to 0.2.0 couldn't be verified", with "View": About says what to do.
    Unverified(String),
}

impl Notice {
    /// What a run has to say: the banner's state, in a toast. A click is answered in About
    /// and the "can't play" toast makes its own, so only startup and resume runs say
    /// anything. A setup waiting, or an update that didn't install, is said at every start,
    /// but in a long session only when the setup was just downloaded.
    pub fn of(found: &Found, trigger: Trigger) -> Option<Notice> {
        let fresh = match trigger {
            Trigger::Startup => true,
            Trigger::Resume => found.downloaded,
            Trigger::Manual | Trigger::Retry | Trigger::Help => return None,
        };
        if let Some(v) = found.failed_install.as_ref().filter(|_| fresh) {
            return Some(Notice::InstallFailed(v.clone()));
        }
        if let Some(r) = &found.ready {
            return fresh.then(|| Notice::Ready(r.version.clone()));
        }
        let announce = found.announce.clone();
        if found.unverified {
            return announce.map(|r| Notice::Unverified(r.version));
        }
        announce.map(Notice::Out)
    }
}

#[cfg(test)]
mod tests {
    use super::install::tests::{BY_MAIN, SETUP, SIGNED_SUMS, serve, test_cache};
    use super::*;

    /// `releases/latest` for 9.0.0, its files on the project's releases, as GitHub gives;
    /// `signed`: with `SHA256SUMS.txt.minisig`.
    fn reply(signed: bool) -> Vec<u8> {
        let file = |name: &str| {
            format!(
                r#"{{"name":"{name}","browser_download_url":"{}v9.0.0/{name}"}}"#,
                install::DOWNLOADS
            )
        };
        // The setup for the machine the tests run on (x64 or ARM64, as the updater picks it).
        let setup = install::setup_name("9.0.0", std::env::consts::ARCH).expect("x64 or ARM64");
        let mut files = vec![file(&setup), file("SHA256SUMS.txt")];
        if signed {
            files.push(file("SHA256SUMS.txt.minisig"));
        }
        format!(
            r#"{{"tag_name":"v9.0.0","html_url":"{PAGES}releases/tag/v9.0.0","assets":[{}]}}"#,
            files.join(",")
        )
        .into_bytes()
    }

    /// GitHub's reply (`signed` or not) and 9.0.0's files, signed with the test main key.
    fn github(signed: bool) -> String {
        let setup = format!(
            "/{}",
            install::setup_name("9.0.0", std::env::consts::ARCH).expect("x64 or ARM64")
        );
        serve(move |path| match path {
            "/latest" => (200, reply(signed)),
            "/SHA256SUMS.txt" => (200, SIGNED_SUMS.to_vec()),
            "/SHA256SUMS.txt.minisig" => (200, BY_MAIN.to_vec()),
            p if p == setup => (200, SETUP.to_vec()),
            _ => (404, Vec::new()),
        })
    }

    #[tokio::test]
    async fn a_manual_check_on_the_installed_copy_downloads_the_setup() {
        let cache = test_cache("manual-run");
        let base = github(true);

        // "Check for updates" off: the click asks all the same.
        let running = Running::begin(true).expect("no run going");
        let remote = Remote::local(&base, true);
        let found = run(running, cache.clone(), Trigger::Manual, false, true, remote).await;

        assert_eq!(
            found.newer.as_ref().map(|r| r.version.as_str()),
            Some("9.0.0")
        );
        let ready = found.ready.expect("the setup downloaded and checked");
        assert!(ready.verified(&cache));
        assert_eq!(found.announce, None, "a click is answered in About");
        let stamp = Stamp::load(&stamp_path(&cache));
        assert_eq!(stamp.ready, Some(ready));
        assert_eq!(
            stamp.announced.as_deref(),
            Some("9.0.0"),
            "nor toasted later"
        );
        assert!(!status().running);
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[tokio::test]
    async fn an_unsigned_release_is_refused_for_good_and_said() {
        let cache = test_cache("unsigned-run");
        let base = github(false);

        let running = Running::begin(true).expect("no run going");
        let remote = Remote::local(&base, true);
        let found = run(running, cache.clone(), Trigger::Startup, true, true, remote).await;

        assert_eq!(found.ready, None, "never installed unsigned");
        assert!(found.unverified);
        assert!(!cache.join("updates").exists(), "nothing downloaded");
        let stamp = Stamp::load(&stamp_path(&cache));
        assert!(stamp.unverified(CURRENT));
        assert_eq!(stamp.to_fetch(CURRENT, SystemTime::now(), true), None);
        let about = Status {
            stamp,
            kind: Kind::Installed,
            ..Status::default()
        };
        assert_eq!(about.banner(true), Banner::Unverified);
        assert_eq!(
            Notice::of(&found, Trigger::Startup),
            Some(Notice::Unverified("9.0.0".into()))
        );
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn a_second_run_while_one_is_running_does_nothing() {
        let first = Running::begin(true).expect("no run going");
        assert!(Running::begin(true).is_none(), "one run at a time");
        assert!(status().checking);
        drop(first);
        assert!(!status().checking);
        assert!(Running::begin(true).is_some(), "free again once it ended");
    }
}
