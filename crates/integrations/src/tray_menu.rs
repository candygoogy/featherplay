//! The tray icon's right-click menu: Windows' own, built with muda (tray-icon shows it on a
//! right click). A grey first line names the song, then Pause or Play, Previous, Next; Like
//! (checked when liked) and Not for me; Show featherplay and Quit. The labels follow the
//! player (`TrayMenu::show`), and the menu opens in the taskbar's light or dark mode
//! (`in_taskbar_mode`).

use std::cell::RefCell;

use featherplay_core::tr;
use tray_icon::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{PCSTR, w};

use crate::taskbar::light_taskbar;
use crate::tray::{ellipsize, utf16_len};

/// The menu and the entries whose labels follow the player.
pub struct TrayMenu {
    menu: Menu,
    header: MenuItem,
    play: MenuItem,
    previous: MenuItem,
    next: MenuItem,
    like: CheckMenuItem,
    not_for_me: MenuItem,
    /// The labels last set, so an unchanged state costs nothing.
    shown: RefCell<Labels>,
}

impl TrayMenu {
    /// Builds the menu for nothing loaded.
    pub fn new() -> Result<TrayMenu, String> {
        let l = labels(&MenuState::default());
        let item =
            |c: TrayChoice, text: &str, enabled| MenuItem::with_id(c.id(), text, enabled, None);
        let menu = TrayMenu {
            menu: Menu::new(),
            header: MenuItem::with_id("header", &l.header, false, None),
            play: item(TrayChoice::PlayPause, l.play, l.controls),
            previous: item(TrayChoice::Previous, tr!("Previous"), l.controls),
            next: item(TrayChoice::Next, tr!("Next"), l.controls),
            like: CheckMenuItem::with_id(TrayChoice::Like.id(), l.like, l.controls, l.liked, None),
            not_for_me: item(TrayChoice::NotForMe, tr!("Not for me"), l.controls),
            shown: RefCell::new(l),
        };
        let show = item(TrayChoice::Show, tr!("Show featherplay"), true);
        let quit = item(TrayChoice::Quit, tr!("Quit"), true);
        let separator = PredefinedMenuItem::separator;
        menu.menu
            .append_items(&[
                &menu.header,
                &separator(),
                &menu.play,
                &menu.previous,
                &menu.next,
                &separator(),
                &menu.like,
                &menu.not_for_me,
                &separator(),
                &show,
                &quit,
            ])
            .map_err(|e| format!("tray menu: {e}"))?;
        Ok(menu)
    }

    /// The muda menu, for tray-icon to show.
    pub fn menu(&self) -> Menu {
        self.menu.clone()
    }

    /// Sets the labels for the player's state `s`; only what changed is sent to Windows.
    pub fn show(&self, s: &MenuState) {
        let new = labels(s);
        // A click on Like flips the check by itself, whether or not the like then goes
        // through: the player has the last word.
        if self.like.is_checked() != new.liked {
            self.like.set_checked(new.liked);
        }
        let mut old = self.shown.borrow_mut();
        if *old == new {
            return;
        }
        if old.header != new.header {
            self.header.set_text(&new.header);
        }
        if old.play != new.play {
            self.play.set_text(new.play);
        }
        if old.like != new.like {
            self.like.set_text(new.like);
        }
        if old.controls != new.controls {
            for item in [&self.play, &self.previous, &self.next, &self.not_for_me] {
                item.set_enabled(new.controls);
            }
            self.like.set_enabled(new.controls);
        }
        *old = new;
    }
}

/// Runs `open`, which shows a menu and returns once it closes, with the process's menus in
/// the taskbar's light or dark mode, as Windows' own menus on the taskbar are; then puts the
/// process's mode back. Only for the menu's life: a forced mode also changes what
/// `ShouldAppsUseDarkMode` says (dark apps for a dark taskbar, whatever the apps' setting),
/// which winit reads for the windows' theme.
pub fn in_taskbar_mode<T>(open: impl FnOnce() -> T) -> T {
    let mode = app_mode(light_taskbar(), high_contrast());
    let forced = mode != AppMode::Default && set_app_mode(mode);
    let out = open();
    if forced {
        set_app_mode(AppMode::Default);
    }
    out
}

/// Whether Windows' high contrast is on.
fn high_contrast() -> bool {
    let mut hc = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    // SAFETY: a HIGHCONTRASTW of the stated size that lives across the call.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some((&raw mut hc).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok() && hc.dwFlags.contains(HCF_HIGHCONTRASTON)
}

/// Sets the process's preferred app mode and has the menus redraw in it: uxtheme's
/// `SetPreferredAppMode` (ordinal 135) and `FlushMenuThemes` (136), from Windows 10 1903 on,
/// the calls the common Win32 dark-mode code makes. False when uxtheme lacks them.
fn set_app_mode(mode: AppMode) -> bool {
    type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
    type FlushMenuThemes = unsafe extern "system" fn();
    // SAFETY: uxtheme from System32; the ordinals are looked up and called with the
    // signatures they have had since 1903. The module stays loaded for the process.
    unsafe {
        let Ok(uxtheme) = LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
        else {
            return false;
        };
        let (Some(set), Some(flush)) = (
            GetProcAddress(uxtheme, PCSTR(135 as *const u8)),
            GetProcAddress(uxtheme, PCSTR(136 as *const u8)),
        ) else {
            return false;
        };
        let set: SetPreferredAppMode = std::mem::transmute(set);
        let flush: FlushMenuThemes = std::mem::transmute(flush);
        set(mode as i32);
        flush();
    }
    true
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

/// `SetPreferredAppMode`'s modes (uxtheme ordinal 135).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppMode {
    /// The system decides (high contrast keeps its own colours).
    Default = 0,
    ForceDark = 2,
    ForceLight = 3,
}

/// The menus' mode for a light or dark taskbar.
pub fn app_mode(light_taskbar: bool, high_contrast: bool) -> AppMode {
    match (high_contrast, light_taskbar) {
        (true, _) => AppMode::Default,
        (false, true) => AppMode::ForceLight,
        (false, false) => AppMode::ForceDark,
    }
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
    fn the_menu_follows_the_taskbar() {
        assert_eq!(app_mode(true, false), AppMode::ForceLight);
        assert_eq!(app_mode(false, false), AppMode::ForceDark);
    }

    #[test]
    fn high_contrast_keeps_the_system_colours() {
        assert_eq!(app_mode(false, true), AppMode::Default);
    }
}
