//! What the user chose for a song's lyrics, per video id: the timing nudge ("0.5 s later")
//! and a pick in "Choose other lyrics" (an LRCLIB record, or none at all):
//! `{config}/lyrics.json`. Kept on this PC only; nothing about the account. A song with
//! neither isn't stored. At most `CAP` songs; the ones changed longest ago go first.
//!
//! Read leniently, as a hand-edited file or a newer featherplay's may hold anything: a
//! nudge past `MAX_NUDGE_MS` is clamped and one that isn't a number is 0, a pick this build
//! doesn't know is no pick, and a song entry that can't be read is skipped; a file that
//! isn't JSON at all is no choices. Never an error.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Paths, atomic};

/// Songs kept. A pick holds its words (a few kB); a nudge alone is a few bytes.
pub const CAP: usize = 1000;
/// The most the lines move either way, in milliseconds.
pub const MAX_NUDGE_MS: i32 = 10_000;

/// One song's choices.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsChoice {
    /// How much later (positive) or sooner (negative) the lines show, in milliseconds, on
    /// top of any `[offset:]` their LRC carries.
    #[serde(
        default,
        skip_serializing_if = "is_zero",
        deserialize_with = "lenient_nudge"
    )]
    pub nudge_ms: i32,
    /// The lyrics picked for the song instead of the automatic match.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_pick"
    )]
    pub pick: Option<LyricsPick>,
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

/// A whole number of milliseconds, clamped to `MAX_NUDGE_MS` either way; anything else 0.
fn lenient_nudge<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i32, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    let ms = value
        .as_i64()
        .or_else(|| value.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
        .unwrap_or(0)
        .clamp(i64::from(-MAX_NUDGE_MS), i64::from(MAX_NUDGE_MS));
    Ok(i32::try_from(ms).unwrap_or(0))
}

/// A pick this build knows; anything else is no pick (the automatic match).
fn lenient_pick<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<LyricsPick>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).ok())
}

/// What "Choose other lyrics" picked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LyricsPick {
    /// "None of these": no lyrics for this song.
    None,
    /// An LRCLIB record, its words kept as LRCLIB gave them, so it shows without asking.
    Lrclib {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        synced: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plain: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    id: String,
    #[serde(flatten)]
    choice: LyricsChoice,
}

#[derive(Default, Serialize)]
struct File {
    songs: Vec<Entry>,
}

/// The file as read: each entry on its own, so one that can't be read is skipped alone.
#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    songs: Vec<serde_json::Value>,
}

/// The choices in memory, oldest change first. Cheap to clone, to save it off the
/// caller's thread.
#[derive(Clone, Debug)]
pub struct LyricsChoices {
    path: PathBuf,
    songs: Vec<Entry>,
}

impl LyricsChoices {
    /// The choices saved last; none if there's no file or it can't be read.
    pub fn load(paths: &Paths) -> LyricsChoices {
        let path = paths.config.join("lyrics.json");
        let songs = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<RawFile>(&b).ok())
            .map(|f| {
                f.songs
                    .into_iter()
                    .filter_map(|e| serde_json::from_value::<Entry>(e).ok())
                    .collect()
            })
            .unwrap_or_default();
        LyricsChoices { path, songs }
    }

    /// `id`'s choices; the defaults (on time, the automatic match) when it has none.
    pub fn get(&self, id: &str) -> LyricsChoice {
        self.songs
            .iter()
            .rev()
            .find(|e| e.id == id)
            .map(|e| e.choice.clone())
            .unwrap_or_default()
    }

    /// Sets `id`'s choices as the newest change; the defaults forget the song. Past `CAP`,
    /// the oldest go.
    pub fn set(&mut self, id: &str, choice: LyricsChoice) {
        self.songs.retain(|e| e.id != id);
        if choice != LyricsChoice::default() {
            self.songs.push(Entry {
                id: id.to_owned(),
                choice,
            });
        }
        let over = self.songs.len().saturating_sub(CAP);
        self.songs.drain(..over);
    }

    /// Writes the choices (atomically, so a crash leaves the old file).
    pub fn save(&self) -> io::Result<()> {
        let file = File {
            songs: self.songs.clone(),
        };
        let bytes = serde_json::to_vec(&file).map_err(io::Error::other)?;
        atomic::write(&self.path, &bytes)
    }
}
