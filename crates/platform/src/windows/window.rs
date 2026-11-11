//! Window operations on a window's `Handle` (its HWND here): a title bar in the app's
//! colours, an opaque surface that's repainted whole when Windows drops it, the window icon,
//! minimising it and bringing it forward, and watching its theme, displays, size, presses and
//! taskbar button; Windows' animation setting. For the tray's mini player: the work area and
//! scale of the screen under a point, the popup's border, whether a press is what brought it
//! to the front, which window is in front, and presses anywhere on screen. For the sign-in
//! window: where a window's client area is, and a window frame's size.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use raw_window_handle::RawWindowHandle;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_BB_ENABLE, DWM_BLURBEHIND, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE, DwmEnableBlurBehindWindow,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, GetUpdateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateIconFromResourceEx, DestroyIcon, GA_ROOT, GetAncestor, GetClassNameW,
    GetForegroundWindow, HC_ACTION, HHOOK, HICON, ICON_BIG, ICON_SMALL, IsIconic, LR_DEFAULTCOLOR,
    MSLLHOOKSTRUCT, RegisterWindowMessageW, SC_MINIMIZE, SC_RESTORE, SIZE_MAXIMIZED,
    SIZE_MINIMIZED, SIZE_RESTORED, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SendMessageW, SetForegroundWindow, SetWindowsHookExW, SystemParametersInfoW,
    UnhookWindowsHookEx, WH_MOUSE_LL, WINDOW_EX_STYLE, WM_DISPLAYCHANGE, WM_DPICHANGED,
    WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEACTIVATE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT,
    WM_POINTERACTIVATE, WM_POINTERDOWN, WM_RBUTTONDOWN, WM_SETICON, WM_SETTINGCHANGE, WM_SIZE,
    WM_SYSCOMMAND, WM_XBUTTONDOWN, WS_OVERLAPPEDWINDOW, WindowFromPoint,
};

pub use crate::common::window::*;

thread_local! {
    /// The icon handles `set_icon` made that a window may still hold; each is destroyed
    /// once WM_SETICON hands it back as replaced.
    static MADE: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
}

/// A window, as the window functions take it: here its HWND. Made from the windowing
/// library's raw handle (`from_raw`); its `Debug` prints the HWND in hex.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Handle(isize);

impl Handle {
    /// The window `raw` names, if it's a Win32 window.
    pub fn from_raw(raw: RawWindowHandle) -> Option<Handle> {
        match raw {
            RawWindowHandle::Win32(h) => Some(Handle(h.hwnd.get())),
            _ => None,
        }
    }

    pub(crate) fn from_hwnd(h: HWND) -> Handle {
        Handle(h.0 as isize)
    }

    pub(crate) fn hwnd(self) -> HWND {
        HWND(self.0 as *mut c_void)
    }
}

impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

/// The title bar in the app's own colours: dark or light caption buttons, the window's
/// ground as the caption colour and the main text colour for the title, so it matches the
/// app even when Windows' app mode differs. No Mica: its wallpaper tint didn't match the
/// opaque client area below it. Caption and text colours need Windows 11; older versions
/// keep the plain dark or light title bar.
pub fn title_bar(h: Handle, dark: bool, caption: Rgb, text: Rgb) -> Result<(), String> {
    set(
        h,
        DWMWA_USE_IMMERSIVE_DARK_MODE,
        &windows::core::BOOL::from(dark),
    )?;
    set(h, DWMWA_CAPTION_COLOR, &colorref(caption))?;
    set(h, DWMWA_TEXT_COLOR, &colorref(text))
}

/// Makes DWM ignore the client area's alpha. Slint asks winit for a transparent window,
/// which on Windows turns on blur-behind with an empty region: every pixel the renderer
/// hasn't painted (alpha 0) then shows the windows behind. The app is opaque throughout.
pub fn opaque(h: Handle) -> Result<(), String> {
    let off = DWM_BLURBEHIND {
        dwFlags: DWM_BB_ENABLE,
        fEnable: false.into(),
        ..Default::default()
    };
    // SAFETY: a live HWND and a filled-in DWM_BLURBEHIND.
    unsafe { DwmEnableBlurBehindWindow(h.hwnd(), &off) }.map_err(|e| e.message())
}

/// The subclass that watches paints (`repaint_when_invalidated`).
const REPAINT_SUBCLASS: usize = 0xfea7_0002;

/// Runs a WM_PAINT's drawing, the second argument (`repaint_when_invalidated`).
type AroundPaint = Box<dyn Fn(bool, &mut dyn FnMut())>;

/// What the paint subclass calls.
struct Paints {
    invalidated: Box<dyn Fn()>,
    around_paint: AroundPaint,
    wheel: Box<dyn Fn(i16)>,
}

/// Calls `invalidated` when Windows asks for a repaint of its own (restored from minimized,
/// a display change): a WM_PAINT with an update region. The renderer's own redraws carry
/// none (winit requests them with RDW_INTERNALPAINT). It runs before winit draws in that
/// same WM_PAINT, so a repaint of everything asked for there makes that frame whole.
///
/// Each WM_PAINT's drawing (winit's, inside the window procedure) runs inside
/// `around_paint(whole, paint)`, which should call `paint` once: a measurement can time it.
/// `whole` is whether that paint had an update region. `wheel` gets each WM_MOUSEWHEEL's
/// delta, in 120ths of a notch.
pub fn repaint_when_invalidated(
    h: Handle,
    invalidated: impl Fn() + 'static,
    around_paint: impl Fn(bool, &mut dyn FnMut()) + 'static,
    wheel: impl Fn(i16) + 'static,
) -> Result<(), String> {
    let data = Box::into_raw(Box::new(Paints {
        invalidated: Box::new(invalidated),
        around_paint: Box::new(around_paint),
        wheel: Box::new(wheel),
    }));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok = unsafe {
        SetWindowSubclass(
            h.hwnd(),
            Some(repaint_proc),
            REPAINT_SUBCLASS,
            data as usize,
        )
    };
    if !ok.as_bool() {
        // SAFETY: never shared, the subclass wasn't installed.
        drop(unsafe { Box::from_raw(data) });
        return Err("SetWindowSubclass failed".into());
    }
    Ok(())
}

unsafe extern "system" fn repaint_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if msg == WM_PAINT {
        // SAFETY: `data` is the box from `repaint_when_invalidated`, alive until WM_NCDESTROY.
        let paints = unsafe { &*(data as *const Paints) };
        // SAFETY: a live HWND; no rectangle is asked for, nothing is erased.
        let whole = unsafe { GetUpdateRect(hwnd, None, false) }.as_bool();
        if whole {
            (paints.invalidated)();
        }
        let mut result = None;
        (paints.around_paint)(whole, &mut || {
            // SAFETY: the arguments this subclass was called with.
            result = Some(unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) });
        });
        if let Some(result) = result {
            return result;
        }
    } else if msg == WM_MOUSEWHEEL {
        // SAFETY: `data` is the box from `repaint_when_invalidated`, alive until WM_NCDESTROY.
        let paints = unsafe { &*(data as *const Paints) };
        // The wheel delta, in 120ths of a notch: fractions come from a precision touchpad, a
        // free-spinning wheel or a mouse driver's own smooth scrolling.
        (paints.wheel)(((wparam.0 >> 16) & 0xffff) as u16 as i16);
    } else if msg == WM_NCDESTROY {
        // SAFETY: the window is going; nothing calls this subclass with `data` again.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(repaint_proc), REPAINT_SUBCLASS);
            drop(Box::from_raw(data as *mut Paints));
        }
    }
    // SAFETY: the arguments this subclass was called with.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// One DWM window attribute.
fn set<T>(h: Handle, attr: DWMWINDOWATTRIBUTE, value: &T) -> Result<(), String> {
    // SAFETY: a live HWND and a pointer to a value of the size given.
    unsafe {
        DwmSetWindowAttribute(
            h.hwnd(),
            attr,
            value as *const T as *const c_void,
            size_of::<T>() as u32,
        )
    }
    .map_err(|e| e.message())
}

/// A Win32 COLORREF (0x00BBGGRR).
fn colorref(c: Rgb) -> COLORREF {
    COLORREF(u32::from(c.red) | u32::from(c.green) << 8 | u32::from(c.blue) << 16)
}

/// Sets the title bar (small) and taskbar (big) icons, sized for `scale`, from the PNG
/// entries of `ico` (an `.ico` file's bytes). Call it again with the other drawing when the
/// taskbar's theme changes (`when_colours_change`).
pub fn set_icon(h: Handle, scale: f32, ico: &[u8]) -> Result<(), String> {
    for (which, base) in [(ICON_SMALL, 16.0), (ICON_BIG, 32.0)] {
        let px = (base * scale).round() as i32;
        let png = ico_entry(ico, px as u32).ok_or("no icon entry")?;
        // SAFETY: PNG icon bits (accepted since Vista); the icon lives as long as the window.
        let icon =
            unsafe { CreateIconFromResourceEx(png, true, 0x0003_0000, px, px, LR_DEFAULTCOLOR) }
                .map_err(|e| e.message())?;
        // SAFETY: a live HWND; WM_SETICON takes the icon handle as LPARAM and returns the
        // one it replaces.
        let old = unsafe {
            SendMessageW(
                h.hwnd(),
                WM_SETICON,
                Some(WPARAM(which as usize)),
                Some(LPARAM(icon.0 as isize)),
            )
        };
        MADE.with(|m| {
            let mut m = m.borrow_mut();
            m.push(icon.0 as isize);
            if old.0 != 0
                && let Some(i) = m.iter().position(|&o| o == old.0)
            {
                m.swap_remove(i);
                // SAFETY: an icon made here, which no window holds any more.
                let _ = unsafe { DestroyIcon(HICON(old.0 as *mut c_void)) };
            }
        });
    }
    Ok(())
}

/// The subclass that watches theme changes (`when_colours_change`).
const COLOURS_SUBCLASS: usize = 0xfea7_0003;

type Changed = Box<dyn Fn()>;

/// Calls `changed` when Windows' light/dark settings change (a WM_SETTINGCHANGE for
/// "ImmersiveColorSet"), so the window icon can follow the taskbar's theme.
pub fn when_colours_change(h: Handle, changed: impl Fn() + 'static) -> Result<(), String> {
    let data = Box::into_raw(Box::new(Box::new(changed) as Changed));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok = unsafe {
        SetWindowSubclass(
            h.hwnd(),
            Some(colours_proc),
            COLOURS_SUBCLASS,
            data as usize,
        )
    };
    if !ok.as_bool() {
        // SAFETY: never shared, the subclass wasn't installed.
        drop(unsafe { Box::from_raw(data) });
        return Err("SetWindowSubclass failed".into());
    }
    Ok(())
}

unsafe extern "system" fn colours_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if msg == WM_SETTINGCHANGE {
        // SAFETY: for WM_SETTINGCHANGE, LPARAM is null or a NUL-terminated UTF-16 string.
        if unsafe { is_colour_set(lparam) } {
            // SAFETY: `data` is the box from `when_colours_change`, alive until WM_NCDESTROY.
            let changed = unsafe { &*(data as *const Changed) };
            changed();
        }
    } else if msg == WM_NCDESTROY {
        // SAFETY: the window is going; nothing calls this subclass with `data` again.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(colours_proc), COLOURS_SUBCLASS);
            drop(Box::from_raw(data as *mut Changed));
        }
    }
    // SAFETY: the arguments this subclass was called with.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// The subclass that watches minimising (`when_minimised`).
const MINIMISED_SUBCLASS: usize = 0xfea7_0005;

type Minimised = Box<dyn Fn(bool)>;

/// Calls `changed(true)` when the window is minimised and `changed(false)` when it's restored
/// or maximised (WM_SIZE), after the window has handled the message. It may be called again
/// for the same state (each restore or resize sends one).
pub fn when_minimised(h: Handle, changed: impl Fn(bool) + 'static) -> Result<(), String> {
    let data = Box::into_raw(Box::new(Box::new(changed) as Minimised));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok = unsafe {
        SetWindowSubclass(
            h.hwnd(),
            Some(minimised_proc),
            MINIMISED_SUBCLASS,
            data as usize,
        )
    };
    if !ok.as_bool() {
        // SAFETY: never shared, the subclass wasn't installed.
        drop(unsafe { Box::from_raw(data) });
        return Err("SetWindowSubclass failed".into());
    }
    Ok(())
}

unsafe extern "system" fn minimised_proc(
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
            let _ = RemoveWindowSubclass(hwnd, Some(minimised_proc), MINIMISED_SUBCLASS);
            drop(Box::from_raw(data as *mut Minimised));
        }
        // SAFETY: the arguments this subclass was called with.
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: the arguments this subclass was called with.
    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    if msg == WM_SIZE {
        // SAFETY: `data` is the box from `when_minimised`, alive until WM_NCDESTROY.
        let changed = unsafe { &*(data as *const Minimised) };
        match wparam.0 as u32 {
            SIZE_MINIMIZED => changed(true),
            SIZE_RESTORED | SIZE_MAXIMIZED => changed(false),
            _ => {}
        }
    }
    result
}

/// The subclass that watches the displays (`when_displays_change`).
const DISPLAYS_SUBCLASS: usize = 0xfea7_0004;

/// Calls `changed` after something that may change the displays' scale: the window's own
/// scale (WM_DPICHANGED), a display added, removed or resized (WM_DISPLAYCHANGE), or a
/// setting (WM_SETTINGCHANGE, which a new scale on the primary display sends), so the tray
/// icon can be drawn for its new size. `changed` should be cheap when nothing changed.
pub fn when_displays_change(h: Handle, changed: impl Fn() + 'static) -> Result<(), String> {
    let data = Box::into_raw(Box::new(Box::new(changed) as Changed));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok = unsafe {
        SetWindowSubclass(
            h.hwnd(),
            Some(displays_proc),
            DISPLAYS_SUBCLASS,
            data as usize,
        )
    };
    if !ok.as_bool() {
        // SAFETY: never shared, the subclass wasn't installed.
        drop(unsafe { Box::from_raw(data) });
        return Err("SetWindowSubclass failed".into());
    }
    Ok(())
}

unsafe extern "system" fn displays_proc(
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
            let _ = RemoveWindowSubclass(hwnd, Some(displays_proc), DISPLAYS_SUBCLASS);
            drop(Box::from_raw(data as *mut Changed));
        }
        // SAFETY: the arguments this subclass was called with.
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    }
    // The window handles the message first, then the icon follows.
    // SAFETY: the arguments this subclass was called with.
    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    if matches!(msg, WM_DPICHANGED | WM_DISPLAYCHANGE | WM_SETTINGCHANGE) {
        // SAFETY: `data` is the box from `when_displays_change`, alive until WM_NCDESTROY.
        let changed = unsafe { &*(data as *const Changed) };
        changed();
    }
    result
}

/// The subclass that watches the user moving or sizing the window (`when_sizing`). Far from
/// the others' numbers, which other changes may add to.
const SIZING_SUBCLASS: usize = 0xfea7_0010;

type Sizing = Box<dyn Fn(bool)>;

/// Calls `changed(true)` when the user starts moving or sizing the window by its frame or
/// title bar (WM_ENTERSIZEMOVE, before the window handles it) and `changed(false)` when they
/// let go (WM_EXITSIZEMOVE, after). Windows runs its own loop in between, applying the
/// drag's next step once the window has painted the last one.
pub fn when_sizing(h: Handle, changed: impl Fn(bool) + 'static) -> Result<(), String> {
    let data = Box::into_raw(Box::new(Box::new(changed) as Sizing));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok =
        unsafe { SetWindowSubclass(h.hwnd(), Some(sizing_proc), SIZING_SUBCLASS, data as usize) };
    if !ok.as_bool() {
        // SAFETY: never shared, the subclass wasn't installed.
        drop(unsafe { Box::from_raw(data) });
        return Err("SetWindowSubclass failed".into());
    }
    Ok(())
}

unsafe extern "system" fn sizing_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE};
    if msg == WM_NCDESTROY {
        // SAFETY: the window is going; nothing calls this subclass with `data` again.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(sizing_proc), SIZING_SUBCLASS);
            drop(Box::from_raw(data as *mut Sizing));
        }
        // SAFETY: the arguments this subclass was called with.
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: `data` is the box from `when_sizing`, alive until WM_NCDESTROY.
    let changed = unsafe { &*(data as *const Sizing) };
    if msg == WM_ENTERSIZEMOVE {
        changed(true);
    }
    // SAFETY: the arguments this subclass was called with.
    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    if msg == WM_EXITSIZEMOVE {
        changed(false);
    }
    result
}

/// Whether a WM_SETTINGCHANGE's LPARAM names "ImmersiveColorSet".
///
/// # Safety
/// `lparam` is null or points to a NUL-terminated UTF-16 string.
unsafe fn is_colour_set(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    let name = windows::core::PCWSTR(lparam.0 as *const u16);
    let want = windows::core::w!("ImmersiveColorSet");
    // SAFETY: the caller's contract; `w!` is a NUL-terminated static.
    unsafe { name.as_wide() == want.as_wide() }
}

/// Restores the window if it's minimized and brings it to the front. The restore goes the
/// way the title bar's and taskbar's do (WM_SYSCOMMAND), so winit's own minimised state
/// follows and Windows plays its restore animation where it plays one; a window minimised
/// from maximised comes back maximised.
pub fn bring_forward(h: Handle) {
    // SAFETY: handle-only calls; a stale handle makes them fail, nothing is dereferenced.
    unsafe {
        if IsIconic(h.hwnd()).as_bool() {
            SendMessageW(
                h.hwnd(),
                WM_SYSCOMMAND,
                Some(WPARAM(SC_RESTORE as usize)),
                None,
            );
        }
        let _ = SetForegroundWindow(h.hwnd());
    }
}

/// Minimises the window as its title bar button does (WM_SYSCOMMAND, so winit's state
/// follows), with Windows' minimise animation where it plays one.
pub fn minimise(h: Handle) {
    // SAFETY: a handle-only message; a stale handle makes it fail.
    unsafe {
        SendMessageW(
            h.hwnd(),
            WM_SYSCOMMAND,
            Some(WPARAM(SC_MINIMIZE as usize)),
            None,
        );
    }
}

/// The subclass that watches presses (`when_pressed`).
const PRESSED_SUBCLASS: usize = 0xfea7_0007;

/// Whether an activation by a press came since the last press, and the callback, for
/// `pressed_proc`.
type Pressed = (Cell<bool>, Box<dyn Fn(bool)>);

/// Calls `pressed(from_behind)` each time a mouse button, a finger or a pen goes down in the
/// window: `from_behind` when that press is what activates it (Windows sends an inactive
/// window WM_MOUSEACTIVATE, or WM_POINTERACTIVATE for touch and pen, just before the press
/// reaches it). By the time the press arrives, the window is already in front either way.
pub fn when_pressed(h: Handle, pressed: impl Fn(bool) + 'static) -> Result<(), String> {
    let data = Box::into_raw(Box::new((
        Cell::new(false),
        Box::new(pressed) as Box<dyn Fn(bool)>,
    )));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok = unsafe {
        SetWindowSubclass(
            h.hwnd(),
            Some(pressed_proc),
            PRESSED_SUBCLASS,
            data as usize,
        )
    };
    if !ok.as_bool() {
        // SAFETY: never shared, the subclass wasn't installed.
        drop(unsafe { Box::from_raw(data) });
        return Err("SetWindowSubclass failed".into());
    }
    Ok(())
}

unsafe extern "system" fn pressed_proc(
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
            let _ = RemoveWindowSubclass(hwnd, Some(pressed_proc), PRESSED_SUBCLASS);
            drop(Box::from_raw(data as *mut Pressed));
        }
    } else if matches!(msg, WM_MOUSEACTIVATE | WM_POINTERACTIVATE) {
        // SAFETY: `data` is the box from `when_pressed`, alive until WM_NCDESTROY.
        let (activating, _) = unsafe { &*(data as *const Pressed) };
        activating.set(true);
    } else if matches!(
        msg,
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN | WM_POINTERDOWN
    ) {
        // SAFETY: `data` is the box from `when_pressed`, alive until WM_NCDESTROY.
        let (activating, pressed) = unsafe { &*(data as *const Pressed) };
        pressed(activating.take());
    }
    // SAFETY: the arguments this subclass was called with.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// The window in front (`GetForegroundWindow`) and its class, or `None` while no window is
/// in front, as for a moment while Windows switches.
pub fn foreground() -> Option<Foreground> {
    // SAFETY: no preconditions; it returns null when no window is in front.
    let h = unsafe { GetForegroundWindow() };
    if h.is_invalid() {
        return None;
    }
    Some(Foreground {
        window: Handle::from_hwnd(h),
        class: class_of(h),
    })
}

/// A window's class, as `GetClassNameW` gives it; empty for no window or a stale handle.
fn class_of(h: HWND) -> String {
    let mut name = [0u16; 256];
    // SAFETY: a window handle and a buffer of the length the slice gives; a null or stale
    // handle makes it return 0.
    let len = unsafe { GetClassNameW(h, &mut name) };
    let len = usize::try_from(len).unwrap_or(0).min(name.len());
    String::from_utf16_lossy(&name[..len])
}

thread_local! {
    /// What `press_hook` tells of each press, while a `PressWatch` lives on this thread.
    static ON_PRESS: Cell<Option<fn(&str)>> = const { Cell::new(None) };
}

/// The low-level mouse hook `watch_presses` installed; dropping it removes the hook.
pub struct PressWatch(HHOOK);

impl Drop for PressWatch {
    fn drop(&mut self) {
        ON_PRESS.set(None);
        // SAFETY: the hook `watch_presses` installed on this thread, removed once, here.
        let _ = unsafe { UnhookWindowsHookEx(self.0) };
    }
}

/// Calls `on_press(class)` each time a mouse button goes down anywhere on screen, with the
/// class of the top-level window under it (a touch or pen tap may never reach the hook), until the returned watch is dropped. One watch at a time per thread, on
/// a thread that pumps messages (the UI thread): Windows calls the hook from its message
/// loop, before the press reaches the window, and waits for it, so `on_press` must be quick.
pub fn watch_presses(on_press: fn(&str)) -> Result<PressWatch, String> {
    // SAFETY: no preconditions; null asks for the exe's own module.
    let module = unsafe { GetModuleHandleW(None) }.map_err(|e| format!("module: {e}"))?;
    // SAFETY: `press_hook` has the hook procedure's signature and lives as long as the exe.
    let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(press_hook), Some(module.into()), 0) }
        .map_err(|e| format!("SetWindowsHookExW: {e}"))?;
    ON_PRESS.set(Some(on_press));
    Ok(PressWatch(hook))
}

unsafe extern "system" fn press_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let down = matches!(
        u32::try_from(wparam.0),
        Ok(WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN)
    );
    if let (true, Ok(HC_ACTION), Some(on_press)) = (down, u32::try_from(code), ON_PRESS.get()) {
        // SAFETY: for HC_ACTION, `lparam` points at this event's MSLLHOOKSTRUCT.
        let at = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt };
        // SAFETY: plain lookups; no window under the point gives null, whose class is empty.
        let root = unsafe { GetAncestor(WindowFromPoint(at), GA_ROOT) };
        on_press(&class_of(root));
    }
    // SAFETY: the arguments this hook was called with.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Whether the window is minimised (hidden or not).
pub fn is_minimised(h: Handle) -> bool {
    // SAFETY: a handle-only call; a stale handle reads as not minimised.
    unsafe { IsIconic(h.hwnd()) }.as_bool()
}

/// Where the window's client area starts on screen, in physical pixels.
pub fn client_origin(h: Handle) -> Option<(i32, i32)> {
    let mut p = POINT::default();
    // SAFETY: a handle and an out-parameter; a stale handle fails.
    unsafe { ClientToScreen(h.hwnd(), &mut p) }
        .as_bool()
        .then_some((p.x, p.y))
}

/// The frame Windows draws round a titled, resizable window's client area at `scale` (the
/// borders, invisible ones included, and the title bar): left, top, right, bottom, in
/// physical pixels; none where Windows can't say. The style is the one winit gives a window
/// with decorations that can be resized (the sign-in window); a fixed-size one has a thinner
/// frame.
pub fn frame_insets(scale: f32) -> ScreenRect {
    let mut r = RECT::default();
    let dpi = (scale * 96.0).round() as u32;
    // SAFETY: an in-out rectangle; the styles are plain flags.
    match unsafe {
        AdjustWindowRectExForDpi(&mut r, WS_OVERLAPPEDWINDOW, false, WINDOW_EX_STYLE(0), dpi)
    } {
        Ok(()) => (-r.left, -r.top, r.right, r.bottom),
        Err(_) => (0, 0, 0, 0),
    }
}

/// The subclass that watches the taskbar button (`when_taskbar_button`).
const BUTTON_SUBCLASS: usize = 0xfea7_0006;

/// The registered "TaskbarButtonCreated" message and the callback, for `button_proc`.
type Button = (u32, Box<dyn Fn()>);

/// Calls `created` each time the taskbar has made the window's button ("TaskbarButtonCreated":
/// when the window shows, and again after Explorer restarts), after the window has handled it.
pub fn when_taskbar_button(h: Handle, created: impl Fn() + 'static) -> Result<(), String> {
    // SAFETY: a NUL-terminated static name.
    let msg = unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarButtonCreated")) };
    if msg == 0 {
        return Err("RegisterWindowMessage(TaskbarButtonCreated) failed".into());
    }
    let data = Box::into_raw(Box::new((msg, Box::new(created) as Box<dyn Fn()>)));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok =
        unsafe { SetWindowSubclass(h.hwnd(), Some(button_proc), BUTTON_SUBCLASS, data as usize) };
    if !ok.as_bool() {
        // SAFETY: never shared, the subclass wasn't installed.
        drop(unsafe { Box::from_raw(data) });
        return Err("SetWindowSubclass failed".into());
    }
    Ok(())
}

unsafe extern "system" fn button_proc(
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
            let _ = RemoveWindowSubclass(hwnd, Some(button_proc), BUTTON_SUBCLASS);
            drop(Box::from_raw(data as *mut Button));
        }
        // SAFETY: the arguments this subclass was called with.
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: the arguments this subclass was called with.
    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    // SAFETY: `data` is the box from `when_taskbar_button`, alive until WM_NCDESTROY.
    let (created_msg, created) = unsafe { &*(data as *const Button) };
    if msg == *created_msg {
        created();
    }
    result
}

/// The work area (the screen less the taskbar) of the monitor nearest to a point, and that
/// monitor's scale factor, both in physical pixels.
pub fn work_area_at(x: i32, y: i32) -> Option<(ScreenRect, f32)> {
    // SAFETY: MonitorFromPoint has no preconditions; NEAREST always returns a monitor.
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: a monitor handle and a MONITORINFO with its size filled in.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    let (mut dpi_x, mut dpi_y) = (96, 96);
    // SAFETY: a monitor handle and two out-parameters.
    let scale =
        match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } {
            Ok(()) => dpi_x as f32 / 96.0,
            Err(_) => 1.0,
        };
    let w = info.rcWork;
    Some(((w.left, w.top, w.right, w.bottom), scale))
}

/// The border Windows 11 draws around a rounded popup, in the app's hairline colour rather
/// than the system's accent-or-grey one. Older versions have no such border.
pub fn popup_border(h: Handle, colour: Rgb) -> Result<(), String> {
    set(h, DWMWA_BORDER_COLOR, &colorref(colour))
}

/// Windows' "Animation effects" setting (Settings > Accessibility > Visual effects).
pub fn animations_enabled() -> bool {
    let mut on = windows::core::BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut _ as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_err() || on.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorref_is_bgr() {
        assert_eq!(
            colorref(Rgb {
                red: 0x12,
                green: 0x34,
                blue: 0x56
            })
            .0,
            0x0056_3412
        );
    }

    #[test]
    fn a_press_right_after_a_mouse_or_pointer_activation_is_from_behind() {
        use std::rc::Rc;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED,
        };
        use windows::core::w;

        // A window of the system's static class, never shown.
        // SAFETY: a system class and static strings.
        let h = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            )
        }
        .expect("window");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s = seen.clone();
        when_pressed(Handle::from_hwnd(h), move |from_behind| {
            s.borrow_mut().push(from_behind);
        })
        .expect("subclass");
        let send = |msg| {
            // SAFETY: a live window of this thread; these messages carry no pointers.
            unsafe { SendMessageW(h, msg, None, None) };
        };

        send(WM_MOUSEACTIVATE);
        send(WM_LBUTTONDOWN);
        send(WM_LBUTTONDOWN);
        send(WM_POINTERACTIVATE);
        send(WM_POINTERDOWN);
        send(WM_RBUTTONDOWN);
        send(WM_MOUSEACTIVATE);
        send(WM_XBUTTONDOWN);
        send(WM_MBUTTONDOWN);
        assert_eq!(*seen.borrow(), [true, false, true, false, true, false]);

        // SAFETY: the window made above, on this thread.
        unsafe { DestroyWindow(h) }.expect("destroy");
        assert_eq!(
            Rc::strong_count(&seen),
            1,
            "the subclass outlived its window"
        );
    }

    #[test]
    fn only_immersive_colour_set_is_a_colour_change() {
        let name = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let set = name("ImmersiveColorSet");
        let other = name("Environment");
        // SAFETY: NUL-terminated UTF-16 strings, and null.
        unsafe {
            assert!(is_colour_set(LPARAM(set.as_ptr() as isize)));
            assert!(!is_colour_set(LPARAM(other.as_ptr() as isize)));
            assert!(!is_colour_set(LPARAM(0)));
        }
    }
}
