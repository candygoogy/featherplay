//! Watch playlist (radio)

use featherplay_core::model::{
    AlbumId, AlbumRef, ArtistId, ArtistRef, LikeStatus, Track, TrackId, VideoType,
};
use serde_json::Value;

use super::{MUSIC_VIDEO_TYPE, best_raw, check_error, json, parse_duration, text, video_type};
use crate::ApiError;

const WATCH_PANEL: &str = "/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer/contents";
const PAGE_TYPE: &str =
    "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType";

/// Tracks of the watch playlist. Items without a video id (automix
/// previews and the like) are skipped. A music video whose counterpart is a
/// song gets that song as its `counterpart` (its song version).
pub fn radio(raw: &str) -> Result<Vec<Track>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let items = v
        .pointer(WATCH_PANEL)
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Parse("watch playlist has no queue".into()))?;
    Ok(items.iter().filter_map(watch_item).collect())
}

/// The `seed` track itself from its watch playlist: what a pasted link shows. `None` when
/// the queue doesn't hold it, or there's no queue at all (an id that doesn't play).
pub fn seed_track(raw: &str, seed: &TrackId) -> Result<Option<Track>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let Some(items) = v.pointer(WATCH_PANEL).and_then(Value::as_array) else {
        return Ok(None);
    };
    Ok(items.iter().filter_map(watch_item).find(|t| t.id == *seed))
}

fn watch_item(item: &Value) -> Option<Track> {
    let (primary, counterpart) = match item.get("playlistPanelVideoWrapperRenderer") {
        Some(w) => (
            w.pointer("/primaryRenderer/playlistPanelVideoRenderer")?,
            w.pointer("/counterpart/0/counterpartRenderer/playlistPanelVideoRenderer"),
        ),
        None => (item.get("playlistPanelVideoRenderer")?, None),
    };
    let mut track = panel_track(primary)?;
    if track.video_type == VideoType::MusicVideo
        && let Some(mut song) = counterpart.and_then(panel_track)
        && song.video_type == VideoType::Song
    {
        // The counterpart's byline names its artists without links; the video's has them.
        for a in song.artists.iter_mut().filter(|a| a.id.is_none()) {
            a.id = track
                .artists
                .iter()
                .find(|v| v.name == a.name)
                .and_then(|v| v.id.clone());
        }
        track.counterpart = Some(Box::new(song));
    }
    Some(track)
}

fn panel_track(r: &Value) -> Option<Track> {
    let id = r
        .get("videoId")
        .or_else(|| r.pointer("/navigationEndpoint/watchEndpoint/videoId"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let (artists, album) = byline(
        r.pointer("/longBylineText/runs")
            .or_else(|| r.pointer("/shortBylineText/runs"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default(),
    );
    let video_type = r
        .pointer("/navigationEndpoint/watchEndpoint")
        .and_then(|w| w.pointer(MUSIC_VIDEO_TYPE))
        .and_then(Value::as_str)
        .map(video_type)
        .unwrap_or(VideoType::Other);
    Some(Track {
        id: TrackId(id.to_string()),
        title: text(r.get("title")).unwrap_or_default(),
        artists,
        album,
        duration: text(r.get("lengthText"))
            .as_deref()
            .and_then(parse_duration),
        video_type,
        thumbnail: best_raw(r.pointer("/thumbnail/thumbnails")),
        // The watch playlist carries no rating.
        like: LikeStatus::Indifferent,
        counterpart: None,
    })
}

/// Artists and album from byline runs such as
/// `Artist • Album • 1997` or `Artist & Other • 139M views • 1.1M likes`.
/// Even runs are values, odd runs are separators. Linked runs are typed by
/// their browse page; unlinked runs count as artists only before the first
/// `•`.
pub(super) fn byline(runs: &[Value]) -> (Vec<ArtistRef>, Option<AlbumRef>) {
    let mut artists = Vec::new();
    let mut album = None;
    let mut group = 0;
    for (i, run) in runs.iter().enumerate() {
        let t = run.get("text").and_then(Value::as_str).unwrap_or_default();
        if i % 2 == 1 {
            if t.trim() == "•" {
                group += 1;
            }
            continue;
        }
        match browse_target(run) {
            Some((id, page)) if page == "MUSIC_PAGE_TYPE_ALBUM" || id.starts_with("MPRE") => {
                album = Some(AlbumRef {
                    id: AlbumId(id.to_string()),
                    title: t.to_string(),
                });
            }
            Some((id, page))
                if page == "MUSIC_PAGE_TYPE_ARTIST"
                    || page == "MUSIC_PAGE_TYPE_USER_CHANNEL"
                    || id.starts_with("UC") =>
            {
                artists.push(ArtistRef {
                    id: Some(ArtistId(id.to_string())),
                    name: t.to_string(),
                });
            }
            Some(_) => {}
            None if group == 0 && !t.trim().is_empty() => artists.push(ArtistRef {
                id: None,
                name: t.to_string(),
            }),
            None => {}
        }
    }
    (artists, album)
}

/// (browseId, pageType) of a run's browse endpoint.
pub(super) fn browse_target(run: &Value) -> Option<(&str, &str)> {
    let b = run.pointer("/navigationEndpoint/browseEndpoint")?;
    let id = b.get("browseId").and_then(Value::as_str)?;
    let page = b
        .pointer(PAGE_TYPE)
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some((id, page))
}
