//! About page (ui/pages/about.slint): the version, Open log folder and Copy diagnostics, the
//! licences in full and a short list of third-party components, and a banner when a newer
//! release is out. The tagline, the disclaimer and the Made with Slint badge are in the
//! Slint file.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use featherplay_core::tr;
use featherplay_store::{Paths, Settings};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app::Ctx;
use crate::controller::PlayerState;
use crate::diagnostics;
use crate::language;
use crate::ui::{AboutData, Credit};
use crate::updates::{self, Release};

const MIT: &str = include_str!("../../../../LICENSE");
const OFL: &str = include_str!("../../ui/fonts/manrope/OFL.txt");

/// The main components and their licences; `THIRD_PARTY.md` has every crate.
/// Licence names stay as their authors wrote them; the words around them are translated.
fn credits() -> [(&'static str, &'static str); 11] {
    [
        (
            "Slint",
            "Royalty-free Desktop, Mobile, and Web Applications License 2.0",
        ),
        (tr!("Skia, Slint's renderer"), "BSD 3-Clause License"),
        ("ytmapi-rs", "MIT License"),
        (tr!("rodio and cpal"), "MIT or Apache 2.0; Apache 2.0"),
        ("Symphonia", "Mozilla Public License 2.0"),
        (tr!("tokio and reqwest"), "MIT License; MIT or Apache 2.0"),
        (
            "tray-icon, global-hotkey, keyring, windows-rs",
            "MIT or Apache 2.0",
        ),
        (
            "webview2-com, souvlaki, discord-rich-presence",
            "MIT License",
        ),
        (
            "yt-dlp",
            tr!("The Unlicense. Downloaded on first run, not bundled."),
        ),
        (
            "Deno",
            tr!("MIT License. Downloaded on first run, not bundled."),
        ),
        (
            tr!("Every other component"),
            tr!("Listed with its licence in THIRD_PARTY.md in the source repository."),
        ),
    ]
}

thread_local! {
    /// The newer release the banner offers.
    static UPDATE: RefCell<Option<Release>> = const { RefCell::new(None) };
}

/// The banner for a newer release, or none. With its setup downloaded and checked
/// (`updates::ready_version`), the banner offers "Restart to update" instead of its page.
pub fn show_update(ctx: &Ctx, release: Option<&Release>) {
    UPDATE.with(|u| *u.borrow_mut() = release.cloned());
    refresh_update(ctx);
}

/// The banner again, after the downloaded setup was dropped.
pub fn refresh_update(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<AboutData>();
    let ready = updates::ready_version();
    UPDATE.with(|u| {
        let u = u.borrow();
        let version = u.as_ref().map_or("", |r| r.version.as_str());
        data.set_update_version(version.into());
        data.set_update_ready(!version.is_empty() && ready.as_deref() == Some(version));
    });
}

/// The component list, in the language shown now.
fn show_credits(data: &AboutData) {
    let credits: Vec<Credit> = credits()
        .iter()
        .map(|(name, licence)| Credit {
            name: (*name).into(),
            licence: (*licence).into(),
        })
        .collect();
    data.set_credits(ModelRc::new(VecModel::from(credits)));
}

/// Once at startup: the texts change only with the language.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<AboutData>();
    let c = ctx.clone();
    data.on_get_update(move || {
        let Some(release) = UPDATE.with(|u| u.borrow().clone()) else {
            return;
        };
        if updates::ready_version().as_deref() == Some(release.version.as_str()) {
            updates::restart(&c);
        } else {
            updates::open(&c, &release.url);
        }
    });
    data.set_version(env!("CARGO_PKG_VERSION").into());
    data.set_mit_text(text(MIT).into());
    data.set_ofl_text(text(OFL).into());
    show_credits(&data);
    let weak = ui.as_weak();
    language::on_change(move || {
        if let Some(ui) = weak.upgrade() {
            show_credits(&ui.global::<AboutData>());
        }
    });
    data.on_open_log_folder(open_log_folder);
    let c = ctx.clone();
    data.on_diagnostics(move || diagnostics(&c).into());
    let c = ctx.clone();
    data.on_copied(move || c.toast(tr!("Diagnostics copied")));
}

/// The diagnostics text: what's known now, with the account's state but never its name.
fn diagnostics(ctx: &Ctx) -> String {
    let settings = Settings::load(&Paths::resolve());
    diagnostics::report(ctx.signed_in(), &settings).text()
}

/// Opens the log folder in Explorer, off the UI thread (the shell call can take a moment).
fn open_log_folder() {
    let dir = diagnostics::log_dir(&Paths::resolve());
    let opened = std::thread::Builder::new()
        .name("featherplay-open-logs".into())
        .spawn(move || {
            if let Err(e) = std::fs::create_dir_all(&dir)
                .map_err(|e| e.to_string())
                .and_then(|()| shell_open(&dir))
            {
                log!("open the log folder {}: {e}", dir.display());
            }
        });
    if let Err(e) = opened {
        log!("open the log folder: {e}");
    }
}

/// Opens `path` as Explorer would (a folder opens in a window of its own).
fn shell_open(path: &Path) -> Result<(), String> {
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};

    let path = HSTRING::from(path.as_os_str());
    // SAFETY: COM is initialised on this thread for the shell call and released after; the
    // strings outlive the call.
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let done = ShellExecuteW(None, w!("open"), &path, None, None, SW_SHOWNORMAL);
        if com.is_ok() {
            CoUninitialize();
        }
        // Values above 32 mean success.
        if done.0 as isize > 32 {
            Ok(())
        } else {
            Err(format!("ShellExecute returned {}", done.0 as isize))
        }
    }
}

/// The page is being shown.
pub fn open(_ctx: &Rc<Ctx>) {}

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(_ctx: &Rc<Ctx>, _state: &PlayerState) {}

/// Signed in or out.
pub fn account_changed(_ctx: &Rc<Ctx>, _signed_in: bool) {}

/// The text as shown: plain line breaks (the checkout may have CRLF), no outer blank lines.
fn text(s: &str) -> String {
    s.trim().replace("\r\n", "\n")
}
