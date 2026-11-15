//! The Up next panel's pinned row shows the current track's cover, also when that cover is
//! already in memory (shown in the list the track was played from, or as an upcoming row on
//! screen before the track started); and a queue played from Liked songs says so. On the
//! offscreen platform with the fixtures and generated covers: nothing is fetched.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use featherplay::controller::{Action, PlayerState, QueueAction, QueueInfo};
use featherplay::pages::playlist;
use featherplay::ui::{Frame, PlaylistData, QueueData, Theme};
use featherplay::{prepare, queue_panel};
use featherplay_controller::catalog::Source;
use featherplay_controller::fixtures::Fixtures;
use featherplay_core::model::{PlaylistId, Track};
use featherplay_store::Paths;
use sha1::{Digest, Sha1};
use slint::{ComponentHandle, Model};
use tokio::sync::mpsc;

use crate::support;

const WAIT: Duration = Duration::from_secs(10);

/// The design-covers file name the fixtures look for: 12 hex digits of the URL's SHA-1.
fn cover_name(url: &str) -> String {
    let digest = Sha1::digest(url.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}.jpg", &hex[..12])
}

/// A home whose design-covers folder has a plain cover for every radio track.
fn home_with_covers(tracks: &[Track]) -> PathBuf {
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("upnext-cover-home");
    let covers = home.join("design-covers");
    std::fs::create_dir_all(&covers).expect("covers dir");
    let img = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 60, 40]));
    for url in tracks.iter().filter_map(|t| t.thumbnail.as_deref()) {
        img.save(covers.join(cover_name(url))).expect("cover");
    }
    home
}

fn state(current: &Track, upcoming: &[Track]) -> PlayerState {
    PlayerState {
        current: Some(current.clone()),
        upcoming: upcoming.into(),
        playing: true,
        audible: false,
        loading: false,
        position: Duration::ZERO,
        shuffle: false,
        repeat: Default::default(),
        radio: false,
        volume: 0.5,
        queue: QueueInfo::default(),
        speed: 1.0,
    }
}

/// The next action the window sent, past the rows on screen it asked to resolve ahead.
fn next_sent(
    sent: &mut mpsc::UnboundedReceiver<Action>,
) -> Result<Action, mpsc::error::TryRecvError> {
    loop {
        match sent.try_recv() {
            Ok(Action::Prepare(_)) => continue,
            other => return other,
        }
    }
}

fn has_cover(img: &slint::Image) -> bool {
    img.size().width > 0
}

/// One test: the platform can be installed once per process.
#[test]
fn current_row_cover_and_liked_songs_label() {
    let tracks = Fixtures::load()
        .and_then(|f| f.radio().map_err(|e| e.to_string()))
        .expect("radio fixture");
    let tracks: Vec<Track> = tracks
        .into_iter()
        .filter(|t| t.thumbnail.is_some())
        .collect();
    assert!(tracks.len() >= 4, "the radio fixture has covers");
    let home = home_with_covers(&tracks);
    // SAFETY: set before any other thread of this test process starts.
    unsafe { std::env::set_var("FEATHERPLAY_HOME", &home) };

    let fixtures = Arc::new(Fixtures::load().expect("fixtures"));
    let mut bare = support::bare(Paths::resolve().cache, Some(fixtures.clone()));
    bare.catalog.set(Source::Fixtures(fixtures));
    let (ui, ctx, pump) = (&bare.ui, &bare.ctx, &bare.pump);
    let sent = &mut bare.sent;
    ui.global::<Theme>().set_still(true);
    let data = ui.global::<QueueData>();
    // Up next asks for the covers of the rows it shows: the panel is open, and drawn.
    prepare::init(ctx);
    ui.window().set_size(slint::PhysicalSize::new(1280, 800));
    ui.global::<Frame>().set_queue_open(true);
    ui.show().expect("show");
    let draw = || {
        ui.window().request_redraw();
        ui.window().take_snapshot().expect("snapshot");
    };

    // 1. The track's 40 px cover is in memory (the Liked list's row showed it), then it
    // starts playing.
    let loaded = Rc::new(std::cell::Cell::new(false));
    let l = loaded.clone();
    let url = tracks[1].thumbnail.clone().unwrap_or_default();
    ctx.covers.load(&url, 40, move |_| l.set(true));
    assert!(pump.until(|| loaded.get(), WAIT), "the list's cover loads");
    queue_panel::update(ctx, &state(&tracks[1], &tracks[2..3]));
    assert!(
        pump.until(|| has_cover(&data.get_current().cover), WAIT),
        "the current row shows a cover already in memory"
    );

    // 2. The next track's cover is shown in its upcoming row; then it becomes current.
    queue_panel::update(ctx, &state(&tracks[0], &tracks[2..4]));
    let upcoming_shown = || {
        draw();
        let rows = data.get_upcoming();
        rows.row_count() == 2 && rows.iter().all(|r| has_cover(&r.cover))
    };
    assert!(pump.until(upcoming_shown, WAIT), "the upcoming covers load");
    queue_panel::update(ctx, &state(&tracks[2], &tracks[3..4]));
    assert_eq!(data.get_current().id.as_str(), tracks[2].id.0);
    assert!(
        pump.until(|| has_cover(&data.get_current().cover), WAIT),
        "the current row keeps the cover its upcoming row had"
    );
    // A later state for the same track keeps it.
    queue_panel::update(ctx, &state(&tracks[2], &tracks[3..4]));
    assert!(has_cover(&data.get_current().cover));

    // 3. Played from Liked songs, the panel says so.
    playlist::init(ctx);
    playlist::open(ctx, &PlaylistId("LM".into()));
    let pdata = ui.global::<PlaylistData>();
    assert!(
        pump.until(
            || !pdata.get_loading() && pdata.get_tracks().row_count() > 0,
            WAIT
        ),
        "Liked songs loads"
    );
    // Opening the list resolves its first row ahead, and only that one.
    match next_sent(sent) {
        Ok(Action::Prefetch { id, track }) => {
            assert_eq!(id.0, pdata.get_tracks().row_data(0).unwrap().id.as_str());
            assert!(track.is_some());
        }
        other => panic!("expected the first row's prefetch, got {other:?}"),
    }
    assert!(next_sent(sent).is_err(), "one prefetch");
    pdata.invoke_play(0);
    match next_sent(sent) {
        Ok(Action::Queue(QueueAction::PlayFrom { from, start, .. })) => {
            assert_eq!(from.label, "Liked songs");
            assert_eq!(start, 0);
        }
        other => panic!("expected a labelled play, got {other:?}"),
    }
    pdata.invoke_shuffle();
    match next_sent(sent) {
        Ok(Action::Queue(QueueAction::PlayFrom { from, .. })) => {
            assert_eq!(from.label, "Liked songs")
        }
        other => panic!("expected a labelled shuffle, got {other:?}"),
    }
}
