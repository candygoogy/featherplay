//! The notification-area icon. The icon is the colour Tri-vane, in the drawing for the
//! taskbar's theme (the light one has a crimson ring round the play), at the small-icon size
//! for the primary display's scale; `refresh_icon` follows a change of either. A left click
//! is a `TrayEvent::Click` (the app opens its mini player there); a right click opens
//! Windows' own menu (`tray_menu`), whose choices come back as `TrayEvent::Chose`; a double
//! click sends `UiCommand::Show`, which brings back a window hidden to the tray.
//!
//! Resting on the icon shows the shell's own tooltip, the hover text (`tooltip_text`). The
//! pointer's moves over the icon never reach tray-icon, which would arm a timer on each that
//! could run on for good: a subclass of its window stops them as `TrayEvent::Hover`, which
//! the app uses to bring the hover text's time up to date.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use featherplay_core::text::plain;
use featherplay_core::tr;
use tray_icon::menu::MenuEvent;
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    SM_CXSMICON, WM_MOUSEMOVE, WM_NCDESTROY, WM_RBUTTONUP,
};

use crate::taskbar::light_taskbar;
use crate::tray_menu::{MenuState, TrayChoice, TrayMenu, in_taskbar_mode};
use crate::{NowPlaying, Sender, TrayEvent, UiCommand, icons};

/// Windows keeps 128 UTF-16 units of tooltip, including the terminator.
pub const TOOLTIP_MAX_UTF16: usize = 127;
/// The longest line of hover text, in UTF-16 units: longer ones are cut with "…", so the
/// tooltip stays two lines of a sensible width.
pub const TOOLTIP_LINE_UTF16: usize = 64;

pub struct Tray {
    icon: TrayIcon,
    /// The right-click menu tray-icon shows.
    menu: TrayMenu,
    /// The hover text last set, so an unchanged one isn't sent to the shell again.
    tip: RefCell<String>,
    /// The slot size and taskbar theme the icon was drawn for.
    drawn: Cell<(u32, bool)>,
    /// The subclass on tray-icon's window, if it went in (owned; freed in `drop`).
    subclass: Option<(HWND, *mut Subclass)>,
}

/// The app's handler for the icon's events (called from the icon window's procedure).
type OnEvent = Arc<dyn Fn(TrayEvent) + Send + Sync>;

/// tray-icon's callback message for its icon (`WM_USER_TRAYICON` in its Windows backend).
const TRAY_CALLBACK: u32 = 6002;
/// Our subclass id on tray-icon's window.
const SUBCLASS_ID: usize = 0xfea7_0002;

/// What a callback from the shell becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Callback {
    /// Passed on to tray-icon.
    Legacy(u32),
    /// The menu is about to open: passed on to tray-icon, which shows it in the taskbar's
    /// light or dark mode (`in_taskbar_mode`).
    Menu(u32),
    /// One of the app's events, kept from tray-icon.
    Event(TrayEvent),
}

/// Translates a callback (its message is the whole lParam). The pointer over the icon is the
/// app's hover and never reaches tray-icon: on each move tray-icon arms a 15 ms timer to
/// notice the pointer leave, and only a tick that has seen a move since the last one can stop
/// it, so a pointer that rested on the icon, then left, left the timer running for good,
/// waking the UI thread 64 times a second, paused or not (tray-icon 0.26.1).
fn callback(event: u32) -> Callback {
    match event {
        WM_MOUSEMOVE => Callback::Event(TrayEvent::Hover),
        // A right click, or (the icon speaking the shell's first version) Enter, Space or
        // the menu key on it.
        WM_RBUTTONUP => Callback::Menu(event),
        other => Callback::Legacy(other),
    }
}

/// The subclass's state, borrowed by its procedure through `dwRefData`.
struct Subclass {
    on_event: OnEvent,
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: `data` is the `Subclass` that `Tray` owns until it removes this procedure.
    let st = unsafe { &*(data as *const Subclass) };
    if msg == TRAY_CALLBACK {
        match callback(lparam.0 as u32) {
            Callback::Event(ev) => {
                (st.on_event)(ev);
                return LRESULT(0);
            }
            // tray-icon shows the menu in there, and returns once it has closed.
            Callback::Menu(_) => {
                // SAFETY: the window's own message, passed down the subclass chain.
                let pass_down = || unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
                return in_taskbar_mode(pass_down);
            }
            Callback::Legacy(_) => {}
        }
    }
    if msg == WM_NCDESTROY {
        // SAFETY: removing our own procedure from this window.
        let _ = unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID) };
    }
    // SAFETY: the window's own message, passed down the subclass chain.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Subclasses tray-icon's window. `None` if that fails (the pointer's moves then reach
/// tray-icon).
fn subclass(hwnd: HWND, on_event: OnEvent) -> Option<*mut Subclass> {
    let st = Box::into_raw(Box::new(Subclass { on_event }));
    // SAFETY: tray-icon's live window, on its thread; `st` outlives the subclass.
    let ok = unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, st as usize) };
    if !ok.as_bool() {
        // SAFETY: never shared, the subclass didn't go in.
        drop(unsafe { Box::from_raw(st) });
        log::warn!("tray: SetWindowSubclass failed");
        return None;
    }
    Some(st)
}

/// The tray icon's image for a `slot`-pixel slot on a light or dark taskbar.
fn image(slot: u32, light: bool) -> Result<Icon, String> {
    let (rgba, size) = icons::tray_icon(slot, light);
    Icon::from_rgba(rgba, size, size).map_err(|e| format!("tray icon image: {e}"))
}

/// The notification area's icon size: the small-icon size at the primary display's scale
/// (the notification area lives on the primary taskbar). Read each time, since the
/// process's own system DPI stays what it was at sign-in.
fn slot() -> u32 {
    // SAFETY: plain queries with no preconditions; a failed DPI query leaves 96.
    let px = unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let (mut dpi, mut unused) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
        GetSystemMetricsForDpi(SM_CXSMICON, dpi)
    };
    px.clamp(16, 64) as u32
}

impl Tray {
    /// Adds the icon. Call on the UI thread: the icon's window gets its messages there, and
    /// `on_event` is called from its window procedure (so it should only post the event on).
    pub fn new(
        tx: Sender<UiCommand>,
        on_event: impl Fn(TrayEvent) + Send + Sync + 'static,
    ) -> Result<Tray, String> {
        let on_event: OnEvent = Arc::new(on_event);
        let menu = TrayMenu::new()?;
        let drawn = (slot(), light_taskbar());
        let tip = tooltip_text(None, Duration::ZERO);
        let icon = TrayIconBuilder::new()
            .with_icon(image(drawn.0, drawn.1)?)
            .with_tooltip(&tip)
            .with_menu(Box::new(menu.menu()))
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(true)
            .build()
            .map_err(|e| format!("tray icon: {e}"))?;

        let at =
            |p: tray_icon::dpi::PhysicalPosition<f64>| (p.x.round() as i32, p.y.round() as i32);
        // A double click is down, up, double click, up: the last release belongs to it.
        let after_double = AtomicBool::new(false);
        let on_tray = on_event.clone();
        TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| match ev {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } => {
                if !after_double.swap(false, Ordering::Relaxed) {
                    let (x, y) = at(position);
                    on_tray(TrayEvent::Click { x, y });
                }
            }
            // tray-icon shows the menu just after.
            TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Up,
                ..
            } => on_tray(TrayEvent::Menu),
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                after_double.store(true, Ordering::Relaxed);
                let _ = tx.send(UiCommand::Show);
            }
            // The pointer over the icon never reaches tray-icon: the subclass makes it a
            // `TrayEvent::Hover` (`callback`).
            _ => {}
        }));
        let on_menu = on_event.clone();
        MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
            if let Some(choice) = TrayChoice::from_id(ev.id().as_ref()) {
                on_menu(TrayEvent::Chose(choice));
            }
        }));
        let hwnd = HWND(icon.window_handle());
        let subclass = subclass(hwnd, on_event).map(|st| (hwnd, st));
        Ok(Tray {
            icon,
            menu,
            tip: RefCell::new(tip),
            drawn: Cell::new(drawn),
            subclass,
        })
    }

    /// Draws the icon again when the taskbar's theme or the primary display's scale has
    /// changed since it was drawn; otherwise nothing is sent to the shell.
    pub fn refresh_icon(&self) {
        let now = (slot(), light_taskbar());
        if self.drawn.get() == now {
            return;
        }
        match image(now.0, now.1).and_then(|i| {
            self.icon
                .set_icon(Some(i))
                .map_err(|e| format!("tray icon: {e}"))
        }) {
            Ok(()) => self.drawn.set(now),
            Err(e) => log::warn!("tray: {e}"),
        }
    }

    /// Sets the hover text, cut to what Windows keeps. Nothing is sent when it's unchanged.
    pub fn set_tooltip(&self, s: &str) {
        let s = cut_utf16(s, TOOLTIP_MAX_UTF16);
        if *self.tip.borrow() == s {
            return;
        }
        match self.icon.set_tooltip(Some(s)) {
            Ok(()) => s.clone_into(&mut self.tip.borrow_mut()),
            Err(e) => log::warn!("tray: set_tooltip: {e}"),
        }
    }

    /// Sets the right-click menu's labels for the player's state.
    pub fn set_menu(&self, s: &MenuState) {
        self.menu.show(s);
    }
}

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

pub(crate) fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s` in at most `max` UTF-16 units, ending in "…" when it had to be cut.
pub(crate) fn ellipsize(s: &str, max: usize) -> String {
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

impl Drop for Tray {
    fn drop(&mut self) {
        TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
        if let Some((hwnd, st)) = self.subclass.take() {
            // SAFETY: our own procedure comes out first; then nothing borrows its state (the
            // window, destroyed with `icon` after this, makes no further call into it).
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
                drop(Box::from_raw(st));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONDOWN};

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
    fn tooltip_playing_is_the_title_then_artist_and_time_of_length() {
        let text = tooltip_text(
            Some(&np("faceplant", "LØLØ", true)),
            Duration::from_secs(63),
        );
        assert_eq!(
            text,
            "faceplant
LØLØ, 1:03 of 2:31"
        );
    }

    #[test]
    fn tooltip_paused_says_where_it_paused() {
        let text = tooltip_text(
            Some(&np("faceplant", "LØLØ", false)),
            Duration::from_secs(63),
        );
        assert_eq!(
            text,
            "faceplant
LØLØ, paused at 1:03"
        );
    }

    #[test]
    fn tooltip_with_nothing_loaded_names_the_app() {
        assert_eq!(
            tooltip_text(None, Duration::ZERO),
            "featherplay\nNothing playing"
        );
    }

    #[test]
    fn tooltip_without_a_length_shows_the_time_alone() {
        let mut n = np("Live", "Someone", true);
        n.duration = None;
        let text = tooltip_text(Some(&n), Duration::from_secs(3725));
        assert_eq!(
            text,
            "Live
Someone, 1:02:05"
        );
    }

    #[test]
    fn tooltip_time_never_passes_the_length() {
        let text = tooltip_text(Some(&np("T", "A", true)), Duration::from_secs(400));
        assert!(text.ends_with("2:31 of 2:31"), "{text}");
    }

    #[test]
    fn tooltip_without_an_artist_keeps_the_time() {
        let playing = tooltip_text(Some(&np("Title", "", true)), Duration::ZERO);
        assert_eq!(
            playing,
            "Title
0:00 of 2:31"
        );
        let paused = tooltip_text(Some(&np("Title", "", false)), Duration::ZERO);
        assert_eq!(
            paused,
            "Title
Paused at 0:00"
        );
    }

    #[test]
    fn tooltip_shows_styled_letters_plain() {
        let text = tooltip_text(Some(&np("𝙡𝙪𝙪", "𝔅𝔩𝔬𝔬𝔡𝔪𝔬𝔬𝔫", true)), Duration::ZERO);
        assert_eq!(
            text,
            "luu
Bloodmoon, 0:00 of 2:31"
        );
    }

    #[test]
    fn tooltip_keeps_a_title_on_one_line() {
        let text = tooltip_text(
            Some(&np(
                "Two
lines",
                "A",
                true,
            )),
            Duration::ZERO,
        );
        assert_eq!(text.lines().count(), 2, "{text}");
        assert_eq!(text.lines().next(), Some("Two lines"));
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
    fn a_pointer_over_the_icon_is_a_hover_that_tray_icon_never_sees() {
        assert_eq!(callback(WM_MOUSEMOVE), Callback::Event(TrayEvent::Hover));
    }

    #[test]
    fn a_right_click_reaches_tray_icon_which_opens_the_menu() {
        assert_eq!(callback(WM_RBUTTONUP), Callback::Menu(WM_RBUTTONUP));
    }

    #[test]
    fn other_mouse_messages_reach_tray_icon() {
        for msg in [WM_LBUTTONUP, WM_RBUTTONDOWN, WM_LBUTTONDBLCLK] {
            assert_eq!(callback(msg), Callback::Legacy(msg));
        }
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
