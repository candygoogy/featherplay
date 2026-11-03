//! Only synced lines are clickable. A synced line lights up under the pointer and a click
//! plays from it; plain lyrics have no times to seek to, so their lines neither light up nor
//! answer a click. In Now playing's full view, on the offscreen platform with the fixtures;
//! no window is shown.

use std::cell::Cell;
use std::rc::Rc;

use featherplay::lyrics::{self, SnapshotLyrics};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, Player};
use image::RgbaImage;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

fn frame(scene: &Scene) -> RgbaImage {
    let w = scene.ui.window();
    offscreen::present(w).expect("present");
    let px = offscreen::shown(w).expect("shown");
    RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()).expect("image")
}

/// The pointer moves onto `at` (two moves, as a hand makes) and rests there: how many pixels
/// changed.
fn hover(scene: &Scene, at: LogicalPosition) -> usize {
    let before = frame(scene);
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(at.x - 6.0, at.y),
    });
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    scene.settle();
    let after = frame(scene);
    // The stage above the player bar (the side panel starts at 880 at this size).
    (0..680)
        .flat_map(|y| (0..880).map(move |x| (x, y)))
        .filter(|&(x, y)| before.get_pixel(x, y) != after.get_pixel(x, y))
        .count()
}

fn click(scene: &Scene, at: LogicalPosition) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerExited);
    scene.settle();
}

/// One test: the platform can be installed once per process.
#[test]
fn only_synced_lines_light_up_and_seek() {
    let scene = Scene::new(1280, 800, true, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.theme(true);
    scene.ui.show().expect("show");
    let data = scene.ui.global::<LyricsData>();
    data.set_full(true);
    // The bar's seek, as a click on a line asks for it.
    let seeks = Rc::new(Cell::new(0));
    let s = seeks.clone();
    scene
        .ui
        .global::<Player>()
        .on_seek(move |_| s.set(s.get() + 1));
    // A line in the middle of the lyrics column.
    let at = LogicalPosition::new(420.0, 330.0);

    // Plain lyrics: no hover, and a click asks nothing of the line.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Plain);
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::Plain);
    let asked = Rc::new(Cell::new(0));
    let a = asked.clone();
    data.on_seek_line(move |_| a.set(a.get() + 1));
    let lit = hover(&scene, at);
    assert_eq!(lit, 0, "a plain line lit up under the pointer");
    click(&scene, at);
    assert_eq!(asked.get(), 0, "a plain line answered a click");
    assert_eq!(seeks.get(), 0, "a click on a plain line seeked");
    // The app's own handlers again, for the synced lines.
    lyrics::init(&scene.ctx);

    // Synced lyrics: the line lights up, and a click plays from it.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Synced);
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::Synced);
    let lit = hover(&scene, at);
    assert!(lit > 500, "a synced line didn't light up ({lit} px)");
    click(&scene, at);
    assert_eq!(seeks.get(), 1, "a click on a synced line didn't seek");
}
