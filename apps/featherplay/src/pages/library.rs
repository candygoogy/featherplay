//! Library page (ui/pages/library.slint): Liked (song rows, covers as they scroll in),
//! Playlists, Albums and Artists (card grids).
//!
//! Each list shows at once from what it last held: still in memory from an earlier visit,
//! or the on-disk cache of the last good result (`featherplay_store::LibraryCache`, per
//! identity), read off the UI thread. One call per visit refreshes it behind that, and the
//! fresh result updates the rows in place, so the list keeps its scroll position and the
//! skeleton shows only when there is nothing at all to show. A good result replaces the
//! cache; a failed refresh keeps the list on screen.

use std::cell::{Cell, RefCell};
use std::io;
use std::rc::Rc;
use std::time::Instant;

use featherplay_core::model::Track;
use featherplay_store::{LibraryCache, LibraryList, Paths, Settings};
use featherplay_ytm::{AlbumSummary, ApiError, ArtistSummary, PlaylistSummary};

use slint::{ComponentHandle, Model, VecModel};

use crate::app::Ctx;
use crate::busy::Busy;
use crate::catalog::Source;
use crate::controller::PlayerState;
use crate::pages::content::{self, CARD_COVER, TrackList};
use crate::router::LibraryTab;
use crate::ui::{AppWindow, CardItem, LibraryData, PageKind};
use crate::ui_bridge::{album_card, artist_card, model, playlist_card};

thread_local! {
    static LIKED: RefCell<Option<TrackList>> = const { RefCell::new(None) };
    /// The card tab `LibraryData.cards` holds, and its rows.
    static CARDS: RefCell<Option<(LibraryTab, Rc<VecModel<CardItem>>)>> = const { RefCell::new(None) };
    /// Bumped when the account or identity changes: results asked for before are dropped.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    /// The account changes seen so far (the first is startup's, which changes nothing).
    static ACCOUNT_EVENTS: Cell<u64> = const { Cell::new(0) };
}

/// Once at startup: connect `LibraryData`'s callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<LibraryData>();
    let c = ctx.clone();
    data.on_play(move |i| with_liked(|l| l.play(&c, i.max(0) as usize, false)));
    let c = ctx.clone();
    data.on_like(move |i| with_liked(|l| l.toggle_like(&c, i.max(0) as usize)));
    let c = ctx.clone();
    data.on_cover_needed(move |i| with_liked(|l| l.need_cover(&c.covers, i.max(0) as usize)));
    let c = ctx.clone();
    data.on_play_card(move |it| content::play_card(&c, &it));
    let c = ctx.clone();
    data.on_retry(move || {
        // While the stored session can't connect, this load waits for a reconnect.
        c.reconnect();
        if let Some(ui) = c.window() {
            open(
                &c,
                LibraryTab::from_index(ui.global::<LibraryData>().get_tab()),
            );
        }
    });
}

fn with_liked(f: impl FnOnce(&mut TrackList)) {
    LIKED.with(|l| {
        if let Some(list) = l.borrow_mut().as_mut() {
            f(list);
        }
    });
}

fn generation() -> u64 {
    GENERATION.with(Cell::get)
}

/// Whether `tab`'s list is on the page already (from memory, the cache or a refresh).
fn showing(tab: LibraryTab) -> bool {
    match tab {
        LibraryTab::Liked => LIKED.with(|l| l.borrow().is_some()),
        _ => CARDS.with(|c| c.borrow().as_ref().is_some_and(|(t, _)| *t == tab)),
    }
}

/// The page is being shown.
pub fn open(ctx: &Rc<Ctx>, tab: LibraryTab) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<LibraryData>();
    data.set_tab(tab.index());
    data.set_error("".into());
    // Another grid tab's cards go; Liked's rows stay for when it's shown again.
    if tab != LibraryTab::Liked && !showing(tab) {
        CARDS.with(|c| *c.borrow_mut() = None);
        data.set_cards(Default::default());
    }
    data.set_loading(true);
    let started = Instant::now();
    if !showing(tab) {
        show_cached(ctx, tab, started);
    }
    refresh(ctx, tab, started);
}

/// Forgets the lists on the page and in memory (not the cache on disk); loads asked for
/// before are dropped.
pub fn forget(ctx: &Ctx) {
    GENERATION.with(|g| g.set(g.get() + 1));
    LIKED.with(|l| *l.borrow_mut() = None);
    CARDS.with(|c| *c.borrow_mut() = None);
    if let Some(ui) = ctx.window() {
        let data = ui.global::<LibraryData>();
        data.set_liked(Default::default());
        data.set_cards(Default::default());
    }
}

/// Which identity's cache to read: the live session's; while the stored session is still
/// connecting or can't connect, the one the account menu saved (`Settings::identity`); none
/// signed out or with fixtures.
enum CacheKey {
    Known(String),
    Saved,
}

impl CacheKey {
    fn of(source: &Source, pending: bool) -> Option<CacheKey> {
        match source {
            Source::Live(api) => Some(CacheKey::Known(api.identity().key())),
            Source::Unreachable => Some(CacheKey::Saved),
            Source::SignedOut(_) if pending => Some(CacheKey::Saved),
            Source::SignedOut(_) | Source::Fixtures(_) => None,
        }
    }

    /// Blocks (the settings file).
    fn resolve(self, paths: &Paths) -> String {
        match self {
            CacheKey::Known(key) => key,
            CacheKey::Saved => Settings::load(paths).identity.unwrap_or_default(),
        }
    }
}

/// The identity key of the session a result came from, to cache it under.
pub(crate) fn source_key(source: &Source) -> Option<String> {
    match source {
        Source::Live(api) => Some(api.identity().key()),
        _ => None,
    }
}

fn list_of(tab: LibraryTab) -> LibraryList {
    match tab {
        LibraryTab::Liked => LibraryList::Liked,
        LibraryTab::Playlists => LibraryList::Playlists,
        LibraryTab::Albums => LibraryList::Albums,
        LibraryTab::Artists => LibraryList::Artists,
    }
}

/// A tab's list as the API gives it, so the cache holds no UI types.
enum Lists {
    Liked(Vec<Track>),
    Playlists(Vec<PlaylistSummary>),
    Albums(Vec<AlbumSummary>),
    Artists(Vec<ArtistSummary>),
}

impl Lists {
    fn len(&self) -> usize {
        match self {
            Lists::Liked(l) => l.len(),
            Lists::Playlists(l) => l.len(),
            Lists::Albums(l) => l.len(),
            Lists::Artists(l) => l.len(),
        }
    }

    fn read(cache: &LibraryCache, key: &str, tab: LibraryTab) -> Option<Lists> {
        let list = list_of(tab);
        Some(match tab {
            LibraryTab::Liked => Lists::Liked(cache.load(key, list)?),
            LibraryTab::Playlists => Lists::Playlists(cache.load(key, list)?),
            LibraryTab::Albums => Lists::Albums(cache.load(key, list)?),
            LibraryTab::Artists => Lists::Artists(cache.load(key, list)?),
        })
    }

    async fn fetch(source: &Source, tab: LibraryTab) -> Result<Lists, ApiError> {
        Ok(match tab {
            LibraryTab::Liked => Lists::Liked(source.liked_songs().await?),
            LibraryTab::Playlists => Lists::Playlists(source.library_playlists().await?),
            LibraryTab::Albums => Lists::Albums(source.library_albums().await?),
            LibraryTab::Artists => Lists::Artists(source.library_artists().await?),
        })
    }

    /// Replaces the cache of `key`'s list, off the UI thread.
    fn save(&self, ctx: &Ctx, key: String, tab: LibraryTab) {
        type Save = Box<dyn FnOnce(&LibraryCache, &str, LibraryList) -> io::Result<()> + Send>;
        let save: Save = match self {
            Lists::Liked(l) => {
                let l = l.clone();
                Box::new(move |c, k, list| c.save(k, list, &l))
            }
            Lists::Playlists(l) => {
                let l = l.clone();
                Box::new(move |c, k, list| c.save(k, list, &l))
            }
            Lists::Albums(l) => {
                let l = l.clone();
                Box::new(move |c, k, list| c.save(k, list, &l))
            }
            Lists::Artists(l) => {
                let l = l.clone();
                Box::new(move |c, k, list| c.save(k, list, &l))
            }
        };
        ctx.catalog.runtime().spawn_blocking(move || {
            let cache = LibraryCache::new(&Paths::resolve());
            if let Err(e) = save(&cache, &key, list_of(tab)) {
                log!("library cache: saving {tab:?} failed: {}", e.kind());
            }
        });
    }
}

/// Shows the cached list of `tab`, if there is one, unless the page has moved on or a
/// fresh result got there first.
fn show_cached(ctx: &Rc<Ctx>, tab: LibraryTab, started: Instant) {
    let Some(key) = CacheKey::of(&ctx.catalog.source(), ctx.session_pending()) else {
        return;
    };
    let (visit, generation_then) = (ctx.visit(), generation());
    let busy = Busy::start();
    let task = ctx.catalog.runtime().spawn_blocking(move || {
        let paths = Paths::resolve();
        let key = key.resolve(&paths);
        Lists::read(&LibraryCache::new(&paths), &key, tab)
    });
    let ctx = ctx.clone();
    let spawned = slint::spawn_local(async move {
        let cached = task.await.ok().flatten();
        let current = visit == ctx.visit() && generation_then == generation();
        if let Some(lists) = cached
            && current
            && !showing(tab)
            && let Some(ui) = ctx.window()
        {
            log!(
                "library {tab:?}: {} from the cache in {} ms",
                lists.len(),
                started.elapsed().as_millis()
            );
            show(&ctx, &ui, tab, lists);
        }
        drop(busy);
    });
    if let Err(e) = spawned {
        log!("library cache: {e}");
    }
}

/// One call for `tab`'s list; a good result updates the page in place and the cache.
fn refresh(ctx: &Rc<Ctx>, tab: LibraryTab, started: Instant) {
    let generation_then = generation();
    ctx.load_for_page(
        move |source| async move {
            let key = source_key(&source);
            (key, Lists::fetch(&source, tab).await)
        },
        move |ctx, ui, (key, result)| {
            if generation_then != generation() {
                return;
            }
            ui.global::<LibraryData>().set_loading(false);
            match result {
                Ok(lists) => {
                    log!(
                        "library {tab:?}: {} fetched in {} ms",
                        lists.len(),
                        started.elapsed().as_millis()
                    );
                    if let Some(key) = key {
                        lists.save(ctx, key, tab);
                    }
                    show(ctx, ui, tab, lists);
                }
                Err(e) => failed(ctx, ui, tab, &e),
            }
        },
    );
}

/// A refresh failed. A list on the page stays, unless the session has ended.
fn failed(ctx: &Ctx, ui: &AppWindow, tab: LibraryTab, e: &ApiError) {
    let what = match tab {
        LibraryTab::Liked => "liked songs",
        LibraryTab::Playlists => "library playlists",
        LibraryTab::Albums => "library albums",
        LibraryTab::Artists => "library artists",
    };
    if showing(tab) && !matches!(e, ApiError::AuthExpired) {
        log!("{what}: refreshing failed, the last list stays: {e}");
        return;
    }
    if showing(tab) {
        forget(ctx);
    }
    ui.global::<LibraryData>()
        .set_error(content::error_text(what, e).into());
}

fn show(ctx: &Rc<Ctx>, ui: &AppWindow, tab: LibraryTab, lists: Lists) {
    let cards = |items: Vec<(CardItem, Option<String>)>| show_cards(ctx, ui, tab, items);
    match lists {
        Lists::Liked(tracks) => show_liked(ctx, ui, tracks),
        Lists::Playlists(list) => cards(
            list.into_iter()
                // Liked songs has its own tab and its own sidebar row.
                .filter(|p| !crate::pages::playlist::is_liked_songs(&p.id))
                .map(|p| (playlist_card(&p), p.thumbnail))
                .collect(),
        ),
        Lists::Albums(list) => cards(
            list.into_iter()
                .map(|a| (album_card(&a), a.thumbnail))
                .collect(),
        ),
        Lists::Artists(list) => cards(
            list.into_iter()
                .map(|a| (artist_card(&a), a.thumbnail))
                .collect(),
        ),
    }
}

/// The first list builds the rows; later ones update them in place.
fn show_liked(ctx: &Rc<Ctx>, ui: &AppWindow, tracks: Vec<Track>) {
    let playing = content::playing(ctx.player().as_ref());
    let updated = LIKED.with(|l| match l.borrow_mut().as_mut() {
        Some(list) => {
            list.replace(tracks.clone(), playing.as_ref());
            true
        }
        None => false,
    });
    if updated {
        return;
    }
    let (list, model) = TrackList::new(tracks, ctx);
    list.hint_first(ctx);
    LIKED.with(|l| *l.borrow_mut() = Some(list));
    ui.global::<LibraryData>().set_liked(model);
}

/// Cards for a grid tab: a new model for a new tab, else the rows updated in place. A card
/// still in the list keeps its loaded cover.
fn show_cards(
    ctx: &Rc<Ctx>,
    ui: &AppWindow,
    tab: LibraryTab,
    items: Vec<(CardItem, Option<String>)>,
) {
    let existing = CARDS.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|(t, _)| *t == tab)
            .map(|(_, rows)| rows.clone())
    });
    let Some(rows) = existing else {
        let (items, urls): (Vec<_>, Vec<_>) = items.into_iter().unzip();
        let (rows, model) = model(items);
        CARDS.with(|c| *c.borrow_mut() = Some((tab, rows.clone())));
        ui.global::<LibraryData>().set_cards(model);
        ctx.covers.cards(PageKind::Library, &rows, urls, CARD_COVER);
        return;
    };
    let loaded: std::collections::HashMap<String, slint::Image> = rows
        .iter()
        .filter(|r| r.cover.size().width > 0)
        .map(|r| (r.id.to_string(), r.cover))
        .collect();
    let mut urls = Vec::with_capacity(items.len());
    for (i, (mut item, url)) in items.iter().cloned().enumerate() {
        match loaded.get(item.id.as_str()) {
            Some(img) => {
                item.cover = img.clone();
                urls.push(None);
            }
            None => urls.push(url),
        }
        if i < rows.row_count() {
            if rows.row_data(i).as_ref() != Some(&item) {
                rows.set_row_data(i, item);
            }
        } else {
            rows.push(item);
        }
    }
    while rows.row_count() > items.len() {
        rows.remove(rows.row_count() - 1);
    }
    ctx.covers.cards(PageKind::Library, &rows, urls, CARD_COVER);
}

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(_ctx: &Rc<Ctx>, state: &PlayerState) {
    let playing = content::playing(Some(state));
    with_liked(|l| l.mark_playing(playing.as_ref()));
}

/// Signed in or out, or another session: the lists were someone else's (startup's first
/// news changes nothing). A Library on screen loads again.
pub fn account_changed(ctx: &Rc<Ctx>, _signed_in: bool) {
    let first = ACCOUNT_EVENTS.with(|n| {
        n.set(n.get() + 1);
        n.get() == 1
    });
    if !first {
        identity_changed(ctx);
    }
}

/// The app acts as another identity: its lists are another's.
pub fn identity_changed(ctx: &Rc<Ctx>) {
    forget(ctx);
    if let crate::router::Page::Library { tab } = ctx.current_page() {
        open(ctx, tab);
    }
}
