//! The update notices in the window (offscreen, fixture data, no request): a stop for
//! failures shows plain words in the toast, with the link the controller found; a newer
//! release puts a banner at the top of About; About's version row says what the last check
//! found; Settings has "Check for updates". An update's toast waits for a hidden window,
//! says what the banner says, stays ten seconds, and never follows a click in About; a long
//! session checks again when playback resumes, once, if a check is due.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use featherplay::app::{self, Ctx, NativeWindow};
use featherplay::controller::UiEvent;
use featherplay::snapshot::Scene;
use featherplay::ui::{AboutData, Frame, SettingKey, SettingsData, Toast, UpdateState};
use featherplay::updates::install::Ready;
use featherplay::updates::{CheckError, Found, Help, Kind, Notice, Release, Trigger};
use featherplay::{offscreen, pages, updates};
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer};

fn release() -> Release {
    Release {
        version: "0.2.0".into(),
        url: "https://github.com/candygoogy/featherplay/releases/tag/v0.2.0".into(),
        assets: Vec::new(),
    }
}

#[test]
fn a_stop_shows_its_words_and_the_link_found() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("album").expect("album");
    scene.settle();
    let toast = scene.ui.global::<Toast>();

    app::on_event(UiEvent::Stopped {
        text: "Songs can't play. featherplay may need an update.",
        help: Some(Help {
            label: "Details",
            url: "https://github.com/candygoogy/featherplay/issues/42".into(),
            restart: false,
        }),
    });
    assert!(toast.get_open());
    assert_eq!(
        toast.get_text().as_str(),
        "Songs can't play. featherplay may need an update."
    );
    assert_eq!(toast.get_action().as_str(), "Details");
}

#[test]
fn a_stop_with_nothing_to_link_has_no_button() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("album").expect("album");
    scene.settle();
    let toast = scene.ui.global::<Toast>();

    app::on_event(UiEvent::Stopped {
        text: "This song isn't available.",
        help: None,
    });
    assert!(toast.get_open());
    assert_eq!(toast.get_text().as_str(), "This song isn't available.");
    assert_eq!(toast.get_action().as_str(), "");
}

fn shot(scene: &Scene) -> SharedPixelBuffer<Rgba8Pixel> {
    scene.ui.window().take_snapshot().expect("snapshot")
}

#[test]
fn a_newer_release_puts_a_banner_at_the_top_of_about() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("about").expect("about");
    scene.settle();
    let about = scene.ui.global::<AboutData>();
    assert_eq!(about.get_update_version().as_str(), "", "no banner yet");
    let before = shot(&scene);

    app::on_event(UiEvent::Release(release()));
    scene.settle();
    assert_eq!(about.get_update_version().as_str(), "0.2.0");
    let after = shot(&scene);
    // The banner sits above the logo, so the top of the page changes.
    let width = before.width() as usize;
    let top = |s: &SharedPixelBuffer<Rgba8Pixel>| s.as_slice()[..width * 200].to_vec();
    assert_ne!(top(&before), top(&after), "the banner shows at the top");

    // An older or the same version clears it.
    pages::about::show_update(&scene.ctx, None);
    assert_eq!(about.get_update_version().as_str(), "");
}

#[test]
fn check_for_updates_is_a_setting_on_by_default() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    let data = scene.ui.global::<SettingsData>();
    assert!(data.get_check_for_updates());
    assert!(pages::settings::check_for_updates());

    data.invoke_toggled(SettingKey::CheckForUpdates, false);
    assert!(!data.get_check_for_updates());
    assert!(!pages::settings::check_for_updates());
}

#[test]
fn install_updates_is_a_setting_on_by_default() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    let data = scene.ui.global::<SettingsData>();
    assert!(data.get_install_updates());
    assert!(pages::settings::install_updates());

    data.invoke_toggled(SettingKey::InstallUpdates, false);
    assert!(!data.get_install_updates());
    assert!(!pages::settings::install_updates());
}

#[test]
fn a_downloaded_update_offers_restart_to_update() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("about").expect("about");
    scene.settle();
    let about = scene.ui.global::<AboutData>();
    let toast = scene.ui.global::<Toast>();

    updates::ready_for_snapshot(&scene.ctx);
    app::on_event(UiEvent::Release(release()));
    scene.settle();

    assert!(toast.get_open());
    assert_eq!(
        toast.get_text().as_str(),
        "featherplay 0.2.0 is ready to install"
    );
    assert_eq!(toast.get_action().as_str(), "Restart to update");
    assert_eq!(about.get_update_version().as_str(), "0.2.0");
    assert!(
        about.get_update_state() == UpdateState::Ready,
        "the banner offers Restart to update"
    );
}

#[test]
fn a_newer_release_than_the_downloaded_one_is_not_offered_as_ready() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("about").expect("about");
    scene.settle();
    let about = scene.ui.global::<AboutData>();

    updates::ready_for_snapshot(&scene.ctx);
    let mut newer = release();
    newer.version = "0.3.0".into();
    app::on_event(UiEvent::Release(newer));
    scene.settle();

    assert_eq!(about.get_update_version().as_str(), "0.3.0");
    assert!(
        about.get_update_state() != UpdateState::Ready,
        "0.2.0's setup doesn't install 0.3.0"
    );
}

fn now() -> u64 {
    updates::now_unix()
}

fn setup() -> Ready {
    Ready {
        version: "0.2.0".into(),
        file: "featherplay-setup-0.2.0.exe".into(),
        sha256: String::new(),
    }
}

/// About open, with the updater's state as `f` makes it.
fn about_with(f: impl FnOnce(&mut updates::Status)) -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("about").expect("about");
    scene.settle();
    updates::set_status(&scene.ctx, f);
    scene.settle();
    scene
}

#[test]
fn the_version_row_says_when_it_last_checked() {
    let scene = about_with(|_| {});
    let about = scene.ui.global::<AboutData>();
    assert_eq!(about.get_check_line().as_str(), "", "never checked");
    assert_eq!(about.get_check_pill().as_str(), "Check for updates");

    for (ago, line) in [
        (20, "Up to date. Checked just now."),
        (60, "Up to date. Checked 1 minute ago."),
        (2 * 3600 + 300, "Up to date. Checked 2 hours ago."),
        (3 * 86_400, "Up to date. Checked 3 days ago."),
    ] {
        updates::set_status(&scene.ctx, |s| {
            s.stamp.checked = now() - ago;
            s.stamp.answered = now() - ago;
        });
        assert_eq!(about.get_check_line().as_str(), line);
        assert_eq!(about.get_check_pill().as_str(), "Check for updates");
    }

    // A newer release: the banner's button is the action, so the pill goes.
    pages::about::show_update(&scene.ctx, Some(&release()));
    assert_eq!(about.get_check_line().as_str(), "featherplay 0.2.0 is out.");
    assert_eq!(about.get_check_pill().as_str(), "");
}

#[test]
fn a_failed_check_offers_try_again() {
    let scene = about_with(|s| {
        s.stamp.checked = now();
        s.stamp.error = Some(CheckError::Offline);
    });
    let about = scene.ui.global::<AboutData>();
    assert_eq!(
        about.get_check_line().as_str(),
        "Couldn't check: no connection."
    );
    assert_eq!(about.get_check_pill().as_str(), "Try again");

    updates::set_status(&scene.ctx, |s| s.stamp.error = Some(CheckError::Http(502)));
    assert_eq!(
        about.get_check_line().as_str(),
        "Couldn't check: GitHub answered 502."
    );
    assert_eq!(about.get_check_pill().as_str(), "Try again");

    // Rate limited: no pill until GitHub's reset.
    updates::set_status(&scene.ctx, |s| {
        s.stamp.error = Some(CheckError::Limited { until: now() + 600 });
    });
    assert!(
        about
            .get_check_line()
            .starts_with("GitHub is busy. Try again after ")
    );
    assert_eq!(about.get_check_pill().as_str(), "");

    // While a check runs, the pill says so and a click does nothing more.
    updates::set_status(&scene.ctx, |s| {
        s.stamp.error = None;
        s.running = true;
        s.checking = true;
    });
    assert_eq!(about.get_check_line().as_str(), "Checking for updates…");
    assert_eq!(about.get_check_pill().as_str(), "Checking…");
    assert!(about.get_checking());
}

#[test]
fn a_failed_install_says_so_in_the_banner() {
    let scene = about_with(|s| {
        s.kind = Kind::Installed;
        s.stamp.latest = Some(release());
        s.stamp.ready = Some(setup());
        s.stamp.launched = Some("0.2.0".into());
    });
    let about = scene.ui.global::<AboutData>();
    assert_eq!(about.get_update_state(), UpdateState::InstallFailed);

    // It installed after all (0.2.0 runs): nothing to say. Here: launched an older one.
    updates::set_status(&scene.ctx, |s| s.stamp.launched = None);
    assert_eq!(about.get_update_state(), UpdateState::Ready);
    // The download failed; Scoop's copy; the portable copy.
    updates::set_status(&scene.ctx, |s| {
        s.stamp.ready = None;
        s.stamp
            .fetch_tried("0.2.0", std::time::SystemTime::now(), false);
    });
    assert_eq!(about.get_update_state(), UpdateState::DownloadFailed);
    updates::set_status(&scene.ctx, |s| s.kind = Kind::Scoop);
    assert_eq!(about.get_update_state(), UpdateState::Scoop);
    updates::set_status(&scene.ctx, |s| s.kind = Kind::Portable);
    assert_eq!(about.get_update_state(), UpdateState::Get);
}

/// The window's Win32 side: minimised or not.
struct Minimised(Cell<bool>);

impl NativeWindow for Minimised {
    fn minimise(&self) {
        self.0.set(true);
    }

    fn is_minimised(&self) -> bool {
        self.0.get()
    }

    fn bring_forward(&self) {
        self.0.set(false);
    }

    fn animations_enabled(&self) -> bool {
        false
    }
}

fn home() -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    scene
}

#[test]
fn an_update_toast_waits_while_the_window_is_in_the_tray() {
    let scene = home();
    let toast = scene.ui.global::<Toast>();
    scene.ui.global::<Frame>().set_hidden(true);

    updates::tell(&scene.ctx, Notice::Out(release()));
    assert!(!toast.get_open(), "nobody would see it");

    app::show_window(&scene.ctx);
    assert!(toast.get_open(), "shown once the window is back");
    assert_eq!(toast.get_text().as_str(), "featherplay 0.2.0 is out");
    assert_eq!(toast.get_action().as_str(), "Get it");
}

#[test]
fn an_update_toast_waits_while_the_window_is_minimised() {
    let scene = home();
    let toast = scene.ui.global::<Toast>();
    let window = Rc::new(Minimised(Cell::new(true)));
    scene.ctx.set_native(window.clone());

    updates::tell(&scene.ctx, Notice::Ready("0.2.0".into()));
    assert!(!toast.get_open());

    window.0.set(false);
    updates::window_shown(&scene.ctx);
    assert_eq!(
        toast.get_text().as_str(),
        "featherplay 0.2.0 is ready to install"
    );
    assert_eq!(toast.get_action().as_str(), "Restart to update");
}

#[test]
fn an_update_toast_stays_ten_seconds() {
    let scene = home();
    let toast = scene.ui.global::<Toast>();
    offscreen::freeze_clock();

    updates::tell(&scene.ctx, Notice::Out(release()));
    offscreen::advance(Duration::from_secs(6));
    assert!(
        toast.get_open(),
        "a plain action toast would be gone by now"
    );
    offscreen::advance(Duration::from_secs(5));
    assert!(!toast.get_open());
}

#[test]
fn a_failed_install_toast_offers_try_again() {
    let scene = home();
    let toast = scene.ui.global::<Toast>();
    let found = Found {
        newer: Some(release()),
        ready: Some(setup()),
        failed_install: Some("0.2.0".into()),
        ..Found::default()
    };

    // A long session's daily check doesn't say it again; the next start does.
    updates::finished(&scene.ctx, Trigger::Resume, &found);
    assert!(!toast.get_open());
    updates::finished(&scene.ctx, Trigger::Startup, &found);

    assert_eq!(
        toast.get_text().as_str(),
        "The update to 0.2.0 didn't install"
    );
    assert_eq!(toast.get_action().as_str(), "Try again");
}

#[test]
fn a_check_from_about_never_toasts() {
    let found = || Found {
        newer: Some(release()),
        announce: Some(release()),
        ..Found::default()
    };
    // A click in About: answered in About.
    let scene = home();
    let toast = scene.ui.global::<Toast>();
    updates::finished(&scene.ctx, Trigger::Manual, &found());
    assert!(!toast.get_open());
    // A startup check that ends while About shows: its banner says it.
    scene.open("about").expect("about");
    scene.settle();
    updates::finished(&scene.ctx, Trigger::Startup, &found());
    assert!(!toast.get_open());
    // Anywhere else, it's toasted.
    scene.open("home").expect("home");
    scene.settle();
    updates::finished(&scene.ctx, Trigger::Startup, &found());
    assert!(toast.get_open());
}

thread_local! {
    static RUNS: Cell<u32> = const { Cell::new(0) };
}

fn count_run(_ctx: &Rc<Ctx>, trigger: Trigger) {
    assert_eq!(trigger, Trigger::Resume);
    RUNS.with(|r| r.set(r.get() + 1));
}

#[test]
fn resuming_playback_runs_a_due_check_once() {
    let scene = home();
    scene.sound(false).expect("paused");
    // A check is due now; the stand-in counts the runs instead of asking GitHub.
    updates::run_with(count_run, 0);

    scene.sound(true).expect("playing");
    assert_eq!(RUNS.with(Cell::get), 1);
    // Playing on (the position moves), or paused and playing again before it's due: none.
    scene.sound(true).expect("playing");
    scene.sound(false).expect("paused");
    scene.sound(true).expect("playing");
    assert_eq!(RUNS.with(Cell::get), 1, "once, until the run sets the next");
}

#[test]
fn a_check_not_yet_due_is_not_run_on_resume() {
    let scene = home();
    scene.sound(false).expect("paused");
    updates::run_with(count_run, updates::now_unix() + 3600);

    scene.sound(true).expect("playing");
    app::show_window(&scene.ctx);
    assert_eq!(RUNS.with(Cell::get), 0);
}
