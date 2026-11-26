//! Skip silence's marks on the seek line, on the offscreen platform (no window is shown):
//! hovered, a mark's tip says so, and a click inside one seeks to its end; once the
//! controller sends none (Skip silence off, a track change), a click seeks where it lands.

use featherplay::bar;
use featherplay::controller::Action;
use featherplay::snapshot::Scene;
use featherplay::ui::Player;
use i_slint_core::items::AccessibleRole;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

use crate::support::label::{self, LogicalRect};

/// A mark narrower than the bar's minimum is hit where it's drawn: pinned to the line's
/// start or end, centred inside it.
#[test]
fn a_short_mark_is_hit_where_it_is_drawn() {
    let min = 0.02;
    // A start mark 0.5 % long reaches 2 %; an end mark likewise from 98 %.
    assert_eq!(bar::silence_end([(0.0, 0.005)], 0.015, min), Some(0.005));
    assert_eq!(bar::silence_end([(0.997, 1.0)], 0.981, min), Some(1.0));
    // An inner one is centred: 0.5 ± 0.01.
    assert_eq!(bar::silence_end([(0.499, 0.501)], 0.509, min), Some(0.501));
    assert_eq!(bar::silence_end([(0.499, 0.501)], 0.52, min), None);
    // One near an end stays within the line: 0.1 % in, it's drawn from 0.
    assert_eq!(bar::silence_end([(0.001, 0.003)], 0.019, min), Some(0.003));
    // A long one is hit as it is.
    assert_eq!(bar::silence_end([(0.4, 0.6)], 0.61, min), None);
}

/// The window on the fixtures, playing, at 1280 × 800 (scale 1), with marks on the bar.
fn with_marks(marks: &[(f32, f32)]) -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.settle();
    bar::show_silences(&scene.ctx, marks);
    scene.settle();
    scene.take_actions();
    scene
}

/// The seek line, in window coordinates.
fn seek_line(scene: &Scene) -> LogicalRect {
    let line = label::shown_where(scene, |item, l| {
        l == "Seek" && item.accessible_role() == AccessibleRole::Slider
    });
    label::rect(line.first().expect("the seek line"))
}

/// The point `fraction` along the seek line.
fn along(scene: &Scene, fraction: f32) -> LogicalPosition {
    let line = seek_line(scene);
    LogicalPosition::new(
        line.origin.x + line.size.width * fraction,
        line.origin.y + line.size.height / 2.0,
    )
}

fn hover(scene: &Scene, at: LogicalPosition) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    scene.settle();
}

fn click(scene: &Scene, at: LogicalPosition) {
    let w = scene.ui.window();
    hover(scene, at);
    w.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button: PointerEventButton::Left,
    });
    scene.settle();
}

/// Where the window asked to seek since the last call, as fractions of the duration.
fn sought(scene: &Scene) -> Vec<f32> {
    let duration = scene.ui.global::<Player>().get_duration_s();
    scene
        .take_actions()
        .into_iter()
        .filter_map(|a| match a {
            Action::SeekTo(at) => Some(at.as_secs_f32() / duration),
            _ => None,
        })
        .collect()
}

#[test]
fn a_mark_says_silence_when_hovered_and_a_click_inside_seeks_to_its_end() {
    let scene = with_marks(&[(0.4, 0.6)]);
    hover(&scene, along(&scene, 0.2));
    assert!(
        !label::shows(&scene, "Silence"),
        "a tip over the plain line"
    );
    hover(&scene, along(&scene, 0.5));
    assert!(label::shows(&scene, "Silence"), "no tip over the mark");
    click(&scene, along(&scene, 0.45));
    let to = sought(&scene);
    assert_eq!(to.len(), 1, "a click inside the mark: {to:?}");
    assert!(
        (to[0] - 0.6).abs() < 0.001,
        "a click inside the mark sought {to:?}, not the mark's end"
    );

    // Skip silence turned off, or a track change: the controller sends no marks, and a
    // click seeks where it lands.
    bar::show_silences(&scene.ctx, &[]);
    scene.settle();
    hover(&scene, along(&scene, 0.5));
    assert!(
        !label::shows(&scene, "Silence"),
        "a tip with the marks gone"
    );
    click(&scene, along(&scene, 0.45));
    let to = sought(&scene);
    assert_eq!(to.len(), 1, "a click with the marks gone: {to:?}");
    assert!(
        (to[0] - 0.45).abs() < 0.01,
        "a click with the marks gone sought {to:?}"
    );
}
