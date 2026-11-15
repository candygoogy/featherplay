//! The player bar from 960 px down to 640, on the offscreen platform (no window is shown). As
//! it narrows, the title shortens, then the speed pill, the volume's slider (its button stays)
//! and the extra buttons go in turn; the title, the transport and the time row never run into
//! each other or into the right-hand side. At 960 px everything still shows. The steps below
//! 640 px are `narrow_shell.rs`'s.

use std::ops::ControlFlow;
use std::time::Duration;

use featherplay::app;
use featherplay::controller::{SleepKind, SleepState, UiEvent};
use featherplay::snapshot::Scene;
use featherplay::ui::{Player, SettingsData, Theme};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::window::WindowInner;
use slint::ComponentHandle;
use slint::platform::WindowEvent;

const H: u32 = 800;

/// The left and right edges of the visible element named `label` in the bar (at or below
/// `top`), if there is one.
fn span(scene: &Scene, label: &str, top: f32) -> Option<(f32, f32)> {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut found = None;
    root.visit_descendants(|item| {
        let named = item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|l| l == label);
        if named && item.is_visible() {
            let at = item.map_to_window(item.geometry().origin);
            if at.y >= top {
                found = Some((at.x, at.x + item.geometry().width()));
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(())
    });
    found
}

/// The worst case for room: Linux without a tray (the mini player's button), the sleep timer
/// on (its crescent, or running: its wider pill) and the speed at 1.25x (its pill), at 960,
/// 800 and 640 px.
#[test]
fn a_narrow_bar_gives_groups_up_and_nothing_overlaps() {
    let scene = Scene::new(960, H, true, false).expect("scene");
    scene.play(false).expect("player");
    let data = scene.ui.global::<SettingsData>();
    data.set_has_tray(false);
    data.set_sleep_timer(true);
    scene.ui.global::<Player>().set_speed(3);
    let t = scene.ui.global::<Theme>();
    let top = H as f32 - t.get_bar_inset() - t.get_bar_h();
    let title_x = t.get_bar_inset() + 84.0;
    let title = scene.ui.global::<Player>().get_title();
    let right = [
        "Playback speed, 1.25×",
        "Mini player",
        "Sleep timer",
        "Lyrics",
        "Not for me",
        "Mute",
    ];

    // The window's width, the sleep timer's minutes left if it runs, and what shows on the
    // right then.
    for (width, running, shown) in [
        (960, Some(24), &right[..]),
        (960, None, &right[..]),
        (
            800,
            None,
            &["Mini player", "Sleep timer", "Lyrics", "Not for me"][..],
        ),
        (640, None, &["Not for me"][..]),
    ] {
        app::on_event(UiEvent::Sleep(running.map(|minutes| SleepState {
            kind: SleepKind::Time,
            minutes,
            fading: false,
            left: Duration::from_secs(u64::from(minutes) * 60),
        })));
        let case = match running {
            Some(m) => format!("{width} px, the timer at {m} min"),
            None => format!("{width} px"),
        };
        let w = scene.ui.window();
        w.set_size(slint::PhysicalSize::new(width, H));
        let size = slint::LogicalSize::new(width as f32, H as f32);
        w.dispatch_event(WindowEvent::Resized { size });
        scene.settle();
        let edge = |label: &str| {
            span(&scene, label, top).unwrap_or_else(|| panic!("{case}: no {label:?}"))
        };
        for label in right {
            assert_eq!(
                span(&scene, label, top).is_some(),
                shown.contains(&label),
                "{case}: is {label:?} there?"
            );
        }
        let right_start = right
            .iter()
            .filter_map(|l| span(&scene, l, top))
            .map(|(left, _)| left)
            .fold(f32::MAX, f32::min);
        let title_end = edge(&title).1;
        let shuffle_start = edge("Shuffle off").0;
        let repeat_end = edge("Repeat off").1;
        // The times (44 px) and their gaps (12 px) either side of the seek line.
        let seek = edge("Seek");
        let (time_start, time_end) = (seek.0 - 56.0, seek.1 + 56.0);
        assert!(
            title_end <= time_start && title_end < shuffle_start,
            "{case}: the title ends at {title_end}, the time row starts at {time_start}, \
             Shuffle at {shuffle_start}"
        );
        assert!(
            right_start - repeat_end >= 8.0 && right_start - time_end >= 8.0,
            "{case}: Repeat ends at {repeat_end}, the time row at {time_end}, the \
             right-hand side starts at {right_start}"
        );
        assert!(
            title_end - title_x >= 80.0,
            "{case}: the title has only {} px",
            title_end - title_x
        );
    }
}
