//! The tray icon's right-click menu: Windows' own, built with muda (tray-icon shows it on a
//! right click). A grey first line names the song, then Pause or Play, Previous, Next; Like
//! (checked when liked) and Not for me; while lyrics are on, Desktop lyrics (checked while
//! the strip shows) and, while it shows, Lock or Unlock desktop lyrics; Show featherplay and
//! Quit. The labels follow the player (`TrayMenu::show`), and the menu opens in the
//! taskbar's light or dark mode (`in_taskbar_mode`).

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

use crate::common::tray::{DesktopLyricsMenu, Labels, MenuState, TrayChoice, labels};
use crate::windows::taskbar::light_taskbar;

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
    /// The desktop lyrics' group, put in before Show featherplay while it's offered.
    desktop: CheckMenuItem,
    lock: MenuItem,
    desktop_end: PredefinedMenuItem,
    /// The group as it's in the menu now.
    desktop_shown: RefCell<DesktopLyricsMenu>,
}

/// Where the desktop lyrics' group starts: after the header, the transport and the rating,
/// each group with its separator.
const DESKTOP_AT: usize = 9;

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
            desktop: CheckMenuItem::with_id(
                TrayChoice::DesktopLyrics.id(),
                tr!("Desktop lyrics"),
                true,
                false,
                None,
            ),
            lock: item(
                TrayChoice::LockDesktopLyrics,
                tr!("Lock desktop lyrics"),
                true,
            ),
            desktop_end: PredefinedMenuItem::separator(),
            desktop_shown: RefCell::new(DesktopLyricsMenu::default()),
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
        self.show_desktop_lyrics(s.desktop_lyrics);
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

    /// Puts the desktop lyrics' group in or takes it out, and sets its check and lock label.
    fn show_desktop_lyrics(&self, new: DesktopLyricsMenu) {
        // A click on the entry flips its check by itself: the strip has the last word.
        if self.desktop.is_checked() != new.on {
            self.desktop.set_checked(new.on);
        }
        let mut old = self.desktop_shown.borrow_mut();
        if *old == new {
            return;
        }
        let had_lock = old.lock_label().is_some();
        if old.offered && !new.offered {
            let _ = self.menu.remove(&self.desktop);
            let _ = self.menu.remove(&self.desktop_end);
        } else if !old.offered && new.offered {
            let _ = self
                .menu
                .insert_items(&[&self.desktop, &self.desktop_end], DESKTOP_AT);
        }
        let label = new.lock_label();
        match (had_lock, label) {
            (true, None) => {
                let _ = self.menu.remove(&self.lock);
            }
            (false, Some(_)) => {
                let _ = self.menu.insert(&self.lock, DESKTOP_AT + 1);
            }
            _ => {}
        }
        if let Some(label) = label {
            self.lock.set_text(label);
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
