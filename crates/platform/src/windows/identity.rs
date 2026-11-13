//! The app's identity for the shell, so the media card (Win+A, the volume flyout, the
//! lock screen) shows "featherplay" and its icon instead of "Unknown app".
//!
//! The card names a media session after its AppUserModelID (AUMID). An unpackaged exe has
//! none the shell can resolve, so it is named in two places:
//! - **The process:** `set_process_app_id`, before any window exists. The window, and the
//!   SMTC session bound to it, take the process's AUMID.
//! - **Where the shell looks the AUMID up** (`register`):
//!   - a Start menu shortcut whose `System.AppUserModel.ID` is the AUMID. The shell resolves
//!     an unpackaged app's name and icon through it, as it does for the taskbar. This is
//!     the part the media card needs.
//!   - `HKCU\Software\Classes\AppUserModelId\<AUMID>` with `DisplayName` and `IconUri`, the
//!     per-user registration Windows' notification platform reads for unpackaged apps.
//!     Toasts go out under the AUMID once it is in place (`registered`), and take their
//!     name and icon from it; it costs two values.
//!
//! Registration is per user and writes nothing outside the user's own Start menu, HKCU
//! and the app's data dir. It only runs in a normal install (`registration_allowed`):
//! never in a debug build and never with `FEATHERPLAY_HOME` set, so dev runs and tests
//! leave the user's shell alone. `unregister` undoes it.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, RPC_E_CHANGED_MODE};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PropVariantClear, PropVariantToStringAlloc,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize, IPersistFile, STGM_READ,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
};
use windows::Win32::System::Variant::{VT_EMPTY, VT_LPWSTR};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    FOLDERID_Programs, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath, SHStrDupW, SLGP_RAWPATH,
    SetCurrentProcessExplicitAppUserModelID, ShellLink,
};
use windows::core::{HSTRING, Interface, PCWSTR};

pub use crate::common::identity::*;

/// The shortcut's tooltip in Start.
const DESCRIPTION: &str = "featherplay music player";

/// Names this process `AUMID`. Call at startup, before any window: windows created later
/// take it, and so does the media session bound to them. Writes nothing to disk.
pub fn set_process_app_id() -> Result<(), String> {
    // SAFETY: a valid, NUL-terminated wide string that outlives the call.
    unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID)) }
        .map_err(|e| format!("set the AppUserModelID: {}", e.message()))
}

/// The registry key under HKCU.
pub fn registry_key() -> String {
    format!(r"Software\Classes\AppUserModelId\{AUMID}")
}

/// The values under `registry_key`, both `REG_SZ`.
pub fn registry_values(icon_file: &Path) -> [(&'static str, String); 2] {
    [
        ("DisplayName", DISPLAY_NAME.to_string()),
        ("IconUri", icon_file.display().to_string()),
    ]
}

/// What a shortcut holds, as far as the identity goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub target: PathBuf,
    pub working_dir: PathBuf,
    /// The icon is the exe's own (resource index 0).
    pub icon: PathBuf,
    pub description: String,
    /// `System.AppUserModel.ID`.
    pub app_id: String,
}

impl Shortcut {
    /// The Start menu shortcut for `exe`: starts it in its own folder, with its icon and
    /// the app's AUMID.
    pub fn for_exe(exe: &Path) -> Shortcut {
        Shortcut {
            target: exe.to_path_buf(),
            working_dir: exe.parent().map(Path::to_path_buf).unwrap_or_default(),
            icon: exe.to_path_buf(),
            description: DESCRIPTION.to_string(),
            app_id: AUMID.to_string(),
        }
    }

    /// Whether the shortcut read back from disk already does the job: the same target and
    /// AUMID. Paths compare without regard to ASCII case, as Windows does.
    fn same_identity(&self, on_disk: &Shortcut) -> bool {
        let path_eq = |a: &Path, b: &Path| {
            a.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
        };
        path_eq(&self.target, &on_disk.target) && self.app_id == on_disk.app_id
    }
}

impl Registration {
    /// The current user's Start menu Programs folder and `data_dir`.
    pub fn for_current_user(exe: PathBuf, data_dir: &Path) -> Result<Registration, String> {
        Ok(Registration::new(exe, &programs_folder()?, data_dir))
    }
}

/// Writes the icon file, the registry entry and the Start menu shortcut, each only if it
/// differs from what is there (so it can run at every start, and a moved exe repoints the
/// shortcut). The caller checks `registration_allowed` first. Blocks on file and COM
/// calls for a few milliseconds; call it off the UI thread.
pub fn register(reg: &Registration, icon: &[u8]) -> Result<Changes, String> {
    let mut changes = Changes::default();
    if std::fs::read(&reg.icon_file).ok().as_deref() != Some(icon) {
        if let Some(dir) = reg.icon_file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&reg.icon_file, icon)
            .map_err(|e| format!("{}: {e}", reg.icon_file.display()))?;
        changes.icon = true;
    }

    let key = registry_key();
    let values = registry_values(&reg.icon_file);
    if values
        .iter()
        .any(|(name, value)| read_hkcu_string(&key, name).as_deref() != Some(value.as_str()))
    {
        write_hkcu_strings(&key, &values)?;
        changes.registry = true;
    }

    let _com = Com::init()?;
    let wanted = Shortcut::for_exe(&reg.exe);
    let current = read_shortcut(&reg.shortcut).ok();
    if !current.is_some_and(|c| wanted.same_identity(&c)) {
        write_shortcut(&reg.shortcut, &wanted)?;
        changes.shortcut = true;
    }
    Ok(changes)
}

/// Removes what `register` wrote: the shortcut (only if it carries our AUMID), the
/// registry key and the icon file. Anything already gone is fine.
pub fn unregister(reg: &Registration) -> Result<(), String> {
    let mut errors = Vec::new();
    {
        let _com = Com::init()?;
        match read_shortcut(&reg.shortcut) {
            Ok(s) if s.app_id == AUMID => {
                if let Err(e) = std::fs::remove_file(&reg.shortcut) {
                    errors.push(format!("{}: {e}", reg.shortcut.display()));
                }
            }
            Ok(_) => errors.push(format!(
                "{} isn't featherplay's shortcut; left in place",
                reg.shortcut.display()
            )),
            Err(_) if !reg.shortcut.exists() => {}
            Err(e) => errors.push(e),
        }
    }
    let key = HSTRING::from(registry_key());
    // SAFETY: HKCU is a predefined key; the subkey is a NUL-terminated wide string.
    let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &key) };
    if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
        errors.push(format!(
            "delete HKCU\\{key}: {}",
            status.to_hresult().message()
        ));
    }
    match std::fs::remove_file(&reg.icon_file) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => errors.push(format!("{}: {e}", reg.icon_file.display())),
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// COM on this thread for the shell link calls, released on drop. A thread already in
/// the other apartment keeps it (ShellLink works in either).
struct Com(bool);

impl Com {
    fn init() -> Result<Com, String> {
        // SAFETY: paired with CoUninitialize in Drop when it succeeded.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_ok() {
            Ok(Com(true))
        } else if hr == RPC_E_CHANGED_MODE {
            Ok(Com(false))
        } else {
            Err(format!("COM: {}", hr.message()))
        }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// The current user's `Start Menu\Programs` folder.
fn programs_folder() -> Result<PathBuf, String> {
    // SAFETY: a known folder ID; the returned string is freed with CoTaskMemFree.
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None)
            .map_err(|e| format!("the Start menu folder: {}", e.message()))?;
        let path = p.to_string().map(PathBuf::from);
        CoTaskMemFree(Some(p.0 as *const _));
        path.map_err(|e| format!("the Start menu folder: {e}"))
    }
}

/// Saves `s` as a `.lnk` at `path`. Needs COM on this thread.
fn write_shortcut(path: &Path, s: &Shortcut) -> Result<(), String> {
    let err = |what: &str, e: windows::core::Error| format!("shortcut {what}: {}", e.message());
    // SAFETY: COM calls on interfaces created here, with NUL-terminated wide strings
    // that outlive each call.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| err("create", e))?;
        link.SetPath(&HSTRING::from(s.target.as_os_str()))
            .map_err(|e| err("target", e))?;
        link.SetWorkingDirectory(&HSTRING::from(s.working_dir.as_os_str()))
            .map_err(|e| err("working dir", e))?;
        link.SetIconLocation(&HSTRING::from(s.icon.as_os_str()), 0)
            .map_err(|e| err("icon", e))?;
        link.SetDescription(&HSTRING::from(s.description.as_str()))
            .map_err(|e| err("description", e))?;

        let store: IPropertyStore = link.cast().map_err(|e| err("properties", e))?;
        // A VT_LPWSTR in COM memory, as InitPropVariantFromString makes it (that helper
        // is inline in the SDK headers, so not exported). SetValue copies it; it's cleared
        // after. A string borrowed from Rust's heap corrupted the heap here.
        let mut pv = PROPVARIANT::default();
        {
            let inner = &mut *pv.Anonymous.Anonymous;
            inner.vt = VT_LPWSTR;
            inner.Anonymous.pwszVal = SHStrDupW(&HSTRING::from(s.app_id.as_str()))
                .map_err(|e| err("AppUserModelID", e))?;
        }
        let set = store.SetValue(&PKEY_AppUserModel_ID, &pv);
        let _ = PropVariantClear(&mut pv);
        set.map_err(|e| err("AppUserModelID", e))?;
        store.Commit().map_err(|e| err("commit", e))?;

        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let file: IPersistFile = link.cast().map_err(|e| err("file", e))?;
        file.Save(&HSTRING::from(path.as_os_str()), true)
            .map_err(|e| err("save", e))
    }
}

/// Reads back what `write_shortcut` sets. Needs COM on this thread.
fn read_shortcut(path: &Path) -> Result<Shortcut, String> {
    let err = |what: &str, e: windows::core::Error| format!("shortcut {what}: {}", e.message());
    // SAFETY: COM calls on interfaces created here; buffers are sized as passed; the
    // PROPVARIANT and the string allocated from it are freed before returning.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| err("create", e))?;
        let file: IPersistFile = link.cast().map_err(|e| err("file", e))?;
        file.Load(&HSTRING::from(path.as_os_str()), STGM_READ)
            .map_err(|e| err("load", e))?;
        let mut buf = [0u16; 1024];
        let text = |buf: &[u16]| {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..end])
        };
        link.GetPath(&mut buf, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
            .map_err(|e| err("target", e))?;
        let target = PathBuf::from(text(&buf));
        buf.fill(0);
        link.GetWorkingDirectory(&mut buf)
            .map_err(|e| err("working dir", e))?;
        let working_dir = PathBuf::from(text(&buf));
        buf.fill(0);
        let mut index = 0;
        link.GetIconLocation(&mut buf, &mut index)
            .map_err(|e| err("icon", e))?;
        let icon = PathBuf::from(text(&buf));
        buf.fill(0);
        link.GetDescription(&mut buf)
            .map_err(|e| err("description", e))?;
        let description = text(&buf);

        let store: IPropertyStore = link.cast().map_err(|e| err("properties", e))?;
        let mut pv = store
            .GetValue(&PKEY_AppUserModel_ID)
            .map_err(|e| err("AppUserModelID", e))?;
        let app_id = if pv.Anonymous.Anonymous.vt == VT_EMPTY {
            String::new()
        } else {
            match PropVariantToStringAlloc(&pv) {
                Ok(p) => {
                    let s = p.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(p.0 as *const _));
                    s
                }
                Err(_) => String::new(),
            }
        };
        let _ = PropVariantClear(&mut pv);
        Ok(Shortcut {
            target,
            working_dir,
            icon,
            description,
            app_id,
        })
    }
}

/// A `REG_SZ` value under HKCU, if it is there.
pub(crate) fn read_hkcu_string(subkey: &str, name: &str) -> Option<String> {
    let subkey = HSTRING::from(subkey);
    let name = HSTRING::from(name);
    let mut buf = [0u16; 1024];
    let mut len = size_of_val(&buf) as u32;
    // SAFETY: the buffer and its size in bytes are passed together; RRF_RT_REG_SZ makes
    // the result NUL-terminated.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &subkey,
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let chars = (len as usize / 2).min(buf.len());
    let end = buf[..chars].iter().position(|&c| c == 0).unwrap_or(chars);
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// Creates `HKCU\{subkey}` if needed and sets each value as `REG_SZ`.
pub(crate) fn write_hkcu_strings(subkey: &str, values: &[(&str, String)]) -> Result<(), String> {
    let path = HSTRING::from(subkey);
    let mut key = HKEY::default();
    // SAFETY: HKCU is a predefined key; `key` receives the opened handle, closed below.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &path,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "create HKCU\\{subkey}: {}",
            status.to_hresult().message()
        ));
    }
    let mut result = Ok(());
    for (name, value) in values {
        let data: Vec<u8> = value
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        // SAFETY: an open key; REG_SZ data is a NUL-terminated UTF-16 string.
        let status =
            unsafe { RegSetValueExW(key, &HSTRING::from(*name), None, REG_SZ, Some(&data)) };
        if status != ERROR_SUCCESS {
            result = Err(format!("set {name}: {}", status.to_hresult().message()));
            break;
        }
    }
    // SAFETY: the handle opened above.
    let _ = unsafe { RegCloseKey(key) };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_key_and_values() {
        assert_eq!(
            registry_key(),
            r"Software\Classes\AppUserModelId\featherplay.featherplay"
        );
        let icon = Path::new(r"C:\Users\u\AppData\Local\featherplay\featherplay.ico");
        assert_eq!(
            registry_values(icon),
            [
                ("DisplayName", "featherplay".to_string()),
                (
                    "IconUri",
                    r"C:\Users\u\AppData\Local\featherplay\featherplay.ico".to_string()
                ),
            ]
        );
    }

    #[test]
    fn registration_layout() {
        let reg = Registration::new(
            PathBuf::from(r"E:\Apps\featherplay\featherplay.exe"),
            Path::new(r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu\Programs"),
            Path::new(r"C:\Users\u\AppData\Local\featherplay"),
        );
        assert_eq!(
            reg.shortcut,
            PathBuf::from(
                r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\featherplay.lnk"
            )
        );
        assert_eq!(
            reg.icon_file,
            PathBuf::from(r"C:\Users\u\AppData\Local\featherplay\featherplay.ico")
        );
    }

    #[test]
    fn shortcut_for_exe() {
        let exe = Path::new(r"E:\Apps\featherplay\featherplay.exe");
        let s = Shortcut::for_exe(exe);
        assert_eq!(s.target, exe);
        assert_eq!(s.working_dir, Path::new(r"E:\Apps\featherplay"));
        assert_eq!(s.icon, exe);
        assert_eq!(s.app_id, AUMID);
        assert!(!s.description.to_lowercase().contains("youtube"));
    }

    #[test]
    fn shortcut_identity_ignores_case_but_not_moves() {
        let s = Shortcut::for_exe(Path::new(r"E:\Apps\featherplay\featherplay.exe"));
        let mut disk = Shortcut::for_exe(Path::new(r"e:\apps\Featherplay\featherplay.EXE"));
        disk.description = "edited by hand".into();
        assert!(s.same_identity(&disk));
        let moved = Shortcut::for_exe(Path::new(r"F:\featherplay.exe"));
        assert!(!s.same_identity(&moved));
        let other_id = Shortcut {
            app_id: "Someone.Else".into(),
            ..s.clone()
        };
        assert!(!s.same_identity(&other_id));
    }

    /// Writes a shortcut into `$FEATHERPLAY_HOME/tmp` (never the Start menu) and reads it
    /// back, under a test-only AUMID.
    #[test]
    fn shortcut_round_trip_outside_start_menu() {
        let Some(home) = std::env::var_os("FEATHERPLAY_HOME").filter(|h| !h.is_empty()) else {
            eprintln!("FEATHERPLAY_HOME unset; skipping the shortcut round trip");
            return;
        };
        let dir = PathBuf::from(home).join("tmp").join("identity-test");
        let path = dir.join(SHORTCUT_FILE);
        let start_menu = programs_folder().unwrap();
        assert!(!path.starts_with(&start_menu));

        let exe = dir.join("featherplay.exe");
        let s = Shortcut {
            app_id: format!("{AUMID}.Test"),
            ..Shortcut::for_exe(&exe)
        };
        let _com = Com::init().unwrap();
        write_shortcut(&path, &s).unwrap();
        let back = read_shortcut(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(back, s);
    }
}
