//! The playing mark (the wave) moves only while sound plays, on the offscreen platform (no
//! window is shown), with Windows' animation effects on: its one clock (`Mark`) steps at
//! 30 fps while the current track plays, and nothing is armed or redrawn when nothing plays,
//! or when it's paused, loading, minimized, hidden in the tray, or with reduced motion.

use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay::offscreen;
use featherplay::ui::{
    AppWindow, Frame, Mark, Nav, PageKind, Player, PlaylistData, Theme, TrackRow,
};
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

/// Three rows; `b` is the current track.
fn rows() -> ModelRc<TrackRow> {
    let rows: Vec<TrackRow> = ["a", "b", "c"]
        .iter()
        .enumerate()
        .map(|(i, id)| TrackRow {
            id: (*id).into(),
            index: (i + 1).to_string().into(),
            title: format!("Title {id}").into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration: "3:00".into(),
            playing: *id == "b",
            ..Default::default()
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

/// Runs timers and animations for `ms`, rendering a frame every few milliseconds as the
/// window would, and returns the frames.
fn run_for(w: &slint::Window, ms: u64) -> Vec<SharedPixelBuffer<Rgba8Pixel>> {
    let end = Instant::now() + Duration::from_millis(ms);
    let mut frames = Vec::new();
    while Instant::now() < end {
        slint::platform::update_timers_and_animations();
        frames.push(w.take_snapshot().expect("snapshot"));
        std::thread::sleep(Duration::from_millis(4));
    }
    frames
}

/// The distinct frames among `frames`.
fn distinct(frames: &[SharedPixelBuffer<Rgba8Pixel>]) -> usize {
    let mut seen: Vec<&[Rgba8Pixel]> = Vec::new();
    for f in frames {
        if !seen.contains(&f.as_slice()) {
            seen.push(f.as_slice());
        }
    }
    seen.len()
}

/// The pixels above the bar (which shows the track and its play button).
fn content(shot: &SharedPixelBuffer<Rgba8Pixel>) -> Vec<Rgba8Pixel> {
    let w = shot.width() as usize;
    shot.as_slice()
        .chunks(w)
        .take(700)
        .flatten()
        .copied()
        .collect()
}

/// Settles, then checks nothing animates, no timer is armed, the clock holds still and no
/// frame changes.
fn assert_idle(w: &slint::Window, mark: &Mark, after: &str) {
    run_for(w, 300);
    slint::platform::update_timers_and_animations();
    w.take_snapshot().expect("snapshot");
    assert!(!w.has_active_animations(), "still animating {after}");
    let timer = slint::platform::duration_until_next_timer_update();
    assert!(timer.is_none(), "a timer is still armed {after}: {timer:?}");
    let frame = mark.get_frame();
    let frames = run_for(w, 400);
    assert_eq!(mark.get_frame(), frame, "the mark's clock stepped {after}");
    assert_eq!(distinct(&frames), 1, "the window redrew {after}");
}

/// The clock is armed and steps at its rate (30 fps, every 33 ms), and the wave visibly moves.
/// The rate is counted with timers alone: snapshots take long enough to delay the steps.
fn assert_moving(w: &slint::Window, mark: &Mark, when: &str) {
    run_for(w, 300);
    let frame = mark.get_frame();
    let end = Instant::now() + Duration::from_millis(650);
    while Instant::now() < end {
        slint::platform::update_timers_and_animations();
        std::thread::sleep(Duration::from_millis(2));
    }
    let steps = mark.get_frame() - frame;
    // 30 fps is 19 or 20 steps; 10 fps would be 6 or 7.
    assert!(
        (13..=22).contains(&steps),
        "the mark's clock stepped {steps} times in 650 ms {when}"
    );
    let frames = run_for(w, 400);
    assert!(
        distinct(&frames) >= 3,
        "the mark didn't move {when} ({} distinct frames)",
        distinct(&frames)
    );
    assert!(
        slint::platform::duration_until_next_timer_update().is_some(),
        "the mark's clock isn't armed {when}"
    );
}

/// One test: the platform can be installed once per process.
#[test]
fn the_wave_moves_only_while_playing_and_shown() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    let theme = ui.global::<Theme>();
    theme.set_motion(true);
    ui.show().expect("show");
    let w = ui.window();
    let mark = ui.global::<Mark>();
    let player = ui.global::<Player>();
    let frame = ui.global::<Frame>();

    // A playlist whose second row is the current track, nothing playing yet.
    let playlist = ui.global::<PlaylistData>();
    playlist.set_title("Playlist".into());
    playlist.set_tracks(rows());
    ui.global::<Nav>().set_page(PageKind::Playlist);
    assert_idle(w, &mark, "with nothing playing");
    let rest = content(&w.take_snapshot().expect("snapshot"));

    player.set_has_track(true);
    player.set_playing(true);
    assert_moving(w, &mark, "while playing");

    // Paused: frozen at the rest pose.
    player.set_playing(false);
    assert_idle(w, &mark, "after pausing");
    assert!(
        content(&w.take_snapshot().expect("snapshot")) == rest,
        "the paused wave isn't at its rest pose"
    );

    // Resolving or buffering before the first sound: the bar's progress line shimmers, but
    // the wave rests and its clock holds.
    player.set_playing(true);
    player.set_loading(true);
    run_for(w, 300);
    let at = mark.get_frame();
    let frames: Vec<_> = run_for(w, 400).iter().map(content).collect();
    assert_eq!(
        mark.get_frame(),
        at,
        "the mark's clock stepped while loading"
    );
    assert!(
        frames.iter().all(|f| *f == rest),
        "the wave moved while loading"
    );
    player.set_loading(false);
    assert_moving(w, &mark, "once audible");

    w.set_minimized(true);
    assert_idle(w, &mark, "while minimized");
    w.set_minimized(false);
    assert_moving(w, &mark, "after restoring");

    frame.set_hidden(true);
    assert_idle(w, &mark, "while hidden in the tray");
    frame.set_hidden(false);
    assert_moving(w, &mark, "after showing from the tray");

    // Windows' animation effects off: a still wave.
    theme.set_motion(false);
    assert_idle(w, &mark, "with reduced motion");
    theme.set_motion(true);
    assert_moving(w, &mark, "with motion back on");

    // Stopped: nothing current.
    player.set_playing(false);
    player.set_has_track(false);
    assert_idle(w, &mark, "after stopping");
}
