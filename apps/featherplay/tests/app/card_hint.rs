//! A song card on Home resolves ahead when the pointer rests on it, as a song row does
//! (`HoverIntent` in `Card`, ui/parts.slint), with its track, so the stream resolved is the
//! one Play asks for; album, playlist and artist cards don't. On the offscreen platform.

use std::collections::HashSet;
use std::time::Duration;

use crate::support::home_at;
use featherplay::controller::Action;
use featherplay::offscreen;
use featherplay::ui::{HomeData, ItemKind, ShelfShape};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, Model};

#[test]
fn resting_on_a_song_card_resolves_it_ahead() {
    let scene = home_at(1920, 1200, 1.0);
    let w = scene.ui.window();
    let (mut songs, mut others) = (HashSet::new(), HashSet::new());
    for shelf in scene.ui.global::<HomeData>().get_shelves().iter() {
        if shelf.shape == ShelfShape::Songs {
            continue; // Quick picks are rows.
        }
        for card in shelf.items.iter() {
            let id = card.id.to_string();
            if card.kind == ItemKind::Song {
                songs.insert(id);
            } else {
                others.insert(id);
            }
        }
    }
    assert!(!songs.is_empty(), "the fixtures have song cards");
    let _ = scene.take_actions();
    // Rest on points across the page, each long enough for a hint (120 ms).
    let mut hinted = Vec::new();
    for y in (150..1200).step_by(60) {
        for x in (40..1500).step_by(90) {
            w.dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(x as f32, y as f32),
            });
            offscreen::advance(Duration::from_millis(150));
            for a in scene.take_actions() {
                if let Action::Prefetch { id, track } = a {
                    hinted.push((id.0, track.is_some()));
                }
            }
        }
    }
    let cards: Vec<_> = hinted.iter().filter(|(id, _)| songs.contains(id)).collect();
    assert!(!cards.is_empty(), "no song card was hinted: {hinted:?}");
    assert!(
        cards.iter().all(|(_, track)| *track),
        "a song card's hint came without its track: {cards:?}"
    );
    assert!(
        !hinted.iter().any(|(id, _)| others.contains(id)),
        "an album, playlist or artist card was hinted: {hinted:?}"
    );
}
