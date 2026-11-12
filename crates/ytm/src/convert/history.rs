//! History

use featherplay_core::model::Track;
use serde_json::Value;

use super::home::home_section;
use super::playlists::playlist_row;
use super::{check_error, json, layout, logged_in, no_contents, non_empty, text};
use crate::{ApiError, HistoryDay};

/// The listening history (`FEmusic_history`): one shelf of song rows per day, newest first.
/// Signed out, the page only asks to sign in: `AuthExpired`.
pub fn history(raw: &str) -> Result<Vec<HistoryDay>, ApiError> {
    if logged_in(raw) == Some(false) {
        return Err(ApiError::AuthExpired);
    }
    let v = json(raw)?;
    check_error(&v)?;
    let Some(section) = home_section(&v) else {
        if no_contents(&v) {
            return Ok(Vec::new());
        }
        return Err(ApiError::Parse(format!(
            "history in an unknown layout ({})",
            layout(&v)
        )));
    };
    let sections = section
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let days: Vec<HistoryDay> = sections
        .iter()
        .filter_map(|s| s.get("musicShelfRenderer"))
        .filter_map(|shelf| {
            let tracks: Vec<Track> = shelf
                .get("contents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(playlist_row)
                .collect();
            let title = text(shelf.get("title")).and_then(non_empty)?;
            (!tracks.is_empty()).then_some(HistoryDay { title, tracks })
        })
        .collect();
    let asks_to_sign_in = sections.iter().any(|s| {
        s.pointer("/itemSectionRenderer/contents/0/messageRenderer")
            .is_some()
    });
    if days.is_empty() && asks_to_sign_in {
        return Err(ApiError::AuthExpired);
    }
    Ok(days)
}
