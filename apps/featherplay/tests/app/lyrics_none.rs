//! Lyrics on, but none to show: Now playing's stage keeps the cover view (it never turns into
//! an empty lyrics page). The Lyrics pill says "No lyrics", and a press on it leaves the stage
//! as it is, with a note under the pill saying where featherplay looked. The side panel's
//! Lyrics tab stays and shows an empty state; after a failed lookup it, and the pill, offer to
//! ask again. A song with lyrics still opens the view, and the view stays open through a new
//! song's lookup. On the offscreen platform with the fixtures and art on every cover; no
//! window is shown.

use std::cell::Cell;
use std::ops::ControlFlow;
use std::rc::Rc;

use featherplay::lyrics::{self, SnapshotLyrics};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, Nav, NowPlayingData, PageKind, Theme};
use featherplay_controller::fixtures::Fixtures;
use i_slint_core::accessibility::{AccessibilityAction, AccessibleStringProperty};
use i_slint_core::item_tree::ItemRc;
use i_slint_core::window::WindowInner;
use image::RgbaImage;
use slint::ComponentHandle;

use crate::support;

const W: u32 = 1280;
const H: u32 = 800;

fn frame(scene: &Scene) -> RgbaImage {
    let w = scene.ui.window();
    offscreen::present(w).expect("present");
    let px = offscreen::shown(w).expect("shown");
    RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()).expect("image")
}

fn far(p: [u8; 4], q: [u8; 4]) -> bool {
    (0..3).map(|i| u32::from(p[i].abs_diff(q[i]))).sum::<u32>() > 120
}

/// The first visible element whose accessible label is `label`.
fn find(scene: &Scene, label: &str) -> Option<ItemRc> {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    root.visit_descendants(|item| {
        let says = item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|l| l == label);
        if says && item.is_visible() {
            ControlFlow::Break(item.clone())
        } else {
            ControlFlow::Continue(())
        }
    })
}

fn press(scene: &Scene, label: &str) {
    find(scene, label)
        .unwrap_or_else(|| panic!("nothing visible says {label:?}"))
        .accessible_action(&AccessibilityAction::Default);
    scene.settle();
}

/// The cover on the stage, left to right: the first row well under the header where most of
/// a wide stretch differs from the stage's ground.
fn cover(scene: &Scene, img: &RgbaImage) -> (u32, u32) {
    let t = scene.ui.global::<Theme>();
    let stage = (W as f32 - t.get_nowplaying_side_w()) as u32 - 8;
    let ground = img.get_pixel(4, 300).0;
    // Below the pill and its note, which may reach over the cover's top corner.
    for y in 140..H / 2 {
        let hits: Vec<u32> = (0..stage)
            .filter(|&x| far(img.get_pixel(x, y).0, ground))
            .collect();
        if let (Some(&l), Some(&r)) = (hits.first(), hits.last())
            && r - l > 120
            && hits.len() as u32 * 2 > r - l
        {
            return (l, r);
        }
    }
    panic!("no cover on the stage");
}

/// The playing track (the radio's first) wears a light cover with a narrow red stripe, whatever
/// the machine has cached: every row of the art is mostly light, clear of the dark stage.
fn covers() {
    let f = Fixtures::load().expect("fixtures");
    let radio = f.radio().expect("radio");
    let url = radio[0].thumbnail.clone().expect("track 0's cover");
    let art = image::RgbImage::from_fn(120, 120, |x, _| {
        if (50..70).contains(&x) {
            image::Rgb([210, 40, 50])
        } else {
            image::Rgb([225, 225, 230])
        }
    });
    support::home_with_covers("lyrics-none-home", &[(&url, art)]);
}

fn scene() -> Scene {
    covers();
    support::fixture_covers();
    let scene = Scene::new(W, H, true, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    scene
}

/// The full view asked for, then the lookup ends with `kind`.
fn lyrics_on(scene: &Scene, kind: SnapshotLyrics) {
    scene.ui.global::<LyricsData>().set_full(true);
    scene.settle();
    lyrics::show_for_snapshot(&scene.ctx, kind);
    scene.settle();
}

#[test]
fn no_lyrics_keep_the_cover_view_and_the_pill_says_so() {
    let scene = scene();
    let data = scene.ui.global::<LyricsData>();
    let before = frame(&scene);
    let ground = before.get_pixel(4, 300).0;
    let (l0, r0) = cover(&scene, &before);

    lyrics_on(&scene, SnapshotLyrics::None);
    assert_eq!(data.get_state(), LyricsState::None);
    assert!(!data.get_showing(), "the full view opened on no lyrics");
    assert!(
        find(&scene, "No lyrics").is_some(),
        "the pill doesn't say so"
    );
    assert!(!data.get_hint(), "the note showed without a press");

    // The stage is as it was: its ground, and the cover large and in its place.
    let img = frame(&scene);
    assert!(
        !far(img.get_pixel(4, 300).0, ground),
        "the stage took another colour"
    );
    assert_eq!(cover(&scene, &img), (l0, r0), "the cover moved");

    // A press on the pill changes nothing but the note.
    data.set_hint(false);
    press(&scene, "No lyrics");
    assert!(!data.get_showing(), "a press opened the full view");
    assert!(data.get_full(), "a press turned lyrics off for the session");
    assert!(data.get_hint(), "a press didn't show the note");
    assert_eq!(
        cover(&scene, &frame(&scene)),
        (l0, r0),
        "a press moved the cover"
    );

    // The bar's microphone (or Ctrl+L) from another page: Now playing opens on the cover,
    // with the same note.
    scene.open("home").expect("home");
    data.set_hint(false);
    data.invoke_toggle_view();
    // Read before `settle`: the note shows for 4 s of real time, and on a loaded machine
    // settling the page change can take longer than that.
    let hint = data.get_hint();
    scene.settle();
    assert_eq!(scene.ui.global::<Nav>().get_page(), PageKind::NowPlaying);
    assert!(!data.get_showing(), "the microphone opened the full view");
    assert!(hint, "the microphone didn't show the note");
}

#[test]
fn the_lyrics_tab_stays_and_says_why() {
    let scene = scene();
    let np = scene.ui.global::<NowPlayingData>();
    np.set_tab(2);
    np.invoke_tab_selected(2);
    lyrics_on(&scene, SnapshotLyrics::None);
    assert!(
        find(
            &scene,
            "No lyrics for this song. Searched LRCLIB and YouTube Music."
        )
        .is_some(),
        "the Lyrics tab has no empty state"
    );
    assert!(
        find(&scene, "Try again").is_none(),
        "none found is an answer: nothing to try again"
    );
    assert_eq!(np.get_tab(), 2);
}

#[test]
fn a_failed_lookup_offers_try_again_in_the_tab_and_the_pill() {
    let scene = scene();
    let data = scene.ui.global::<LyricsData>();
    let np = scene.ui.global::<NowPlayingData>();
    np.set_tab(2);
    np.invoke_tab_selected(2);
    lyrics_on(&scene, SnapshotLyrics::Failed);
    assert_eq!(data.get_state(), LyricsState::Error);
    assert!(!data.get_showing(), "the full view opened on a failure");
    let asked = Rc::new(Cell::new(0));
    let a = asked.clone();
    data.on_retry(move || a.set(a.get() + 1));

    press(&scene, "Try again");
    assert_eq!(asked.get(), 1, "the tab's Try again didn't ask");
    press(&scene, "Retry lyrics");
    assert_eq!(asked.get(), 2, "the pill didn't ask again");

    // LRCLIB down: the same offer, saying so.
    lyrics_on(&scene, SnapshotLyrics::Unavailable);
    assert_eq!(data.get_message(), "Lyrics are unavailable right now");
    press(&scene, "Try again");
    assert_eq!(asked.get(), 3, "Try again didn't ask after an outage");
}

#[test]
fn a_song_with_lyrics_still_opens_the_view_and_keeps_it_through_a_lookup() {
    let scene = scene();
    let data = scene.ui.global::<LyricsData>();

    // Asked for on a song with none, the view opens once a song has some.
    lyrics_on(&scene, SnapshotLyrics::None);
    assert!(!data.get_showing());
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Synced);
    scene.settle();
    assert!(data.get_showing(), "synced lyrics didn't open the view");

    // A new song's lookup keeps it open; one that finds nothing closes it, with no note: nobody
    // pressed anything.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Loading);
    scene.settle();
    assert!(data.get_showing(), "the view closed during a lookup");
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::None);
    scene.settle();
    assert!(!data.get_showing(), "the view stayed open on no lyrics");
    assert!(!data.get_hint(), "the note showed on its own");

    // The toggle still closes and opens it on a song with lyrics.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Synced);
    scene.settle();
    data.invoke_toggle_view();
    scene.settle();
    assert!(!data.get_showing() && !data.get_full());
    data.invoke_toggle_view();
    scene.settle();
    assert!(data.get_showing());
}

#[test]
fn the_note_shows_for_a_press_and_never_on_its_own_for_a_later_song() {
    let scene = scene();
    let data = scene.ui.global::<LyricsData>();

    // Pressed while the song is still looked up: the pill waits, and the lookup that finds
    // nothing shows the note.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Loading);
    scene.settle();
    press(&scene, "Lyrics");
    assert!(
        find(&scene, "Finding lyrics…").is_some(),
        "the pill doesn't wait"
    );
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::None);
    scene.settle();
    assert!(data.get_hint(), "the press got no answer");

    // The next songs with none say "No lyrics" on the pill, with no note until it's pressed.
    data.set_hint(false);
    for _ in 0..2 {
        lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Loading);
        scene.settle();
        lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::None);
        scene.settle();
        assert!(
            !data.get_hint(),
            "the note showed on its own for a later song"
        );
        assert!(
            find(&scene, "No lyrics").is_some(),
            "the pill doesn't say so"
        );
    }
    press(&scene, "No lyrics");
    assert!(data.get_hint(), "a press didn't show the note");

    // The microphone from another page, where Now playing then looks the song up again: the
    // lookup that ends with none still answers the press.
    scene.open("home").expect("home");
    data.set_hint(false);
    data.invoke_toggle_view();
    scene.settle();
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Loading);
    scene.settle();
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::None);
    scene.settle();
    assert!(
        data.get_hint(),
        "a lookup after the press hid its note for good"
    );

    // Spent once answered: the next lookup that finds nothing shows no note.
    data.set_hint(false);
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Loading);
    scene.settle();
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::None);
    scene.settle();
    assert!(!data.get_hint(), "the press was answered twice");
}
