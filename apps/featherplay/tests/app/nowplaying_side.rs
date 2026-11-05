//! On Now playing, the bar's Up next button shows and hides the page's side panel (Up next
//! and Related), as it does the frame's panel on the other pages. Hidden, the stage takes the
//! whole width: the window's right edge shows the stage's wash, the same colour as its left
//! edge at that height. The choice is the page's own: the frame's panel keeps its state.
//! Through the renderer's own frames (`offscreen::install_presenting`); no window is shown.

use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, Theme};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 800.0;

fn click(w: &slint::Window, at: LogicalPosition) {
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    let button = PointerEventButton::Left;
    w.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button,
    });
    // Off the button again, so its hover doesn't stay in the frames that follow.
    w.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(WIDTH / 2.0, 20.0),
    });
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

/// Whether the window's right edge shows the side panel: its colour there differs from the
/// stage's at the left edge, at the same height (the wash runs down the stage, not across).
fn side_shown(scene: &Scene) -> bool {
    let w = scene.ui.window();
    let y = 300.0;
    pixel(w, LogicalPosition::new(WIDTH - 6.0, y)) != pixel(w, LogicalPosition::new(6.0, y))
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
