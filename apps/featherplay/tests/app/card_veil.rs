//! A Home card's art shows its hover and pressed veils, on the offscreen platform (no window is
//! shown): hovering the card shades its art, pressing shades it more, and leaving clears it.
//! The veil is drawn over the cover rather than in the cover's layer (`Card`, ui/parts.slint),
//! so this holds that it still shows, and only over the art.

use crate::support::label;
use crate::support::{frames, home_at};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{HomeData, Theme};
use i_slint_core::items::AccessibleRole;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, Rgba8Pixel, SharedPixelBuffer};

/// The "Listen together" shelf's first card at 1920×1360, scale 1, Up next open, light (tall
/// enough for the card's text to clear the floating bar under the band and two rails): the
/// middle of its art, and a point on its title's line past the title's end, where no veil
/// goes. The art is the card above its title, less the gap between them.
fn card_points(scene: &Scene) -> (LogicalPosition, LogicalPosition) {
    let shelves = scene.ui.global::<HomeData>().get_shelves();
    let shelf = shelves
        .iter()
        .find(|s| s.title == "Listen together")
        .expect("a Listen together shelf");
    let title = shelf.items.row_data(0).expect("a card").title;
    let with = |role: AccessibleRole| {
        let found = label::shown_where(scene, |item, l| {
            l == title.as_str() && item.accessible_role() == role
        });
        label::rect(
            found
                .first()
                .unwrap_or_else(|| panic!("no {title:?} {role:?}")),
        )
    };
    let (card, text) = (with(AccessibleRole::Button), with(AccessibleRole::Text));
    let art_foot = text.min_y() - scene.ui.global::<Theme>().get_card_text_gap();
    (
        LogicalPosition::new(card.center().x, (card.min_y() + art_foot) / 2.0),
        LogicalPosition::new(text.max_x() - 40.0, text.center().y),
    )
}

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
    // The feed's rails under the band, where the fixtures put them.
    crate::support::no_shortcuts(&scene);
    scene.theme(false);
    scene.ui.global::<Theme>().set_motion(true);
    let w = scene.ui.window();
    let (art, text_gap) = card_points(&scene);
    // Off the cards first, then let the covers' layers warm.
    w.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(1000.0, 60.0),
    });
    let _ = frames(w, 60, |_| {});
    let shot = || offscreen::shown(w).expect("shown");
    let rest = shot();

    w.dispatch_event(WindowEvent::PointerMoved { position: art });
    let _ = frames(w, 60, |_| {});
    let hovered = shot();

    w.dispatch_event(WindowEvent::PointerPressed {
        position: art,
        button: PointerEventButton::Left,
    });
    let _ = frames(w, 60, |_| {});
    let pressed = shot();
    w.dispatch_event(WindowEvent::PointerReleased {
        position: art,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(1000.0, 60.0),
    });
    let _ = frames(w, 60, |_| {});
    let left = shot();

    for (name, s) in [
        ("rest", &rest),
        ("hovered", &hovered),
        ("pressed", &pressed),
    ] {
        crate::support::pixels::save(name, s);
    }

    let (r, h, p, l) = (
        luma(&rest, art),
        luma(&hovered, art),
        luma(&pressed, art),
        luma(&left, art),
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
        (luma(&hovered, text_gap) - luma(&rest, text_gap)).abs() < 1.0,
        "the veil spills past the art"
    );
}
