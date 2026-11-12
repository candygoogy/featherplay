//! Home's hero: the resume card or the featured pick, and the header's wash.

use std::rc::Rc;

use featherplay_core::model::{Track, TrackId};
use featherplay_core::tr;
use featherplay_ytm::{HomeItem, HomeShelf};
use slint::ComponentHandle;

use super::STATE;
use crate::app::Ctx;
use crate::controller::{Action, PlayerState};
use crate::follow::{
    self,
    colour::{self, Lch, Rgb},
};
use crate::pages::content::ROW_COVER;
use crate::song_menu;
use crate::ui::HomeData;
use crate::ui_bridge::{home_card, home_thumbnail, model, to_rows};

/// The hero's cover, in logical pixels (`Theme.hero-cover`).
const HERO_COVER: u32 = 152;
/// Rows in the fresh-start hero's right column (two rows of two).
const HERO_ROWS: usize = 4;
/// Rows in the resume card's queue column, at most (`ResumeCard`).
const HERO_NEXT: usize = 3;

/// What the hero shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hero {
    /// The current track and what's next in the queue.
    Resume,
    /// The first item of `shelves[shelf]`, with the next few from it.
    Featured { shelf: usize },
    /// No session and no feed (yet).
    Empty,
}

/// Resume when the player has a track (a restored session or a queue); otherwise feature
/// the first shelf that has items.
pub fn choose_hero(state: Option<&PlayerState>, shelves: &[HomeShelf]) -> Hero {
    if state.is_some_and(|s| s.current.is_some()) {
        return Hero::Resume;
    }
    match shelves.iter().position(|s| !s.items.is_empty()) {
        Some(shelf) => Hero::Featured { shelf },
        None => Hero::Empty,
    }
}

/// Shows the hero the player state and the feed call for; cheap when nothing changed.
pub(super) fn show_hero(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<HomeData>();
    let state = ctx.player();
    let hero = STATE.with(|s| choose_hero(state.as_ref(), &s.borrow().shelves));
    let changed = STATE.with(|s| s.borrow().hero.as_ref() != Some(&hero));
    data.set_resume(hero == Hero::Resume);
    data.set_has_featured(matches!(hero, Hero::Featured { .. }));

    match &hero {
        Hero::Resume => {
            let Some(state) = state else { return };
            let url = state.current.as_ref().and_then(|t| t.thumbnail.clone());
            set_hero_cover(ctx, url.clone());
            show_wash(ctx, url);
            let next: Vec<Track> = state.upcoming.iter().take(HERO_NEXT).cloned().collect();
            let ids: Vec<TrackId> = next.iter().map(|t| t.id.clone()).collect();
            if changed || STATE.with(|s| s.borrow().next != ids) {
                song_menu::remember(&next);
                let urls = next.iter().map(|t| t.thumbnail.clone()).collect();
                let (rows, model) = model(to_rows(&next));
                data.set_next(model);
                ctx.covers
                    .fill(&rows, urls, ROW_COVER, |row, img| row.cover = img);
                STATE.with(|s| s.borrow_mut().next = ids);
            }
        }
        Hero::Featured { shelf } if changed => {
            let items: Vec<HomeItem> = STATE.with(|s| {
                s.borrow().shelves[*shelf]
                    .items
                    .iter()
                    .take(HERO_ROWS + 1)
                    .cloned()
                    .collect()
            });
            let title = STATE.with(|s| s.borrow().shelves[*shelf].title.clone());
            if let HomeItem::Track(t) = &items[0] {
                // Its Play button starts at once.
                ctx.send(Action::Prefetch {
                    id: t.id.clone(),
                    track: Some(t.clone()),
                });
            }
            data.set_featured(home_card(&items[0]));
            data.set_featured_from(tr!("More from {}", title).into());
            set_hero_cover(ctx, home_thumbnail(&items[0]));
            let more = &items[1..];
            let (rows, model) = model(more.iter().map(home_card).collect());
            data.set_featured_more(model);
            let urls = more.iter().map(home_thumbnail).collect();
            ctx.covers
                .fill(&rows, urls, ROW_COVER, |row, img| row.cover = img);
            STATE.with(|s| s.borrow_mut().next.clear());
        }
        _ => {}
    }
    STATE.with(|s| s.borrow_mut().hero = Some(hero));
}

fn set_hero_cover(ctx: &Ctx, url: Option<String>) {
    let Some(ui) = ctx.window() else { return };
    if STATE.with(|s| s.borrow().hero_url == url) {
        return;
    }
    STATE.with(|s| s.borrow_mut().hero_url = url.clone());
    ui.global::<HomeData>().set_hero_cover(Default::default());
    let Some(url) = url else { return };
    let weak = ui.as_weak();
    let wanted = url.clone();
    ctx.covers.load(&url, HERO_COVER, move |img| {
        let still = STATE.with(|s| s.borrow().hero_url.as_deref() == Some(wanted.as_str()));
        if still && let Some(ui) = weak.upgrade() {
            ui.global::<HomeData>().set_hero_cover(img);
        }
    });
}

/// The header's wash from the resumed song's cover: its colour is worked out once per cover,
/// off the UI thread. Until the new one is known the last one stays, so a track change
/// crossfades colour to colour (`HomePage.wash`).
fn show_wash(ctx: &Rc<Ctx>, url: Option<String>) {
    if STATE.with(|s| s.borrow().wash_url != url) {
        STATE.with(|s| s.borrow_mut().wash_url = url.clone());
        let Some(url) = url else {
            STATE.with(|s| s.borrow_mut().wash_seed = None);
            if let Some(ui) = ctx.window() {
                let data = ui.global::<HomeData>();
                data.set_wash_dark(Default::default());
                data.set_wash_light(Default::default());
            }
            return;
        };
        let wanted = url.clone();
        let c = ctx.clone();
        ctx.covers.load(&url, HERO_COVER, move |img| {
            let current = |w: &str| STATE.with(|s| s.borrow().wash_url.as_deref() == Some(w));
            if !current(&wanted) {
                return;
            }
            let paint = c.clone();
            follow::cover_seed(&c, &img, move |seed| {
                if current(&wanted) {
                    STATE.with(|s| s.borrow_mut().wash_seed = Some(seed));
                    paint_wash(&paint);
                }
            });
        });
    }
    paint_wash(ctx);
}

/// Sets the wash for the known cover colour.
fn paint_wash(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let Some(seed) = STATE.with(|s| s.borrow().wash_seed) else {
        return;
    };
    let washes = follow::washes_on_ground(hero_washes(seed));
    let data = ui.global::<HomeData>();
    data.set_wash_dark(washes.dark);
    data.set_wash_light(washes.light);
}

/// The header wash's two shades for the resumed song's cover: its own colour, grey for
/// grey art (below `NEUTRAL_CHROMA`), as Now playing's wash and the neutral accent.
fn hero_washes(seed: Lch) -> (Rgb, Rgb) {
    colour::washes(seed)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use featherplay_core::model::RepeatMode;

    use super::super::test_items::{album, shelf, track};
    use super::*;

    fn state(current: Option<Track>, upcoming: Vec<Track>) -> PlayerState {
        PlayerState {
            current,
            upcoming,
            playing: false,
            audible: false,
            loading: false,
            position: Duration::ZERO,
            shuffle: false,
            repeat: RepeatMode::Off,
            radio: false,
            volume: 1.0,
            queue: Default::default(),
            speed: 1.0,
        }
    }

    #[test]
    fn a_queue_resumes() {
        let shelves = [shelf("Quick picks", vec![HomeItem::Track(track("p"))])];
        let s = state(Some(track("a")), vec![track("b")]);
        assert_eq!(choose_hero(Some(&s), &shelves), Hero::Resume);
        // A restored session without anything after it still resumes.
        let s = state(Some(track("a")), Vec::new());
        assert_eq!(choose_hero(Some(&s), &[]), Hero::Resume);
    }

    #[test]
    fn an_empty_queue_features_the_first_shelf_with_items() {
        let shelves = [
            shelf("Empty", Vec::new()),
            shelf("Quick picks", vec![HomeItem::Track(track("p")), album()]),
            shelf("Albums for you", vec![album()]),
        ];
        let empty = state(None, Vec::new());
        assert_eq!(
            choose_hero(Some(&empty), &shelves),
            Hero::Featured { shelf: 1 }
        );
        assert_eq!(choose_hero(None, &shelves), Hero::Featured { shelf: 1 });
    }

    #[test]
    fn nothing_to_show_before_the_feed_arrives() {
        assert_eq!(choose_hero(None, &[]), Hero::Empty);
        assert_eq!(
            choose_hero(Some(&state(None, Vec::new())), &[shelf("x", Vec::new())]),
            Hero::Empty
        );
    }

    #[test]
    fn the_header_washes_in_a_colourful_covers_own_colour() {
        let seed = Lch {
            l: 0.6,
            c: 0.14,
            h: 1.0,
        };
        assert_eq!(hero_washes(seed), colour::washes(seed));
    }

    #[test]
    fn grey_art_washes_the_header_grey() {
        let grey = Lch {
            l: 0.5,
            c: 0.02,
            h: 0.3,
        };
        let (dark, light) = hero_washes(grey);
        for shade in [dark, light] {
            assert!(colour::to_lch(shade).c < 0.005, "a tinted wash: {shade:?}");
        }
    }
}
