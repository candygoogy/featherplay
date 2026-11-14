//! A live check against LRCLIB: `cargo test -p featherplay-lyrics -- --ignored`.
//! One lookup, at most two requests (`get`, then `search` if that finds nothing).

use std::time::Duration;

use featherplay_lyrics::Lyrics;
use featherplay_lyrics::enrich::Client as EnrichmentClient;
use featherplay_lyrics::lrclib::{Lookup, LrcLib, Query};

#[tokio::test]
#[ignore = "live: asks lrclib.net"]
async fn live_lrclib_finds_synced_lyrics() {
    let lrclib = LrcLib::new().expect("client");
    let found = lrclib
        .find(&Query {
            title: "Around the World".into(),
            artist: "Daft Punk".into(),
            album: Some("Homework".into()),
            duration: Some(Duration::from_secs(429)),
        })
        .await
        .unwrap_or_else(|e| panic!("lookup: {e}"));
    match found {
        Lookup::Found {
            lyrics: Lyrics::Synced(lines),
            via,
            ..
        } => println!("synced: {} lines ({via:?})", lines.len()),
        other => panic!("synced lyrics expected, got {other:?}"),
    }
}

/// Two requests, one for each button. The source lines are short and made for this test.
#[tokio::test]
#[ignore = "live: sends sample lines to Google Translate"]
async fn live_google_translates_and_romanizes_a_batch() {
    let client = EnrichmentClient::new().expect("client");
    let lines = vec!["今ここで歌っている".into(), "七色にきらめいて".into()];
    let translated = client
        .translate(&lines, "en")
        .await
        .unwrap_or_else(|e| panic!("translate: {e}"));
    let romanized = client
        .romanize(&lines)
        .await
        .unwrap_or_else(|e| panic!("romanize: {e}"));
    assert_eq!(translated.lines.len(), lines.len());
    assert_eq!(romanized.lines.len(), lines.len());
    assert!(translated.lines.iter().all(Option::is_some));
    assert!(romanized.lines.iter().all(Option::is_some));
    println!("translation: {:?}", translated.lines);
    println!("romanization: {:?}", romanized.lines);
}
