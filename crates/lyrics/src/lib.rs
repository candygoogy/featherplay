//! Lyrics: the LRC format (`lrc`), the LRCLIB lookup (`lrclib`), the Lyrictified lookup
//! (`lyrictified`), word-timed lyrics from Unison (`unison`) and the formats they come in
//! (`words`), and optional translation and romanization (`enrich`).
//!
//! The app asks for the current track's lyrics only while its lyrics view is open; this
//! crate holds no cache and never retries.

pub mod enrich;
mod http;
pub mod lrc;
pub mod lrclib;
pub mod lyrictified;
pub mod unison;
pub mod words;

pub use lrc::{Line, Lyrics};
pub use words::{Part, WordLine};
