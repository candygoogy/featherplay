//! The bar's once-a-second clock runs only while something shows the time: minimised (or in
//! the tray) with the mini player closed, nothing wakes once a second while sound plays;
//! the mini player opened then ticks again, and the window shown again shows the time moved
//! on. On the offscreen platform with the fixtures; no window is shown.

use std::time::{Duration, Instant};

use featherplay::snapshot::Scene;
use featherplay::ui::{Player, Theme};
use featherplay::{memory, mini};
use slint::ComponentHandle;

/// Runs timers and the queued work for `ms`, as the event loop would.
fn run_for(scene: &Scene, ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        scene.settle_once();
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// The next timer that's due, if any.
fn next_timer() -> Option<Duration> {
    slint::platform::duration_until_next_timer_update()
}

#[test]
fn the_bars_clock_stops_while_nothing_shows_the_time() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.ui.global::<Theme>().set_motion(false);
    scene.ui.show().expect("show");
    scene.open("settings").expect("settings");
    scene.sound(true).expect("playing");
    scene.settle();
    run_for(&scene, 300);
    let player = scene.ui.global::<Player>();
    assert!(
        next_timer().is_some_and(|t| t <= Duration::from_secs(1)),
        "the bar's clock isn't ticking while sound plays: next timer {:?}",
        next_timer()
    );

    // Minimised, as the window's hook reports it: the next tick finds nothing to show the
    // time and doesn't arm another. Only the minute's memory log is left.
    scene.ui.window().set_minimized(true);
    memory::hidden(&scene.ctx, true);
    run_for(&scene, 1300);
    let at = player.get_position_s();
    assert!(
        next_timer().is_none_or(|t| t > Duration::from_secs(30)),
        "something still wakes while minimised: next timer {:?}",
        next_timer()
    );

    // The mini player shows the time: the clock ticks for it, and stops again once it closes.
    let popup = mini::create(&scene.ctx).expect("mini player");
    popup.show().expect("show");
    run_for(&scene, 300);
    let ticks = popup.global::<Player>().get_position_s();
    run_for(&scene, 1100);
    assert!(
        popup.global::<Player>().get_position_s() > ticks,
        "the mini player's time didn't move on"
    );
    mini::close();
    drop(popup);
    run_for(&scene, 1300);
    assert!(
        next_timer().is_none_or(|t| t > Duration::from_secs(30)),
        "the clock went on after the mini player closed: next timer {:?}",
        next_timer()
    );

    // Shown again: the time has moved on at once, and the clock ticks.
    scene.ui.window().set_minimized(false);
    memory::hidden(&scene.ctx, false);
    assert!(
        player.get_position_s() >= at + 2.0,
        "the time shown didn't catch up: {} then {}",
        at,
        player.get_position_s()
    );
    run_for(&scene, 300);
    assert!(
        next_timer().is_some_and(|t| t <= Duration::from_secs(1)),
        "the bar's clock didn't start again: next timer {:?}",
        next_timer()
    );
}
