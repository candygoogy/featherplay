//! A page whose card the pointer rests on is read ahead (`pages/ahead.rs`): a click then
//! shows it from that answer, with no second request, and a sweep across cards reads
//! nothing. On the offscreen platform with fixture data, where each recording read stands
//! for a request (`Fixtures::reads`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::support::home_at;
use featherplay::fixtures::Fixtures;
use featherplay::offscreen;
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{AlbumData, ItemKind, PageHover, PlaylistData};
use featherplay_core::model::{AlbumId, PlaylistId};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, Model};

/// The recordings that stand for a page's requests.
const PAGES: [&str; 4] = ["album", "artist", "playlist", "playlist_mix"];

fn fixtures(scene: &Scene) -> Arc<Fixtures> {
    scene.ctx.catalog.fixtures().expect("fixtures")
}

fn page_reads(f: &Fixtures) -> usize {
    PAGES.iter().map(|name| f.reads(name)).sum()
}

/// Waits, a little at most, for `done`: a read ahead runs on the runtime.
fn wait_for(done: impl Fn() -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(2);
    while !done() {
        if Instant::now() > until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

fn rest_on(scene: &Scene, kind: ItemKind, id: &str) {
    scene
        .ui
        .global::<PageHover>()
        .invoke_rested(kind, id.into());
}

#[test]
fn a_rest_then_a_click_reads_the_album_once() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.settle();
    let f = fixtures(&scene);
    let before = f.reads("album");

    rest_on(&scene, ItemKind::Album, "MPREb_rested");
    assert!(
        wait_for(|| f.reads("album") == before + 1),
        "nothing read ahead"
    );
    scene.ctx.navigate(Page::Album {
        id: AlbumId("MPREb_rested".into()),
    });
    scene.settle();

    let data = scene.ui.global::<AlbumData>();
    assert!(!data.get_loading());
    assert_ne!(data.get_title(), "", "the album didn't show");
    assert_eq!(f.reads("album") - before, 1);
}

/// The mix recording's first page is read ahead; the click reads the rest from there.
#[test]
fn a_playlist_read_ahead_reads_only_its_later_pages_after_the_click() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.settle();
    let f = fixtures(&scene);
    let mix = "RDTMAK5uy_FIXTURE_MIX_100000000000000000000";
    let before = f.reads("playlist_mix");

    rest_on(&scene, ItemKind::Playlist, mix);
    assert!(
        wait_for(|| f.reads("playlist_mix") == before + 1),
        "nothing read ahead"
    );
    scene.ctx.navigate(Page::Playlist {
        id: PlaylistId(mix.into()),
    });
    scene.settle();

    let rows = scene.ui.global::<PlaylistData>().get_tracks().row_count();
    assert_eq!(
        rows, 8,
        "the first page's five songs and the next page's three"
    );
    // The first page read once (ahead), the later ones once.
    assert_eq!(f.reads("playlist_mix") - before, 2);
}

#[test]
fn a_rest_on_one_card_then_a_click_on_another_shows_the_one_clicked() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.settle();
    let f = fixtures(&scene);
    let (before, mix) = (f.reads("playlist"), f.reads("playlist_mix"));

    rest_on(
        &scene,
        ItemKind::Playlist,
        "RDTMAK5uy_FIXTURE_MIX_100000000000000000000",
    );
    assert!(
        wait_for(|| f.reads("playlist_mix") == mix + 1),
        "nothing read ahead"
    );
    // Until its answer is in, so it's kept when the other page opens.
    std::thread::sleep(Duration::from_millis(200));
    scene.ctx.navigate(Page::Playlist {
        id: PlaylistId("PLclicked".into()),
    });
    scene.settle();

    let data = scene.ui.global::<PlaylistData>();
    assert_eq!(data.get_id(), "PLclicked");
    assert_ne!(data.get_title(), "", "the playlist didn't show");
    assert_ne!(data.get_title(), "Archive Mix", "the card rested on showed");
    assert_eq!(f.reads("playlist") - before, 1);
}

#[test]
fn sweeping_across_cards_reads_nothing_and_resting_reads_ahead() {
    let scene = home_at(1920, 1200, 1.0);
    let w = scene.ui.window();
    let f = fixtures(&scene);
    let move_to = |x: i32, y: i32| {
        w.dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(x as f32, y as f32),
        })
    };
    let before = page_reads(&f);

    // Across every shelf at 750 px/s: 30 px every 40 ms.
    for y in (150..1200).step_by(60) {
        for x in (40..1500).step_by(30) {
            move_to(x, y);
            offscreen::advance(Duration::from_millis(40));
        }
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(page_reads(&f), before, "a sweep read a page ahead");

    // The same points, resting on each a moment.
    for y in (150..1200).step_by(60) {
        for x in (40..1500).step_by(90) {
            move_to(x, y);
            offscreen::advance(Duration::from_millis(200));
        }
    }
    assert!(
        wait_for(|| page_reads(&f) > before),
        "no rest on a card read its page ahead"
    );
}
