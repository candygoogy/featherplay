//! Home's Now playing card (the band): the session (resuming or playing) or a featured pick,
//! and the pick's colour.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use featherplay_core::model::{ArtistId, LikeStatus, Track, TrackId};
use featherplay_ytm::{HomeItem, HomeShelf};
use slint::ComponentHandle;

use super::STATE;
use crate::app::Ctx;
use crate::controller::{Action, PlayerState};
use crate::follow;
use crate::pages::content::ROW_COVER;
use crate::pages::now_playing;
use crate::song_menu;
use crate::ui::{Frame, HomeData, TrackRow};
use crate::ui_bridge::{home_card, home_thumbnail, model, to_rows};

/// The card's cover, in logical pixels: its art, edge to edge (`BandSizes.height`).
const HERO_COVER: u32 = 224;
/// Rows in the card's Up next column (`Band`: one column of three; its "Next" line shows the
/// first).
const HERO_NEXT: usize = 3;
/// A queue shorter than the column is filled with "More like this".
const HERO_COLUMN: usize = HERO_NEXT;
/// The artist's round photo beside the artists, in logical pixels.
const ARTIST_PHOTO: u32 = 22;

thread_local! {
    /// The artists' photos asked for this session: the photo's address once the artist's
    /// page answered, `None` while it's asked for, or for an artist without one (or whose page
    /// failed). Each artist is asked for once.
    static PHOTOS: RefCell<HashMap<ArtistId, Option<String>>> = RefCell::new(HashMap::new());
}

/// What the band shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hero {
    /// The current track and what's next in the queue.
    Resume,
    /// The first item of `shelves[shelf]`.
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
    ctx.with_player(|s| show_hero_for(ctx, s));
}

/// `show_hero` for the player's `state` as it comes (`player_changed`), without a copy of it:
/// it comes with every position.
pub(super) fn show_hero_for(ctx: &Rc<Ctx>, state: Option<&PlayerState>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<HomeData>();
    let hero = STATE.with(|s| choose_hero(state, &s.borrow().shelves));
    let changed = STATE.with(|s| s.borrow().hero.as_ref() != Some(&hero));
    data.set_resume(hero == Hero::Resume);
    data.set_has_featured(matches!(hero, Hero::Featured { .. }));

    match &hero {
        Hero::Resume => {
            let Some(state) = state else { return };
            let current = state.current.as_ref();
            let new_track = current.is_some_and(|t| data.get_track().id != t.id.0.as_str());
            if let Some(t) = current {
                show_current(ctx, t, changed);
            }
            set_hero_cover(ctx, current.and_then(|t| t.thumbnail.as_deref()));
            show_artist_photo(ctx, current);
            if new_track {
                // "album · year": the year is the album's, as Now playing shows it.
                now_playing::show_year(ctx, current);
            }
            // The player's state comes with every position: the same songs next copy nothing.
            let upcoming = || state.upcoming.iter().take(HERO_NEXT);
            let same = STATE.with(|s| s.borrow().next.iter().eq(upcoming().map(|t| &t.id)));
            if changed || !same {
                let next: Vec<Track> = upcoming().cloned().collect();
                let ids: Vec<TrackId> = next.iter().map(|t| t.id.clone()).collect();
                song_menu::remember(&next);
                let urls = next.iter().map(|t| t.thumbnail.clone()).collect();
                let (rows, model) = model(to_rows(&next));
                data.set_next(model);
                ctx.covers
                    .fill(&rows, urls, ROW_COVER, |row, img| row.cover = img);
                STATE.with(|s| s.borrow_mut().next = ids);
            }
            want_related(ctx, state);
            show_more_like_this(ctx, state);
        }
        Hero::Featured { shelf } if changed => {
            let (pick, title) = STATE.with(|s| {
                let s = s.borrow();
                let shelf = &s.shelves[*shelf];
                (shelf.items[0].clone(), shelf.title.clone())
            });
            if let HomeItem::Track(t) = &pick {
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
            data.set_featured(home_card(&pick));
            data.set_featured_shelf(title.as_str().into());
            let url = home_thumbnail(&pick);
            set_hero_cover(ctx, url.as_deref());
            show_ink(ctx, url);
            STATE.with(|s| s.borrow_mut().next.clear());
            clear_more_like_this(ctx);
        }
        _ => {}
    }
    STATE.with(|s| s.borrow_mut().hero = Some(hero));
}

/// On Home, a short queue's "More like this" is asked for at once rather than when the
/// Related tab opens (once a song), while something on screen would show it: the band's
/// queue column, or the Up next panel or sheet.
fn want_related(ctx: &Rc<Ctx>, state: &PlayerState) {
    let Some(ui) = ctx.window() else { return };
    if ctx.current_page() == crate::router::Page::Home {
        let column = STATE.with(|s| s.borrow().band_column);
        let shown = column || ui.global::<Frame>().get_tabs_open();
        now_playing::want_related(ctx, state, shown);
    }
}

/// The band got room for its queue column or lost it, or the Up next panel or sheet opened
/// or closed (`HomeData.band-column`): a short queue's "More like this" is asked for if it
/// shows now.
pub(super) fn band_column(ctx: &Rc<Ctx>, room: bool) {
    STATE.with(|s| s.borrow_mut().band_column = room);
    ctx.with_player(|s| {
        if let Some(state) = s {
            want_related(ctx, state);
        }
    });
}

/// Which of the song's related tracks fill a queue shorter than a column, as indexes into
/// `related`: the first ones that aren't the song or already next, as many as the queue
/// leaves rows for.
fn more_like_this(current: &TrackId, upcoming: &[Track], related: &[Track]) -> Vec<usize> {
    let room = HERO_COLUMN.saturating_sub(upcoming.len());
    related
        .iter()
        .enumerate()
        .filter(|(_, t)| &t.id != current && !upcoming.iter().any(|u| u.id == t.id))
        .map(|(i, _)| i)
        .take(room)
        .collect()
}

/// "More like this" under a short queue, from the song's Related tab when it has been loaded
/// (`now_playing::related_of`; nothing is asked for here). Rebuilt only when what it shows
/// changes.
fn show_more_like_this(ctx: &Ctx, state: &PlayerState) {
    let Some(current) = &state.current else {
        clear_more_like_this(ctx);
        return;
    };
    // Read in place, and copied only when the rows change: the player's state comes with
    // every position.
    let changed = now_playing::with_related(&current.id, |related| {
        let at = more_like_this(&current.id, &state.upcoming, related);
        let same = STATE.with(|s| {
            let s = s.borrow();
            s.related.len() == at.len()
                && at
                    .iter()
                    .zip(&s.related)
                    .all(|(&i, id)| related[i].id == *id)
        });
        (!same).then(|| {
            let shown: Vec<Track> = at.iter().map(|&i| related[i].clone()).collect();
            (at, shown)
        })
    });
    let Some((at, shown)) = changed else { return };
    let ids: Vec<TrackId> = shown.iter().map(|t| t.id.clone()).collect();
    let Some(ui) = ctx.window() else { return };
    song_menu::remember(&shown);
    let urls = shown.iter().map(|t| t.thumbnail.clone()).collect();
    let (rows, model) = model(to_rows(&shown));
    ui.global::<HomeData>().set_related(model);
    ctx.covers
        .fill(&rows, urls, ROW_COVER, |row, img| row.cover = img);
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.related = ids;
        s.related_at = at;
    });
}

fn clear_more_like_this(ctx: &Ctx) {
    if STATE.with(|s| s.borrow().related.is_empty()) {
        return;
    }
    if let Some(ui) = ctx.window() {
        ui.global::<HomeData>().set_related(Default::default());
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.related.clear();
        s.related_at.clear();
    });
}

/// The current song for the band's actions (`song_menu::find`) and its row (`set_track`),
/// made again only when the song or its like changed, or the band did (`changed`), as the
/// player's state comes with every position. A song the song menu no longer knows (its
/// memory was full) is remembered again.
fn show_current(ctx: &Ctx, t: &Track, changed: bool) {
    let like = song_menu::like_of(&t.id);
    let same = !changed
        && like.is_some()
        && STATE.with(|s| {
            s.borrow()
                .band_track
                .as_ref()
                .is_some_and(|(shown, was)| shown == t && *was == like)
        });
    if same {
        return;
    }
    song_menu::remember(std::slice::from_ref(t));
    set_track(ctx, row_of(t));
    let like = song_menu::like_of(&t.id);
    STATE.with(|s| s.borrow_mut().band_track = Some((t.clone(), like)));
}

fn row_of(t: &Track) -> TrackRow {
    let mut row = to_rows(std::slice::from_ref(t)).pop().unwrap_or_default();
    // A like made since the track was loaded.
    if let Some(like) = song_menu::like_of(&t.id) {
        row.liked = like == LikeStatus::Like;
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

fn set_hero_cover(ctx: &Ctx, url: Option<&str>) {
    let Some(ui) = ctx.window() else { return };
    if STATE.with(|s| s.borrow().hero_url.as_deref() == url) {
        return;
    }
    STATE.with(|s| s.borrow_mut().hero_url = url.map(str::to_string));
    ui.global::<HomeData>().set_hero_cover(Default::default());
    let Some(url) = url else { return };
    let weak = ui.as_weak();
    let wanted = url.to_string();
    ctx.covers.load(url, HERO_COVER, move |img| {
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

/// The first artist's round photo for the card (`HomeData.artist-photo`), and its initial
/// meanwhile. Called only while Home shows: the artist's page is asked for once per artist
/// this session, for its photo; a song by an artist already asked for asks nothing.
fn show_artist_photo(ctx: &Rc<Ctx>, track: Option<&Track>) {
    let Some(ui) = ctx.window() else { return };
    let first = track.and_then(|t| t.artists.first());
    let id = first.and_then(|a| a.id.as_ref());
    let name = first.map_or("", |a| a.name.as_str());
    // The player's state comes with every position: the same artist again copies nothing.
    let same = STATE.with(|s| {
        s.borrow()
            .photo_for
            .as_ref()
            .is_some_and(|(was, was_name)| was.as_ref() == id && was_name == name)
    });
    if same {
        return;
    }
    let id = id.cloned();
    STATE.with(|s| s.borrow_mut().photo_for = Some((id.clone(), name.to_string())));
    let data = ui.global::<HomeData>();
    data.set_artist_photo(Default::default());
    let initial: String = first
        .and_then(|a| a.name.chars().find(|c| c.is_alphanumeric()))
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    data.set_artist_initial(initial.into());
    let Some(id) = id else { return };
    match PHOTOS.with(|p| p.borrow().get(&id).cloned()) {
        Some(Some(url)) => load_artist_photo(ctx, &id, &url),
        // Asked for already: it comes, or there's none.
        Some(None) => {}
        None => {
            PHOTOS.with(|p| p.borrow_mut().insert(id.clone(), None));
            let asked = id.clone();
            ctx.load(
                move |source| async move { source.artist(&asked).await },
                move |ctx, _, result| {
                    let url = match result {
                        Ok(a) => a.summary.thumbnail,
                        Err(e) => {
                            log!("home: artist {}: {e}", id.0);
                            None
                        }
                    };
                    PHOTOS.with(|p| p.borrow_mut().insert(id.clone(), url.clone()));
                    if let Some(url) = url {
                        load_artist_photo(ctx, &id, &url);
                    }
                },
            );
        }
    }
}

/// Decodes the photo at `url` for the card, if `id` is still the card's artist then.
fn load_artist_photo(ctx: &Ctx, id: &ArtistId, url: &str) {
    let Some(ui) = ctx.window() else { return };
    let weak = ui.as_weak();
    let wanted = id.clone();
    ctx.covers.load(url, ARTIST_PHOTO, move |img| {
        let still = STATE.with(|s| {
            s.borrow()
                .photo_for
                .as_ref()
                .is_some_and(|(id, _)| id.as_ref() == Some(&wanted))
        });
        if still && let Some(ui) = weak.upgrade() {
            ui.global::<HomeData>().set_artist_photo(img);
        }
    });
}

/// The featured pick's band from its cover's colour, worked out once per cover off the UI
/// thread, as the lyrics panel's is (`follow::lyric_inks_for`: the panel's deep ink in the
/// dark theme, a light tint in the light one; grey for grey art). Until it's known the band
/// is grey, then it crossfades (`Band`).
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
    data.set_featured_ink_light(inks.tint);
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
            upcoming: upcoming.into(),
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
            crossfade: None,
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
    fn more_like_this_fills_a_short_queue_only() {
        let ids = |v: &[&str]| v.iter().map(|i| track(i)).collect::<Vec<_>>();
        let related = ids(&["a", "r1", "b", "r2", "r3", "r4"]);
        let current = TrackId("a".into());
        // A radio still loading: three related tracks, not the song itself.
        assert_eq!(more_like_this(&current, &[], &related), [1, 2, 3]);
        // The end of an album, one song left: two, none already next.
        assert_eq!(more_like_this(&current, &ids(&["b"]), &related), [1, 3]);
        assert_eq!(more_like_this(&current, &ids(&["x", "y"]), &related), [1]);
        // A full column needs none; nothing loaded gives none.
        assert!(more_like_this(&current, &ids(&["x", "y", "z"]), &related).is_empty());
        assert!(more_like_this(&current, &[], &[]).is_empty());
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
