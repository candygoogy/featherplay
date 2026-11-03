//! The tray's mini player on the offscreen platform (no window is shown): it mirrors the main
//! window's track and theme, its play button goes through the main window's player, Escape
//! and a lost activation close it, and while it shows nothing in it animates or keeps a
//! timer. Prints what creating it costs.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay::mini;
use featherplay::snapshot::Scene;
use featherplay::ui::{MiniPlayer, Player, Theme, Toast};
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
