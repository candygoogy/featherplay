//! Which Windows this is, from the registry: `Windows 11 25H2, build 26200.7171`.

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, REG_VALUE_TYPE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::core::HSTRING;

const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// Windows 11 kept Windows 10's major version, and its `ProductName` still says "Windows 10":
/// the build number tells them apart.
const FIRST_WINDOWS_11_BUILD: u32 = 22000;

pub fn windows() -> String {
    let build = string("CurrentBuild");
    let number: Option<u32> = build.as_deref().and_then(|b| b.parse().ok());
    let name = match number {
        Some(n) if n >= FIRST_WINDOWS_11_BUILD => "Windows 11",
        Some(_) => "Windows 10",
        None => "Windows",
    };
    let release = string("DisplayVersion").map_or_else(String::new, |v| format!(" {v}"));
    let build = match (build, dword("UBR")) {
        (Some(b), Some(ubr)) => format!(", build {b}.{ubr}"),
        (Some(b), None) => format!(", build {b}"),
        (None, _) => String::new(),
    };
    format!("{name}{release}{build}")
}

fn string(name: &str) -> Option<String> {
    let (key, name) = (HSTRING::from(KEY), HSTRING::from(name));
    let mut buf = [0u16; 256];
    let mut len = size_of_val(&buf) as u32;
    // SAFETY: the buffer and its size in bytes are passed together; RRF_RT_REG_SZ makes the
    // result NUL-terminated.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &key,
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
    let s = String::from_utf16_lossy(&buf[..end]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn dword(name: &str) -> Option<u32> {
    let (key, name) = (HSTRING::from(KEY), HSTRING::from(name));
    let mut value = 0u32;
    let mut len = size_of::<u32>() as u32;
    let mut kind = REG_VALUE_TYPE::default();
    // SAFETY: a u32 out value with its size in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &key,
            &name,
            RRF_RT_REG_DWORD,
            Some(&mut kind),
            Some((&mut value as *mut u32).cast()),
            Some(&mut len),
        )
    };
    (status == ERROR_SUCCESS).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_windows_and_its_build() {
        let w = windows();
        assert!(w.starts_with("Windows 1"), "{w}");
        assert!(w.contains(", build "), "{w}");
    }
}
