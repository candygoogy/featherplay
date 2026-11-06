//! The bottom bar's total and scrubber for a track the catalog gave no length (a Home
//! shelf's quick pick), on the offscreen platform (no window is shown): empty while the
//! stream resolves, then the stream's length.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use featherplay::app::Ctx;
use featherplay::bar;
use featherplay::catalog::Catalog;
use featherplay::controller::{PlayerState, QueueInfo};
use featherplay::covers::Covers;
use featherplay::offscreen;
use featherplay::ui::{AppWindow, Player as Bar};
use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};
use featherplay_core::player::{Input, PlaybackPrefs, Player, Status, StreamInfo};
use slint::ComponentHandle;
use tokio::sync::mpsc;

fn state(p: &Player) -> PlayerState {
    PlayerState {
        current: p.queue().current().cloned(),
        upcoming: p.queue().upcoming().to_vec(),
        playing: p.wants_play(),
        audible: matches!(p.status(), Status::Playing),
        loading: matches!(p.status(), Status::Loading),
        position: p.position(),
        shuffle: false,
        repeat: Default::default(),
        radio: false,
        volume: 0.7,
        queue: QueueInfo::default(),
    }
}

/// One test: the platform can be installed once per process.
#[test]
fn the_total_appears_once_the_stream_resolves() {
    let _pump = offscreen::install().expect("platform");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let ui = AppWindow::new().expect("window");
    let catalog = Catalog::new(rt.handle().clone()).expect("catalog");
    let cache = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bar_total");
    let covers = Covers::new(rt.handle().clone(), cache, None);
    let (actions, _) = mpsc::unbounded_channel();
    let ctx = Ctx::new(&ui, catalog, covers, actions);
    let bar_of = || ui.global::<Bar>();

    let quick_pick = Track {
        id: TrackId("sleepless".into()),
        title: "Sleepless".into(),
        artists: Vec::new(),
        album: None,
        duration: None,
        video_type: VideoType::Song,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        song_version: None,
    };
    let mut p = Player::new(PlaybackPrefs {
        prefer_song_versions: true,
        skip_non_music_in_radio: true,
        scrobble_other_media: true,
        list_end: Default::default(),
        skip_disliked: true,
    });
    p.handle(Input::PlayTracks {
        tracks: vec![quick_pick],
        start: 0,
        radio: false,
    });
    bar::apply(&ctx, &state(&p));
    assert_eq!(bar_of().get_total().as_str(), "");
    assert_eq!(bar_of().get_duration_s(), 0.0);

    p.handle(Input::Resolved {
        id: TrackId("sleepless".into()),
        result: Ok(StreamInfo {
            expires_at: SystemTime::now() + Duration::from_secs(6 * 3600),
            duration: Some(Duration::from_secs(215)),
        }),
    });
    p.handle(Input::AudioStarted);
    bar::apply(&ctx, &state(&p));
    assert_eq!(bar_of().get_total().as_str(), "3:35");
    assert_eq!(bar_of().get_duration_s(), 215.0);
    assert_eq!(bar_of().get_elapsed().as_str(), "0:00");
}
