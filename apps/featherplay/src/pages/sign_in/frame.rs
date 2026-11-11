//! The sign-in window's frame (`ui/sign_in_window.slint`), on the UI thread: a window of its
//! own with back, reload, the read-only address and the status strip, round the room where
//! the sign-in thread's WebView2 page sits as a child window (`webview`). The frame shows
//! what the page thread tells it (`webview::Event`) and passes back, reload, close, resizes
//! and moves on to it (`webview::Remote`; `featherplay_platform::webview::follow` passes the
//! resizes and moves). Once the page thread has ended, the frame goes. The page is Windows
//! only (`Feature::WebViewSignIn`): elsewhere the window says it didn't open.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use featherplay_platform::window::{self, Handle, ScreenRect};
use slint::{CloseRequestResponse, ComponentHandle, LogicalSize, PhysicalPosition, Timer};

use super::address::Address;
use super::webview::Outcome;
#[cfg(windows)]
use super::webview::{self, Event, Room};
use crate::app::Ctx;
#[cfg(windows)]
use crate::chrome;
use crate::popup;
use crate::shell;
use crate::ui::{AppWindow, SettingsData, SignInWindow, Theme};

/// Logical size the window opens at.
pub const SIZE: (f32, f32) = (540.0, 780.0);

/// How long after the frame goes the log says what the app holds: by then WebView2's
/// processes and the page thread have ended.
const SETTLED: Duration = Duration::from_secs(60);

thread_local! {
    /// The frame while a sign-in runs.
    static FRAME: RefCell<Option<SignInWindow>> = const { RefCell::new(None) };
}

/// Builds the frame in the main window's theme, without showing it: `open` shows it, the
/// snapshot renders it as it is.
pub fn create(ctx: &Rc<Ctx>) -> Result<SignInWindow, String> {
    let main = ctx.window().ok_or("no main window")?;
    let ui = SignInWindow::new().map_err(|e| format!("window: {e}"))?;
    popup::sync_theme(
        &main.global::<Theme>(),
        &ui.global::<Theme>(),
        main.global::<SettingsData>().get_appearance(),
    );
    ui.window().set_size(LogicalSize::new(SIZE.0, SIZE.1));
    Ok(ui)
}

/// Shows `address` in the frame's pill.
pub fn show_address(ui: &SignInWindow, address: &Address) {
    ui.set_host(address.host.as_str().into());
    ui.set_path(address.path.as_str().into());
    ui.set_secure(address.secure);
    ui.set_google(address.google);
}

/// Opens the frame and Google's page in it at `start`; `done` runs on the sign-in thread
/// once the page has closed (the frame is gone by the time it's called back on the UI
/// thread through `closed`).
///
/// Winit makes the frame's window on a later turn of the event loop than `show`, so the
/// page goes in once its handle exists (`shell::when_handle`). An error returned here, or one
/// told to `super::didnt_open` later, means the window didn't open and `done` never runs.
pub fn open(
    ctx: &Rc<Ctx>,
    profile: PathBuf,
    start: &'static str,
    done: impl FnOnce(Outcome) + Send + 'static,
) -> Result<(), String> {
    if is_open() {
        return Err("the sign-in window is already open".into());
    }
    log!("sign-in window opening: private {:.0} MB", private_mb());
    let ui = create(ctx)?;
    if let Some(main) = ctx.window() {
        place_over_page(&main, &ui);
    }
    ui.show().map_err(|e| format!("show: {e}"))?;
    // Kept here while the window is made: `when_handle` holds it weakly.
    FRAME.with_borrow_mut(|f| *f = Some(ui.clone_strong()));
    // Closed before the page went in (`embed` replaces this): nothing to wait for, and
    // dropping the frame stops the wait.
    ui.window().on_close_requested(|| {
        let _ = slint::invoke_from_event_loop(|| super::window_closed(Outcome::Cancelled));
        CloseRequestResponse::HideWindow
    });
    shell::when_handle(
        &ui,
        move |ui, handle| {
            if let Err(e) = embed(ui, handle, profile, start, done) {
                closed();
                super::didnt_open(&e);
            }
        },
        |_| {
            closed();
            super::didnt_open("the sign-in window got no handle within 2 s");
        },
    );
    Ok(())
}

/// Once the frame's window exists: its colours, then Google's page in it.
#[cfg(windows)]
fn embed(
    ui: &SignInWindow,
    handle: Handle,
    profile: PathBuf,
    start: &'static str,
    done: impl FnOnce(Outcome) + Send + 'static,
) -> Result<(), String> {
    dress(ui, handle);
    let room = Room {
        top: ui.get_page_top(),
        bottom: ui.get_page_bottom(),
    };
    let remote = webview::open(handle, room, profile, start, tell, done)?;
    ui.on_back(move || remote.back());
    ui.on_reload(move || remote.reload());
    ui.window().on_close_requested(move || {
        if remote.is_alive() {
            // The page thread reads the cookies first; the frame goes once it has ended.
            remote.close();
            return CloseRequestResponse::KeepWindowShown;
        }
        // The page thread is gone without saying how it ended: end the sign-in here, so the
        // window can still be closed and the page stops waiting.
        let _ = slint::invoke_from_event_loop(|| {
            super::window_closed(Outcome::Failed("the sign-in window stopped".into()));
        });
        CloseRequestResponse::HideWindow
    });
    if let Err(e) = featherplay_platform::webview::follow(handle, remote) {
        log!("sign-in window: can't follow its size: {e}");
    }
    Ok(())
}

/// The page is Windows only: here the window didn't open.
#[cfg(not(windows))]
fn embed(
    _ui: &SignInWindow,
    _handle: Handle,
    _profile: PathBuf,
    _start: &'static str,
    _done: impl FnOnce(Outcome) + Send + 'static,
) -> Result<(), String> {
    use featherplay_platform::{Feature, Unsupported};
    Err(Unsupported(Feature::WebViewSignIn).to_string())
}

/// The frame's least height, logical pixels (`min-height` in `ui/sign_in_window.slint`).
const LEAST_H: f32 = 480.0;

/// Before it's shown, puts the frame's window over the main window's page: centred on the
/// room the page has (right of the sidebar, left of Up next while it's open, above the bar;
/// on the sign-in page, round its block), inside that screen's work area, and no taller than
/// it (down to its least height). With the main window minimised or not there yet, Windows
/// places it.
fn place_over_page(main: &AppWindow, ui: &SignInWindow) {
    let window = main.window();
    let Some(h) = shell::handle(window) else {
        return;
    };
    if window::is_minimised(h) {
        return;
    }
    let Some((left, top)) = window::client_origin(h) else {
        return;
    };
    let main_scale = window.scale_factor();
    let height = window.size().height as f32 / main_scale;
    let x = main.get_page_x() + main.get_page_width() / 2.0;
    let y = (height - main.global::<Theme>().get_bar_clear()) / 2.0;
    let centre = (
        left + (x * main_scale).round() as i32,
        top + (y * main_scale).round() as i32,
    );
    // The frame opens on that screen, at its scale.
    let Some((work, scale)) = window::work_area_at(centre.0, centre.1) else {
        return;
    };
    let px = |v: f32| (v * scale).round() as i32;
    let client = (px(SIZE.0), px(SIZE.1));
    let (x, y, client_h) = place(
        centre,
        client,
        px(LEAST_H),
        window::frame_insets(scale),
        work,
    );
    if client_h != client.1 {
        ui.window()
            .set_size(LogicalSize::new(SIZE.0, client_h as f32 / scale));
    }
    ui.window().set_position(PhysicalPosition::new(x, y));
}

/// Where a window goes whose client area is `client` big (at least `least` high) inside a
/// frame of `insets` (left, top, right, bottom), all in physical pixels: its frame's top-left
/// corner, centred on `centre`, title bar included, and moved inside the work area `work`;
/// and its client area's height, cut to fit the work area's (its title bar then at the top).
fn place(
    centre: (i32, i32),
    client: (i32, i32),
    least: i32,
    insets: ScreenRect,
    work: ScreenRect,
) -> (i32, i32, i32) {
    let (l, t, r, b) = insets;
    let (left, top, right, bottom) = work;
    let client_h = client.1.min(bottom - top - t - b).max(least);
    let (w, h) = (client.0 + l + r, client_h + t + b);
    let inside = |v: i32, lo: i32, hi: i32| v.min(hi).max(lo);
    (
        inside(centre.0 - w / 2, left, right - w),
        inside(centre.1 - h / 2, top, bottom - h),
        client_h,
    )
}

/// The frame is up, or waiting for its window.
pub fn is_open() -> bool {
    FRAME.with_borrow(Option::is_some)
}

/// On the UI thread, once the page thread has ended: the frame goes.
pub fn closed() {
    if let Some(ui) = FRAME.with_borrow_mut(Option::take) {
        let _ = ui.hide();
        // For a hand test: what the app holds once the window has had time to let go (not
        // if it has opened again meanwhile).
        Timer::single_shot(SETTLED, || {
            if !is_open() {
                log!(
                    "sign-in window closed a minute ago: private {:.0} MB",
                    private_mb()
                );
            }
        });
    }
}

fn private_mb() -> f64 {
    crate::memory::private_bytes() as f64 / 1024.0 / 1024.0
}

/// The title bar in the frame's colours, an opaque client area, the app's icon.
#[cfg(windows)]
fn dress(ui: &SignInWindow, handle: Handle) {
    let theme = ui.global::<Theme>();
    if let Err(e) = window::opaque(handle) {
        log!("sign-in window: opaque: {e}");
    }
    if let Err(e) = chrome::title_bar(
        handle,
        theme.get_dark(),
        theme.get_panel(),
        theme.get_text1(),
    ) {
        log!("sign-in window: title bar: {e}");
    }
    let _ = chrome::set_icon(handle, ui.window().scale_factor());
}

/// From the sign-in thread: shown on the UI thread.
#[cfg(windows)]
fn tell(event: Event) {
    let _ = slint::invoke_from_event_loop(move || {
        FRAME.with_borrow(|f| {
            let Some(ui) = f else { return };
            match event {
                Event::Address(a) => show_address(ui, &a),
                Event::CanGoBack(b) => ui.set_can_back(b),
                // Hidden at once; it goes once the profile is cleared (`closed`).
                Event::Closing => {
                    let _ = ui.hide();
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::place;

    // A 1920 x 1080 screen at 100 %, its taskbar 40 px; a window frame of 8 px with a 31 px
    // title bar; the frame's 540 x 780 client area, 480 high at least.
    const WORK: (i32, i32, i32, i32) = (0, 0, 1920, 1040);
    const INSETS: (i32, i32, i32, i32) = (8, 31, 8, 8);
    const CLIENT: (i32, i32) = (540, 780);
    const LEAST: i32 = 480;
    const OUTER: (i32, i32) = (556, 819);

    #[test]
    fn the_window_centres_on_the_room() {
        let (x, y, h) = place((1000, 500), CLIENT, LEAST, INSETS, WORK);
        assert_eq!((x + OUTER.0 / 2, y + OUTER.1 / 2), (1000, 500));
        assert_eq!(h, CLIENT.1);
    }

    #[test]
    fn the_window_stays_on_the_screen() {
        // A room near the right edge, the window moved in.
        assert_eq!(
            place((1800, 500), CLIENT, LEAST, INSETS, WORK).0,
            1920 - OUTER.0
        );
        // A second screen left of the first.
        assert_eq!(
            place((-1800, 500), CLIENT, LEAST, INSETS, (-1920, 0, 0, 1040)).0,
            -1920
        );
    }

    #[test]
    fn on_a_short_screen_the_window_fits_its_height() {
        // 1366 x 768 at 100 %: the window shrinks to the work area, its title bar at the top.
        let (_, y, h) = place((683, 350), CLIENT, LEAST, INSETS, (0, 0, 1366, 728));
        assert_eq!((y, h), (0, 728 - 39));
        // Shorter than its least height: that, from the top.
        let (_, y, h) = place((683, 250), CLIENT, LEAST, INSETS, (0, 0, 1366, 500));
        assert_eq!((y, h), (0, LEAST));
    }
}
