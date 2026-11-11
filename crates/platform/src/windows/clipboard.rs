//! The clipboard: copying text, a secret kept out of the clipboard history, and what a
//! paste would find.

use std::ffi::c_void;

pub use crate::common::clipboard::*;

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
    copy(text, true)
}

/// Puts `text` on the clipboard, as Ctrl+C in a text field does.
pub fn copy_text(text: &str) -> Result<(), String> {
    copy(text, false)
}

/// `copy_secret` when `private`, else `copy_text`.
fn copy(text: &str, private: bool) -> Result<(), String> {
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
                let private_formats = if private { &PRIVATE_FORMATS[..] } else { &[] };
                for name in private_formats {
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

/// Opens the clipboard for a moment to see what a paste would get, never reading the text.
pub fn state() -> ClipboardState {
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
