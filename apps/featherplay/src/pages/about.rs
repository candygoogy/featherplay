//! About page (ui/pages/about.slint): the version row with "Check for updates" and what the
//! last check found, Open log folder and Copy diagnostics, the licences in full and a short
//! list of third-party components, and a banner when a newer release is out (`updates`). The tagline, the disclaimer and the Made with Slint badge are in the
//! Slint file.

use std::path::Path;
use std::rc::Rc;

use featherplay_core::{tr, trn};
use featherplay_store::{Paths, Settings};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app::Ctx;
use crate::controller::PlayerState;
use crate::diagnostics;
use crate::language;
use crate::pages;
use crate::ui::{AboutData, Credit, UpdateState};
use crate::updates::{self, Banner, CheckError, Release, Status};

const MIT: &str = include_str!("../../../../LICENSE");
const OFL: &str = include_str!("../../ui/fonts/manrope/OFL.txt");

/// The main components and their licences; `THIRD_PARTY.md` has every crate.
/// Licence names stay as their authors wrote them; the words around them are translated.
fn credits() -> [(&'static str, &'static str); 12] {
    [
        (
            "Slint",
            "Royalty-free Desktop, Mobile, and Web Applications License 2.0",
        ),
        (tr!("Skia, Slint's renderer"), "BSD 3-Clause License"),
        ("ytmapi-rs", "MIT License"),
        (tr!("rodio and cpal"), "MIT or Apache 2.0; Apache 2.0"),
        ("Symphonia", "Mozilla Public License 2.0"),
        ("QuickJS-NG", "MIT License"),
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

/// The latest release a check found elsewhere ("can't play"), or none: the banner and the
/// version row follow.
pub fn show_update(ctx: &Ctx, release: Option<&Release>) {
    updates::set_status(ctx, |s| s.stamp.latest = release.cloned());
}

/// The version row and the banner, as the updater's state is now (`updates::status`). The
/// row's "Checked … ago" is worked out here, when About opens and after a check; it doesn't
/// tick.
pub fn refresh_update(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<AboutData>();
    let status = updates::status();
    let newer = status.stamp.newer(updates::CURRENT);
    data.set_update_version(newer.map_or("", |r| r.version.as_str()).into());
    data.set_update_state(banner_state(
        status.banner(pages::settings::install_updates()),
    ));
    let (line, pill) = version_row(&status, updates::now_unix());
    data.set_check_line(line.into());
    data.set_check_pill(pill.into());
    data.set_checking(status.checking);
}

fn banner_state(banner: Banner) -> UpdateState {
    match banner {
        Banner::None => UpdateState::None,
        Banner::Get => UpdateState::Get,
        Banner::Downloading => UpdateState::Downloading,
        Banner::Ready => UpdateState::Ready,
        Banner::DownloadFailed => UpdateState::DownloadFailed,
        Banner::CannotInstall => UpdateState::CannotInstall,
        Banner::InstallFailed => UpdateState::InstallFailed,
        Banner::Scoop => UpdateState::Scoop,
        Banner::Starting => UpdateState::Starting,
        Banner::Unverified => UpdateState::Unverified,
    }
}

/// The version row's status line and its pill's label ("" for none) at `now` (seconds
/// since 1970): the board's S0-S4c.
pub fn version_row(status: &Status, now: u64) -> (String, String) {
    let check = || tr!("Check for updates").to_string();
    let again = || tr!("Try again").to_string();
    // S3: the banner's button is the action.
    if let Some(r) = status.stamp.newer(updates::CURRENT) {
        return (tr!("featherplay {} is out.", r.version), String::new());
    }
    if status.checking {
        return (tr!("Checking for updates…").into(), tr!("Checking…").into());
    }
    match status.stamp.error {
        // No pill until GitHub's reset; the next visit after it shows one.
        Some(CheckError::Limited { until }) => (
            tr!(
                "GitHub is busy. Try again after {}.",
                clock_after(until, now)
            ),
            if now >= until { again() } else { String::new() },
        ),
        Some(CheckError::Offline) => (tr!("Couldn't check: no connection.").into(), again()),
        Some(CheckError::Http(code)) => (tr!("Couldn't check: GitHub answered {}.", code), again()),
        Some(CheckError::Unreadable) => (
            tr!("Couldn't check: the answer wasn't readable.").into(),
            again(),
        ),
        None if status.stamp.answered > 0 => (checked_ago(now, status.stamp.answered), check()),
        None => (String::new(), check()),
    }
}

/// "Up to date. Checked 2 hours ago.", from the last answer.
fn checked_ago(now: u64, answered: u64) -> String {
    let secs = now.saturating_sub(answered);
    let (minutes, hours, days) = (secs / 60, secs / 3600, secs / 86_400);
    if minutes == 0 {
        tr!("Up to date. Checked just now.").into()
    } else if hours == 0 {
        trn!(
            "Up to date. Checked {n} minute ago."
                | "Up to date. Checked {n} minutes ago." % minutes
        )
    } else if days == 0 {
        trn!("Up to date. Checked {n} hour ago." | "Up to date. Checked {n} hours ago." % hours)
    } else {
        trn!("Up to date. Checked {n} day ago." | "Up to date. Checked {n} days ago." % days)
    }
}

/// The local clock time `until` (seconds since 1970) falls at, "14:32", rounded up to the
/// minute, from the local time now.
fn clock_after(until: u64, now: u64) -> String {
    let local = crate::diagnostics::clock::Stamp::now();
    let wait = until.saturating_sub(now).div_ceil(60);
    let minutes = (u64::from(local.hour) * 60 + u64::from(local.minute) + wait) % (24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The banner's solid button (`primary`) or its text link, as its state has them.
fn update_clicked(ctx: &Rc<Ctx>, primary: bool) {
    let status = updates::status();
    let Some(release) = status.stamp.newer(updates::CURRENT).cloned() else {
        return;
    };
    match (status.banner(pages::settings::install_updates()), primary) {
        (Banner::Ready | Banner::InstallFailed, true) => updates::restart(ctx),
        (Banner::DownloadFailed, true) => updates::retry_download(ctx),
        (Banner::InstallFailed, false) => open_log_folder(),
        // Not the release page: its files are what couldn't be verified.
        (Banner::Unverified, true) => updates::open(ctx, updates::SITE),
        // "Get it", "What's new", "Get it on GitHub": the release's page.
        (Banner::Get | Banner::CannotInstall, true)
        | (
            Banner::Get
            | Banner::Downloading
            | Banner::Ready
            | Banner::DownloadFailed
            | Banner::Scoop,
            false,
        ) => updates::open(ctx, &release.url),
        _ => {}
    }
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
    data.on_get_update(move || update_clicked(&c, true));
    let c = ctx.clone();
    data.on_update_secondary(move || update_clicked(&c, false));
    let c = ctx.clone();
    data.on_check_now(move || updates::check_now(&c));
    let c = ctx.clone();
    data.on_command_copied(move || c.toast(tr!("Command copied")));
    data.set_version(env!("CARGO_PKG_VERSION").into());
    refresh_update(ctx);
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

/// What the button below does, from elsewhere (the failed helpers download's toast): the
/// same text on the clipboard, then the same "Diagnostics copied".
pub fn copy_diagnostics(ctx: &Rc<Ctx>) {
    match crate::native::copy_text(&diagnostics(ctx)) {
        Ok(()) => ctx.toast(tr!("Diagnostics copied")),
        Err(e) => log!("copy diagnostics: {e}"),
    }
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

/// The page is being shown: "Checked … ago" as of now.
pub fn open(ctx: &Rc<Ctx>) {
    refresh_update(ctx);
}

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(_ctx: &Rc<Ctx>, _state: &PlayerState) {}

/// Signed in or out.
pub fn account_changed(_ctx: &Rc<Ctx>, _signed_in: bool) {}

/// The text as shown: plain line breaks (the checkout may have CRLF), no outer blank lines.
fn text(s: &str) -> String {
    s.trim().replace("\r\n", "\n")
}
