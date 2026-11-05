//! Paused with the window on screen, nothing renders and no timer is armed (the paused CPU
//! budget, 0.012 % of the machine), on the offscreen platform (no window is shown), with
//! Windows' animation effects on: on each page once it has loaded, after pausing from the
//! tray icon's menu, after opening and closing the mini player (open and unpinned, its
//! foreground poll is the one timer), and with it pinned.

use std::time::{Duration, Instant};

use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, MiniData, Player, Theme};
use featherplay::{mini, offscreen, shell};
use featherplay_integrations::TrayChoice;
use slint::ComponentHandle;
use slint::platform::WindowEvent;

/// Runs timers and animations for `ms`, rendering a frame every few milliseconds as the
/// window would.
fn run_for(scene: &Scene, windows: &[&slint::Window], ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        scene.settle_once();
        for w in windows {
            w.take_snapshot().expect("snapshot");
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// The frames the windows ask for over `ms`.
fn frames_over(scene: &Scene, windows: &[&slint::Window], ms: u64) -> u64 {
    let before = offscreen::redraws();
    run_for(scene, windows, ms);
    offscreen::redraws() - before
}

/// Lets `windows` settle, then checks that over a further half second nothing asks for a
/// frame and no timer is armed.
fn assert_idle(scene: &Scene, windows: &[&slint::Window], after: &str) {
    run_for(scene, windows, 800);
    let frames = frames_over(scene, windows, 500);
    let animating: Vec<bool> = windows.iter().map(|w| w.has_active_animations()).collect();
    let timer = slint::platform::duration_until_next_timer_update();
    assert!(
        frames == 0 && !animating.contains(&true) && timer.is_none(),
        "after {after}: {frames} frame(s) asked for in 500 ms, animating {animating:?}, \
         next timer {timer:?}"
    );
}

/// As `assert_idle`, with the open, unpinned mini player's foreground poll as the one timer
/// allowed (pinning it next leaves none, which shows the poll was the only one).
fn assert_idle_but_the_poll(scene: &Scene, windows: &[&slint::Window], after: &str) {
    run_for(scene, windows, 800);
    let frames = frames_over(scene, windows, 500);
    let animating: Vec<bool> = windows.iter().map(|w| w.has_active_animations()).collect();
    let timer = slint::platform::duration_until_next_timer_update();
    assert!(
        frames == 0
            && !animating.contains(&true)
            && mini::polls()
            && timer.is_some_and(|t| t <= mini::POLL),
        "after {after}: {frames} frame(s) asked for in 500 ms, animating {animating:?}, \
         next timer {timer:?}, polling {}",
        mini::polls()
    );
}

/// One test: the platform can be installed once per process.
#[test]
fn paused_nothing_renders_and_no_timer_is_armed() {
    let scene = Scene::new(1280, 800, false, false).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.sound(false).expect("paused");
    scene.theme(true);
    scene.ui.show().expect("show");
    let main = scene.ui.window();
    for page in [
        "home",
        "library",
        "album",
        "playlist",
        "liked",
        "artist",
        "search",
        "now-playing",
        "now-playing-lyrics",
        "settings",
    ] {
        scene.open(page).expect("page");
        scene.settle();
        assert_idle(&scene, &[main], &format!("opening {page}"));
    }

    // Playing, Pause in the tray icon's menu goes through the main window's player, and the
    // controller reports the pause.
    scene.open("home").expect("home");
    scene.sound(true).expect("playing");
    scene.settle();
    // The count works: playing, the Up next mark and the bar's clock ask for frames.
    scene.ui.global::<Frame>().set_queue_open(true);
    assert!(
        frames_over(&scene, &[main], 500) > 0,
        "playing asked for no frames"
    );
    scene.ui.global::<Frame>().set_queue_open(false);
    shell::tray_choice(TrayChoice::PlayPause);
    scene.sound(false).expect("paused");
    scene.settle();
    assert!(!scene.ui.global::<Player>().get_playing());
    assert_idle(&scene, &[main], "pausing from the tray menu");

    // The mini player, opened and closed: open and unpinned, only its foreground poll runs.
    let player = mini::create(&scene.ctx).expect("mini player");
    player.show().expect("show");
    player
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    scene.settle();
    assert_idle_but_the_poll(&scene, &[main, player.window()], "opening the mini player");
    player
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(false));
    scene.settle();
    assert!(!mini::is_open(), "the mini player didn't close");
    drop(player);
    assert_idle(&scene, &[main], "closing the mini player");
    assert_eq!(
        offscreen::windows_alive(),
        1,
        "a closed popup's window lives on"
    );

    // Pinned, it stays open without the activation, and still keeps no timer.
    let player = mini::create(&scene.ctx).expect("mini player");
    player.show().expect("show");
    player
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    player.global::<MiniData>().invoke_toggle_pin();
    player
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(false));
    scene.settle();
    assert!(mini::is_open(), "the pinned mini player closed");
    assert_idle(&scene, &[main, player.window()], "pinning the mini player");
    mini::close();
    drop(player);
    assert_idle(&scene, &[main], "closing the pinned mini player");
    assert_eq!(
        offscreen::windows_alive(),
        1,
        "a closed popup's window lives on"
    );
}
