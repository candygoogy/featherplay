//! Lyrics: the LRC format (`lrc`) and the LRCLIB lookup (`lrclib`).
//!
//! The app asks for the current track's lyrics only while its lyrics view is open; this
//! crate holds no cache and never retries.

pub mod lrc;
pub mod lrclib;

pub use lrc::{Line, Lyrics};
