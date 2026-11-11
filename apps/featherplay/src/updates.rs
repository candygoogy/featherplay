//! The update check's triggers and what the window shows of it: the startup run once the
//! first page is up, a long session's runs (`maybe_run`), About's buttons, the toasts (held
//! while the window is hidden), "Restart to update", and About's snapshot states. The check
//! itself, its stamp and the download are `featherplay_controller::updates`, re-exported
//! here.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use featherplay_core::tr;
use slint::ComponentHandle;

use featherplay_controller::updates::install::Ready;
pub use featherplay_controller::updates::*;

use crate::app::Ctx;
use crate::router::Page;
use crate::ui::{Frame, ToastTone};
use crate::{busy, pages, toast};

/// How long an update's toast stays (hovering holds it): longer than a plain toast, as it
/// comes once and unasked.
const TOAST_FOR: Duration = Duration::from_secs(10);

/// About's banner and row may have changed off the UI thread: they're redrawn there.
fn changed_elsewhere() {
    let _ = slint::invoke_from_event_loop(|| {
        if let Some(ctx) = Ctx::get() {
            pages::about::refresh_update(&ctx);
        }
    });
}

/// Hands the update runs `changed_elsewhere`. `Ctx::new` calls it, so the window, the tests
/// and the snapshots all redraw About as before.
pub fn connect() {
    on_change(changed_elsewhere);
}

thread_local! {
    /// The startup run is still to come (the real app only: never in tests or snapshots).
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static CACHE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// When a long session's run is due (seconds since 1970); none until the startup run
    /// has ended, and while one runs.
    static NEXT_DUE: Cell<Option<u64>> = const { Cell::new(None) };
    /// A toast that came while the window was hidden: shown when it's back.
    static HELD: RefCell<Option<Notice>> = const { RefCell::new(None) };
    /// What starts a run; tests put a stand-in here (`run_with`).
    static STARTER: Cell<fn(&Rc<Ctx>, Trigger)> = const { Cell::new(start) };
}

/// The app's window is up: the startup run comes once the first page has loaded.
pub fn arm(cache: PathBuf) {
    CACHE.with(|c| *c.borrow_mut() = Some(cache));
    ARMED.with(|a| a.set(true));
    let kind = Kind::detect();
    edit_status(|s| s.kind = kind);
}

/// Tests: runs start with `starter` instead of a request, and a long session's run is due
/// at `due` (seconds since 1970).
pub fn run_with(starter: fn(&Rc<Ctx>, Trigger), due: u64) {
    STARTER.with(|s| s.set(starter));
    NEXT_DUE.with(|d| d.set(Some(due)));
}

/// A load arrived, or the session became known: once nothing is loading any more, the
/// first page is up, and the startup run comes (once). With "Check for updates" off it
/// asks nothing, but reads the stamp for About and tells of an update that didn't install.
pub fn page_settled(ctx: &Rc<Ctx>) {
    if !ARMED.with(Cell::get) || ctx.session_pending() || busy::pending() > 0 {
        return;
    }
    ARMED.with(|a| a.set(false));
    if !pages::settings::check_for_updates() {
        log!("update check: off in Settings");
    }
    STARTER.with(Cell::get)(ctx, Trigger::Startup);
}

/// Playback resumed, the window came back, or "Check for updates" was turned on: a run if
/// one is due. One comparison, nothing read from disk.
pub fn maybe_run(ctx: &Rc<Ctx>) {
    let Some(due) = NEXT_DUE.with(Cell::get) else {
        return;
    };
    if now_unix() < due || !pages::settings::check_for_updates() {
        return;
    }
    NEXT_DUE.with(|d| d.set(None));
    STARTER.with(Cell::get)(ctx, Trigger::Resume);
}

/// About's "Check for updates", or "Try again" after a failed check.
pub fn check_now(ctx: &Rc<Ctx>) {
    STARTER.with(Cell::get)(ctx, Trigger::Manual);
}

/// The banner's "Try again" after a failed download.
pub fn retry_download(ctx: &Rc<Ctx>) {
    STARTER.with(Cell::get)(ctx, Trigger::Retry);
}

/// Starts a run on the runtime, unless one is going (then a startup or resume run is due
/// again a minute later, so a long session doesn't stop checking).
fn start(ctx: &Rc<Ctx>, trigger: Trigger) {
    let Some(cache) = CACHE.with(|c| c.borrow().clone()) else {
        return;
    };
    // A click shows "Checking…" at once; otherwise only once GitHub is asked.
    let Some(running) = Running::begin(trigger == Trigger::Manual) else {
        if matches!(trigger, Trigger::Startup | Trigger::Resume) {
            NEXT_DUE.with(|d| d.set(Some(now_unix() + COOLDOWN)));
        }
        return;
    };
    let (check_on, install_on) = (
        pages::settings::check_for_updates(),
        pages::settings::install_updates(),
    );
    let installed = status().kind == Kind::Installed;
    pages::about::refresh_update(ctx);
    ctx.load(
        move |_| {
            let remote = Remote::github(installed);
            run(running, cache, trigger, check_on, install_on, remote)
        },
        move |ctx, _ui, found| finished(ctx, trigger, &found),
    );
}

/// A run ended: About shows its answer, a long session's next run is set, and a startup or
/// resume run's news is toasted, unless About is showing it already.
pub fn finished(ctx: &Rc<Ctx>, trigger: Trigger, found: &Found) {
    NEXT_DUE.with(|d| d.set(Some(found.next_due)));
    pages::about::refresh_update(ctx);
    if let Some(notice) = Notice::of(found, trigger) {
        tell(ctx, notice);
    }
}

/// Toasts `notice`, or holds it until the window is back from the tray or the taskbar. Not
/// on About, whose banner says it.
pub fn tell(ctx: &Rc<Ctx>, notice: Notice) {
    if ctx.current_page() == Page::About {
        return;
    }
    let hidden = ctx
        .window()
        .is_none_or(|ui| ui.global::<Frame>().get_hidden())
        || ctx.native().is_some_and(|w| w.is_minimised());
    if hidden {
        HELD.with(|h| *h.borrow_mut() = Some(notice));
    } else {
        show(ctx, &notice);
    }
}

/// The window is back (from the tray, or the taskbar): a toast held for it shows, and a
/// run comes if one is due.
pub fn window_shown(ctx: &Rc<Ctx>) {
    if let Some(notice) = HELD.with(|h| h.borrow_mut().take())
        && ctx.current_page() != Page::About
    {
        show(ctx, &notice);
    }
    maybe_run(ctx);
}

/// News is neutral; an update that didn't install or couldn't be verified is an error.
fn show(ctx: &Ctx, notice: &Notice) {
    match notice {
        Notice::Out(release) if status().kind == Kind::Scoop => toast::show_for(
            ctx,
            ToastTone::Neutral,
            &tr!("featherplay {} is out", release.version),
            tr!("View"),
            |c| c.navigate(Page::About),
            TOAST_FOR,
        ),
        Notice::Out(release) => {
            let url = release.url.clone();
            toast::show_for(
                ctx,
                ToastTone::Neutral,
                &tr!("featherplay {} is out", release.version),
                tr!("Get it"),
                move |c| open(c, &url),
                TOAST_FOR,
            );
        }
        Notice::Ready(version) => toast::show_for(
            ctx,
            ToastTone::Neutral,
            &tr!("featherplay {} is ready to install", version),
            tr!("Restart to update"),
            restart,
            TOAST_FOR,
        ),
        Notice::InstallFailed(version) => toast::show_for(
            ctx,
            ToastTone::Error,
            &tr!("The update to {} didn't install", version),
            tr!("Try again"),
            restart,
            TOAST_FOR,
        ),
        Notice::Unverified(version) => toast::show_for(
            ctx,
            ToastTone::Error,
            &tr!("The update to {} couldn't be verified", version),
            tr!("View"),
            |c| c.navigate(Page::About),
            TOAST_FOR,
        ),
    }
}

/// "Restart to update" (the toast's and the banner's), or "Try again" after an update
/// didn't install: starts the downloaded setup (checked once more), then quits; the setup
/// starts the new version. The version is noted first, so the next start can tell if the
/// update didn't install. If the setup can't be started, the release is offered as a page
/// to open instead.
pub fn restart(ctx: &Rc<Ctx>) {
    let install_on = pages::settings::install_updates();
    let (Some(ready), Some(cache)) = (
        status().ready(install_on).cloned(),
        CACHE.with(|c| c.borrow().clone()),
    ) else {
        return;
    };
    // The stamp has one writer at a time: not while a run (a check, a download) is going.
    let Some(running) = Running::begin(false) else {
        return;
    };
    if edit_status(|s| std::mem::replace(&mut s.restarting, true)) {
        return;
    }
    pages::about::refresh_update(ctx);
    log!("update: starting the setup for {}", ready.version);
    ctx.load(
        move |_| async move {
            tokio::task::spawn_blocking(move || {
                let _running = running;
                let path = stamp_path(&cache);
                let mut stamp = Stamp::load(&path);
                stamp.launched = Some(ready.version.clone());
                if let Err(e) = stamp.save(&path) {
                    log!("saving the update check: {}", e.kind());
                }
                let started = install::launch(&cache, &ready);
                if started.is_err() {
                    stamp.launched = None;
                    let _ = stamp.save(&path);
                }
                started
            })
            .await
            .map_err(|e| format!("setup task: {e}"))?
        },
        |ctx, _ui, started| match started {
            Ok(()) => {
                log!("update: the setup is running; quitting");
                ctx.quit();
            }
            Err(e) => {
                log!("update: {e}");
                edit_status(|s| {
                    s.restarting = false;
                    s.launch_failed = true;
                });
                pages::about::refresh_update(ctx);
                ctx.toast_error(tr!("The update couldn't start"));
            }
        },
    );
}

/// The version About's snapshots and tests offer as the newer release: far ahead of any real
/// one, so they keep working as featherplay's own version climbs.
pub const SAMPLE_VERSION: &str = "9.2.0";

/// The release used by About's snapshots and tests: `SAMPLE_VERSION`, with its page.
pub fn sample_release() -> Release {
    Release {
        version: SAMPLE_VERSION.into(),
        url: format!("https://github.com/candygoogy/featherplay/releases/tag/v{SAMPLE_VERSION}"),
        assets: Vec::new(),
    }
}

/// Sets the updater's state as `f` makes it, and redraws About: `--snapshot about-…` and
/// tests.
pub fn set_status(ctx: &Ctx, f: impl FnOnce(&mut Status)) {
    edit_status(f);
    pages::about::refresh_update(ctx);
}

/// `--snapshot about-update-ready`: a setup for `SAMPLE_VERSION` waiting, with its toast.
pub fn ready_for_snapshot(ctx: &Ctx) {
    set_status(ctx, |s| {
        s.kind = Kind::Installed;
        s.stamp.latest = Some(sample_release());
        s.stamp.ready = Some(Ready {
            version: SAMPLE_VERSION.into(),
            file: format!("featherplay-setup-{SAMPLE_VERSION}.exe"),
            sha256: String::new(),
            signed: true,
        });
    });
    show(ctx, &Notice::Ready(SAMPLE_VERSION.into()));
}

/// `--snapshot about-…`: the version row and the banner in the state `page` names (the
/// design board's S1-S4c and B1-B9; plain `about` is S0).
pub fn for_snapshot(ctx: &Ctx, page: &str) {
    if page == "about-update-ready" {
        ready_for_snapshot(ctx);
        return;
    }
    let now = now_unix();
    let setup = Ready {
        version: SAMPLE_VERSION.into(),
        file: format!("featherplay-setup-{SAMPLE_VERSION}.exe"),
        sha256: String::new(),
        signed: true,
    };
    set_status(ctx, |s| {
        s.stamp.checked = now;
        let tried = |refused| Fetch {
            version: SAMPLE_VERSION.into(),
            attempts: 1,
            last: now,
            refused,
            unverified: false,
        };
        match page.strip_prefix("about-") {
            Some("check-checking") => {
                s.running = true;
                s.checking = true;
            }
            Some("check-latest") => s.stamp.answered = now - 2 * 3600,
            Some("check-offline") => s.stamp.error = Some(CheckError::Offline),
            Some("check-limited") => {
                s.stamp.error = Some(CheckError::Limited { until: now + 1800 });
            }
            Some("check-failed") => s.stamp.error = Some(CheckError::Http(502)),
            Some(state) if state.starts_with("update") => {
                s.stamp.latest = Some(sample_release());
                s.kind = Kind::Installed;
                match state {
                    "update-downloading" => s.downloading = true,
                    "update-failed-download" => s.stamp.fetch = Some(tried(false)),
                    "update-cannot-install" => s.stamp.fetch = Some(tried(true)),
                    "update-unverified" => {
                        s.stamp.fetch = Some(Fetch {
                            unverified: true,
                            ..tried(true)
                        });
                    }
                    "update-failed-install" => {
                        s.stamp.ready = Some(setup.clone());
                        s.stamp.launched = Some(SAMPLE_VERSION.into());
                    }
                    "update-scoop" => s.kind = Kind::Scoop,
                    "update-starting" => {
                        s.stamp.ready = Some(setup.clone());
                        s.restarting = true;
                    }
                    // B1: the portable copy.
                    _ => s.kind = Kind::Portable,
                }
            }
            _ => {}
        }
    });
}

/// Opens one of the project's pages, or the website, in the browser, off the UI thread.
pub fn open(ctx: &Ctx, url: &str) {
    if !ours(url) && url != SITE {
        return;
    }
    let url = url.to_string();
    crate::task::blocking_detached(ctx.catalog.runtime(), move || {
        if let Err(e) = featherplay_platform::open::url(&url) {
            log!("opening the release page: {e}");
        }
    });
}
