//! The volume slider keeps its hovered look while the wheel turns it: the layer that holds
//! the hover during a wheel scroll (`Frame.scrolling`) stops at the bar, which never scrolls.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, Player, Theme};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};

/// Whether the slider took a wheel notch at `at` (it calls `set-volume`).
fn takes_wheel(scene: &Scene, calls: &Rc<RefCell<usize>>, at: LogicalPosition) -> bool {
    let before = *calls.borrow();
    offscreen::wheel(scene.ui.window(), at, 1.0);
    *calls.borrow() > before
}

/// The rendered colour at a logical point (the window is at scale 1).
fn pixel(w: &slint::Window, p: LogicalPosition) -> [u8; 3] {
    let shot = w.take_snapshot().expect("snapshot");
    let px = shot.as_slice()[p.y as usize * shot.width() as usize + p.x as usize];
    [px.r, px.g, px.b]
}

/// Whether `px` is the accent (the hovered fill), not the resting fill's `text2`.
fn is_accent(scene: &Scene, px: [u8; 3]) -> bool {
    let a = scene.ui.global::<Theme>().get_accent();
    let close = |x: u8, y: u8| x.abs_diff(y) <= 6;
    close(px[0], a.red()) && close(px[1], a.green()) && close(px[2], a.blue())
}

#[test]
fn the_slider_stays_lit_while_the_wheel_holds_the_hover() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.ui.show().expect("show");
    scene.settle();
    let player = scene.ui.global::<Player>();
    player.set_volume(0.8);
    // Count the slider's calls but keep the volume where it is.
    let calls = Rc::new(RefCell::new(0usize));
    let seen = calls.clone();
    player.on_set_volume(move |_, _| *seen.borrow_mut() += 1);
    let w = scene.ui.window();

    // The slider's left edge and its vertical extent, by trying notches.
    let mut found = None;
    'scan: for y in (700..800).step_by(4) {
        for x in (800..1280).step_by(6) {
            let at = LogicalPosition::new(x as f32, y as f32);
            w.dispatch_event(WindowEvent::PointerMoved { position: at });
            if takes_wheel(&scene, &calls, at) {
                found = Some(at);
                break 'scan;
            }
        }
    }
    let top = found.expect("no volume slider took the wheel");
    let mut bottom = top.y;
    while takes_wheel(&scene, &calls, LogicalPosition::new(top.x, bottom + 1.0)) {
        bottom += 1.0;
    }
    let fill = LogicalPosition::new(top.x + 3.0, ((top.y + bottom) / 2.0).round());

    w.dispatch_event(WindowEvent::PointerMoved { position: fill });
    scene.settle();
    let px = pixel(w, fill);
    assert!(
        is_accent(&scene, px),
        "the hovered fill isn't the accent: {px:?}"
    );

    // What `wheel.rs` does on a notch: the hover-holding layer goes over the window, then
    // the notch itself arrives.
    scene.ui.global::<Frame>().set_scrolling(true);
    assert!(
        takes_wheel(&scene, &calls, fill),
        "the wheel no longer reaches the slider"
    );
    scene.settle();
    let px = pixel(w, fill);
    assert!(
        is_accent(&scene, px),
        "the slider went grey under the wheel: {px:?}"
    );
}
