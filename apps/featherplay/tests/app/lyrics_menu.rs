//! The lyrics menu (the ⋯ beside the Lyrics pill on Now playing's full view) and "Choose
//! other lyrics": the text size for the app (saved as a setting, Ctrl+= and Ctrl+- too), the
//! lines half a second sooner or later for this song (kept on this PC, noted in the credit,
//! Reset), and LRCLIB's matches for the song, a pick used at once and kept for the song;
//! Large lays out in the narrowest window. On the offscreen platform with the demo
//! fixtures; no window is shown.

use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use featherplay::snapshot::Scene;
use featherplay::ui::{
    ChooseState, Frame, LyricsChoose, LyricsData, LyricsMenu, LyricsState, NowPlayingData, Theme,
};
use featherplay_store::{LyricsChoices, LyricsPick, Paths};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::items::AccessibleRole;
use i_slint_core::lengths::LogicalRect;
use i_slint_core::window::WindowInner;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model};

/// Now playing's full lyrics view on the demo set's first song, its synced lines in.
fn scene() -> Scene {
    // SAFETY: set before anything reads it, on the test's only thread so far (nextest runs
    // each test in its own process).
    unsafe { std::env::set_var("FEATHERPLAY_FIXTURES", "demo") };
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.settle();
    scene.ui.global::<LyricsData>().invoke_toggle_view();
    scene.settle();
    let data = scene.ui.global::<LyricsData>();
    assert_eq!(data.get_state(), LyricsState::Synced);
    assert!(data.get_showing(), "the full view didn't open");
    scene
}

/// The playing song's choices as saved (they're written off the UI thread).
fn saved(scene: &Scene, ok: impl Fn(&featherplay_store::LyricsChoice) -> bool) -> bool {
    let id = scene
        .ctx
        .player()
        .and_then(|s| s.current)
        .expect("a song")
        .id;
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if ok(&LyricsChoices::load(&Paths::resolve()).get(&id.0)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn ctrl(scene: &Scene, key: &str) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    w.dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    w.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    scene.settle();
}

#[test]
fn the_menu_sizes_the_lines_and_moves_them_for_this_song() {
    let scene = scene();
    let data = scene.ui.global::<LyricsData>();
    let menu = scene.ui.global::<LyricsMenu>();
    // The ⋯ is made as a frame is: one first, so it's there to open its menu.
    scene.ui.window().take_snapshot().expect("frame");
    menu.set_open_asked(menu.get_open_asked() + 1);
    scene.settle();
    assert!(menu.get_open(), "the ⋯ didn't open the menu");

    // Text size: shown at once and saved as a setting.
    scene.take_actions();
    data.invoke_pick_size(2);
    scene.settle();
    assert_eq!(data.get_size(), 2);
    let actions = format!("{:?}", scene.take_actions());
    assert!(actions.contains("LyricsSize(Large)"), "{actions}");

    // Timing: the lines move later, the credit says so, and it's kept for the song.
    let on_time = data.get_current();
    assert!(on_time > 0, "no line playing at 2:41");
    assert!(!data.get_timing_moved());
    for _ in 0..30 {
        data.invoke_move_timing(1);
    }
    scene.settle();
    assert_eq!(data.get_timing(), "10 s later", "it stops at 10 s");
    assert!(data.get_timing_moved());
    assert!(
        data.get_current() < on_time,
        "the line playing didn't move back: {} vs {on_time}",
        data.get_current()
    );
    assert!(
        data.get_credit().ends_with(" · 10 s later for this song"),
        "{}",
        data.get_credit()
    );
    assert!(
        saved(&scene, |c| c.nudge_ms == 10_000),
        "the timing wasn't kept"
    );
    assert!(menu.get_open(), "moving the lines closed the menu");

    // Reset: on time again, and nothing kept.
    data.invoke_reset_timing();
    scene.settle();
    assert_eq!(data.get_current(), on_time);
    assert_eq!(data.get_timing(), "On time");
    assert!(!data.get_credit().contains("for this song"));
    assert!(saved(&scene, |c| c.nudge_ms == 0));

    // Ctrl+- and Ctrl+= step the size while the lyrics show (the menu closed first, by
    // Escape: keys wait under it).
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    scene.settle();
    assert!(!menu.get_open());
    ctrl(&scene, "-");
    assert_eq!(data.get_size(), 1);
    ctrl(&scene, "-");
    ctrl(&scene, "-");
    assert_eq!(data.get_size(), 0, "past Small");
    ctrl(&scene, "=");
    assert_eq!(data.get_size(), 1);
}

#[test]
fn another_match_picked_is_used_at_once_and_kept_for_the_song() {
    let scene = scene();
    let data = scene.ui.global::<LyricsData>();
    let choose = scene.ui.global::<LyricsChoose>();
    let first_line = |scene: &Scene| {
        scene
            .ui
            .global::<LyricsData>()
            .get_lines()
            .iter()
            .find(|l| !l.is_empty())
            .unwrap_or_default()
    };
    let automatic = first_line(&scene);

    data.invoke_choose();
    scene.settle();
    assert!(choose.get_open());
    assert_eq!(choose.get_state(), ChooseState::List);
    let matches = choose.get_matches();
    assert_eq!(matches.row_count(), 4);
    let in_use: Vec<bool> = matches.iter().map(|m| m.in_use).collect();
    assert_eq!(
        in_use,
        [true, false, false, false],
        "the lyrics shown are marked"
    );
    assert!(!choose.get_picked());

    // The live recording: its lines replace the automatic match's, and it's kept.
    let live_first = matches.row_data(1).expect("row").first_line;
    choose.invoke_pick(1);
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::Synced);
    assert_eq!(first_line(&scene), live_first);
    assert_ne!(first_line(&scene), automatic);
    assert!(data.get_picked());
    assert!(saved(&scene, |c| matches!(
        c.pick,
        Some(LyricsPick::Lrclib { id: 2, .. })
    )));

    // Opened again: the pick is in use, and the foot offers the automatic match back.
    data.invoke_choose();
    scene.settle();
    assert!(choose.get_picked());
    let in_use: Vec<bool> = choose.get_matches().iter().map(|m| m.in_use).collect();
    assert_eq!(in_use, [false, true, false, false]);
    choose.invoke_automatic();
    choose.set_open(false);
    scene.settle();
    assert_eq!(first_line(&scene), automatic);
    assert!(!data.get_picked());
    assert!(saved(&scene, |c| c.pick.is_none()));

    // None of these: no lyrics for this song, said so.
    choose.invoke_pick_none();
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::None);
    assert_eq!(data.get_why(), "You chose no lyrics for this song.");
}

/// Where every lyric line of a lyrics view lies, in window coordinates, top to bottom: the
/// lines whose left edge is in `from..to` (the full view's column, or the side tab's).
fn line_rects(scene: &Scene, from: f32, to: f32) -> Vec<LogicalRect> {
    let lines: Vec<String> = scene
        .ui
        .global::<LyricsData>()
        .get_lines()
        .iter()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut rects = Vec::new();
    root.visit_descendants(|item| {
        if item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|v| lines.iter().any(|l| *l == v.as_str()))
        {
            let g = item.geometry();
            let r = LogicalRect::new(item.map_to_window(g.origin), g.size);
            if (from..to).contains(&r.origin.x) && r.size.height > 0.0 {
                rects.push(r);
            }
        }
        ControlFlow::<()>::Continue(())
    });
    rects.sort_by(|a, b| a.origin.y.total_cmp(&b.origin.y));
    rects
}

/// The lines don't overlap, stay inside `right`, and the long ones wrap (taller than one
/// line of type `size`).
fn assert_laid_out(rects: &[LogicalRect], right: f32, size: f32, view: &str) {
    assert!(rects.len() > 5, "{view}: {} lines found", rects.len());
    for pair in rects.windows(2) {
        assert!(
            pair[1].origin.y >= pair[0].origin.y + pair[0].size.height - 0.5,
            "{view}: lines overlap: {:?} and {:?}",
            pair[0],
            pair[1]
        );
    }
    for r in rects {
        assert!(
            r.origin.x + r.size.width <= right + 0.5,
            "{view}: past the edge: {r:?}"
        );
    }
    assert!(
        rects.iter().any(|r| r.size.height > 2.0 * size),
        "{view}: no line wraps at Large"
    );
}

#[test]
fn large_text_fits_the_narrowest_window_in_the_full_view_and_the_side_tab() {
    // SAFETY: as in `scene`.
    unsafe { std::env::set_var("FEATHERPLAY_FIXTURES", "demo") };
    // The window's least width.
    let scene = Scene::new(960, 640, true, false).expect("scene");
    scene.ui.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(960.0, 640.0),
    });
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.settle();
    let data = scene.ui.global::<LyricsData>();
    data.invoke_pick_size(2);
    data.invoke_toggle_view();
    scene.settle();
    scene.ui.window().take_snapshot().expect("frame");
    assert!(data.get_showing());
    // Below `wide-min`, Now playing's side panel is `panel-w` wide; the stage is the rest.
    // Open, as the side tab needs it below.
    scene.ui.global::<Frame>().set_side_open(true);
    scene.settle();
    scene.ui.window().take_snapshot().expect("frame");
    let t = scene.ui.global::<Theme>();
    let stage = 960.0 - t.get_panel_w();
    assert_laid_out(
        &line_rects(&scene, 0.0, stage),
        stage,
        30.0,
        "the full view",
    );

    // The side tab, at Large.
    data.invoke_toggle_view();
    let np = scene.ui.global::<NowPlayingData>();
    np.set_tab(2);
    np.invoke_tab_selected(2);
    scene.settle();
    scene.ui.window().take_snapshot().expect("frame");
    assert!(!data.get_showing());
    assert_laid_out(
        &line_rects(&scene, stage, 960.0),
        960.0,
        24.0,
        "the side tab",
    );
}
