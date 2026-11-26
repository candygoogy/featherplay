//! Lyrictified (https://api.lyrictified.xyz): a lyrics server with a hand-picked catalogue of
//! `.lrc`, `.elrc` (enhanced LRC, timed by the word) and `.ttml` (timed by the word or the line)
//! files, which grows as its owner approves submissions and finds from other sources. Its
//! documentation is the README of the Lyrictified-Server repository.
//!
//! A lookup is two requests: `GET /search?song=…` for the files whose title matches, ranked by
//! the server (two for a title in two languages, one by each half), and `GET /lyrics/{id}/raw`
//! for the one picked. The title goes without a guest credit ("(feat. …)", `title`). The
//! server needs every word of a request to be among a file's title, artist and tags, and files
//! name their artists as their source did ("ryo, 夏吉ゆうこ" for a song YouTube Music credits
//! to "ryo (supercell)"), so the artist is never sent: the track's artists are matched against
//! the files found here (`best_hit`).
//!
//! Every request is sent once, with a `User-Agent` naming featherplay, no cookies, redirects only
//! within lyrictified.xyz, a timeout and a cap on the body.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;

use crate::title;
use crate::words::{self, WordLine};

const BASE: &str = "https://api.lyrictified.xyz/";
/// Long: a search for a song it has to look up first takes it 7–10 s.
const TIMEOUT: Duration = Duration::from_secs(15);
/// A song's lyrics are a few tens of kB.
const MAX_BODY: usize = 2 * 1024 * 1024;
/// How many matches the search is asked for: enough that a common title still has its artist
/// among them.
const MAX_HITS: &str = "20";

/// What a lookup is matched on: the title, and the names of the track's artists (none when
/// unknown). Only the title is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub song: String,
    pub artists: Vec<String>,
}

/// One file in a search's answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hit {
    /// The server's id for the file: 16 hex digits.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// `lrc`, `elrc` or `ttml`.
    pub format: String,
    /// How well it fits the request, plus the admin's rating of it.
    pub score: i64,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SearchReply {
    results: Vec<Hit>,
}

crate::http::server_error!("Lyrictified");

/// What a lookup came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// Lyrics for the track, timed by the word or by the line, as the file is.
    Found {
        lines: Vec<WordLine>,
        /// The file's id on the server.
        id: String,
        format: String,
    },
    /// Nothing that fits, or nothing that could be read.
    Missed,
}

impl fmt::Display for Lookup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Lookup::Found { lines, format, .. } => {
                let kind = if words::has_words(lines) {
                    "word-timed"
                } else {
                    "line-timed"
                };
                write!(f, "{kind} {format}, {} lines", lines.len())
            }
            Lookup::Missed => f.write_str("no match"),
        }
    }
}

/// The matches in a search's answer, in the server's order.
pub fn parse_search(json: &str) -> Result<Vec<Hit>, Error> {
    serde_json::from_str::<SearchReply>(json)
        .map(|r| r.results)
        .map_err(|e| Error::Parse(e.to_string()))
}

/// Ids are used in a path: hex digits only.
fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Words too common in artist credits to tell two artists apart.
const COMMON: &[&str] = &[
    "the",
    "and",
    "feat",
    "featuring",
    "ft",
    "with",
    "dj",
    "mc",
    "lil",
    "band",
    "orchestra",
];

/// The words of a name, for comparing names: lowercase letters and digits (any script), at
/// least three characters each (two for a name in a script without spaces), none of the
/// common ones (`COMMON`), so "ryo (supercell)" is `ryo` and `supercell`.
fn name_words(name: &str) -> Vec<String> {
    name.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| {
            let n = w.chars().count();
            (n >= 3 || (n == 2 && !w.is_ascii())) && !COMMON.contains(w)
        })
        .map(str::to_string)
        .collect()
}

/// A title as compared: lowercase letters and digits in any script, the rest dropped, so
/// "Ex-Otogibanashi" and "ex otogibanashi" are one title.
fn title_key(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Whether a file's artist names one of the track's: they share a word. A track with no artist
/// known matches any file.
fn same_artist(file_artist: &str, artists: &[String]) -> bool {
    if artists.iter().all(|a| name_words(a).is_empty()) {
        return true;
    }
    let file = name_words(file_artist);
    artists
        .iter()
        .flat_map(|a| name_words(a))
        .any(|w| file.contains(&w))
}

/// The file to fetch: among those in a format read here with one of the track's own titles
/// (`title::lookup_titles`, compared by `title_key` with the file's also without its guests;
/// the server matches a title's words anywhere in a file's title, artist and tags) by one of
/// the track's artists (`same_artist`), the best score the server gave, and of those, timed by
/// the word (`elrc`, `ttml`) before `lrc`; the server's order after that.
pub fn best_hit(hits: Vec<Hit>, song: &str, artists: &[String]) -> Option<Hit> {
    let titles: Vec<String> = title::lookup_titles(song)
        .iter()
        .map(|t| title_key(t))
        .filter(|t| !t.is_empty())
        .collect();
    let usable: Vec<Hit> = hits
        .into_iter()
        .filter(|h| id_ok(&h.id) && matches!(h.format.as_str(), "lrc" | "elrc" | "ttml"))
        .filter(|h| titles.contains(&title_key(&title::without_guests(&h.title))))
        .filter(|h| same_artist(&h.artist, artists))
        .collect();
    let top = usable.iter().map(|h| h.score).max()?;
    usable
        .into_iter()
        .filter(|h| h.score == top)
        .min_by_key(|h| !matches!(h.format.as_str(), "elrc" | "ttml"))
}

/// A file's lines, by its format. Lines without word timing (a plain `.lrc`) have no `parts`.
/// `None` if there's nothing timed to show.
pub fn parse_lyrics(format: &str, body: &str) -> Option<Vec<WordLine>> {
    let lines = match format {
        "ttml" => words::parse_ttml_lines(body),
        // Plain LRC is enhanced LRC with no word stamps.
        "lrc" | "elrc" => words::parse_enhanced_lrc_lines(body),
        _ => None,
    }?;
    lines.iter().any(|l| !l.text.is_empty()).then_some(lines)
}

/// The Lyrictified client. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Lyrictified {
    client: reqwest::Client,
    base: String,
}

impl Lyrictified {
    pub fn new() -> Result<Lyrictified, Error> {
        Lyrictified::with_base(BASE)
    }

    /// A client of a Lyrictified server at `base` (`https://…/`): another instance, or a local
    /// stand-in in tests.
    pub fn with_base(base: &str) -> Result<Lyrictified, Error> {
        let client = crate::http::client("lyrictified.xyz", TIMEOUT).map_err(Error::Network)?;
        Ok(Lyrictified {
            client,
            base: base.to_string(),
        })
    }

    /// The lyrics Lyrictified holds for the track: a search by its title without guests
    /// (`title::lookup_titles`), or for a title in two languages one by each half, the second
    /// only when the first finds no match; and when one has a match, one request for the file.
    /// Nothing is asked without a title. A title the server has never seen answers nothing
    /// while it looks the song up for later, so a miss isn't final.
    pub async fn find(&self, q: &Query) -> Result<Lookup, Error> {
        let song = q.song.trim();
        if song.is_empty() {
            return Ok(Lookup::Missed);
        }
        // The server needs every word of the search in a file, so a whole title in two
        // languages finds only a file titled the same; a search by either half finds that too.
        let titles = title::lookup_titles(song);
        let searches = if titles.len() > 1 {
            &titles[1..]
        } else {
            &titles[..]
        };
        let mut found = None;
        for search in searches {
            let params = [("song", search.as_str()), ("limit", MAX_HITS)];
            let Some(body) = self.fetch("search", &params).await? else {
                continue;
            };
            found = best_hit(parse_search(&body)?, song, &q.artists);
            if found.is_some() {
                break;
            }
        }
        let Some(hit) = found else {
            return Ok(Lookup::Missed);
        };
        let Some(file) = self.fetch(&format!("lyrics/{}/raw", hit.id), &[]).await? else {
            return Ok(Lookup::Missed);
        };
        Ok(match parse_lyrics(&hit.format, &file) {
            Some(lines) => Lookup::Found {
                lines,
                id: hit.id,
                format: hit.format,
            },
            None => Lookup::Missed,
        })
    }

    /// The body of `GET {base}{path}?{params}`, or `None` on a 404.
    async fn fetch(&self, path: &str, params: &[(&str, &str)]) -> Result<Option<String>, Error> {
        Ok(crate::http::get(&self.client, &self.base, path, params, MAX_BODY).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A search's answer as the server writes it.
    const SEARCH: &str = r#"{"results":[{"id":"07199b690c7c4ee7","title":"Idol","artist":"YOASOBI","album":"Idol","format":"lrc","relativePath":"YOASOBI/IDOL/YOASOBI - IDOL EN.lrc","rating":0,"tags":[],"ignore":false,"reverse":false,"ignorePatterns":"","offset":0,"score":800}]}"#;

    fn hit(id: &str, format: &str, score: i64) -> Hit {
        Hit {
            id: id.into(),
            title: "Song".into(),
            format: format.into(),
            score,
            ..Hit::default()
        }
    }

    #[test]
    fn a_search_answer_should_read_into_hits() {
        let hits = parse_search(SEARCH).expect("hits");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "07199b690c7c4ee7");
        assert_eq!((hits[0].format.as_str(), hits[0].score), ("lrc", 800));
        assert_eq!(parse_search(r#"{"results":[]}"#).expect("none"), vec![]);
        assert_eq!(parse_search("{}").expect("none"), vec![]);
        assert!(parse_search("not json").is_err());
    }

    #[test]
    fn the_best_hit_should_have_the_top_score_and_prefer_word_timing_among_equals() {
        let hits = vec![
            hit("aa", "lrc", 800),
            hit("bb", "elrc", 800),
            hit("cc", "ttml", 800),
            hit("dd", "ttml", 700),
        ];
        assert_eq!(best_hit(hits, "Song", &[]).map(|h| h.id), Some("bb".into()));
        // A word-timed file with a lower score doesn't beat a better match.
        let hits = vec![hit("aa", "lrc", 900), hit("bb", "ttml", 800)];
        assert_eq!(best_hit(hits, "song", &[]).map(|h| h.id), Some("aa".into()));
        // Ids that aren't hex (they go into a path) and formats that can't be read are left out.
        let hits = vec![
            hit("../admin", "lrc", 999),
            hit("aa", "mp3", 999),
            hit("", "lrc", 999),
            hit("0f", "lrc", 1),
        ];
        assert_eq!(best_hit(hits, "Song", &[]).map(|h| h.id), Some("0f".into()));
        assert_eq!(best_hit(vec![], "Song", &[]), None);
    }

    fn by(id: &str, title: &str, artist: &str, score: i64) -> Hit {
        Hit {
            title: title.into(),
            artist: artist.into(),
            ..hit(id, "ttml", score)
        }
    }

    #[test]
    fn a_file_should_be_by_one_of_the_tracks_artists_by_a_shared_name() {
        // As the server holds "Ex-Otogibanashi": its source's artist credit, not YouTube Music's.
        let artists = vec![
            "ryo (supercell)".to_string(),
            "Kaguya(cv.Yuko Natsuyoshi)".to_string(),
        ];
        let hits = vec![
            by("aa", "Ex-Otogibanashi", "Someone Else", 900),
            by("bb", "Ex-Otogibanashi", "ryo, 夏吉ゆうこ, 早見沙織", 150),
        ];
        assert_eq!(
            best_hit(hits, "Ex-Otogibanashi", &artists).map(|h| h.id),
            Some("bb".into())
        );
        // Another artist's song of the same title is no match.
        let other = vec![by("aa", "Ex-Otogibanashi", "Someone Else", 900)];
        assert_eq!(best_hit(other, "Ex-Otogibanashi", &artists), None);
        // Nor is another title with the search's words in it, by an artist sharing only "The".
        let artists = vec!["The Paper Kites".to_string()];
        let other = vec![by("aa", "Hello Goodbye", "The Beatles", 900)];
        assert_eq!(best_hit(other, "Hello", &artists), None);
        assert!(!same_artist("The Beatles", &artists));
        let same = vec![by("aa", "hello!", "The Paper Kites", 100)];
        assert_eq!(
            best_hit(same, "Hello", &artists).map(|h| h.id),
            Some("aa".into())
        );
        // No shared name, no match; an unknown artist (or a name with no word in it) matches any.
        assert!(!same_artist("Someone", &["Else".to_string()]));
        assert!(same_artist("Anyone", &[]));
        assert!(same_artist("Anyone", &["?".to_string()]));
        assert!(same_artist("YOASOBI", &["Yoasobi".to_string()]));
    }

    #[test]
    fn a_files_guest_credit_should_not_keep_it_from_matching() {
        // The search goes without the guest, so it finds files titled either way.
        let file = || vec![by("aa", "Song (feat. A Guest)", "The Band", 100)];
        let artists = ["The Band".to_string()];
        for song in ["Song (feat. A Guest)", "Song", "Song (ft. Another)"] {
            assert_eq!(
                best_hit(file(), song, &artists).map(|h| h.id),
                Some("aa".into()),
                "{song}"
            );
        }
        let plain = vec![by("bb", "Song", "The Band", 100)];
        assert_eq!(
            best_hit(plain, "Song (feat. A Guest)", &artists).map(|h| h.id),
            Some("bb".into())
        );
        // Another recording's name stays: no match.
        let live = vec![by("cc", "Song (Live)", "The Band", 100)];
        assert_eq!(best_hit(live, "Song", &artists), None);
    }

    /// What the server writes for a word-timed song from KuGou or QQ (Japanese: no spaces).
    const TTML: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<tt xmlns=\"http://www.w3.org/ns/ttml\" xmlns:itunes=\"http://music.apple.com/lyric-ttml-internal\" xmlns:ttm=\"http://www.w3.org/ns/ttml#metadata\" itunes:timing=\"Word\"><head><metadata><ttm:agent type=\"person\" xml:id=\"v1\" /></metadata></head><body dur=\"0:12.000\">\n<div begin=\"0:05.000\" end=\"0:12.000\">\n<p begin=\"0:05.000\" end=\"0:07.000\" ttm:agent=\"v1\" itunes:key=\"L1\"><span begin=\"0:05.000\" end=\"0:05.500\">青</span><span begin=\"0:05.500\" end=\"0:06.000\">い</span><span begin=\"0:06.000\" end=\"0:07.000\">ためしうた</span></p>\n<p begin=\"0:08.000\" end=\"0:10.000\" ttm:agent=\"v1\" itunes:key=\"L2\"><span begin=\"0:08.000\" end=\"0:08.500\">Test</span> <span begin=\"0:08.500\" end=\"0:10.000\">tune</span></p>\n</div>\n</body></tt>\n";

    #[test]
    fn a_ttml_file_should_give_its_words_and_keep_cjk_pieces_without_spaces() {
        let lines = parse_lyrics("ttml", TTML).expect("lines");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "青いためしうた");
        assert!(lines[0].parts.iter().all(|p| !p.spaced));
        assert_eq!(lines[0].parts.len(), 3);
        assert_eq!(lines[1].text, "Test tune");
        assert!(lines[1].parts[1].spaced);
        assert_eq!(lines[1].parts[1].start, ms(8_500));
    }

    /// What the server writes for a word-timed song as `.elrc`.
    const ELRC: &str = "[ti:Song]\n[ar:Artist]\n[al:Album]\n[00:05.00]<00:05.00>Hel<00:05.30>lo <00:05.60>world<00:06.00>\n[00:08.00]<00:08.00>Next <00:08.50>line<00:09.00>\n";

    #[test]
    fn an_elrc_file_should_give_its_words_and_an_lrc_file_its_lines() {
        let lines = parse_lyrics("elrc", ELRC).expect("lines");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].parts[2].start, ms(5_600));
        assert!(words::has_words(&lines));

        let plain = "[00:01.00]Hello from Lyrictified\n[00:04.00]This is the first lyric file\n";
        let lines = parse_lyrics("lrc", plain).expect("lines");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].start, ms(4_000));
        assert_eq!(lines[1].text, "This is the first lyric file");
        assert!(!words::has_words(&lines));

        assert_eq!(parse_lyrics("lrc", "no stamps here"), None);
        assert_eq!(parse_lyrics("mp3", plain), None);
    }
}
