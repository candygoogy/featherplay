//! Conversions from the model types to the Slint structs the pages share. Pure: no I/O,
//! no event loop, so they are unit-tested below. Covers start empty;
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
            selected: false,
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

#[cfg(test)]
mod tests {
    use featherplay_core::model::{
        AlbumId, ArtistId, ArtistRef, LikeStatus, PlaylistId, TrackId, VideoType,
    };

    use super::*;
    use crate::controller::now_playing;

    fn track(title: &str, artists: &[&str]) -> Track {
        Track {
            id: TrackId("abc123".into()),
            title: title.into(),
            artists: artists
                .iter()
                .map(|name| ArtistRef {
                    id: Some(ArtistId(format!("UC{name}"))),
                    name: (*name).into(),
                })
                .collect(),
            album: None,
            duration: Some(Duration::from_secs(237)),
            video_type: VideoType::Song,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            counterpart: None,
        }
    }

    #[test]
    fn format_duration_cases() {
        assert_eq!(format_duration(Duration::ZERO), "0:00");
        assert_eq!(format_duration(Duration::from_secs(237)), "3:57");
        assert_eq!(format_duration(Duration::from_secs(3723)), "1:02:03");
    }

    #[test]
    fn to_rows_joins_artists() {
        let rows = to_rows(&[track("Song", &["A", "B"])]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].artist.as_str(), "A, B");
        assert_eq!(rows[0].title.as_str(), "Song");
        assert_eq!(rows[0].duration.as_str(), "3:57");
    }

    #[test]
    fn a_music_video_shows_its_short_title() {
        let raw = "Mirror ft. Bruno Mars (Official Music Video) (feat. Bruno Mars)";
        let mv = Track {
            video_type: VideoType::MusicVideo,
            ..track(raw, &["Lil Wayne", "Bruno Mars"])
        };
        let short = "Mirror ft. Bruno Mars";
        assert_eq!(to_rows(std::slice::from_ref(&mv))[0].title.as_str(), short);
        // The media card, the tray's tooltip, Discord and the toasts start from `now_playing`.
        assert_eq!(now_playing(&mv, true, Duration::ZERO).title, short);
        assert_eq!(
            home_card(&HomeItem::Track(mv.clone())).title.as_str(),
            short
        );
        assert_eq!(mv.title, raw, "the track keeps its raw title");
    }

    #[test]
    fn to_rows_unicode_long_title_ok() {
        // Right-to-left Arabic, an emoji with a skin tone and a ZWJ family, repeated past 300
        // chars.
        let unit = "مرحبا بالعالم 👋🏽👨‍👩‍👧 ";
        let mut title = String::new();
        while title.chars().count() < 300 {
            title.push_str(unit);
        }
        let title: String = title.chars().take(300).collect();
        assert_eq!(title.chars().count(), 300);
        let rows = to_rows(&[track(&title, &["فيروز"])]);
        assert_eq!(rows[0].title.as_str(), title);
        assert_eq!(rows[0].artist.as_str(), "فيروز");
    }

    #[test]
    fn home_items_become_cards_of_their_kind() {
        let mut song = track("Sleepless", &["Dutch Melrose", "B"]);
        song.thumbnail = Some("https://img/song".into());
        let card = home_card(&HomeItem::Track(song));
        assert_eq!(card.kind, ItemKind::Song);
        assert_eq!(card.id.as_str(), "abc123");
        assert_eq!(card.title.as_str(), "Sleepless");
        assert_eq!(card.subtitle.as_str(), "Dutch Melrose, B");
        assert!(!card.round);

        let album = HomeItem::Album(AlbumSummary {
            id: AlbumId("MPREb_x".into()),
            title: "HIT ME HARD AND SOFT".into(),
            artists: vec![ArtistRef {
                id: None,
                name: "Billie Eilish".into(),
            }],
            year: Some("2024".into()),
            thumbnail: Some("https://img/album".into()),
            single: false,
        });
        let card = home_card(&album);
        assert_eq!(card.kind, ItemKind::Album);
        assert_eq!(card.id.as_str(), "MPREb_x");
        assert_eq!(card.subtitle.as_str(), "Billie Eilish, 2024");
        assert_eq!(home_thumbnail(&album).as_deref(), Some("https://img/album"));

        let playlist = HomeItem::Playlist(PlaylistSummary {
            id: PlaylistId("RDTMAK5uy".into()),
            title: "Archive Mix".into(),
            author: None,
            count: None,
            views: None,
            avatar: None,
            thumbnail: None,
            owned: false,
        });
        let card = home_card(&playlist);
        assert_eq!(card.kind, ItemKind::Playlist);
        assert_eq!(card.subtitle.as_str(), "");
        assert_eq!(home_thumbnail(&playlist), None);

        let artist = home_card(&HomeItem::Artist(ArtistSummary {
            id: ArtistId("UCn81".into()),
            name: "Nessa Barrett".into(),
            thumbnail: None,
        }));
        assert_eq!(artist.kind, ItemKind::Artist);
        assert!(artist.round, "artists are round");
    }

    #[test]
    fn a_community_playlist_keeps_its_collage_and_wears_its_creators_avatar_if_it_has_one() {
        let community = |avatar: Option<&str>| {
            HomeItem::Playlist(PlaylistSummary {
                id: PlaylistId("PLcommunity".into()),
                title: "late night drive".into(),
                author: Some("Creator".into()),
                count: None,
                views: Some("529K views".into()),
                avatar: avatar.map(str::to_string),
                thumbnail: Some("https://img/collage".into()),
                owned: false,
            })
        };
        // (case, the creator's avatar, the badge)
        let cases = [
            (
                "with an avatar",
                Some("https://img/avatar"),
                Some("https://img/avatar"),
            ),
            ("without an avatar", None, None),
        ];
        for (case, avatar, badge) in cases {
            let item = community(avatar);
            assert_eq!(
                home_card(&item).subtitle.as_str(),
                "Creator, 529K views",
                "{case}"
            );
            assert_eq!(home_badge(&item).as_deref(), badge, "{case}");
        }
        assert_eq!(
            home_thumbnail(&community(Some("https://img/avatar"))).as_deref(),
            Some("https://img/collage")
        );
    }
}
