//! The player bar's tooltips, on the offscreen platform with fixture data (no window is
//! shown): every transport button has one now, Next's included, and Settings' "Show
//! tooltips" (Window) turns them all off and back on, saved with the other settings.

use featherplay::snapshot::Scene;
use featherplay::ui::{SettingKey, SettingsData, Theme, Tooltips};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, Rgba8Pixel};

/// The transport's buttons at 1280 wide: 36, 36, the 48 px play button, 36, 36, 12 apart,
/// centred on the bar.
const PLAY_X: f32 = 640.0;
const NEXT_X: f32 = 688.0;

/// The window at 1280 × 800 with the fixture radio playing. Still (as the snapshots are), a
/// tooltip shows at once on hover.
fn scene() -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.settle();
    scene
}

/// The pixels of a strip just above the bar, 40 px either side of `centre`, with the pointer
/// on the transport button at `hover` (or off the bar, `None`).
fn strip(scene: &Scene, hover: Option<f32>, centre: f32) -> Vec<Rgba8Pixel> {
    let w = scene.ui.window();
    let bar_top = 800.0 - scene.ui.global::<Theme>().get_bar_clear();
    let at = match hover {
        Some(x) => LogicalPosition::new(x, bar_top + 8.0 + 24.0),
        None => LogicalPosition::new(640.0, 400.0),
    };
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    scene.settle_once();
    let shot = w.take_snapshot().expect("snapshot");
    let y = (bar_top - 20.0) as u32;
    (centre as u32 - 40..centre as u32 + 40)
        .map(|x| shot.as_slice()[(y * shot.width() + x) as usize])
        .collect()
}

#[test]
fn next_shows_a_tooltip() {
    let scene = scene();
    let off = strip(&scene, None, NEXT_X);
    assert_ne!(
        strip(&scene, Some(NEXT_X), NEXT_X),
        off,
        "no tooltip over the hovered Next"
    );
}

#[test]
fn show_tooltips_off_hides_them_and_on_brings_them_back() {
    let scene = scene();
    scene.open("settings").expect("settings");
    scene.settle();
    let data = scene.ui.global::<SettingsData>();
    let without = strip(&scene, None, PLAY_X);

    data.invoke_toggled(SettingKey::Tooltips, false);
    assert!(!data.get_tooltips());
    assert!(!scene.ui.global::<Tooltips>().get_on());
    assert!(!featherplay::pages::settings::tooltips());
    assert_eq!(
        strip(&scene, Some(PLAY_X), PLAY_X),
        without,
        "a tooltip showed with the setting off"
    );

    strip(&scene, None, PLAY_X);
    data.invoke_toggled(SettingKey::Tooltips, true);
    assert!(featherplay::pages::settings::tooltips());
    assert_ne!(
        strip(&scene, Some(PLAY_X), PLAY_X),
        without,
        "no tooltip with the setting back on"
    );
}
