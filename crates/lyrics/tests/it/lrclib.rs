//! LRCLIB's JSON, from recorded responses: one `/api/get` record and the `/api/search`
//! answer for "Don't Stop The Music" by Rihanna (2026-10-30). Their lyrics were replaced
//! with placeholder lines before they were stored; the shape and metadata are LRCLIB's.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::Duration;

use featherplay_lyrics::Lyrics;
use featherplay_lyrics::lrclib::{
    Lookup, LrcLib, Query, Via, best_match, parse_record, parse_search, same_artist, same_title,
};

const GET: &str = include_str!("../fixtures/lrclib_get.json");
const SEARCH_DONT_STOP: &str = include_str!("../fixtures/lrclib_search_dont_stop.json");

fn query(title: &str, artist: &str, seconds: Option<u64>) -> Query {
    Query {
        title: title.into(),
        artist: artist.into(),
        album: None,
        duration: seconds.map(Duration::from_secs),
    }
}

/// The track as YouTube Music gives it: its album isn't any of the search results' albums
/// that are 267 s long, and the stream is 267.4 s.
fn dont_stop() -> Query {
    Query {
        title: "Don't Stop The Music".into(),
        artist: "Rihanna".into(),
        album: Some("Good Girl Gone Bad: Reloaded".into()),
        duration: Some(Duration::from_millis(267_400)),
    }
}

#[test]
fn lrclib_records_should_give_synced_lines_and_search_should_pick_the_closest_length() {
    let record = parse_record(GET).expect("the recorded record parses");
    let Some(Lyrics::Synced(lines)) = record.lyrics() else {
        panic!("synced lyrics expected, got {:?}", record.lyrics());
    };
    assert_eq!(lines.len(), 24);
    assert_eq!(lines[0].start, Duration::from_millis(12_400));
    assert_eq!(lines[0].text, "Morning light on the harbour wall");
    assert_eq!(lines[4].text, "");

    // `/api/search` answers with an array of the same records. One a minute too long, one
    // without synced lyrics at the right length, and the recorded one: the synced record
    // of the right length wins, and nothing matches a length far from all of them.
    let mut long: serde_json::Value = serde_json::from_str(GET).expect("json");
    long["duration"] = 427.0.into();
    let mut plain = long.clone();
    plain["duration"] = 368.0.into();
    plain["syncedLyrics"] = serde_json::Value::Null;
    // Some records carry `null` where a flag is expected.
    plain["instrumental"] = serde_json::Value::Null;
    let search = format!("[{long}, {plain}, {GET}]");
    let found = parse_search(&search).expect("the search parses");
    assert_eq!(found.len(), 3);
    let get_lucky = |s| query("Get Lucky", "Daft Punk", Some(s));
    let best = best_match(found.clone(), &get_lucky(369)).expect("a match");
    assert!(matches!(best.lyrics(), Some(Lyrics::Synced(_))));
    assert_eq!(best.duration, Some(367.0));
    assert!(best_match(found, &get_lucky(200)).is_none());
}

#[test]
fn search_should_find_synced_lyrics_for_dont_stop_the_music_whatever_the_album() {
    let found = parse_search(SEARCH_DONT_STOP).expect("the recorded search parses");
    assert_eq!(found.len(), 20);
    let best = best_match(found, &dont_stop()).expect("a match");
    assert!(matches!(best.lyrics(), Some(Lyrics::Synced(_))));
    let off = (best.duration.expect("a length") - 267.4).abs();
    assert!(off <= 3.0, "{off} s from the track's length");
}

#[test]
fn search_should_never_pick_another_song_of_the_same_length() {
    // LRCLIB's search is loose: put another artist's song, synced and exactly as long as
    // the track, in front of the recorded results.
    let mut other: serde_json::Value = serde_json::from_str(GET).expect("json");
    other["trackName"] = "Don't Stop Me Now".into();
    other["artistName"] = "Queen".into();
    other["duration"] = 267.4.into();
    let mut stranger = other.clone();
    stranger["trackName"] = "Don't Stop The Music".into();
    stranger["artistName"] = "Jamie Cullum".into();
    let recorded = SEARCH_DONT_STOP.trim().trim_start_matches('[');
    let search = format!("[{other}, {stranger}, {recorded}");
    let found = parse_search(&search).expect("the search parses");
    let best = best_match(found, &dont_stop()).expect("a match");
    let artist = best.artist_name.unwrap_or_default().to_lowercase();
    assert!(artist.contains("rihanna"), "picked {artist}");
}

#[test]
fn titles_should_match_across_case_punctuation_apostrophes_and_feat_parts() {
    assert!(same_title("Don't Stop The Music", "Dont Stop the Music"));
    assert!(same_title(
        "Don't Stop The Music",
        "don\u{2019}t stop the music"
    ));
    assert!(same_title("Señorita", "SEÑORITA!"));
    assert!(same_title("Stay (feat. Justin Bieber)", "Stay"));
    assert!(same_title("Stay [ft. Justin Bieber]", "Stay"));
    assert!(same_title("Stay feat. Justin Bieber", "Stay"));
    assert!(!same_title("Don't Stop The Music", "Don't Stop Me Now"));
    assert!(!same_title("Stay", "Stay With Me"));
}

#[test]
fn artists_should_match_duplicated_and_joined_names() {
    assert!(same_artist("Rihanna", "Rihanna/Rihanna"));
    assert!(same_artist("Rihanna", "rihanna rihanna"));
    assert!(same_artist("Rihanna", "Rihanna feat. Jay-Z"));
    assert!(same_artist("Rihanna", "Rihanna, Jay-Z"));
    assert!(same_artist("Rihanna", "Rihanna & Jay-Z"));
    assert!(same_artist("Beyoncé", "BEYONCE"));
    assert!(same_artist("Simon & Garfunkel", "Simon & Garfunkel"));
    assert!(!same_artist("Rihanna", "Jamie Cullum"));
}

/// A stand-in for lrclib.net on a local port: `get` answers with `get_status` (and an
/// empty record), `search` with `search_body`. Serves `requests` requests, then stops.
fn stand_in(get_status: u16, search_body: &'static str, requests: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
    let base = format!("http://{}/api/", listener.local_addr().expect("address"));
    std::thread::spawn(move || {
        for stream in listener.incoming().take(requests) {
            let Ok(mut stream) = stream else { continue };
            let mut line = String::new();
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            reader.read_line(&mut line).expect("request line");
            // Read the headers up to the blank line.
            let mut header = String::new();
            while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                header.clear();
            }
            let (status, body) = if line.starts_with("GET /api/get") {
                (get_status, "{}")
            } else {
                (200, search_body)
            };
            let reply = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    base
}

#[tokio::test]
async fn a_failed_exact_lookup_should_still_search() {
    let base = stand_in(503, SEARCH_DONT_STOP, 2);
    let lrclib = LrcLib::with_base(&base).expect("client");
    let found = lrclib.find(&dont_stop()).await.expect("the search answers");
    let Lookup::Found { lyrics, via } = found else {
        panic!("lyrics expected, got {found:?}");
    };
    assert!(matches!(lyrics, Lyrics::Synced(_)));
    assert_eq!(via, Via::Search { results: 20 });
}

#[tokio::test]
async fn a_search_without_a_match_should_be_a_miss() {
    let base = stand_in(404, "[]", 2);
    let lrclib = LrcLib::with_base(&base).expect("client");
    let found = lrclib.find(&dont_stop()).await.expect("both answer");
    assert_eq!(found, Lookup::Missed { results: 0 });
}

#[tokio::test]
async fn a_failed_exact_lookup_and_an_empty_search_should_be_an_error() {
    // The exact record may exist: the miss isn't certain, so the caller asks again later.
    let base = stand_in(503, "[]", 2);
    let lrclib = LrcLib::with_base(&base).expect("client");
    assert!(lrclib.find(&dont_stop()).await.is_err());
}
