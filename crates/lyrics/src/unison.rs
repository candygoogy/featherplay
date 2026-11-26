//! Unison (https://unison.betterlyrics.org): the community-timed lyrics of Better Lyrics, found
//! by the song's YouTube video ID. featherplay asks it only for word-timed lyrics, which LRCLIB
//! rarely has (`words` reads them: TTML with a timed `<span>` for each word or syllable of a
//! line, or LRC with a `<mm:ss.xx>` stamp before each one). Whatever else it holds for the song
//! (lyrics timed by the line, or plain) is left to LRCLIB.
//!
//! Written from the formats and the one public endpoint (`GET /lyrics?v=<video id>`, no login),
//! not from Better Lyrics' code, which is GPL-3.0.
//!
//! Every request is sent once, with a `User-Agent` naming featherplay, no cookies, no
//! identity, redirects only within betterlyrics.org, a timeout and a cap on the body.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;

use crate::words::{self, WordLine};

const BASE: &str = "https://unison.betterlyrics.org/";
/// Short: its answer is taken before LRCLIB's, so a slow one mustn't hold the lyrics up.
const TIMEOUT: Duration = Duration::from_secs(6);
/// A word-timed song is a few tens of kB of TTML.
const MAX_BODY: usize = 2 * 1024 * 1024;
/// The most characters of the name of who timed the lyrics that are shown (it's anyone's own).
const MAX_SUBMITTER_CHARS: usize = 40;

/// Who timed the lyrics, as shown in the credit: no control characters, spaces collapsed, at
/// most `MAX_SUBMITTER_CHARS` (an ellipsis past them); `None` when nothing is left.
fn submitter_name(name: &str) -> Option<String> {
    let clean: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect();
    let clean = clean.join(" ");
    if clean.is_empty() {
        return None;
    }
    if clean.chars().count() <= MAX_SUBMITTER_CHARS {
        return Some(clean);
    }
    let cut: String = clean.chars().take(MAX_SUBMITTER_CHARS - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

crate::http::server_error!("Unison");

/// What a lookup came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// Word-timed lyrics, Unison's entry `id`, and who timed them when it says.
    Words {
        lines: Vec<WordLine>,
        id: u64,
        submitter: Option<String>,
    },
    /// Nothing for the video, or nothing timed by the word.
    Missed,
}

impl fmt::Display for Lookup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Lookup::Words { lines, .. } => {
                let words = lines.iter().filter(|l| !l.parts.is_empty()).count();
                write!(f, "word-timed, {words} of {} lines", lines.len())
            }
            Lookup::Missed => f.write_str("no word-timed lyrics"),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Reply {
    data: Option<Entry>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Entry {
    id: u64,
    lyrics: String,
    /// `ttml`, `lrc` or `plain`.
    format: String,
    hidden: bool,
    submitter: Option<Submitter>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Submitter {
    display_name: Option<String>,
}

/// What Unison's answer comes to: its entry's word-timed lyrics, if it has any.
pub fn parse_reply(json: &str) -> Result<Lookup, Error> {
    let reply: Reply = serde_json::from_str(json).map_err(|e| Error::Parse(e.to_string()))?;
    let Some(entry) = reply.data.filter(|e| !e.hidden) else {
        return Ok(Lookup::Missed);
    };
    let lines = match entry.format.as_str() {
        "ttml" => words::parse_ttml(&entry.lyrics),
        "lrc" => words::parse_enhanced_lrc(&entry.lyrics),
        _ => None,
    };
    Ok(match lines {
        Some(lines) => Lookup::Words {
            lines,
            id: entry.id,
            submitter: entry
                .submitter
                .and_then(|s| s.display_name)
                .and_then(|n| submitter_name(&n)),
        },
        None => Lookup::Missed,
    })
}

/// Whether `id` looks like a YouTube video ID: what the lookup is allowed to send.
fn video_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The Unison client. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Unison {
    client: reqwest::Client,
    base: String,
}

impl Unison {
    pub fn new() -> Result<Unison, Error> {
        Unison::with_base(BASE)
    }

    /// A client of a Unison at `base` (`https://…/`): a local stand-in in tests.
    pub fn with_base(base: &str) -> Result<Unison, Error> {
        let client = crate::http::client("betterlyrics.org", TIMEOUT).map_err(Error::Network)?;
        Ok(Unison {
            client,
            base: base.to_string(),
        })
    }

    /// The word-timed lyrics Unison holds for the YouTube video `id`. One request. A video
    /// Unison has nothing for (a 404), or whose lyrics aren't timed by the word, is `Missed`.
    pub async fn find(&self, id: &str) -> Result<Lookup, Error> {
        if !video_id(id) {
            return Ok(Lookup::Missed);
        }
        match crate::http::get(&self.client, &self.base, "lyrics", &[("v", id)], MAX_BODY).await? {
            Some(body) => parse_reply(&body),
            None => Ok(Lookup::Missed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTML: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div><p begin="0:31.725" end="0:34.415"><span begin="0:31.725" end="0:31.957">Made</span> <span begin="0:31.957" end="0:32.188">for</span><span begin="0:32.188" end="0:32.438">ty</span></p><p begin="0:10.000">An untimed line</p></div></body></tt>"#;

    #[test]
    fn the_reply_should_give_word_timed_lyrics_and_who_timed_them() {
        let body = serde_json::json!({
            "success": true,
            "data": {
                "id": 17,
                "format": "ttml",
                "lyrics": TTML,
                "submitter": { "displayName": " A  Tester\u{7} " },
            }
        })
        .to_string();
        let Lookup::Words {
            lines,
            id,
            submitter,
        } = parse_reply(&body).expect("a reply")
        else {
            panic!("missed");
        };
        assert_eq!((lines.len(), id), (2, 17));
        assert_eq!(submitter.as_deref(), Some("A Tester"));
        assert_eq!(
            submitter_name(&"x".repeat(60)).map(|n| n.chars().count()),
            Some(40)
        );
        assert_eq!(submitter_name(" \t "), None);
    }

    #[test]
    fn the_reply_should_miss_for_line_timed_plain_hidden_or_no_lyrics() {
        let entry = |format: &str, lyrics: &str, hidden: bool| {
            serde_json::json!({ "data": { "id": 1, "format": format, "lyrics": lyrics, "hidden": hidden } })
                .to_string()
        };
        for body in [
            entry("plain", "Just words", false),
            entry("lrc", "[00:01.00]One", false),
            entry("ttml", TTML, true),
            entry("ttml", "not xml", false),
            r#"{"success":true,"data":null}"#.to_string(),
            "{}".to_string(),
        ] {
            assert_eq!(
                parse_reply(&body).expect("a reply"),
                Lookup::Missed,
                "{body}"
            );
        }
        assert!(parse_reply("not json").is_err());
    }

    #[test]
    fn only_video_ids_should_be_sent() {
        assert!(video_id("T0Ry1m0Blug"));
        assert!(video_id("R-hYM3BqTbA"));
        for bad in ["", "a b", "a/b", "a?b=c", "demo track", &"x".repeat(33)] {
            assert!(!video_id(bad), "{bad:?}");
        }
    }
}
