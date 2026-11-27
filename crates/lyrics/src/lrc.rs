//! The LRC format: `[mm:ss.xx]` time stamps before each line.

use std::time::Duration;

/// One timed line. `text` is empty for a stamped blank line (an instrumental gap).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub start: Duration,
    pub text: String,
}

/// A track's lyrics: timed lines, or one block of plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lyrics {
    Synced(Vec<Line>),
    Plain(String),
}

/// Parses LRC text into lines sorted by time. `None` if no line carries a time stamp, so
/// the caller can fall back to plain lyrics.
///
/// A line may carry several stamps (`[00:10.00][00:42.00]Chorus`), and becomes one line
/// per stamp. `[offset:±ms]` moves every line: a positive offset shows them sooner. Other
/// tags (`[ar:…]`, `[ti:…]`) and untimed lines are left out; a stamp with no text is kept
/// as an empty line, the gap before the next one.
pub fn parse(text: &str) -> Option<Vec<Line>> {
    let mut offset_ms: i64 = 0;
    let mut timed: Vec<(i64, String)> = Vec::new();
    let mut stamps: Vec<i64> = Vec::new();
    for raw in text.lines() {
        let words = stamped(raw, &mut stamps, &mut offset_ms).trim();
        timed.extend(stamps.iter().map(|&ms| (ms, words.to_string())));
    }
    if timed.is_empty() {
        return None;
    }
    // Stable, so lines sharing a stamp keep the file's order.
    timed.sort_by_key(|(ms, _)| *ms);
    let lines = timed
        .into_iter()
        .map(|(ms, text)| Line {
            start: Duration::from_millis(u64::try_from(ms.saturating_sub(offset_ms)).unwrap_or(0)),
            text,
        })
        .collect();
    Some(lines)
}

/// Reads the tags at the start of one line of LRC: its time stamps into `stamps` (emptied
/// first; left empty on a line with a metadata tag, which holds no lyrics), and an
/// `[offset:±ms]` into `offset_ms`. Returns the text after the tags, as written.
pub(crate) fn stamped<'a>(raw: &'a str, stamps: &mut Vec<i64>, offset_ms: &mut i64) -> &'a str {
    stamps.clear();
    let mut rest = raw.trim();
    while let Some(tag) = rest.strip_prefix('[') {
        let Some(end) = tag.find(']') else { break };
        let inner = &tag[..end];
        if let Some(ms) = stamp_ms(inner) {
            stamps.push(ms);
        } else if let Some(value) = inner.strip_prefix("offset:") {
            *offset_ms = value.trim().parse().unwrap_or(0);
        } else {
            // A metadata tag: nothing on this line is lyrics.
            stamps.clear();
            break;
        }
        rest = &tag[end + 1..];
    }
    rest
}

/// `mm:ss`, `mm:ss.x`, `mm:ss.xx`, `mm:ss.xxx` or `mm:ss:xx`, in milliseconds.
pub(crate) fn stamp_ms(tag: &str) -> Option<i64> {
    let (minutes, rest) = tag.trim().split_once(':')?;
    let (seconds, fraction) = match rest.split_once(['.', ':']) {
        Some((s, f)) => (s, f),
        None => (rest, ""),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(minutes) || !digits(seconds) || !(fraction.is_empty() || digits(fraction)) {
        return None;
    }
    let minutes: i64 = minutes.parse().ok()?;
    let seconds: i64 = seconds.parse().ok()?;
    // Tenths, hundredths or thousandths, by how many digits there are.
    let frac_ms = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().ok()? * 100,
        2 => fraction.parse::<i64>().ok()? * 10,
        _ => fraction.get(..3)?.parse::<i64>().ok()?,
    };
    // A stamp too far out for milliseconds in an `i64` is no stamp.
    minutes
        .checked_mul(60)?
        .checked_add(seconds)?
        .checked_mul(1000)?
        .checked_add(frac_ms)
}

/// The line playing at `at`: the last one that has started. `None` before the first.
pub fn line_at(lines: &[Line], at: Duration) -> Option<usize> {
    lines.partition_point(|l| l.start <= at).checked_sub(1)
}

/// When the line after the one playing at `at` starts, if there is one.
pub fn next_start(lines: &[Line], at: Duration) -> Option<Duration> {
    lines
        .get(lines.partition_point(|l| l.start <= at))
        .map(|l| l.start)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn line(start: u64, text: &str) -> Line {
        Line {
            start: ms(start),
            text: text.into(),
        }
    }

    #[test]
    fn parse_should_sort_stamps_apply_the_offset_and_keep_blank_gaps() {
        let text = "[ar:Someone]\n\
                    [offset:+500]\n\
                    [00:10.00][00:30.50]Chorus line\n\
                    [00:05.20]First line\n\
                    \n\
                    [00:20.000]\n\
                    [00:25:10] Second line  \n\
                    not a timed line\n\
                    [01:02.5]Last\r\n";
        assert_eq!(
            parse(text),
            Some(vec![
                line(4_700, "First line"),
                line(9_500, "Chorus line"),
                line(19_500, ""),
                line(24_600, "Second line"),
                line(30_000, "Chorus line"),
                line(62_000, "Last"),
            ])
        );
        // No stamps at all: plain text, for the caller's unsynced fallback.
        assert_eq!(parse("[ti:Title]\nJust words\nMore words"), None);
        // A negative offset delays the lines.
        assert_eq!(
            parse("[offset:-250]\n[00:01.00]One"),
            Some(vec![line(1_250, "One")])
        );
        // A stamp past what milliseconds in an `i64` hold is left out; an offset that
        // would push a line past them stops it there.
        assert_eq!(
            parse("[200000000000000:00]Too late\n[00:01.00]One"),
            Some(vec![line(1_000, "One")])
        );
        assert_eq!(
            parse("[offset:-9223372036854775808]\n[153722867280912:00]Last"),
            Some(vec![line(i64::MAX as u64, "Last")])
        );
    }

    #[test]
    fn line_at_should_pick_the_last_started_line_and_next_start_the_one_after() {
        let lines = vec![line(1_000, "a"), line(4_000, "b"), line(9_000, "c")];
        assert_eq!(line_at(&lines, ms(500)), None);
        assert_eq!(line_at(&lines, ms(1_000)), Some(0));
        assert_eq!(line_at(&lines, ms(5_000)), Some(1));
        assert_eq!(line_at(&lines, ms(60_000)), Some(2));
        assert_eq!(line_at(&[], ms(5_000)), None);
        assert_eq!(next_start(&lines, ms(500)), Some(ms(1_000)));
        assert_eq!(next_start(&lines, ms(4_000)), Some(ms(9_000)));
        assert_eq!(next_start(&lines, ms(9_500)), None);
    }

    /// Text made of LRC's own pieces (brackets, colons, digit runs of any length, offsets,
    /// tags, line breaks) as well as any characters, so most lines get far into the parser.
    fn lrc_like() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            Just("[".to_string()),
            Just("]".to_string()),
            Just(":".to_string()),
            Just(".".to_string()),
            Just("offset:".to_string()),
            Just("ar:".to_string()),
            Just("+".to_string()),
            Just("-".to_string()),
            Just("\n".to_string()),
            Just("\r\n".to_string()),
            "[0-9]{1,22}",
            "\\[[0-9]{1,20}:[0-9]{1,20}([.:][0-9]{1,5})?\\]",
            "\\[offset:[+-]?[0-9]{1,20}\\]",
            any::<String>(),
        ];
        prop::collection::vec(piece, 0..40).prop_map(|p| p.concat())
    }

    proptest! {
        /// Any text gives lines or `None`, never a panic, and the lines are in time order.
        #[test]
        fn any_text_parses_into_lines_in_time_order(text in lrc_like()) {
            if let Some(lines) = parse(&text) {
                prop_assert!(lines.is_sorted_by_key(|l| l.start), "{lines:?}");
            }
        }

        /// Well-formed timed lines, in any order, come back sorted by time; lines that share
        /// a time keep the file's order.
        #[test]
        fn timed_lines_come_back_in_time_order(
            written in prop::collection::vec((0u64..6_000_000, "[a-zA-Z0-9 ,.'!?]{0,24}"), 1..20),
        ) {
            let text: String = written
                .iter()
                .map(|(t, words)| {
                    format!("[{:02}:{:02}.{:03}]{words}\n", t / 60_000, t / 1_000 % 60, t % 1_000)
                })
                .collect();
            let mut want: Vec<Line> = written
                .iter()
                .map(|(t, words)| line(*t, words.trim()))
                .collect();
            want.sort_by_key(|l| l.start);
            prop_assert_eq!(parse(&text), Some(want));
        }
    }
}
