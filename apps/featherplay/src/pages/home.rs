//! Home (ui/pages/home.slint): the hero, then the Home feed shelves, each in the shape of what
//! it holds (`arrange`), your mixes first.
//!
//! The feed is fetched once and kept for `FEED_TTL` (Home is opened often; the feed changes
//! slowly), and again after a sign-in. It goes on as the user scrolls near its end, a page at
//! a time (`Paging`): never two at once, at most `MAX_PAGES` per feed, and none without a
//! scroll. Liked songs is pinned as the first shortcut (`with_liked`). The hero follows the
//! player: `choose_hero` picks "pick up where you left off" when there's a current track,
//! else a featured pick from the first shelf.
//!
//! Signed in, the first pages' shelves are kept on disk for the identity (`CachedFeed`), so
//! the next start shows Home at once from them while the feed is fetched behind. A fresh feed
//! that differs doesn't replace the cached one under the user: it waits for Home's next open,
//! and its later pages go on under the cached shelves as the user scrolls.
//! Another identity's feed is never shown, nor any signed out; a day-old one isn't either.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay_core::model::{Track, TrackId};
use featherplay_ytm::{HomeFeed, HomeItem, HomeShelf};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::app::Ctx;
use crate::controller::{Action, PlayerState};
use crate::follow::colour::Lch;
use crate::pages::content;
use crate::router::Page;
use crate::sidebar;
use crate::song_menu;
use crate::ui::{AppWindow, HomeData, HomeShelfRow, MoreState, PageKind, ShelfShape};
use crate::ui_bridge::model;

mod feed;
mod hero;
mod shelves;

pub use feed::{MAX_PAGES, More, Paging};
use feed::{Owner, account_news, fetch, more, owner_of, show_cached, show_feed};
use hero::show_hero;
pub use hero::{Hero, choose_hero};
pub(crate) use shelves::shelf_row;
pub use shelves::{arrange, with_liked};
use shelves::{is_shortcuts, shape};

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

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(ctx: &Rc<Ctx>, _state: &PlayerState) {
    show_hero(ctx);
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
    if ctx.current_page() == Page::Home {
        open(ctx);
    }
}

/// What the tests in `home/` build their shelves from.
#[cfg(test)]
mod test_items {
    use featherplay_core::model::{ArtistId, LikeStatus, PlaylistId, VideoType};
    use featherplay_ytm::{AlbumSummary, ArtistSummary, PlaylistSummary};

    use super::*;

    pub(super) fn track(id: &str) -> Track {
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
