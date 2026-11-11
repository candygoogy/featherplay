//! Skip silence's marks on the seek line, on the offscreen platform (no window is shown):
//! the controller's marks reach the bar's data (`Player.silences`) as they come, and none
//! after a track change; hovered, a mark's tip says so, and a click inside one seeks to its
//! end.

use std::ops::ControlFlow;
use std::path::PathBuf;

use featherplay::app::Ctx;
use featherplay::bar;
use featherplay::controller::Action;
use featherplay::covers::Covers;
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{AppWindow, Player};
use featherplay_controller::catalog::Catalog;
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::items::AccessibleRole;
use i_slint_core::lengths::LogicalRect;
use i_slint_core::window::WindowInner;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};
use tokio::sync::mpsc;

#[test]
fn the_bar_takes_the_marks_and_drops_them_with_the_track() {
    let _pump = offscreen::install().expect("platform");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let ui = AppWindow::new().expect("window");
    let catalog = Catalog::new(rt.handle().clone()).expect("catalog");
    let cache = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bar_silences");
    let covers = Covers::new(rt.handle().clone(), cache, None);
    let (actions, _) = mpsc::unbounded_channel();
    let ctx = Ctx::new(&ui, catalog, covers, actions);

    let spans = || {
        let m = ui.global::<Player>().get_silences();
        (0..m.row_count())
            .filter_map(|i| m.row_data(i))
            .map(|s| (s.start, s.end))
            .collect::<Vec<_>>()
    };
    assert!(spans().is_empty());
    bar::show_silences(&ctx, &[(0.0, 0.1), (0.85, 1.0)]);
    assert_eq!(spans(), [(0.0, 0.1), (0.85, 1.0)]);
    // A track change: the controller sends none.
    bar::show_silences(&ctx, &[]);
    assert!(spans().is_empty());
}

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
    w.dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1280.0, 800.0),
    });
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
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut found = None;
    root.visit_descendants(|item| {
        if item.is_visible()
            && item.accessible_role() == AccessibleRole::Slider
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|v| v == "Seek")
        {
            let g = item.geometry();
            found = Some(LogicalRect::new(item.map_to_window(g.origin), g.size));
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    found.expect("the seek line")
}

/// Whether a shown element reads `text`, as a `Text` does.
fn shows(scene: &Scene, text: &str) -> bool {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    root.visit_descendants(|item| {
        if item.is_visible()
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|v| v == text)
        {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .is_some()
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
fn a_click_inside_a_mark_seeks_to_its_end() {
    let scene = with_marks(&[(0.4, 0.6)]);
    click(&scene, along(&scene, 0.45));
    let to = sought(&scene);
    assert_eq!(to.len(), 1, "{to:?}");
    assert!(
        (to[0] - 0.6).abs() < 0.001,
        "sought {to:?}, not the mark's end"
    );
}

#[test]
fn hovered_over_a_mark_the_tip_says_silence() {
    let scene = with_marks(&[(0.4, 0.6)]);
    hover(&scene, along(&scene, 0.2));
    assert!(!shows(&scene, "Silence"), "over the plain line");
    hover(&scene, along(&scene, 0.5));
    assert!(shows(&scene, "Silence"), "over the mark");
}

#[test]
fn with_skip_silence_off_a_click_seeks_where_it_lands() {
    let scene = with_marks(&[(0.4, 0.6)]);
    // Turned off: the controller sends no marks.
    bar::show_silences(&scene.ctx, &[]);
    scene.settle();
    hover(&scene, along(&scene, 0.5));
    assert!(!shows(&scene, "Silence"));
    click(&scene, along(&scene, 0.45));
    let to = sought(&scene);
    assert_eq!(to.len(), 1, "{to:?}");
    assert!((to[0] - 0.45).abs() < 0.01, "sought {to:?}");
}
