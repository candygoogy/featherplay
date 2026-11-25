//! The window's own title bar: the app draws it at the client area's top, and Windows still
//! owns the window. The caption goes from the frame (`WM_NCCALCSIZE`; the borders, the corners,
//! the shadow and the resize edges stay Windows'), and the hit test (`WM_NCHITTEST`) says what
//! each point of the app's bar is: the free space drags the window (double-click, the window
//! menu and Aero Snap come with it), the top band resizes it, and the window buttons are
//! Windows' own buttons to it, the maximise one showing Snap layouts. Their pointer messages
//! then come as non-client ones, which the subclass turns into the hover and press the app
//! draws, and the click into the system command.
//!
//! Mica later is the system backdrop and a see-through ground on top of this; nothing more
//! of the frame.

use std::cell::Cell;
use std::sync::OnceLock;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, ScreenToClient,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETAUTOHIDEBAREX, APPBARDATA, DefSubclassProc,
    RemoveWindowSubclass, SHAppBarMessage, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnableMenuItem, GWL_STYLE, GetForegroundWindow, GetSystemMenu, GetWindowLongPtrW, HTCAPTION,
    HTCLIENT, HTCLOSE, HTMAXBUTTON, HTMINBUTTON, HTTOP, IsZoomed, MF_BYCOMMAND, MF_ENABLED,
    MF_GRAYED, NCCALCSIZE_PARAMS, PostMessageW, RegisterWindowMessageW, SC_CLOSE, SC_MAXIMIZE,
    SC_MINIMIZE, SC_MOVE, SC_RESTORE, SC_SIZE, SET_WINDOW_POS_FLAGS, SIZE_MAXIMIZED,
    SIZE_MINIMIZED, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SendMessageW, SetWindowPos, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WA_INACTIVE,
    WM_ACTIVATE, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK,
    WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_NCRBUTTONUP,
    WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOLORCHANGE, WM_SYSCOMMAND, WS_CAPTION,
};

use super::Handle;
use crate::common::window::{CaptionButton, Contrast, Rgb, TitleBar, TitleHit, title_hit};

/// The subclass that draws no caption (`own_title_bar`).
const TITLE_SUBCLASS: usize = 0xfea7_0011;

/// How tall the band along the top that still resizes the window is, at 100 %.
const EDGE: u32 = 4;
/// How far a maximised window's client area keeps off an edge with an auto-hidden taskbar,
/// so the pointer there still brings the taskbar up.
const AUTOHIDE_GAP: i32 = 2;

/// What the app draws of the window buttons and the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptionState {
    /// The button under the pointer.
    pub hover: Option<CaptionButton>,
    /// The button pressed, until the button goes up.
    pub pressed: Option<CaptionButton>,
    /// The window is the active one; inactive, the name and the buttons fade.
    pub active: bool,
    /// Maximised: the maximise button shows Restore.
    pub maximised: bool,
}

/// What the subclass keeps.
struct Own {
    /// The app draws the title bar; off, it's Windows' own again.
    on: Cell<bool>,
    bar: Box<dyn Fn() -> TitleBar>,
    changed: Box<dyn Fn(CaptionState)>,
    state: Cell<CaptionState>,
    /// A `WM_NCMOUSELEAVE` was asked for and hasn't come.
    tracking: Cell<bool>,
}

impl Own {
    fn update(&self, f: impl FnOnce(&mut CaptionState)) {
        let mut s = self.state.get();
        f(&mut s);
        if s != self.state.get() {
            self.state.set(s);
            (self.changed)(s);
        }
    }
}

/// Lets the app draw the window's title bar while `on`: the caption leaves the frame, and
/// `bar` (asked at each hit test, physical pixels) says where the controls and the window
/// buttons are in the app's bar. `changed` hears the buttons' hover and press, and whether the
/// window is active and maximised, as they change (from the window procedure: keep it short).
/// Off, the window keeps Windows' title bar until `set_own_title_bar` turns it on.
pub fn own_title_bar(
    h: Handle,
    on: bool,
    bar: impl Fn() -> TitleBar + 'static,
    changed: impl Fn(CaptionState) + 'static,
) -> Result<(), String> {
    let hwnd = h.hwnd();
    // SAFETY: handle-only reads.
    let (active, maximised) = unsafe { (GetForegroundWindow() == hwnd, IsZoomed(hwnd).as_bool()) };
    let state = CaptionState {
        hover: None,
        pressed: None,
        active,
        maximised,
    };
    let data = Box::into_raw(Box::new(Own {
        on: Cell::new(on),
        bar: Box::new(bar),
        changed: Box::new(changed),
        state: Cell::new(state),
        tracking: Cell::new(false),
    }));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok = unsafe { SetWindowSubclass(hwnd, Some(title_proc), TITLE_SUBCLASS, data as usize) };
    if !ok.as_bool() {
        // SAFETY: never shared, the subclass wasn't installed.
        drop(unsafe { Box::from_raw(data) });
        return Err("SetWindowSubclass failed".into());
    }
    // SAFETY: just installed, alive until WM_NCDESTROY.
    (unsafe { &*data }.changed)(state);
    if on {
        frame_changed(h)?;
    }
    Ok(())
}

/// Turns the app's own title bar on or off in a window `own_title_bar` set up: the window
/// keeps its place and outer size, and its client area grows over the caption or gives it
/// back.
pub fn set_own_title_bar(h: Handle, on: bool) -> Result<(), String> {
    let msg = switch_message().ok_or("RegisterWindowMessage failed")?;
    // SAFETY: a window of this thread; the message carries a flag.
    let done = unsafe { SendMessageW(h.hwnd(), msg, Some(WPARAM(usize::from(on))), None) };
    if done.0 == 0 {
        return Err("no title bar subclass".into());
    }
    Ok(())
}

/// The message `set_own_title_bar` sends the subclass (registered: winit uses `WM_APP`'s).
fn switch_message() -> Option<u32> {
    static MSG: OnceLock<u32> = OnceLock::new();
    // SAFETY: a NUL-terminated static name.
    let msg = *MSG.get_or_init(|| unsafe {
        RegisterWindowMessageW(windows::core::w!("featherplay-own-title-bar"))
    });
    (msg != 0).then_some(msg)
}

/// Windows' contrast theme's colours while one is on (Settings > Accessibility > Contrast
/// themes): the window, its text, the highlight and the text on it.
pub fn contrast() -> Option<Contrast> {
    use windows::Win32::Graphics::Gdi::{
        COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor,
        SYS_COLOR_INDEX,
    };
    use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut hc = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    // SAFETY: SPI_GETHIGHCONTRAST fills a HIGHCONTRASTW of the size given.
    unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some(&mut hc as *mut _ as *mut std::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()?;
    if hc.dwFlags & HCF_HIGHCONTRASTON != HCF_HIGHCONTRASTON {
        return None;
    }
    let colour = |i: SYS_COLOR_INDEX| {
        // SAFETY: a plain lookup of a system colour (0x00BBGGRR).
        let c = unsafe { GetSysColor(i) };
        Rgb {
            red: (c & 0xff) as u8,
            green: (c >> 8 & 0xff) as u8,
            blue: (c >> 16 & 0xff) as u8,
        }
    };
    Some(Contrast {
        ground: colour(COLOR_WINDOW),
        text: colour(COLOR_WINDOWTEXT),
        highlight: colour(COLOR_HIGHLIGHT),
        on_highlight: colour(COLOR_HIGHLIGHTTEXT),
    })
}

/// What a window button does, as Windows' own buttons do it: posted, so it runs once the
/// message at hand (a click, a screen reader's action) is done.
pub fn caption_button(h: Handle, button: CaptionButton) {
    let hwnd = h.hwnd();
    let command = match button {
        CaptionButton::Minimise => SC_MINIMIZE,
        // SAFETY: a handle-only read.
        CaptionButton::Maximise if unsafe { IsZoomed(hwnd) }.as_bool() => SC_RESTORE,
        CaptionButton::Maximise => SC_MAXIMIZE,
        CaptionButton::Close => SC_CLOSE,
    };
    // SAFETY: a handle-only message; a stale handle fails.
    let _ = unsafe {
        PostMessageW(
            Some(hwnd),
            WM_SYSCOMMAND,
            WPARAM(command as usize),
            LPARAM(0),
        )
    };
}

/// Has Windows work out the window's frame again (`WM_NCCALCSIZE`), keeping its place, size
/// and order.
fn frame_changed(h: Handle) -> Result<(), String> {
    let flags: SET_WINDOW_POS_FLAGS =
        SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
    // SAFETY: a window of this thread and plain flags.
    unsafe { SetWindowPos(h.hwnd(), None, 0, 0, 0, 0, flags) }.map_err(|e| e.message())
}

/// The window button a non-client hit code names.
fn button_of(hit: usize) -> Option<CaptionButton> {
    match hit as u32 {
        HTMINBUTTON => Some(CaptionButton::Minimise),
        HTMAXBUTTON => Some(CaptionButton::Maximise),
        HTCLOSE => Some(CaptionButton::Close),
        _ => None,
    }
}

/// The hit code Windows takes for `hit`.
fn hit_code(hit: TitleHit) -> u32 {
    match hit {
        TitleHit::Client => HTCLIENT,
        TitleHit::Drag => HTCAPTION,
        TitleHit::Top => HTTOP,
        TitleHit::Button(CaptionButton::Minimise) => HTMINBUTTON,
        TitleHit::Button(CaptionButton::Maximise) => HTMAXBUTTON,
        TitleHit::Button(CaptionButton::Close) => HTCLOSE,
    }
}

unsafe extern "system" fn title_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        // SAFETY: the window is going; nothing calls this subclass with `data` again.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(title_proc), TITLE_SUBCLASS);
            drop(Box::from_raw(data as *mut Own));
        }
        // SAFETY: the arguments this subclass was called with.
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: `data` is the box from `own_title_bar`, alive until WM_NCDESTROY.
    let own = unsafe { &*(data as *const Own) };
    // SAFETY: the arguments this subclass was called with.
    let default = || unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    match msg {
        WM_ACTIVATE => own.update(|s| s.active = (wparam.0 & 0xffff) as u32 != WA_INACTIVE),
        // A contrast theme on, off or changed: the app reads its colours again.
        WM_SYSCOLORCHANGE | WM_SETTINGCHANGE => (own.changed)(own.state.get()),
        WM_SIZE if wparam.0 as u32 != SIZE_MINIMIZED => {
            own.update(|s| s.maximised = wparam.0 as u32 == SIZE_MAXIMIZED);
        }
        _ => {}
    }
    if Some(msg) == switch_message() {
        let on = wparam.0 != 0;
        if own.on.replace(on) != on {
            own.update(|s| {
                s.hover = None;
                s.pressed = None;
            });
            if let Err(e) = frame_changed(Handle::from_hwnd(hwnd)) {
                log::warn!("title bar: {e}");
            }
        }
        return LRESULT(1);
    }
    if !own.on.get() || full_screen(hwnd) {
        return default();
    }
    match msg {
        WM_NCCALCSIZE if wparam.0 != 0 => {
            let params = lparam.0 as *mut NCCALCSIZE_PARAMS;
            // SAFETY: with WPARAM true, LPARAM points at the NCCALCSIZE_PARAMS, whose first
            // rectangle is the window's proposed bounds; the default procedure makes it the
            // client area's.
            let outer = unsafe { (*params).rgrc[0] };
            let result = default();
            // SAFETY: as above, no other reference to it is alive.
            let params = unsafe { &mut *params };
            // Windows' frame on every side but the top, which the app's bar takes up to the
            // window's edge. Maximised, the frame hangs past the screen's edge by its border
            // on every side: the top keeps off it by as much as the left does.
            // SAFETY: a handle-only read.
            let maximised = unsafe { IsZoomed(hwnd) }.as_bool();
            let client = &mut params.rgrc[0];
            client.top = outer.top;
            if maximised {
                client.top += client.left - outer.left;
                keep_off_autohide(hwnd, client);
            }
            result
        }
        WM_NCHITTEST => {
            let hit = default();
            if hit.0 != HTCLIENT as isize {
                return hit;
            }
            let mut p = POINT {
                x: i32::from((lparam.0 & 0xffff) as u16 as i16),
                y: i32::from(((lparam.0 >> 16) & 0xffff) as u16 as i16),
            };
            // SAFETY: a handle and a point on the stack; a stale handle leaves it as it was.
            if !unsafe { ScreenToClient(hwnd, &mut p) }.as_bool() {
                return hit;
            }
            // SAFETY: a handle-only read.
            let edge = if unsafe { IsZoomed(hwnd) }.as_bool() {
                0
            } else {
                // SAFETY: a handle-only read.
                (EDGE * unsafe { GetDpiForWindow(hwnd) }).div_ceil(96) as i32
            };
            LRESULT(hit_code(title_hit(&(own.bar)(), p.x, p.y, edge)) as isize)
        }
        WM_NCMOUSEMOVE => {
            let over = button_of(wparam.0);
            own.update(|s| s.hover = over);
            if !own.tracking.replace(true) {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                // SAFETY: a filled-in TRACKMOUSEEVENT for a window of this thread.
                if unsafe { TrackMouseEvent(&mut track) }.is_err() {
                    own.tracking.set(false);
                }
            }
            // Windows would hot-track a caption button of its own there.
            if over.is_some() {
                LRESULT(0)
            } else {
                default()
            }
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => match button_of(wparam.0) {
            Some(b) => {
                own.update(|s| s.pressed = Some(b));
                LRESULT(0)
            }
            None => default(),
        },
        WM_NCLBUTTONUP => {
            let pressed = own.state.get().pressed;
            own.update(|s| s.pressed = None);
            match button_of(wparam.0) {
                Some(b) => {
                    if pressed == Some(b) {
                        caption_button(Handle::from_hwnd(hwnd), b);
                    }
                    LRESULT(0)
                }
                None => default(),
            }
        }
        // Right-click on the bar's free space: the window menu at the pointer, as on Windows'
        // caption (which opens it only over a caption of its own).
        WM_NCRBUTTONUP if wparam.0 as u32 == HTCAPTION => {
            let (x, y) = (
                i32::from((lparam.0 & 0xffff) as u16 as i16),
                i32::from(((lparam.0 >> 16) & 0xffff) as u16 as i16),
            );
            window_menu(hwnd, x, y);
            LRESULT(0)
        }
        WM_NCMOUSELEAVE => {
            own.tracking.set(false);
            own.update(|s| {
                s.hover = None;
                s.pressed = None;
            });
            default()
        }
        WM_MOUSEMOVE => {
            own.update(|s| s.hover = None);
            default()
        }
        _ => default(),
    }
}

/// The window menu (Restore, Move, Size, Minimise, Maximise, Close) at `(x, y)` on screen, its
/// entries enabled as Windows enables them for the window's state; the one picked is posted as
/// the system command.
fn window_menu(hwnd: HWND, x: i32, y: i32) {
    // SAFETY: handle-only calls on the window's own menu; a stale handle gives none.
    unsafe {
        let menu = GetSystemMenu(hwnd, false);
        if menu.is_invalid() {
            return;
        }
        let maximised = IsZoomed(hwnd).as_bool();
        for (item, on) in [
            (SC_RESTORE, maximised),
            (SC_MOVE, !maximised),
            (SC_SIZE, !maximised),
            (SC_MINIMIZE, true),
            (SC_MAXIMIZE, !maximised),
            (SC_CLOSE, true),
        ] {
            let state = if on { MF_ENABLED } else { MF_GRAYED };
            let _ = EnableMenuItem(menu, item, MF_BYCOMMAND | state);
        }
        let picked = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            x,
            y,
            None,
            hwnd,
            None,
        );
        if picked.0 != 0 {
            let _ = PostMessageW(
                Some(hwnd),
                WM_SYSCOMMAND,
                WPARAM(picked.0 as usize),
                LPARAM(0),
            );
        }
    }
}

/// The window is in full screen: winit's borderless full screen takes the caption and the
/// frame from its style. Windows' own procedure is right for it then: the client area is the
/// whole window, with no gap kept for an auto-hidden taskbar if it was maximised, and nothing
/// in it drags, resizes or is a window button.
fn full_screen(hwnd: HWND) -> bool {
    // SAFETY: a handle-only read.
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    style & WS_CAPTION.0 == 0
}

/// Keeps a maximised window's client area `AUTOHIDE_GAP` off each edge of its monitor that has
/// an auto-hidden taskbar: a client area over the whole screen reads as a full-screen app, and
/// the taskbar would stay hidden.
fn keep_off_autohide(hwnd: HWND, client: &mut RECT) {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: a handle-only call; NEAREST always returns a monitor.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    // SAFETY: a monitor handle and a MONITORINFO with its size filled in.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return;
    }
    for edge in [ABE_TOP, ABE_BOTTOM, ABE_LEFT, ABE_RIGHT] {
        let mut bar = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            uEdge: edge,
            rc: info.rcMonitor,
            ..Default::default()
        };
        // SAFETY: an APPBARDATA with its size, edge and monitor filled in.
        if unsafe { SHAppBarMessage(ABM_GETAUTOHIDEBAREX, &mut bar) } == 0 {
            continue;
        }
        match edge {
            ABE_TOP => client.top += AUTOHIDE_GAP,
            ABE_BOTTOM => client.bottom -= AUTOHIDE_GAP,
            ABE_LEFT => client.left += AUTOHIDE_GAP,
            _ => client.right -= AUTOHIDE_GAP,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowRect,
        RegisterClassW, SendMessageW, SetWindowLongPtrW, WINDOW_EX_STYLE, WNDCLASSW, WS_MAXIMIZE,
        WS_OVERLAPPEDWINDOW,
    };
    use windows::core::w;

    unsafe extern "system" fn plain(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        // SAFETY: the arguments the window procedure was called with.
        unsafe { DefWindowProcW(h, msg, w, l) }
    }

    /// A plain titled window (Windows' default procedure), never shown, 800 x 600 outside.
    fn window() -> HWND {
        let class = WNDCLASSW {
            lpfnWndProc: Some(plain),
            lpszClassName: w!("featherplay-title-bar-test"),
            ..Default::default()
        };
        // SAFETY: a filled-in class with a static name; registering it twice fails harmlessly.
        unsafe { RegisterClassW(&class) };
        // SAFETY: the class above and static strings.
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("featherplay-title-bar-test"),
                w!(""),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                800,
                600,
                None,
                None,
                None,
                None,
            )
        }
        .expect("window")
    }

    fn rects(h: HWND) -> (RECT, RECT) {
        let (mut outer, mut client) = (RECT::default(), RECT::default());
        // SAFETY: a live window and out-parameters.
        unsafe {
            GetWindowRect(h, &mut outer).expect("window rect");
            GetClientRect(h, &mut client).expect("client rect");
        }
        (outer, client)
    }

    #[test]
    fn the_caption_goes_and_comes_back_and_the_hit_test_follows_the_bar() {
        let h = window();
        let (outer, before) = rects(h);
        let seen = Rc::new(Cell::new(None));
        let s = seen.clone();
        own_title_bar(
            Handle::from_hwnd(h),
            true,
            || TitleBar {
                height: 48,
                buttons_x: 600,
                button_w: 46,
                controls: vec![(100, 200)],
            },
            move |state| s.set(Some(state)),
        )
        .expect("subclass");
        assert!(
            seen.get()
                .is_some_and(|s| !s.maximised && s.hover.is_none())
        );
        let (_, own) = rects(h);
        // The client area now runs to the window's top; the sides and the bottom keep the frame.
        assert_eq!(own.right, before.right);
        assert!(own.bottom > before.bottom, "{own:?} {before:?}");

        let mut origin = POINT::default();
        // SAFETY: a live window and a point on the stack.
        let _ = unsafe { windows::Win32::Graphics::Gdi::ClientToScreen(h, &mut origin) };
        assert_eq!(
            origin.y, outer.top,
            "the client area starts at the window's top"
        );
        let hit = |x: i32, y: i32| {
            let at = ((origin.y + y) as u16 as isize) << 16 | (origin.x + x) as u16 as isize;
            // SAFETY: a live window of this thread; WM_NCHITTEST carries a point.
            unsafe { SendMessageW(h, WM_NCHITTEST, None, Some(LPARAM(at))) }.0 as u32
        };
        assert_eq!(hit(50, 20), HTCAPTION);
        assert_eq!(hit(150, 20), HTCLIENT);
        assert_eq!(hit(650, 20), HTMAXBUTTON);
        assert_eq!(hit(50, 1), HTTOP);
        assert_eq!(hit(50, 100), HTCLIENT);

        set_own_title_bar(Handle::from_hwnd(h), false).expect("off");
        assert_eq!(rects(h).1, before, "Windows' caption is back");
        assert_eq!(hit(50, 20), HTCAPTION);

        // SAFETY: the window made above, on this thread.
        unsafe { DestroyWindow(h) }.expect("destroy");
    }

    #[test]
    fn a_full_screen_window_is_all_client_area_and_nothing_drags_it() {
        let h = window();
        own_title_bar(
            Handle::from_hwnd(h),
            true,
            || TitleBar {
                height: 48,
                buttons_x: 600,
                button_w: 46,
                controls: vec![],
            },
            |_| {},
        )
        .expect("subclass");
        // The style winit gives a maximised window going borderless full screen: no caption
        // and no frame, still maximised.
        // SAFETY: a live window of this thread; the frame is worked out again after it.
        unsafe {
            let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
            let full = (style & !WS_OVERLAPPEDWINDOW.0) | WS_MAXIMIZE.0;
            SetWindowLongPtrW(h, GWL_STYLE, full as isize);
        }
        frame_changed(Handle::from_hwnd(h)).expect("frame");
        let (outer, client) = rects(h);
        assert_eq!(
            (client.right, client.bottom),
            (outer.right - outer.left, outer.bottom - outer.top),
            "the client area is the whole window"
        );
        let at = (outer.top + 20) << 16 | (outer.left + 50);
        // SAFETY: a live window of this thread; WM_NCHITTEST carries a point.
        let hit = unsafe { SendMessageW(h, WM_NCHITTEST, None, Some(LPARAM(at as isize))) };
        assert_eq!(
            hit.0 as u32, HTCLIENT,
            "the bar's place doesn't drag the window"
        );

        // SAFETY: the window made above, on this thread.
        unsafe { DestroyWindow(h) }.expect("destroy");
    }
}
