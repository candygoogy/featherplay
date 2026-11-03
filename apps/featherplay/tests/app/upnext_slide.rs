//! The Up next panel slides over the page, and the page is laid out at its new width once:
//! opening, it keeps its width until the slide ends, so nothing shows empty beside the
//! sliding panel; closing, it takes its full width at once, under the panel sliding away.
//! Through the renderer's own frames (`offscreen::install_presenting`); no window is shown.
//!
//! With `UPNEXT_SHOTS=<dir>`, the frames halfway through the opening and the closing slide
//! are written there as PNGs.

use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, Theme};
use slint::{ComponentHandle, Window};

/// One frame at 60 Hz.
const FRAME: Duration = Duration::from_millis(16);

fn frame(w: &Window) {
    offscreen::advance(FRAME);
    offscreen::present(w).expect("present");
}

/// The frames of a slide, each with the page's width (logical px) as it was drawn and whether
/// the slide was still running after it; halfway through, the shown frame is saved as `name`
/// if `UPNEXT_SHOTS` is set.
fn slide(scene: &Scene, name: &str) -> Vec<(f32, bool)> {
    let w = scene.ui.window();
    let mut widths = Vec::new();
    loop {
        frame(w);
        widths.push((scene.ui.get_page_width(), w.has_active_animations()));
        if widths.len() == 4
            && let Some(dir) = std::env::var_os("UPNEXT_SHOTS")
        {
            let shot = offscreen::shown(w).expect("shown");
            let path = std::path::Path::new(&dir).join(format!("{name}.png"));
            image::save_buffer(
                &path,
                shot.as_bytes(),
                shot.width(),
                shot.height(),
                image::ExtendedColorType::Rgba8,
            )
            .expect("png");
        }
        if !widths[widths.len() - 1].1 || widths.len() > 120 {
            break;
        }
    }
    // A frame or two for what follows the slide's end.
    for _ in 0..2 {
        frame(w);
        widths.push((scene.ui.get_page_width(), w.has_active_animations()));
    }
    widths
}

/// One test: the platform can be installed once per process.
#[test]
fn the_page_narrows_once_the_panel_is_in() {
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let w = scene.ui.window();
    for _ in 0..10 {
        frame(w);
    }
    let full = scene.ui.get_page_width();

    scene.ui.global::<Frame>().set_queue_open(true);
    let opening = slide(&scene, "up-next-opening");
    assert!(
        opening.len() > 6,
        "the panel didn't slide: the test lost its point"
    );
    assert!(
        opening
            .iter()
            .filter(|(_, sliding)| *sliding)
            .all(|&(p, _)| p == full),
        "the page's width changed while the panel slid in: {opening:?}"
    );
    let narrow = opening[opening.len() - 1].0;
    assert!(
        narrow < full,
        "the page didn't make room for the panel: {narrow} of {full}"
    );

    scene.ui.global::<Frame>().set_queue_open(false);
    let closing = slide(&scene, "up-next-closing");
    assert!(
        closing.iter().all(|&(p, _)| p == full),
        "the page didn't take its full width at once as the panel slid away: {closing:?}"
    );
}
