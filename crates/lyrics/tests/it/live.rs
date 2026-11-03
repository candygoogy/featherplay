//! A live check against LRCLIB: `cargo test -p featherplay-lyrics -- --ignored`.
//! One lookup, at most two requests (`get`, then `search` if that finds nothing).

use std::time::Duration;

use featherplay_lyrics::Lyrics;
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
        } => println!("synced: {} lines ({via:?})", lines.len()),
        other => panic!("synced lyrics expected, got {other:?}"),
    }
}
