//! A cover that crossfades (the bar's, Now playing's) keeps the art it showed last under the
//! new one only while that fades in. Left there after, it was drawn in every frame the cover
//! repaints, hidden by the art over it: on Now playing after every new song, about 3 ms of a
//! 4K frame of the lyrics toggle. On the offscreen platform with the fixtures; no window is
//! shown. The test hands the page its art itself, as the cover cache does when the art is in
//! memory (art that has to be fetched fades in over the empty tile instead).

use std::ops::ControlFlow;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{NowPlayingData, Theme};
use i_slint_core::item_tree::ItemRc;
use i_slint_core::items::{ClippedImage, ImageItem};
use i_slint_core::window::WindowInner;
use slint::{ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer};

use crate::support::idle;

/// Now playing's cover is the only art this wide in the window.
const COVER_MIN: f32 = 200.0;

/// The images with art to draw that are as large as Now playing's cover; not the page's
/// picture of the cover's field and its strips.
fn images(scene: &Scene) -> usize {
    let np = scene.ui.global::<NowPlayingData>();
    let field = [
        np.get_backdrop(),
        np.get_backdrop_top(),
        np.get_backdrop_bottom(),
    ];
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut count = 0;
    root.visit_descendants(|item| {
        let plain = item
            .downcast::<ImageItem>()
            .map(|i| i.as_pin_ref().source());
        let clipped = item
            .downcast::<ClippedImage>()
            .map(|i| i.as_pin_ref().source());
        let large = item.geometry().width() >= COVER_MIN;
        let art = plain.or(clipped).filter(|art| !field.contains(art));
        if large && art.is_some_and(|art| art.size().width > 0) {
            count += 1;
        }
        ControlFlow::<()>::Continue(())
    });
    count
}

/// Art of one shade, at the size the page asks for.
fn art(shade: u8) -> Image {
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(440, 440);
    pixels.make_mut_bytes().fill(shade);
    Image::from_rgb8(pixels)
}

#[test]
fn the_art_last_shown_goes_once_the_new_art_has_faded_in() {
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let page = scene.ui.global::<NowPlayingData>();
    page.set_cover(art(40));
    idle(&scene, 120);
    assert_eq!(
        images(&scene),
        1,
        "Now playing's cover, and nothing else as large"
    );

    // New art fades in over it.
    page.set_cover(art(200));
    idle(&scene, 4);
    assert_eq!(
        images(&scene),
        2,
        "the new art didn't fade in over the last"
    );

    // The fade is 240 ms; well past it, only the new art is left.
    idle(&scene, 120);
    assert_eq!(
        images(&scene),
        1,
        "the art last shown is still under the new one after the fade"
    );
}
