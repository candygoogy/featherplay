//! Now playing's title row, offscreen (no window is shown): "Not for me", like and the song
//! menu after the title are the bar's buttons, one size and evenly spaced, centred on the
//! title's line; a title too long for the line elides before them, so they stay on it and
//! inside the stage, beside the Up next panel in a narrow window too. A song rated Dislike
//! shows the thumbs-down filled.

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
/// bar's is at the bottom) and the buttons on its line (the bar has "Not for me" and like
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
            .filter(|r| (centre_y(r) - centre_y(&title)).abs() < 24.0)
            .min_by(|a, b| a.origin.x.total_cmp(&b.origin.x))
            .unwrap_or_else(|| panic!("no {label} button on the title's line"))
    };
    (
        title,
        [on_line("Not for me"), on_line("Like"), on_line("More")],
    )
}

/// Checks the row: one size, one gap, centred on the title's line, inside the cover's room.
fn check(scene: &Scene, width: f32) -> (LogicalRect, [LogicalRect; 3]) {
    let theme = scene.ui.global::<Theme>();
    let right = stage_right(scene, width);
    let (title, buttons) = row(scene, right);
    for b in &buttons {
        assert_eq!(
            b.size, buttons[0].size,
            "the buttons differ in size: {buttons:?}"
        );
        assert!(
            (centre_y(b) - centre_y(&title)).abs() < 0.5,
            "a button isn't centred on the title's line: {b:?}, title {title:?}"
        );
    }
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(
        near(buttons[0].size.width, 36.0),
        "not the bar's 36 px buttons"
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
        near(buttons[0].origin.x - title.max_x(), theme.get_s3()),
        "the gap after the title isn't `s3`: {title:?}, {buttons:?}"
    );
    // The cover is centred in the stage with at least `s9` either side.
    assert!(
        buttons[2].max_x() <= right - theme.get_s9(),
        "the buttons run past the cover: {buttons:?}, the stage ends at {right}"
    );
    (title, buttons)
}

#[test]
fn the_title_rows_buttons_match_and_stay_on_the_line() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.settle();
    let (short, at) = check(&scene, 1280.0);

    // Too long for the line: the title elides before the buttons, which move out to the
    // cover's right edge and no further.
    scene.ui.global::<Player>().set_title(LONG.into());
    scene.settle();
    let (long, moved) = check(&scene, 1280.0);
    assert!(
        long.size.width > short.size.width,
        "the long title didn't take the room"
    );
    assert!(
        moved[0].origin.x > at[0].origin.x,
        "the buttons didn't follow the title"
    );
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
