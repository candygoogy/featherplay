//! Translation and romanization for the lyrics view.
//!
//! Both use Google Translate's unauthenticated `translate_a` endpoints, with no account
//! data. The app asks them only after the user turns Translation or Romanization on; this
//! module keeps no disk cache and knows nothing about the view.
//!
//! - Translation posts to `translate_a/t`, which takes one `q` field per line and answers
//!   one entry per field, in order. Lines therefore never share a text and need no marker;
//!   an answer with a different entry count is an error.
//! - Romanization only comes from `translate_a/single` (`dt=rm`), which romanizes one text
//!   as a whole and turns line breaks into spaces. Lines are sent joined by a mark (`LINE_MARK`)
//!   and split back on it; if the piece count isn't the line count, the chunk is an error.
//!
//! Repeated lines are sent once, and the lines go out in chunks of at most `MAX_TEXT`
//! characters and `MAX_LINES` lines, one request after another.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::time::Duration;

use reqwest::redirect::{Attempt, Policy};
use serde_json::Value;

const BASE: &str = "https://translate.googleapis.com/translate_a";
const USER_AGENT: &str = concat!("featherplay/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(15);

/// Characters of lyric text per request. Google's own translator page takes 5,000 at a
/// time; staying under that keeps us inside what the service is built to answer, while a
/// typical song (1,000 to 3,000 characters) still fits in one or two requests. The text
/// goes in a POST body, so no URL length applies.
const MAX_TEXT: usize = 4_500;
/// Lines per request, so a chunk of very short lines stays a modest form and answer.
const MAX_LINES: usize = 100;
/// An answer is a few times its request at most (translation plus JSON quoting); 1 MiB is
/// far above that and still small enough to hold in memory without a second thought.
const MAX_RESPONSE: usize = 1024 * 1024;

/// Romanization's line boundary: the pilcrow, a "paragraph" sign that lyrics don't use.
/// Google keeps it as it is in the romanized text (checked on 2026-11-18 with Japanese,
/// Korean, Russian, Chinese and Hindi lines), though it may eat the spaces around it.
/// Lines that contain it aren't sent.
const LINE_MARK: char = '¶';
const LINE_JOIN: &str = "\n¶\n";

/// One pass over a group of lines. An empty entry means the service returned no useful text
/// for that line (including a translation identical to the original).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    pub lines: Vec<Option<String>>,
    pub detected_language: Option<String>,
}

/// A translation request failed or returned a shape this client cannot safely line up with
/// the original lyrics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    fn network(error: impl fmt::Display) -> Error {
        Error(error.to_string())
    }

    fn response(message: impl Into<String>) -> Error {
        Error(message.into())
    }

    /// Reqwest's Display text can include the request URL. Report only the kind, so no
    /// request detail reaches the app's log.
    fn request(error: &reqwest::Error) -> Error {
        let kind = if error.is_timeout() {
            "timed out"
        } else if error.is_connect() {
            "couldn't connect"
        } else if error.is_redirect() {
            "redirect failed"
        } else if error.is_body() || error.is_decode() {
            "response failed"
        } else {
            "request failed"
        };
        Error(format!("Google Translate {kind}"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The Google Translate client. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    base: String,
}

impl Client {
    pub fn new() -> Result<Client, Error> {
        Self::with_base(BASE)
    }

    /// A client pointed at another endpoint, for tests. `base` stands for the `translate_a`
    /// directory: requests go to `{base}/t` and `{base}/single`.
    pub fn with_base(base: &str) -> Result<Client, Error> {
        let base_url = reqwest::Url::parse(base).map_err(Error::network)?;
        let allowed_host = base_url.host_str().map(str::to_owned);
        let redirects = Policy::custom(move |attempt: Attempt<'_>| {
            if attempt.url().host_str() == allowed_host.as_deref() && attempt.previous().len() < 3 {
                attempt.follow()
            } else {
                attempt.error("translation redirect to another host refused")
            }
        });
        let http = featherplay_proxy::client()
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .redirect(redirects)
            .build()
            .map_err(Error::network)?;
        Ok(Client {
            http,
            base: base.trim_end_matches('/').to_owned(),
        })
    }

    /// Translates non-empty `lines` to `target` (`"en"`, for example), in a few batched
    /// requests. The returned vector always has one entry per input line.
    pub async fn translate(&self, lines: &[String], target: &str) -> Result<Batch, Error> {
        let plan = Plan::new(lines, |text| text.chars().count() <= MAX_TEXT);
        let mut answers = vec![None; plan.unique.len()];
        let mut languages = Vec::new();
        for range in chunks(plan.unique.iter().map(|text| text.chars().count())) {
            let texts = &plan.unique[range.clone()];
            let form: Vec<(&str, &str)> = texts.iter().map(|text| ("q", text.as_str())).collect();
            let params = [("client", "gtx"), ("sl", "auto"), ("tl", target)];
            let body = self.post("t", &params, &form).await?;
            let read = read_translations(&body, texts.len())?;
            for ((slot, text), (translation, language)) in
                answers[range].iter_mut().zip(texts).zip(read)
            {
                languages.extend(language);
                *slot = useful(text, &translation);
            }
        }
        Ok(Batch {
            lines: plan.spread(&answers),
            detected_language: most_common(languages),
        })
    }

    /// Romanizes lines that contain a non-Latin writing system. Source-language detection is
    /// automatic. The returned vector always has one entry per input line.
    pub async fn romanize(&self, lines: &[String]) -> Result<Batch, Error> {
        let plan = Plan::new(lines, |text| {
            has_non_latin(text)
                && !text.contains(LINE_MARK)
                && text.chars().count() + LINE_JOIN.chars().count() <= MAX_TEXT
        });
        let mut answers = vec![None; plan.unique.len()];
        let mut detected_language = None;
        let costs = plan
            .unique
            .iter()
            .map(|text| text.chars().count() + LINE_JOIN.chars().count());
        for range in chunks(costs) {
            let texts = &plan.unique[range.clone()];
            let joined = texts.join(LINE_JOIN);
            let params = [
                ("client", "gtx"),
                ("sl", "auto"),
                ("tl", "en"),
                ("dt", "rm"),
                ("dj", "1"),
            ];
            let body = self
                .post("single", &params, &[("q", joined.as_str())])
                .await?;
            let (romanized, language) = read_romanization(&body)?;
            if detected_language.is_none() {
                detected_language = language;
            }
            // No romanization at all: Google has nothing to offer for these lines.
            let Some(romanized) = romanized else { continue };
            let pieces = split_lines(&romanized, texts.len())?;
            for ((slot, text), piece) in answers[range].iter_mut().zip(texts).zip(pieces) {
                *slot = useful(text, piece);
            }
        }
        Ok(Batch {
            lines: plan.spread(&answers),
            detected_language,
        })
    }

    /// The body of a form POST to `{base}/{path}`, read up to [`MAX_RESPONSE`] bytes.
    async fn post(
        &self,
        path: &str,
        params: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<String, Error> {
        let mut response = self
            .http
            .post(format!("{}/{path}", self.base))
            .query(params)
            .form(form)
            .send()
            .await
            .map_err(|e| Error::request(&e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::response(format!(
                "Google Translate answered {status}"
            )));
        }
        let too_large = || Error::response("Google Translate's answer is too large");
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE as u64)
        {
            return Err(too_large());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| Error::request(&e))? {
            if body.len() + chunk.len() > MAX_RESPONSE {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        String::from_utf8(body).map_err(|_| Error::response("Google Translate's answer isn't text"))
    }
}

/// The distinct lines to send, and which of them each input line is.
#[derive(Debug, PartialEq, Eq)]
struct Plan {
    unique: Vec<String>,
    slots: Vec<Option<usize>>,
}

impl Plan {
    /// Trimmed, non-blank lines that pass `wanted`, each kept once.
    fn new(lines: &[String], wanted: impl Fn(&str) -> bool) -> Plan {
        let mut unique: Vec<String> = Vec::new();
        let mut seen: HashMap<&str, usize> = HashMap::new();
        let slots = lines
            .iter()
            .map(|line| {
                let text = line.trim();
                if text.is_empty() || !wanted(text) {
                    return None;
                }
                Some(*seen.entry(text).or_insert_with(|| {
                    unique.push(text.to_owned());
                    unique.len() - 1
                }))
            })
            .collect();
        Plan { unique, slots }
    }

    /// One entry per input line: the answer for its distinct line.
    fn spread(&self, answers: &[Option<String>]) -> Vec<Option<String>> {
        self.slots
            .iter()
            .map(|slot| slot.and_then(|i| answers.get(i).cloned().flatten()))
            .collect()
    }
}

/// Splits items with the given costs into consecutive ranges of at most [`MAX_TEXT`] total
/// cost and [`MAX_LINES`] items. An item that alone costs more gets a range of its own.
fn chunks(costs: impl IntoIterator<Item = usize>) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let (mut start, mut total, mut end) = (0, 0, 0);
    for (i, cost) in costs.into_iter().enumerate() {
        if i > start && (total + cost > MAX_TEXT || i - start == MAX_LINES) {
            ranges.push(start..i);
            (start, total) = (i, 0);
        }
        total += cost;
        end = i + 1;
    }
    if start < end {
        ranges.push(start..end);
    }
    ranges
}

/// `translate_a/t`'s answer: a JSON array with one entry per `q`, each entry either the
/// translation or `[translation, detected source language]`. Anything else, including a
/// different entry count, is an error.
fn read_translations(body: &str, expected: usize) -> Result<Vec<(String, Option<String>)>, Error> {
    let unexpected = || Error::response("Google Translate's answer has an unexpected shape");
    let value: Value = serde_json::from_str(body).map_err(|_| unexpected())?;
    let entries = value.as_array().ok_or_else(unexpected)?;
    if entries.len() != expected {
        return Err(Error::response(format!(
            "Google Translate answered {} of {expected} lines",
            entries.len()
        )));
    }
    entries
        .iter()
        .map(|entry| match entry {
            Value::String(text) => Ok((text.clone(), None)),
            Value::Array(parts) => {
                let text = parts
                    .first()
                    .and_then(Value::as_str)
                    .ok_or_else(unexpected)?;
                let language = parts.get(1).and_then(Value::as_str).map(str::to_owned);
                Ok((text.to_owned(), language))
            }
            _ => Err(unexpected()),
        })
        .collect()
}

/// `translate_a/single`'s `dj=1` answer for `dt=rm`: the romanized source text (the
/// `src_translit` fields of its `sentences`, in order) and the detected language (`src`).
fn read_romanization(body: &str) -> Result<(Option<String>, Option<String>), Error> {
    let unexpected = || Error::response("Google Translate's answer has an unexpected shape");
    let value: Value = serde_json::from_str(body).map_err(|_| unexpected())?;
    let object = value.as_object().ok_or_else(unexpected)?;
    let language = object.get("src").and_then(Value::as_str).map(str::to_owned);
    let sentences = match object.get("sentences") {
        None | Some(Value::Null) => return Ok((None, language)),
        Some(Value::Array(sentences)) => sentences,
        Some(_) => return Err(unexpected()),
    };
    let romanized: String = sentences
        .iter()
        .filter_map(|sentence| sentence.get("src_translit").and_then(Value::as_str))
        .collect();
    Ok((
        (!romanized.trim().is_empty()).then_some(romanized),
        language,
    ))
}

/// The romanized text cut back into `count` lines at [`LINE_MARK`], each trimmed. A piece
/// count other than `count` is an error: the lines can't be lined up with certainty.
fn split_lines(text: &str, count: usize) -> Result<Vec<&str>, Error> {
    let pieces: Vec<&str> = text.split(LINE_MARK).map(str::trim).collect();
    if pieces.len() == count {
        Ok(pieces)
    } else {
        Err(Error::response(format!(
            "Google Translate's romanization has {} of {count} lines",
            pieces.len()
        )))
    }
}

/// The answer, unless it is empty or only repeats the source line.
fn useful(source: &str, answer: &str) -> Option<String> {
    let answer = answer.trim();
    (!answer.is_empty() && answer.to_lowercase() != source.trim().to_lowercase())
        .then(|| answer.to_owned())
}

/// The language most lines came back with; on a tie, the one seen first.
fn most_common(languages: Vec<String>) -> Option<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for language in languages {
        match counts.iter_mut().find(|(seen, _)| *seen == language) {
            Some((_, n)) => *n += 1,
            None => counts.push((language, 1)),
        }
    }
    let best = counts.iter().map(|(_, n)| *n).max()?;
    counts.into_iter().find(|(_, n)| *n == best).map(|(l, _)| l)
}

/// Whether `text` contains a letter outside the Latin writing-system blocks. Combining
/// marks and punctuation do not count; accented Latin lyrics therefore do not offer a
/// romanization that would merely repeat them.
pub fn has_non_latin(text: &str) -> bool {
    text.chars().any(|c| c.is_alphabetic() && !is_latin(c))
}

fn is_latin(c: char) -> bool {
    matches!(
        u32::from(c),
        // Basic Latin through combining diacritical marks (Latin-1, Extended-A and -B, IPA,
        // spacing modifiers).
        0x0000..=0x036F
            // Phonetic extensions and Latin Extended Additional.
            | 0x1D00..=0x1EFF
            // Super- and subscripts, letterlike symbols, number forms, enclosed letters.
            | 0x2070..=0x218F
            | 0x2460..=0x24FF
            // Latin Extended-C, -D and -E.
            | 0x2C60..=0x2C7F
            | 0xA720..=0xA7FF
            | 0xAB30..=0xAB6F
            // Latin ligatures and fullwidth Latin letters.
            | 0xFB00..=0xFB06
            | 0xFF21..=0xFF3A
            | 0xFF41..=0xFF5A
    )
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn owned(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|&line| line.to_owned()).collect()
    }

    #[test]
    fn translations_are_read_one_per_line_with_their_language() {
        let body = r#"[["singing here now","ja"],["The night is dark","ru"],"plain"]"#;
        let read = read_translations(body, 3).expect("the answer reads");
        assert_eq!(
            read,
            [
                ("singing here now".to_owned(), Some("ja".to_owned())),
                ("The night is dark".to_owned(), Some("ru".to_owned())),
                ("plain".to_owned(), None),
            ]
        );
    }

    #[test]
    fn a_translation_answer_with_another_line_count_or_shape_is_an_error() {
        assert!(read_translations(r#"[["one","ja"]]"#, 2).is_err());
        assert!(read_translations(r#"["one","ja"]"#, 1).is_err());
        assert!(read_translations(r#"[[1,"ja"]]"#, 1).is_err());
        assert!(read_translations(r#"{"sentences":[]}"#, 1).is_err());
        assert!(read_translations("<html>", 1).is_err());
    }

    #[test]
    fn romanization_is_read_from_its_sentences_and_split_at_the_marks() {
        // The shape of a 2026-11-18 answer for made-up lines; Google ate one space.
        let body = r#"{"sentences":[{"src_translit":"Ima koko de utatte iru¶ nanairo ni kirameite\n¶\nashita e"}],"src":"ja"}"#;
        let (text, language) = read_romanization(body).expect("the answer reads");
        assert_eq!(language.as_deref(), Some("ja"));
        let text = text.expect("romanized text");
        assert_eq!(
            split_lines(&text, 3).expect("three lines"),
            ["Ima koko de utatte iru", "nanairo ni kirameite", "ashita e"]
        );
        assert!(split_lines(&text, 2).is_err());
        assert_eq!(
            read_romanization(r#"{"sentences":[{"trans":"x"}],"src":"ja"}"#),
            Ok((None, Some("ja".to_owned())))
        );
    }

    #[test]
    fn repeated_and_blank_lines_are_planned_once_and_answered_at_every_index() {
        let lines = owned(&["夜", "", " 夜 ", "朝", "  "]);
        let plan = Plan::new(&lines, |_| true);
        assert_eq!(plan.unique, ["夜", "朝"]);
        let answers = [Some("night".to_owned()), None];
        assert_eq!(
            plan.spread(&answers),
            [
                Some("night".to_owned()),
                None,
                Some("night".to_owned()),
                None,
                None
            ]
        );
        assert_eq!(useful("Hello", " hello "), None);
        assert_eq!(useful("夜", ""), None);
    }

    #[test]
    fn chunks_keep_to_the_text_and_line_limits() {
        assert_eq!(chunks([]), Vec::<Range<usize>>::new());
        assert_eq!(chunks([3_000, 1_500, 1]), [0..2, 2..3]);
        assert_eq!(chunks([9_000, 10]), [0..1, 1..2]);
        let many = chunks(std::iter::repeat_n(1, MAX_LINES * 2 + 1));
        assert_eq!(
            many,
            [
                0..MAX_LINES,
                MAX_LINES..MAX_LINES * 2,
                MAX_LINES * 2..MAX_LINES * 2 + 1
            ]
        );
    }

    #[test]
    fn non_latin_letters_are_told_from_accented_latin() {
        assert!(has_non_latin("今ここで"));
        assert!(has_non_latin("Ночь"));
        assert!(has_non_latin("밤하늘"));
        assert!(has_non_latin("Καλημέρα"));
        assert!(!has_non_latin("Déjà vu, ça va? Ægir, łza, Đorđe"));
        assert!(!has_non_latin("1, 2, 3 ... ♪ ¶"));
    }

    proptest! {
        /// Lines joined with the mark split back into the same lines, trimmed, whatever
        /// spaces or line breaks the service leaves around the mark.
        #[test]
        fn marked_lines_split_back_into_the_same_lines(
            lines in prop::collection::vec("[^¶]{0,16}", 1..12),
            gap in prop::sample::select(vec!["\n¶\n", "¶ ", "¶", " ¶ ", "\n\n¶\n\n"]),
        ) {
            let joined = lines.join(gap);
            let split = split_lines(&joined, lines.len()).expect("as many pieces as lines");
            let trimmed: Vec<&str> = lines.iter().map(|line| line.trim()).collect();
            prop_assert_eq!(split, trimmed);
        }

        /// Whatever the repeats and blanks, every line gets the answer of its own trimmed
        /// text, and every distinct text is planned once.
        #[test]
        fn planned_answers_land_on_every_line_with_that_text(
            lines in prop::collection::vec(prop::sample::select(vec!["", " ", "夜", " 夜", "朝 ", "星"]), 0..24),
        ) {
            let lines = owned(&lines);
            let plan = Plan::new(&lines, |_| true);
            let distinct: std::collections::HashSet<&String> = plan.unique.iter().collect();
            prop_assert_eq!(distinct.len(), plan.unique.len());
            let answers: Vec<Option<String>> =
                plan.unique.iter().map(|text| Some(format!("<{text}>"))).collect();
            let spread = plan.spread(&answers);
            prop_assert_eq!(spread.len(), lines.len());
            for (line, answer) in lines.iter().zip(spread) {
                let text = line.trim();
                prop_assert_eq!(answer, (!text.is_empty()).then(|| format!("<{text}>")));
            }
        }

        /// Any JSON array of entries gives one result per line sent or an error.
        #[test]
        fn any_translation_answer_gives_one_result_per_line_or_fails(
            entries in prop::collection::vec(
                prop_oneof![
                    "[a-z ]{0,8}".prop_map(Value::from),
                    ("[a-z ]{0,8}", "[a-z]{2}").prop_map(|(t, l)| Value::from(vec![t, l])),
                    Just(Value::Null),
                    any::<i32>().prop_map(Value::from),
                ],
                0..8,
            ),
            expected in 0usize..8,
        ) {
            let body = Value::Array(entries).to_string();
            if let Ok(read) = read_translations(&body, expected) {
                prop_assert_eq!(read.len(), expected);
            }
        }
    }
}
