//! The bar's mini player button, on the offscreen platform (no window is shown): it's there
//! only while the desktop has no tray (`SettingsData.has-tray`: always on Windows, on Linux
//! while a StatusNotifierWatcher shows the icon), and a click on it opens the mini player,
//! another closes it, as a click on the tray icon does; Ctrl+M too.

use featherplay::mini;
use featherplay::snapshot::Scene;
use featherplay::ui::{SettingsData, Theme};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};

const W: u32 = 1280;
const H: u32 = 800;

fn click(scene: &Scene, at: LogicalPosition) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    for pressed in [true, false] {
        let button = PointerEventButton::Left;
        w.dispatch_event(if pressed {
            WindowEvent::PointerPressed {
                position: at,
                button,
            }
        } else {
            WindowEvent::PointerReleased {
                position: at,
                button,
            }
        });
    }
    scene.settle_once();
}

/// The points along the bar's middle, from the window's middle to its right edge, where a
/// click opens the mini player (each closed again at once).
fn opening_spots(scene: &Scene) -> Vec<LogicalPosition> {
    let t = scene.ui.global::<Theme>();
    let y = H as f32 - t.get_bar_inset() - t.get_bar_h() / 2.0;
    let mut spots = Vec::new();
    for x in (W / 2..W).step_by(4) {
        let at = LogicalPosition::new(x as f32, y);
        click(scene, at);
        if mini::is_open() {
            spots.push(at);
            mini::close();
        }
    }
    spots
}

#[test]
fn the_mini_player_button_shows_only_without_a_tray_and_toggles_it() {
    let scene = Scene::new(W, H, true, false).expect("scene");
    scene.settle();
    let data = scene.ui.global::<SettingsData>();

    data.set_has_tray(true);
    scene.settle_once();
    assert!(
        opening_spots(&scene).is_empty(),
        "with a tray, something in the bar opened the mini player"
    );

    data.set_has_tray(false);
    scene.settle_once();
    let spots = opening_spots(&scene);
    assert!(!spots.is_empty(), "without a tray, no button opened it");
    let (first, last) = (spots[0].x, spots[spots.len() - 1].x);
    assert!(
        last - first < 36.0,
        "more than one 36 px button: {first}..{last}"
    );

    let button = spots[spots.len() / 2];
    click(&scene, button);
    assert!(mini::is_open(), "the click didn't open the mini player");
    click(&scene, button);
    assert!(!mini::is_open(), "a second click didn't close it");

    // Ctrl+M, as the button.
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::WindowActiveChanged(true));
    let ctrl_m = |w: &slint::Window| {
        for text in [SharedString::from(Key::Control), "m".into()] {
            w.dispatch_event(WindowEvent::KeyPressed { text });
        }
        for text in [SharedString::from("m"), Key::Control.into()] {
            w.dispatch_event(WindowEvent::KeyReleased { text });
        }
        scene.settle_once();
    };
    ctrl_m(w);
    assert!(mini::is_open(), "Ctrl+M didn't open the mini player");
    ctrl_m(w);
    assert!(!mini::is_open(), "Ctrl+M didn't close it");
}
