//! Home's band: the session (resuming or playing) or a featured pick, and the pick's colour.

use std::rc::Rc;

use featherplay_core::model::{Track, TrackId};
use featherplay_core::tr;
use featherplay_ytm::{HomeItem, HomeShelf};
use slint::ComponentHandle;

use super::STATE;
use crate::app::Ctx;
use crate::controller::{Action, PlayerState};
use crate::follow;
use crate::pages::content::ROW_COVER;
use crate::pages::now_playing;
use crate::song_menu;
use crate::ui::{HomeData, TrackRow};
use crate::ui_bridge::{home_card, home_thumbnail, model, to_rows};

/// The band's cover, in logical pixels (`BandSizes.cover`).
const HERO_COVER: u32 = 176;
/// Rows in the fresh-start band's right column (`Band`: one column of three).
const HERO_ROWS: usize = 3;
/// Rows in the band's queue: three columns of three at most (`Band`, from ~1700 px).
const HERO_NEXT: usize = 9;

/// What the band shows.
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

/// Shows the band the player state and the feed call for; cheap when nothing changed.
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
            let current = state.current.as_ref();
            let url = current.and_then(|t| t.thumbnail.clone());
            let new_track = current.is_some_and(|t| data.get_track().id != t.id.0.as_str());
            if let Some(t) = current {
                // The band's actions find it (`song_menu::find`).
                song_menu::remember(std::slice::from_ref(t));
                set_track(ctx, row_of(t));
            }
            set_hero_cover(ctx, url);
            if new_track {
                // "album · year": the year is the album's, as Now playing shows it.
                now_playing::show_year(ctx, current);
            }
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
                song_menu::remember(std::slice::from_ref(t));
                set_track(ctx, row_of(t));
                // Its Play button starts at once.
                ctx.send(Action::Prefetch {
                    id: t.id.clone(),
                    track: Some(t.clone()),
                });
            } else {
                set_track(ctx, TrackRow::default());
            }
            data.set_featured(home_card(&items[0]));
            data.set_featured_shelf(title.as_str().into());
            data.set_featured_from(tr!("More from {}", title).into());
            let url = home_thumbnail(&items[0]);
            set_hero_cover(ctx, url.clone());
            show_ink(ctx, url);
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

fn row_of(t: &Track) -> TrackRow {
    let mut row = to_rows(std::slice::from_ref(t)).pop().unwrap_or_default();
    // A like made since the track was loaded.
    if let Some(like) = song_menu::like_of(&t.id) {
        row.liked = like == featherplay_core::model::LikeStatus::Like;
    }
    row
}

/// The song the band's actions act on, with the band's cover once it's in (the song menu's
/// head shows it).
fn set_track(ctx: &Ctx, mut row: TrackRow) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<HomeData>();
    let old = data.get_track();
    if old.id == row.id {
        row.cover = old.cover.clone();
    } else {
        row.cover = data.get_hero_cover();
    }
    // Set only when it changed: the player's state comes again with every position.
    if row != old {
        data.set_track(row);
    }
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
            let data = ui.global::<HomeData>();
            data.set_hero_cover(img.clone());
            let mut track = data.get_track();
            track.cover = img;
            data.set_track(track);
        }
    });
}

/// The featured pick's panel from its cover's colour, worked out once per cover off the UI
/// thread, as the lyrics panel's is (`follow::lyric_inks_for`; graphite for grey art). Until
/// it's known the band is graphite, then it crossfades (`Band`).
fn show_ink(ctx: &Rc<Ctx>, url: Option<String>) {
    if STATE.with(|s| s.borrow().ink_url == url) {
        return;
    }
    STATE.with(|s| s.borrow_mut().ink_url = url.clone());
    paint_ink(ctx, follow::lyric_inks_for(None, None));
    let Some(url) = url else { return };
    let wanted = url.clone();
    let c = ctx.clone();
    ctx.covers.load(&url, HERO_COVER, move |img| {
        let current = |w: &str| STATE.with(|s| s.borrow().ink_url.as_deref() == Some(w));
        if !current(&wanted) {
            return;
        }
        let paint = c.clone();
        follow::cover_seed(&c, &img, move |seed| {
            if current(&wanted) {
                paint_ink(&paint, follow::lyric_inks_for(Some(seed), None));
            }
        });
    });
}

fn paint_ink(ctx: &Ctx, inks: follow::LyricInks) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<HomeData>();
    data.set_featured_ink_dark(inks.dark);
    data.set_featured_ink_light(inks.light);
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
}
