//! The last check and the setup's download, kept in the cache (`update-check.json`), and
//! what is due from them.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::install::Ready;
use super::{
    CHECK_EVERY, CHECK_RETRY, CheckError, FETCH_ATTEMPTS, FETCH_RETRY, Release, STAMP, is_newer,
    unix,
};

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
