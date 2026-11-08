//! Skip silence's marks, remembered per song: `{cache}/silences.json`.

use featherplay_store::silence_cache::{CAP, SilenceCache, SongSilences};

use crate::home::TestHome;

fn silences(n: u64) -> SongSilences {
    SongSilences {
        rate: 48_000,
        frames: 11_520_000 + n,
        runs: vec![(0, 48_000), (4_800_000, 144_000 + n)],
    }
}

#[test]
fn saved_silences_load_back_by_video_id() {
    let Some(home) = TestHome::new("silences-round-trip") else {
        return;
    };
    let mut cache = SilenceCache::load(&home.paths());
    cache.insert("vid-a", silences(1));
    cache.insert("vid-b", silences(2));
    cache.save().expect("save");
    let back = SilenceCache::load(&home.paths());
    assert_eq!(back.get("vid-a"), Some(&silences(1)));
    assert_eq!(back.get("vid-b"), Some(&silences(2)));
    assert_eq!(back.get("vid-c"), None);
}

#[test]
fn the_oldest_song_goes_past_the_cap() {
    let Some(home) = TestHome::new("silences-cap") else {
        return;
    };
    let mut cache = SilenceCache::load(&home.paths());
    for i in 0..CAP as u64 {
        cache.insert(&format!("vid-{i}"), silences(i));
    }
    // Scanned again (another stream of it): the newest now, so it stays.
    cache.insert("vid-0", silences(0));
    cache.insert("one-more", silences(9));
    cache.save().expect("save");
    let back = SilenceCache::load(&home.paths());
    assert_eq!(back.get("vid-1"), None, "the oldest went");
    assert_eq!(back.get("vid-0"), Some(&silences(0)));
    assert_eq!(back.get("vid-2"), Some(&silences(2)));
    assert_eq!(back.get("one-more"), Some(&silences(9)));
}

#[test]
fn a_damaged_file_reads_as_empty() {
    let Some(home) = TestHome::new("silences-damaged") else {
        return;
    };
    let paths = home.paths();
    std::fs::create_dir_all(&paths.cache).expect("cache dir");
    std::fs::write(paths.cache.join("silences.json"), b"{\"songs\": [1, 2").expect("write");
    let mut cache = SilenceCache::load(&paths);
    assert_eq!(cache.get("vid-a"), None);
    cache.insert("vid-a", silences(1));
    cache.save().expect("save over it");
    assert_eq!(SilenceCache::load(&paths).get("vid-a"), Some(&silences(1)));
}
