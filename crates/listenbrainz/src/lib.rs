//! ListenBrainz client: `submit-listens` (now playing, single listens, imports from the
//! offline queue) and `validate-token`.
//!
//! The user token is passed in by the caller and never logged or shown by `Debug`.

mod client;
mod payload;

pub use client::{API_ROOT, ListenBrainz, ListenBrainzError, MAX_BATCH};
pub use payload::{Listen, ListenType, payload};
