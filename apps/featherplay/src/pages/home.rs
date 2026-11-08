//! Home (ui/pages/home.slint): the hero, then the Home feed shelves, each in the shape of what
//! it holds (`arrange`), your mixes first.
//!
//! The feed is fetched once and kept for `FEED_TTL` (Home is opened often; the feed changes
//! slowly), and again after a sign-in. It goes on as the user scrolls near its end, a page at
//! a time (`Paging`): never two at once, at most `MAX_PAGES` per feed, and none without a
//! scroll. Liked songs is pinned as the first shortcut (`with_liked`). The hero follows the
//! player: `choose_hero` picks "pick up where you left off" when there's a current track,
//! else a featured pick from the first shelf.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay_core::model::{PlaylistId, Track, TrackId, VideoType};
use featherplay_core::tr;
use featherplay_ytm::{HomeItem, HomeShelf, PlaylistSummary};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::app::Ctx;
use crate::controller::{Action, PlayerState};
use crate::follow::{
    self,
    colour::{self, Lch, Rgb},
};
use crate::pages::content::{self, CARD_COVER, ROW_COVER};
use crate::router::Page;
use crate::sidebar;
use crate::song_menu;
use crate::ui::{AppWindow, HomeData, HomeShelfRow, MoreState, PageKind, ShelfShape};
use crate::ui_bridge::{home_badge, home_card, home_thumbnail, model, to_rows};

/// How long a fetched feed is shown before Home fetches it again.
const FEED_TTL: Duration = Duration::from_secs(30 * 60);
/// The hero's cover, in logical pixels (`Theme.hero-cover`).
const HERO_COVER: u32 = 152;
/// Rows in the fresh-start hero's right column (two rows of two).
const HERO_ROWS: usize = 4;
/// Cells in the resume card's queue strip, at most (`ResumeCard.cols`).
const HERO_NEXT: usize = 3;
/// Pages of the feed per Home load, the first ones included.
pub const MAX_PAGES: usize = 6;
/// Pages `YtMusic::home` fetches: the first and the continuation fetched with it.
const FIRST_PAGES: usize = 2;
/// A community playlist's avatar badge, in logical pixels.
const BADGE: u32 = 28;

/// The row after the last shelf, as `Paging` has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum More {
    /// Nothing to show: no page loading, or no more pages.
    None,
    Loading,
    /// The last page failed: a quiet Try again.
    Failed,
}

/// The feed's later pages, fetched as the user scrolls near the end: one at a time, up to
/// `MAX_PAGES` per feed, and a failed one only again when the user asks. Each feed (a fetch
/// of the first pages) is a new load; a page that arrives for an older one is dropped.
#[derive(Debug, Default)]
pub struct Paging {
    next: Option<String>,
    pages: usize,
    busy: bool,
    failed: bool,
    load: u64,
}

impl Paging {
    /// A new feed: `pages` fetched so far, `next` the token of the page after them.
    pub fn start(&mut self, next: Option<String>, pages: usize) {
        *self = Paging {
            next,
            pages,
            load: self.load + 1,
            ..Paging::default()
        };
    }

    /// The user scrolled near the end: the page to fetch now, with its load, if one may go.
    pub fn want(&mut self) -> Option<(u64, String)> {
        if self.busy || self.failed || self.pages >= MAX_PAGES {
            return None;
        }
        let token = self.next.clone()?;
        self.busy = true;
        Some((self.load, token))
    }

    /// Try again after a failed page.
    pub fn retry(&mut self) -> Option<(u64, String)> {
        self.failed = false;
        self.want()
    }

    /// The page for `load` arrived, `next` the token after it. False when it's an older
    /// feed's: drop it.
    pub fn loaded(&mut self, load: u64, next: Option<String>) -> bool {
        if load != self.load || !self.busy {
            return false;
        }
        self.busy = false;
        self.pages += 1;
        self.next = next;
        true
    }

    /// The page for `load` failed. False when it's an older feed's.
    pub fn failed(&mut self, load: u64) -> bool {
        if load != self.load || !self.busy {
            return false;
        }
        self.busy = false;
        self.failed = true;
        true
    }

    pub fn more(&self) -> More {
        if self.busy {
            More::Loading
        } else if self.failed {
            More::Failed
        } else {
            More::None
        }
    }
}

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

#[derive(Default)]
struct State {
    shelves: Vec<HomeShelf>,
    fetched: Option<Instant>,
    fetching: bool,
    /// Bumped on sign-in and sign-out; a fetch from before is dropped.
    account: u64,
    hero: Option<Hero>,
    hero_url: Option<String>,
    /// The resumed song's cover the wash is (being) worked out for, and its colour once it is.
    wash_url: Option<String>,
    wash_seed: Option<Lch>,
    next: Vec<TrackId>,
    paging: Paging,
    /// The shelves `HomeData` shows, so a later page's are added to them.
    rows: Option<Rc<VecModel<HomeShelfRow>>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Once at startup: connect `HomeData`'s callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<HomeData>();
    let c = ctx.clone();
    data.on_resume_clicked(move || c.send(Action::Toggle));
    let c = ctx.clone();
    data.on_play_card(move |it| content::play_card(&c, &it));
    // After an error: fetch the feed again (one call; ignored while one runs). While the
    // stored session can't connect, reconnect it instead: Home shows it's loading, and
    // `account_changed` (or `UiEvent::Unreachable`) opens it again with the answer.
    let c = ctx.clone();
    data.on_retry(move || {
        if c.reconnect() {
            open(&c);
        } else if !STATE.with(|s| s.borrow().fetching) {
            fetch(&c);
        }
    });
    let c = ctx.clone();
    data.on_near_end(move || more(&c, false));
    let c = ctx.clone();
    data.on_retry_more(move || more(&c, true));
    bind_rows(&ui);
    let c = ctx.clone();
    data.on_play_next(move |i| {
        // Jumps by playing the same queue from that row on.
        let Some(state) = c.player() else { return };
        let Some(current) = state.current else { return };
        let start = i.max(0) as usize + 1;
        let mut tracks = vec![current];
        tracks.extend(state.upcoming);
        if start < tracks.len() {
            c.send(Action::Play {
                tracks,
                start,
                radio: state.radio,
            });
        }
    });
    let c = ctx.clone();
    data.on_play_featured(move || {
        if let Some(item) = featured(0) {
            play_item(&c, item);
        }
    });
    let c = ctx.clone();
    data.on_open_featured(move |i| {
        if let Some(item) = featured((i + 1).max(0) as usize) {
            open_item(&c, item);
        }
    });
    let c = ctx.clone();
    data.on_play_song(move |id| {
        let track = STATE.with(|s| {
            s.borrow()
                .shelves
                .iter()
                .flat_map(|s| &s.items)
                .find_map(|i| match i {
                    HomeItem::Track(t) if t.id.0 == id.as_str() => Some(t.clone()),
                    _ => None,
                })
        });
        if let Some(t) = track {
            play_song(&c, t);
        }
    });
}

/// Item `i` of the featured shelf (0 is the pick itself).
fn featured(i: usize) -> Option<HomeItem> {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(Hero::Featured { shelf }) = s.hero else {
            return None;
        };
        s.shelves.get(shelf)?.items.get(i).cloned()
    })
}

fn play_song(ctx: &Ctx, t: Track) {
    song_menu::remember(std::slice::from_ref(&t));
    ctx.send(Action::Play {
        tracks: vec![t],
        start: 0,
        radio: true,
    });
}

fn play_item(ctx: &Rc<Ctx>, item: HomeItem) {
    match item {
        HomeItem::Track(t) => play_song(ctx, t),
        HomeItem::Album(a) => content::play_album(ctx, a.id),
        HomeItem::Playlist(p) => content::play_playlist(ctx, p.id),
        HomeItem::Artist(a) => content::play_artist(ctx, a.id),
    }
}

fn open_item(ctx: &Rc<Ctx>, item: HomeItem) {
    match item {
        HomeItem::Track(t) => play_song(ctx, t),
        HomeItem::Album(a) => ctx.navigate(Page::Album { id: a.id }),
        HomeItem::Playlist(p) => ctx.navigate(Page::Playlist { id: p.id }),
        HomeItem::Artist(a) => ctx.navigate(Page::Artist { id: a.id }),
    }
}

/// The page is being shown.
pub fn open(ctx: &Rc<Ctx>) {
    // At startup the stored session isn't back yet: Home shows it's loading, and
    // `account_changed` opens it again once the controller has signed in (or found no
    // session), so the feed is fetched once, with the session.
    if ctx.session_pending() {
        if let Some(ui) = ctx.window() {
            let data = ui.global::<HomeData>();
            data.set_loading(true);
            data.set_error("".into());
        }
        show_hero(ctx);
        return;
    }
    let fresh = STATE.with(|s| {
        let s = s.borrow();
        s.fetching || s.fetched.is_some_and(|t| t.elapsed() < FEED_TTL)
    });
    if !fresh {
        fetch(ctx);
    }
    show_hero(ctx);
}

/// Fetches the feed (one call; the result is kept even if the user has moved on).
fn fetch(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<HomeData>();
    data.set_loading(true);
    data.set_error("".into());
    data.set_empty(false);
    data.set_more(MoreState::Idle);
    let account = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.fetching = true;
        // A later page still on its way belongs to the feed being replaced.
        s.paging.start(None, 0);
        s.account
    });
    let asked = Instant::now();
    ctx.load(
        |source| async move { source.home().await },
        move |ctx, ui, result| {
            if STATE.with(|s| s.borrow().account) != account {
                return;
            }
            let took = asked.elapsed().as_millis();
            match &result {
                Ok(feed) => log!("home: {} shelves in {took} ms", feed.shelves.len()),
                Err(e) => log!("home: failed in {took} ms: {e}"),
            }
            let data = ui.global::<HomeData>();
            data.set_loading(false);
            STATE.with(|s| s.borrow_mut().fetching = false);
            match result {
                Ok(feed) => {
                    show_shelves(ctx, &with_liked(&feed.shelves, &sidebar::shortcuts()));
                    STATE.with(|s| {
                        let mut s = s.borrow_mut();
                        s.shelves = feed.shelves;
                        s.paging.start(feed.more, FIRST_PAGES);
                        s.fetched = Some(Instant::now());
                        s.hero = None;
                    });
                }
                Err(e) => data.set_error(content::error_text("home", &e).into()),
            }
            show_hero(ctx);
        },
    );
}

fn show_shelves(ctx: &Ctx, shelves: &[HomeShelf]) {
    let Some(ui) = ctx.window() else { return };
    let rows: Vec<HomeShelfRow> = arrange(shelves)
        .into_iter()
        .map(|(i, shape)| shelf_row(ctx, &shelves[i], shape, PageKind::Home))
        .collect();
    let data = ui.global::<HomeData>();
    data.set_empty(rows.is_empty());
    let (rows, model) = model(rows);
    data.set_shelves(model);
    STATE.with(|s| s.borrow_mut().rows = Some(rows));
}

/// The user scrolled near the end of Home (or pressed Try again on a failed page): fetches
/// the next page, if `Paging` lets one go now, and adds its shelves under the others.
fn more(ctx: &Rc<Ctx>, retry: bool) {
    let want = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if retry {
            s.paging.retry()
        } else {
            s.paging.want()
        }
    });
    let Some((load, token)) = want else { return };
    show_more(ctx);
    ctx.load(
        move |source| async move { source.home_more(&token).await },
        move |ctx, _, result| {
            let current = STATE.with(|s| {
                let mut s = s.borrow_mut();
                match &result {
                    Ok(feed) => s.paging.loaded(load, feed.more.clone()),
                    Err(_) => s.paging.failed(load),
                }
            });
            if !current {
                return;
            }
            match result {
                Ok(feed) => {
                    add_shelves(ctx, &feed.shelves);
                    STATE.with(|s| s.borrow_mut().shelves.extend(feed.shelves));
                }
                Err(e) => log!("home: the next page failed: {e}"),
            }
            show_more(ctx);
        },
    );
}

/// The picks for the Shortcuts row changed (the library arrived, or something was opened):
/// the row is put in, or its tiles replaced, in place. A feed with your mixes keeps them as
/// its shortcuts, so nothing changes then.
pub fn shortcuts_changed(ctx: &Ctx) {
    let Some(rows) = STATE.with(|s| s.borrow().rows.clone()) else {
        return;
    };
    let shelves = STATE.with(|s| s.borrow().shelves.clone());
    let has_mixes = arrange(&with_liked(&shelves, &[]))
        .first()
        .is_some_and(|(_, shape)| *shape == ShelfShape::Mixes);
    if has_mixes {
        return;
    }
    let shown = with_liked(&shelves, &sidebar::shortcuts());
    let Some(row) = shown
        .first()
        .filter(|s| is_shortcuts(s))
        .map(|s| shelf_row(ctx, s, ShelfShape::Mixes, PageKind::Home))
    else {
        return;
    };
    let first_is_shortcuts = rows
        .row_data(0)
        .is_some_and(|r| r.shape == ShelfShape::Mixes);
    if first_is_shortcuts {
        rows.set_row_data(0, row);
    } else {
        rows.insert(0, row);
    }
}

/// `--snapshot home-shortcuts`: the feed without its mixes, as an account with none yet
/// sees it, so the Shortcuts row shows.
pub fn drop_mixes_for_snapshot(ctx: &Ctx) {
    let shelves: Vec<HomeShelf> = STATE.with(|s| {
        s.borrow()
            .shelves
            .iter()
            .filter(|s| shape(&s.items) != ShelfShape::Mixes)
            .cloned()
            .collect()
    });
    STATE.with(|s| s.borrow_mut().shelves = shelves.clone());
    show_shelves(ctx, &with_liked(&shelves, &sidebar::shortcuts()));
}

/// A later page's shelves, under the ones shown.
fn add_shelves(ctx: &Ctx, shelves: &[HomeShelf]) {
    let Some(rows) = STATE.with(|s| s.borrow().rows.clone()) else {
        return;
    };
    for (i, shape) in arrange(shelves) {
        rows.push(shelf_row(ctx, &shelves[i], shape, PageKind::Home));
    }
}

/// Gives Home's `ListView` one model for its rows, for the window's life, so it only ever
/// gains or loses rows at the end and keeps the ones it shows (see home.slint), filled to the
/// rows `HomeData` has now. `init` does it; a window built without the app's wiring (a UI
/// test) needs it to show Home.
pub fn bind_rows(ui: &AppWindow) {
    let data = ui.global::<HomeData>();
    let rows = Rc::new(VecModel::<i32>::default());
    sync_rows(&rows, data.get_row_count());
    data.set_rows(ModelRc::from(rows.clone()));
    data.on_sync_rows(move |count| sync_rows(&rows, count));
}

/// Grows or shrinks `rows` to `count` entries at its end; entry `i` is `i`.
fn sync_rows(rows: &VecModel<i32>, count: i32) {
    let count = usize::try_from(count).unwrap_or(0);
    while rows.row_count() > count {
        rows.remove(rows.row_count() - 1);
    }
    while rows.row_count() < count {
        rows.push(i32::try_from(rows.row_count()).unwrap_or(i32::MAX));
    }
}

/// The row after the last shelf, from `Paging`.
fn show_more(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let more = STATE.with(|s| s.borrow().paging.more());
    ui.global::<HomeData>().set_more(match more {
        More::None => MoreState::Idle,
        More::Loading => MoreState::Loading,
        More::Failed => MoreState::Failed,
    });
}

/// A shelf for `HomeData`, its covers on their way at the size its shape shows them. Its
/// cards' covers belong to `page` (`Covers::cards`).
pub(crate) fn shelf_row(
    ctx: &Ctx,
    s: &HomeShelf,
    shape: ShelfShape,
    page: PageKind,
) -> HomeShelfRow {
    let (cards, items) = model(s.items.iter().map(home_card).collect());
    // Community playlists' creators, over their collages: an empty image where there's none.
    let urls: Vec<Option<String>> = s.items.iter().map(home_badge).collect();
    let badges = if urls.iter().any(Option::is_some) {
        let (images, badges) = model(vec![slint::Image::default(); urls.len()]);
        ctx.covers
            .fill(&images, urls, BADGE, |slot, img| *slot = img);
        badges
    } else {
        ModelRc::default()
    };
    let mut songs = ModelRc::default();
    // For the song menu, and so a song card resolved ahead (`RowHover`) is the song Play
    // starts (`Player::play_id`).
    let tracks: Vec<Track> = s.items.iter().filter_map(as_track).cloned().collect();
    song_menu::remember(&tracks);
    if shape == ShelfShape::Songs {
        // Quick picks: rows, not cards.
        let urls = tracks.iter().map(|t| t.thumbnail.clone()).collect();
        let (rows, model_rows) = model(to_rows(&tracks));
        ctx.covers
            .fill(&rows, urls, ROW_COVER, |row, img| row.cover = img);
        songs = model_rows;
    } else if shape == ShelfShape::Videos {
        // 16:9 art, as the tile shows it (no square's middle band).
        let urls = s.items.iter().map(home_thumbnail).collect();
        ctx.covers.cards_wide(page, &cards, urls, cover_px(shape));
    } else {
        let urls = s.items.iter().map(home_thumbnail).collect();
        ctx.covers.cards(page, &cards, urls, cover_px(shape));
    }
    HomeShelfRow {
        title: s.title.as_str().into(),
        lead: lead(s.strapline.as_deref()).into(),
        items,
        songs,
        badges,
        shape,
    }
}

/// The size, in logical pixels, a shape's covers are fetched at: about the widest its tiles
/// get (`Shapes.min-w` in home.slint sets how many fit a row, so a tile stays under twice
/// it). Covers are square, except a video tile's: 16:9, this wide.
fn cover_px(shape: ShelfShape) -> u32 {
    match shape {
        ShelfShape::Songs => ROW_COVER,
        ShelfShape::Mixes => 64,
        ShelfShape::Videos => 320,
        ShelfShape::Playlists => 160,
        ShelfShape::Artists => 140,
        _ => CARD_COVER,
    }
}

/// Rows per column of a song grid (`Shapes.grid-rows`): fewer songs than that stay cards.
const SONG_GRID_ROWS: usize = 4;

fn as_track(item: &HomeItem) -> Option<&Track> {
    match item {
        HomeItem::Track(t) => Some(t),
        _ => None,
    }
}

/// The shelves to show, in order, as (index in `shelves`, shape): empty shelves are dropped,
/// and the first shelf of your mixes moves up under the hero (they're what gets replayed most).
pub fn arrange(shelves: &[HomeShelf]) -> Vec<(usize, ShelfShape)> {
    let mut out: Vec<(usize, ShelfShape)> = shelves
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.items.is_empty())
        .map(|(i, s)| {
            let shape = if is_shortcuts(s) {
                ShelfShape::Mixes
            } else {
                shape(&s.items)
            };
            (i, shape)
        })
        .collect();
    if let Some(at) = out.iter().position(|(_, s)| *s == ShelfShape::Mixes) {
        let mixes = out.remove(at);
        out.insert(0, mixes);
    }
    out
}

/// How a shelf shows, from what it holds. Anything else (a mix of kinds, a kind added later)
/// is standard cards.
fn shape(items: &[HomeItem]) -> ShelfShape {
    let all = |f: fn(&HomeItem) -> bool| !items.is_empty() && items.iter().all(f);
    if all(|i| matches!(i, HomeItem::Track(_))) {
        let videos = items
            .iter()
            .filter_map(as_track)
            .filter(|t| t.video_type != VideoType::Song)
            .count();
        if videos * 2 > items.len() {
            ShelfShape::Videos
        } else if items.len() >= SONG_GRID_ROWS {
            ShelfShape::Songs
        } else {
            ShelfShape::Cards
        }
    } else if all(|i| matches!(i, HomeItem::Playlist(p) if is_mix(&p.id.0))) {
        ShelfShape::Mixes
    } else if all(|i| matches!(i, HomeItem::Playlist(_))) {
        ShelfShape::Playlists
    } else if all(|i| matches!(i, HomeItem::Artist(_))) {
        ShelfShape::Artists
    } else {
        ShelfShape::Cards
    }
}

/// Liked songs ("LM"), as Home pins it.
fn liked_songs() -> HomeItem {
    HomeItem::Playlist(PlaylistSummary {
        id: PlaylistId(LIKED.into()),
        title: tr!("Liked songs").into(),
        author: None,
        count: None,
        views: None,
        avatar: None,
        thumbnail: None,
        owned: false,
    })
}

/// The id of Liked songs, and of the feed's own "Liked Music" tile.
const LIKED: &str = "LM";

/// Tiles in the Shortcuts row, Liked songs included.
const SHORTCUTS: usize = 4;
/// The Shortcuts row's title (`with_liked`).
fn shortcuts_title() -> &'static str {
    tr!("Shortcuts")
}

/// The row `with_liked` makes without mixes: Liked songs, then playlists and albums, shown
/// as your mixes are, as wide shortcut tiles.
fn is_shortcuts(s: &HomeShelf) -> bool {
    s.title == shortcuts_title()
        && matches!(s.items.first(), Some(HomeItem::Playlist(p)) if p.id.0 == LIKED)
}

/// The feed as Home shows it: Liked songs pinned as the first of your mixes' shortcuts, so
/// it's always one click away. The feed's own Liked Music tile goes, so it shows once.
/// Without a shelf of mixes, Liked songs leads a "Shortcuts" row of its own, followed by
/// `picks` (the playlists and albums opened or played lately, `sidebar::shortcuts`), four
/// tiles at most; with no picks there's no row, never a lone tile. Nothing before the feed
/// arrives.
pub fn with_liked(shelves: &[HomeShelf], picks: &[HomeItem]) -> Vec<HomeShelf> {
    if shelves.is_empty() {
        return Vec::new();
    }
    let mut out = shelves.to_vec();
    for s in &mut out {
        s.items
            .retain(|i| !matches!(i, HomeItem::Playlist(p) if p.id.0 == LIKED));
    }
    match out
        .iter()
        .position(|s| !s.items.is_empty() && shape(&s.items) == ShelfShape::Mixes)
    {
        Some(at) => out[at].items.insert(0, liked_songs()),
        None if !picks.is_empty() => {
            let mut items = vec![liked_songs()];
            items.extend(picks.iter().take(SHORTCUTS - 1).cloned());
            out.insert(
                0,
                HomeShelf {
                    title: shortcuts_title().into(),
                    strapline: None,
                    items,
                },
            );
        }
        None => {}
    }
    out
}

/// Your own mixes: the ones made for the account (My Supermix, Replay, Discover, Archive:
/// "RDTMAK…") and Liked Music. Editorial playlists ("RDCLAK…") are playlists.
fn is_mix(id: &str) -> bool {
    id == "LM" || id.starts_with("RDTMAK")
}

/// The words before a shelf's title, from its strapline, in sentence case: "Similar to"
/// Ari Abdul. Personal (scrubbed) and label-only straplines show nothing.
fn lead(strapline: Option<&str>) -> &'static str {
    match strapline {
        Some("SIMILAR TO") => tr!("Similar to"),
        _ => "",
    }
}

/// Shows the hero the player state and the feed call for; cheap when nothing changed.
fn show_hero(ctx: &Rc<Ctx>) {
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

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(ctx: &Rc<Ctx>, _state: &PlayerState) {
    show_hero(ctx);
}

/// Signed in or out: the feed is the account's, so it's fetched again.
pub fn account_changed(ctx: &Rc<Ctx>, signed_in: bool) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.fetched = None;
        s.fetching = false;
        s.account += 1;
        s.paging.start(None, 0);
        if !signed_in {
            s.shelves.clear();
            s.hero = None;
        }
    });
    if let Some(ui) = ctx.window()
        && !signed_in
    {
        let data = ui.global::<HomeData>();
        data.set_shelves(Default::default());
        data.set_empty(false);
        data.set_more(MoreState::Idle);
    }
    if ctx.current_page() == Page::Home {
        open(ctx);
    }
}

#[cfg(test)]
mod tests {
    use featherplay_core::model::{ArtistId, LikeStatus, RepeatMode};
    use featherplay_ytm::{AlbumSummary, ArtistSummary};

    use super::*;

    fn track(id: &str) -> Track {
        Track {
            id: TrackId(id.into()),
            title: id.into(),
            artists: Vec::new(),
            album: None,
            duration: None,
            video_type: VideoType::Song,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            counterpart: None,
        }
    }

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

    fn shelf(title: &str, items: Vec<HomeItem>) -> HomeShelf {
        HomeShelf {
            title: title.into(),
            strapline: None,
            items,
        }
    }

    fn album() -> HomeItem {
        HomeItem::Album(AlbumSummary {
            id: featherplay_core::model::AlbumId("MPREb_a".into()),
            title: "An album".into(),
            artists: Vec::new(),
            year: None,
            thumbnail: None,
        })
    }

    fn video(id: &str) -> HomeItem {
        let mut t = track(id);
        t.video_type = VideoType::UserVideo;
        HomeItem::Track(t)
    }

    fn song(id: &str) -> HomeItem {
        HomeItem::Track(track(id))
    }

    fn playlist(id: &str) -> HomeItem {
        HomeItem::Playlist(PlaylistSummary {
            id: PlaylistId(id.into()),
            title: id.into(),
            author: None,
            count: None,
            views: None,
            avatar: None,
            thumbnail: None,
            owned: false,
        })
    }

    fn artist() -> HomeItem {
        HomeItem::Artist(ArtistSummary {
            id: ArtistId("UCa".into()),
            name: "An artist".into(),
            thumbnail: None,
        })
    }

    fn shapes(shelves: &[HomeShelf]) -> Vec<ShelfShape> {
        arrange(shelves).into_iter().map(|(_, s)| s).collect()
    }

    #[test]
    fn each_shelf_takes_the_shape_of_what_it_holds() {
        let songs = || (0..4).map(|i| song(&i.to_string())).collect::<Vec<_>>();
        let shelves = [
            shelf("Quick picks", songs()),
            shelf("Listen together", vec![video("v"), song("s"), video("w")]),
            shelf("Ari Abdul", vec![playlist("PLa"), playlist("PLb")]),
            shelf("Isabel LaRosa", vec![artist(), artist()]),
            shelf("Albums for you", vec![album(), album()]),
        ];
        assert_eq!(
            shapes(&shelves),
            [
                ShelfShape::Songs,
                ShelfShape::Videos,
                ShelfShape::Playlists,
                ShelfShape::Artists,
                ShelfShape::Cards,
            ]
        );
    }

    #[test]
    fn mixed_or_short_shelves_are_standard_cards() {
        let shelves = [
            // An artist with playlists ("Similar to"): standard cards, round only the artist.
            shelf("Similar", vec![artist(), playlist("PLa"), playlist("PLb")]),
            // Too few songs to fill a column of the grid.
            shelf("Two songs", vec![song("a"), song("b")]),
            shelf("Mixed", vec![song("a"), album()]),
            // Your mixes with an editorial playlist among them.
            shelf(
                "Not all mixes",
                vec![playlist("LM"), playlist("RDCLAK5uy_x")],
            ),
        ];
        assert_eq!(
            shapes(&shelves),
            [
                ShelfShape::Cards,
                ShelfShape::Cards,
                ShelfShape::Cards,
                ShelfShape::Playlists,
            ]
        );
    }

    #[test]
    fn your_mixes_come_first_and_empty_shelves_are_dropped() {
        let shelves = [
            shelf(
                "Quick picks",
                (0..4).map(|i| song(&i.to_string())).collect(),
            ),
            shelf("Empty", Vec::new()),
            shelf(
                "Fresh finds, old favorites",
                vec![
                    playlist("RDTMAK5uy_a"),
                    playlist("LM"),
                    playlist("RDTMAK5uy_b"),
                ],
            ),
            shelf("Albums for you", vec![album()]),
            shelf("More mixes", vec![playlist("RDTMAK5uy_c")]),
        ];
        assert_eq!(
            arrange(&shelves),
            [
                (2, ShelfShape::Mixes),
                (0, ShelfShape::Songs),
                (3, ShelfShape::Cards),
                (4, ShelfShape::Mixes),
            ]
        );
    }

    #[test]
    fn similar_to_leads_the_title_in_sentence_case() {
        assert_eq!(lead(Some("SIMILAR TO")), "Similar to");
        // Personal (scrubbed) and label straplines don't show.
        assert_eq!(lead(Some("STATION")), "");
        assert_eq!(lead(Some("REDACTED")), "");
        assert_eq!(lead(None), "");
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

    fn ids(shelf: &HomeShelf) -> Vec<&str> {
        shelf
            .items
            .iter()
            .map(|i| match i {
                HomeItem::Playlist(p) => p.id.0.as_str(),
                _ => "?",
            })
            .collect()
    }

    #[test]
    fn liked_songs_is_the_first_shortcut_and_shows_once() {
        let shelves = [
            shelf(
                "Quick picks",
                (0..4).map(|i| song(&i.to_string())).collect(),
            ),
            shelf(
                "Fresh finds, old favorites",
                vec![
                    playlist("RDTMAK5uy_a"),
                    playlist("LM"),
                    playlist("RDTMAK5uy_b"),
                ],
            ),
        ];
        // Recent picks don't join a feed that has your mixes.
        let pinned = with_liked(&shelves, &[playlist("PLrecent")]);
        let (first, shape) = arrange(&pinned)[0];
        assert_eq!(shape, ShelfShape::Mixes);
        assert_eq!(ids(&pinned[first]), ["LM", "RDTMAK5uy_a", "RDTMAK5uy_b"]);
        let HomeItem::Playlist(liked) = &pinned[first].items[0] else {
            unreachable!()
        };
        assert_eq!(liked.title, "Liked songs");
    }

    #[test]
    fn without_mixes_the_shortcuts_are_liked_songs_and_recent_picks() {
        let shelves = [shelf("Albums for you", vec![album(), album()])];
        let recent = [playlist("PLa"), playlist("PLb"), playlist("PLc")];
        let pinned = with_liked(&shelves, &recent);
        let (first, shape) = arrange(&pinned)[0];
        assert_eq!(shape, ShelfShape::Mixes);
        assert_eq!(pinned[first].title, "Shortcuts");
        assert_eq!(ids(&pinned[first]), ["LM", "PLa", "PLb", "PLc"]);
        // Nothing to pin to before the feed arrives.
        assert!(with_liked(&[], &recent).is_empty());
    }

    #[test]
    fn the_shortcuts_row_shows_four_at_most() {
        let shelves = [shelf("Albums for you", vec![album(), album()])];
        let recent: Vec<HomeItem> = (0..6).map(|i| playlist(&format!("PL{i}"))).collect();
        let pinned = with_liked(&shelves, &recent);
        assert_eq!(pinned[0].items.len(), 4);
    }

    #[test]
    fn the_shortcuts_row_is_never_a_single_tile() {
        let shelves = [shelf("Albums for you", vec![album(), album()])];
        let pinned = with_liked(&shelves, &[]);
        assert_eq!(pinned.len(), 1, "a row with Liked songs alone was added");
        assert_ne!(arrange(&pinned)[0].1, ShelfShape::Mixes);
    }

    #[test]
    fn an_album_among_the_shortcuts_keeps_the_row_a_shortcut_row() {
        let shelves = [shelf(
            "Quick picks",
            (0..4).map(|i| song(&i.to_string())).collect(),
        )];
        let pinned = with_liked(&shelves, &[album()]);
        let (first, shape) = arrange(&pinned)[0];
        assert_eq!(shape, ShelfShape::Mixes);
        assert_eq!(pinned[first].items.len(), 2);
    }

    #[test]
    fn more_shelves_load_one_page_at_a_time_up_to_the_cap() {
        let mut paging = Paging::default();
        paging.start(Some("t2".into()), 2);
        let (load, token) = paging.want().expect("a page to fetch");
        assert_eq!(token, "t2");
        // Never two at once: nothing more goes out while that one loads.
        assert_eq!(paging.want(), None);
        assert_eq!(paging.more(), More::Loading);
        let mut fetched = 1;
        assert!(paging.loaded(load, Some("t3".into())));
        while let Some((load, token)) = paging.want() {
            fetched += 1;
            assert!(paging.loaded(load, Some(format!("{token}+"))));
        }
        assert_eq!(2 + fetched, MAX_PAGES);
        assert_eq!(paging.more(), More::None);
    }

    #[test]
    fn a_failed_page_offers_try_again_and_keeps_home() {
        let mut paging = Paging::default();
        paging.start(Some("t2".into()), 2);
        let (load, _) = paging.want().expect("a page to fetch");
        assert!(paging.failed(load));
        assert_eq!(paging.more(), More::Failed);
        // No retry by itself, however far the user scrolls.
        assert_eq!(paging.want(), None);
        let (load, token) = paging.retry().expect("Try again fetches it again");
        assert_eq!(token, "t2");
        assert!(paging.loaded(load, None));
        assert_eq!(paging.more(), More::None);
        assert_eq!(paging.want(), None, "no page after the last");
    }

    #[test]
    fn a_page_for_an_older_feed_is_dropped() {
        let mut paging = Paging::default();
        paging.start(Some("old".into()), 2);
        let (old, _) = paging.want().expect("a page to fetch");
        // Home fetched again (a sign-in, or the feed went stale) while that page loaded.
        paging.start(Some("new".into()), 2);
        assert!(!paging.loaded(old, Some("x".into())));
        assert_eq!(paging.want().map(|(_, t)| t).as_deref(), Some("new"));
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
