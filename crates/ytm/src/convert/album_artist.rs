//! Album and artist pages

use featherplay_core::model::{AlbumId, AlbumRef, ArtistId, ArtistRef, Track, TrackId, VideoType};
use serde_json::Value;
use ytmapi_rs::common::{ArtistChannelID, YoutubeID};
use ytmapi_rs::parse::{AlbumResult, GetArtist};
use ytmapi_rs::query::GetArtistQuery;

use super::playlists::{
    first_run_text, flex_column, greyed_out, playlist_row, row_duration, row_like, runs_of,
};
use super::radio::browse_target;
use super::{
    Extra, MUSIC_VIDEO_TYPE, PLAY_WATCH_ENDPOINT, ROW_THUMBNAILS, Skipped, album_ref, artist_ref,
    best, best_raw, check_error, json, layout, like, log_left_out, merge_skipped, non_empty,
    process_lenient, unavailable, video_type,
};
use crate::{Album, AlbumSummary, ApiError, Artist, ArtistSummary};

const ALBUM_HEADER: &str = "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer";
const ALBUM_ROWS: &str = "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicShelfRenderer/contents";

/// The album page does not repeat its own id, so the caller passes it.
/// Tracks inherit the album's artists (unless a row lists its own), the
/// album ref and the album cover.
///
/// Read from raw JSON one row at a time, like playlists: ytmapi-rs 0.3.3 fails the
/// whole album on one row without a duration or plays column. Greyed-out rows are
/// dropped; a row without a duration is kept without one.
pub fn album(raw: &str, id: &AlbumId) -> Result<Album, ApiError> {
    if unavailable(raw) {
        return Err(ApiError::NotFound);
    }
    let v = json(raw)?;
    check_error(&v)?;
    let h = v
        .pointer(ALBUM_HEADER)
        .ok_or_else(|| ApiError::Parse(format!("album has no header ({})", layout(&v))))?;
    let title = first_run_text(h.get("title"))
        .ok_or_else(|| ApiError::Parse("album has no title".into()))?;
    let artists = runs_of(h.get("straplineTextOne"))
        .iter()
        .step_by(2)
        .map(|run| ArtistRef {
            id: run
                .pointer("/navigationEndpoint/browseEndpoint/browseId")
                .and_then(Value::as_str)
                .map(|id| ArtistId(id.to_string())),
            name: run
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .collect();
    let summary = AlbumSummary {
        id: id.clone(),
        title,
        artists,
        year: runs_of(h.get("subtitle"))
            .get(2)
            .and_then(|r| r.get("text"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .and_then(non_empty),
        thumbnail: best_raw(h.pointer(ROW_THUMBNAILS)),
    };
    let album_ref = AlbumRef {
        id: id.clone(),
        title: summary.title.clone(),
    };
    let rows = v
        .pointer(ALBUM_ROWS)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let tracks = rows
        .iter()
        .filter_map(|item| {
            let r = item.get("musicResponsiveListItemRenderer")?;
            if greyed_out(r) {
                return None;
            }
            let title = first_run_text(flex_column(r, 0))?;
            let watch = r.pointer(PLAY_WATCH_ENDPOINT);
            let tid = watch
                .and_then(|w| w.get("videoId"))
                .or_else(|| r.pointer("/playlistItemData/videoId"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())?;
            let video_type = watch
                .and_then(|w| w.pointer(MUSIC_VIDEO_TYPE))
                .or_else(|| {
                    runs_of(flex_column(r, 0)).first().and_then(|run| {
                        run.pointer("/navigationEndpoint/watchEndpoint")
                            .and_then(|w| w.pointer(MUSIC_VIDEO_TYPE))
                    })
                })
                .and_then(Value::as_str)
                .map(video_type)
                .unwrap_or(VideoType::Song);
            let own: Vec<ArtistRef> = runs_of(flex_column(r, 1))
                .iter()
                .filter_map(|run| {
                    let (aid, page) = browse_target(run)?;
                    (page == "MUSIC_PAGE_TYPE_ARTIST" || page == "MUSIC_PAGE_TYPE_USER_CHANNEL")
                        .then(|| ArtistRef {
                            id: Some(ArtistId(aid.to_string())),
                            name: run
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        })
                })
                .collect();
            Some(Track {
                artists: if own.is_empty() {
                    summary.artists.clone()
                } else {
                    own
                },
                album: Some(album_ref.clone()),
                duration: row_duration(r),
                video_type,
                thumbnail: summary.thumbnail.clone(),
                like: row_like(r),
                counterpart: None,
                title,
                id: TrackId(tid.to_string()),
            })
        })
        .collect();
    Ok(Album { summary, tracks })
}

/// Top songs carry no duration on the artist page. A top song ytmapi-rs can't read is read
/// from its row (`playlist_row`); an album it can't read is left out.
pub fn artist(raw: &str) -> Result<Artist, ApiError> {
    if unavailable(raw) {
        return Err(ApiError::NotFound);
    }
    let extra = Extra::from_raw(raw)?;
    let q = GetArtistQuery::new(ArtistChannelID::from_raw(""));
    let (a, skipped): (GetArtist, _) = process_lenient(raw, &q)?;
    // The songs shelf is the page's one list of rows; ytmapi-rs reads cards only in the
    // albums carousel.
    let (song_rows, cards): (Vec<Skipped>, Vec<Skipped>) = skipped
        .into_iter()
        .partition(|s| s.renderer == "musicResponsiveListItemRenderer");
    log_left_out("artist albums", &cards);
    let summary = ArtistSummary {
        id: ArtistId(a.channel_id.get_raw().to_string()),
        name: a.name,
        thumbnail: best(&a.thumbnails),
    };
    let me = ArtistRef {
        id: Some(summary.id.clone()),
        name: summary.name.clone(),
    };
    let releases = a.top_releases;
    let top_songs = releases
        .songs
        .map(|s| s.results)
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let id = s.video_id.get_raw().to_string();
            Track {
                artists: s.artists.into_iter().map(artist_ref).collect(),
                album: Some(album_ref(s.album)),
                duration: None,
                video_type: extra.video_type_or(&id, VideoType::Song),
                thumbnail: extra.rows.get(&id).and_then(|r| r.thumbnail.clone()),
                like: like(&s.like_status),
                counterpart: None,
                title: s.title,
                id: TrackId(id),
            }
        })
        .collect();
    let top_songs = merge_skipped("artist top songs", top_songs, song_rows, playlist_row);
    let releases_of = |r: Option<ytmapi_rs::parse::GetArtistAlbums>| -> Vec<AlbumSummary> {
        r.map(|r| r.results)
            .unwrap_or_default()
            .into_iter()
            .map(|a: AlbumResult| AlbumSummary {
                id: AlbumId(a.album_id.get_raw().to_string()),
                title: a.title,
                artists: vec![me.clone()],
                year: non_empty(a.year),
                thumbnail: best(&a.thumbnails),
            })
            .collect()
    };
    Ok(Artist {
        albums: releases_of(releases.albums),
        singles: releases_of(releases.singles),
        summary,
        top_songs,
    })
}
