//! A new song's lyrics arrive without a long frame, at 4K 150 % with the full lyrics view open
//! (once turned on it stays on, song after song) and the side panel's Lyrics tab made once,
//! the long fixture lyrics (80 lines, some wrapping), playing. Every line of every lyrics view
//! that exists is a text to shape; all of them in the frame the lookup answered cost
//! ~15 ms there (the real window's log had 12–20 ms frames as each song's lyrics came in), so
//! `lyrics::fill` hands the lines over a few at a time. The ceiling holds in optimised builds.

use crate::lyrics_frames::{scene, step};
use featherplay::ui::{LyricsData, LyricsState, NowPlayingData};
use slint::{ComponentHandle, Model};

/// The most a frame may cost while a new song's lyrics come in, ms of CPU (optimised).
const ARRIVE_MS: f64 = 8.0;

/// The frames from a track change until its lyrics are all in and the view rests: the lookup
/// waits a second after a change (`lyrics::SKIP_WAIT`).
fn arrive(scene: &featherplay::snapshot::Scene, track: usize) -> (f64, usize) {
    let data = scene.ui.global::<LyricsData>();
    scene.play_track(false, track).expect("track");
    // The track change itself (the cover and its colours) settles first.
    for _ in 0..200 {
        step(scene);
    }
    let (mut worst, mut frames) = (0.0f64, 0);
    for _ in 0..600 {
        let f = step(scene);
        frames += 1;
        worst = worst.max(f.cpu);
        if std::env::var_os("LYRICS_ARRIVE_FRAMES").is_some() && f.cpu > 3.0 {
            println!(
                "  frame {frames}: {:.2} ms, {} px, {} lines, {:?}",
                f.cpu,
                f.damaged,
                data.get_lines().row_count(),
                data.get_state()
            );
        }
        if data.get_state() == LyricsState::Synced
            && data.get_lines().row_count() == 80
            && !scene.ui.window().has_active_animations()
            && frames > 200
        {
            break;
        }
    }
    assert_eq!(data.get_state(), LyricsState::Synced, "no lyrics came");
    (worst, frames)
}

#[test]
fn a_new_songs_lyrics_arrive_without_a_long_frame() {
    let scene = scene();
    let np = scene.ui.global::<NowPlayingData>();
    // The side panel's Lyrics tab once, then back to Up next: its view stays alive, hidden.
    np.set_tab(2);
    np.invoke_tab_selected(2);
    for _ in 0..100 {
        step(&scene);
    }
    np.set_tab(0);
    np.invoke_tab_selected(0);
    let data = scene.ui.global::<LyricsData>();
    data.invoke_toggle_view();
    for _ in 0..150 {
        step(&scene);
    }
    assert!(data.get_showing(), "the full view didn't open");
    let mut last = String::new();
    for attempt in 0..3 {
        let (worst, frames) = arrive(&scene, 1 + attempt);
        println!("lyrics arriving at 4K 150 %: worst frame {worst:.2} ms ({frames} frames)");
        if cfg!(debug_assertions) || worst <= ARRIVE_MS {
            return;
        }
        last = format!("a frame cost {worst:.2} ms while the lyrics came in, over {ARRIVE_MS}");
    }
    panic!("{last}");
}
