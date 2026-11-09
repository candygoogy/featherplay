//! Conversions from the model types to the Slint structs the pages share. Pure: no I/O,
//! no event loop, so they're unit-tested in `tests/ui_bridge.rs`. Covers start empty;
//! `covers::Covers` fills them in when they arrive.

use std::rc::Rc;
use std::time::Duration;

use featherplay_core::model::{ArtistRef, LikeStatus, Track};
use featherplay_core::text::plain;
use featherplay_core::tr;
use featherplay_ytm::{AlbumSummary, ArtistSummary, HomeItem, PlaylistSummary};
use slint::{ModelRc, SharedString, VecModel};

use crate::ui::{CardItem, ItemKind, TrackRow};

/// `"3:57"`, or `"1:02:03"` from an hour up. Fractions of a second are dropped.
pub fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A title, artist or album name as the UI shows it: styled maths letters ("𝙡𝙪𝙪") as plain
/// ones (`text::plain`). Only for what's drawn; what's scrobbled, searched or sent back keeps
/// the name as written.
pub fn shown(s: &str) -> SharedString {
    plain(s).as_ref().into()
}

/// `"A, B"`.
pub fn artist_names(artists: &[ArtistRef]) -> String {
    artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// One row per track, numbered from 1, in order. Titles are only `shown`; eliding long ones
/// is Slint's job.
pub fn to_rows(tracks: &[Track]) -> Vec<TrackRow> {
    tracks
        .iter()
        .enumerate()
        .map(|(i, t)| TrackRow {
            id: t.id.0.as_str().into(),
            index: (i + 1).to_string().into(),
            title: shown(&t.display_title()),
            artist: shown(&artist_names(&t.artists)),
            artist_id: t
                .artists
                .iter()
                .find_map(|a| a.id.as_ref())
                .map(|id| SharedString::from(id.0.as_str()))
                .unwrap_or_default(),
            album: t
                .album
                .as_ref()
                .map(|a| shown(&a.title))
                .unwrap_or_default(),
            album_id: t
                .album
                .as_ref()
                .map(|a| SharedString::from(a.id.0.as_str()))
                .unwrap_or_default(),
            duration: t
                .duration
                .map(|d| SharedString::from(format_duration(d)))
                .unwrap_or_default(),
            cover: Default::default(),
            liked: t.like == LikeStatus::Like,
            playing: false,
        })
        .collect()
}

/// "Daft Punk, 1996": an album card's subtitle, the artists, then the year, as written.
pub fn album_subtitle(a: &AlbumSummary) -> String {
    let mut subtitle = artist_names(&a.artists);
    if let Some(year) = &a.year {
        if !subtitle.is_empty() {
            subtitle.push_str(", ");
        }
        subtitle.push_str(year);
    }
    subtitle
}

pub fn album_card(a: &AlbumSummary) -> CardItem {
    CardItem {
        id: a.id.0.as_str().into(),
        kind: ItemKind::Album,
        title: shown(&a.title),
        subtitle: shown(&album_subtitle(a)),
        cover: Default::default(),
        round: false,
    }
}

pub fn artist_card(a: &ArtistSummary) -> CardItem {
    CardItem {
        id: a.id.0.as_str().into(),
        kind: ItemKind::Artist,
        title: shown(&a.name),
        subtitle: tr!("Artist").into(),
        cover: Default::default(),
        round: true,
    }
}

/// A playlist card's subtitle, as written: the author, then the track count when known.
pub fn playlist_subtitle(p: &PlaylistSummary) -> String {
    let mut parts = Vec::new();
    if let Some(author) = &p.author {
        parts.push(author.clone());
    }
    if let Some(views) = &p.views {
        parts.push(views.clone());
    }
    if let Some(n) = p.count {
        parts.push(if n == 1 {
            "1 song".to_string()
        } else {
            format!("{n} songs")
        });
    }
    parts.join(", ")
}

pub fn playlist_card(p: &PlaylistSummary) -> CardItem {
    CardItem {
        id: p.id.0.as_str().into(),
        kind: ItemKind::Playlist,
        title: shown(&p.title),
        subtitle: shown(&playlist_subtitle(p)),
        cover: Default::default(),
        round: false,
    }
}

/// A Home feed item as a card: albums, playlists and artists as above; a song (or video)
/// with its artists as the subtitle.
pub fn home_card(item: &HomeItem) -> CardItem {
    match item {
        HomeItem::Album(a) => album_card(a),
        HomeItem::Playlist(p) => playlist_card(p),
        HomeItem::Artist(a) => artist_card(a),
        HomeItem::Track(t) => CardItem {
            id: t.id.0.as_str().into(),
            kind: ItemKind::Song,
            title: shown(&t.display_title()),
            subtitle: shown(&artist_names(&t.artists)),
            cover: Default::default(),
            round: false,
        },
    }
}

/// The cover URL of a Home feed item.
pub fn home_thumbnail(item: &HomeItem) -> Option<String> {
    match item {
        HomeItem::Album(a) => a.thumbnail.clone(),
        HomeItem::Playlist(p) => p.thumbnail.clone(),
        HomeItem::Artist(a) => a.thumbnail.clone(),
        HomeItem::Track(t) => t.thumbnail.clone(),
    }
}

/// The round badge a Home card wears over its cover: a community playlist's creator.
pub fn home_badge(item: &HomeItem) -> Option<String> {
    match item {
        HomeItem::Playlist(p) => p.avatar.clone(),
        _ => None,
    }
}

/// A `VecModel` behind a `ModelRc`, keeping the `Rc` so covers can update rows in place.
pub fn model<T: Clone + 'static>(items: Vec<T>) -> (Rc<VecModel<T>>, ModelRc<T>) {
    let rc = Rc::new(VecModel::from(items));
    let model = ModelRc::from(rc.clone());
    (rc, model)
}
