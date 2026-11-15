//! The bottom bar's total and scrubber for a track the catalog gave no length (a Home
//! shelf's quick pick), on the offscreen platform (no window is shown): empty while the
//! stream resolves, then the stream's length. That the player fills the length in from the
//! stream is `featherplay-core`'s test (`the_stream_length_fills_in_a_missing_duration`).

use std::path::PathBuf;
use std::time::Duration;

use featherplay::bar;
use featherplay::controller::{PlayerState, QueueInfo};
use featherplay::ui::Player as Bar;
use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};
use slint::ComponentHandle;

use crate::support;

/// The quick pick playing from its start, its length `duration` (none until it resolves).
fn playing(duration: Option<Duration>) -> PlayerState {
    PlayerState {
        current: Some(Track {
            id: TrackId("sleepless".into()),
            title: "Sleepless".into(),
            artists: Vec::new(),
            album: None,
            duration,
            video_type: VideoType::Song,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            counterpart: None,
        }),
        upcoming: Vec::new().into(),
        playing: true,
        audible: duration.is_some(),
        loading: duration.is_none(),
        position: Duration::ZERO,
        shuffle: false,
        repeat: Default::default(),
        radio: false,
        volume: 0.7,
        queue: QueueInfo::default(),
        speed: 1.0,
    }
}

/// One test: the platform can be installed once per process.
#[test]
fn the_total_appears_once_the_stream_resolves() {
    let bare = support::bare(
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bar_total"),
        None,
    );
    let (ui, ctx) = (&bare.ui, &bare.ctx);
    let bar_of = || ui.global::<Bar>();

    bar::apply(ctx, &playing(None));
    assert_eq!(bar_of().get_total().as_str(), "", "a total with no length");
    assert_eq!(bar_of().get_duration_s(), 0.0);

    bar::apply(ctx, &playing(Some(Duration::from_secs(215))));
    assert_eq!(bar_of().get_total().as_str(), "3:35");
    assert_eq!(bar_of().get_duration_s(), 215.0);
    assert_eq!(bar_of().get_elapsed().as_str(), "0:00");
}
