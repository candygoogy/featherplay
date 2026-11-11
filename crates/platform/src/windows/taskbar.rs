//! Previous / play-pause / next buttons under the taskbar thumbnail (`ITaskbarList3`).
//!
//! The buttons can only be added once the taskbar button exists. Explorer says so with
//! the registered "TaskbarButtonCreated" message, and says it again after it restarts,
//! so the window is subclassed to catch that message and the `WM_COMMAND` clicks. A click
//! is reported to the caller as a `ThumbButton`.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use featherplay_core::tr;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, TRUE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, RemoveWindowSubclass, SetWindowSubclass, THB_FLAGS, THB_ICON,
    THB_TOOLTIP, THBF_ENABLED, THBN_CLICKED, THUMBBUTTON, TaskbarList,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON,
    ICONINFO, MSGFLT_ALLOW, RegisterWindowMessageW, SM_CXSMICON, WM_COMMAND, WM_NCDESTROY,
};
use windows::core::w;

pub use crate::common::taskbar::*;

use super::icons::{self, Glyph};

const ID_PREVIOUS: u32 = 1001;
const ID_PLAY_PAUSE: u32 = 1002;
const ID_NEXT: u32 = 1003;
/// Our subclass id on the window (any value unique among this window's subclasses).
const SUBCLASS_ID: usize = 0xfea7_0001;

pub struct TaskbarButtons {
    hwnd: HWND,
    /// Owned; the subclass procedure borrows it through `dwRefData` until `drop`.
    state: *mut State,
}

/// The app's handler for a click (called from the window procedure).
type OnClick = Box<dyn Fn(ThumbButton)>;

struct State {
    on_click: OnClick,
    created_msg: u32,
    list: RefCell<Option<ITaskbarList3>>,
    added: Cell<bool>,
    playing: Cell<bool>,
    icons: [HICON; 4],
}

impl TaskbarButtons {
    /// Subclasses `hwnd`; the buttons are added when "TaskbarButtonCreated" arrives (or
    /// on the first `set_playing` if it came too early). Call on the window's thread;
    /// `on_click` is called from the window's procedure.
    pub fn attach(hwnd: isize, on_click: impl Fn(ThumbButton) + 'static) -> Result<Self, String> {
        let hwnd = HWND(hwnd as *mut c_void);
        // S_FALSE or RPC_E_CHANGED_MODE when winit already set up COM on this thread;
        // either way the thread has an apartment, which is all ITaskbarList3 needs.
        // SAFETY: the reserved argument is None, as the API requires. The apartment is never
        // uninitialised, so COM objects made on this thread stay usable for its whole life.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        // SAFETY: `w!` gives a static, NUL-terminated UTF-16 string.
        let created_msg = unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) };
        if created_msg == 0 {
            return Err("RegisterWindowMessage(TaskbarButtonCreated) failed".into());
        }
        // Lets the message through when the app runs elevated (Explorer doesn't).
        // SAFETY: only integers and a handle go in, and the optional CHANGEFILTERSTRUCT out
        // pointer is None. A stale `hwnd` makes the call fail; it is never dereferenced here.
        unsafe {
            let _ = ChangeWindowMessageFilterEx(hwnd, created_msg, MSGFLT_ALLOW, None);
            let _ = ChangeWindowMessageFilterEx(hwnd, WM_COMMAND, MSGFLT_ALLOW, None);
        }
        let icons = make_icons()?;
        let state = Box::into_raw(Box::new(State {
            on_click: Box::new(on_click),
            created_msg,
            list: RefCell::new(None),
            added: Cell::new(false),
            playing: Cell::new(false),
            icons,
        }));
        // SAFETY: `subclass_proc` has the SUBCLASSPROC signature. Its `dwRefData` is `state`,
        // a leaked Box that stays valid until `drop`, which removes the subclass before it
        // frees the Box. SetWindowSubclass refuses windows of other threads, so the
        // procedure only ever runs on this thread, the one that owns `state`.
        let ok =
            unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, state as usize) };
        if !ok.as_bool() {
            // SAFETY: not shared yet, the subclass was never installed.
            drop(unsafe { Box::from_raw(state) });
            return Err("SetWindowSubclass failed".into());
        }
        // No add here: before Explorer has made the taskbar button, ThumbBarAddButtons can
        // report success and still be lost (seen on Windows 11).
        Ok(TaskbarButtons { hwnd, state })
    }

    /// Shows pause while playing, play otherwise.
    pub fn set_playing(&self, playing: bool) {
        // SAFETY: `state` lives until drop; only this thread touches it.
        let st = unsafe { &*self.state };
        let changed = st.playing.replace(playing) != playing;
        if !st.added.get() {
            // TaskbarButtonCreated came before the subclass was in place. By the first
            // state update (seconds after the window appeared) the taskbar button exists.
            match st.add(self.hwnd) {
                Ok(()) => log::info!("[taskbar] thumb buttons added (TaskbarButtonCreated missed)"),
                Err(e) => log::warn!("[taskbar] {e}"),
            }
        } else if changed && let Err(e) = st.update(self.hwnd) {
            log::warn!("[taskbar] update: {e}");
        }
    }
}

impl Drop for TaskbarButtons {
    fn drop(&mut self) {
        // SAFETY: `TaskbarButtons` holds a raw pointer, so it is neither Send nor Sync and
        // drops on the window's thread, the only thread `subclass_proc` runs on. Once
        // RemoveWindowSubclass returns, Windows no longer passes `state` to it, so the Box
        // made in `attach` (freed nowhere else, and drop runs once) is ours alone. The icons
        // belong to `state` and nothing else destroys them.
        unsafe {
            // Fails harmlessly if WM_NCDESTROY already removed it.
            let _ = RemoveWindowSubclass(self.hwnd, Some(subclass_proc), SUBCLASS_ID);
            let state = Box::from_raw(self.state);
            for icon in state.icons {
                let _ = DestroyIcon(icon);
            }
        }
    }
}

impl State {
    /// The COM pointer, cloned out so no borrow is held across a call that may pump
    /// messages back into `subclass_proc`.
    fn taskbar(&self) -> Result<ITaskbarList3, String> {
        if let Some(list) = self.list.borrow().clone() {
            return Ok(list);
        }
        // SAFETY: `attach` initialised COM on this thread, the only one `State` is used on;
        // the CLSID is a static GUID, and the returned interface is reference-counted by
        // its windows-rs wrapper.
        let list: ITaskbarList3 =
            unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| format!("ITaskbarList3: {e}"))?;
        // SAFETY: a call without arguments on the interface just created on this thread.
        unsafe { list.HrInit() }.map_err(|e| format!("ITaskbarList3::HrInit: {e}"))?;
        *self.list.borrow_mut() = Some(list.clone());
        Ok(list)
    }

    fn add(&self, hwnd: HWND) -> Result<(), String> {
        let list = self.taskbar()?;
        // SAFETY: the buttons are a temporary that lives across the call, which copies
        // them; their icons belong to `self` and are destroyed only when it is dropped.
        unsafe { list.ThumbBarAddButtons(hwnd, &self.buttons()) }
            .map_err(|e| format!("ThumbBarAddButtons: {e}"))?;
        self.added.set(true);
        Ok(())
    }

    fn update(&self, hwnd: HWND) -> Result<(), String> {
        let list = self.taskbar()?;
        // SAFETY: as in `add`: the buttons outlive the call, and their icons outlive `self`.
        unsafe { list.ThumbBarUpdateButtons(hwnd, &self.buttons()) }
            .map_err(|e| format!("ThumbBarUpdateButtons: {e}"))
    }

    fn buttons(&self) -> [THUMBBUTTON; 3] {
        let [prev, play, pause, next] = self.icons;
        let (pp_icon, pp_tip) = if self.playing.get() {
            (pause, tr!("Pause"))
        } else {
            (play, tr!("Play"))
        };
        [
            button(ID_PREVIOUS, prev, tr!("Previous")),
            button(ID_PLAY_PAUSE, pp_icon, pp_tip),
            button(ID_NEXT, next, tr!("Next")),
        ]
    }

    /// "TaskbarButtonCreated": the first time, or again after Explorer restarted (which
    /// drops the buttons, so a fresh ITaskbarList3 adds them again).
    fn button_created(&self, hwnd: HWND) {
        let again = self.added.get();
        *self.list.borrow_mut() = None;
        match self.add(hwnd) {
            Ok(()) => log::info!("[taskbar] TaskbarButtonCreated: thumb buttons added"),
            // Already added by the `set_playing` fallback: a late echo, not a restart.
            Err(_) if again => match self.update(hwnd) {
                Ok(()) => {
                    log::info!("[taskbar] TaskbarButtonCreated: thumb buttons already present")
                }
                Err(e) => log::warn!("[taskbar] TaskbarButtonCreated: {e}"),
            },
            Err(e) => log::warn!("[taskbar] TaskbarButtonCreated: {e}"),
        }
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: `data` is the `State` from `attach`; it outlives the subclass.
    let st = unsafe { &*(data as *const State) };
    if msg == st.created_msg {
        st.button_created(hwnd);
    } else if msg == WM_COMMAND && (wparam.0 >> 16) as u32 & 0xffff == THBN_CLICKED {
        let button = match (wparam.0 & 0xffff) as u32 {
            ID_PREVIOUS => Some(ThumbButton::Previous),
            ID_PLAY_PAUSE => Some(ThumbButton::PlayPause),
            ID_NEXT => Some(ThumbButton::Next),
            _ => None,
        };
        if let Some(button) = button {
            (st.on_click)(button);
            return LRESULT(0);
        }
    } else if msg == WM_NCDESTROY {
        // SAFETY: the documented place to remove a subclass, with the procedure and id it
        // was installed with. `data` is not freed here; `TaskbarButtons::drop` frees it.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
        }
    }
    // SAFETY: called from inside the subclass procedure, with the message it was given.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn button(id: u32, icon: HICON, tip: &str) -> THUMBBUTTON {
    let mut sz_tip = [0u16; 260];
    for (dst, src) in sz_tip.iter_mut().zip(tip.encode_utf16().take(259)) {
        *dst = src;
    }
    THUMBBUTTON {
        dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
        iId: id,
        hIcon: icon,
        szTip: sz_tip,
        dwFlags: THBF_ENABLED,
        ..Default::default()
    }
}

/// Previous, play, pause, next, in the small-icon size, dark glyphs on a light taskbar.
fn make_icons() -> Result<[HICON; 4], String> {
    // SAFETY: an integer index in, an integer out; no memory is involved.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64) as u32;
    let color = if light_taskbar() {
        [0x1c, 0x1c, 0x1c]
    } else {
        [0xff, 0xff, 0xff]
    };
    let mut out = [HICON::default(); 4];
    let glyphs = [Glyph::Previous, Glyph::Play, Glyph::Pause, Glyph::Next];
    for (i, g) in glyphs.into_iter().enumerate() {
        match hicon(&icons::glyph(g, size, color), size) {
            Ok(icon) => out[i] = icon,
            Err(e) => {
                for icon in &out[..i] {
                    // SAFETY: made by `hicon` just above and not handed out yet, so
                    // nothing else holds or destroys it.
                    let _ = unsafe { DestroyIcon(*icon) };
                }
                return Err(e);
            }
        }
    }
    Ok(out)
}

/// `SystemUsesLightTheme` (the taskbar's theme); dark when it can't be read. The app's
/// window icon follows it too (`featherplay::native::set_icon`).
pub fn light_taskbar() -> bool {
    let mut value = 0u32;
    let mut len = size_of::<u32>() as u32;
    // SAFETY: the data pointer is `value`, the 4 bytes `len` says it has, and
    // RRF_RT_REG_DWORD makes the call fail rather than write anything but a DWORD. Both live
    // across the call; the key and value names are static NUL-terminated strings.
    let res = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut c_void),
            Some(&mut len),
        )
    };
    res.is_ok() && value != 0
}

/// A 32-bit icon from straight-alpha RGBA.
fn hicon(rgba: &[u8], size: u32) -> Result<HICON, String> {
    // The copy into the DIB section below relies on this length.
    if rgba.len() != size as usize * size as usize * 4 {
        return Err(format!(
            "icon: {} bytes for {size}x{size} pixels",
            rgba.len()
        ));
    }
    let s = size as i32;
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: s,
            biHeight: -s, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    // SAFETY: `bmi` is a complete header for a top-down 32-bit size x size bitmap, and
    // `bits` is a live out pointer for the pixel address. No file mapping (None, 0).
    let color = unsafe { CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) }
        .map_err(|e| format!("CreateDIBSection: {e}"))?;
    // SAFETY: CreateDIBSection succeeded, so `bits` points to the section's size*size 32-bit
    // pixels, which is `rgba.len()` bytes (checked at the top). Nothing else uses that
    // memory until the bitmap is handed to CreateIconIndirect, after `dst`'s last use.
    let dst = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, rgba.len()) };
    for (d, s) in dst
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(rgba.as_chunks::<4>().0)
    {
        *d = [s[2], s[1], s[0], s[3]]; // BGRA
    }
    // All-zero AND mask: the alpha channel decides.
    let stride = size.div_ceil(16) as usize * 2;
    let mask_bits = vec![0u8; stride * size as usize];
    // SAFETY: a 1-bit bitmap reads rows padded to 2 bytes, so `stride * size` bytes, which
    // is exactly `mask_bits`; it lives across the call, which copies it.
    let mask = unsafe { CreateBitmap(s, s, 1, 1, Some(mask_bits.as_ptr() as *const c_void)) };
    let info = ICONINFO {
        fIcon: TRUE,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: color,
    };
    // SAFETY: `info` names two live bitmaps; the call copies them into the icon.
    let icon = unsafe { CreateIconIndirect(&info) };
    // SAFETY: the icon has its own copies, so both bitmaps are unused now. They were made
    // here, are selected into no DC, and each is deleted once. A failed (null) handle
    // makes DeleteObject fail without effect.
    unsafe {
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
    }
    icon.map_err(|e| format!("CreateIconIndirect: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hicon_rejects_pixels_that_dont_fill_the_icon() {
        let too_short = vec![0u8; 16 * 16 * 4 - 4];

        assert!(hicon(&too_short, 16).is_err());
    }
}
