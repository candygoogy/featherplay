//! The row at the window's top (`ui/top_bar.slint`), on the offscreen platform: Back and Forward
//! work by their labels, and the keys reach Back, Forward and the search field in that order; on Windows
//! the window's own title bar is the default (Settings › Window › Title bar), with the window
//! buttons at the top right and the row's controls left out of the drag area, and Windows' own
//! title bar takes the window buttons away.

use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, Nav, PageKind, SettingKey, SettingsData};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};

use crate::support::label;

fn key(scene: &Scene, text: impl Into<SharedString>) {
    let w = scene.ui.window();
    let text = text.into();
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
    scene.settle();
}

/// Ctrl+K: the keys go to the search field.
fn search_keys(scene: &Scene) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(scene, "k");
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

#[test]
fn back_forward_and_search_in_the_row_work_by_pointer_and_keys() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    let page = || scene.ui.global::<Nav>().get_page();
    scene.open("home").expect("home");
    scene.settle();
    scene.open("settings").expect("settings");
    scene.settle();

    label::click(&scene, "Back");
    scene.settle();
    assert_eq!(page(), PageKind::Home, "Back");
    label::click(&scene, "Forward");
    scene.settle();
    assert_eq!(page(), PageKind::Settings, "Forward");
    // Forward stays where it is with nowhere to go, dimmed and not pressable.
    assert!(label::shows(&scene, "Forward"));
    label::click(&scene, "Forward");
    scene.settle();
    assert_eq!(page(), PageKind::Settings);

    // From the field, Shift+Tab goes back through Forward to Back: Tab's order is Back,
    // Forward, search.
    label::click(&scene, "Back");
    scene.settle();
    search_keys(&scene);
    key(&scene, Key::Backtab);
    key(&scene, Key::Return);
    assert_eq!(page(), PageKind::Settings, "Enter on Forward");
    // Forward, with nowhere to go now, is passed by.
    search_keys(&scene);
    key(&scene, Key::Backtab);
    key(&scene, " ");
    assert_eq!(page(), PageKind::Home, "Space on Back");
}

#[test]
fn the_own_title_bar_is_the_default_on_windows_and_windows_one_takes_its_buttons_away() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    let frame = scene.ui.global::<Frame>();
    let data = scene.ui.global::<SettingsData>();
    assert!(!data.get_system_title_bar());
    assert_eq!(frame.get_own_bar(), cfg!(windows));
    // The Window section's rows are made a tick each after the page's first frame.
    for _ in 0..8 {
        scene.ui.window().take_snapshot().expect("snapshot");
        std::thread::sleep(std::time::Duration::from_millis(2));
        scene.settle_once();
    }
    let row = !label::any_where(&scene, |_, l| l == "Title bar").is_empty();
    assert_eq!(row, cfg!(windows), "the Title bar row");
    if !cfg!(windows) {
        assert!(!label::shows(&scene, "Close"));
        return;
    }

    // The window buttons at the top right, the row's height, Windows' width each.
    let close = label::rects(&scene, "Close")
        .into_iter()
        .find(|r| r.min_y() == 0.0)
        .expect("a Close button at the top");
    assert_eq!(
        (close.max_x(), close.height(), close.width()),
        (1280.0, 48.0, 46.0)
    );
    assert!(label::shows(&scene, "Minimise") && label::shows(&scene, "Maximise"));
    // The hit test leaves Back, Forward and the field to the app, and nothing else in the row.
    let spans = in_row(&scene, &["Back", "Forward", "Search settings"]);
    assert_eq!(spans.len(), 3, "{spans:?}");

    // Windows' own title bar: no window buttons in the window; back again.
    data.invoke_toggled(SettingKey::SystemTitleBar, true);
    scene.settle();
    assert!(data.get_system_title_bar() && !frame.get_own_bar());
    assert!(!label::shows(&scene, "Minimise"));
    data.invoke_toggled(SettingKey::SystemTitleBar, false);
    scene.settle();
    assert!(frame.get_own_bar() && label::shows(&scene, "Minimise"));

    // On Now playing the page's header row is the window's top row: Back, "Playing from" and
    // the Lyrics pill are the app's, the rest drags.
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.settle();
    // The page tells the row where its controls are from its change handlers, which run by
    // the next frame.
    scene.ui.window().take_snapshot().expect("a frame");
    let spans = in_row(&scene, &["Back", "Lyrics"]);
    assert_eq!(spans.len(), 3, "{spans:?}");
}

/// The row's spans the hit test leaves to the app (`caption-hits`), checked to hold the shown
/// elements labelled `names` in the row.
fn in_row(scene: &Scene, names: &[&str]) -> Vec<(f32, f32)> {
    let hits = scene.ui.get_caption_hits();
    let spans: Vec<(f32, f32)> = (0..hits.row_count() / 2)
        .filter_map(|i| Some((hits.row_data(2 * i)?, hits.row_data(2 * i + 1)?)))
        .filter(|&(_, w)| w > 0.0)
        .collect();
    for name in names {
        let r = label::rects(scene, name)
            .into_iter()
            .find(|r| r.max_y() <= 48.0)
            .unwrap_or_else(|| panic!("{name} in the row"));
        assert!(
            spans
                .iter()
                .any(|&(x, w)| x <= r.min_x() + 0.5 && x + w + 0.5 >= r.max_x()),
            "{name} at {r:?} isn't among {spans:?}"
        );
    }
    spans
}
