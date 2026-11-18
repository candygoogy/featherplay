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
//! through it, so a row never jumps away under the pointer.
//!
//! Home's Shortcuts (`shortcuts`) share the pins, and learn from plays only, not opens: a
//! song played long enough from an album, a playlist or an artist (`played`, from the
//! controller's `UiEvent::Played`) puts it first in a record of its own. What's removed from
//! Shortcuts stays out until it's played again (`hide`). Both are kept beside the recent
//! record, with what the pins name (their titles and covers, for a pin outside the library).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use featherplay_controller::catalog::Source;
use featherplay_controller::place::Place;
use featherplay_core::model::Track;
use featherplay_core::tr;
use featherplay_store::{LibraryCache, LibraryList, Paths};
use featherplay_ytm::{
    AccountInfo, AlbumSummary, ArtistSummary, HomeItem, HomeShelf, PlaylistSummary,
};
use slint::{ComponentHandle, Image, ModelRc, VecModel};

use crate::app::Ctx;
use crate::library_list::{self, Entry, Kind, Shortcut, Sources};
use crate::pages::{home, settings};
use crate::ui::{CardItem, ItemKind, SidebarData};
use crate::ui_bridge::{artist_names, model, playlist_card, shown};
use crate::{account_menu, playlist_menu};

/// The sidebar's list covers, in logical pixels.
const COVER: u32 = 24;
/// The account photo, in logical pixels: decoded for its largest use, the avatar on
/// Settings' Discord card (76 inside its ring); the sidebar's 32 px row shows it smaller.
const AVATAR: u32 = 76;

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
    /// Played from lately, for Shortcuts; `played_shown` as Home last took it (`shortcuts`),
    /// so new plays wait for Home's next visit.
    played: Vec<Entry>,
    played_shown: Vec<Entry>,
    /// Removed from Shortcuts, by key.
    hidden: Vec<String>,
    /// What the pins name, so a pin outside the library still has its title and cover.
    pinned: Vec<Entry>,
    /// The Library page's albums and artists as last kept on disk, until the chips fetch
    /// their own: Shortcuts' library after the playlists.
    stored: Vec<Entry>,
    /// The playlists, albums and artists seen this session (pages opened, Home's feed), by
    /// key: what a play from one of them shows.
    seen: HashMap<String, Entry>,
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
        let task = ctx
            .catalog
            .runtime()
            .spawn_blocking(move || Kept::load(&LibraryCache::new(&Paths::resolve()), &key));
        let c = ctx.clone();
        let spawned = slint::spawn_local(async move {
            let kept = task.await.unwrap_or_default();
            let current = LIBRARY.with_borrow_mut(|l| {
                let current = l.session == session;
                if current {
                    // Anything noted meanwhile stays in front.
                    let mut merged = l.recent.clone();
                    for e in kept.recent.into_iter().rev() {
                        if !merged.iter().any(|m| m.key() == e.key()) {
                            merged.push(e);
                        }
                    }
                    merged.truncate(library_list::MAX_RECENT);
                    l.recent = merged;
                    let mut played = std::mem::take(&mut l.played);
                    for e in kept.played {
                        if !played.iter().any(|m| m.key() == e.key()) {
                            played.push(e);
                        }
                    }
                    played.truncate(library_list::MAX_RECENT);
                    l.played_shown.clone_from(&played);
                    l.played = played;
                    l.hidden.extend(kept.hidden);
                    l.pinned = kept.pinned;
                    l.stored = kept.stored;
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

/// What's kept per identity in the library cache: the records and the Library page's lists
/// Shortcuts uses. Read off the UI thread.
#[derive(Default)]
struct Kept {
    recent: Vec<Entry>,
    played: Vec<Entry>,
    hidden: Vec<String>,
    pinned: Vec<Entry>,
    stored: Vec<Entry>,
}

impl Kept {
    fn load(cache: &LibraryCache, key: &str) -> Kept {
        let albums: Vec<AlbumSummary> = cache.load(key, LibraryList::Albums).unwrap_or_default();
        let artists: Vec<ArtistSummary> = cache.load(key, LibraryList::Artists).unwrap_or_default();
        Kept {
            recent: cache.load(key, LibraryList::Recent).unwrap_or_default(),
            played: cache.load(key, LibraryList::Played).unwrap_or_default(),
            hidden: cache.load(key, LibraryList::Hidden).unwrap_or_default(),
            pinned: cache.load(key, LibraryList::Pinned).unwrap_or_default(),
            stored: albums
                .iter()
                .map(album_entry)
                .chain(artists.iter().map(artist_entry))
                .collect(),
        }
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
            home::shortcuts_changed(ctx);
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
        kind: item_kind(e.kind),
        title: shown(&e.title),
        subtitle: shown(&library_list::subtitle(e)),
        cover: Image::default(),
        round: e.kind == Kind::Artist,
    }
}

pub fn item_kind(kind: Kind) -> ItemKind {
    match kind {
        Kind::Playlist => ItemKind::Playlist,
        Kind::Album => ItemKind::Album,
        Kind::Artist => ItemKind::Artist,
    }
}

pub fn playlist_entry(p: &PlaylistSummary) -> Entry {
    Entry {
        kind: Kind::Playlist,
        id: featherplay_ytm::convert::bare_playlist_id(&p.id.0).to_string(),
        title: p.title.clone(),
        subtitle: p.author.clone().unwrap_or_default(),
        count: p.count,
        single: false,
        thumbnail: p.thumbnail.clone(),
    }
}

pub fn album_entry(a: &AlbumSummary) -> Entry {
    Entry {
        kind: Kind::Album,
        id: a.id.0.clone(),
        title: a.title.clone(),
        subtitle: artist_names(&a.artists),
        count: None,
        single: a.single,
        thumbnail: a.thumbnail.clone(),
    }
}

pub fn artist_entry(a: &ArtistSummary) -> Entry {
    Entry {
        kind: Kind::Artist,
        id: a.id.0.clone(),
        title: a.name.clone(),
        subtitle: String::new(),
        count: None,
        single: false,
        thumbnail: a.thumbnail.clone(),
    }
}

/// Saves `value` as `list` for the identity, off the UI thread and in order, so two quick
/// changes save the newer one last. Nothing without an identity (fixtures).
fn save<T: serde::Serialize + Send + 'static>(key: Option<String>, list: LibraryList, value: T) {
    let Some(key) = key else { return };
    featherplay_controller::task::write_in_order(move || {
        let cache = LibraryCache::new(&Paths::resolve());
        if let Err(e) = cache.save(&key, list, &value) {
            log!("sidebar {list:?}: saving failed: {}", e.kind());
        }
    });
}

/// Keeps what an entry seen this session shows, with what was known of it before.
fn see(l: &mut Library, e: &Entry) {
    let key = e.key();
    let e = match l.seen.get(&key) {
        Some(old) => e.clone().filled_from(old),
        None => e.clone(),
    };
    l.seen.insert(key, e);
}

/// A playlist, album or artist was opened (its page, or a play started from it): it goes
/// to the front of the recent record, for the sidebar's order, which follows the next time
/// a list is shown. Opening doesn't change Home's Shortcuts: only plays do (`played`).
pub fn opened(_ctx: &Ctx, e: Entry) {
    let saved = LIBRARY.with_borrow_mut(|l| {
        see(l, &e);
        library_list::note(&mut l.recent, e);
        (l.key.clone(), l.recent.clone())
    });
    save(saved.0, LibraryList::Recent, saved.1);
}

/// Home's feed showed these: what a play from one of them shows in Shortcuts (a mix's line
/// is the feed's).
pub fn seen_on_home(shelves: &[HomeShelf]) {
    LIBRARY.with_borrow_mut(|l| {
        for item in shelves.iter().flat_map(|s| &s.items) {
            let e = match item {
                HomeItem::Album(a) => album_entry(a),
                HomeItem::Playlist(p) => playlist_entry(p),
                HomeItem::Artist(a) => artist_entry(a),
                HomeItem::Track(_) => continue,
            };
            see(l, &e);
        }
    });
}

/// What the library knows of `key`: the pins' records, the lists, then what was seen,
/// played or opened.
fn known(l: &Library, key: &str) -> Option<Entry> {
    let lists = [
        &l.pinned,
        l.playlists.as_deref().unwrap_or_default(),
        l.albums.as_deref().unwrap_or_default(),
        l.artists.as_deref().unwrap_or_default(),
        &l.stored,
        &l.played,
        &l.recent,
    ];
    let listed = lists
        .iter()
        .find_map(|list| list.iter().find(|e| e.key() == key));
    match (l.seen.get(key), listed) {
        (Some(seen), Some(listed)) => Some(seen.clone().filled_from(listed)),
        (seen, listed) => seen.or(listed).cloned(),
    }
}

/// A song played long enough from `place` (an album, a playlist or an artist; `label` is
/// its name as Up next shows it): it goes first in the played record that Shortcuts
/// suggests from (back in Shortcuts if it was removed), and in the sidebar's recent record.
/// Home shows it from its next visit, so the row never changes under the pointer. With
/// "Suggest from what you play" off, nothing is recorded.
pub fn played(place: &Place, label: &str, track: &Track) {
    let (kind, id) = match place {
        Place::Album(id) => (Kind::Album, id.0.clone()),
        Place::Playlist(id) => (
            Kind::Playlist,
            featherplay_ytm::convert::bare_playlist_id(&id.0).to_string(),
        ),
        Place::Artist(id) => (Kind::Artist, id.0.clone()),
        Place::History => return,
    };
    if !settings::shortcut_suggestions() || (kind == Kind::Playlist && id == "LM") {
        return;
    }
    let key = library_list::key(kind, &id);
    let saved = LIBRARY.with_borrow_mut(|l| {
        // What the song itself says, when nothing knows the place better.
        let from_song = Entry {
            kind,
            id,
            title: label.to_string(),
            subtitle: if kind == Kind::Album {
                artist_names(&track.artists)
            } else {
                String::new()
            },
            count: None,
            single: false,
            thumbnail: (kind == Kind::Album)
                .then(|| track.thumbnail.clone())
                .flatten(),
        };
        let e = match known(l, &key) {
            Some(known) => known.filled_from(&from_song),
            None => from_song,
        };
        library_list::note(&mut l.recent, e.clone());
        library_list::played(&mut l.played, &mut l.hidden, e);
        (
            l.key.clone(),
            l.recent.clone(),
            l.played.clone(),
            l.hidden.clone(),
        )
    });
    let (identity, recent, played, hidden) = saved;
    save(identity.clone(), LibraryList::Recent, recent);
    save(identity.clone(), LibraryList::Played, played);
    save(identity, LibraryList::Hidden, hidden);
}

/// Home's Shortcuts after Liked songs (`library_list::shortcuts`). `visit`: Home is being
/// shown, so what was played since its last visit joins; otherwise the row keeps the plays
/// it had and only the pins, removals and the library change.
pub fn shortcuts(visit: bool) -> Vec<Shortcut> {
    LIBRARY.with_borrow_mut(|l| {
        if visit {
            l.played_shown.clone_from(&l.played);
        }
        let library: Vec<Entry> = l
            .playlists
            .iter()
            .chain(l.albums.iter())
            .chain(l.artists.iter())
            .flatten()
            .chain(&l.stored)
            .cloned()
            .collect();
        let seen: Vec<Entry> = l.seen.values().cloned().collect();
        library_list::shortcuts(&Sources {
            pins: &l.pins,
            known: &[
                l.pinned.as_slice(),
                library.as_slice(),
                seen.as_slice(),
                l.played.as_slice(),
                l.recent.as_slice(),
            ],
            played: &l.played_shown,
            library: &library,
            hidden: &l.hidden,
            suggest: settings::shortcut_suggestions(),
        })
    })
}

/// Whether the entry is pinned.
pub fn is_pinned(kind: Kind, id: &str) -> bool {
    let key = library_list::key(kind, id);
    LIBRARY.with_borrow(|l| l.pins.contains(&key))
}

/// Pins the entry (first among the pins: the top of its sidebar list, first after Liked
/// songs in Shortcuts), or unpins it, and keeps that in the settings for this identity,
/// with what it names in the library cache. True when it's pinned now.
pub fn toggle_pin(ctx: &Ctx, kind: Kind, id: &str) -> bool {
    let key = library_list::key(kind, id);
    let (pinned, identity, pins, entries, hidden) = LIBRARY.with_borrow_mut(|l| {
        let pinned = library_list::toggle_pin(&mut l.pins, &key);
        if pinned {
            // A pin is never a removed suggestion.
            l.hidden.retain(|h| *h != key);
            if let Some(e) = known(l, &key) {
                l.pinned.retain(|p| p.key() != key);
                l.pinned.push(e);
            }
        } else {
            l.pinned.retain(|p| p.key() != key);
        }
        (
            pinned,
            l.key.clone(),
            l.pins.clone(),
            l.pinned.clone(),
            l.hidden.clone(),
        )
    });
    if let Some(identity) = &identity {
        settings::set_pins(ctx, identity, pins);
    }
    save(identity.clone(), LibraryList::Pinned, entries);
    save(identity, LibraryList::Hidden, hidden);
    show(ctx);
    home::shortcuts_changed(ctx);
    pinned
}

/// Moves a pin one place earlier or `later` (Shortcuts' Move left and Move right), in the
/// sidebar's order too.
pub fn move_pin(ctx: &Ctx, kind: Kind, id: &str, later: bool) {
    let key = library_list::key(kind, id);
    let saved = LIBRARY.with_borrow_mut(|l| {
        library_list::move_pin(&mut l.pins, &key, later)
            .then(|| l.key.clone().map(|k| (k, l.pins.clone())))
    });
    let Some(saved) = saved else { return };
    if let Some((identity, pins)) = saved {
        settings::set_pins(ctx, &identity, pins);
    }
    show(ctx);
    home::shortcuts_changed(ctx);
}

/// Where a pin is among the pins, and how many there are.
pub fn pin_place(kind: Kind, id: &str) -> Option<(usize, usize)> {
    let key = library_list::key(kind, id);
    LIBRARY.with_borrow(|l| {
        l.pins
            .iter()
            .position(|p| *p == key)
            .map(|at| (at, l.pins.len()))
    })
}

/// Removes a suggestion from Shortcuts (`hidden: true`) until it's played again, or puts
/// it back (Undo).
pub fn hide(ctx: &Ctx, kind: Kind, id: &str, hidden: bool) {
    let key = library_list::key(kind, id);
    let saved = LIBRARY.with_borrow_mut(|l| {
        if hidden {
            library_list::hide(&mut l.hidden, &key);
        } else {
            l.hidden.retain(|h| *h != key);
        }
        (l.key.clone(), l.hidden.clone())
    });
    save(saved.0, LibraryList::Hidden, saved.1);
    home::shortcuts_changed(ctx);
}

/// "Forget suggestions": the played record and the removed list go; the pins stay.
pub fn forget_suggestions(ctx: &Ctx) {
    let identity = LIBRARY.with_borrow_mut(|l| {
        l.played.clear();
        l.played_shown.clear();
        l.hidden.clear();
        l.key.clone()
    });
    save(identity.clone(), LibraryList::Played, Vec::<Entry>::new());
    save(identity, LibraryList::Hidden, Vec::<String>::new());
    home::shortcuts_changed(ctx);
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

/// `--snapshot home-shortcuts*` (in memory), once the chips' lists have loaded: the first
/// playlist, album and artist pinned, and some of the others played from. `fresh`: a new
/// account's, two playlists and nothing pinned or played.
pub fn seed_for_snapshot(ctx: &Ctx, fresh: bool) {
    LIBRARY.with_borrow_mut(|l| {
        if fresh {
            if let Some(p) = l.playlists.as_mut() {
                p.truncate(2);
            }
            l.albums = Some(Vec::new());
            l.artists = Some(Vec::new());
            l.stored.clear();
            l.pins.clear();
            l.pinned.clear();
            l.played.clear();
            l.played_shown.clear();
            return;
        }
        let list = |v: &Option<Vec<Entry>>| v.clone().unwrap_or_default();
        let (p, a, r) = (list(&l.playlists), list(&l.albums), list(&l.artists));
        let pins: Vec<Entry> = [p.first(), a.first(), r.first()]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let played: Vec<Entry> = [
            p.get(1),
            a.get(1),
            p.get(2),
            r.get(1),
            a.get(2),
            p.get(3),
            r.get(2),
            a.get(3),
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
        for e in played.iter().chain(&pins) {
            see(l, e);
        }
        l.pins = pins.iter().map(Entry::key).collect();
        l.pinned = pins;
        l.played_shown.clone_from(&played);
        l.played = played;
    });
    show(ctx);
}
