//! Settings' search, on the offscreen platform with fixture data (no window is shown): Ctrl+F
//! on Settings puts the keys in the field in the page's header; typing hides the rows not
//! found and the sections left without any, the index listing the others with their counts;
//! nothing found shows the empty state; the first Escape clears the field, the next leaves
//! it. In the top bar, the settings that match follow the music's suggestions unless "Show
//! settings in search suggestions" is off, and picking one opens Settings at its row, lit,
//! its switch taking the keys when picked with the keyboard.

use std::ops::ControlFlow;
use std::time::Duration;

use featherplay::offscreen;
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{
    SettingId, SettingKey, SettingsData, SettingsSearch, SuggestionKind, TopBarData,
};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::window::WindowInner;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, Rgba8Pixel, SharedPixelBuffer, SharedString};

use crate::support::pixels;

/// At 1920 wide the index sits left of the column, 40 px into the page past the sidebar.
const INDEX_LEFT: usize = 240 + 40;
const INDEX_RIGHT: usize = INDEX_LEFT + 184;

fn tap(w: &slint::Window, text: SharedString) {
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

fn ctrl(scene: &Scene, key: &str) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    tap(w, key.into());
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    scene.settle();
}

fn type_text(scene: &Scene, text: &str) {
    for c in text.chars() {
        tap(scene.ui.window(), c.to_string().into());
    }
    scene.settle();
}

/// A few drawn frames, the clock moving on between them: the page makes its sections below
/// the fold one per tick after its first frame.
fn frames(scene: &Scene) {
    for _ in 0..5 {
        scene.ui.window().take_snapshot().expect("snapshot");
        std::thread::sleep(Duration::from_millis(2));
        scene.settle_once();
    }
}

/// The window at this size, at 100 %.
fn resize(scene: &Scene, width: u32, height: u32) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    w.set_size(slint::PhysicalSize::new(width, height));
    w.dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
}

/// Settings at 1920 × 1080 with every section made, the keys in the window.
fn settings() -> Scene {
    let scene = Scene::new(1920, 1080, true, false).expect("scene");
    resize(&scene, 1920, 1080);
    scene.play(false).expect("player");
    scene.open("settings").expect("settings");
    scene.settle();
    frames(&scene);
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    scene
}

/// The first visible element whose accessible label is `label`, and its top in the window.
fn find(scene: &Scene, label: &str) -> Option<f32> {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    root.visit_descendants(|item| {
        let says = item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|l| l == label);
        if says && item.is_visible() {
            ControlFlow::Break(item.map_to_window(item.geometry().origin).y)
        } else {
            ControlFlow::Continue(())
        }
    })
}

fn counts(scene: &Scene) -> Vec<i32> {
    scene
        .ui
        .global::<SettingsSearch>()
        .get_counts()
        .iter()
        .collect()
}

/// The section index's entries: runs of rows of the index column that hold anything but
/// the ground.
fn index_entries(s: &SharedPixelBuffer<Rgba8Pixel>) -> usize {
    let px = |x: usize, y: usize| s.as_slice()[y * s.width() as usize + x];
    let ground = px(INDEX_RIGHT + 8, 150);
    let busy: Vec<usize> = (150..900)
        .filter(|&y| (INDEX_LEFT..INDEX_RIGHT).any(|x| px(x, y) != ground))
        .collect();
    busy.windows(2).filter(|w| w[1] != w[0] + 1).count() + usize::from(!busy.is_empty())
}

#[test]
fn typing_filters_the_rows_and_the_index_counts_them() {
    let scene = settings();
    let search = scene.ui.global::<SettingsSearch>();
    if pixels::checked() {
        assert_eq!(
            index_entries(&scene.ui.window().take_snapshot().unwrap()),
            8
        );
    }

    // Ctrl+F puts the keys in the field: what's typed filters the page.
    ctrl(&scene, "f");
    type_text(&scene, "volume");
    assert_eq!(search.get_query(), "volume");
    assert!(search.get_active());
    // Volume step in Playback; Even out loudness (its line) and Match song volumes in Sound.
    assert_eq!(counts(&scene), [1, 2, 0, 0, 0, 0, 0, 0]);
    assert_eq!(search.get_total(), 3);
    assert!(find(&scene, "Volume step").is_some());
    assert!(find(&scene, "Match song volumes").is_some());
    assert!(find(&scene, "Crossfade").is_none(), "a row not found shows");
    assert!(
        find(&scene, "Appearance").is_none(),
        "a section with none found shows"
    );
    // The words found are marked: "Volume" of the title, and the typed part of "volumes".
    let hit = search.invoke_hit(SettingId::VolumeStep);
    let marks: Vec<_> = hit.title_marks.iter().map(|m| (m.start, m.end)).collect();
    assert_eq!(marks, [(0, 6)]);
    let hit = search.invoke_hit(SettingId::MatchVolumes);
    let marks: Vec<_> = hit.title_marks.iter().map(|m| (m.start, m.end)).collect();
    assert_eq!(marks, [(11, 17)]);
    // The index lists the two sections.
    let shot = scene.ui.window().take_snapshot().unwrap();
    pixels::save("volume", &shot);
    if pixels::checked() {
        assert_eq!(index_entries(&shot), 2);
    }

    // The first Escape clears the field: the whole page again.
    tap(scene.ui.window(), Key::Escape.into());
    scene.settle();
    assert_eq!(search.get_query(), "");
    assert!(!search.get_active());
    assert!(find(&scene, "Crossfade").is_some());
    // The keys stay in the field until the next Escape, which leaves it.
    type_text(&scene, "dark");
    assert_eq!(counts(&scene), [0, 0, 0, 0, 0, 0, 1, 0]);
    tap(scene.ui.window(), Key::Escape.into());
    tap(scene.ui.window(), Key::Escape.into());
    scene.settle();
    type_text(&scene, "xy");
    assert_eq!(search.get_query(), "", "the field kept the keys");
}

#[test]
fn nothing_found_shows_the_empty_state() {
    let scene = settings();
    ctrl(&scene, "f");
    type_text(&scene, "weather");
    let search = scene.ui.global::<SettingsSearch>();
    assert!(search.get_active());
    assert_eq!(search.get_total(), 0);
    assert!(find(&scene, "No settings match “weather”").is_some());
    assert!(find(&scene, "Prefer song versions").is_none());
    // One letter filters nothing.
    tap(scene.ui.window(), Key::Escape.into());
    type_text(&scene, "s");
    assert!(!search.get_active());
    assert!(find(&scene, "Prefer song versions").is_some());
}

#[test]
fn sleep_timer_finds_its_switch_and_once_on_its_fade() {
    let scene = settings();
    ctrl(&scene, "f");
    type_text(&scene, "sleep timer");
    let search = scene.ui.global::<SettingsSearch>();
    assert_eq!(search.get_total(), 1);
    assert_eq!(counts(&scene), [1, 0, 0, 0, 0, 0, 0, 0]);
    // Turned on, the Fade out row shows, found by its line.
    scene
        .ui
        .global::<SettingsData>()
        .invoke_toggled(SettingKey::SleepTimer, true);
    scene.settle();
    assert_eq!(search.get_total(), 2);
}

#[test]
fn ctrl_f_brings_the_field_back_at_the_page_top() {
    let scene = settings();
    let top = find(&scene, "Search settings").expect("the field");
    // Scrolled down, the field goes with the title.
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(1800.0, 500.0),
            delta_x: 0.0,
            delta_y: -1500.0,
        });
    scene.settle();
    assert!(find(&scene, "Search settings").is_none_or(|y| y < top - 100.0));
    ctrl(&scene, "f");
    assert_eq!(find(&scene, "Search settings"), Some(top));
    type_text(&scene, "tray");
    assert_eq!(scene.ui.global::<SettingsSearch>().get_query(), "tray");
    // Leaving Settings and coming back, the search is gone.
    scene.open("home").expect("home");
    scene.settle();
    scene.open("settings").expect("settings");
    scene.settle();
    assert_eq!(scene.ui.global::<SettingsSearch>().get_query(), "");
    assert!(!scene.ui.global::<SettingsSearch>().get_active());
}

/// Home with `text` typed in the top bar, its suggestions in.
fn suggest(scene: &Scene, text: &str) {
    ctrl(scene, "k");
    for c in text.chars() {
        tap(scene.ui.window(), c.to_string().into());
        offscreen::advance(Duration::from_millis(50));
        scene.run_queued();
    }
    offscreen::advance(Duration::from_millis(400));
    scene.settle();
}

fn settings_rows(scene: &Scene) -> Vec<String> {
    let field = scene.ui.global::<TopBarData>();
    field
        .get_suggestions()
        .iter()
        .filter(|s| s.kind == SuggestionKind::Setting)
        .map(|s| format!("{} | {}", s.title, s.subtitle))
        .collect()
}

#[test]
fn a_suggestion_opens_settings_at_its_row() {
    let scene = Scene::new(1568, 980, true, false).expect("scene");
    resize(&scene, 1568, 980);
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.settle();
    offscreen::freeze_clock();
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let field = scene.ui.global::<TopBarData>();

    // After the music, the settings found by name or keyword ("ReplayGain" isn't typed).
    suggest(&scene, "match song");
    assert_eq!(
        settings_rows(&scene),
        ["Match song volumes | Setting • Sound"]
    );
    let from = field.get_settings_from();
    assert!(from > 0, "the settings don't follow the music");
    for _ in 0..=from {
        tap(scene.ui.window(), Key::DownArrow.into());
    }
    assert_eq!(field.get_highlighted(), from);
    tap(scene.ui.window(), Key::Return.into());
    scene.settle();

    // Settings, the top bar emptied, the row lit and on screen.
    assert_eq!(scene.ctx.current_page(), Page::Settings);
    assert_eq!(field.get_query(), "");
    let search = scene.ui.global::<SettingsSearch>();
    assert_eq!(search.get_lit(), SettingId::MatchVolumes);
    assert!(search.get_lit_on());
    // The clock is frozen: each step moves it on for the page's ticks (its sections made,
    // then the row asked where it is, then the scroll).
    for _ in 0..8 {
        offscreen::advance(Duration::from_millis(16));
        frames(&scene);
    }
    let y = find(&scene, "Match song volumes").expect("the row");
    // On the reading line, a third of the way down the page above the bar.
    let reading = 64.0 + (980.0 - 64.0 - 98.0) / 3.0;
    assert!(
        (y - reading).abs() < 40.0,
        "the row is at {y}, the reading line at {reading}"
    );
    // Picked with the keyboard, its switch has the keys: Enter flips it.
    let data = scene.ui.global::<SettingsData>();
    assert!(data.get_match_volumes());
    tap(scene.ui.window(), Key::Return.into());
    assert!(
        !data.get_match_volumes(),
        "the row's switch didn't get the keys"
    );
    // The light goes after 1.2 s.
    offscreen::advance(Duration::from_millis(1300));
    scene.settle();
    assert!(!search.get_lit_on());
}

#[test]
fn the_switch_keeps_settings_out_of_the_suggestions() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    offscreen::freeze_clock();
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    // Two letters: no settings yet; three: Crossfade (Fade on skip shows only while
    // Crossfade is on).
    suggest(&scene, "cr");
    assert!(settings_rows(&scene).is_empty());
    tap(scene.ui.window(), Key::Escape.into());
    tap(scene.ui.window(), Key::Escape.into());
    suggest(&scene, "crossf");
    assert_eq!(settings_rows(&scene), ["Crossfade | Setting • Playback"]);
    tap(scene.ui.window(), Key::Escape.into());
    tap(scene.ui.window(), Key::Escape.into());

    let data = scene.ui.global::<SettingsData>();
    data.invoke_toggled(SettingKey::SettingsInSuggestions, false);
    assert!(!data.get_settings_in_suggestions());
    suggest(&scene, "crossf");
    assert!(settings_rows(&scene).is_empty());
    assert_eq!(scene.ui.global::<TopBarData>().get_settings_from(), -1);
    // The music's suggestions still show.
    assert!(
        scene
            .ui
            .global::<TopBarData>()
            .get_suggestions()
            .row_count()
            > 0
    );
}
