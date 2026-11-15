//! The update notices in the window (offscreen, fixture data, no request): a newer release
//! puts a banner at the top of About, which follows the updater's state; About's version row
//! says what the last check found. An update's toast waits for a hidden window, says what
//! the banner says, stays ten seconds, and never follows a click in About; a long session
//! checks again when playback resumes, once, if a check is due. The stops' toasts are in
//! `toasts`, the switches in `setting_switches`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use featherplay::app::{self, Ctx, NativeWindow};
use featherplay::controller::UiEvent;
use featherplay::snapshot::Scene;
use featherplay::ui::{AboutData, Frame, Toast, UpdateState};
use featherplay::updates::install::Ready;
use featherplay::updates::{CheckError, Found, Kind, Notice, Release, Trigger};
use featherplay::{offscreen, pages, updates};
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer};

fn release() -> Release {
    Release {
        version: "9.2.0".into(),
        url: "https://github.com/candygoogy/featherplay/releases/tag/v9.2.0".into(),
        assets: Vec::new(),
    }
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
    assert_eq!(about.get_update_version().as_str(), "9.2.0");
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
        "featherplay 9.2.0 is ready to install"
    );
    assert_eq!(toast.get_action().as_str(), "Restart to update");
    assert_eq!(about.get_update_version().as_str(), "9.2.0");
    assert!(
        about.get_update_state() == UpdateState::Ready,
        "the banner offers Restart to update"
    );
}

fn now() -> u64 {
    updates::now_unix()
}

fn setup() -> Ready {
    Ready {
        version: "9.2.0".into(),
        file: "featherplay-setup-9.2.0.exe".into(),
        sha256: String::new(),
        signed: true,
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
fn the_version_row_says_what_the_last_check_found() {
    let now = now();
    // (case, the updater's state, the row's line, its pill)
    type Case = (
        &'static str,
        fn(&mut updates::Status, u64),
        &'static str,
        &'static str,
    );
    let cases: [Case; 9] = [
        ("never checked", |_, _| {}, "", "Check for updates"),
        (
            "checked 20 s ago",
            |s, now| s.stamp.answered = now - 20,
            "Up to date. Checked just now.",
            "Check for updates",
        ),
        (
            "checked 60 s ago",
            |s, now| s.stamp.answered = now - 60,
            "Up to date. Checked 1 minute ago.",
            "Check for updates",
        ),
        (
            "checked 2 h 5 min ago",
            |s, now| s.stamp.answered = now - (2 * 3600 + 300),
            "Up to date. Checked 2 hours ago.",
            "Check for updates",
        ),
        (
            "checked 3 days ago",
            |s, now| s.stamp.answered = now - 3 * 86_400,
            "Up to date. Checked 3 days ago.",
            "Check for updates",
        ),
        // The banner's button is the action, so the pill goes.
        (
            "a newer release",
            |s, now| {
                s.stamp.answered = now - 20;
                s.stamp.latest = Some(release());
            },
            "featherplay 9.2.0 is out.",
            "",
        ),
        (
            "offline",
            |s, now| {
                s.stamp.checked = now;
                s.stamp.error = Some(CheckError::Offline);
            },
            "Couldn't check: no connection.",
            "Try again",
        ),
        (
            "GitHub answered 502",
            |s, now| {
                s.stamp.checked = now;
                s.stamp.error = Some(CheckError::Http(502));
            },
            "Couldn't check: GitHub answered 502.",
            "Try again",
        ),
        // While a check runs, the pill says so and a click does nothing more.
        (
            "checking",
            |s, _| {
                s.running = true;
                s.checking = true;
            },
            "Checking for updates…",
            "Checking…",
        ),
    ];
    for (case, state, line, pill) in cases {
        let mut status = updates::Status::default();
        state(&mut status, now);
        let (got_line, got_pill) = pages::about::version_row(&status, now);
        assert_eq!(got_line, line, "{case}: the line");
        assert_eq!(got_pill, pill, "{case}: the pill");
    }

    // Rate limited: no pill until GitHub's reset.
    let mut limited = updates::Status::default();
    limited.stamp.checked = now;
    limited.stamp.error = Some(CheckError::Limited { until: now + 600 });
    let (line, pill) = pages::about::version_row(&limited, now);
    assert!(
        line.starts_with("GitHub is busy. Try again after "),
        "rate limited: {line}"
    );
    assert_eq!(pill, "", "rate limited: the pill");
}

/// About's banner and version row follow the updater's state: a walk through the banner's
/// states.
#[test]
fn about_shows_the_updaters_state_in_its_banner_and_version_row() {
    let scene = about_with(|s| {
        s.running = true;
        s.checking = true;
    });
    let about = scene.ui.global::<AboutData>();
    assert_eq!(about.get_check_line().as_str(), "Checking for updates…");
    assert_eq!(about.get_check_pill().as_str(), "Checking…");
    assert!(about.get_checking());

    updates::set_status(&scene.ctx, |s| {
        s.running = false;
        s.checking = false;
        s.kind = Kind::Installed;
        s.stamp.latest = Some(release());
        s.stamp.ready = Some(setup());
        s.stamp.launched = Some("9.2.0".into());
    });
    assert!(!about.get_checking());
    assert_eq!(
        about.get_update_state(),
        UpdateState::InstallFailed,
        "a failed install says so"
    );
    // It installed after all (9.2.0 runs): nothing to say. Here: launched an older one.
    updates::set_status(&scene.ctx, |s| s.stamp.launched = None);
    assert_eq!(about.get_update_state(), UpdateState::Ready);
    // "Restart to update" was clicked: the setup is starting.
    updates::set_status(&scene.ctx, |s| s.restarting = true);
    assert_eq!(about.get_update_state(), UpdateState::Starting);
    // The setup wouldn't start: it's offered as a page to open instead.
    updates::set_status(&scene.ctx, |s| {
        s.restarting = false;
        s.launch_failed = true;
        s.stamp.ready = None;
    });
    assert_eq!(about.get_update_state(), UpdateState::CannotInstall);
    // Downloading; then the download failed.
    updates::set_status(&scene.ctx, |s| {
        s.launch_failed = false;
        s.downloading = true;
    });
    assert_eq!(about.get_update_state(), UpdateState::Downloading);
    updates::set_status(&scene.ctx, |s| {
        s.downloading = false;
        s.stamp
            .fetch_tried("9.2.0", std::time::SystemTime::now(), false);
    });
    assert_eq!(about.get_update_state(), UpdateState::DownloadFailed);
    // An update that could not be verified says so.
    updates::set_status(&scene.ctx, |s| {
        s.stamp
            .fetch_unverified("9.2.0", std::time::SystemTime::now());
    });
    assert_eq!(about.get_update_state(), UpdateState::Unverified);
    // Scoop's copy; the portable copy.
    updates::set_status(&scene.ctx, |s| s.kind = Kind::Scoop);
    assert_eq!(about.get_update_state(), UpdateState::Scoop);
    updates::set_status(&scene.ctx, |s| s.kind = Kind::Portable);
    assert_eq!(about.get_update_state(), UpdateState::Get);
}

#[test]
fn an_unverified_update_toast_leads_to_about() {
    let scene = home();
    let toast = scene.ui.global::<Toast>();
    let found = Found {
        newer: Some(release()),
        announce: Some(release()),
        unverified: true,
        ..Found::default()
    };

    updates::finished(&scene.ctx, Trigger::Startup, &found);

    assert_eq!(
        toast.get_text().as_str(),
        "The update to 9.2.0 couldn't be verified"
    );
    assert_eq!(toast.get_action().as_str(), "View");
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
    assert_eq!(toast.get_text().as_str(), "featherplay 9.2.0 is out");
    assert_eq!(toast.get_action().as_str(), "Get it");
}

#[test]
fn an_update_toast_waits_while_the_window_is_minimised() {
    let scene = home();
    let toast = scene.ui.global::<Toast>();
    let window = Rc::new(Minimised(Cell::new(true)));
    scene.ctx.set_native(window.clone());

    updates::tell(&scene.ctx, Notice::Ready("9.2.0".into()));
    assert!(!toast.get_open());

    window.0.set(false);
    updates::window_shown(&scene.ctx);
    assert_eq!(
        toast.get_text().as_str(),
        "featherplay 9.2.0 is ready to install"
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
        failed_install: Some("9.2.0".into()),
        ..Found::default()
    };

    // A long session's daily check doesn't say it again; the next start does.
    updates::finished(&scene.ctx, Trigger::Resume, &found);
    assert!(!toast.get_open());
    updates::finished(&scene.ctx, Trigger::Startup, &found);

    assert_eq!(
        toast.get_text().as_str(),
        "The update to 9.2.0 didn't install"
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
fn resuming_playback_runs_a_check_once_when_one_is_due() {
    let scene = home();
    scene.sound(false).expect("paused");
    // Not due yet: neither playing again nor the window coming back runs one. The stand-in
    // counts the runs instead of asking GitHub.
    updates::run_with(count_run, updates::now_unix() + 3600);
    scene.sound(true).expect("playing");
    app::show_window(&scene.ctx);
    assert_eq!(RUNS.with(Cell::get), 0, "not due yet");

    // Due now.
    scene.sound(false).expect("paused");
    updates::run_with(count_run, 0);
    scene.sound(true).expect("playing");
    assert_eq!(RUNS.with(Cell::get), 1, "due");
    // Playing on (the position moves), or paused and playing again before it's due: none.
    scene.sound(true).expect("playing");
    scene.sound(false).expect("paused");
    scene.sound(true).expect("playing");
    assert_eq!(RUNS.with(Cell::get), 1, "once, until the run sets the next");
}
