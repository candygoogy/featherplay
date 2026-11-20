//! "Cover behind the lyrics": the lyrics menu's switch lays the song's cover, blurred, behind
//! Now playing's full lyrics and round its Up next card, made at the page's size in physical
//! pixels, and takes it away again; the card floats with the option on and off; and with it
//! on, a playing song draws no more frames than with it off. On the
//! offscreen platform at 1600 x 1000 at 150 %, art on every cover, a frozen clock; no window
//! is shown.

use featherplay::snapshot::Scene;
use featherplay::ui::{
    Frame, LyricsData, LyricsMenu, LyricsState, NowPlayingData, SettingsData, Theme,
};
use i_slint_core::accessibility::AccessibilityAction;
use i_slint_core::items::AccessibleRole;
use slint::ComponentHandle;

use crate::support::{self, dist, idle, label, long_lyrics, scene_at, step};

const W: u32 = 1600;
const H: u32 = 1000;
const SCALE: f32 = 1.5;

/// Now playing's full view, the long lyrics' synced lines in, the song playing.
fn scene() -> Scene {
    long_lyrics();
    let scene = scene_at(W, H, SCALE, "now-playing", true);
    let data = scene.ui.global::<LyricsData>();
    data.invoke_toggle_view();
    support::settle_in_frames(&scene);
    idle(&scene, 150);
    assert_eq!(data.get_state(), LyricsState::Synced);
    assert!(data.get_showing(), "the full view didn't open");
    scene
}

/// Flips the menu's switch, as a screen reader does.
fn flip(scene: &Scene) {
    let found = label::shown_where(scene, |item, l| {
        l == "Cover behind the lyrics" && item.accessible_role() == AccessibleRole::Switch
    });
    assert_eq!(found.len(), 1, "{} switches shown", found.len());
    found[0].accessible_action(&AccessibilityAction::Default);
    scene.settle();
}

/// The picture's size in physical pixels, (0, 0) without one.
fn picture(scene: &Scene) -> (u32, u32) {
    let s = scene.ui.global::<NowPlayingData>().get_backdrop().size();
    (s.width, s.height)
}

#[test]
fn the_menus_switch_lays_the_cover_behind_the_full_lyrics() {
    let scene = scene();
    assert_eq!(picture(&scene), (0, 0), "a picture while the option is off");
    let menu = scene.ui.global::<LyricsMenu>();
    menu.set_open_asked(menu.get_open_asked() + 1);
    idle(&scene, 60);
    assert!(menu.get_open(), "the ⋯ didn't open the menu");

    flip(&scene);
    support::settle_in_frames(&scene);
    idle(&scene, 120);
    assert!(scene.ui.global::<SettingsData>().get_lyrics_blur());
    assert!(featherplay::pages::settings::lyrics_blur(), "not saved");
    // The page's size in physical pixels: the whole window, the Up next card's room too.
    let (w, h) = picture(&scene);
    assert!(w == W && h == H, "a {w}x{h} picture for {W}x{H}");
    // The stage: the window less the side panel (340 px wide at this width).
    let side = scene.ui.global::<Theme>().get_panel_w();
    let stage = ((W as f32 / SCALE - side) * SCALE) as u32;
    let np = scene.ui.global::<NowPlayingData>();
    for (edge, strip) in [
        ("top", np.get_backdrop_top()),
        ("bottom", np.get_backdrop_bottom()),
    ] {
        assert!(strip.size().width > 0, "no {edge} strip");
    }
    // It's what the stage shows: the cover's colours across it, where the plain panel (and
    // its wash, which changes only down the stage) is one colour along a row.
    let shown = support::frame(&scene);
    let (left, right) = (
        shown.get_pixel(60, H - 240),
        shown.get_pixel(stage - 60, H - 240),
    );
    let apart = (0..3)
        .map(|k| left[k].abs_diff(right[k]))
        .max()
        .unwrap_or(0);
    assert!(
        apart > 6,
        "one colour across the stage: {left:?} and {right:?}"
    );

    // And off again: the picture goes, the panel's colour comes back.
    flip(&scene);
    idle(&scene, 30);
    assert!(!featherplay::pages::settings::lyrics_blur());
    assert_eq!(picture(&scene), (0, 0), "the picture stayed");
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

/// While the song plays nothing more runs with the picture: the same frames repaint, over the
/// same pixels, a line's glide and the playing mark's steps alike, and nothing at rest.
#[test]
fn a_playing_song_draws_no_more_frames_with_the_cover_behind() {
    let scene = scene();
    let w = scene.ui.window();
    // The same stretch of the song for each, from the same line.
    let off = playing(&scene, 960, 4);

    scene.ui.global::<SettingsData>().set_lyrics_blur(true);
    support::settle_in_frames(&scene);
    idle(&scene, 200);
    assert!(picture(&scene).0 > 0, "no picture");
    assert!(!w.has_active_animations(), "something animates at rest");
    let on = playing(&scene, 960, 4);
    println!(
        "4 s playing, 4 glides: {} frames over {:.1} Mpx without the cover, {} over {:.1} Mpx with it",
        off.0,
        off.1 as f64 / 1e6,
        on.0,
        on.1 as f64 / 1e6
    );
    // A frame or two of margin for the playing mark's steps (30 a second).
    assert!(
        on.0 <= off.0 + 2,
        "{} frames repainted with the cover, {} without",
        on.0,
        off.0
    );
    assert!(
        on.1 as f64 <= off.1 as f64 * 1.01,
        "{} pixels repainted with the cover, {} without",
        on.1,
        off.1
    );
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

/// Up next is a card floating in from the window's top and right edges, with the option off
/// and on; round it the page goes on: the field's bands (the stage's at the same height) with
/// the option off, the cover's picture with it on; and the card rests on a shadow, which
/// darkens the strip right of it.
///
/// With featherplay's own title bar, as on Windows: the card starts under its row, and the
/// strip above it is clear of the shadow. Under the desktop's title bar (Linux) the card is
/// `bar-inset` from the top, and its shadow (28 px of blur) darkens the whole strip above it,
/// a few levels at the point this looks at.
#[test]
fn up_next_floats_as_a_card_with_the_cover_behind_and_without() {
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
    for on in [false, true] {
        if on {
            scene.ui.global::<SettingsData>().set_lyrics_blur(true);
            support::settle_in_frames(&scene);
            idle(&scene, 120);
            assert!(picture(&scene).0 > 0, "no picture");
        }
        let (x0, y0, x1, _) = up_next_tab(&scene);
        assert!(
            x0 > card_x && x1 < W as f32 - inset && y0 > inset,
            "option {on}: the Up next tab at {x0}..{x1}, {y0}, not on the card ({card_x}..{})",
            W as f32 - inset
        );
        let shown = support::frame(&scene);
        let gutter = shown.get_pixel(gx, gy).0;
        let want = if on {
            let p = scene.ui.global::<NowPlayingData>().get_backdrop();
            let p = p.to_rgba8().expect("pixels");
            let at = (gy * p.width() + gx) as usize;
            let px = p.as_slice()[at];
            // The picture's bytes are BGRA, the window's order (`follow::backdrop`).
            [px.b, px.g, px.r, px.a]
        } else {
            shown.get_pixel(4, gy).0
        };
        assert!(
            dist(gutter, want) <= 6,
            "option {on}: above the card {gutter:?}, not the page's {want:?}"
        );
        // The card itself is opaque, a step apart from what's round it.
        let card = shown.get_pixel(W - inset as u32 - 4, ry).0;
        assert!(
            dist(card, gutter) > 6,
            "option {on}: the card's edge {card:?} is the page's {gutter:?}"
        );
        // Its shadow: right of it the page is darker than the stage's left edge there.
        let (right, page) = (shown.get_pixel(rx, ry).0, shown.get_pixel(4, ry).0);
        if !on {
            let sum = |p: [u8; 4]| p[..3].iter().map(|&v| u32::from(v)).sum::<u32>();
            assert!(
                sum(right) + 6 < sum(page),
                "no shadow right of the card: {right:?} by the page's {page:?}"
            );
        }
    }
}
