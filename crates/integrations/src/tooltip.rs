//! The tray icon's hover text for the current track (`platform::tray::Tray::set_tooltip`
//! shows it).

use std::time::Duration;

use featherplay_core::text::plain;
use featherplay_core::tr;
use featherplay_platform::tray::{
    TIMED_TOOLTIP, TOOLTIP_LINE_UTF16, TOOLTIP_MAX_UTF16, ellipsize, utf16_len,
};

use crate::NowPlaying;

/// The hover text, two lines: the title, then the artist and where the song is ("LØLØ, 1:03
/// of 2:42" while it plays, "LØLØ, paused at 1:03" while paused; the time alone when the
/// length is unknown); "featherplay" and "Nothing playing" with nothing loaded. A line longer
/// than `TOOLTIP_LINE_UTF16` is cut with "…" (the artist before the time, which stays whole),
/// and the whole text keeps within what Windows keeps.
pub fn tooltip_text(np: Option<&NowPlaying>, at: Duration) -> String {
    let Some(np) = np else {
        return format!("featherplay\n{}", tr!("Nothing playing"));
    };
    // Styled maths letters as plain ones, as the window shows them (the shell's font has
    // none of them either).
    let one_line = |s: &str| plain(s).replace(['\r', '\n'], " ").trim().to_string();
    let (title, artist) = (one_line(&np.title), one_line(&np.artist));
    let at = np.duration.map_or(at, |d| at.min(d));
    let time = match (np.playing, np.duration) {
        (true, Some(d)) => tr!("{0} of {1}", clock(at), clock(d)),
        (true, None) => clock(at),
        (false, _) if artist.is_empty() => tr!("Paused at {}", clock(at)),
        (false, _) => tr!("paused at {}", clock(at)),
    };
    let second = if artist.is_empty() {
        time
    } else {
        // The artist, a comma and a space, the time.
        let room = TOOLTIP_LINE_UTF16.saturating_sub(2 + utf16_len(&time));
        format!("{}, {time}", ellipsize(&artist, room))
    };
    let room = TOOLTIP_LINE_UTF16.min(TOOLTIP_MAX_UTF16.saturating_sub(1 + utf16_len(&second)));
    format!("{}\n{second}", ellipsize(&title, room))
}

/// The tray icon's hover text: `tooltip_text` where the tray brings it up to date as it
/// shows (`TIMED_TOOLTIP`, Windows); elsewhere (Linux) the title, then the artist, with no
/// time to go stale.
pub fn tray_tooltip(np: Option<&NowPlaying>, at: Duration) -> String {
    match np {
        Some(np) if !TIMED_TOOLTIP => untimed(np),
        _ => tooltip_text(np, at),
    }
}

/// The title, then the artist (if any), each on one line within `TOOLTIP_LINE_UTF16`.
fn untimed(np: &NowPlaying) -> String {
    let one_line = |s: &str| plain(s).replace(['\r', '\n'], " ").trim().to_string();
    let (title, artist) = (one_line(&np.title), one_line(&np.artist));
    let title = ellipsize(&title, TOOLTIP_LINE_UTF16);
    if artist.is_empty() {
        title
    } else {
        format!("{title}\n{}", ellipsize(&artist, TOOLTIP_LINE_UTF16))
    }
}

/// "2:41", or "1:02:41" from an hour.
fn clock(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn np(title: &str, artist: &str, playing: bool) -> NowPlaying {
        NowPlaying {
            title: title.into(),
            artist: artist.into(),
            playing,
            duration: Some(Duration::from_secs(151)),
            ..Default::default()
        }
    }

    #[test]
    fn tooltip_texts() {
        let secs = Duration::from_secs;
        let live = NowPlaying {
            duration: None,
            ..np("Live", "Someone", true)
        };
        // (case, what plays, the time, the hover text)
        let cases = [
            (
                "playing: the title, then the artist and the time of the length",
                Some(np("faceplant", "LØLØ", true)),
                secs(63),
                "faceplant\nLØLØ, 1:03 of 2:31",
            ),
            (
                "paused: where it paused",
                Some(np("faceplant", "LØLØ", false)),
                secs(63),
                "faceplant\nLØLØ, paused at 1:03",
            ),
            (
                "nothing loaded names the app",
                None,
                Duration::ZERO,
                "featherplay\nNothing playing",
            ),
            (
                "without a length the time alone",
                Some(live),
                secs(3725),
                "Live\nSomeone, 1:02:05",
            ),
            (
                "the time never passes the length",
                Some(np("T", "A", true)),
                secs(400),
                "T\nA, 2:31 of 2:31",
            ),
            (
                "without an artist the time stays, playing",
                Some(np("Title", "", true)),
                Duration::ZERO,
                "Title\n0:00 of 2:31",
            ),
            (
                "without an artist the time stays, paused",
                Some(np("Title", "", false)),
                Duration::ZERO,
                "Title\nPaused at 0:00",
            ),
            (
                "styled letters show plain",
                Some(np("𝙡𝙪𝙪", "𝔅𝔩𝔬𝔬𝔡𝔪𝔬𝔬𝔫", true)),
                Duration::ZERO,
                "luu\nBloodmoon, 0:00 of 2:31",
            ),
            (
                "a title keeps to one line",
                Some(np("Two\nlines", "A", true)),
                Duration::ZERO,
                "Two lines\nA, 0:00 of 2:31",
            ),
            (
                "non-ASCII stays whole",
                Some(np("Ølstua · 夜に駆ける", "LØLØ", false)),
                Duration::ZERO,
                "Ølstua · 夜に駆ける\nLØLØ, paused at 0:00",
            ),
        ];
        for (case, np, at, text) in cases {
            assert_eq!(tooltip_text(np.as_ref(), at), text, "{case}");
        }
    }

    #[test]
    fn tooltip_long_title_ends_in_an_ellipsis_on_two_lines() {
        let title = "Symphony No. 9 in D Minor, Op. 125 \"Choral\": IV. Presto ".repeat(4);
        let text = tooltip_text(Some(&np(&title, "LØLØ", true)), Duration::from_secs(63));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].ends_with('…'), "{}", lines[0]);
        assert!(utf16_len(lines[0]) <= TOOLTIP_LINE_UTF16, "{}", lines[0]);
        assert_eq!(lines[1], "LØLØ, 1:03 of 2:31");
    }

    #[test]
    fn tooltip_keeps_non_ascii_whole() {
        let text = tooltip_text(
            Some(&np("Ølstua · 夜に駆ける", "LØLØ", false)),
            Duration::ZERO,
        );
        assert_eq!(
            text,
            "Ølstua · 夜に駆ける
LØLØ, paused at 0:00"
        );
    }

    #[test]
    fn tooltip_never_passes_the_windows_limit() {
        // Long everything, in two-unit characters: still two lines within 127 units, the
        // time kept whole.
        let long = "𝄞".repeat(200);
        let text = tooltip_text(Some(&np(&long, &long, true)), Duration::from_secs(63));
        assert!(
            utf16_len(&text) <= TOOLTIP_MAX_UTF16,
            "{}",
            utf16_len(&text)
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].ends_with('…'), "{}", lines[0]);
        assert!(lines[1].ends_with("…, 1:03 of 2:31"), "{}", lines[1]);
    }

    #[test]
    fn the_untimed_text_is_the_title_then_the_artist() {
        assert_eq!(untimed(&np("faceplant", "LØLØ", true)), "faceplant\nLØLØ");
        assert_eq!(untimed(&np("Title", "", false)), "Title");
        let at = Duration::from_secs(63);
        let shown = tray_tooltip(Some(&np("faceplant", "LØLØ", true)), at);
        assert_eq!(shown.ends_with("1:03 of 2:31"), TIMED_TOOLTIP, "{shown}");
        assert_eq!(tray_tooltip(None, at), tooltip_text(None, at));
    }
}
