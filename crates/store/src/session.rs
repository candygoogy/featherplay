use std::fs;
use std::io;
use std::path::PathBuf;

use featherplay_core::model::{RepeatMode, Session, SessionPosition, Track};
use serde::{Deserialize, Serialize};

use crate::{Paths, atomic};

fn file(p: &Paths) -> PathBuf {
    p.config.join("session.json")
}

/// The position (`SessionPosition`) saved after `session.json` was, for the queue with the
/// same `queue_tag`. 0.7.0 and before read `session.json` alone, with its older position.
fn position_file(p: &Paths) -> PathBuf {
    p.config.join("session-position.json")
}

/// The session as loaded, and the tag of its queue (`None` in a file 0.7.0 or before
/// wrote): a position saved later for it goes with this tag (`save_session_position`).
#[derive(Debug, PartialEq, Eq)]
pub struct SavedSession {
    pub session: Session,
    pub queue_tag: Option<u64>,
}

/// `session.json` as written: the session, then the tag that ties the position file to it.
#[derive(Serialize)]
struct SessionOut<'a> {
    #[serde(flatten)]
    session: &'a Session,
    queue_tag: u64,
}

/// `session.json` as read. The session's fields again rather than a flattened `Session`,
/// which serde would read into a tree first (the whole queue, twice).
#[derive(Deserialize)]
struct SessionIn {
    queue: Vec<Track>,
    index: usize,
    position_ms: u64,
    radio: bool,
    shuffle: bool,
    repeat: RepeatMode,
    #[serde(default)]
    queue_tag: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct PositionFile {
    queue_tag: u64,
    #[serde(flatten)]
    position: SessionPosition,
}

/// Loads the saved session from `config/session.json`, with the newer position from
/// `config/session-position.json` when that was saved for the same queue (and fits it). A
/// missing or corrupt `session.json` gives `None`; a missing, corrupt or stale position file
/// leaves `session.json`'s position.
pub fn load_session(p: &Paths) -> Option<SavedSession> {
    let bytes = fs::read(file(p)).ok()?;
    let read: SessionIn = serde_json::from_slice(&bytes).ok()?;
    let mut session = Session {
        queue: read.queue,
        index: read.index,
        position_ms: read.position_ms,
        radio: read.radio,
        shuffle: read.shuffle,
        repeat: read.repeat,
    };
    if let Some(tag) = read.queue_tag
        && let Some(position) = load_position(p, tag, session.queue.len())
    {
        session.set_position(position);
    }
    Some(SavedSession {
        session,
        queue_tag: read.queue_tag,
    })
}

fn load_position(p: &Paths, tag: u64, len: usize) -> Option<SessionPosition> {
    let bytes = fs::read(position_file(p)).ok()?;
    let read: PositionFile = serde_json::from_slice(&bytes).ok()?;
    (read.queue_tag == tag && read.position.index < len).then_some(read.position)
}

/// Writes the whole session to `config/session.json` (atomically), tagged `queue_tag`.
pub fn save_session(s: &Session, queue_tag: u64, p: &Paths) -> io::Result<()> {
    let out = SessionOut {
        session: s,
        queue_tag,
    };
    let bytes = serde_json::to_vec(&out).map_err(io::Error::other)?;
    atomic::write(&file(p), &bytes)
}

/// Writes the position of the queue saved with `queue_tag` to `config/session-position.json`
/// (atomically). Loaded over `session.json`'s only while that has the same tag.
pub fn save_session_position(
    position: &SessionPosition,
    queue_tag: u64,
    p: &Paths,
) -> io::Result<()> {
    let out = PositionFile {
        queue_tag,
        position: *position,
    };
    let bytes = serde_json::to_vec(&out).map_err(io::Error::other)?;
    atomic::write(&position_file(p), &bytes)
}

fn version_file(p: &Paths) -> PathBuf {
    p.cache.join("client-version.txt")
}

/// The web client version the last sign-in read (`1.20261106.01.00`), so the next start can
/// sign in without reading it again (`YtMusic::resume`). The same for every account.
pub fn load_client_version(p: &Paths) -> Option<String> {
    let text = fs::read_to_string(version_file(p)).ok()?;
    let version = text.trim();
    (!version.is_empty()).then(|| version.to_string())
}

pub fn save_client_version(p: &Paths, version: &str) -> io::Result<()> {
    atomic::write(&version_file(p), version.as_bytes())
}
