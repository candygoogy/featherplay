//! The sidebar: the library as a list under three chips (Playlists, Albums, Artists; loaded
//! once signed in, each list when its chip is first picked, with 24 px covers) and the
//! account row (the account's photo and name, fetched once per session and identity; a
//! click opens the account menu, `account_menu`). The nav items go straight to `Nav` in
//! Slint; search is the top bar's (`top_bar`).
//!
//! The list's order (`library_list::order`): what the user pinned from the right-click menu
//! (kept in the settings, per identity), then what they opened or played lately (kept in the
//! library cache, per identity, on this device only), then the library's own order. A list
//! is ordered when it's shown (signing in, a chip, a pin), not while the user clicks
//! through it, so a row never jumps away under the pointer. The same record of what was
//! opened gives Home its shortcuts (`shortcuts`).

use std::cell::RefCell;
use std::rc::Rc;

use featherplay_controller::catalog::Source;
use featherplay_core::tr;
use featherplay_store::{LibraryCache, LibraryList, Paths};
use featherplay_ytm::{AccountInfo, AlbumSummary, ArtistSummary, HomeItem, PlaylistSummary};
use slint::{ComponentHandle, Image, ModelRc, VecModel};

use crate::app::Ctx;
use crate::library_list::{self, Entry, Kind};
use crate::pages::{home, settings};
use crate::ui::{CardItem, ItemKind, SidebarData};
use crate::ui_bridge::{
    album_subtitle, artist_card, model, playlist_card, playlist_subtitle, shown,
};
use crate::{account_menu, playlist_menu};

/// The sidebar's list covers, in logical pixels.
const COVER: u32 = 24;
/// The account photo, in logical pixels: decoded for its largest use, the avatar on
/// Settings' Discord card (76 inside its ring); the sidebar's 32 px row shows it smaller.
const AVATAR: u32 = 76;
/// Home's shortcuts after Liked songs.
const SHORTCUTS: usize = 3;

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let c = ctx.clone();
    ui.global::<SidebarData>()
        .on_chip_picked(move |i| pick_chip(&c, chip_kind(i)));
}

pub fn account_changed(ctx: &Rc<Ctx>, signed_in: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SidebarData>();
    data.set_signed_in(signed_in);
    data.set_signed_out(!signed_in);
    // A new session (or none): what was fetched belonged to the old one.
    forget_account();
    forget_library(ctx);
    if !signed_in {
        show_account(ctx, None);
        data.set_playlists(Default::default());
        return;
    }
    load_account(ctx);
    load_library(ctx);
}

/// The app acts as another identity: its name, photo and library, fetched again. The
/// row keeps the old ones until the new ones arrive.
pub fn identity_changed(ctx: &Rc<Ctx>) {
    forget_account();
    forget_library(ctx);
    load_account(ctx);
    load_library(ctx);
}

fn forget_account() {
    ACCOUNT.with_borrow_mut(|a| {
        a.session += 1;
        a.info = None;
        a.fetching = false;
    });
}

/// The account row's state for this session, on the UI thread.
#[derive(Default)]
struct Account {
    /// Bumped on sign-out, so a late answer for the old session is dropped.
    session: u64,
    info: Option<AccountInfo>,
    fetching: bool,
}

/// The library list's state for this session and identity, on the UI thread.
#[derive(Default)]
struct Library {
    /// Bumped on every account or identity change, so a late answer is dropped.
    session: u64,
    /// The identity key pins and the recent list are kept under; `None` keeps them in
    /// memory only (fixtures, signed out).
    key: Option<String>,
    playlists: Option<Vec<Entry>>,
    albums: Option<Vec<Entry>>,
    artists: Option<Vec<Entry>>,
    fetching: [bool; 3],
    chip: Option<Kind>,
    pins: Vec<String>,
    recent: Vec<Entry>,
}

impl Library {
    fn list(&self, kind: Kind) -> Option<&Vec<Entry>> {
        match kind {
            Kind::Playlist => self.playlists.as_ref(),
            Kind::Album => self.albums.as_ref(),
            Kind::Artist => self.artists.as_ref(),
        }
    }
}

thread_local! {
    static ACCOUNT: RefCell<Account> = RefCell::new(Account::default());
    static LIBRARY: RefCell<Library> = RefCell::new(Library::default());
}

fn chip_kind(i: i32) -> Kind {
    match i {
        1 => Kind::Album,
        2 => Kind::Artist,
        _ => Kind::Playlist,
    }
}

fn chip_index(kind: Kind) -> i32 {
    match kind {
        Kind::Playlist => 0,
        Kind::Album => 1,
        Kind::Artist => 2,
    }
}

fn slot(kind: Kind) -> usize {
    chip_index(kind) as usize
}

/// Fetches the account's name and photo once per session and identity (at startup when
/// signed in, after each sign-in and switch) and keeps it in memory. A failure leaves the generic row
/// ("Signed in" with the person glyph); it isn't retried until the next sign-in or switch.
fn load_account(ctx: &Rc<Ctx>) {
    let (session, cached) = ACCOUNT.with_borrow(|a| (a.session, a.info.clone()));
    if let Some(info) = cached {
        show_account(ctx, Some(&info));
        return;
    }
    let busy = ACCOUNT.with_borrow_mut(|a| std::mem::replace(&mut a.fetching, true));
    if busy {
        return;
    }
    ctx.load(
        |source| async move { source.account_info().await },
        move |ctx, _ui, result| {
            let current = ACCOUNT.with_borrow_mut(|a| {
                let current = a.session == session;
                if current {
                    a.fetching = false;
                    if let Ok(info) = &result {
                        a.info = Some(info.clone());
                    }
                }
                current
            });
            if !current {
                return;
            }
            match result {
                Ok(info) => show_account(ctx, Some(&info)),
                // The error never carries the account's data.
                Err(e) => log!("account info: {e}"),
            }
        },
    );
}

/// The account row: the name, the handle and the round photo, or the generic row
/// ("Signed in" with the person glyph) for `None`. Until the photo arrives, and if it
/// can't be loaded, the avatar shows the name's initial.
fn show_account(ctx: &Ctx, info: Option<&AccountInfo>) {
    let Some(ui) = ctx.window() else { return };
    account_menu::active_handle(ctx, info.and_then(|i| i.handle.as_deref()));
    let data = ui.global::<SidebarData>();
    let name = info.map(|i| i.name.trim()).unwrap_or_default();
    data.set_display_name(name.into());
    let handle = info.and_then(|i| i.handle.as_deref()).unwrap_or_default();
    data.set_handle(handle.trim().into());
    let initial: String = name
        .chars()
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect();
    data.set_initial(initial.into());
    data.set_has_avatar(false);
    data.set_avatar(Image::default());
    let Some(photo) = info.and_then(|i| i.photo.as_deref()) else {
        return;
    };
    let session = ACCOUNT.with_borrow(|a| a.session);
    let weak = ctx.ui.clone();
    ctx.covers.load(photo, AVATAR, move |img| {
        if ACCOUNT.with_borrow(|a| a.session) != session {
            return;
        }
        if let Some(ui) = weak.upgrade() {
            let data = ui.global::<SidebarData>();
            data.set_avatar(img);
            data.set_has_avatar(true);
        }
    });
}

/// Drops the old session's lists, pins and recent record.
fn forget_library(ctx: &Ctx) {
    LIBRARY.with_borrow_mut(|l| {
        let session = l.session + 1;
        let chip = l.chip;
        *l = Library {
            session,
            chip,
            ..Library::default()
        };
    });
    if let Some(ui) = ctx.window() {
        let data = ui.global::<SidebarData>();
        data.set_items(Default::default());
        data.set_pinned(Default::default());
    }
}

/// The identity key the source acts as, for what's kept per identity. Fixtures and no
/// session keep nothing.
fn identity_key(source: &Source) -> Option<String> {
    match source {
        Source::Live(api) => Some(api.identity().key()),
        _ => None,
    }
}

/// After a sign-in or a switch: the pins (settings), the recent record (the cache, read off
/// the UI thread), the playlists, and the chip's list.
fn load_library(ctx: &Rc<Ctx>) {
    let key = identity_key(&ctx.catalog.source());
    let pins = key.as_deref().map(settings::pins).unwrap_or_default();
    let session = LIBRARY.with_borrow_mut(|l| {
        l.key = key.clone();
        l.pins = pins;
        l.session
    });
    if let Some(key) = key {
        let task = ctx.catalog.runtime().spawn_blocking(move || {
            LibraryCache::new(&Paths::resolve()).load::<Vec<Entry>>(&key, LibraryList::Recent)
        });
        let c = ctx.clone();
        let spawned = slint::spawn_local(async move {
            let recent = task.await.ok().flatten().unwrap_or_default();
            let current = LIBRARY.with_borrow_mut(|l| {
                let current = l.session == session;
                if current {
                    // Anything noted meanwhile stays in front.
                    let mut merged = l.recent.clone();
                    for e in recent.into_iter().rev() {
                        if !merged.iter().any(|m| m.key() == e.key()) {
                            merged.push(e);
                        }
                    }
                    merged.truncate(library_list::MAX_RECENT);
                    l.recent = merged;
                }
                current
            });
            if current {
                show(&c);
                home::shortcuts_changed(&c);
            }
        });
        if let Err(e) = spawned {
            log!("sidebar recent: {e}");
        }
    }
    load_playlists(ctx);
    let chip = LIBRARY.with_borrow(|l| l.chip).unwrap_or(Kind::Playlist);
    if chip != Kind::Playlist {
        fetch(ctx, chip);
    }
}

/// Fetches the library playlists (again after a change to them).
pub fn load_playlists(ctx: &Rc<Ctx>) {
    LIBRARY.with_borrow_mut(|l| l.fetching[slot(Kind::Playlist)] = false);
    fetch(ctx, Kind::Playlist);
}

/// One call for the `kind` list, unless one is on its way.
fn fetch(ctx: &Rc<Ctx>, kind: Kind) {
    let session = LIBRARY.with_borrow_mut(|l| {
        let busy = std::mem::replace(&mut l.fetching[slot(kind)], true);
        (!busy).then_some(l.session)
    });
    let Some(session) = session else { return };
    ctx.load(
        move |source| async move {
            match kind {
                Kind::Playlist => source.library_playlists().await.map(Fetched::Playlists),
                Kind::Album => source.library_albums().await.map(Fetched::Albums),
                Kind::Artist => source.library_artists().await.map(Fetched::Artists),
            }
        },
        move |ctx, ui, result| {
            let current = LIBRARY.with_borrow_mut(|l| {
                let current = l.session == session;
                if current {
                    l.fetching[slot(kind)] = false;
                }
                current
            });
            if !current {
                return;
            }
            let fetched = match result {
                Ok(f) => f,
                Err(e) => {
                    log!("sidebar {kind:?}: {e}");
                    return;
                }
            };
            let entries = match &fetched {
                Fetched::Playlists(p) => {
                    // The right-click menu offers Rename and Delete on the account's own.
                    playlist_menu::remember(p);
                    // The song menu's Add to playlist lists them in the library's order.
                    let (rows, model) = model(p.iter().map(playlist_card).collect());
                    ui.global::<SidebarData>().set_playlists(model);
                    let urls = p.iter().map(|p| p.thumbnail.clone()).collect();
                    ctx.covers
                        .fill(&rows, urls, COVER, |row, img| row.cover = img);
                    p.iter().map(playlist_entry).collect()
                }
                Fetched::Albums(a) => a.iter().map(album_entry).collect(),
                Fetched::Artists(a) => a.iter().map(artist_entry).collect(),
            };
            LIBRARY.with_borrow_mut(|l| match kind {
                Kind::Playlist => l.playlists = Some(entries),
                Kind::Album => l.albums = Some(entries),
                Kind::Artist => l.artists = Some(entries),
            });
            show(ctx);
            if kind == Kind::Playlist {
                home::shortcuts_changed(ctx);
            }
        },
    );
}

/// What a list's call brought.
enum Fetched {
    Playlists(Vec<PlaylistSummary>),
    Albums(Vec<AlbumSummary>),
    Artists(Vec<ArtistSummary>),
}

/// A chip was picked: its list, fetched the first time.
fn pick_chip(ctx: &Rc<Ctx>, kind: Kind) {
    let loaded = LIBRARY.with_borrow_mut(|l| {
        l.chip = Some(kind);
        l.list(kind).is_some()
    });
    if let Some(ui) = ctx.window() {
        ui.global::<SidebarData>().set_chip(chip_index(kind));
    }
    show(ctx);
    if !loaded {
        fetch(ctx, kind);
    }
}

/// Shows the chip's list in order: pinned, played lately, the rest.
fn show(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SidebarData>();
    let (kind, shown) = LIBRARY.with_borrow(|l| {
        let kind = l.chip.unwrap_or(Kind::Playlist);
        let shown = l
            .list(kind)
            .map(|items| library_list::order(items, &l.pins, &l.recent));
        (kind, shown)
    });
    data.set_chip(chip_index(kind));
    let Some(shown) = shown else {
        data.set_items(Default::default());
        data.set_pinned(Default::default());
        data.set_empty_text("".into());
        return;
    };
    data.set_empty_text(
        if shown.is_empty() {
            empty_text(kind)
        } else {
            ""
        }
        .into(),
    );
    let pinned: Vec<bool> = shown.iter().map(|(_, p)| *p).collect();
    let urls = shown.iter().map(|(e, _)| e.thumbnail.clone()).collect();
    let (rows, items) = model(shown.iter().map(|(e, _)| card(e)).collect());
    data.set_items(items);
    data.set_pinned(ModelRc::new(VecModel::from(pinned)));
    ctx.covers
        .fill(&rows, urls, COVER, |row, img| row.cover = img);
}

/// What the list says when the library has none of `kind`.
fn empty_text(kind: Kind) -> &'static str {
    match kind {
        Kind::Playlist => tr!("Playlists you make or save show up here."),
        Kind::Album => tr!("Albums you save show up here."),
        Kind::Artist => tr!("Artists you follow show up here."),
    }
}

fn card(e: &Entry) -> CardItem {
    CardItem {
        id: e.id.as_str().into(),
        kind: match e.kind {
            Kind::Playlist => ItemKind::Playlist,
            Kind::Album => ItemKind::Album,
            Kind::Artist => ItemKind::Artist,
        },
        title: shown(&e.title),
        subtitle: shown(&e.subtitle),
        cover: Image::default(),
        round: e.kind == Kind::Artist,
    }
}

pub fn playlist_entry(p: &PlaylistSummary) -> Entry {
    Entry {
        kind: Kind::Playlist,
        id: featherplay_ytm::convert::bare_playlist_id(&p.id.0).to_string(),
        title: p.title.clone(),
        subtitle: playlist_subtitle(p),
        thumbnail: p.thumbnail.clone(),
    }
}

pub fn album_entry(a: &AlbumSummary) -> Entry {
    Entry {
        kind: Kind::Album,
        id: a.id.0.clone(),
        title: a.title.clone(),
        subtitle: album_subtitle(a),
        thumbnail: a.thumbnail.clone(),
    }
}

pub fn artist_entry(a: &ArtistSummary) -> Entry {
    Entry {
        kind: Kind::Artist,
        id: a.id.0.clone(),
        title: a.name.clone(),
        subtitle: artist_card(a).subtitle.to_string(),
        thumbnail: a.thumbnail.clone(),
    }
}

/// A playlist, album or artist was opened or played: it goes to the front of the recent
/// record (saved off the UI thread). The sidebar's order follows the next time a list is
/// shown; Home's shortcuts follow now.
pub fn opened(ctx: &Ctx, e: Entry) {
    let saved = LIBRARY.with_borrow_mut(|l| {
        library_list::note(&mut l.recent, e);
        l.key.clone().map(|k| (k, l.recent.clone()))
    });
    if let Some((key, recent)) = saved {
        crate::task::blocking_detached(ctx.catalog.runtime(), move || {
            let cache = LibraryCache::new(&Paths::resolve());
            if let Err(e) = cache.save(&key, LibraryList::Recent, &recent) {
                log!("sidebar recent: saving failed: {}", e.kind());
            }
        });
    }
    home::shortcuts_changed(ctx);
}

/// Home's shortcuts after Liked songs: the playlists and albums opened or played lately,
/// then the library's playlists and albums.
pub fn shortcuts() -> Vec<HomeItem> {
    LIBRARY.with_borrow(|l| {
        let library: Vec<Entry> = l
            .playlists
            .iter()
            .chain(l.albums.iter())
            .flatten()
            .cloned()
            .collect();
        library_list::shortcuts(&l.recent, &library, SHORTCUTS)
            .iter()
            .map(home_item)
            .collect()
    })
}

fn home_item(e: &Entry) -> HomeItem {
    match e.kind {
        Kind::Album => HomeItem::Album(AlbumSummary {
            id: featherplay_core::model::AlbumId(e.id.clone()),
            title: e.title.clone(),
            artists: Vec::new(),
            year: None,
            thumbnail: e.thumbnail.clone(),
        }),
        Kind::Artist => HomeItem::Artist(ArtistSummary {
            id: featherplay_core::model::ArtistId(e.id.clone()),
            name: e.title.clone(),
            thumbnail: e.thumbnail.clone(),
        }),
        Kind::Playlist => HomeItem::Playlist(PlaylistSummary {
            id: featherplay_core::model::PlaylistId(e.id.clone()),
            title: e.title.clone(),
            author: None,
            count: None,
            views: None,
            avatar: None,
            thumbnail: e.thumbnail.clone(),
            owned: false,
        }),
    }
}

/// Whether the entry is pinned to the top of the list.
pub fn is_pinned(kind: Kind, id: &str) -> bool {
    let key = library_list::key(kind, id);
    LIBRARY.with_borrow(|l| l.pins.contains(&key))
}

/// Pins the entry to the top of its list, or unpins it, and keeps that in the settings
/// for this identity. True when it's pinned now.
pub fn toggle_pin(ctx: &Ctx, kind: Kind, id: &str) -> bool {
    let key = library_list::key(kind, id);
    let (pinned, saved) = LIBRARY.with_borrow_mut(|l| {
        let pinned = library_list::toggle_pin(&mut l.pins, &key);
        (pinned, l.key.clone().map(|k| (k, l.pins.clone())))
    });
    if let Some((identity, pins)) = saved {
        settings::set_pins(ctx, &identity, pins);
    }
    show(ctx);
    pinned
}

/// `--snapshot sidebar-*`: the list on a chip, with the second entry pinned (in memory).
pub fn show_for_snapshot(ctx: &Rc<Ctx>, kind: Kind) {
    pick_chip(ctx, kind);
}

/// `--snapshot sidebar-*`, once the list has loaded: pins its third entry (in memory), so
/// the pin shows.
pub fn pin_for_snapshot(ctx: &Ctx) {
    let entry = LIBRARY.with_borrow(|l| {
        let kind = l.chip.unwrap_or(Kind::Playlist);
        l.list(kind).and_then(|list| list.get(2).cloned())
    });
    if let Some(e) = entry
        && !is_pinned(e.kind, &e.id)
    {
        toggle_pin(ctx, e.kind, &e.id);
    }
}
