//! Home's feed: its first pages and their disk cache, whose feed it is, and the later pages
//! as the user scrolls (`Paging`).

use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use featherplay_controller::catalog::Source;
use featherplay_store::{LibraryCache, LibraryList, Paths};
use featherplay_ytm::{HomeFeed, HomeItem, HomeShelf};
use serde::{Deserialize, Serialize};
use slint::ComponentHandle;

use super::hero::show_hero;
use super::shelves::with_liked;
use super::{STATE, State, add_shelves, show_shelves};
use crate::app::Ctx;
use crate::busy::Busy;
use crate::pages::content;
use crate::router::Page;
use crate::sidebar;
use crate::ui::{AppWindow, HomeData, MoreState};

/// Pages of the feed per Home load, the first ones included.
pub const MAX_PAGES: usize = 6;
/// Pages `YtMusic::home` fetches: the first and the continuation fetched with it.
const FIRST_PAGES: usize = 2;

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

/// Home's feed as kept on disk: next to the Library's lists, per identity
/// (`LibraryList::Home`), deleted with them on sign-out and a new sign-in, and on its own
/// when the session ends. Only the first pages' shelves, without the next page's token.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct CachedFeed {
    /// When it was fetched, in seconds since the Unix epoch.
    saved: u64,
    shelves: Vec<HomeShelf>,
}

/// A cached feed older than this isn't shown: Home loads as on a first start.
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

impl CachedFeed {
    /// `shelves` as fetched at `now`, without any URL that carries an IP address.
    fn new(shelves: &[HomeShelf], now: SystemTime) -> CachedFeed {
        let strip = |url: &mut Option<String>| {
            if url.as_deref().is_some_and(carries_ip) {
                *url = None;
            }
        };
        let mut shelves = shelves.to_vec();
        for item in shelves.iter_mut().flat_map(|s| s.items.iter_mut()) {
            match item {
                HomeItem::Track(t) => strip(&mut t.thumbnail),
                HomeItem::Album(a) => strip(&mut a.thumbnail),
                HomeItem::Artist(a) => strip(&mut a.thumbnail),
                HomeItem::Playlist(p) => {
                    strip(&mut p.thumbnail);
                    strip(&mut p.avatar);
                }
            }
        }
        CachedFeed {
            saved: unix_secs(now),
            shelves,
        }
    }

    /// The shelves to show at `now`: none once the feed is older than `CACHE_TTL` (or
    /// saved later than now: the clock moved).
    fn usable(self, now: SystemTime) -> Option<Vec<HomeShelf>> {
        let age = unix_secs(now).checked_sub(self.saved)?;
        (age < CACHE_TTL.as_secs() && !self.shelves.is_empty()).then_some(self.shelves)
    }
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A URL whose host is an IP address, or whose query names one (`ip=`, as stream URLs do).
/// One that doesn't parse counts as carrying one.
fn carries_ip(url: &str) -> bool {
    let Some(url) = reqwest::Url::parse(url)
        .ok()
        .filter(|u| u.host_str().is_some())
    else {
        return true;
    };
    let host = url.host_str().unwrap_or_default();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.parse::<std::net::IpAddr>().is_ok() || url.query_pairs().any(|(k, _)| k == "ip")
}

/// Whose feed it is: the sign-in (`YtMusic::session_number`: two sessions' own identities
/// can be two Google accounts) and the identity's key, which the disk cache is kept under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Owner {
    session: u64,
    key: String,
}

/// A live session's only, so a feed is never read or kept for one signed out (or while the
/// stored session is still being restored).
pub(super) fn owner_of(source: &Source) -> Option<Owner> {
    match source {
        Source::Live(api) => Some(Owner {
            session: api.session_number(),
            key: api.identity().key(),
        }),
        _ => None,
    }
}

/// Whether the shelves on screen, the feed of `owner` (`None`: a visitor's), may stay while
/// `current` is the session: never another sign-in's or identity's. While the stored session
/// can't connect (`current` unknown) it's still the one they were read for.
fn keeps(owner: Option<&Owner>, current: Option<&Owner>) -> bool {
    match (owner, current) {
        (Some(owner), Some(current)) => owner == current,
        _ => true,
    }
}

/// With nothing on the page yet, once per account: the feed the live session's identity
/// last fetched, from the disk (off the UI thread), unless it's older than `CACHE_TTL` or
/// the fetch got there first. The fetch then refreshes it behind.
pub(super) fn show_cached(ctx: &Rc<Ctx>) {
    let Some(owner) = owner_of(&ctx.catalog.source()) else {
        return;
    };
    let account = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let read = !s.shelves.is_empty() || std::mem::replace(&mut s.cache_read, true);
        (!read).then_some(s.account)
    });
    let Some(account) = account else { return };
    let started = Instant::now();
    let busy = Busy::start();
    let key = owner.key.clone();
    let task = ctx.catalog.runtime().spawn_blocking(move || {
        LibraryCache::new(&Paths::resolve()).load::<CachedFeed>(&key, LibraryList::Home)
    });
    let ctx = ctx.clone();
    let spawned = slint::spawn_local(async move {
        let cached = task.await.ok().flatten();
        let shelves = cached.and_then(|c| c.usable(SystemTime::now()));
        let current = STATE.with(|s| {
            let s = s.borrow();
            s.account == account && s.shelves.is_empty()
        });
        let still_ours = owner_of(&ctx.catalog.source()).as_ref() == Some(&owner);
        if let Some(shelves) = shelves
            && current
            && still_ours
            && let Some(ui) = ctx.window()
        {
            log!(
                "home: {} shelves from the cache in {} ms",
                shelves.len(),
                started.elapsed().as_millis()
            );
            // A refresh that failed first said so; with the cached feed, it stays quiet.
            ui.global::<HomeData>().set_error("".into());
            show_shelves(&ctx, &with_liked(&shelves, &sidebar::shortcuts()));
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.shelves = shelves;
                s.owner = Some(owner);
                s.from_cache = true;
                s.hero = None;
            });
            show_hero(&ctx);
            #[cfg(feature = "perf-harness")]
            crate::perf::mark(crate::perf::Mark::Load);
        }
        drop(busy);
    });
    if let Err(e) = spawned {
        log!("home cache: {e}");
    }
}

/// Fetches the feed (one call; the result is kept even if the user has moved on).
pub(super) fn fetch(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<HomeData>();
    data.set_loading(true);
    data.set_error("".into());
    data.set_empty(false);
    data.set_more(MoreState::Idle);
    let asking_as = (!ctx.session_pending())
        .then(|| owner_of(&ctx.catalog.source()))
        .flatten();
    let account = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.fetching = true;
        s.fetch_owner = asking_as;
        // A later page still on its way belongs to the feed being replaced.
        s.paging.start(None, 0);
        s.account
    });
    let asked = Instant::now();
    ctx.load(
        |source| async move { (owner_of(&source), source.home().await) },
        move |ctx, ui, (owner, result)| {
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
                    if let Some(owner) = &owner {
                        save_feed(ctx, owner.clone(), &feed.shelves);
                    }
                    let on_screen = ctx.current_page() == Page::Home;
                    let shown = STATE.with(|s| {
                        let mut s = s.borrow_mut();
                        s.fetched = Some(Instant::now());
                        s.fetch_owner = owner.clone();
                        hold(&mut s, feed, owner, on_screen)
                    });
                    if let Some(feed) = shown {
                        show_feed(ctx, feed);
                    }
                }
                // A cached feed on screen stays, as the Library's lists do (the next open
                // fetches again), unless the session has ended.
                Err(e)
                    if STATE.with(|s| s.borrow().from_cache)
                        && !matches!(e, featherplay_ytm::ApiError::AuthExpired) =>
                {
                    log!("home: refreshing failed, the cached feed stays: {e}");
                }
                Err(e) => {
                    if e == featherplay_ytm::ApiError::AuthExpired {
                        session_ended(ctx, ui);
                    }
                    data.set_error(content::error_text("home", &e).into());
                }
            }
            show_hero(ctx);
        },
    );
}

/// The feed was refused, the session has ended: its shelves go, on screen and on disk.
fn session_ended(ctx: &Ctx, ui: &AppWindow) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.shelves.clear();
        s.hero = None;
        s.owner = None;
        s.from_cache = false;
    });
    ui.global::<HomeData>().set_shelves(Default::default());
    featherplay_controller::task::blocking_detached(ctx.catalog.runtime(), || {
        if let Err(e) = LibraryCache::new(&Paths::resolve()).forget(LibraryList::Home) {
            log!("home cache: removing failed: {}", e.kind());
        }
    });
}

/// A fresh feed in place of the one shown.
pub(super) fn show_feed(ctx: &Ctx, feed: HomeFeed) {
    show_shelves(ctx, &with_liked(&feed.shelves, &sidebar::shortcuts()));
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.shelves = feed.shelves;
        s.paging.start(feed.more, FIRST_PAGES);
        s.from_cache = false;
        s.hero = None;
    });
}

/// A fresh feed of `owner`'s: the one to show now, if any (`show_feed`). While the cached
/// feed of the same owner is on screen (`on_screen`: Home is the page) it stays, and takes
/// the fresh feed's next-page token, so scrolling goes on under it (the cache keeps none).
/// With the same shelves the fresh feed is that one; otherwise it waits for Home's next open
/// (`pending`), as swapped now the rows the user is looking at would change under them.
fn hold(s: &mut State, feed: HomeFeed, owner: Option<Owner>, on_screen: bool) -> Option<HomeFeed> {
    if !(s.from_cache && on_screen && s.owner == owner) {
        s.owner = owner;
        return Some(feed);
    }
    s.paging.start(feed.more.clone(), FIRST_PAGES);
    if s.shelves == feed.shelves {
        s.from_cache = false;
    } else {
        log!("home: the fresh feed shows the next time Home opens");
        s.pending = Some(feed);
    }
    None
}

/// A later page's shelves but those whose title already shows: under a cached feed the
/// pages are the fresh feed's, which can repeat a shelf of the cached first pages.
fn unseen(shown: &[HomeShelf], page: Vec<HomeShelf>) -> Vec<HomeShelf> {
    page.into_iter()
        .filter(|p| !shown.iter().any(|s| s.title == p.title))
        .collect()
}

/// Replaces the identity's cached feed, off the UI thread.
fn save_feed(ctx: &Ctx, owner: Owner, shelves: &[HomeShelf]) {
    let cached = CachedFeed::new(shelves, SystemTime::now());
    let catalog = ctx.catalog.clone();
    featherplay_controller::task::blocking_detached(ctx.catalog.runtime(), move || {
        // Signed out, ended or another sign-in meanwhile: its cache was just deleted, and
        // this feed isn't the new one's.
        if owner_of(&catalog.source()).as_ref() != Some(&owner) {
            return;
        }
        let cache = LibraryCache::new(&Paths::resolve());
        if let Err(e) = cache.save(&owner.key, LibraryList::Home, &cached) {
            log!("home cache: saving failed: {}", e.kind());
        }
    });
}

/// The user scrolled near the end of Home (or pressed Try again on a failed page): fetches
/// the next page, if `Paging` lets one go now, and adds its shelves under the others.
pub(super) fn more(ctx: &Rc<Ctx>, retry: bool) {
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
                    let shelves = STATE.with(|s| unseen(&s.borrow().shelves, feed.shelves));
                    add_shelves(ctx, &shelves);
                    STATE.with(|s| s.borrow_mut().shelves.extend(shelves));
                }
                Err(e) => log!("home: the next page failed: {e}"),
            }
            show_more(ctx);
        },
    );
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

/// `account_changed` on the state: `current` is the session now (`owner_of`). Returns
/// whether the shelves on screen go.
pub(super) fn account_news(s: &mut State, signed_in: bool, current: Option<Owner>) -> bool {
    // The start's own news, with the feed fetched (or being fetched) already as this
    // session: it stands, and so does a fresh feed held behind the cached one.
    let same = signed_in
        && current.is_some()
        && s.fetch_owner == current
        && (s.fetching || s.fetched.is_some());
    if !same {
        s.fetched = None;
        s.fetching = false;
        s.fetch_owner = None;
        s.account += 1;
        s.cache_read = false;
        s.pending = None;
        s.paging.start(None, 0);
    }
    // Never another sign-in's or identity's feed, nor any signed out.
    let clear = !signed_in || !keeps(s.owner.as_ref(), current.as_ref());
    if clear {
        s.shelves.clear();
        s.hero = None;
        s.owner = None;
        s.from_cache = false;
        s.pending = None;
    }
    clear
}

#[cfg(test)]
mod tests {
    use super::super::test_items::{artist, playlist, shelf, song, track};
    use super::*;

    #[test]
    fn a_cached_feed_older_than_a_day_isnt_shown() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let hours = |h: u64| Duration::from_secs(h * 3600);
        let feed = || CachedFeed::new(&[shelf("Quick picks", vec![song("a")])], t0);
        assert!(feed().usable(t0 + hours(23)).is_some());
        assert_eq!(feed().usable(t0 + hours(25)), None);
        // Saved "later than now": the clock went back; it's not trusted either.
        assert_eq!(feed().usable(t0 - hours(1)), None);
        assert_eq!(CachedFeed::new(&[], t0).usable(t0), None, "nothing to show");
    }

    #[test]
    fn the_cached_feed_keeps_no_url_carrying_an_ip() {
        let mut t = track("a");
        t.thumbnail = Some("https://i.ytimg.com/vi/a/hq.jpg?ip=192.0.2.1&sig=x".into());
        let HomeItem::Playlist(mut p) = playlist("PLa") else {
            unreachable!()
        };
        p.thumbnail = Some("https://lh3.googleusercontent.com/abc=w544-h544".into());
        p.avatar = Some("https://192.0.2.1/avatar.jpg".into());
        let HomeItem::Artist(mut a) = artist() else {
            unreachable!()
        };
        a.thumbnail = Some("https://[2001:db8::1]/a.jpg".into());
        let shelves = [shelf(
            "Mixed",
            vec![
                HomeItem::Track(t),
                HomeItem::Playlist(p),
                HomeItem::Artist(a),
            ],
        )];
        let kept = CachedFeed::new(&shelves, SystemTime::now()).shelves;
        let urls: Vec<Option<&str>> = kept[0]
            .items
            .iter()
            .flat_map(|i| match i {
                HomeItem::Track(t) => vec![t.thumbnail.as_deref()],
                HomeItem::Playlist(p) => vec![p.thumbnail.as_deref(), p.avatar.as_deref()],
                HomeItem::Artist(a) => vec![a.thumbnail.as_deref()],
                HomeItem::Album(a) => vec![a.thumbnail.as_deref()],
            })
            .collect();
        assert_eq!(
            urls,
            [
                None,
                Some("https://lh3.googleusercontent.com/abc=w544-h544"),
                None,
                None
            ]
        );
    }

    fn signed_in(sapisid: &str, identity: featherplay_ytm::Identity) -> Owner {
        let api = featherplay_ytm::YtMusic::resume(
            &format!("SAPISID={sapisid};"),
            &featherplay_ytm::Identity::default(),
            "1.20261101.01.00",
        )
        .unwrap()
        .as_identity(&identity)
        .unwrap();
        owner_of(&Source::Live(api)).unwrap()
    }

    fn own(sapisid: &str) -> Owner {
        signed_in(sapisid, featherplay_ytm::Identity::default())
    }

    fn brand(sapisid: &str) -> Owner {
        signed_in(sapisid, featherplay_ytm::Identity::new(2, None).unwrap())
    }

    /// The start: the cached feed of `owner` on screen, its refresh asked as `owner`.
    fn cached_on_screen(owner: &Owner) -> State {
        State {
            shelves: vec![shelf("Quick picks", vec![song("cached")])],
            owner: Some(owner.clone()),
            from_cache: true,
            cache_read: true,
            fetching: true,
            fetch_owner: Some(owner.clone()),
            ..State::default()
        }
    }

    fn fresh_feed() -> HomeFeed {
        HomeFeed {
            shelves: vec![shelf("Quick picks", vec![song("fresh")])],
            more: Some("next".into()),
        }
    }

    #[test]
    fn another_identitys_feed_never_stays_on_screen() {
        // An account switch: the brand account's feed goes, and the reverse.
        assert!(!keeps(Some(&brand("a")), Some(&own("a"))));
        assert!(!keeps(Some(&own("a")), Some(&brand("a"))));
        assert!(keeps(Some(&brand("a")), Some(&brand("a"))));
        // Another sign-in's own identity can be another Google account.
        assert!(!keeps(Some(&own("a")), Some(&own("b"))));
        // A visitor's feed is nobody's; while the stored session can't connect, the feed
        // read for it stays.
        assert!(keeps(None, Some(&own("a"))));
        assert!(keeps(Some(&own("a")), None));
        assert_eq!(own("a").key, "");
        assert_eq!(brand("a").key, "2");
    }

    #[test]
    fn an_account_switch_drops_the_cached_feed_and_the_fresh_one_held_behind_it() {
        let mut s = cached_on_screen(&own("a"));
        assert_eq!(hold(&mut s, fresh_feed(), Some(own("a")), true), None);
        assert!(s.pending.is_some(), "held for Home's next open");
        s.fetching = false;
        // The account menu switches to the brand account.
        assert!(account_news(&mut s, true, Some(brand("a"))));
        assert!(s.shelves.is_empty() && s.pending.is_none() && s.owner.is_none());
        assert!(
            !s.from_cache && !s.cache_read,
            "the brand account's cache is read next"
        );
        // A new sign-in, another Google account: the same.
        let mut s = cached_on_screen(&own("a"));
        assert!(account_news(&mut s, true, Some(own("b"))));
        assert!(s.shelves.is_empty());
    }

    #[test]
    fn the_starts_own_news_keeps_the_cached_feed_and_its_refresh() {
        let mut s = cached_on_screen(&own("a"));
        let account = s.account;
        assert!(!account_news(&mut s, true, Some(own("a"))));
        assert!(s.fetching && s.account == account, "the refresh goes on");
        assert_eq!(s.shelves.len(), 1);
        // Its fresh feed came first and is held: it stays held.
        let mut s = cached_on_screen(&own("a"));
        hold(&mut s, fresh_feed(), Some(own("a")), true);
        s.fetching = false;
        s.fetched = Some(Instant::now());
        assert!(!account_news(&mut s, true, Some(own("a"))));
        assert!(s.pending.is_some());
    }

    #[test]
    fn signed_out_the_feed_goes_and_none_is_read() {
        let mut s = cached_on_screen(&own("a"));
        hold(&mut s, fresh_feed(), Some(own("a")), true);
        assert!(account_news(&mut s, false, None));
        assert!(s.shelves.is_empty() && s.pending.is_none() && s.owner.is_none());
        let guest = featherplay_ytm::YtMusic::guest().unwrap();
        assert_eq!(owner_of(&Source::SignedOut(guest)), None);
        assert_eq!(owner_of(&Source::Unreachable), None);
    }

    #[test]
    fn a_fresh_feed_waits_while_the_cached_one_is_on_screen() {
        // Home is the page: the fresh feed waits for its next open.
        let mut s = cached_on_screen(&own("a"));
        assert_eq!(hold(&mut s, fresh_feed(), Some(own("a")), true), None);
        assert_eq!(s.shelves[0].items, vec![song("cached")]);
        // Scrolling goes on under the cached shelves, with the fresh feed's later pages
        // (the cache keeps no token), but none that shows already.
        assert_eq!(
            s.paging.want().map(|(_, t)| t).as_deref(),
            Some("next"),
            "the cached feed pages on"
        );
        let page = vec![
            shelf("Quick picks", vec![song("again")]),
            shelf("Albums for you", vec![song("b")]),
        ];
        let added: Vec<String> = unseen(&s.shelves, page)
            .into_iter()
            .map(|s| s.title)
            .collect();
        assert_eq!(added, ["Albums for you"]);
        // The same shelves: nothing to swap, the next page may load.
        let mut s = cached_on_screen(&own("a"));
        let same = HomeFeed {
            shelves: s.shelves.clone(),
            more: Some("next".into()),
        };
        assert_eq!(hold(&mut s, same, Some(own("a")), true), None);
        assert!(!s.from_cache && s.pending.is_none());
        assert!(s.paging.want().is_some());
        // Another page is shown: the fresh feed goes in at once.
        let mut s = cached_on_screen(&own("a"));
        assert_eq!(
            hold(&mut s, fresh_feed(), Some(own("a")), false),
            Some(fresh_feed())
        );
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
}
