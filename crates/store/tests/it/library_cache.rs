//! The Library's on-disk cache: the last good list of each tab, per identity.

use std::fs;

use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};
use featherplay_store::{LibraryCache, LibraryList};

use crate::home::TestHome;

impl TestHome {
    fn cache(&self) -> LibraryCache {
        LibraryCache::new(&self.paths())
    }
}

fn track(id: &str) -> Track {
    Track {
        id: TrackId(id.into()),
        title: format!("Title {id}"),
        artists: Vec::new(),
        album: None,
        duration: None,
        video_type: VideoType::Song,
        thumbnail: Some(format!("https://example.invalid/{id}.jpg")),
        like: LikeStatus::Like,
        counterpart: None,
    }
}

#[test]
fn a_saved_list_loads_back_unchanged() {
    let Some(home) = TestHome::new("library-round-trip") else {
        return;
    };
    let cache = home.cache();
    let liked = vec![track("a"), track("b")];
    cache
        .save("", LibraryList::Liked, &liked)
        .expect("save liked");
    let back: Option<Vec<Track>> = cache.load("", LibraryList::Liked);
    assert_eq!(back, Some(liked));
}

#[test]
fn each_identity_keeps_its_own_lists() {
    let Some(home) = TestHome::new("library-identities") else {
        return;
    };
    let cache = home.cache();
    let own = vec![track("own")];
    let brand = vec![track("brand")];
    cache.save("", LibraryList::Liked, &own).expect("save own");
    cache
        .save("0/100000000000000000002", LibraryList::Liked, &brand)
        .expect("save brand");

    let read = |identity: &str| cache.load::<Vec<Track>>(identity, LibraryList::Liked);
    assert_eq!(read(""), Some(own));
    assert_eq!(read("0/100000000000000000002"), Some(brand));
    // An identity that never saved anything has nothing, and so has another list.
    assert_eq!(read("1"), None);
    assert_eq!(cache.load::<Vec<Track>>("", LibraryList::Albums), None);
}

#[test]
fn clearing_forgets_every_identity() {
    let Some(home) = TestHome::new("library-clear") else {
        return;
    };
    let cache = home.cache();
    cache
        .save("", LibraryList::Playlists, &vec![track("a")])
        .expect("save");
    cache
        .save("2", LibraryList::Artists, &vec![track("b")])
        .expect("save");
    cache.clear().expect("clear");
    assert_eq!(cache.load::<Vec<Track>>("", LibraryList::Playlists), None);
    assert_eq!(cache.load::<Vec<Track>>("2", LibraryList::Artists), None);
    // Clearing an empty cache is fine too.
    cache.clear().expect("clear again");
}

#[test]
fn a_damaged_file_reads_as_nothing() {
    let Some(home) = TestHome::new("library-damaged") else {
        return;
    };
    let cache = home.cache();
    cache
        .save("", LibraryList::Liked, &vec![track("a")])
        .expect("save");
    let file = fs::read_dir(home.0.join("cache/library"))
        .expect("library dir")
        .flatten()
        .flat_map(|d| fs::read_dir(d.path()).expect("identity dir").flatten())
        .map(|f| f.path())
        .next()
        .expect("a cache file");
    fs::write(&file, b"{ not json").expect("damage");
    assert_eq!(cache.load::<Vec<Track>>("", LibraryList::Liked), None);
}

#[test]
fn forgetting_a_list_drops_it_for_every_identity_and_keeps_the_others() {
    let Some(home) = TestHome::new("library-forget-list") else {
        return;
    };
    let cache = home.cache();
    for identity in ["", "2"] {
        cache
            .save(identity, LibraryList::Home, &vec![track("feed")])
            .expect("save home");
    }
    cache
        .save("", LibraryList::Liked, &vec![track("a")])
        .expect("save liked");
    cache.forget(LibraryList::Home).expect("forget");
    assert_eq!(cache.load::<Vec<Track>>("", LibraryList::Home), None);
    assert_eq!(cache.load::<Vec<Track>>("2", LibraryList::Home), None);
    assert_eq!(
        cache.load::<Vec<Track>>("", LibraryList::Liked),
        Some(vec![track("a")])
    );
    // Nothing left to forget is fine too, and so is a cache never made.
    cache.forget(LibraryList::Home).expect("forget again");
    cache.clear().expect("clear");
    cache
        .forget(LibraryList::Home)
        .expect("forget with no cache");
}
