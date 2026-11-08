//! A Home card's art shows its hover and pressed veils, on the offscreen platform (no window is
//! shown): hovering the card shades its art, pressing shades it more, and leaving clears it.
//! The veil is drawn over the cover rather than in the cover's layer (`Card`, ui/parts.slint),
//! so this holds that it still shows, and only over the art.

use crate::support::{frames, home_at};
use featherplay::offscreen;
use featherplay::ui::Theme;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Rgba8Pixel, SharedPixelBuffer};

/// The "Listen together" card's art at 1920×1360, scale 1, Up next open, light (tall enough
/// for the card's text to clear the floating bar under the resume card).
const ART: LogicalPosition = LogicalPosition::new(420.0, 1033.0);
/// Below the art, in the card's text: no veil there.
const TEXT_GAP: LogicalPosition = LogicalPosition::new(560.0, 1153.0);

fn luma(shot: &SharedPixelBuffer<Rgba8Pixel>, at: LogicalPosition) -> f64 {
    // The mean over a 9×9 patch: the art is noisy.
    let (x0, y0) = (at.x as u32 - 4, at.y as u32 - 4);
    let mut sum = 0.0;
    for y in y0..y0 + 9 {
        for x in x0..x0 + 9 {
            let p = shot.as_slice()[(y * shot.width() + x) as usize];
            sum += 0.2126 * f64::from(p.r) + 0.7152 * f64::from(p.g) + 0.0722 * f64::from(p.b);
        }
    }
    sum / 81.0
}

#[test]
fn a_card_shades_its_art_when_hovered_and_more_when_pressed() {
    let scene = home_at(1920, 1360, 1.0);
    scene.theme(false);
    scene.ui.global::<Theme>().set_motion(true);
    let w = scene.ui.window();
    // Off the cards first, then let the covers' layers warm.
    w.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(1000.0, 60.0),
    });
    let _ = frames(w, 60, |_| {});
    let shot = || offscreen::shown(w).expect("shown");
    let rest = shot();

    w.dispatch_event(WindowEvent::PointerMoved { position: ART });
    let _ = frames(w, 60, |_| {});
    let hovered = shot();

    w.dispatch_event(WindowEvent::PointerPressed {
        position: ART,
        button: PointerEventButton::Left,
    });
    let _ = frames(w, 60, |_| {});
    let pressed = shot();
    w.dispatch_event(WindowEvent::PointerReleased {
        position: ART,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(1000.0, 60.0),
    });
    let _ = frames(w, 60, |_| {});
    let left = shot();

    if let Some(dir) = std::env::var_os("CARD_VEIL_SHOTS") {
        for (name, s) in [
            ("rest", &rest),
            ("hovered", &hovered),
            ("pressed", &pressed),
        ] {
            let path = std::path::Path::new(&dir).join(format!("card-veil-{name}.png"));
            image::RgbaImage::from_raw(s.width(), s.height(), s.as_bytes().to_vec())
                .expect("image")
                .save(path)
                .expect("save");
        }
    }

    let (r, h, p, l) = (
        luma(&rest, ART),
        luma(&hovered, ART),
        luma(&pressed, ART),
        luma(&left, ART),
    );
    println!("art luma: rest {r:.1}, hovered {h:.1}, pressed {p:.1}, left {l:.1}");
    assert!(
        h < r - 2.0,
        "hovering doesn't shade the art ({r:.1} → {h:.1})"
    );
    assert!(
        p < h - 2.0,
        "pressing doesn't shade it more ({h:.1} → {p:.1})"
    );
    assert!(
        (l - r).abs() < 1.0,
        "leaving doesn't clear it ({r:.1} → {l:.1})"
    );
    assert!(
        (luma(&hovered, TEXT_GAP) - luma(&rest, TEXT_GAP)).abs() < 1.0,
        "the veil spills past the art"
    );
}
