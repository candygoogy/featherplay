//! Where songs are silent, per video id, so a song played again shows Skip silence's marks
//! at once, without scanning it again: `{cache}/silences.json`. Each entry is the audio
//! engine's scan of the stream (`featherplay_audio::Silences`): its rate, its length and its
//! silent runs, in frames. No URL, nothing about the account. At most `CAP` songs; the
//! oldest go first.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Paths, atomic};

/// Songs kept: a few months of listening, about 50 KB.
pub const CAP: usize = 500;

/// A song's silent runs, as the audio engine's scan found them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SongSilences {
    pub rate: u32,
    pub frames: u64,
    /// (first frame, frames).
    pub runs: Vec<(u64, u64)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    id: String,
    #[serde(flatten)]
    silences: SongSilences,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    songs: Vec<Entry>,
}

/// The cache in memory, oldest first. Cheap to clone, to save it off the caller's thread.
#[derive(Clone, Debug)]
pub struct SilenceCache {
    path: PathBuf,
    songs: Vec<Entry>,
}

impl SilenceCache {
    /// The cache saved last; empty if there's none or it can't be read.
    pub fn load(paths: &Paths) -> SilenceCache {
        let path = paths.cache.join("silences.json");
        let songs = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<File>(&b).ok())
            .map(|f| f.songs)
            .unwrap_or_default();
        SilenceCache { path, songs }
    }

    pub fn get(&self, id: &str) -> Option<&SongSilences> {
        self.songs
            .iter()
            .rev()
            .find(|e| e.id == id)
            .map(|e| &e.silences)
    }

    /// Adds or replaces `id`'s silences as the newest; past `CAP`, the oldest go.
    pub fn insert(&mut self, id: &str, silences: SongSilences) {
        self.songs.retain(|e| e.id != id);
        self.songs.push(Entry {
            id: id.to_owned(),
            silences,
        });
        let over = self.songs.len().saturating_sub(CAP);
        self.songs.drain(..over);
    }

    /// Writes the cache (atomically, so a crash leaves the old one).
    pub fn save(&self) -> io::Result<()> {
        let file = File {
            songs: self.songs.clone(),
        };
        let bytes = serde_json::to_vec(&file).map_err(io::Error::other)?;
        atomic::write(&self.path, &bytes)
    }
}
