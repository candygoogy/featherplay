//! Lyrics

use std::time::Duration;

use serde_json::Value;

use super::{check_error, json, layout, text};
use crate::{ApiError, PlainLyrics, TimedLine, TimedLyrics};

const WATCH_TABS: &str = "/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs";

/// The browse id of the Lyrics tab in a watch-next response, or `None` when the track
/// has no lyrics (the tab is there but unselectable, without an endpoint).
pub fn lyrics_browse_id(raw: &str) -> Result<Option<String>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let tabs = v
        .pointer(WATCH_TABS)
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Parse(format!("no watch tabs in {}", layout(&v))))?;
    // Found by its browse id's prefix rather than its place: the tabs' order varies.
    Ok(tabs.iter().find_map(|t| {
        t.pointer("/tabRenderer/endpoint/browseEndpoint/browseId")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("MPLY"))
            .map(str::to_string)
    }))
}

/// The plain lyrics on a Lyrics tab's browse page, or `None` when it says there are none.
pub fn lyrics(raw: &str) -> Result<Option<PlainLyrics>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let shelf = v
        .pointer("/contents/sectionListRenderer/contents")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find_map(|i| i.get("musicDescriptionShelfRenderer"))
        });
    let Some(shelf) = shelf else {
        // "Lyrics not available" (a `messageRenderer`), or an empty page.
        return Ok(None);
    };
    let found = text(shelf.get("description"))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .map(|words| PlainLyrics {
            text: words,
            source: text(shelf.get("footer")).filter(|s| !s.trim().is_empty()),
        });
    Ok(found)
}

/// Where the mobile client's Lyrics tab page keeps its lines, as ytmusicapi reads them.
const TIMED_LYRICS: &str =
    "/contents/elementRenderer/newElement/type/componentType/model/timedLyricsModel/lyricsData";

/// The timed lyrics on a Lyrics tab's page as the mobile client gets it, sorted by start, or
/// `None` when they aren't timed. Most songs come untimed (the same lines, without a
/// `cueRange`); as in ytmusicapi, one untimed line makes them all untimed.
pub fn timed_lyrics(raw: &str) -> Result<Option<TimedLyrics>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let Some(data) = v.pointer(TIMED_LYRICS) else {
        return Ok(None);
    };
    let Some(raw_lines) = data.get("timedLyricsData").and_then(Value::as_array) else {
        return Ok(None);
    };
    let lines: Option<Vec<TimedLine>> = raw_lines.iter().map(timed_line).collect();
    let Some(mut lines) = lines.filter(|l| !l.is_empty()) else {
        return Ok(None);
    };
    // Stable, so lines sharing a start keep the page's order.
    lines.sort_by_key(|l| l.start);
    let source = data
        .get("sourceMessage")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(Some(TimedLyrics { lines, source }))
}

/// One `timedLyricsData` entry, or `None` without a start time.
fn timed_line(v: &Value) -> Option<TimedLine> {
    let start = v
        .pointer("/cueRange/startTimeMilliseconds")
        .and_then(Value::as_str)?
        .parse()
        .ok()?;
    Some(TimedLine {
        start: Duration::from_millis(start),
        text: v
            .get("lyricLine")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
    })
}
