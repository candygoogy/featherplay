//! Win32 touches on the main window: a title bar in the app's colours, an opaque surface
//! that's repainted whole when Windows drops it, the Tri-vane icon for the title bar and
//! taskbar (dark or light, as the taskbar is), minimising it and bringing it forward, its
//! taskbar button showing up, and Windows' animation setting. For the tray's mini player:
//! the work area and scale of the screen under a point, the popup's border, whether a press
//! is what brought it to the front, which window is in front, and presses anywhere on screen.
//! For the sign-in window: where the main window's client area is, and a window frame's size.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use slint::Color;
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

/// The Tri-vane logo, 16 to 256 px, from `build.rs`, drawn for dark grounds: the play sits
/// in a clearance ring of the ground's colour. The exe's embedded icon is this one, and
/// `identity` writes it out for the media card's registration.
pub(crate) const ICON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/featherplay.ico"));

/// The light variant, for a light taskbar: the ring round the play is a deep crimson, so
/// the white play doesn't read as a hole on white.
const ICON_LIGHT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/featherplay-light.ico"));

/// The icon for the taskbar's theme.
fn icon_for(light_taskbar: bool) -> &'static [u8] {
    if light_taskbar { ICON_LIGHT } else { ICON }
}

thread_local! {
    /// The icon handles `set_icon` made that a window may still hold; each is destroyed
    /// once WM_SETICON hands it back as replaced.
    static MADE: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
}

fn hwnd(h: isize) -> HWND {
    HWND(h as *mut c_void)
}

/// The title bar in the app's own colours: dark or light caption buttons, the window's
/// ground as the caption colour and the main text colour for the title, so it matches the
/// app even when Windows' app mode differs. No Mica: its wallpaper tint didn't match the
/// opaque client area below it. Caption and text colours need Windows 11; older versions
/// keep the plain dark or light title bar.
pub fn title_bar(h: isize, dark: bool, caption: Color, text: Color) -> Result<(), String> {
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
pub fn opaque(h: isize) -> Result<(), String> {
    let off = DWM_BLURBEHIND {
        dwFlags: DWM_BB_ENABLE,
        fEnable: false.into(),
        ..Default::default()
    };
    // SAFETY: a live HWND and a filled-in DWM_BLURBEHIND.
    unsafe { DwmEnableBlurBehindWindow(hwnd(h), &off) }.map_err(|e| e.message())
}

/// The subclass that watches paints (`repaint_when_invalidated`).
const REPAINT_SUBCLASS: usize = 0xfea7_0002;

type Invalidated = Box<dyn Fn()>;

/// Calls `invalidated` when Windows asks for a repaint of its own (restored from minimized,
/// a display change): a WM_PAINT with an update region. The renderer's own redraws carry
/// none (winit requests them with RDW_INTERNALPAINT). It runs before winit draws in that
/// same WM_PAINT, so `repaint::all` there makes that frame whole.
pub fn repaint_when_invalidated(h: isize, invalidated: impl Fn() + 'static) -> Result<(), String> {
    let data = Box::into_raw(Box::new(Box::new(invalidated) as Invalidated));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok =
        unsafe { SetWindowSubclass(hwnd(h), Some(repaint_proc), REPAINT_SUBCLASS, data as usize) };
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
        let invalidated = unsafe { &*(data as *const Invalidated) };
        // SAFETY: a live HWND; no rectangle is asked for, nothing is erased.
        let whole = unsafe { GetUpdateRect(hwnd, None, false) }.as_bool();
        if whole {
            invalidated();
        }
        if frame_log::enabled() {
            // winit draws inside this WM_PAINT: Slint's render and softbuffer's BitBlt.
            let started = frame_log::Mark::now();
            // SAFETY: the arguments this subclass was called with.
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            frame_log::painted(started, whole);
            return result;
        }
    } else if msg == WM_MOUSEWHEEL && frame_log::enabled() {
        // The wheel delta, in 120ths of a notch: fractions come from a precision touchpad, a
        // free-spinning wheel or a mouse driver's own smooth scrolling.
        frame_log::wheel(((wparam.0 >> 16) & 0xffff) as u16 as i16);
    } else if msg == WM_NCDESTROY {
        // SAFETY: the window is going; nothing calls this subclass with `data` again.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(repaint_proc), REPAINT_SUBCLASS);
            drop(Box::from_raw(data as *mut Invalidated));
        }
    }
    // SAFETY: the arguments this subclass was called with.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// The real window's frames, for measuring smoothness on the user's own screen: with
/// `FEATHERPLAY_FRAME_LOG` set, logged once a second while frames come. Off (the default), a
/// WM_PAINT or a wheel message costs one read of a cached flag.
///
/// Each line has how many WM_PAINTs there were, how long each took (Slint's render plus
/// softbuffer's BitBlt), how much of the window each one copied (the renderer's damage, in
/// millions of physical pixels: a frame that repaints across the window shows here), the gaps
/// between frames in a run of them, the whole-window repaints from an update region, the wheel messages (and how many carried a fraction of a notch), and
/// the UI thread's own CPU time inside those paints: user time is Slint's work, kernel time is
/// mostly GDI's copy to the window, and the rest of the paint time was spent waiting
/// (preempted, or blocked in the copy). Windows counts thread time in whole clock ticks
/// (15.6 ms), so only the sums over the second mean something, not single frames.
mod frame_log {
    use std::cell::RefCell;
    use std::sync::OnceLock;
    use std::sync::atomic::Ordering;

    use i_slint_renderer_skia::software_surface::PRESENTED_PIXELS;
    use std::time::{Duration, Instant};

    /// Frames further apart than this start a new run (the window went idle in between).
    const RUN_GAP: Duration = Duration::from_millis(100);

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> isize;
        fn GetThreadTimes(
            thread: isize,
            creation: *mut u64,
            exit: *mut u64,
            kernel: *mut u64,
            user: *mut u64,
        ) -> i32;
    }

    /// A moment on this thread: the clock, its user and kernel time so far (100 ns units), and
    /// the pixels presented so far.
    #[derive(Clone, Copy)]
    pub struct Mark {
        at: Instant,
        user: u64,
        kernel: u64,
        pixels: u64,
    }

    impl Mark {
        pub fn now() -> Self {
            let (mut creation, mut exit, mut kernel, mut user) = (0, 0, 0, 0);
            // SAFETY: the calling thread's pseudo-handle and four valid, aligned out pointers
            // (a FILETIME is a little-endian u64 in two halves).
            unsafe {
                GetThreadTimes(
                    GetCurrentThread(),
                    &mut creation,
                    &mut exit,
                    &mut kernel,
                    &mut user,
                )
            };
            Self {
                at: Instant::now(),
                user,
                kernel,
                pixels: PRESENTED_PIXELS.load(Ordering::Relaxed),
            }
        }
    }

    #[derive(Default)]
    struct Window {
        since: Option<Instant>,
        last: Option<Instant>,
        paints: Vec<f64>,
        /// Millions of physical pixels each paint copied to the window.
        copied: Vec<f64>,
        gaps: Vec<f64>,
        whole: u32,
        /// Wheel messages, and how many of them carried a fraction of a notch.
        wheels: u32,
        fractional: u32,
        /// This thread's CPU time inside the paints, in 100 ns units.
        user: u64,
        kernel: u64,
    }

    thread_local! {
        static LOG: RefCell<Window> = RefCell::new(Window::default());
    }

    pub fn enabled() -> bool {
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("FEATHERPLAY_FRAME_LOG").is_some())
    }

    fn quantile(v: &mut [f64], q: f64) -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * q).round() as usize]
    }

    pub fn wheel(delta: i16) {
        LOG.with_borrow_mut(|w| {
            w.wheels += 1;
            w.fractional += u32::from(delta % 120 != 0);
        });
    }

    pub fn painted(started: Mark, whole: bool) {
        let ended = Mark::now();
        let ms = (ended.at - started.at).as_secs_f64() * 1000.0;
        LOG.with_borrow_mut(|w| {
            if let Some(last) = w.last {
                let gap = started.at - last;
                if gap < RUN_GAP {
                    w.gaps.push(gap.as_secs_f64() * 1000.0);
                }
            }
            w.last = Some(started.at);
            w.paints.push(ms);
            w.copied
                .push(ended.pixels.saturating_sub(started.pixels) as f64 / 1e6);
            w.whole += u32::from(whole);
            w.user += ended.user.saturating_sub(started.user);
            w.kernel += ended.kernel.saturating_sub(started.kernel);
            let since = *w.since.get_or_insert(started.at);
            if started.at - since >= Duration::from_secs(1) {
                let frames = w.paints.len();
                let painting: f64 = w.paints.iter().sum();
                let slow = w.gaps.iter().filter(|&&g| g > 8.4).count();
                let (p50, p95, max) = (
                    quantile(&mut w.paints, 0.5),
                    quantile(&mut w.paints, 0.95),
                    quantile(&mut w.paints, 1.0),
                );
                let (g50, gmax) = (quantile(&mut w.gaps, 0.5), quantile(&mut w.gaps, 1.0));
                let (c50, cmax) = (
                    quantile(&mut w.copied, 0.5),
                    quantile(&mut w.copied, 1.0),
                );
                let (user, kernel) = (w.user as f64 / 1e4, w.kernel as f64 / 1e4);
                log!(
                    "frames: {frames} in {:.2} s, paint median {p50:.2} p95 {p95:.2} max {max:.2} ms, copied median {c50:.2} max {cmax:.2} Mpx, gap median {g50:.2} max {gmax:.2} ms, gaps over 8.4 ms {slow}, whole-window {}, wheel messages {} ({} fractional); painting took {painting:.0} ms, of it CPU {user:.0} ms user + {kernel:.0} ms kernel",
                    (started.at - since).as_secs_f64(),
                    w.whole,
                    w.wheels,
                    w.fractional
                );
                *w = Window {
                    last: w.last,
                    ..Window::default()
                };
            }
        });
    }
}

/// One DWM window attribute.
fn set<T>(h: isize, attr: DWMWINDOWATTRIBUTE, value: &T) -> Result<(), String> {
    // SAFETY: a live HWND and a pointer to a value of the size given.
    unsafe {
        DwmSetWindowAttribute(
            hwnd(h),
            attr,
            value as *const T as *const c_void,
            size_of::<T>() as u32,
        )
    }
    .map_err(|e| e.message())
}

/// A Win32 COLORREF (0x00BBGGRR).
fn colorref(c: Color) -> COLORREF {
    COLORREF(u32::from(c.red()) | u32::from(c.green()) << 8 | u32::from(c.blue()) << 16)
}

/// Sets the title bar (small) and taskbar (big) icons, sized for `scale`, in the drawing for
/// the taskbar's theme. Call it again when that theme changes (`when_colours_change`).
pub fn set_icon(h: isize, scale: f32) -> Result<(), String> {
    let ico = icon_for(featherplay_integrations::light_taskbar());
    for (which, base) in [(ICON_SMALL, 16.0), (ICON_BIG, 32.0)] {
        let px = (base * scale).round() as i32;
        let png = entry(ico, px as u32).ok_or("no icon entry")?;
        // SAFETY: PNG icon bits (accepted since Vista); the icon lives as long as the window.
        let icon =
            unsafe { CreateIconFromResourceEx(png, true, 0x0003_0000, px, px, LR_DEFAULTCOLOR) }
                .map_err(|e| e.message())?;
        // SAFETY: a live HWND; WM_SETICON takes the icon handle as LPARAM and returns the
        // one it replaces.
        let old = unsafe {
            SendMessageW(
                hwnd(h),
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
pub fn when_colours_change(h: isize, changed: impl Fn() + 'static) -> Result<(), String> {
    let data = Box::into_raw(Box::new(Box::new(changed) as Changed));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok =
        unsafe { SetWindowSubclass(hwnd(h), Some(colours_proc), COLOURS_SUBCLASS, data as usize) };
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
pub fn when_minimised(h: isize, changed: impl Fn(bool) + 'static) -> Result<(), String> {
    let data = Box::into_raw(Box::new(Box::new(changed) as Minimised));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok = unsafe {
        SetWindowSubclass(
            hwnd(h),
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
pub fn when_displays_change(h: isize, changed: impl Fn() + 'static) -> Result<(), String> {
    let data = Box::into_raw(Box::new(Box::new(changed) as Changed));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok = unsafe {
        SetWindowSubclass(
            hwnd(h),
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
pub fn when_sizing(h: isize, changed: impl Fn(bool) + 'static) -> Result<(), String> {
    let data = Box::into_raw(Box::new(Box::new(changed) as Sizing));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok =
        unsafe { SetWindowSubclass(hwnd(h), Some(sizing_proc), SIZING_SUBCLASS, data as usize) };
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

/// The PNG of the smallest entry at least `px` wide (else the largest).
fn entry(ico: &[u8], px: u32) -> Option<&[u8]> {
    let count = u16::from_le_bytes([*ico.get(4)?, *ico.get(5)?]) as usize;
    let mut best: Option<(u32, &[u8])> = None;
    for i in 0..count {
        let e = ico.get(6 + 16 * i..6 + 16 * (i + 1))?;
        let size = if e[0] == 0 { 256 } else { e[0] as u32 };
        let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let off = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        let data = ico.get(off..off + len)?;
        let better = match best {
            None => true,
            Some((b, _)) => (size >= px && (b < px || size < b)) || (b < px && size > b),
        };
        if better {
            best = Some((size, data));
        }
    }
    best.map(|(_, d)| d)
}

/// Restores the window if it's minimized and brings it to the front. The restore goes the
/// way the title bar's and taskbar's do (WM_SYSCOMMAND), so winit's own minimised state
/// follows and Windows plays its restore animation where it plays one; a window minimised
/// from maximised comes back maximised.
pub fn bring_forward(h: isize) {
    // SAFETY: handle-only calls; a stale handle makes them fail, nothing is dereferenced.
    unsafe {
        if IsIconic(hwnd(h)).as_bool() {
            SendMessageW(
                hwnd(h),
                WM_SYSCOMMAND,
                Some(WPARAM(SC_RESTORE as usize)),
                None,
            );
        }
        let _ = SetForegroundWindow(hwnd(h));
    }
}

/// Minimises the window as its title bar button does (WM_SYSCOMMAND, so winit's state
/// follows), with Windows' minimise animation where it plays one.
pub fn minimise(h: isize) {
    // SAFETY: a handle-only message; a stale handle makes it fail.
    unsafe {
        SendMessageW(
            hwnd(h),
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
pub fn when_pressed(h: isize, pressed: impl Fn(bool) + 'static) -> Result<(), String> {
    let data = Box::into_raw(Box::new((
        Cell::new(false),
        Box::new(pressed) as Box<dyn Fn(bool)>,
    )));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok =
        unsafe { SetWindowSubclass(hwnd(h), Some(pressed_proc), PRESSED_SUBCLASS, data as usize) };
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

/// The window in front: what `foreground` reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Foreground {
    pub hwnd: isize,
    /// Its window class, as `GetClassNameW` gives it (`Shell_TrayWnd`).
    pub class: String,
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
        hwnd: h.0 as isize,
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
pub fn is_minimised(h: isize) -> bool {
    // SAFETY: a handle-only call; a stale handle reads as not minimised.
    unsafe { IsIconic(hwnd(h)) }.as_bool()
}

/// Where the window's client area starts on screen, in physical pixels.
pub fn client_origin(h: isize) -> Option<(i32, i32)> {
    let mut p = POINT::default();
    // SAFETY: a handle and an out-parameter; a stale handle fails.
    unsafe { ClientToScreen(hwnd(h), &mut p) }
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
pub fn when_taskbar_button(h: isize, created: impl Fn() + 'static) -> Result<(), String> {
    // SAFETY: a NUL-terminated static name.
    let msg = unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarButtonCreated")) };
    if msg == 0 {
        return Err("RegisterWindowMessage(TaskbarButtonCreated) failed".into());
    }
    let data = Box::into_raw(Box::new((msg, Box::new(created) as Box<dyn Fn()>)));
    // SAFETY: a live HWND on this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok =
        unsafe { SetWindowSubclass(hwnd(h), Some(button_proc), BUTTON_SUBCLASS, data as usize) };
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

/// A screen rectangle in physical pixels: left, top, right, bottom.
pub type ScreenRect = (i32, i32, i32, i32);

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
pub fn popup_border(h: isize, colour: Color) -> Result<(), String> {
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

// The clipboard calls, declared here rather than turning on another `windows` feature.
#[link(name = "user32")]
unsafe extern "system" {
    fn OpenClipboard(owner: isize) -> i32;
    fn CloseClipboard() -> i32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, mem: isize) -> isize;
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> isize;
    fn GlobalLock(mem: isize) -> *mut c_void;
    fn GlobalUnlock(mem: isize) -> i32;
    fn GlobalFree(mem: isize) -> isize;
}

/// `GMEM_MOVEABLE`: clipboard data must be movable memory.
const GMEM_MOVEABLE: u32 = 0x0002;

/// The formats that keep clipboard data out of Windows' clipboard history, the cloud
/// clipboard and clipboard managers, as password managers set them.
const PRIVATE_FORMATS: [&str; 3] = [
    "ExcludeClipboardContentFromMonitorProcessing",
    "CanIncludeInClipboardHistory",
    "CanUploadToCloudClipboard",
];

/// Puts a secret (the local API's token) on the clipboard as text, kept out of the
/// clipboard history and the cloud clipboard.
pub fn copy_secret(text: &str) -> Result<(), String> {
    let utf16: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let bytes: Vec<u8> = utf16.iter().flat_map(|u| u.to_ne_bytes()).collect();
    // SAFETY: the clipboard is opened, filled with memory this function allocates and
    // hands over (or frees when the hand-over fails), and closed again on every path.
    unsafe {
        if OpenClipboard(0) == 0 {
            return Err(format!(
                "the clipboard is busy (error {})",
                windows::Win32::Foundation::GetLastError().0
            ));
        }
        let result = if EmptyClipboard() == 0 {
            Err("the clipboard couldn't be emptied".to_string())
        } else {
            set_clipboard(CF_UNICODETEXT, &bytes).and_then(|()| {
                for name in PRIVATE_FORMATS {
                    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
                    let format = RegisterClipboardFormatW(wide.as_ptr());
                    // A DWORD of 0: "no" for the two Can… formats; any data for the first.
                    if format != 0 {
                        set_clipboard(format, &0u32.to_ne_bytes())?;
                    }
                }
                Ok(())
            })
        };
        CloseClipboard();
        result
    }
}

/// Hands `data` to the open clipboard as `format`.
///
/// # Safety
/// The clipboard must be open and emptied by this process.
unsafe fn set_clipboard(format: u32, data: &[u8]) -> Result<(), String> {
    // SAFETY: as the caller promises; the memory is ours until `SetClipboardData` takes it.
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, data.len());
        if mem == 0 {
            return Err("out of memory for the clipboard".into());
        }
        let p = GlobalLock(mem);
        if p.is_null() {
            GlobalFree(mem);
            return Err("the clipboard memory couldn't be locked".into());
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), p.cast::<u8>(), data.len());
        GlobalUnlock(mem);
        if SetClipboardData(format, mem) == 0 {
            GlobalFree(mem);
            return Err("the clipboard didn't take the text".into());
        }
        Ok(())
    }
}

/// `CF_UNICODETEXT`, the text a paste reads.
const CF_UNICODETEXT: u32 = 13;

/// What a paste would find on the system clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardState {
    /// It opens and holds text.
    Text,
    /// It opens but holds no text (an image, files, or nothing).
    NoText,
    /// It doesn't open: another program holds it, or Windows denies this process (the Win32
    /// error, 5 for access denied).
    Closed { error: u32 },
}

/// Opens the clipboard for a moment to see what a paste would get, never reading the text.
pub fn clipboard_state() -> ClipboardState {
    // SAFETY: plain Win32 calls without pointers; the clipboard is closed again at once.
    unsafe {
        if OpenClipboard(0) == 0 {
            return ClipboardState::Closed {
                error: windows::Win32::Foundation::GetLastError().0,
            };
        }
        let text = IsClipboardFormatAvailable(CF_UNICODETEXT) != 0;
        CloseClipboard();
        if text {
            ClipboardState::Text
        } else {
            ClipboardState::NoText
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorref_is_bgr() {
        assert_eq!(
            colorref(Color::from_rgb_u8(0x12, 0x34, 0x56)).0,
            0x0056_3412
        );
    }

    #[test]
    fn icon_has_every_size() {
        for ico in [ICON, ICON_LIGHT] {
            for px in [16, 20, 24, 32, 48, 256] {
                let png = entry(ico, px).expect("entry");
                assert_eq!(&png[1..4], b"PNG");
                // Width from the IHDR chunk.
                let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
                assert_eq!(w, px);
            }
            let w = |png: &[u8]| u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
            assert_eq!(w(entry(ico, 22).unwrap()), 24);
            assert_eq!(w(entry(ico, 512).unwrap()), 256);
        }
    }

    #[test]
    fn light_taskbar_gets_the_light_icon() {
        assert_ne!(ICON, ICON_LIGHT);
        assert_eq!(icon_for(true), ICON_LIGHT);
        assert_eq!(icon_for(false), ICON);
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
        when_pressed(h.0 as isize, move |from_behind| {
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
