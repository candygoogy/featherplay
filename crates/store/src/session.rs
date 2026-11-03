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
