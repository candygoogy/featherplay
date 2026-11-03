//! Space plays and pauses from anywhere in the window, as in YouTube Music's web player: the
//! bar's play button's path (`Player.toggle`, `Action::Toggle`), on the offscreen platform
//! with fixture data (no window is shown). Not while a text field has the keys (Space types
//! there); a held Space flips playback once; with nothing loaded it does nothing. A focused
//! Settings switch leaves Space to playback and still flips on Enter. The tray's mini player
//! takes Space the same way.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use featherplay::controller::Action;
use featherplay::mini;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, LyricsData, Player, SettingKey, SettingsData, Theme, TopBarData};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};

fn tap(w: &slint::Window, text: SharedString) {
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

fn space(w: &slint::Window) {
    tap(w, " ".into());
}

/// Space held down: the press, then the keyboard's auto-repeats, then the release.
fn held_space(w: &slint::Window, repeats: usize) {
    w.dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    for _ in 0..repeats {
        w.dispatch_event(WindowEvent::KeyPressRepeated { text: " ".into() });
    }
    w.dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
}

fn ctrl_k(w: &slint::Window) {
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    tap(w, "k".into());
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

fn click(w: &slint::Window, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

/// How many toggles the window asked the controller for since the last call.
fn toggles(scene: &Scene) -> usize {
    scene
        .take_actions()
        .iter()
        .filter(|a| matches!(a, Action::Toggle))
        .count()
}

/// The window at 1280 × 800 with the fixture radio playing (or nothing, `idle`), keys in it.
fn scene(idle: bool, page: &str) -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(idle).expect("player");
    scene.open(page).expect("page");
    scene.settle();
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    scene.take_actions();
    scene
}

#[test]
fn space_toggles_playing_then_paused_then_playing() {
    let scene = scene(false, "home");
    let w = scene.ui.window();
    let player = scene.ui.global::<Player>();
    assert!(player.get_playing());

    // Each Space asks for the toggle; the controller's answer flips the bar.
    space(w);
    assert_eq!(toggles(&scene), 1, "Space didn't pause");
    scene.sound(false).expect("paused");
    assert!(!player.get_playing());
    space(w);
    assert_eq!(toggles(&scene), 1, "Space didn't play");
    scene.sound(true).expect("playing");
    assert!(player.get_playing());

    // The same on every page, on Now playing, its full lyrics view and with the Up next
    // panel open.
    let on = |what: &str| {
        scene.settle();
        scene.take_actions();
        space(w);
        assert_eq!(toggles(&scene), 1, "Space on {what}");
    };
    for page in [
        "library",
        "album",
        "playlist",
        "artist",
        "search",
        "about",
        "explore",
        "mood",
        "history",
        "liked",
        "settings",
        "now-playing",
    ] {
        scene.open(page).expect(page);
        on(page);
    }
    scene.ui.global::<LyricsData>().set_full(true);
    on("the full lyrics view");
    scene.ui.global::<LyricsData>().set_full(false);
    scene.open("home").expect("home");
    scene.ui.global::<Frame>().set_queue_open(true);
    on("Up next");

    // Clicking the bar's play button leaves the keys where they were: Space after it still
    // plays and pauses, once each. The button sits in the middle of the bar, its transport
    // row 8 px down from the bar's top.
    scene.take_actions();
    let bar_top = 800.0 - scene.ui.global::<Theme>().get_bar_clear();
    click(w, 640.0, bar_top + 8.0 + 22.0);
    assert_eq!(toggles(&scene), 1, "the click missed the play button");
    space(w);
    assert_eq!(toggles(&scene), 1, "Space after a click on play");
}

#[test]
fn space_in_the_search_field_types_and_leaves_playback() {
    let scene = scene(false, "home");
    let w = scene.ui.window();
    ctrl_k(w);
    for c in ["a", " ", "b"] {
        tap(w, c.into());
    }
    assert_eq!(scene.ui.global::<TopBarData>().get_query(), "a b");
    assert_eq!(toggles(&scene), 0, "Space in the field reached playback");

    // Out of the field (Escape clears, then leaves), Space plays and pauses again.
    tap(w, Key::Escape.into());
    tap(w, Key::Escape.into());
    space(w);
    assert_eq!(toggles(&scene), 1);
}

#[test]
fn held_space_toggles_once() {
    let scene = scene(false, "home");
    held_space(scene.ui.window(), 5);
    assert_eq!(toggles(&scene), 1, "auto-repeats flipped playback again");
}

#[test]
fn space_with_nothing_loaded_does_nothing() {
    let scene = scene(true, "home");
    assert!(!scene.ui.global::<Player>().get_has_track());
    space(scene.ui.window());
    held_space(scene.ui.window(), 3);
    assert_eq!(toggles(&scene), 0);
    assert!(!scene.ui.global::<Player>().get_playing());
}

#[test]
fn a_clicked_settings_switch_leaves_space_to_playback_and_flips_on_enter() {
    let scene = scene(false, "settings");
    let w = scene.ui.window();
    let flipped = Rc::new(RefCell::new(Vec::<(SettingKey, bool)>::new()));
    let seen = flipped.clone();
    scene
        .ui
        .global::<SettingsData>()
        .on_toggled(move |key, on| seen.borrow_mut().push((key, on)));

    // The first switch on the page: Playback's first row.
    let (x, y) = SWITCH;
    click(w, x, y);
    assert_eq!(flipped.borrow().len(), 1, "the click missed the switch");

    // Clicking it gave it the keys (its focus ring shows). Space doesn't click it again: it
    // plays and pauses, as on YouTube Music.
    space(w);
    assert_eq!(flipped.borrow().len(), 1, "Space flipped the switch again");
    assert_eq!(toggles(&scene), 1, "Space on a focused switch didn't pause");

    // Enter still works the focused switch.
    tap(w, Key::Return.into());
    assert_eq!(
        flipped.borrow().len(),
        2,
        "Enter didn't flip the focused switch"
    );
    assert_eq!(toggles(&scene), 0);
}

/// Where Settings' first switch sits at 1280 × 800 (from a snapshot of the page).
const SWITCH: (f32, f32) = (1155.0, 271.0);

#[test]
fn the_mini_player_takes_space_once_per_press() {
    let scene = scene(false, "home");
    let mini = mini::create(&scene.ctx).expect("mini player");
    mini.show().expect("show");
    let mw = mini.window();
    mw.dispatch_event(WindowEvent::WindowActiveChanged(true));
    scene.settle();

    let toggled = Rc::new(Cell::new(0));
    let count = toggled.clone();
    scene
        .ui
        .global::<Player>()
        .on_toggle(move || count.set(count.get() + 1));
    space(mw);
    assert_eq!(toggled.get(), 1);
    held_space(mw, 5);
    assert_eq!(toggled.get(), 2, "auto-repeats flipped playback again");
}

#[test]
fn the_play_buttons_tooltip_shows_its_shortcut() {
    // Still: the tooltip shows at once on hover, as in the snapshots.
    let scene = scene(false, "home");
    let w = scene.ui.window();
    let bar_top = 800.0 - scene.ui.global::<Theme>().get_bar_clear();
    // A strip just above the bar, over the play button: where the tooltip sits.
    let strip = |shot: &slint::SharedPixelBuffer<slint::Rgba8Pixel>| {
        let y = (bar_top - 20.0) as u32;
        (600..680)
            .map(|x| shot.as_slice()[(y * shot.width() + x) as usize])
            .collect::<Vec<_>>()
    };
    w.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(640.0, 400.0),
    });
    scene.settle_once();
    let before = strip(&w.take_snapshot().expect("snapshot"));

    w.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(640.0, bar_top + 8.0 + 24.0),
    });
    scene.settle_once();
    let shown = strip(&w.take_snapshot().expect("snapshot"));
    assert_ne!(shown, before, "no tooltip over the hovered play button");
}
