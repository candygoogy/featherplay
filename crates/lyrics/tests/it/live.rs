//! A live check against LRCLIB: `cargo test -p featherplay-lyrics -- --ignored`.
//! One lookup, at most two requests (`get`, then `search` if that finds nothing).

use std::time::Duration;

use featherplay_lyrics::Lyrics;
use featherplay_lyrics::enrich::Client as EnrichmentClient;
use featherplay_lyrics::lrclib::{Lookup, LrcLib, Query};
use featherplay_lyrics::lyrictified::{
    Lookup as LyrictifiedLookup, Lyrictified, Query as LyrictifiedQuery,
};
use featherplay_lyrics::unison::{Lookup as UnisonLookup, Unison};

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

/// Two requests: a search and the file, for a song Lyrictified holds timed by the word
/// ("Ex-Otogibanashi", whose file names its artists as QQ Music does). A title the server has
/// not seen answers nothing while it looks the song up, so this needs a song it has.
#[tokio::test]
#[ignore = "live: asks api.lyrictified.xyz"]
async fn live_lyrictified_finds_lyrics() {
    let lyrictified = Lyrictified::new().expect("client");
    let found = lyrictified
        .find(&LyrictifiedQuery {
            song: "Ex-Otogibanashi".into(),
            artists: vec![
                "ryo (supercell)".into(),
                "Kaguya(cv.Yuko Natsuyoshi)".into(),
            ],
        })
        .await
        .unwrap_or_else(|e| panic!("lookup: {e}"));
    match found {
        LyrictifiedLookup::Found {
            ref lines,
            ref id,
            ref format,
        } => {
            println!("{found}: file {id} ({format}), {} lines", lines.len());
            assert!(lines.len() > 10);
            assert!(lines.windows(2).all(|w| w[0].start <= w[1].start));
            assert!(lines.iter().any(|l| !l.parts.is_empty()), "no word timing");
            println!(
                "first line: {:?}",
                lines.iter().find(|l| !l.text.is_empty())
            );
        }
        other => panic!("lyrics expected, got {other:?}"),
    }
}

/// One request for a song Unison holds word-timed lyrics for ("Harvey", Her's).
#[tokio::test]
#[ignore = "live: asks unison.betterlyrics.org"]
async fn live_unison_finds_word_timed_lyrics() {
    let unison = Unison::new().expect("client");
    let found = unison
        .find("T0Ry1m0Blug")
        .await
        .unwrap_or_else(|e| panic!("lookup: {e}"));
    match found {
        UnisonLookup::Words { lines, id, .. } => {
            let words: usize = lines.iter().map(|l| l.parts.len()).sum();
            println!(
                "word-timed: entry {id}, {} lines, {words} pieces",
                lines.len()
            );
            assert!(lines.len() > 20 && words > 100);
            assert!(lines.windows(2).all(|w| w[0].start <= w[1].start));
            println!(
                "first line: {:?} {:?}",
                lines[0].text,
                lines[0].parts.first()
            );
        }
        other => panic!("word-timed lyrics expected, got {other:?}"),
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
