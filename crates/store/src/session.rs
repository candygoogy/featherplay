use std::fs;
use std::io;
use std::path::PathBuf;

use featherplay_core::model::Session;

use crate::{Paths, atomic};

fn file(p: &Paths) -> PathBuf {
    p.config.join("session.json")
}

/// Loads the saved session from `config/session.json`. A missing or corrupt file gives `None`.
pub fn load_session(p: &Paths) -> Option<Session> {
    let bytes = fs::read(file(p)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save_session(s: &Session, p: &Paths) -> io::Result<()> {
    let bytes = serde_json::to_vec(s).map_err(io::Error::other)?;
    atomic::write(&file(p), &bytes)
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
