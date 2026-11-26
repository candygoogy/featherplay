//! Lyrics timed by the word: TTML with a timed `<span>` for each word or syllable of a line (what
//! Unison and Lyrictified serve), and enhanced LRC with a `<mm:ss.xx>` stamp before each one. Also
//! the lines of either when they carry no word timing, so a source can still give them as
//! lines. Written from the formats, not from any player's code.

use std::time::Duration;

use roxmltree::{Document, Node};

use crate::lrc::{self, Line};

/// How deep the TTML is followed into nested spans.
const MAX_DEPTH: usize = 16;
/// How long a piece lasts when nothing says (the last one of a line with no end time).
const FALLBACK_LEN: Duration = Duration::from_millis(400);

/// One sung piece of a line: a word, or a syllable of one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    /// When it starts, in the song.
    pub start: Duration,
    /// When it ends; never before `start`.
    pub end: Duration,
    /// The piece itself, with no space at either end.
    pub text: String,
    /// A space stands before it: it starts a new word. Never set on a line's first piece.
    pub spaced: bool,
}

/// One timed line, with the timing of its words where the lyrics have it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordLine {
    pub start: Duration,
    /// The pieces joined as they were sung; empty for a gap.
    pub text: String,
    /// Empty for a line with no word timing: it is shown as any timed line is.
    pub parts: Vec<Part>,
}

impl WordLine {
    /// The line as the rest of the app knows it.
    pub fn line(&self) -> Line {
        Line {
            start: self.start,
            text: self.text.clone(),
        }
    }
}

/// A piece before its end is settled.
#[derive(Clone)]
struct Raw {
    start: Duration,
    end: Option<Duration>,
    text: String,
    spaced: bool,
}

/// The pieces of one line as they are read, and the spaces between them.
#[derive(Default)]
struct Pieces {
    parts: Vec<Raw>,
    pending_space: bool,
}

impl Pieces {
    /// Whitespace was seen between pieces (or in one's text): the next starts a word.
    fn space(&mut self) {
        if !self.parts.is_empty() {
            self.pending_space = true;
        }
    }

    /// A piece whose text is `raw`, as written: spaces at its ends are the gaps to its
    /// neighbours.
    fn part(&mut self, raw: &str, start: Duration, end: Option<Duration>) {
        let lead = raw.starts_with(char::is_whitespace);
        let trail = raw.ends_with(char::is_whitespace);
        let text = collapse(std::iter::once(raw));
        if text.is_empty() {
            if lead || trail {
                self.space();
            }
            return;
        }
        let spaced = !self.parts.is_empty() && (self.pending_space || lead);
        self.parts.push(Raw {
            start,
            end,
            text,
            spaced,
        });
        self.pending_space = trail;
    }
}

/// Words separated by single spaces.
fn collapse<'a>(texts: impl Iterator<Item = &'a str>) -> String {
    texts
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Settles the ends: a piece with none (or one before its start) ends where the next starts,
/// the last at `line_end`, else a moment after it starts.
fn finish(raw: Vec<Raw>, line_end: Option<Duration>) -> Vec<Part> {
    let starts: Vec<Duration> = raw.iter().map(|p| p.start).collect();
    raw.into_iter()
        .enumerate()
        .map(|(i, p)| {
            let end = p
                .end
                .filter(|e| *e >= p.start)
                .or_else(|| starts.get(i + 1).copied())
                .or(line_end)
                .filter(|e| *e >= p.start)
                .unwrap_or(p.start + FALLBACK_LEN);
            Part {
                start: p.start,
                end,
                text: p.text,
                spaced: p.spaced,
            }
        })
        .collect()
}

/// The pieces joined as sung.
fn joined(parts: &[Part]) -> String {
    let mut text = String::new();
    for p in parts {
        if p.spaced {
            text.push(' ');
        }
        text.push_str(&p.text);
    }
    text
}

/// The longest time read, in seconds: far past any song, and well inside `Duration`.
const MAX_SECONDS: u64 = 1_000_000_000;

/// `digits` or `digits.digits` times `unit_ns` nanoseconds, exactly (fraction digits past the
/// nanosecond are dropped). Digits only: no sign, no exponent.
fn decimal_ns(text: &str, unit_ns: u64) -> Option<u64> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty()) || !digits(whole) || !digits(fraction) {
        return None;
    }
    let whole: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction = &fraction[..fraction.len().min(9)];
    let part = if fraction.is_empty() {
        0
    } else {
        let scale = 10u128.pow(u32::try_from(fraction.len()).ok()?);
        u64::try_from(u128::from(fraction.parse::<u64>().ok()?) * u128::from(unit_ns) / scale)
            .ok()?
    };
    whole.checked_mul(unit_ns)?.checked_add(part)
}

/// A TTML time expression: `1:02.807`, `0:31.725`, `00:00:31.725`, `31.725s`, `500ms`.
/// Integer arithmetic throughout, and nothing past `MAX_SECONDS`: the time comes from a server.
fn clock(value: &str) -> Option<Duration> {
    const NS: u64 = 1_000_000_000;
    let value = value.trim();
    let ns = if let Some(ms) = value.strip_suffix("ms") {
        decimal_ns(ms, 1_000_000)?
    } else if let Some(s) = value.strip_suffix('s') {
        decimal_ns(s, NS)?
    } else {
        let fields: Vec<&str> = value.split(':').collect();
        if fields.len() > 3 {
            return None;
        }
        let (last, whole) = fields.split_last()?;
        let mut minutes: u64 = 0;
        for field in whole {
            if field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            minutes = minutes.checked_mul(60)?.checked_add(field.parse().ok()?)?;
        }
        minutes
            .checked_mul(60)?
            .checked_mul(NS)?
            .checked_add(decimal_ns(last, NS)?)?
    };
    (ns <= MAX_SECONDS * NS).then(|| Duration::from_nanos(ns))
}

fn has_timed_child(node: Node) -> bool {
    node.descendants()
        .skip(1)
        .any(|d| d.is_element() && d.attribute("begin").and_then(clock).is_some())
}

/// Reads the timed pieces under `node` (a line): each element with a `begin` that holds none
/// of its own; whitespace between them is the gap between words.
fn walk(node: Node, out: &mut Pieces, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for child in node.children() {
        if child.is_text() {
            if child
                .text()
                .is_some_and(|t| t.chars().any(char::is_whitespace))
            {
                out.space();
            }
        } else if child.is_element() {
            if child.has_tag_name("br") {
                out.space();
            } else if let Some(start) = child.attribute("begin").and_then(clock)
                && !has_timed_child(child)
            {
                let text: String = child
                    .descendants()
                    .filter(Node::is_text)
                    .filter_map(|n| n.text())
                    .collect();
                out.part(&text, start, child.attribute("end").and_then(clock));
            } else {
                walk(child, out, depth + 1);
            }
        }
    }
}

/// Parses TTML: each `<p>` is a line, and the timed `<span>`s in it its words or syllables,
/// told apart by the spaces between them (`Hel<span>lo</span> <span>you</span>` is two words).
/// A `<p>` with no timed span is a line with no word timing. Lines come sorted by time.
///
/// `None` if the document isn't XML, or no line has word timing (`parse_ttml_lines` keeps
/// those): lyrics timed by the line are another source's kind of lookup.
pub fn parse_ttml(xml: &str) -> Option<Vec<WordLine>> {
    parse_ttml_lines(xml).filter(|lines| has_words(lines))
}

/// Whether any line has the timing of its words.
pub fn has_words(lines: &[WordLine]) -> bool {
    lines.iter().any(|l| !l.parts.is_empty())
}

/// `parse_ttml`, for any timed lines: those with word timing have their `parts`, the rest are
/// lines as `lrc::parse` gives them. `None` if the document isn't XML or has no timed line.
pub fn parse_ttml_lines(xml: &str) -> Option<Vec<WordLine>> {
    let doc = Document::parse(xml).ok()?;
    let mut lines: Vec<WordLine> = Vec::new();
    for p in doc
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("p"))
    {
        let begin = p.attribute("begin").and_then(clock);
        let mut pieces = Pieces::default();
        walk(p, &mut pieces, 0);
        if pieces.parts.is_empty() {
            let text = collapse(
                p.descendants()
                    .filter(Node::is_text)
                    .filter_map(|n| n.text()),
            );
            if let (Some(start), false) = (begin, text.is_empty()) {
                lines.push(WordLine {
                    start,
                    text,
                    parts: Vec::new(),
                });
            }
            continue;
        }
        let first = pieces.parts[0].start;
        let start = begin.map_or(first, |b| b.min(first));
        let parts = finish(pieces.parts, p.attribute("end").and_then(clock));
        lines.push(WordLine {
            start,
            text: joined(&parts),
            parts,
        });
    }
    lines.sort_by_key(|l| l.start);
    (!lines.is_empty()).then_some(lines)
}

/// Parses enhanced LRC: `[mm:ss.xx]` before a line, and `<mm:ss.xx>` before each of its words
/// or syllables (`[00:12.00]<00:12.00>Hel<00:12.30>lo <00:12.60>you<00:13.00>`; a stamp with
/// nothing after it ends the piece before it). A line with no `<>` stamps has no word timing.
/// `[offset:±ms]` moves everything, as in `lrc::parse`.
///
/// `None` if no line has word timing (`parse_enhanced_lrc_lines` keeps those).
pub fn parse_enhanced_lrc(text: &str) -> Option<Vec<WordLine>> {
    parse_enhanced_lrc_lines(text).filter(|lines| has_words(lines))
}

/// `parse_enhanced_lrc`, for any timed lines: those with `<>` stamps have their `parts`, the
/// rest are lines as `lrc::parse` gives them. `None` if there is no timed line.
pub fn parse_enhanced_lrc_lines(text: &str) -> Option<Vec<WordLine>> {
    let mut offset_ms: i64 = 0;
    let mut found: Vec<(i64, Vec<Raw>, String)> = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps: Vec<i64> = Vec::new();
        while let Some(tag) = rest.strip_prefix('[') {
            let Some(end) = tag.find(']') else { break };
            let inner = &tag[..end];
            if let Some(ms) = lrc::stamp_ms(inner) {
                stamps.push(ms);
            } else if let Some(value) = inner.strip_prefix("offset:") {
                offset_ms = value.trim().parse().unwrap_or(0);
            } else {
                stamps.clear();
                break;
            }
            rest = &tag[end + 1..];
        }
        let Some(&first) = stamps.first() else {
            continue;
        };
        let pieces = scan_stamped(rest, first);
        if pieces.is_empty() {
            let words = collapse(std::iter::once(rest));
            found.extend(stamps.into_iter().map(|ms| (ms, Vec::new(), words.clone())));
        } else {
            // A line sung more than once (`[00:10.00][01:10.00]<00:10.00>…`): its words are
            // timed for the first stamp and move with each of the others.
            for ms in stamps {
                let by = ms.saturating_sub(first);
                let moved = pieces
                    .iter()
                    .map(|p| Raw {
                        start: shift(p.start, by),
                        end: p.end.map(|e| shift(e, by)),
                        ..p.clone()
                    })
                    .collect();
                found.push((ms, moved, String::new()));
            }
        }
    }
    if found.is_empty() {
        return None;
    }
    found.sort_by_key(|(ms, _, _)| *ms);
    // A positive `[offset:]` shows everything sooner, as in `lrc::parse`.
    let moved = |d: Duration| shift(d, offset_ms.saturating_neg());
    let starts: Vec<Duration> = found
        .iter()
        .map(|(ms, _, _)| moved(Duration::from_millis(u64::try_from(*ms).unwrap_or(0))))
        .collect();
    let mut lines = Vec::with_capacity(found.len());
    for (i, (_, pieces, plain)) in found.into_iter().enumerate() {
        let start = starts[i];
        if pieces.is_empty() {
            lines.push(WordLine {
                start,
                text: plain,
                parts: Vec::new(),
            });
            continue;
        }
        let shifted = pieces
            .into_iter()
            .map(|p| Raw {
                start: moved(p.start),
                end: p.end.map(moved),
                ..p
            })
            .collect::<Vec<_>>();
        let parts = finish(shifted, starts.get(i + 1).copied());
        let first = parts.first().map_or(start, |p| p.start.min(start));
        lines.push(WordLine {
            start: first,
            text: joined(&parts),
            parts,
        });
    }
    // A line's first word may be stamped before the line itself: sorted again by where
    // each line now starts, since the lines are looked up by time (`lrc::line_at`).
    lines.sort_by_key(|l| l.start);
    Some(lines)
}

/// `d` moved by `by_ms` (either way), never before the song's start.
fn shift(d: Duration, by_ms: i64) -> Duration {
    let ms = i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
    Duration::from_millis(u64::try_from(ms.saturating_add(by_ms)).unwrap_or(0))
}

/// The pieces of one LRC line's text: those after a `<mm:ss.xx>` stamp, each from its stamp to
/// the next. Text before the first stamp starts with the line. Empty if there's no stamp.
fn scan_stamped(text: &str, line_start_ms: i64) -> Vec<Raw> {
    let ms = |ms: i64| Duration::from_millis(u64::try_from(ms).unwrap_or(0));
    let mut pieces = Pieces::default();
    let mut tagged = false;
    let mut from = line_start_ms;
    let mut buf = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let (before, tag) = rest.split_at(open);
        let stamp = tag
            .find('>')
            .and_then(|close| lrc::stamp_ms(&tag[1..close]).map(|s| (s, close)));
        match stamp {
            Some((stamp, close)) => {
                buf.push_str(before);
                if !buf.trim().is_empty() {
                    pieces.part(&buf, ms(from), Some(ms(stamp)));
                } else if !buf.is_empty() {
                    pieces.space();
                }
                buf.clear();
                tagged = true;
                from = stamp;
                rest = &tag[close + 1..];
            }
            None => {
                buf.push_str(before);
                buf.push('<');
                rest = &tag[1..];
            }
        }
    }
    buf.push_str(rest);
    if tagged && !buf.trim().is_empty() {
        pieces.part(&buf, ms(from), None);
    }
    pieces.parts
}
#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn part(start: u64, end: u64, text: &str, spaced: bool) -> Part {
        Part {
            start: ms(start),
            end: ms(end),
            text: text.into(),
            spaced,
        }
    }

    const TTML: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xml:lang="en"><body><div>
        <p begin="0:31.725" end="0:34.415" ttm:agent="v1"><span begin="0:31.725" end="0:31.957">Our</span> <span begin="0:31.957" end="0:32.188">sam</span><span begin="0:32.188" end="0:32.438">ple</span> <span begin="0:32.438" end="0:32.707">song</span></p>
        <p begin="1:35.917" end="1:38.337"><span begin="1:35.917" end="1:36.225">I</span> <span begin="1:36.225">would</span><span begin="1:36.474">n't</span></p>
        <p begin="0:10.000">An untimed line</p>
    </div></body></tt>"#;

    #[test]
    fn ttml_should_give_each_line_its_words_with_syllables_glued_and_lines_sorted() {
        let lines = parse_ttml(TTML).expect("word-timed");
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            WordLine {
                start: ms(10_000),
                text: "An untimed line".into(),
                parts: vec![],
            }
        );
        assert_eq!(lines[1].start, ms(31_725));
        assert_eq!(lines[1].text, "Our sample song");
        assert_eq!(
            lines[1].parts,
            vec![
                part(31_725, 31_957, "Our", false),
                part(31_957, 32_188, "sam", true),
                part(32_188, 32_438, "ple", false),
                part(32_438, 32_707, "song", true),
            ]
        );
        // A piece with no end runs to the next one; the last to the line's end.
        assert_eq!(lines[2].parts[1], part(96_225, 96_474, "would", true));
        assert_eq!(lines[2].parts[2], part(96_474, 98_337, "n't", false));
    }

    #[test]
    fn ttml_without_word_timing_or_without_xml_should_be_none() {
        let by_line = r#"<tt><body><div><p begin="0:01.000">One</p><p begin="0:05.000">Two</p></div></body></tt>"#;
        assert_eq!(parse_ttml(by_line), None);
        assert_eq!(parse_ttml("[00:01.00]not xml"), None);
        assert_eq!(parse_ttml(""), None);
    }

    #[test]
    fn nested_spans_should_flatten_into_the_lines_words() {
        let xml = r#"<tt xmlns:ttm="x"><body><p begin="1.0s" end="3s"><span begin="1.0s" end="2.0s">Hey</span> <span ttm:role="x-bg"><span begin="2.0s" end="3.0s">(you)</span></span></p></body></tt>"#;
        let lines = parse_ttml(xml).expect("word-timed");
        assert_eq!(lines[0].text, "Hey (you)");
        assert_eq!(lines[0].parts[1], part(2_000, 3_000, "(you)", true));
    }

    #[test]
    fn clock_should_read_the_time_expressions_of_ttml() {
        assert_eq!(clock("0:31.725"), Some(ms(31_725)));
        assert_eq!(clock("1:02.807"), Some(ms(62_807)));
        assert_eq!(clock("3:31.181"), Some(ms(211_181)));
        assert_eq!(clock("00:01:02.5"), Some(ms(62_500)));
        assert_eq!(clock("01:01:02.5"), Some(ms(3_662_500)));
        assert_eq!(clock("12.5s"), Some(ms(12_500)));
        assert_eq!(clock("250ms"), Some(ms(250)));
        assert_eq!(clock("45"), Some(ms(45_000)));
        assert_eq!(clock(".5s"), Some(ms(500)));
        // Too far out for any song (and for a `Duration` once multiplied): refused, not a panic.
        for bad in [
            "",
            "x",
            "1:2:3:4",
            "-1",
            "1e400",
            "nan",
            "1:xx",
            "1e2s",
            "18446744073709551615:00",
            "1000000001s",
        ] {
            assert_eq!(clock(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn enhanced_lrc_should_time_the_words_and_end_a_line_at_its_closing_stamp() {
        let text = "[ar:Someone]\n\
                    [00:12.00]<00:12.00>Hel<00:12.30>lo <00:12.60>you<00:13.00>\n\
                    [00:15.00]A plain line\n\
                    [00:20.00]<00:20.00>Last <00:20.50>one";
        let lines = parse_enhanced_lrc(text).expect("word-timed");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "Hello you");
        assert_eq!(
            lines[0].parts,
            vec![
                part(12_000, 12_300, "Hel", false),
                part(12_300, 12_600, "lo", false),
                part(12_600, 13_000, "you", true),
            ]
        );
        assert!(lines[1].parts.is_empty());
        assert_eq!(lines[1].text, "A plain line");
        // The last piece of the last line has no end: a moment after it starts.
        assert_eq!(lines[2].parts[1], part(20_500, 20_900, "one", true));
        // No `<>` stamps anywhere: line timing, for LRCLIB's lookup.
        assert_eq!(parse_enhanced_lrc("[00:01.00]One\n[00:02.00]Two"), None);
    }

    #[test]
    fn an_enhanced_lrc_line_sung_twice_should_be_timed_both_times_and_lines_sorted() {
        let text = "[00:10.00][01:10.00]<00:10.00>Hey <00:10.50>you<00:11.00>
                    [00:20.00]<00:05.00>Early
";
        let lines = parse_enhanced_lrc(text).expect("word-timed");
        let starts: Vec<Duration> = lines.iter().map(|l| l.start).collect();
        assert_eq!(starts, vec![ms(5_000), ms(10_000), ms(70_000)]);
        assert_eq!(lines[2].text, "Hey you");
        assert_eq!(lines[2].parts[1], part(70_500, 71_000, "you", true));
    }

    /// TTML-like and LRC-like text: the formats' own pieces as well as any characters, so
    /// most inputs get far into the parsers.
    fn timed_text() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            Just("<p begin=\"0:01.5\">".to_string()),
            Just("<span begin=\"0:01.5\" end=\"0:02\">".to_string()),
            Just("</span>".to_string()),
            Just("</p>".to_string()),
            Just("<tt>".to_string()),
            Just("<br/>".to_string()),
            Just("[00:01.00]".to_string()),
            Just("<00:01.50>".to_string()),
            Just("[offset:".to_string()),
            Just("<".to_string()),
            Just(">".to_string()),
            Just("&amp;".to_string()),
            Just(" ".to_string()),
            Just("\n".to_string()),
            ".{0,6}",
        ];
        proptest::collection::vec(piece, 0..40).prop_map(|p| p.concat())
    }

    proptest! {
        #[test]
        fn the_parsers_should_never_panic_and_keep_their_promises(text in timed_text()) {
            for lines in [parse_ttml(&text), parse_enhanced_lrc(&text)].into_iter().flatten() {
                prop_assert!(lines.iter().any(|l| !l.parts.is_empty()));
                prop_assert!(lines.windows(2).all(|w| w[0].start <= w[1].start));
                for line in &lines {
                    prop_assert!(line.parts.first().is_none_or(|p| !p.spaced));
                    for p in &line.parts {
                        prop_assert!(p.end >= p.start);
                        prop_assert!(!p.text.is_empty() && p.text.trim() == p.text);
                    }
                }
            }
        }

        #[test]
        fn clock_should_never_panic(text in ".{0,12}") {
            let _ = clock(&text);
        }

        /// Fields of any length, as a server could write them: read or refused, never a panic,
        /// and never past `MAX_SECONDS`.
        #[test]
        fn clock_should_refuse_times_too_large_without_panicking(
            fields in proptest::collection::vec("[0-9]{1,24}", 1..4),
            fraction in proptest::option::of("[0-9]{1,12}"),
            unit in prop_oneof![Just(""), Just("s"), Just("ms")],
        ) {
            let text = fields.join(":") + &fraction.map(|f| format!(".{f}")).unwrap_or_default() + unit;
            if let Some(d) = clock(&text) {
                prop_assert!(d <= Duration::from_secs(MAX_SECONDS), "{text}: {d:?}");
            }
        }

        #[test]
        fn clock_should_count_hours_minutes_and_seconds_exactly(
            h in 0u64..1000, m in 0u64..60, s in 0u64..60, ms in 0u64..1000,
        ) {
            let want = Duration::from_millis(((h * 60 + m) * 60 + s) * 1000 + ms);
            prop_assert_eq!(clock(&format!("{h}:{m:02}:{s:02}.{ms:03}")), Some(want));
            prop_assert_eq!(clock(&format!("{}.{ms:03}s", (h * 60 + m) * 60 + s)), Some(want));
        }

        /// Enhanced LRC with stamps anywhere (a word stamped before its line, lines out of order,
        /// lines sung twice, huge stamps): the lines come sorted, every piece ends after it starts.
        #[test]
        fn enhanced_lrc_lines_should_come_sorted_whatever_the_stamps(
            rows in proptest::collection::vec(
                (proptest::collection::vec(0u64..400_000, 1..3), proptest::collection::vec((0u64..400_000, "[a-z]{1,4}"), 0..4)),
                1..8,
            ),
            huge in proptest::option::of(Just("[99999999999999:00.00]<99999999999999:00.00>x
    ")),
        ) {
            let stamp = |ms: u64| format!("{:02}:{:02}.{:02}", ms / 60_000, ms / 1000 % 60, ms / 10 % 100);
            let mut text = huge.unwrap_or_default().to_string();
            for (starts, words) in &rows {
                for s in starts {
                    text += &format!("[{}]", stamp(*s));
                }
                for (at, word) in words {
                    text += &format!("<{}>{word} ", stamp(*at));
                }
                text += "
    ";
            }
            if let Some(lines) = parse_enhanced_lrc_lines(&text) {
                prop_assert!(lines.windows(2).all(|w| w[0].start <= w[1].start), "{text}");
                for p in lines.iter().flat_map(|l| &l.parts) {
                    prop_assert!(p.end >= p.start);
                }
            }
        }
    }
}
