//! LRCLIB (https://lrclib.net): free, no key. `/api/get` matches the track by title,
//! artist, album and length; `/api/search` is the fallback when that finds nothing or
//! fails, matched here by title, artist and length (see `best_match`).
//!
//! Every request is sent once, with a `User-Agent` naming featherplay (LRCLIB asks for
//! one), no cookies, redirects only within lrclib.net, a timeout and a cap on the body.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;

use crate::lrc::{self, Lyrics};

const BASE: &str = "https://lrclib.net/api/";
const TIMEOUT: Duration = Duration::from_secs(10);
/// A record is a few kB; a search answers with up to 20 of them.
const MAX_BODY: usize = 1024 * 1024;
/// How far a record's length may be from the track's to count as the same recording.
const LENGTH_SLACK_S: f64 = 3.0;

/// One LRCLIB record (`/api/get` returns one, `/api/search` an array of them).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Record {
    /// LRCLIB's id for the record.
    pub id: Option<u64>,
    pub track_name: Option<String>,
    /// May join or repeat names: "A, B", "A/A".
    pub artist_name: Option<String>,
    pub album_name: Option<String>,
    /// Seconds.
    pub duration: Option<f64>,
    /// `null` in some records.
    pub instrumental: Option<bool>,
    pub plain_lyrics: Option<String>,
    pub synced_lyrics: Option<String>,
}

impl Record {
    /// Synced lines when the record has them, else its plain text. `None` for an
    /// instrumental or an empty record.
    pub fn lyrics(&self) -> Option<Lyrics> {
        if self.instrumental == Some(true) {
            return None;
        }
        if let Some(lines) = self.synced_lyrics.as_deref().and_then(lrc::parse) {
            return Some(Lyrics::Synced(lines));
        }
        self.plain_lyrics
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| Lyrics::Plain(t.to_string()))
    }

    /// The first line of its words, for telling records apart: the first timed line with
    /// text, or the plain text's first line. `None` for a record without words.
    ///
    /// Read off the text without parsing all of it (`lrc::first_words`): the dialog that
    /// lists records shows one for each.
    pub fn first_line(&self) -> Option<String> {
        if self.instrumental == Some(true) {
            return None;
        }
        if let Some(synced) = self.synced_text() {
            return lrc::first_words(synced).map(str::to_string);
        }
        self.plain_lyrics
            .as_deref()?
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(str::to_string)
    }

    /// Whether `lyrics` gives any, without parsing them.
    fn has_lyrics(&self) -> bool {
        self.instrumental != Some(true)
            && (self.synced_text().is_some()
                || self
                    .plain_lyrics
                    .as_deref()
                    .is_some_and(|t| !t.trim().is_empty()))
    }

    /// The synced lyrics, when `lyrics` would take them: some line has a time stamp.
    fn synced_text(&self) -> Option<&str> {
        self.synced_lyrics
            .as_deref()
            .filter(|s| lrc::has_timed_line(s))
    }

    /// Whether its lyrics are synced, as `lyrics` would give them.
    pub fn synced(&self) -> bool {
        self.synced_lyrics
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
    }
}

/// What a lookup is matched on.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration: Option<Duration>,
}

crate::http::server_error!("LRCLIB");

pub fn parse_record(json: &str) -> Result<Record, Error> {
    serde_json::from_str(json).map_err(|e| Error::Parse(e.to_string()))
}

pub fn parse_search(json: &str) -> Result<Vec<Record>, Error> {
    serde_json::from_str(json).map_err(|e| Error::Parse(e.to_string()))
}

/// The record that best fits the track: the same title and artist (see `same_title` and
/// `same_artist`; the album is ignored, since compilations repeat a recording under many),
/// lyrics, and a length within a few seconds of the track's (any length when it's
/// unknown). Synced before plain, then the closest length.
pub fn best_match(records: Vec<Record>, q: &Query) -> Option<Record> {
    let want = q.duration.map(|d| d.as_secs_f64());
    let off = |r: &Record| match (want, r.duration) {
        (Some(w), Some(d)) => (w - d).abs(),
        _ => 0.0,
    };
    // The track's side of `same_title` and `same_artist`, once for all the records.
    let title = normalise(&q.title);
    let artists = artist_names(&q.artist);
    let same = |r: &Record| {
        !title.is_empty()
            && r.track_name
                .as_deref()
                .is_some_and(|t| normalise(t) == title)
            && r.artist_name
                .as_deref()
                .is_some_and(|a| artist_names(a).iter().any(|n| artists.contains(n)))
    };
    records
        .into_iter()
        .filter(|r| same(r) && off(r) <= LENGTH_SLACK_S && r.has_lyrics())
        .min_by(|a, b| {
            (!a.synced())
                .cmp(&!b.synced())
                .then(off(a).total_cmp(&off(b)))
        })
}

/// Two titles name the same song: equal once `normalise`d, which drops case, accents,
/// apostrophes, punctuation and any "feat." part.
pub fn same_title(a: &str, b: &str) -> bool {
    let a = normalise(a);
    !a.is_empty() && a == normalise(b)
}

/// The track's artist `want` is among a record's `artists`. LRCLIB's artist field may join
/// several ("A, B", "A & B", "A/B", "A feat. B") or repeat one ("A/A", "a a").
pub fn same_artist(want: &str, artists: &str) -> bool {
    let wanted = artist_names(want);
    artist_names(artists).iter().any(|n| wanted.contains(n))
}

/// Every name an artist field may stand for: the whole of it, the whole without a
/// repetition, and each of its joined parts.
fn artist_names(field: &str) -> Vec<String> {
    let whole = normalise(field);
    let mut names = vec![undouble(&whole).to_string(), whole.clone()];
    names.extend(field.split(['/', ',', ';', '&']).map(normalise));
    names.retain(|n| !n.is_empty());
    names
}

/// "a b a b" → "a b": the first half of a name made of the same words twice.
fn undouble(name: &str) -> &str {
    let words: Vec<&str> = name.split(' ').collect();
    let half = words.len() / 2;
    if half > 0 && words.len().is_multiple_of(2) && words[..half] == words[half..] {
        let end = words[..half].iter().map(|w| w.len()).sum::<usize>() + half - 1;
        &name[..end]
    } else {
        name
    }
}

/// Lower case without accents or apostrophes, the "feat." part dropped, every other run of
/// punctuation and spaces as one space: "Don't Stop the Music (feat. X)" → "dont stop the
/// music".
fn normalise(name: &str) -> String {
    let lower = without_feat(&name.to_lowercase());
    let mut out = String::with_capacity(lower.len());
    for c in lower.chars() {
        match c {
            '\'' | '\u{2018}' | '\u{2019}' | '`' | '\u{b4}' => {}
            c if c.is_alphanumeric() => out.push(unaccented(c)),
            _ if !out.is_empty() && !out.ends_with(' ') => out.push(' '),
            _ => {}
        }
    }
    out.truncate(out.trim_end().len());
    out
}

/// How a guest artist is introduced, after a space or an opening bracket.
const FEAT: [&str; 4] = ["feat.", "feat ", "ft.", "featuring "];

/// `lower` without its "feat." part: a bracketed one ("(feat. X)", "[ft. X]") is cut out,
/// a bare one ("feat. X") runs to the end.
fn without_feat(lower: &str) -> String {
    let mut s = lower.to_string();
    while let Some(at) = FEAT.iter().filter_map(|f| find_word(&s, f)).min() {
        let before = s[..at].trim_end();
        let close = match before.chars().last() {
            Some('(') => Some(')'),
            Some('[') => Some(']'),
            _ => None,
        };
        match close {
            Some(close) => {
                let end = s[at..].find(close).map_or(s.len(), |i| at + i + 1);
                s.replace_range(before.len() - 1..end, " ");
            }
            None => s.truncate(at),
        }
    }
    s
}

/// Where `word` starts a word of `s` past its first character (after a space or a bracket).
fn find_word(s: &str, word: &str) -> Option<usize> {
    s.match_indices(word)
        .map(|(i, _)| i)
        .find(|&i| i > 0 && s[..i].ends_with([' ', '(', '[']))
}

/// The base letter of a lower-case Latin letter with an accent.
fn unaccented(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => 'i',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'ų' => 'u',
        'ý' | 'ÿ' => 'y',
        'ś' | 'š' | 'ş' => 's',
        'ź' | 'ż' | 'ž' => 'z',
        'ğ' => 'g',
        'ł' => 'l',
        'ř' => 'r',
        'ť' => 't',
        'ď' => 'd',
        c => c,
    }
}

/// Which request found the lyrics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// `/api/get`: title, artist, album and length all matched.
    Exact,
    /// `/api/search`, which answered with `results` records.
    Search { results: usize },
}

/// What a lookup came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    Found {
        lyrics: Lyrics,
        via: Via,
        /// The record's LRCLIB id, when it has one.
        id: Option<u64>,
    },
    /// LRCLIB has nothing for the track: no exact record, and none of the search's
    /// `results` matched it.
    Missed { results: usize },
}

impl fmt::Display for Lookup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = |l: &Lyrics| match l {
            Lyrics::Synced(_) => "synced",
            Lyrics::Plain(_) => "plain",
        };
        match self {
            Lookup::Found {
                lyrics,
                via: Via::Exact,
                ..
            } => write!(f, "{} (exact match)", kind(lyrics)),
            Lookup::Found {
                lyrics,
                via: Via::Search { results },
                ..
            } => write!(f, "{} (search, {results} results)", kind(lyrics)),
            Lookup::Missed { results } => write!(f, "no match (search, {results} results)"),
        }
    }
}

/// The LRCLIB client. Cheap to clone.
#[derive(Clone, Debug)]
pub struct LrcLib {
    client: reqwest::Client,
    base: String,
}

impl LrcLib {
    pub fn new() -> Result<LrcLib, Error> {
        LrcLib::with_base(BASE)
    }

    /// A client of an LRCLIB at `base` (`…/api/`): another instance, or a local stand-in
    /// in tests.
    pub fn with_base(base: &str) -> Result<LrcLib, Error> {
        let client = crate::http::client("lrclib.net", TIMEOUT).map_err(Error::Network)?;
        Ok(LrcLib {
            client,
            base: base.to_string(),
        })
    }

    /// The track's lyrics: the exact record when it's synced, else the best search result
    /// (synced first), else the exact record's plain text. At most two requests.
    ///
    /// A failed exact lookup doesn't end it: the search may still find the track. An error
    /// only when no lyrics were found and a request failed, since the miss isn't certain.
    pub async fn find(&self, q: &Query) -> Result<Lookup, Error> {
        let mut exact = None;
        let mut failed = None;
        if let (Some(album), Some(duration)) = (&q.album, q.duration) {
            let seconds = duration.as_secs().to_string();
            let params = [
                ("track_name", q.title.as_str()),
                ("artist_name", q.artist.as_str()),
                ("album_name", album.as_str()),
                ("duration", seconds.as_str()),
            ];
            let record = match self.fetch("get", &params).await {
                Ok(body) => body.map(|b| parse_record(&b)).transpose(),
                Err(e) => Err(e),
            };
            match record.map(|r| r.and_then(|r| r.lyrics().map(|l| (l, r.id)))) {
                Ok(Some((lyrics @ Lyrics::Synced(_), id))) => {
                    return Ok(Lookup::Found {
                        lyrics,
                        via: Via::Exact,
                        id,
                    });
                }
                Ok(plain) => exact = plain,
                Err(e) => failed = Some(e),
            }
        }
        let searched = self.search(q).await;
        let exact = exact.map(|(lyrics, id)| Lookup::Found {
            lyrics,
            via: Via::Exact,
            id,
        });
        match (searched, exact, failed) {
            (
                Ok(
                    found @ Lookup::Found {
                        lyrics: Lyrics::Synced(_),
                        ..
                    },
                ),
                _,
                _,
            ) => Ok(found),
            (_, Some(exact), _) => Ok(exact),
            (Ok(Lookup::Missed { .. }), None, Some(e)) => Err(e),
            (searched, None, _) => searched,
        }
    }

    /// `/api/search` by title and artist, and the record among its answers that fits.
    async fn search(&self, q: &Query) -> Result<Lookup, Error> {
        let params = [
            ("track_name", q.title.as_str()),
            ("artist_name", q.artist.as_str()),
        ];
        let Some(body) = self.fetch("search", &params).await? else {
            return Ok(Lookup::Missed { results: 0 });
        };
        let records = parse_search(&body)?;
        let results = records.len();
        Ok(
            match best_match(records, q).and_then(|r| r.lyrics().map(|l| (l, r.id))) {
                Some((lyrics, id)) => Lookup::Found {
                    lyrics,
                    via: Via::Search { results },
                    id,
                },
                None => Lookup::Missed { results },
            },
        )
    }

    /// `/api/search` for `text` (any of title, artist and album, as typed): every record
    /// with words, in LRCLIB's order. One request; what "Choose other lyrics" lists.
    pub async fn search_text(&self, text: &str) -> Result<Vec<Record>, Error> {
        let Some(body) = self.fetch("search", &[("q", text)]).await? else {
            return Ok(Vec::new());
        };
        let mut records = parse_search(&body)?;
        records.retain(Record::has_lyrics);
        Ok(records)
    }

    /// The body of `GET {base}{path}?{params}`, or `None` on a 404.
    async fn fetch(&self, path: &str, params: &[(&str, &str)]) -> Result<Option<String>, Error> {
        Ok(crate::http::get(&self.client, &self.base, path, params, MAX_BODY).await?)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        /// `first_line` and `has_lyrics` read the text as `lyrics` would, without parsing it.
        #[test]
        fn the_quick_reads_agree_with_lyrics(
            synced in proptest::option::of(crate::lrc::tests::lrc_like()),
            plain in proptest::option::of(prop_oneof![
                Just(" \n \r\n".to_string()),
                ".{0,12}( *\\r?\\n *.{0,12}){0,3}",
            ]),
            instrumental in proptest::option::of(any::<bool>()),
        ) {
            let r = Record {
                synced_lyrics: synced,
                plain_lyrics: plain,
                instrumental,
                ..Record::default()
            };
            let lyrics = r.lyrics();
            prop_assert_eq!(r.has_lyrics(), lyrics.is_some());
            let first = lyrics.and_then(|l| match l {
                Lyrics::Synced(lines) => lines.into_iter().map(|l| l.text).find(|t| !t.is_empty()),
                Lyrics::Plain(text) => {
                    text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
                }
            });
            prop_assert_eq!(r.first_line(), first);
        }
    }
}
