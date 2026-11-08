//! The last good result of each Library list, so the Library shows at once and refreshes
//! behind it. Kept per identity (`featherplay_ytm::Identity::key`: "" for the session's own,
//! "2" or "0/<page id>" for another), under `{cache}/library/<identity>/<list>.json`.
//!
//! It holds what the lists show (titles, ids, thumbnail URLs), never cookies or secrets.
//! Sign-out and a new sign-in remove it all (`clear`); a session that ended, Home's feed
//! (`forget`).

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{Paths, atomic};

/// One of the Library's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryList {
    Liked,
    Playlists,
    Albums,
    Artists,
    /// The playlists, albums and artists opened or played lately, for the sidebar's order
    /// and Home's shortcuts. Kept on this device only.
    Recent,
    /// Home's feed as last fetched, so Home shows at once at the next start.
    Home,
}

impl LibraryList {
    fn file_name(self) -> &'static str {
        match self {
            LibraryList::Liked => "liked.json",
            LibraryList::Playlists => "playlists.json",
            LibraryList::Albums => "albums.json",
            LibraryList::Artists => "artists.json",
            LibraryList::Recent => "recent.json",
            LibraryList::Home => "home.json",
        }
    }
}

/// The cache's directory. Cheap to clone; nothing is created until a list is saved.
#[derive(Clone, Debug)]
pub struct LibraryCache {
    dir: PathBuf,
}

impl LibraryCache {
    pub fn new(paths: &Paths) -> LibraryCache {
        LibraryCache {
            dir: paths.cache.join("library"),
        }
    }

    /// The list last saved for `identity`. Missing, unreadable or from an older format: `None`.
    pub fn load<T: DeserializeOwned>(&self, identity: &str, list: LibraryList) -> Option<T> {
        let bytes = fs::read(self.file(identity, list)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Replaces the list saved for `identity` (atomically, so a crash leaves the old one).
    pub fn save<T: Serialize>(
        &self,
        identity: &str,
        list: LibraryList,
        value: &T,
    ) -> io::Result<()> {
        let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
        atomic::write(&self.file(identity, list), &bytes)
    }

    /// Forgets every identity's lists.
    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    /// Forgets `list` for every identity, keeping the other lists. One that can't be
    /// removed doesn't stop the others; the first error is returned.
    pub fn forget(&self, list: LibraryList) -> io::Result<()> {
        let dirs = match fs::read_dir(&self.dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            other => other?,
        };
        let mut first = Ok(());
        for dir in dirs {
            let removed = dir.and_then(|d| fs::remove_file(d.path().join(list.file_name())));
            match removed {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) if first.is_ok() => first = Err(e),
                _ => {}
            }
        }
        first
    }

    fn file(&self, identity: &str, list: LibraryList) -> PathBuf {
        self.dir.join(dir_name(identity)).join(list.file_name())
    }
}

/// A directory name for an identity key: "own" for the session's own (""), otherwise the
/// key with anything but ASCII letters, digits and '-' replaced by '_' ("0/123" → "0_123").
/// Keys are digits and one '/', so distinct keys stay distinct.
fn dir_name(identity: &str) -> String {
    if identity.is_empty() {
        return "own".into();
    }
    identity
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
