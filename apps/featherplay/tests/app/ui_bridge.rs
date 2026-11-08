//! `ui_bridge`: the model-to-row conversions the pages share.

use std::time::Duration;

use featherplay::ui::ItemKind;
use featherplay::ui_bridge::{format_duration, home_badge, home_card, home_thumbnail, to_rows};
use featherplay_core::model::{
    AlbumId, ArtistId, ArtistRef, LikeStatus, PlaylistId, Track, TrackId, VideoType,
};
use featherplay_ytm::{AlbumSummary, ArtistSummary, HomeItem, PlaylistSummary};

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
fn to_rows_unicode_long_title_ok() {
    // Right-to-left Arabic, an emoji with a skin tone and a ZWJ family, repeated past 300 chars.
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

fn community(avatar: Option<&str>) -> HomeItem {
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
}

#[test]
fn a_community_playlist_keeps_its_collage_and_wears_its_creators_avatar() {
    let item = community(Some("https://img/avatar"));
    let card = home_card(&item);
    assert_eq!(card.subtitle.as_str(), "Creator, 529K views");
    assert_eq!(
        home_thumbnail(&item).as_deref(),
        Some("https://img/collage")
    );
    assert_eq!(home_badge(&item).as_deref(), Some("https://img/avatar"));
}

#[test]
fn a_community_playlist_without_an_avatar_has_no_badge() {
    let item = community(None);
    assert_eq!(home_card(&item).subtitle.as_str(), "Creator, 529K views");
    assert_eq!(home_badge(&item), None);
}
