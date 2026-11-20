//! Now playing's picture of the cover's field: made for the page's size in physical pixels
//! and drawn from its bank of rows under both views, round the Up next card too; the lyrics
//! toggle and a playing song leave it as it is, and a playing song draws no more frames on it
//! than on the bands; a new size gets its own. On the offscreen
//! platform at 1600 x 1000 at 150 %, art on every cover, a frozen clock; no window is shown.

use featherplay::follow::backdrop::{self, Layout};
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, LyricsData, LyricsState, NowPlayingData, Theme};
use slint::ComponentHandle;

use crate::support::{self, dist, idle, label, long_lyrics, resize, scene_at, step};

const W: u32 = 1600;
const H: u32 = 1000;
const SCALE: f32 = 1.5;

/// Now playing's cover view, the long lyrics' synced lines looked up, the song playing, the
/// picture in.
fn scene() -> Scene {
    long_lyrics();
    let scene = scene_at(W, H, SCALE, "now-playing", true);
    let data = scene.ui.global::<LyricsData>();
    data.invoke_toggle_view();
    support::settle_in_frames(&scene);
    idle(&scene, 150);
    assert_eq!(data.get_state(), LyricsState::Synced);
    data.invoke_toggle_view();
    idle(&scene, 150);
    scene
}

/// The picture's bank and the page size it's for, physical pixels.
fn bank(scene: &Scene) -> (Layout, Vec<u8>) {
    let np = scene.ui.global::<NowPlayingData>();
    let (w, h) = (np.get_backdrop_w() as u32, np.get_backdrop_h() as u32);
    let pixels = np.get_backdrop().to_rgba8().expect("a picture");
    (Layout::for_page(w, h), pixels.as_bytes().to_vec())
}

/// The page as the bank draws it at `x`, `y`.
fn drawn(bank: &(Layout, Vec<u8>), x: u32, y: u32) -> [u8; 4] {
    let [r, g, b] = backdrop::at(&bank.0, &bank.1, x, y);
    [r, g, b, 255]
}

/// The picture is the page's, in both views: the stage shows it pixel for pixel, with the
/// cover's colours across it where the bands are one colour along a row; the toggle and four
/// seconds of the song playing (its lines gliding) don't make it again, and the strips the
/// lines fade into are cut from it.
#[test]
fn the_field_lies_under_both_views_and_stays() {
    let scene = scene();
    let np = scene.ui.global::<NowPlayingData>();
    assert_eq!(
        (np.get_backdrop_w() as u32, np.get_backdrop_h() as u32),
        (W, H),
        "not the page's size"
    );
    let picture = np.get_backdrop();
    let bank = bank(&scene);
    assert_eq!(
        (picture.size().width, picture.size().height),
        (bank.0.row_width(), bank.0.rows())
    );
    let side = scene.ui.global::<Theme>().get_panel_w();
    let stage = ((W as f32 / SCALE - side) * SCALE) as u32;
    let data = scene.ui.global::<LyricsData>();
    for full in [false, true] {
        if full {
            data.invoke_toggle_view();
            idle(&scene, 150);
            assert!(data.get_showing(), "the full view didn't open");
        }
        let shown = support::frame(&scene);
        // Low on the page, under the bar's fade and clear of the cover, the text and lines.
        let (left, right) = (60, stage - 60);
        for x in [left, right] {
            let (got, want) = (shown.get_pixel(x, H - 240).0, drawn(&bank, x, H - 240));
            assert!(
                dist(got, want) <= 3,
                "full={full}: {got:?} at {x}, not {want:?}"
            );
        }
        let apart = dist(drawn(&bank, left, H - 240), drawn(&bank, right, H - 240));
        assert!(apart > 6, "one colour across the stage");
        assert!(np.get_backdrop() == picture, "full={full}: made again");
    }
    for edge in [np.get_backdrop_top(), np.get_backdrop_bottom()] {
        assert!(edge.size().width > 0, "no strip");
    }
    let from = data.get_current().max(0) + 1;
    for i in 0..960 {
        if i % 240 == 0 {
            data.set_current(from + i / 240);
        }
        step(&scene);
    }
    assert!(
        np.get_backdrop() == picture,
        "made again while the song played"
    );
}

/// Frames that repainted something, and the pixels they repainted, over `n` frames in which
/// the lines move on `glides` times, from line 10. The song is paused and played again from
/// the same place first, so every clock that repaints while it plays (the playing mark's
/// steps, the bar's seconds) starts each stretch at the same point: where they fall against
/// the glides sets how many frames merge, and without this it hung on how many frames the
/// loads before took, which depends on the machine's speed.
fn playing(scene: &Scene, n: usize, glides: i32) -> (usize, u64) {
    scene.sound(false).expect("paused");
    idle(scene, 60);
    scene.sound(true).expect("playing");
    let data = scene.ui.global::<LyricsData>();
    data.set_current(10);
    idle(scene, 200);
    let from = data.get_current().max(0) + 1;
    let (mut frames, mut pixels) = (0, 0);
    for i in 0..n {
        if i % (n / glides as usize) == 0 {
            data.set_current(from + (i / (n / glides as usize)) as i32);
        }
        let f = step(scene);
        frames += usize::from(f.damaged > 0);
        pixels += f.damaged;
    }
    (frames, pixels)
}

/// While the song plays in the full view nothing more runs on the field's picture than on
/// the bands: the same frames repaint, over the same pixels, a line's glide and the playing
/// mark's steps alike, and nothing at rest. The bands are had by taking the page's picture
/// away as a hidden window does.
#[test]
fn a_playing_song_draws_no_more_frames_on_the_field() {
    let scene = scene();
    let w = scene.ui.window();
    let np = scene.ui.global::<NowPlayingData>();
    scene.ui.global::<LyricsData>().invoke_toggle_view();
    idle(&scene, 150);
    np.set_window_away(true);
    idle(&scene, 30);
    assert_eq!(np.get_backdrop().size().width, 0, "the picture stayed");
    let bands = playing(&scene, 960, 4);

    np.set_window_away(false);
    support::settle_in_frames(&scene);
    idle(&scene, 200);
    assert!(np.get_backdrop().size().width > 0, "no picture");
    assert!(!w.has_active_animations(), "something animates at rest");
    let field = playing(&scene, 960, 4);
    println!(
        "4 s playing, 4 glides: {} frames over {:.1} Mpx on the bands, {} over {:.1} Mpx on the field",
        bands.0,
        bands.1 as f64 / 1e6,
        field.0,
        field.1 as f64 / 1e6
    );
    // A frame or two of margin for the playing mark's steps (30 a second).
    assert!(
        field.0 <= bands.0 + 2,
        "{} frames repainted on the field, {} on the bands",
        field.0,
        bands.0
    );
    assert!(
        field.1 as f64 <= bands.1 as f64 * 1.01,
        "{} pixels repainted on the field, {} on the bands",
        field.1,
        bands.1
    );
}

/// A new size gets its picture: at once for one resize (a window maximised), and once the
/// sizes settle for a run of them; meanwhile the page shows the bands.
#[test]
fn a_new_size_gets_its_own_picture() {
    let scene = scene();
    let np = scene.ui.global::<NowPlayingData>();
    let w = scene.ui.window();
    resize(w, 1800, 1100);
    support::settle_in_frames(&scene);
    assert_eq!(
        (np.get_backdrop_w(), np.get_backdrop_h()),
        (1800, 1100),
        "one resize waited"
    );
    for (x, y) in [(1700, 1050), (1650, 1020), (1640, 1010)] {
        resize(w, x, y);
        scene.settle_once();
        step(&scene);
    }
    featherplay::pages::now_playing::backdrop_settled(&scene.ctx);
    support::settle_in_frames(&scene);
    assert_eq!((np.get_backdrop_w(), np.get_backdrop_h()), (1640, 1010));
}

/// Where the "Up next" tab is, physical px.
fn up_next_tab(scene: &Scene) -> (f32, f32, f32, f32) {
    let r = label::rects(scene, "Up next")
        .into_iter()
        .next()
        .expect("no Up next tab shown");
    let k = SCALE;
    (
        r.origin.x * k,
        r.origin.y * k,
        (r.origin.x + r.size.width) * k,
        (r.origin.y + r.size.height) * k,
    )
}

/// Up next is a card floating in from the window's top and right edges; round it the page
/// goes on, the cover's picture there; and the card rests on a shadow, which darkens the
/// strip right of it.
///
/// With featherplay's own title bar, as on Windows: the card starts under its row, and the
/// strip above it is clear of the shadow. Under the desktop's title bar (Linux) the card is
/// `bar-inset` from the top, and its shadow (28 px of blur) darkens the whole strip above it,
/// a few levels at the point this looks at.
#[test]
fn up_next_floats_as_a_card_on_the_field() {
    let scene = scene();
    scene.ui.global::<Frame>().set_own_bar(true);
    support::settle_in_frames(&scene);
    let theme = scene.ui.global::<Theme>();
    let inset = theme.get_bar_inset() * SCALE;
    let side = theme.get_panel_w() * SCALE;
    let card_x = W as f32 - side;
    // Above the card, clear of its shadow and of the window buttons; and right of it.
    let (gx, gy) = ((card_x + 30.0 * SCALE) as u32, (4.0 * SCALE) as u32);
    let (rx, ry) = (W - (inset / 2.0) as u32, H / 2);
    let (x0, y0, x1, _) = up_next_tab(&scene);
    assert!(
        x0 > card_x && x1 < W as f32 - inset && y0 > inset,
        "the Up next tab at {x0}..{x1}, {y0}, not on the card ({card_x}..{})",
        W as f32 - inset
    );
    let bank = bank(&scene);
    let shown = support::frame(&scene);
    let gutter = shown.get_pixel(gx, gy).0;
    let want = drawn(&bank, gx, gy);
    assert!(
        dist(gutter, want) <= 6,
        "above the card {gutter:?}, not the page's {want:?}"
    );
    // The card itself is opaque, a step apart from what's round it.
    let card = shown.get_pixel(W - inset as u32 - 4, ry).0;
    assert!(
        dist(card, gutter) > 6,
        "the card's edge {card:?} is the page's {gutter:?}"
    );
    // Its shadow: right of it the page is darker than the field there.
    let sum = |p: [u8; 4]| p[..3].iter().map(|&v| u32::from(v)).sum::<u32>();
    let (right, page) = (shown.get_pixel(rx, ry).0, drawn(&bank, rx, ry));
    assert!(
        sum(right) + 6 < sum(page),
        "no shadow right of the card: {right:?} by the page's {page:?}"
    );
}
