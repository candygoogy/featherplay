//! Lyrictified and Unison against a local stand-in: what each lookup asks, and what it makes
//! of the answers. The answers' shapes are the servers' (2026-11-26); their lyrics are made up.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;

use featherplay_lyrics::lyrictified::{Lookup, Lyrictified, Query};

/// A server at the returned base that answers each request with `answer(request line)` (a
/// status and a body) and sends each request line down the channel.
fn stand_in(answer: fn(&str) -> (u16, String)) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
    let base = format!("http://{}/", listener.local_addr().expect("address"));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut line = String::new();
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            reader.read_line(&mut line).expect("request line");
            let mut header = String::new();
            while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                header.clear();
            }
            let (status, body) = answer(&line);
            // Told before the answer goes, so a lookup that has its answers was seen asking.
            if tx.send(line.trim().to_string()).is_err() {
                break;
            }
            let reply = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (base, rx)
}

/// A file timed by the word, as Lyrictified serves one.
const TTML: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div><p begin="0:05.000" end="0:07.000"><span begin="0:05.000" end="0:06.000">Made</span> <span begin="0:06.000" end="0:07.000">up</span></p></div></body></tt>"#;

/// Lyrictified: its one file for "Ex-Otogibanashi", for a search by that title only (the
/// server needs each word of the search in the file's title, artist or tags).
fn lyrictified(line: &str) -> (u16, String) {
    if line.starts_with("GET /search?song=Ex-Otogibanashi&") {
        let hit = r#"{"results":[{"id":"baac4586a8478a9f","title":"Ex-Otogibanashi","artist":"ryo, 夏吉ゆうこ, 早見沙織","album":"Album","format":"ttml","score":400}]}"#;
        (200, hit.into())
    } else if line.starts_with("GET /search?") {
        (200, r#"{"results":[]}"#.into())
    } else if line.starts_with("GET /lyrics/baac4586a8478a9f/raw") {
        (200, TTML.into())
    } else {
        (404, "{}".into())
    }
}

#[tokio::test]
async fn lyrictified_should_find_a_song_youtube_music_titles_with_its_guest() {
    let (base, asked) = stand_in(lyrictified);
    let client = Lyrictified::with_base(&base).expect("client");
    // YouTube Music's title and artists for the song.
    let found = client
        .find(&Query {
            song: "Ex-Otogibanashi (feat. Kaguya(cv.Yuko Natsuyoshi))".into(),
            artists: vec!["ryo (supercell) & Yachiyo Runami(cv.Saori Hayami)".into()],
        })
        .await
        .expect("an answer");
    assert!(
        matches!(&found, Lookup::Found { lines, .. } if lines[0].parts.len() == 2),
        "{found:?}"
    );
    let asked: Vec<String> = asked.try_iter().collect();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(asked[0].starts_with("GET /search?song=Ex-Otogibanashi&limit=20 "));

    // A title in two languages: a search by each half, the second when the first finds none.
    let (base, asked) = stand_in(lyrictified);
    let client = Lyrictified::with_base(&base).expect("client");
    let found = client
        .find(&Query {
            song: "エクス・オトギバナシ - Ex-Otogibanashi (feat. ChoKaguyaHime)".into(),
            artists: vec!["ryo (supercell)".into()],
        })
        .await
        .expect("an answer");
    assert!(matches!(found, Lookup::Found { .. }), "{found:?}");
    let asked: Vec<String> = asked.try_iter().collect();
    assert_eq!(asked.len(), 3, "{asked:?}");
    assert!(asked[1].starts_with("GET /search?song=Ex-Otogibanashi&"));
}
