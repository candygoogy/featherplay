//! Up next with a long queue, on the offscreen platform with covers already in the disk
//! cache (nothing is fetched): only the rows on screen and a few either side get their
//! covers; the next song edits the list in place, the rows that stay keeping their covers;
//! and a cover asked for before the list changed lands in its song's row.

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use featherplay::app::Ctx;
use featherplay::controller::{PlayerState, QueueInfo};
use featherplay::covers::sized_url;
use featherplay::offscreen::Pump;
use featherplay::ui::{AppWindow, Frame, QueueData, Theme, TrackRow};
use featherplay::{busy, prepare, queue_panel};
use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};
use sha2::{Digest, Sha256};
use slint::{ComponentHandle, Model};

use crate::support;

const WAIT: Duration = Duration::from_secs(10);
const SONGS: usize = 80;

fn url(i: usize) -> String {
    format!("https://lh3.googleusercontent.com/upnext-rows-{i}=w120-h120-l90-rj")
}

/// Song `i`'s cover: a colour of its own, so a row shows whose cover it has.
fn colour(i: usize) -> [u8; 3] {
    [(i % 9 * 28) as u8, (i / 9 * 28) as u8, 128]
}

fn track(i: usize) -> Track {
    Track {
        id: TrackId(format!("song-{i}")),
        title: format!("Song {i}"),
        artists: Vec::new(),
        album: None,
        duration: Some(Duration::from_secs(200)),
        video_type: VideoType::Song,
        thumbnail: Some(url(i)),
        like: LikeStatus::Indifferent,
        counterpart: None,
    }
}

/// `songs[0]` playing, the rest next.
fn play(ctx: &Rc<Ctx>, songs: &[usize]) {
    let tracks: Vec<Track> = songs.iter().map(|&i| track(i)).collect();
    let state = PlayerState {
        current: tracks.first().cloned(),
        upcoming: tracks[1..].into(),
        playing: false,
        audible: false,
        loading: false,
        position: Duration::ZERO,
        shuffle: false,
        repeat: Default::default(),
        radio: false,
        volume: 0.5,
        queue: QueueInfo::default(),
        speed: 1.0,
    };
    queue_panel::update(ctx, &state);
}

/// Puts song `i`'s cover in the disk cache under `cache`, as a download would have.
fn cache_cover(cache: &Path, i: usize) {
    let dir = cache.join("covers");
    std::fs::create_dir_all(&dir).expect("covers dir");
    // As `covers.rs` names it: 16 bytes of the SHA-256 of the URL asked for (a row's size).
    let asked = sized_url(&url(i), 120);
    let name: String = Sha256::digest(asked.as_bytes())
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect();
    let img = image::RgbImage::from_pixel(48, 48, image::Rgb(colour(i)));
    img.save_with_format(dir.join(format!("{name}.img")), image::ImageFormat::Png)
        .expect("cover");
}

/// The colour in the middle of row `row`'s cover, if it has one.
fn shown_colour(row: &TrackRow) -> Option<(u8, u8)> {
    let px = row.cover.to_rgba8()?;
    px.as_slice()
        .get(px.as_slice().len() / 2)
        .map(|p| (p.r, p.g))
}

fn shows(row: &TrackRow, song: usize) -> bool {
    let [r, g, _] = colour(song);
    shown_colour(row) == Some((r, g))
}

/// Draws, so the list makes its rows and says which show, then lets the covers come.
fn settle(ui: &AppWindow, pump: &Pump) {
    for _ in 0..3 {
        ui.window().request_redraw();
        ui.window().take_snapshot().expect("snapshot");
        assert!(pump.until(|| busy::pending() == 0, WAIT), "the covers came");
    }
}

#[test]
fn up_next_loads_the_covers_on_screen_and_follows_the_queue_in_place() {
    let home =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("upnext-rows-{}", std::process::id()));
    let cache = home.join("cache");
    let _ = std::fs::remove_dir_all(&cache);
    for i in 0..SONGS + 3 {
        cache_cover(&cache, i);
    }

    let bare = support::bare(cache, None);
    let (ui, ctx, pump) = (&bare.ui, &bare.ctx, &bare.pump);
    ui.window().set_size(slint::PhysicalSize::new(1280, 800));
    ui.global::<Theme>().set_still(true);
    // The rows on screen reach Up next through `ShownRows`.
    prepare::init(ctx);
    ui.global::<Frame>().set_queue_open(true);
    ui.show().expect("show");
    let data = ui.global::<QueueData>();

    // Song 0 playing, 1 to 79 next: the panel shows about ten rows.
    let all: Vec<usize> = (0..SONGS).collect();
    play(ctx, &all);
    settle(ui, pump);
    let rows = data.get_upcoming();
    assert_eq!(rows.row_count(), SONGS - 1);
    for i in 0..8 {
        let row = rows.row_data(i).expect("row");
        assert!(
            shows(&row, i + 1),
            "row {i} on screen shows its cover: {:?}",
            shown_colour(&row)
        );
    }
    let covered = rows.iter().filter(|r| shown_colour(r).is_some()).count();
    assert!(
        covered <= 30,
        "{covered} of {} rows have covers: only those on screen and near should",
        SONGS - 1
    );
    assert!(
        rows.iter().skip(40).all(|r| shown_colour(&r).is_none()),
        "a row far below the panel's end got its cover"
    );

    // The next song: the same list, its first row gone, the rest as they were.
    play(ctx, &all[1..]);
    assert!(
        data.get_upcoming() == rows,
        "the next song built the list again"
    );
    assert_eq!(rows.row_count(), SONGS - 2);
    let first = rows.row_data(0).expect("row");
    assert_eq!(first.id.as_str(), "song-2");
    assert!(shows(&first, 2), "a row that stayed lost its cover");
    settle(ui, pump);
    assert!(
        rows.iter()
            .enumerate()
            .all(|(i, r)| r.id == format!("song-{}", i + 2))
    );

    // Play next, twice in a row before any cover is in: songs 80, then 81 and 82, at the
    // front. Song 80's cover, asked for at row 0, lands at row 2, where it is by then.
    let mut queue = vec![1, SONGS];
    queue.extend(2..SONGS);
    play(ctx, &queue);
    let mut queue = vec![1, SONGS + 1, SONGS + 2, SONGS];
    queue.extend(2..SONGS);
    play(ctx, &queue);
    assert!(
        data.get_upcoming() == rows,
        "Play next built the list again"
    );
    settle(ui, pump);
    for (i, song) in [SONGS + 1, SONGS + 2, SONGS, 2, 3].into_iter().enumerate() {
        let row = rows.row_data(i).expect("row");
        assert_eq!(row.id.as_str(), format!("song-{song}"));
        assert!(
            shows(&row, song),
            "row {i} shows another song's cover: {:?}",
            shown_colour(&row)
        );
    }
    let support::Bare { ui, runtime, .. } = bare;
    drop(ui);
    runtime.shutdown_timeout(Duration::from_secs(1));
}
