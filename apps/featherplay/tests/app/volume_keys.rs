//! Up and Down change the volume by the Volume step setting from anywhere in the window,
//! offscreen with fixture data: onto the step's grid (37 % and Up at 5 % is 40 %), within
//! 0–100 %, saved once the keys rest. A text field keeping them is in `transport_keys.rs`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::Player;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, SharedString};

type Calls = Rc<RefCell<Vec<(f32, bool)>>>;

fn tap(w: &slint::Window, text: SharedString) {
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

fn up(w: &slint::Window) {
    tap(w, Key::UpArrow.into());
}

fn down(w: &slint::Window) {
    tap(w, Key::DownArrow.into());
}

/// Records `set-volume` calls and applies them, as `bar.rs` does.
fn record(scene: &Scene) -> Calls {
    let calls: Calls = Rc::default();
    let seen = calls.clone();
    let weak = scene.ui.as_weak();
    scene
        .ui
        .global::<Player>()
        .on_set_volume(move |value, save| {
            seen.borrow_mut().push((value, save));
            if let Some(ui) = weak.upgrade() {
                ui.global::<Player>().set_volume(value.clamp(0.0, 1.0));
            }
        });
    calls
}

/// The window at 1280 × 800 with the fixture radio playing, keys in it, the clock frozen,
/// at the default 5 % step (not the dev settings' step).
fn scene() -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("home").expect("page");
    scene.settle();
    scene.ui.global::<Player>().set_volume_step(5);
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    offscreen::freeze_clock();
    scene
}

fn assert_volume(player: &Player, want: f32, what: &str) {
    let v = player.get_volume();
    assert!((v - want).abs() < 1e-4, "{what}: {v}, not {want}");
}

#[test]
fn up_and_down_step_the_volume_on_the_grid_and_save_once_they_rest() {
    let scene = scene();
    let w = scene.ui.window();
    let player = scene.ui.global::<Player>();
    player.set_volume(0.37);
    let calls = record(&scene);

    up(w);
    assert_volume(&player, 0.40, "37 % and Up");
    up(w);
    assert_volume(&player, 0.45, "40 % and Up");
    down(w);
    down(w);
    assert_volume(&player, 0.35, "45 % and Down twice");
    assert!(
        calls.borrow().iter().all(|&(_, save)| !save),
        "saved while the keys were pressed"
    );

    offscreen::advance(Duration::from_millis(1000));
    slint::platform::update_timers_and_animations();
    let saves: Vec<f32> = calls.borrow().iter().filter(|c| c.1).map(|c| c.0).collect();
    assert_eq!(saves.len(), 1, "saves after the keys rested: {saves:?}");
    assert!((saves[0] - 0.35).abs() < 1e-4);

    // Other steps, and the ends.
    player.set_volume_step(1);
    player.set_volume(0.372);
    down(w);
    assert_volume(&player, 0.37, "37.2 % and Down at 1 %");
    down(w);
    assert_volume(&player, 0.36, "37 % and Down at 1 %");
    player.set_volume_step(10);
    player.set_volume(0.97);
    up(w);
    assert_volume(&player, 1.0, "97 % and Up at 10 %");
    up(w);
    assert_volume(&player, 1.0, "Up at 100 %");
    player.set_volume(0.04);
    down(w);
    assert_volume(&player, 0.0, "4 % and Down at 10 %");
    down(w);
    assert_volume(&player, 0.0, "Down at 0 %");
}
