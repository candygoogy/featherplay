//! "Playing from …" in the Up next panel, derived from the queue a play started.

use std::time::Duration;

use featherplay::controller::{playing_from, queue_info};
use featherplay_core::model::{AlbumId, AlbumRef, LikeStatus, ListEnd, Track, TrackId, VideoType};
use featherplay_core::queue::Queue;

fn track(id: &str, album: Option<(&str, &str)>) -> Track {
    Track {
        id: TrackId(id.into()),
        title: format!("Title {id}"),
        artists: Vec::new(),
        album: album.map(|(id, title)| AlbumRef {
            id: AlbumId(id.into()),
            title: title.into(),
        }),
        duration: Some(Duration::from_secs(200)),
        video_type: VideoType::Song,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        song_version: None,
    }
}

#[test]
fn radios_albums_and_unknown_queues() {
    let hw = Some(("MPREb_hw", "Homework"));
    let album = vec![track("a", hw), track("b", hw), track("c", hw)];
    assert_eq!(playing_from(&album, 1, false), "Homework");
    // A radio is named after the track it started from, wherever that is in the list.
    assert_eq!(playing_from(&album, 1, true), "Title b radio");

    // Tracks from several albums (a playlist, liked songs) need the page's own label.
    let mixed = vec![track("a", hw), track("x", Some(("MPREb_d", "Discovery")))];
    assert_eq!(playing_from(&mixed, 0, false), "");
    let no_album = vec![track("a", hw), track("v", None)];
    assert_eq!(playing_from(&no_album, 0, false), "");
    assert_eq!(playing_from(&[], 0, false), "");
    assert_eq!(playing_from(&[], 0, true), "");
}

fn list(ids: &[&str]) -> Vec<Track> {
    ids.iter().map(|id| track(id, None)).collect()
}

/// A list of `ids` playing its last track, with "x" and "y" autoplayed after it.
fn autoplaying(ids: &[&str]) -> Queue {
    let mut q = Queue::new();
    q.set_list_end(ListEnd::Autoplay);
    q.replace(list(ids), ids.len() - 1, false);
    q.append_continuation(list(&["x", "y"]));
    q
}

#[test]
fn a_list_keeps_its_name_until_autoplay_plays() {
    let q = autoplaying(&["a", "b"]);
    let info = queue_info(&q, "Homework");
    assert_eq!(
        (info.playing_from.as_str(), info.autoplay_at),
        ("Homework", Some(0))
    );
}

#[test]
fn autoplay_names_itself_once_it_plays() {
    let mut q = autoplaying(&["a", "b"]);
    q.advance();
    let info = queue_info(&q, "Homework");
    assert_eq!(
        (info.playing_from.as_str(), info.autoplay_at),
        ("Autoplay", None)
    );
}

#[test]
fn a_radio_keeps_its_name_in_its_continuation() {
    let mut q = Queue::new();
    q.replace(list(&["a"]), 0, true);
    q.append_continuation(list(&["x", "y"]));
    q.advance();
    assert_eq!(
        queue_info(&q, "Title a radio").playing_from,
        "Title a radio"
    );
}
