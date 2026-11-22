//! Home (ui/pages/home.slint): the Now playing card (the band) and the Shortcuts beside it
//! (Liked songs, the pins, what's played lately and the library: `sidebar::shortcuts`, shown
//! in `shortcuts`), then the Home feed's shelves as rails, each in the shape of what it holds
//! (`arrange`), your mixes first.
//!
//! The feed is fetched once and kept for `FEED_TTL` (Home is opened often; the feed changes
//! slowly), and again after a sign-in. It goes on as the user scrolls near its end, a page at
//! a time (`Paging`): never two at once, at most `MAX_PAGES` per feed, and none without a
//! scroll. The feed's own Liked Music tile goes (`without_liked`): Liked songs leads
//! Shortcuts. Shortcuts takes what's been played when Home opens, never while it shows, so
//! no tile moves under the pointer; a pin, a move or a removal shows at once. The band
//! follows the player: `choose_hero` picks the session when there's
//! a current track, else a featured pick from the first shelf.
//!
//! Signed in, the first pages' shelves are kept on disk for the identity (`CachedFeed`), so
//! the next start shows Home at once from them while the feed is fetched behind. A fresh feed
//! that differs doesn't replace the cached one under the user: it waits for Home's next open,
//! and its later pages go on under the cached shelves as the user scrolls.
//! Another identity's feed is never shown, nor any signed out; a day-old one isn't either.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use featherplay_controller::place::Place;
use featherplay_core::model::{PlaylistId, Track, TrackId};
use featherplay_core::tr;
use featherplay_ytm::{HomeFeed, HomeItem, HomeShelf};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::app::Ctx;
use crate::controller::{Action, PlayerState};
use crate::pages::content;
use crate::router::Page;
use crate::sidebar;
use crate::song_menu;
use crate::ui::{AppWindow, CardItem, HomeData, HomeShelfRow, ItemKind, MoreState, PageKind};
use crate::ui_bridge::{model, shown};

mod feed;
mod hero;
mod shelves;

pub use feed::{MAX_PAGES, More, Paging};
use feed::{Owner, account_news, fetch, more, owner_of, show_cached, show_feed};
use hero::show_hero;
pub use hero::{Hero, choose_hero};
pub(crate) use shelves::shelf_row;
pub use shelves::{arrange, without_liked};

/// How long a fetched feed is shown before Home fetches it again.
const FEED_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
struct State {
    shelves: Vec<HomeShelf>,
    fetched: Option<Instant>,
    fetching: bool,
    /// Whom the last fetch asked as, when the source was known.
    fetch_owner: Option<Owner>,
    /// Whose feed is on screen: `None` for a visitor's, or none at all.
    owner: Option<Owner>,
    /// The feed on screen came from the disk cache; no fresh one has replaced it yet.
    from_cache: bool,
    /// The disk cache was read since the last account change.
    cache_read: bool,
    /// The fresh feed that came while the cached one was on screen: it's shown the next
    /// time Home opens, so the page never changes under the user.
    pending: Option<HomeFeed>,
    /// Bumped on sign-in and sign-out; a fetch from before is dropped.
    account: u64,
    hero: Option<Hero>,
    hero_url: Option<String>,
    /// The featured pick's cover the band's colour is (being) worked out for.
    ink_url: Option<String>,
    next: Vec<TrackId>,
    /// "More like this" under a short queue: the related tracks shown, and where each is in
    /// the song's related list (`now_playing::related_of`), which a click plays from.
    related: Vec<TrackId>,
    related_at: Vec<usize>,
    /// The band has room for its queue column, where "More like this" shows, as it last
    /// said (`HomeData.band-column`); forgotten as Home closes, with the band.
    band_column: bool,
    paging: Paging,
    /// The shelves `HomeData` shows, so a later page's are added to them.
    rows: Option<Rc<VecModel<HomeShelfRow>>>,
    /// The Shortcuts' tile size the grid last told (`HomeData.shortcut-tile`), their cards
    /// and how many, and those cards with their cover URLs while the grid may yet tell a
    /// new size for them.
    shortcut_px: Option<u32>,
    shortcut_cards: Weak<VecModel<CardItem>>,
    shortcut_count: usize,
    shortcut_wait: Option<ShortcutCovers>,
}

/// Shortcuts' cards and their cover URLs, by row.
type ShortcutCovers = (Rc<VecModel<CardItem>>, Vec<Option<String>>);

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
    data.on_show_queue(move || crate::queue_panel::show_up_next(&c));
    let c = ctx.clone();
    data.on_play_related(move |i| {
        let at = usize::try_from(i)
            .ok()
            .and_then(|i| STATE.with(|s| s.borrow().related_at.get(i).copied()));
        if let Some(at) = at {
            crate::pages::now_playing::play_related(&c, at);
        }
    });
    let c = ctx.clone();
    data.on_band_column(move |room| hero::band_column(&c, room));
    let c = ctx.clone();
    data.on_shortcut_tile(move |px| shortcut_tile(&c, px));
    let c = ctx.clone();
    data.on_shuffle_liked(move || content::shuffle_playlist(&c, PlaylistId("LM".into())));
    crate::item_menu::init(ctx);
    let c = ctx.clone();
    data.on_play_featured(move || {
        if let Some(item) = featured() {
            play_item(&c, item);
        }
    });
    let c = ctx.clone();
    data.on_open_featured(move || {
        if let Some(item) = featured() {
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

/// The featured pick: the first item of its shelf.
fn featured() -> Option<HomeItem> {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(Hero::Featured { shelf }) = s.hero else {
            return None;
        };
        s.shelves.get(shelf)?.items.first().cloned()
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

/// Another page is shown: the band went with Home, and says again when it's back.
pub fn closed() {
    STATE.with(|s| s.borrow_mut().band_column = false);
}

/// The page is being shown.
pub fn open(ctx: &Rc<Ctx>) {
    // A fresh feed that came while the cached one was on screen: in now, as Home opens.
    if let Some(feed) = STATE.with(|s| s.borrow_mut().pending.take()) {
        show_feed(ctx, feed);
    }
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
    show_cached(ctx);
    show_shortcuts(ctx, true);
    let fresh = STATE.with(|s| {
        let s = s.borrow();
        s.fetching || s.fetched.is_some_and(|t| t.elapsed() < FEED_TTL)
    });
    if !fresh {
        fetch(ctx);
    }
    show_hero(ctx);
}

fn show_shelves(ctx: &Ctx, shelves: &[HomeShelf]) {
    let Some(ui) = ctx.window() else { return };
    sidebar::seen_on_shelves(shelves);
    let shelves = &without_liked(shelves);
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

/// What Shortcuts is made of changed (the library arrived; a pin, a move, a removal; a
/// setting): the row follows at once. What's been played since Home opened waits for its
/// next visit.
pub fn shortcuts_changed(ctx: &Ctx) {
    show_shortcuts(ctx, false);
}

/// Home's Shortcuts row: Liked songs, then `sidebar::shortcuts` (with what's been played
/// since the last visit when `visit`), their covers on their way. None signed out or with
/// the row turned off.
fn show_shortcuts(ctx: &Ctx, visit: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<HomeData>();
    let on = ctx.signed_in() && crate::pages::settings::shortcuts();
    data.set_shortcuts_shown(on);
    if !on {
        data.set_shortcuts(Default::default());
        data.set_shortcut_pins(Default::default());
        STATE.with(|s| s.borrow_mut().shortcut_wait = None);
        return;
    }
    let tiles = sidebar::shortcuts(visit);
    let liked = CardItem {
        id: "LM".into(),
        kind: ItemKind::Playlist,
        title: tr!("Liked songs").into(),
        subtitle: tr!("Everything you liked").into(),
        ..Default::default()
    };
    let cards: Vec<CardItem> = std::iter::once(liked)
        .chain(tiles.iter().map(|t| CardItem {
            id: t.entry.id.as_str().into(),
            kind: sidebar::item_kind(t.entry.kind),
            title: shown(&t.entry.title),
            subtitle: shown(&crate::library_list::subtitle(&t.entry)),
            cover: Default::default(),
            round: t.entry.kind == crate::library_list::Kind::Artist,
        }))
        .collect();
    let urls = std::iter::once(None)
        .chain(tiles.iter().map(|t| t.entry.thumbnail.clone()))
        .collect();
    let pins: Vec<bool> = std::iter::once(false)
        .chain(tiles.iter().map(|t| t.pinned))
        .collect();
    data.set_shortcut_hint(!pins.contains(&true));
    data.set_shortcut_pins(ModelRc::new(VecModel::from(pins)));
    let (rows, items) = model(cards);
    let count = rows.row_count();
    data.set_shortcuts(items);
    let px = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.shortcut_cards = Rc::downgrade(&rows);
        s.shortcut_wait = None;
        let same = std::mem::replace(&mut s.shortcut_count, count) == count;
        s.shortcut_px.filter(|_| same)
    });
    match px {
        Some(px) => ctx.covers.cards(PageKind::Home, &rows, urls, px),
        None => wait_for_tile(ctx, rows, urls),
    }
}

/// The Shortcuts' covers while the grid may yet tell a new tile size for them (it does as
/// it's made, in the frame that first draws it, and in the next turn of the event loop when
/// the number of tiles moves it a step): they wait for it rather than be decoded at one size
/// and again at another. Told nothing new, or in a window that draws no frame for it (a
/// `--snapshot` waits for its covers first), they're asked for at the last size (at first the
/// largest) after two turns.
fn wait_for_tile(ctx: &Ctx, rows: Rc<VecModel<CardItem>>, urls: Vec<Option<String>>) {
    STATE.with(|s| s.borrow_mut().shortcut_wait = Some((rows, urls)));
    let covers = ctx.covers.clone();
    let busy = crate::busy::Busy::on_timers();
    // Two turns: the first may come before the frame that makes the grid.
    slint::Timer::single_shot(Duration::ZERO, move || {
        slint::Timer::single_shot(Duration::ZERO, move || {
            let (wait, px) = STATE.with(|s| {
                let mut s = s.borrow_mut();
                (s.shortcut_wait.take(), s.shortcut_px)
            });
            if let Some((rows, urls)) = wait {
                let px = px.unwrap_or(SHORTCUT_COVER);
                covers.cards(PageKind::Home, &rows, urls, px);
            }
            drop(busy);
        });
    });
}

/// The grid's tile size, as it's made and as it changes (`HomeData.shortcut-tile`): the
/// covers waiting for it are asked for at it, or the ones shown are decoded again for it.
fn shortcut_tile(ctx: &Ctx, px: f32) {
    let px = px.round() as u32;
    if px == 0 {
        return;
    }
    let (wait, cards, changed) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let changed = s.shortcut_px.replace(px) != Some(px);
        (s.shortcut_wait.take(), s.shortcut_cards.upgrade(), changed)
    });
    if let Some((rows, urls)) = wait {
        ctx.covers.cards(PageKind::Home, &rows, urls, px);
    } else if let Some(rows) = cards.filter(|_| changed) {
        ctx.covers.resize_cards(&rows, px);
    }
}

/// `--snapshot home-shortcuts*`: the row as Home's next visit shows it.
pub fn show_shortcuts_for_snapshot(ctx: &Ctx) {
    show_shortcuts(ctx, true);
}

/// A Shortcuts tile's cover, in logical pixels, until the grid tells its size: the largest
/// tile's (home.slint's `ShortcutSizes.tile`).
const SHORTCUT_COVER: u32 = 104;

/// What plays now came from (`PlayingFrom::page`): its Shortcuts tile shows the playing mark.
pub fn playing_from_changed(ctx: &Ctx, state: &PlayerState) {
    let Some(ui) = ctx.window() else { return };
    let (kind, id) = match &state.queue.playing_from.page {
        Some(Place::Album(id)) => (ItemKind::Album, id.0.as_str()),
        Some(Place::Playlist(id)) => (
            ItemKind::Playlist,
            featherplay_ytm::convert::bare_playlist_id(&id.0),
        ),
        Some(Place::Artist(id)) => (ItemKind::Artist, id.0.as_str()),
        _ => (ItemKind::Song, ""),
    };
    let data = ui.global::<HomeData>();
    if data.get_playing_id() != id || data.get_playing_kind() != kind {
        data.set_playing_kind(kind);
        data.set_playing_id(id.into());
    }
}

/// A later page's shelves, under the ones shown.
fn add_shelves(ctx: &Ctx, shelves: &[HomeShelf]) {
    let Some(rows) = STATE.with(|s| s.borrow().rows.clone()) else {
        return;
    };
    sidebar::seen_on_shelves(shelves);
    let shelves = &without_liked(shelves);
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

/// The controller's state changed (track, play/pause, queue): the band follows it while Home
/// is on screen. Elsewhere, minimised or in the tray, nothing is loaded for it (its cover,
/// its rows' covers, the album's year); `open` and `shown_again` catch up.
pub fn player_changed(ctx: &Rc<Ctx>, state: &PlayerState) {
    playing_from_changed(ctx, state);
    if on_screen(ctx) {
        show_hero(ctx);
    }
}

/// The current song's related tracks came (the Related tab asked for them): the band fills a
/// short queue with them.
pub fn related_changed(ctx: &Rc<Ctx>) {
    if on_screen(ctx) {
        show_hero(ctx);
    }
}

/// The window shows again (`memory::hidden`): on Home, the band catches up with the player.
pub fn shown_again(ctx: &Rc<Ctx>) {
    if on_screen(ctx) {
        show_hero(ctx);
    }
}

/// Whether Home shows: its page, in a window that shows something.
fn on_screen(ctx: &Ctx) -> bool {
    ctx.current_page() == Page::Home && !crate::memory::is_hidden()
}

/// Signed in or out: the feed is the account's, so it's fetched again.
pub fn account_changed(ctx: &Rc<Ctx>, signed_in: bool) {
    let current = owner_of(&ctx.catalog.source());
    let cleared = STATE.with(|s| account_news(&mut s.borrow_mut(), signed_in, current));
    if let Some(ui) = ctx.window()
        && cleared
    {
        let data = ui.global::<HomeData>();
        data.set_shelves(Default::default());
        data.set_empty(false);
        data.set_more(MoreState::Idle);
    }
    show_shortcuts(ctx, true);
    if ctx.current_page() == Page::Home {
        open(ctx);
    }
}

/// What the tests in `home/` build their shelves from.
#[cfg(test)]
mod test_items {
    use featherplay_core::model::{ArtistId, PlaylistId, VideoType};
    use featherplay_ytm::{AlbumSummary, ArtistSummary, PlaylistSummary};

    use super::*;
    pub(super) use crate::testing::track;

    pub(super) fn shelf(title: &str, items: Vec<HomeItem>) -> HomeShelf {
        HomeShelf {
            title: title.into(),
            strapline: None,
            items,
        }
    }

    pub(super) fn album() -> HomeItem {
        HomeItem::Album(AlbumSummary {
            id: featherplay_core::model::AlbumId("MPREb_a".into()),
            title: "An album".into(),
            artists: Vec::new(),
            year: None,
            thumbnail: None,
            single: false,
        })
    }

    pub(super) fn video(id: &str) -> HomeItem {
        let mut t = track(id);
        t.video_type = VideoType::UserVideo;
        HomeItem::Track(t)
    }

    pub(super) fn song(id: &str) -> HomeItem {
        HomeItem::Track(track(id))
    }

    pub(super) fn playlist(id: &str) -> HomeItem {
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

    pub(super) fn artist() -> HomeItem {
        HomeItem::Artist(ArtistSummary {
            id: ArtistId("UCa".into()),
            name: "An artist".into(),
            thumbnail: None,
        })
    }
}
