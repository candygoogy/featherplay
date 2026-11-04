//! The tray's mini player on the offscreen platform (no window is shown): it mirrors the main
//! window's track and theme, its play button goes through the main window's player, Escape
//! and a lost activation close it, and while it shows nothing in it animates or keeps a
//! timer. Pinned, it stays open without the activation, saves the pin, closes on its ×,
//! and a press off its controls drags it. A click or a drag on its progress line seeks, and
//! never drags the window. Prints what creating it costs.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay::controller::{AccountAction, Action, Change};
use featherplay::mini;
use featherplay::pages::settings;
use featherplay::snapshot::Scene;
use featherplay::ui::{MiniData, MiniPlayer, Player, Theme, Toast};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Rgba8Pixel, SharedPixelBuffer};

fn rgba(c: slint::Color) -> Rgba8Pixel {
    Rgba8Pixel::new(c.red(), c.green(), c.blue(), c.alpha())
}

fn pixel(shot: &SharedPixelBuffer<Rgba8Pixel>, x: u32, y: u32) -> Rgba8Pixel {
    shot.as_slice()[(y * shot.width() + x) as usize]
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

fn open(scene: &Scene) -> MiniPlayer {
    let mini = mini::create(&scene.ctx).expect("mini player");
    mini.show().expect("show");
    mini.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    scene.settle();
    mini
}

/// The pin, just left of the top-right corner (32 px buttons, 8 px from the edge).
const PIN: (f32, f32) = (284.0, 21.0);
/// The corner: "Open featherplay" unpinned, the × pinned.
const CORNER: (f32, f32) = (316.0, 21.0);

/// The pins the window saved since the last call, oldest first.
fn pins_saved(scene: &Scene) -> Vec<bool> {
    scene
        .take_actions()
        .into_iter()
        .filter_map(|a| match a {
            Action::Account(AccountAction::Set(Change::MiniPinned(on))) => Some(on),
            _ => None,
        })
        .collect()
}

fn lose_activation(mini: &MiniPlayer, scene: &Scene) {
    mini.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(false));
    scene.settle();
}

fn playing_scene() -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.settle();
    scene.take_actions();
    scene
}

/// The progress line: 316 px from x = 12, its middle 90 px down.
const LINE: (f32, f32, f32) = (12.0, 316.0, 90.0);

/// The seeks the window asked for since the last call, oldest first.
fn seeks(scene: &Scene) -> Vec<Duration> {
    scene
        .take_actions()
        .into_iter()
        .filter_map(|a| match a {
            Action::SeekTo(at) => Some(at),
            _ => None,
        })
        .collect()
}

fn assert_near(at: Duration, fraction: f32, duration: f32) {
    let want = fraction * duration;
    assert!(
        (at.as_secs_f32() - want).abs() <= duration * 0.01,
        "sought {at:?}, not {want} s"
    );
}

#[test]
fn a_click_on_the_mini_players_line_seeks_there() {
    let scene = playing_scene();
    let mini = open(&scene);
    let duration = mini.global::<Player>().get_duration_s();
    assert!(duration > 0.0);

    click(mini.window(), LINE.0 + LINE.1 * 0.25, LINE.2);
    scene.settle();
    let sought = seeks(&scene);
    assert_eq!(sought.len(), 1, "{sought:?}");
    assert_near(sought[0], 0.25, duration);
    // The mini player shows the new place at once, as the bar does.
    let own = mini.global::<Player>().get_position_s();
    assert!(
        (own - 0.25 * duration).abs() <= duration * 0.01,
        "at {own} s"
    );
}

#[test]
fn pinned_a_drag_on_the_line_seeks_on_release_and_does_not_move_the_window() {
    let scene = playing_scene();
    let mini = open(&scene);
    let duration = mini.global::<Player>().get_duration_s();
    click(mini.window(), PIN.0, PIN.1);
    scene.settle();
    scene.take_actions();
    let drags = Rc::new(Cell::new(0));
    let count = drags.clone();
    mini.global::<MiniData>()
        .on_drag(move || count.set(count.get() + 1));

    let w = mini.window();
    let from = LogicalPosition::new(LINE.0 + LINE.1 * 0.3, LINE.2);
    let to = LogicalPosition::new(LINE.0 + LINE.1 * 0.6, LINE.2 + 3.0);
    w.dispatch_event(WindowEvent::PointerMoved { position: from });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerMoved { position: to });
    scene.settle();
    assert!(seeks(&scene).is_empty(), "it sought before the release");
    w.dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button: PointerEventButton::Left,
    });
    scene.settle();
    let sought = seeks(&scene);
    assert_eq!(sought.len(), 1, "{sought:?}");
    assert_near(sought[0], 0.6, duration);
    assert_eq!(drags.get(), 0, "a drag on the line moved the window");
}

#[test]
fn pinned_mini_player_stays_open_without_the_activation_and_closes_on_its_cross() {
    let scene = playing_scene();
    let mini = open(&scene);
    assert!(!mini.global::<MiniData>().get_pinned());

    click(mini.window(), PIN.0, PIN.1);
    scene.settle();
    assert!(
        mini.global::<MiniData>().get_pinned(),
        "the pin didn't pin it"
    );
    assert_eq!(pins_saved(&scene), [true]);
    assert!(settings::mini_pinned());

    lose_activation(&mini, &scene);
    assert!(mini::is_open(), "pinned, losing the activation closed it");

    // The tray menu and showing the main window leave a pinned one open too.
    mini::close_unpinned();
    assert!(mini::is_open(), "pinned, the tray menu closed it");

    // The × in the corner closes it, and it stays pinned for next time.
    click(mini.window(), CORNER.0, CORNER.1);
    scene.settle();
    assert!(!mini::is_open(), "the × didn't close it");
    assert!(settings::mini_pinned());
    assert!(pins_saved(&scene).is_empty());
}

#[test]
fn the_pin_is_remembered_and_unpinned_the_mini_player_closes_without_the_activation() {
    let scene = playing_scene();
    let mini = open(&scene);
    click(mini.window(), PIN.0, PIN.1);
    scene.settle();
    mini.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    scene.settle();
    assert!(!mini::is_open(), "Escape didn't close it");
    drop(mini);

    // The next one opens pinned.
    let mini = open(&scene);
    assert!(
        mini.global::<MiniData>().get_pinned(),
        "the pin wasn't kept"
    );
    scene.take_actions();

    // Unpinned again, it closes as a flyout does.
    click(mini.window(), PIN.0, PIN.1);
    scene.settle();
    assert!(!mini.global::<MiniData>().get_pinned());
    assert_eq!(pins_saved(&scene), [false]);
    assert!(!settings::mini_pinned());
    lose_activation(&mini, &scene);
    assert!(
        !mini::is_open(),
        "unpinned, losing the activation didn't close it"
    );
}

#[test]
fn pinned_a_press_off_the_controls_drags_the_mini_player() {
    let scene = playing_scene();
    let mini = open(&scene);
    let drags = Rc::new(Cell::new(0));
    let count = drags.clone();
    mini.global::<MiniData>()
        .on_drag(move || count.set(count.get() + 1));

    // Unpinned, nothing drags.
    click(mini.window(), 44.0, 44.0);
    assert_eq!(drags.get(), 0, "unpinned, a press on the cover dragged it");

    click(mini.window(), PIN.0, PIN.1);
    scene.settle();
    assert_eq!(drags.get(), 0, "the pin's own click dragged it");
    // The cover, and the ground beside the progress line's times.
    click(mini.window(), 44.0, 44.0);
    click(mini.window(), 170.0, 104.0);
    assert_eq!(drags.get(), 2, "a press off the controls didn't drag it");
    // A control is still a control.
    click(mini.window(), 170.0, 136.0);
    assert_eq!(drags.get(), 2, "the play button dragged it");
}

/// One test: the platform can be installed once per process.
#[test]
fn mini_player_mirrors_the_player_and_closes_on_escape_or_lost_activation() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.settle();

    let started = Instant::now();
    let mini = mini::create(&scene.ctx).expect("mini player");
    let created = started.elapsed();
    mini.show().expect("show");
    let started = Instant::now();
    mini.window().take_snapshot().expect("first frame");
    eprintln!(
        "mini player: created in {:.1} ms, first frame in {:.1} ms ({} build)",
        created.as_secs_f64() * 1000.0,
        started.elapsed().as_secs_f64() * 1000.0,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    mini.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    scene.settle();
    assert!(mini::is_open());

    // The main window's track and state.
    let (main, own) = (scene.ui.global::<Player>(), mini.global::<Player>());
    assert!(own.get_has_track());
    assert_eq!(own.get_title(), main.get_title());
    assert_eq!(own.get_artist(), main.get_artist());
    assert_eq!(own.get_playing(), main.get_playing());
    assert_eq!(own.get_duration_s(), main.get_duration_s());
    assert!(own.get_position_s() > 0.0);

    // Its own theme, the main window's: the ground is the bar's colour, the progress line
    // the accent up to the time played. The line runs across the 12 px padding, 316 px,
    // its middle 90 px down.
    let theme = mini.global::<Theme>();
    assert!(theme.get_dark());
    let shot = mini.window().take_snapshot().expect("snapshot");
    assert_eq!((shot.width(), shot.height()), (340, 170));
    assert_eq!(pixel(&shot, 6, 140), rgba(theme.get_bar()));
    let accent = rgba(theme.get_accent());
    let played = (12..328).filter(|&x| pixel(&shot, x, 90) == accent).count() as f32;
    let expected = 316.0 * own.get_position_s() / own.get_duration_s();
    assert!((played - expected).abs() <= 2.0, "{played} of {expected}");
    // The times either side of it.
    assert!(!own.get_elapsed().is_empty());
    assert_eq!(own.get_total(), main.get_total());

    // Nothing animates or waits on a timer while it rests.
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        mini.window().take_snapshot().expect("snapshot");
    }
    assert!(!mini.window().has_active_animations());

    // Play/pause goes to the main window's player, as the bar's button does.
    let toggled = Rc::new(Cell::new(0));
    let count = toggled.clone();
    main.on_toggle(move || count.set(count.get() + 1));
    // The play button: in the middle of the window, on the controls' row (114 px down, 44
    // px tall).
    click(mini.window(), 170.0, 136.0);
    assert_eq!(toggled.get(), 1);

    // "Not for me", the row's right end, goes the tray's way (the window is on screen: the
    // toast with Undo).
    click(mini.window(), 256.0 + 18.0, 136.0);
    scene.settle();
    assert!(
        scene.ui.global::<Toast>().get_open(),
        "not for me didn't reach the player"
    );

    // Escape closes it.
    mini.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    scene.settle();
    assert!(!mini::is_open(), "Escape didn't close it");
    drop(mini);

    // So does another window taking the activation.
    let mini = open(&scene);
    assert!(mini::is_open());
    mini.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(false));
    scene.settle();
    assert!(!mini::is_open(), "losing the activation didn't close it");
    drop(mini);

    // Closed, nothing of it is left to wake the loop (the toast's timer aside, which ends).
    std::thread::sleep(Duration::from_millis(5));
    slint::platform::update_timers_and_animations();
    let toast = scene.ui.global::<Toast>();
    for _ in 0..80 {
        if !toast.get_open() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        slint::platform::update_timers_and_animations();
        scene.settle();
    }
    assert!(!toast.get_open(), "the toast never closed");
    assert!(
        slint::platform::duration_until_next_timer_update().is_none(),
        "a timer is still armed"
    );
}
