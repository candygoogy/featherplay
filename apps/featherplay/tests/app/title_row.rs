//! Now playing's title row, offscreen (no window is shown): "Not for me", like and the song
//! menu are 40 px targets, evenly spaced, pinned at the cover's right edge on the title's
//! first line, so a title's length never moves them; a title too long for its line wraps to
//! a second before them, and they stay inside the stage, beside the Up next panel in a
//! narrow window too. A song rated Dislike shows the thumbs-down filled.

use std::ops::ControlFlow;

use featherplay::snapshot::Scene;
use featherplay::song_menu;
use featherplay::ui::{Frame, Player, Theme};
use featherplay_core::model::{LikeStatus, TrackId};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::lengths::LogicalRect;
use i_slint_core::window::WindowInner;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, PhysicalSize};

const LONG: &str = "In Love With Myself (feat. Somebody) – Extended Club Mix Remastered 2026";

/// Where every shown element labelled `label` is, in window coordinates, left of `right`
/// (the stage's right edge: the side panel's rows have titles and ⋯ buttons too). The
/// lyrics view's wrapped title, hidden in the cover view, has the title's label too.
fn rects(scene: &Scene, label: &str, right: f32) -> Vec<LogicalRect> {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut found = Vec::new();
    root.visit_descendants(|item| {
        if item.is_visible()
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|l| l == label)
        {
            let g = item.geometry();
            let r = LogicalRect::new(item.map_to_window(g.origin), g.size);
            if r.origin.x < right {
                found.push(r);
            }
        }
        ControlFlow::<()>::Continue(())
    });
    found
}

fn centre_y(r: &LogicalRect) -> f32 {
    r.origin.y + r.size.height / 2.0
}

/// The stage's right edge in a window `width` wide: the side panel takes the rest.
fn stage_right(scene: &Scene, width: f32) -> f32 {
    let theme = scene.ui.global::<Theme>();
    width
        - if width >= theme.get_wide_min() {
            theme.get_nowplaying_side_w()
        } else {
            theme.get_panel_w()
        }
}

/// The title and its three buttons, left to right: in the stage, the topmost title (the
/// bar's is at the bottom) and the buttons beside it (the bar has "Not for me" and like
/// too).
fn row(scene: &Scene, right: f32) -> (LogicalRect, [LogicalRect; 3]) {
    let title = scene.ui.global::<Player>().get_title();
    let title = rects(scene, &title, right)
        .into_iter()
        .min_by(|a, b| a.origin.y.total_cmp(&b.origin.y))
        .expect("the title");
    let on_line = |label: &str| {
        rects(scene, label, right)
            .into_iter()
            .filter(|r| centre_y(r) > title.origin.y && centre_y(r) < title.max_y())
            .min_by(|a, b| a.origin.x.total_cmp(&b.origin.x))
            .unwrap_or_else(|| panic!("no {label} button on the title's line"))
    };
    (
        title,
        [on_line("Not for me"), on_line("Like"), on_line("More")],
    )
}

/// Checks the row: 40 px targets, one gap, top-aligned with the title (centred on its first
/// line), clear of the title, inside the stage.
fn check(scene: &Scene, width: f32) -> (LogicalRect, [LogicalRect; 3]) {
    let theme = scene.ui.global::<Theme>();
    let right = stage_right(scene, width);
    let (title, buttons) = row(scene, right);
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    for b in &buttons {
        assert!(
            near(b.size.width, 40.0) && near(b.size.height, 40.0),
            "not 40 px targets: {buttons:?}"
        );
        assert!(
            near(b.origin.y, buttons[0].origin.y),
            "the buttons aren't on one line: {buttons:?}"
        );
    }
    assert!(
        centre_y(&buttons[0]) > title.origin.y && centre_y(&buttons[0]) < title.origin.y + 48.0,
        "the buttons aren't on the title's first line: {buttons:?}, title {title:?}"
    );
    let gaps = [
        buttons[1].origin.x - buttons[0].max_x(),
        buttons[2].origin.x - buttons[1].max_x(),
    ];
    assert!(
        gaps.iter().all(|&g| near(g, theme.get_s1())),
        "uneven gaps between the buttons: {gaps:?}"
    );
    assert!(
        buttons[0].origin.x - title.max_x() >= theme.get_s3() - 0.01,
        "the title runs under the buttons: {title:?}, {buttons:?}"
    );
    assert!(
        buttons[2].max_x() <= right - theme.get_s5(),
        "the buttons run past the stage: {buttons:?}, the stage ends at {right}"
    );
    (title, buttons)
}

#[test]
fn the_title_rows_buttons_stay_put_whatever_the_title() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.ui.global::<Player>().set_title("Da Funk".into());
    scene.settle();
    let (short, at) = check(&scene, 1280.0);

    // Too long for one line: the title wraps to a second; the buttons don't move.
    scene.ui.global::<Player>().set_title(LONG.into());
    scene.settle();
    let (long, moved) = check(&scene, 1280.0);
    assert!(
        long.size.height > short.size.height * 1.5,
        "the long title didn't wrap: {long:?}, one line {short:?}"
    );
    assert_eq!(moved, at, "the buttons moved with the title");
    assert_eq!(
        long.origin.y, short.origin.y,
        "the row moved for a long title"
    );

    // The user's narrow window, 1440 px at 150 %, with Up next open beside the page.
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    w.set_size(PhysicalSize::new(1440, 1200));
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(960.0, 800.0),
    });
    scene.ui.global::<Frame>().set_queue_open(true);
    scene.settle();
    check(&scene, 960.0);
}

/// "Not for me" shows a song rated Dislike filled, as the heart shows a liked one, and a
/// like from the heart clears it.
#[test]
fn a_dislike_fills_the_thumbs_down_and_a_like_clears_it() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.settle();
    let player = scene.ui.global::<Player>();
    assert!(!player.get_disliked());

    let id = TrackId(player.get_track_id().as_str().into());
    song_menu::show_rating(&scene.ctx, &id, LikeStatus::Dislike);
    assert!(player.get_disliked() && !player.get_liked());

    player.invoke_toggle_like();
    assert!(player.get_liked() && !player.get_disliked());
}
