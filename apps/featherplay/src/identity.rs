//! The app's identity for the shell, so the media card shows "featherplay" and the
//! Tri-vane icon. The work is in `featherplay_platform::identity`; this wires it to
//! startup and `--unregister`.

use std::process::ExitCode;

use featherplay_platform::identity::{self, Registration};
use featherplay_store::Paths;

/// Call first thing in a windowed run, before any window or media session: names the
/// process with the app's AUMID, then (only in a normal install, see
/// `identity::registration_allowed`) checks the Start menu shortcut, the registry entry
/// and the icon file on a short-lived thread, writing only what is missing or stale.
pub fn startup() {
    if let Err(e) = identity::set_process_app_id() {
        log!("app identity: {e}");
    }
    if !identity::registration_allowed_here() {
        log!("app identity: dev build or FEATHERPLAY_HOME set, not registering");
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("featherplay-identity".into())
        .spawn(register_now);
    if let Err(e) = spawned {
        log!("app identity: {e}");
    }
}

/// Registers, and on success lets toasts use the app's AUMID (`identity::mark_registered`).
/// A toast sent before this finishes, or after it failed, borrows PowerShell's instead.
fn register_now() {
    let result = registration().and_then(|r| identity::register(&r, crate::native::ICON));
    if result.is_ok() {
        identity::mark_registered();
    }
    match result {
        Ok(c) if c == identity::Changes::default() => log!("app identity: up to date"),
        Ok(c) => log!(
            "app identity: registered (icon {}, registry {}, shortcut {})",
            c.icon,
            c.registry,
            c.shortcut
        ),
        Err(e) => log!("app identity: {e}"),
    }
}

/// `featherplay --unregister`: removes the Start menu shortcut, the registry entry and the
/// icon file. Refuses in a dev build or with `FEATHERPLAY_HOME` set, like registering.
/// A later normal start registers again; to uninstall, run this and then delete the exe.
pub fn unregister() -> ExitCode {
    if !identity::registration_allowed_here() {
        log!("featherplay: --unregister runs only in a release build without FEATHERPLAY_HOME");
        return ExitCode::from(2);
    }
    match registration().and_then(|r| identity::unregister(&r)) {
        Ok(()) => {
            log!("featherplay: removed the Start menu shortcut and the app registration");
            ExitCode::SUCCESS
        }
        Err(e) => {
            log!("featherplay: --unregister: {e}");
            ExitCode::FAILURE
        }
    }
}

/// This exe, the user's Start menu, and the cache dir (`%LOCALAPPDATA%\featherplay`) for
/// the icon file.
fn registration() -> Result<Registration, String> {
    let exe = std::env::current_exe().map_err(|e| format!("the exe's path: {e}"))?;
    Registration::for_current_user(exe, &Paths::resolve().cache)
}
