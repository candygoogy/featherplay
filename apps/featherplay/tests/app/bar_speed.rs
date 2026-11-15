//! The bottom bar's clock at 1.5x, on the offscreen platform (no window is shown): between
//! the controller's updates it runs 1.5 s of the song per second, so the times stay in the
//! song's time.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use featherplay::bar;
use featherplay::controller::{PlayerState, QueueInfo};
use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};

use crate::support;

/// One test: the platform can be installed once per process.
#[test]
fn at_1_5x_the_clock_runs_one_and_a_half_seconds_a_second() {
    let bare = support::bare(
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bar_speed"),
        None,
    );
    let (_ui, ctx) = (&bare.ui, &bare.ctx);

    let song = Track {
        id: TrackId("song".into()),
        title: "Song".into(),
        artists: Vec::new(),
        album: None,
        duration: Some(Duration::from_secs(200)),
        video_type: VideoType::Song,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        counterpart: None,
    };
    let from = Duration::from_secs(10);
    let state = PlayerState {
        current: Some(song),
        upcoming: Vec::new().into(),
        playing: true,
        audible: true,
        loading: false,
        position: from,
        shuffle: false,
        repeat: Default::default(),
        radio: false,
        volume: 0.7,
        queue: QueueInfo::default(),
        speed: 1.5,
    };
    let wait = Duration::from_millis(400);
    let started = Instant::now();
    bar::apply(&ctx, &state);
    std::thread::sleep(wait);
    let at = bar::position().expect("a track") - from;
    let most = started.elapsed().mul_f64(1.5);
    assert!(
        at >= wait.mul_f64(1.5) && at <= most,
        "{at:?} of the song in {wait:?} (at most {most:?})"
    );
}
