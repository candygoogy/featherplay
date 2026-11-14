//! Lyrics: the LRC format (`lrc`), the LRCLIB lookup (`lrclib`), and optional translation
//! and romanization (`enrich`).
//!
//! The app asks for the current track's lyrics only while its lyrics view is open; this
//! crate holds no cache and never retries.

pub mod enrich;
pub mod lrc;
pub mod lrclib;

pub use lrc::{Line, Lyrics};
