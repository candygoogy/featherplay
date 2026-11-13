//! The bar's mini player button, on the offscreen platform (no window is shown): it's there
//! only while the desktop has no tray (`SettingsData.has-tray`: always on Windows, on Linux
//! while a StatusNotifierWatcher shows the icon), and a click on it opens the mini player,
//! another closes it, as a click on the tray icon does; Ctrl+M too.

use featherplay::controller::Action;
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

/// At the narrowest window (960 px), the mini player button's extra room mustn't run the
/// right-hand group into the transport: Repeat and the button stay apart.
#[test]
fn at_the_narrowest_window_the_mini_player_button_clears_the_transport() {
    const NARROW: u32 = 960;
    let scene = Scene::new(NARROW, H, true, false).expect("scene");
    scene.play(false).expect("player");
    let size = slint::LogicalSize::new(NARROW as f32, H as f32);
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::Resized { size });
    scene.settle();
    scene.ui.global::<SettingsData>().set_has_tray(false);
    scene.settle_once();
    let t = scene.ui.global::<Theme>();
    // The transport's row (Repeat) and the right-hand group's (the mini player button).
    let bar_top = H as f32 - t.get_bar_inset() - t.get_bar_h();
    let transport_y = bar_top + 8.0 + t.get_play_size() / 2.0;
    let actions_y = bar_top + t.get_bar_h() / 2.0;
    let mut mini_start = f32::MAX;
    for x in (NARROW / 3..NARROW * 3 / 4).step_by(2) {
        click(&scene, LogicalPosition::new(x as f32, actions_y));
        if mini::is_open() {
            mini_start = mini_start.min(x as f32);
            mini::close();
        }
    }
    scene.take_actions();
    let mut repeat_end = 0.0f32;
    for x in (NARROW / 3..NARROW * 3 / 4).step_by(2) {
        click(&scene, LogicalPosition::new(x as f32, transport_y));
        if scene
            .take_actions()
            .iter()
            .any(|a| matches!(a, Action::SetRepeat(_)))
        {
            repeat_end = x as f32;
        }
    }
    assert!(repeat_end > 0.0, "no Repeat found in the transport");
    assert!(mini_start < f32::MAX, "no mini player button found");
    assert!(
        mini_start - repeat_end >= 8.0,
        "Repeat ends at {repeat_end}, the mini player button starts at {mini_start}"
    );
}
