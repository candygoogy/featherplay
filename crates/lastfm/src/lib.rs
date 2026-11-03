//! Last.fm client: request signing, the desktop auth flow, scrobbling and the offline queue.
//!
//! The API key and secret are passed in by the caller; this crate never reads the environment.

mod client;
mod offline;
mod sign;

pub use client::{LastFm, LastFmError, MAX_BATCH};
pub use offline::{OfflineQueue, PendingScrobble};
pub use sign::api_sig;
