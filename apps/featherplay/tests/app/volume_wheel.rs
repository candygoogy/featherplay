//! The bar's volume slider follows the mouse wheel, offscreen: a notch up or down moves the
//! volume 5 %, within 0–100 %, without saving; the setting is saved once the wheel rests.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::Player;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};

type Calls = Rc<RefCell<Vec<(f32, bool)>>>;

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

/// The slider, found by trying a notch along the bar's right side.
fn slider(scene: &Scene, calls: &Calls) -> LogicalPosition {
    let w = scene.ui.window();
    for y in (700..800).step_by(4) {
        for x in (800..1280).step_by(6) {
            let at = LogicalPosition::new(x as f32, y as f32);
            w.dispatch_event(WindowEvent::PointerMoved { position: at });
            offscreen::wheel(w, at, 1.0);
            if !calls.borrow().is_empty() {
                return at;
            }
        }
    }
    panic!("no volume slider took the wheel");
}

#[test]
fn a_notch_moves_the_volume_5_percent_and_saves_once_it_rests() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let player = scene.ui.global::<Player>();
    player.set_volume(0.5);
    let calls = record(&scene);
    let at = slider(&scene, &calls);
    let w = scene.ui.window();

    assert_eq!(calls.borrow().as_slice(), &[(0.55, false)]);
    offscreen::wheel(w, at, -1.0);
    offscreen::wheel(w, at, -1.0);
    let v = player.get_volume();
    assert!((v - 0.45).abs() < 1e-4, "two notches down: {v}");
    assert!(
        calls.borrow().iter().all(|&(_, save)| !save),
        "saved mid-wheel"
    );

    offscreen::advance(Duration::from_millis(1000));
    slint::platform::update_timers_and_animations();
    let saves: Vec<f32> = calls.borrow().iter().filter(|c| c.1).map(|c| c.0).collect();
    assert_eq!(saves.len(), 1, "saves after the wheel rested: {saves:?}");
    assert!((saves[0] - 0.45).abs() < 1e-4);

    player.set_volume(0.98);
    offscreen::wheel(w, at, 1.0);
    assert_eq!(player.get_volume(), 1.0, "a notch past full stops at 100 %");
}

#[test]
fn a_notch_moves_the_volume_by_the_volume_step() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.ui.show().expect("show");
    scene.settle();
    let player = scene.ui.global::<Player>();
    player.set_volume_step(2);
    player.set_volume(0.5);
    let calls = record(&scene);
    let at = slider(&scene, &calls);
    let v = player.get_volume();
    assert!((v - 0.52).abs() < 1e-4, "a notch up at 2 %: {v}");

    player.set_volume_step(10);
    offscreen::wheel(scene.ui.window(), at, -1.0);
    let v = player.get_volume();
    assert!((v - 0.42).abs() < 1e-4, "a notch down at 10 %: {v}");
}
