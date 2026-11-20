//! On Now playing, the bar's Up next button shows and hides the page's side panel (Up next
//! and Related), as it does the frame's panel on the other pages. Hidden, the stage takes the
//! whole width: the window's right edge shows the page's ground, the cover's field as its
//! picture draws it there. The choice is the page's own: the frame's panel keeps its state.
//! Through the renderer's own frames (`offscreen::install_presenting`); no window is shown.

use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, Player, Theme};
use slint::{ComponentHandle, LogicalPosition};

use crate::support;

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 800.0;

fn click(w: &slint::Window, at: LogicalPosition) {
    support::click(w, at);
    // Off the button again, so its hover doesn't stay in the frames that follow.
    support::point(w, LogicalPosition::new(WIDTH / 2.0, 20.0));
}

/// Frames until every slide and fade has ended.
fn settle(scene: &Scene) {
    let w = scene.ui.window();
    for _ in 0..90 {
        offscreen::advance(Duration::from_millis(16));
        offscreen::present(w).expect("present");
    }
    scene.settle();
}

/// The shown pixel at `at` (logical px, scale 1).
fn pixel(w: &slint::Window, at: LogicalPosition) -> [u8; 3] {
    let shot = offscreen::shown(w).expect("shown");
    let px = shot.as_slice()[at.y as usize * shot.width() as usize + at.x as usize];
    [px.r, px.g, px.b]
}

/// The bar's Up next button: found on Home, the rightmost spot in the bar whose click
/// toggles the frame's panel. Leaves the panel as it found it.
fn queue_button(scene: &Scene) -> LogicalPosition {
    let w = scene.ui.window();
    let theme = scene.ui.global::<Theme>();
    let y = HEIGHT - theme.get_bar_inset() - theme.get_bar_h() / 2.0;
    let frame = scene.ui.global::<Frame>();
    let mut x = WIDTH - theme.get_bar_inset() - 2.0;
    while x > WIDTH / 2.0 {
        let at = LogicalPosition::new(x, y);
        let before = frame.get_queue_open();
        click(w, at);
        if frame.get_queue_open() != before {
            click(w, at);
            settle(scene);
            return at;
        }
        x -= 4.0;
    }
    panic!("no Up next button in the bar");
}

/// Whether the side panel's card shows at the window's right, inside the bar's inset: its
/// colour there is a few levels off the cover's field there (the field runs under the stage
/// and round the card).
fn side_shown(scene: &Scene) -> bool {
    let w = scene.ui.window();
    let (x, y) = (
        WIDTH - scene.ui.global::<Theme>().get_bar_inset() - 6.0,
        300.0,
    );
    let shown = pixel(w, LogicalPosition::new(x, y));
    let field = support::field(&scene.ui).expect("no picture of the field")(x as u32, y as u32);
    (0..3)
        .map(|k| u32::from(shown[k].abs_diff(field[k])))
        .sum::<u32>()
        > 6
}

/// The leftmost and rightmost columns where the stage shows anything but its ground (the
/// cover's field), between the back button's row and the bar (logical px, scale 1).
fn drawn_across(scene: &Scene) -> (f32, f32) {
    let w = scene.ui.window();
    let shot = offscreen::shown(w).expect("shown");
    let width = shot.width() as usize;
    let bar_top = (HEIGHT - scene.ui.global::<Theme>().get_bar_clear()) as usize;
    let (mut left, mut right) = (width, 0);
    // Below the header row.
    let top = 72 + support::np_drop(&scene.ui) as usize;
    let field = support::field(&scene.ui).expect("no picture of the field");
    for y in top..bar_top {
        let row = &shot.as_slice()[y * width..(y + 1) * width];
        let near = |x: usize| {
            let (p, g) = (row[x], field(x as u32, y as u32));
            p.r.abs_diff(g[0]) <= 3 && p.g.abs_diff(g[1]) <= 3 && p.b.abs_diff(g[2]) <= 3
        };
        if let Some(x) = (0..width).find(|&x| !near(x)) {
            left = left.min(x);
        }
        if let Some(x) = (0..width).rev().find(|&x| !near(x)) {
            right = right.max(x + 1);
        }
    }
    (left as f32, right as f32)
}

/// One test: the platform can be installed once per process.
#[test]
fn the_up_next_button_hides_and_shows_now_playings_side_panel() {
    let scene = Scene::new(WIDTH as u32, HEIGHT as u32, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    settle(&scene);
    let button = queue_button(&scene);
    let frame = scene.ui.global::<Frame>();
    let queue_open = frame.get_queue_open();

    scene.open("now-playing").expect("now playing");
    settle(&scene);
    assert!(
        side_shown(&scene),
        "Now playing opened without its side panel"
    );

    click(scene.ui.window(), button);
    settle(&scene);
    assert!(
        !side_shown(&scene),
        "the Up next button left Now playing's side panel up"
    );
    assert!(
        !frame.get_side_open(),
        "the side panel's state didn't follow"
    );

    // A soundtrack's credits, far wider than the cover: the cover and its lines still sit as
    // one block centred in the stage.
    let player = scene.ui.global::<Player>();
    player.set_artist("Petite Meller, Kalman Chodakiewitz, Eli Shiffman".into());
    player.set_album("Cyberpunk: Edgerunners 2 (Original Series Soundtrack)".into());
    settle(&scene);
    let (left, right) = drawn_across(&scene);
    assert!(
        (left + right - WIDTH).abs() <= 4.0,
        "the block isn't centred in the stage: drawn from {left} to {right} of {WIDTH}"
    );

    click(scene.ui.window(), button);
    settle(&scene);
    assert!(
        side_shown(&scene),
        "the Up next button didn't bring the side panel back"
    );
    assert_eq!(
        frame.get_queue_open(),
        queue_open,
        "the frame's panel changed with Now playing's"
    );
}
