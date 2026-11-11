//! The install location from the uninstall key, and starting the setup outside the job.

use std::io;
use std::path::PathBuf;
use std::process::Command;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::HSTRING;

/// The installer's uninstall key under HKCU: Inno Setup names it after the AppId in
/// `featherplay.iss`, plus `_is1`.
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{F2D85B2D-D1BB-4404-AA87-B6D35D0F5283}_is1";
/// `CREATE_BREAKAWAY_FROM_JOB`: the setup must outlive featherplay even if a job object
/// (a terminal, a launcher) would end featherplay's children with it.
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// The installer's `InstallLocation`, if featherplay is installed for this user. Inno
/// writes it with a trailing backslash.
pub fn install_location() -> Option<PathBuf> {
    let (key, name) = (
        HSTRING::from(UNINSTALL_KEY),
        HSTRING::from("InstallLocation"),
    );
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its size in bytes are passed together; RRF_RT_REG_SZ makes the
    // result NUL-terminated.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
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
    (!s.is_empty()).then(|| PathBuf::from(s))
}

/// Starts the setup `command()` makes so that it outlives this process, out of any job
/// this process is in. A job that doesn't allow breaking away refuses that: then it's
/// started plainly.
pub fn start_setup(command: impl Fn() -> Command) -> io::Result<()> {
    use std::os::windows::process::CommandExt;

    command()
        .creation_flags(CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
        .or_else(|_| command().spawn())
        .map(drop)
}
