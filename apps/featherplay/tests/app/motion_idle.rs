//! Motion runs, then stops, on the offscreen platform (no window is
//! shown), with Windows' animation effects on: a page fades in after a navigation, and once
//! it has settled nothing animates and no timer is armed, so a paused app doesn't redraw.

use std::time::{Duration, Instant};

use featherplay::offscreen;
use featherplay::ui::{
    AlbumData, AppWindow, Frame, Nav, PageKind, Player, SongMenu, Theme, Toast, TrackRow,
};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};

/// Runs timers and animations for `ms`, rendering a frame every few milliseconds as the
/// window would.
fn run_for(w: &slint::Window, ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        slint::platform::update_timers_and_animations();
        w.take_snapshot().expect("snapshot");
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// After a last frame: is any animation still running, or any timer armed?
fn busy(w: &slint::Window) -> (bool, Option<Duration>) {
    slint::platform::update_timers_and_animations();
    w.take_snapshot().expect("snapshot");
    (
        w.has_active_animations(),
        slint::platform::duration_until_next_timer_update(),
    )
}

fn assert_idle(w: &slint::Window, after: &str) {
    run_for(w, 600);
    let (animating, timer) = busy(w);
    assert!(!animating, "still animating after {after}");
    assert!(
        timer.is_none(),
        "a timer is still armed after {after}: {timer:?}"
    );
}

/// The pixels right of the sidebar and above the bar, where pages are shown.
fn content(shot: &slint::SharedPixelBuffer<slint::Rgba8Pixel>) -> Vec<slint::Rgba8Pixel> {
    let w = shot.width() as usize;
    shot.as_slice()
        .chunks(w)
        .take(700)
        .flat_map(|row| row[250..].iter().copied())
        .collect()
}

/// One test: the platform can be installed once per process.
#[test]
fn motion_settles_and_leaves_nothing_running() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    let theme = ui.global::<Theme>();
    theme.set_motion(true);
    ui.show().expect("show");
    let w = ui.window();
    assert_idle(w, "showing the window");

    // A mouse navigation: the page fades in, then everything stops.
    let album = ui.global::<AlbumData>();
    album.set_title("Album title".into());
    let nav = ui.global::<Nav>();
    nav.set_page(PageKind::Album);
    let mut frames = Vec::new();
    let end = Instant::now() + Duration::from_millis(400);
    while Instant::now() < end {
        slint::platform::update_timers_and_animations();
        frames.push(w.take_snapshot().expect("snapshot"));
        std::thread::sleep(Duration::from_millis(4));
    }
    let last = frames.last().expect("frames").as_slice().to_vec();
    let differing = frames
        .iter()
        .filter(|f| f.as_slice() != last.as_slice())
        .count();
    assert!(
        differing >= 2,
        "the page appeared without its enter fade ({differing} frames moved)"
    );
    assert_idle(w, "a page enter");

    // A keyboard navigation (Frame.instant) shows the page at once. Not to Settings: it makes
    // its sections below the fold a tick after its first frame, and its scroll indicator
    // shortens then (`settings.slint`, `made`).
    ui.global::<Frame>().set_instant(true);
    nav.set_page(PageKind::About);
    slint::platform::update_timers_and_animations();
    let first = w.take_snapshot().expect("snapshot");
    run_for(w, 400);
    let settled = w.take_snapshot().expect("snapshot");
    // The content area only: the sidebar's own item fades to selected.
    assert!(
        content(&first) == content(&settled),
        "a keyboard navigation faded in"
    );
    assert!(
        !ui.global::<Frame>().get_instant(),
        "Frame.instant stayed set"
    );
    assert_idle(w, "a keyboard navigation");

    // The song menu fades in and out, then nothing runs.
    let menu = ui.global::<SongMenu>();
    let row = TrackRow {
        id: "a".into(),
        title: "Title".into(),
        ..Default::default()
    };
    let before = content(&w.take_snapshot().expect("snapshot"));
    menu.invoke_show(row, 600.0, 200.0, 200.0);
    run_for(w, 300);
    assert!(
        content(&w.take_snapshot().expect("snapshot")) != before,
        "the menu didn't show"
    );
    assert_idle(w, "opening the song menu");
    menu.invoke_close();
    assert_idle(w, "closing the song menu");
    assert!(
        content(&w.take_snapshot().expect("snapshot")) == before,
        "the menu didn't go away"
    );

    // A toast, shown and closed.
    let toast = ui.global::<Toast>();
    toast.set_text("Added to queue: Title".into());
    toast.set_open(true);
    assert_idle(w, "showing a toast");
    toast.set_open(false);
    assert_idle(w, "closing a toast");

    // Hovering the seek line shows its thumb and time; leaving hides them.
    let player = ui.global::<Player>();
    player.set_has_track(true);
    player.set_duration_s(200.0);
    player.set_position_s(50.0);
    let over = |x: f32, y: f32| {
        w.dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(x, y),
        })
    };
    // The seek line: under the transport, 68 px down the bar (702 at 800 px).
    over(640.0, 770.0);
    assert_idle(w, "hovering the seek line");
    over(640.0, 400.0);
    assert_idle(w, "leaving the seek line");

    // The Up next panel slides in and out, wide and narrow.
    let frame = ui.global::<Frame>();
    for (width, height) in [(1280.0, 800.0), (1000.0, 700.0)] {
        w.set_size(slint::LogicalSize::new(width, height));
        assert_idle(w, "resizing");
        frame.set_queue_open(true);
        assert_idle(w, "opening Up next");
        frame.set_queue_open(false);
        assert_idle(w, "closing Up next");
    }

    // Animation effects off: fades stay, shorter; nothing moves or scales.
    theme.set_motion(false);
    assert!(
        theme.get_t_page() > 0 && theme.get_t_page() < 200,
        "fades should stay, shorter"
    );
    assert_eq!(theme.get_t_panel(), 0, "the panel would slide");
    assert_eq!(theme.get_t_press(), 0, "presses would scale");
    assert_eq!(theme.get_rise(), 0.0, "pages would rise");
    assert_eq!(theme.get_nudge(), 0.0, "menus would move");
    assert_eq!(theme.get_pop_from(), 1.0, "menus would grow");
    assert_eq!(theme.get_press_scale(), 1.0, "presses would scale");
    assert_eq!(theme.get_np_from(), 1.0, "Now playing would grow");
    nav.set_page(PageKind::Album);
    assert_idle(w, "a page enter without motion");
}
