//! A card grid (Library's tabs, Search) repaints only the card whose cover came in, on the
//! offscreen platform.
//!
//! Covers reach a grid one card at a time, as they're decoded (`covers::cards`). When each
//! card read its item out of the page's whole model by index, any row set dirtied every card
//! of the grid: each of a tab's ~20 covers arriving repainted the grid and drew every card's
//! cover layer again, and Library's Playlists missed ~10 frames on its first draw (measured
//! 2026-11-08).

use crate::support::{fixture_covers, frames};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LibraryData, Theme};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, Model, PhysicalSize};

const W: u32 = 1600;
const H: u32 = 1000;

#[test]
fn a_cover_arriving_repaints_only_its_card() {
    fixture_covers();
    let scene = Scene::new(W, H, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    let w = scene.ui.window();
    scene.open("library-playlists").expect("library");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    w.set_size(PhysicalSize::new(W, H));
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(W as f32, H as f32),
    });
    scene.settle();
    frames(w, 120, |_| {});

    let cards = scene.ui.global::<LibraryData>().get_cards();
    assert!(
        cards.row_count() >= 6,
        "the fixtures have a grid of playlists"
    );
    // A card's cover changes, as the covers pacer hands one over (here it goes, so the
    // change is surely one).
    let mut row = cards.row_data(1).expect("card 1");
    assert!(row.cover.size().width > 0, "card 1 has its cover");
    row.cover = slint::Image::default();
    cards.set_row_data(1, row);
    let damaged: Vec<u64> = frames(w, 60, |_| {}).iter().map(|f| f.damaged).collect();
    let most = damaged.iter().copied().max().unwrap_or(0);

    // The card's art fades out over a few frames, each repainting the card: ~200 px square
    // with its two lines under it, well under 100,000 px. The grid on screen is a dozen
    // cards or more.
    assert!(most > 0, "the change was drawn");
    assert!(
        most < 100_000,
        "a frame repainted {most} px for one card's cover: more than its card ({damaged:?})"
    );
}
