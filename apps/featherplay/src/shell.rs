//! The Windows integrations on the UI thread: media controls (SMTC) and taskbar thumb
//! buttons on the main window's HWND, plus the tray icon and the global hotkeys.
//!
//! Everything here runs on the main thread inside Slint's event loop. Other threads reach
//! it only through `slint::invoke_from_event_loop` (see `quit`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay_integrations::{
    Hotkeys, MenuState, NowPlaying, Sender, Smtc, TaskbarButtons, Tray, TrayChoice, TrayEvent,
    UiCommand, tooltip_text,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::{self, Ctx};
use crate::ui::Player;

/// How often to look for the HWND, which exists once the event loop has started.
const HWND_POLL: Duration = Duration::from_millis(10);

#[derive(Default)]
struct Integrations {
    smtc: Option<Smtc>,
    taskbar: Option<TaskbarButtons>,
    tray: Option<Tray>,
    hotkeys: Option<Hotkeys>,
    /// For re-registering the hotkeys from Settings (`set_hotkeys`).
    tx: Option<Sender<UiCommand>>,
    /// The latest state, kept for the window-bound parts if it arrives before the HWND,
    /// and to skip updates that change nothing.
    last: Option<NowPlaying>,
    /// Nothing to play (`idle`): the tray's hover text is the app's name.
    idle: bool,
    poll: Option<Rc<Timer>>,
}

thread_local! {
    static INTEGRATIONS: RefCell<Option<Integrations>> = const { RefCell::new(None) };
}

/// Adds the tray icon and hotkeys now, and attaches the media controls and thumb buttons
/// to the window once `find_hwnd` returns its HWND; then calls `on_hwnd` with it.
/// (The winit window is created after the event loop starts, so it is polled for.)
pub fn start(
    find_hwnd: impl Fn() -> Option<isize> + 'static,
    tx: Sender<UiCommand>,
    hotkeys: Option<featherplay_store::Hotkeys>,
    on_hwnd: impl FnOnce(isize) + 'static,
) {
    let mut it = Integrations {
        tx: Some(tx.clone()),
        ..Default::default()
    };
    let t = Instant::now();
    // The tray's window procedure calls this on this thread; the event is handled on the
    // event loop's next turn, outside that procedure.
    let on_tray = |ev: TrayEvent| {
        let _ = slint::invoke_from_event_loop(move || tray_event(ev));
    };
    match Tray::new(tx.clone(), on_tray) {
        Ok(tray) => {
            log!("tray icon created ({})", ms(t));
            it.tray = Some(tray);
        }
        Err(e) => log!("tray: {e}"),
    }
    match &hotkeys {
        None => log!("hotkeys: none set"),
        Some(cfg) => {
            let t = Instant::now();
            match Hotkeys::register(cfg, tx.clone()) {
                Ok(h) => {
                    log!("hotkeys registered ({})", ms(t));
                    it.hotkeys = Some(h);
                }
                Err(e) => log!("hotkeys: {e}"),
            }
        }
    }

    let poll = Rc::new(Timer::default());
    let weak_poll = Rc::downgrade(&poll);
    let mut on_hwnd = Some(on_hwnd);
    poll.start(TimerMode::Repeated, HWND_POLL, move || {
        let Some(hwnd) = find_hwnd() else {
            return;
        };
        if let Some(p) = weak_poll.upgrade() {
            p.stop();
        }
        attach(hwnd, tx.clone());
        if let Some(f) = on_hwnd.take() {
            f(hwnd);
        }
    });
    it.poll = Some(poll);
    INTEGRATIONS.with(|i| *i.borrow_mut() = Some(it));
}

/// The tray icon's events, on the UI thread: a left click toggles the mini player; a right
/// click opens Windows' menu, so the mini player closes; the pointer over the icon brings the
/// hover text's time up to date, as the shell is about to show it.
fn tray_event(ev: TrayEvent) {
    match ev {
        TrayEvent::Hover => {
            if let Some(at) = crate::bar::position() {
                refresh_tooltip(at);
            }
        }
        TrayEvent::Click { x, y } => crate::mini::toggle(x, y),
        TrayEvent::Menu => crate::mini::close_unpinned(),
        TrayEvent::Chose(choice) => tray_choice(choice),
    }
}

/// Does what an entry of the tray icon's menu says, through the bar's paths: the main
/// window's `Player` callbacks, and Not for me as the bar's (a toast with Undo when the
/// window is on screen, else just rated and skipped).
pub fn tray_choice(choice: TrayChoice) {
    let Some(ctx) = Ctx::get() else { return };
    let Some(main) = ctx.window() else { return };
    let player = main.global::<Player>();
    match choice {
        TrayChoice::PlayPause => player.invoke_toggle(),
        TrayChoice::Previous => player.invoke_previous(),
        TrayChoice::Next => player.invoke_next(),
        TrayChoice::Like => {
            // The bar's handler flips the heart at once and sends the rating.
            player.invoke_toggle_like();
            set_tray_menu(&player);
        }
        TrayChoice::NotForMe => crate::not_for_me::from_tray(&ctx),
        TrayChoice::Show => app::show_window(&ctx),
        TrayChoice::Quit => ctx.quit(),
    }
}

/// Sets the tray icon's menu for the main window's player: the song, Pause or Play, Like.
pub fn set_tray_menu(player: &Player) {
    let state = MenuState {
        title: player.get_title().into(),
        artist: player.get_artist().into(),
        has_track: player.get_has_track(),
        playing: player.get_playing(),
        liked: player.get_liked(),
    };
    INTEGRATIONS.with(|i| {
        if let Some(tray) = i.borrow().as_ref().and_then(|it| it.tray.as_ref()) {
            tray.set_menu(&state);
        }
    });
}

/// Draws the tray icon again if the taskbar's theme or the display's scale changed since
/// (`Tray::refresh_icon`).
pub fn refresh_tray_icon() {
    INTEGRATIONS.with(|i| {
        if let Some(tray) = i.borrow().as_ref().and_then(|it| it.tray.as_ref()) {
            tray.refresh_icon();
        }
    });
}

/// Drops the media session, thumb buttons, tray icon and hotkeys. Call on the UI thread
/// once the event loop has ended.
pub fn stop() {
    INTEGRATIONS.with(|i| i.borrow_mut().take());
}

/// Ends the event loop from any thread.
pub fn quit() {
    let _ = slint::invoke_from_event_loop(|| {
        let _ = slint::quit_event_loop();
    });
}

/// The window's HWND, once it exists.
pub fn hwnd(window: &slint::Window) -> Option<isize> {
    match window.window_handle().window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

fn attach(hwnd: isize, tx: Sender<UiCommand>) {
    log!("window ready, HWND {hwnd:#x}");
    let t = Instant::now();
    let smtc = match Smtc::attach(hwnd, tx.clone()) {
        Ok(s) => {
            log!("media controls attached ({})", ms(t));
            Some(s)
        }
        Err(e) => {
            log!("media controls: {e}");
            None
        }
    };
    let t = Instant::now();
    let taskbar = match TaskbarButtons::attach(hwnd, tx) {
        Ok(b) => {
            log!("taskbar subclassed ({})", ms(t));
            Some(b)
        }
        Err(e) => {
            log!("taskbar buttons: {e}");
            None
        }
    };
    let pending = INTEGRATIONS.with(|i| {
        let mut i = i.borrow_mut();
        let it = i.as_mut()?;
        it.smtc = smtc;
        it.taskbar = taskbar;
        it.last.take()
    });
    if let Some(np) = pending {
        apply(np);
    }
}

/// Shows `np` on the media card, the taskbar buttons and the tray tooltip. A no-op
/// before `start` (offscreen snapshots) and when nothing changed.
pub fn apply(np: NowPlaying) {
    INTEGRATIONS.with(|i| {
        let mut i = i.borrow_mut();
        let Some(it) = i.as_mut() else { return };
        if it.last.as_ref() == Some(&np) && !it.idle {
            return;
        }
        it.idle = false;
        let t = Instant::now();
        if let Some(smtc) = &mut it.smtc {
            smtc.update(&np);
        }
        if let Some(taskbar) = &it.taskbar {
            taskbar.set_playing(np.playing);
        }
        if let Some(tray) = &it.tray {
            tray.set_tooltip(&tooltip_text(Some(&np), np.position));
        }
        let took = t.elapsed();
        let over = if took > Duration::from_millis(16) {
            ", over the 16 ms budget"
        } else {
            ""
        };
        log!(
            "ui: {} {:?} (media card {}, {}{over})",
            if np.playing { "playing" } else { "paused" },
            np.title,
            if it.smtc.is_some() { "set" } else { "pending" },
            ms_of(took)
        );
        it.last = Some(np);
    });
}

/// Nothing to play: the tray's hover text says only the app's name.
pub fn idle() {
    INTEGRATIONS.with(|i| {
        let mut i = i.borrow_mut();
        let Some(it) = i.as_mut() else { return };
        it.idle = true;
        if let Some(tray) = &it.tray {
            tray.set_tooltip(&tooltip_text(None, Duration::ZERO));
        }
    });
}

/// Brings the tray's hover text up to date with the position `at`: the pointer is on the
/// icon, so the text is about to show. The shell hears of it only when the shown second
/// has changed (`Tray::set_tooltip`).
pub fn refresh_tooltip(at: Duration) {
    INTEGRATIONS.with(|i| {
        let i = i.borrow();
        let Some(it) = i.as_ref() else { return };
        if it.idle {
            return;
        }
        if let (Some(tray), Some(np)) = (&it.tray, &it.last) {
            tray.set_tooltip(&tooltip_text(Some(np), at));
        }
    });
}

fn ms(since: Instant) -> String {
    ms_of(since.elapsed())
}
fn ms_of(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

/// Replaces the global hotkeys (the Settings page). `None` unregisters them. If the new
/// keys can't all be registered, `previous` is put back and the error returned.
pub fn set_hotkeys(
    cfg: Option<&featherplay_store::Hotkeys>,
    previous: Option<&featherplay_store::Hotkeys>,
) -> Result<(), String> {
    INTEGRATIONS.with(|i| {
        let mut i = i.borrow_mut();
        let Some(it) = i.as_mut() else {
            return Ok(());
        };
        let Some(tx) = it.tx.clone() else {
            return Ok(());
        };
        // Released first: the new set may reuse a key of the old one.
        it.hotkeys = None;
        let Some(cfg) = cfg else {
            log!("hotkeys: none set");
            return Ok(());
        };
        match Hotkeys::register(cfg, tx.clone()) {
            Ok(h) => {
                log!("hotkeys registered");
                it.hotkeys = Some(h);
                Ok(())
            }
            Err(e) => {
                log!("hotkeys: {e}");
                if let Some(prev) = previous {
                    it.hotkeys = Hotkeys::register(prev, tx).ok();
                }
                Err(e)
            }
        }
    })
}

/// Whether the tray icon exists (hiding to the tray needs a way back).
pub fn has_tray() -> bool {
    INTEGRATIONS.with(|i| i.borrow().as_ref().is_some_and(|it| it.tray.is_some()))
}
