//! What every OS's tray shares: the icon's events, the menu's entries and labels, and the
//! hover text's limits.

use featherplay_core::tr;

/// Windows keeps 128 UTF-16 units of tooltip, including the terminator.
pub const TOOLTIP_MAX_UTF16: usize = 127;
/// The longest line of hover text, in UTF-16 units: longer ones are cut with "…", so the
/// tooltip stays two lines of a sensible width.
pub const TOOLTIP_LINE_UTF16: usize = 64;

/// What the tray icon reports to the UI thread itself (the window procedure calls the
/// handler; it should only post the event on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// A left click, released at this point of the screen (physical pixels).
    Click { x: i32, y: i32 },
    /// A double click with the left button: the app brings back a window hidden to the tray.
    DoubleClick,
    /// A right click: Windows' menu opens.
    Menu,
    /// The pointer came onto the icon or moved over it: the hover text may show.
    Hover,
    /// An entry of the right-click menu was chosen.
    Chose(TrayChoice),
}

/// What the menu shows: the player's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuState {
    pub title: String,
    pub artist: String,
    pub has_track: bool,
    pub playing: bool,
    pub liked: bool,
}

/// An entry of the menu that does something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayChoice {
    PlayPause,
    Previous,
    Next,
    Like,
    NotForMe,
    Show,
    Quit,
}

impl TrayChoice {
    const ALL: [TrayChoice; 7] = [
        TrayChoice::PlayPause,
        TrayChoice::Previous,
        TrayChoice::Next,
        TrayChoice::Like,
        TrayChoice::NotForMe,
        TrayChoice::Show,
        TrayChoice::Quit,
    ];

    /// The entry's muda id.
    pub fn id(self) -> &'static str {
        match self {
            TrayChoice::PlayPause => "play-pause",
            TrayChoice::Previous => "previous",
            TrayChoice::Next => "next",
            TrayChoice::Like => "like",
            TrayChoice::NotForMe => "not-for-me",
            TrayChoice::Show => "show",
            TrayChoice::Quit => "quit",
        }
    }

    /// The entry of a muda id, if it's one of these.
    pub fn from_id(id: &str) -> Option<TrayChoice> {
        TrayChoice::ALL.into_iter().find(|c| c.id() == id)
    }
}

/// The longest first line, in UTF-16 units.
pub const HEADER_UTF16: usize = 48;

/// The labels and states that follow the player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Labels {
    /// The grey first line, ready for a Win32 menu ("&" doubled).
    pub header: String,
    pub play: &'static str,
    pub like: &'static str,
    pub liked: bool,
    /// Whether the controls (play, previous, next, like, not for me) work.
    pub controls: bool,
}

/// The labels for the player's state `s`.
pub fn labels(s: &MenuState) -> Labels {
    if !s.has_track {
        return Labels {
            header: tr!("Nothing playing").into(),
            play: tr!("Play"),
            like: tr!("Like"),
            liked: false,
            controls: false,
        };
    }
    Labels {
        header: header(&s.title, &s.artist),
        play: if s.playing { tr!("Pause") } else { tr!("Play") },
        like: if s.liked { tr!("Liked") } else { tr!("Like") },
        liked: s.liked,
        controls: true,
    }
}

/// "title (artist)" on one line within `HEADER_UTF16`, each cut with "…" as needed (the
/// artist keeps two fifths of the room), and "&" doubled so Windows shows it rather than
/// underlining the next letter.
fn header(title: &str, artist: &str) -> String {
    let one_line = |s: &str| s.replace(['\r', '\n', '\t'], " ").trim().to_string();
    let (title, artist) = (one_line(title), one_line(artist));
    let text = if artist.is_empty() {
        ellipsize(&title, HEADER_UTF16)
    } else {
        // " (" and ")".
        let room = HEADER_UTF16 - 3;
        let (t, a) = (utf16_len(&title), utf16_len(&artist));
        let artist_room = if t + a <= room {
            a
        } else {
            a.min(room.saturating_sub(t).max(room * 2 / 5))
        };
        format!(
            "{} ({})",
            ellipsize(&title, room - artist_room),
            ellipsize(&artist, artist_room)
        )
    };
    text.replace('&', "&&")
}

/// The length of `s` in UTF-16 units, as Windows counts it.
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s` in at most `max` UTF-16 units, ending in "…" when it had to be cut.
pub fn ellipsize(s: &str, max: usize) -> String {
    if utf16_len(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    format!("{}…", cut_utf16(s, max - 1).trim_end())
}

/// `s` cut to at most `max` UTF-16 units, on a character boundary.
pub fn cut_utf16(s: &str, max: usize) -> &str {
    let mut units = 0;
    let end = s
        .char_indices()
        .find(|(_, c)| {
            units += c.len_utf16();
            units > max
        })
        .map_or(s.len(), |(i, _)| i);
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(title: &str, artist: &str, playing: bool, liked: bool) -> MenuState {
        MenuState {
            title: title.into(),
            artist: artist.into(),
            has_track: true,
            playing,
            liked,
        }
    }

    fn utf16_len(s: &str) -> usize {
        s.encode_utf16().count()
    }

    #[test]
    fn playing_offers_pause_and_names_the_song() {
        let l = labels(&state("faceplant", "LØLØ", true, false));
        assert_eq!(
            l,
            Labels {
                header: "faceplant (LØLØ)".into(),
                play: "Pause",
                like: "Like",
                liked: false,
                controls: true,
            }
        );
    }

    #[test]
    fn paused_offers_play() {
        assert_eq!(labels(&state("T", "A", false, false)).play, "Play");
    }

    #[test]
    fn a_liked_song_is_checked_and_says_liked() {
        let l = labels(&state("T", "A", true, true));
        assert!(l.liked);
        assert_eq!(l.like, "Liked");
    }

    #[test]
    fn with_nothing_loaded_the_controls_rest() {
        let l = labels(&MenuState::default());
        assert_eq!(l.header, "Nothing playing");
        assert!(!l.controls);
        assert!(!l.liked);
        assert_eq!(l.play, "Play");
    }

    #[test]
    fn without_an_artist_the_title_stands_alone() {
        assert_eq!(labels(&state("Title", "", true, false)).header, "Title");
    }

    #[test]
    fn a_long_title_is_cut_and_keeps_the_artist() {
        let l = labels(&state(&"Very long title ".repeat(8), "LØLØ", true, false));
        assert!(utf16_len(&l.header) <= HEADER_UTF16, "{}", l.header);
        assert!(l.header.ends_with("… (LØLØ)"), "{}", l.header);
    }

    #[test]
    fn a_long_artist_is_cut_too() {
        let long = "x".repeat(100);
        let l = labels(&state(&long, &long, true, false));
        assert!(utf16_len(&l.header) <= HEADER_UTF16, "{}", l.header);
        assert!(l.header.ends_with("…)"), "{}", l.header);
    }

    #[test]
    fn an_ampersand_shows_as_itself_not_as_a_shortcut() {
        let l = labels(&state("Rock & Roll", "A&B", true, false));
        assert_eq!(l.header, "Rock && Roll (A&&B)");
    }

    #[test]
    fn a_title_keeps_to_one_line() {
        let l = labels(&state("Two\nlines\tand a tab", "A", true, false));
        assert_eq!(l.header, "Two lines and a tab (A)");
    }

    #[test]
    fn every_choice_comes_back_from_its_id() {
        use TrayChoice::*;
        for c in [PlayPause, Previous, Next, Like, NotForMe, Show, Quit] {
            assert_eq!(TrayChoice::from_id(c.id()), Some(c));
        }
        assert_eq!(TrayChoice::from_id("header"), None);
    }

    #[test]
    fn cut_keeps_short_text_whole() {
        assert_eq!(cut_utf16("featherplay", TOOLTIP_MAX_UTF16), "featherplay");
    }

    #[test]
    fn cut_never_splits_a_surrogate_pair() {
        // "𝄞" is two UTF-16 units: with room for three, the second one doesn't fit.
        assert_eq!(cut_utf16("a𝄞𝄞", 3), "a𝄞");
    }
}
