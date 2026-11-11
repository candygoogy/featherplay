//! Pages read ahead. When the pointer rests on a card or row that opens an album, playlist
//! or artist (`PageHover`, ui/parts.slint), that page's first request starts, so a click a
//! moment later shows the page from its answer, or takes over the request still in flight:
//! the page never asks twice (`album`, `artist`, `playlist`, then `or_fetch`).
//!
//! - At most `IN_FLIGHT` at once: a newer one drops the oldest still in flight.
//! - An answer is kept `KEEP`, only the last `KEPT`, and is taken once.
//! - A playlist is read as far as its first page; its page reads the rest after the click.
//! - Nothing while the window shows nothing, while the stored session is still being
//!   restored, nor for Liked songs (the account's, which has its own rules). A change of
//!   account drops everything kept.
//! - What's still in flight when another page opens is dropped: its card is gone.
//!
//! The pages shown are kept too (`showing`): Back or Forward to one within `RECENT` shows it
//! as it was left, with no request, and with the likes made since. A playlist is kept once
//! it was read to its end, and dropped when it's changed here (`forget_playlist`).
//!
//! Nothing runs here but on a rest of the pointer: no timer, nothing while paused or idle.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay_controller::catalog::Source;
use featherplay_core::model::{AlbumId, ArtistId, PlaylistId, Track};
use featherplay_ytm::convert::bare_playlist_id;
use featherplay_ytm::{Album, ApiError, Artist, Playlist, PlaylistRest};
use slint::ComponentHandle;
use tokio::task::JoinHandle;

use crate::app::Ctx;
use crate::memory;
use crate::pages::playlist;
use crate::router::Page;
use crate::song_menu;
use crate::ui::{ItemKind, PageHover};

/// How long an answer read ahead is used.
const KEEP: Duration = Duration::from_secs(60);
/// How many answers are kept, in flight or in.
const KEPT: usize = 4;
/// How many requests are in flight at once.
const IN_FLIGHT: usize = 2;
/// How long a page left behind shows again as it was, on Back or Forward.
const RECENT: Duration = Duration::from_secs(5 * 60);
/// How many pages left behind are kept.
const RECENT_KEPT: usize = 6;

/// A request read ahead: in flight, or its answer in.
pub type Ahead = JoinHandle<Answer>;

/// A playlist's first page, and where the list goes on (`Source::playlist_first`).
pub type FirstPage = (Playlist, Option<PlaylistRest>);

/// What a page's first request answered. One type for the three, so they're one kind of
/// task on the runtime.
pub enum Answer {
    Album(Result<Album, ApiError>),
    Artist(Result<Artist, ApiError>),
    Playlist(Result<FirstPage, ApiError>),
}

impl Answer {
    pub fn album(self) -> Option<Result<Album, ApiError>> {
        match self {
            Answer::Album(a) => Some(a),
            _ => None,
        }
    }

    pub fn artist(self) -> Option<Result<Artist, ApiError>> {
        match self {
            Answer::Artist(a) => Some(a),
            _ => None,
        }
    }

    pub fn playlist(self) -> Option<Result<FirstPage, ApiError>> {
        match self {
            Answer::Playlist(a) => Some(a),
            _ => None,
        }
    }

    /// A copy to show again, with the likes made since: a good answer, a playlist only as
    /// a whole list.
    fn again(&self) -> Option<Answer> {
        let mut again = match self {
            Answer::Album(Ok(a)) => Answer::Album(Ok(a.clone())),
            Answer::Artist(Ok(a)) => Answer::Artist(Ok(a.clone())),
            Answer::Playlist(Ok((p, None))) => Answer::Playlist(Ok((p.clone(), None))),
            _ => return None,
        };
        let tracks = match &mut again {
            Answer::Album(Ok(a)) => &mut a.tracks,
            Answer::Artist(Ok(a)) => &mut a.top_songs,
            Answer::Playlist(Ok((p, _))) => &mut p.tracks,
            _ => return None,
        };
        with_likes(tracks);
        Some(again)
    }
}

/// The ratings made since `tracks` were read, as the song menu remembers them.
fn with_likes(tracks: &mut [Track]) {
    for t in tracks {
        if let Some(like) = song_menu::like_of(&t.id) {
            t.like = like;
        }
    }
}

/// The page a card opens.
#[derive(Clone, Debug, PartialEq)]
enum Key {
    Album(AlbumId),
    Artist(ArtistId),
    Playlist(PlaylistId),
}

impl Key {
    /// Songs open no page, and Liked songs isn't read ahead.
    fn of(kind: ItemKind, id: &str) -> Option<Key> {
        if id.is_empty() {
            return None;
        }
        match kind {
            ItemKind::Album => Some(Key::Album(AlbumId(id.into()))),
            ItemKind::Artist => Some(Key::Artist(ArtistId(id.into()))),
            ItemKind::Playlist => {
                let id = PlaylistId(id.into());
                (!playlist::is_liked_songs(&id)).then_some(Key::Playlist(id))
            }
            ItemKind::Song => None,
        }
    }

    fn page(&self) -> Page {
        match self {
            Key::Album(id) => Page::Album { id: id.clone() },
            Key::Artist(id) => Page::Artist { id: id.clone() },
            Key::Playlist(id) => Page::Playlist { id: id.clone() },
        }
    }

    /// The key of `page`, if it's one kept.
    fn of_page(page: &Page) -> Option<Key> {
        match page {
            Page::Album { id } => Some(Key::Album(id.clone())),
            Page::Artist { id } => Some(Key::Artist(id.clone())),
            Page::Playlist { id } if !playlist::is_liked_songs(id) => {
                Some(Key::Playlist(id.clone()))
            }
            _ => None,
        }
    }

    /// For the log.
    fn what(&self) -> &'static str {
        match self {
            Key::Album(_) => "album",
            Key::Artist(_) => "artist",
            Key::Playlist(_) => "playlist",
        }
    }
}

/// The page's first request.
async fn read(source: Source, key: Key) -> Answer {
    match key {
        Key::Album(id) => Answer::Album(source.album(&id).await),
        Key::Artist(id) => Answer::Artist(source.artist(&id).await),
        Key::Playlist(id) => Answer::Playlist(source.playlist_first(&id).await),
    }
}

/// What `Kept` needs of what it keeps.
trait Request {
    fn in_flight(&self) -> bool;
    fn abort(&self);
}

impl Request for Ahead {
    fn in_flight(&self) -> bool {
        !self.is_finished()
    }

    fn abort(&self) {
        JoinHandle::abort(self);
    }
}

struct Entry<R> {
    key: Key,
    started: Instant,
    request: R,
}

/// The requests read ahead, oldest first.
struct Kept<R> {
    entries: Vec<Entry<R>>,
}

impl<R: Request> Kept<R> {
    const fn new() -> Self {
        Kept {
            entries: Vec::new(),
        }
    }

    /// Drops the entries older than `KEEP` at `now`.
    fn expire(&mut self, now: Instant) {
        self.entries.retain(|e| {
            let fresh = now.saturating_duration_since(e.started) < KEEP;
            if !fresh {
                e.request.abort();
            }
            fresh
        });
    }

    fn has(&mut self, key: &Key, now: Instant) -> bool {
        self.expire(now);
        self.entries.iter().any(|e| e.key == *key)
    }

    /// Hands over `key`'s request, if one is kept and not expired.
    fn take(&mut self, key: &Key, now: Instant) -> Option<R> {
        self.expire(now);
        let i = self.entries.iter().position(|e| e.key == *key)?;
        Some(self.entries.remove(i).request)
    }

    /// Keeps `request` for `key`, started at `now`. The oldest request still in flight
    /// gives way when `IN_FLIGHT` are, and the oldest entry when `KEPT` are kept.
    fn insert(&mut self, key: Key, request: R, now: Instant) {
        self.expire(now);
        let flying = self
            .entries
            .iter()
            .filter(|e| e.request.in_flight())
            .count();
        if flying >= IN_FLIGHT
            && let Some(i) = self.entries.iter().position(|e| e.request.in_flight())
        {
            self.entries.remove(i).request.abort();
        }
        if self.entries.len() >= KEPT {
            self.entries.remove(0).request.abort();
        }
        self.entries.push(Entry {
            key,
            started: now,
            request,
        });
    }

    /// Drops the requests still in flight for any page but `page`, and what's expired at
    /// `now`.
    fn keep_only(&mut self, page: &Page, now: Instant) {
        self.expire(now);
        self.entries.retain(|e| {
            let gone = e.request.in_flight() && e.key.page() != *page;
            if gone {
                e.request.abort();
            }
            !gone
        });
    }

    fn clear(&mut self) {
        for e in self.entries.drain(..) {
            e.request.abort();
        }
    }
}

/// The pages shown, newest last: what each showed, and when.
#[derive(Default)]
struct Shown {
    pages: Vec<(Key, Instant, Answer)>,
}

impl Shown {
    fn expire(&mut self, now: Instant) {
        self.pages
            .retain(|(_, at, _)| now.saturating_duration_since(*at) < RECENT);
    }

    /// Keeps `answer` for `key`'s page, shown at `now`. A page shown again keeps the time
    /// it was first kept, so going back and forth doesn't keep it for good.
    fn insert(&mut self, key: Key, answer: Answer, now: Instant) {
        self.expire(now);
        let mut since = now;
        self.pages.retain(|(k, at, _)| {
            if *k == key {
                since = *at;
            }
            *k != key
        });
        if self.pages.len() >= RECENT_KEPT {
            self.pages.remove(0);
        }
        self.pages.push((key, since, answer));
    }

    /// What `key`'s page showed, to show again.
    fn again(&mut self, key: &Key, now: Instant) -> Option<Answer> {
        self.expire(now);
        self.pages
            .iter()
            .find(|(k, ..)| k == key)
            .and_then(|(.., answer)| answer.again())
    }
}

thread_local! {
    static KEPT_AHEAD: RefCell<Kept<Ahead>> = const { RefCell::new(Kept::new()) };
    static SHOWN: RefCell<Shown> = RefCell::default();
    static CHANGES: Cell<u64> = const { Cell::new(0) };
}

/// Once at startup: connect `PageHover`.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let c = ctx.clone();
    ui.global::<PageHover>()
        .on_rested(move |kind, id| rested(&c, kind, &id));
}

/// The pointer rested on a card or row that opens `kind` `id`'s page: its first request
/// starts, unless it's kept already or that page is the one shown.
fn rested(ctx: &Rc<Ctx>, kind: ItemKind, id: &str) {
    let Some(key) = Key::of(kind, id) else { return };
    if memory::is_hidden() || ctx.session_pending() || ctx.current_page() == key.page() {
        return;
    }
    let now = Instant::now();
    if KEPT_AHEAD.with(|k| k.borrow_mut().has(&key, now)) {
        return;
    }
    log!("read ahead: {}", key.what());
    let request = ctx
        .catalog
        .runtime()
        .spawn(read(ctx.catalog.source(), key.clone()));
    KEPT_AHEAD.with(|k| k.borrow_mut().insert(key, request, now));
}

/// The request read ahead for `key`, or on Back or Forward what its page showed when it
/// was left (as an answer already in).
fn take(ctx: &Ctx, key: Key) -> Option<Ahead> {
    let now = Instant::now();
    // Back or Forward: what the page showed, rather than a read ahead still on its way.
    if ctx.revisiting()
        && let Some(answer) = SHOWN.with(|s| s.borrow_mut().again(&key, now))
    {
        log!("{}: as it was left", key.what());
        return Some(ctx.catalog.runtime().spawn(async move { answer }));
    }
    KEPT_AHEAD.with(|k| k.borrow_mut().take(&key, now))
}

/// The album read ahead, if it is: for `or_fetch` with `Answer::album`.
pub fn album(ctx: &Ctx, id: &AlbumId) -> Option<Ahead> {
    take(ctx, Key::Album(id.clone()))
}

/// The artist read ahead, if it is: for `or_fetch` with `Answer::artist`.
pub fn artist(ctx: &Ctx, id: &ArtistId) -> Option<Ahead> {
    take(ctx, Key::Artist(id.clone()))
}

/// The playlist's first page read ahead, if it is: for `or_fetch` with `Answer::playlist`.
pub fn playlist(ctx: &Ctx, id: &PlaylistId) -> Option<Ahead> {
    take(ctx, Key::Playlist(id.clone()))
}

/// How many times a playlist was changed here (`forget_playlist`), for `showing`.
pub fn changes() -> u64 {
    CHANGES.with(Cell::get)
}

/// `page` shows `answer`, a good one (a playlist whole, without a rest), asked for when
/// `changes` said `asked`: Back or Forward to it within `RECENT` shows it again as it is
/// now, unless a playlist was changed here since it was asked for.
pub fn showing(page: &Page, answer: Answer, asked: u64) {
    if asked != changes() {
        return;
    }
    if let Some(key) = Key::of_page(page) {
        SHOWN.with(|s| s.borrow_mut().insert(key, answer, Instant::now()));
    }
}

/// Playlist `id` changed here (songs added, renamed, deleted): it's read again next time.
pub fn forget_playlist(id: &PlaylistId) {
    CHANGES.with(|c| c.set(c.get() + 1));
    let bare = |id: &PlaylistId| bare_playlist_id(&id.0).to_string();
    let gone = bare(id);
    SHOWN.with(|s| {
        s.borrow_mut()
            .pages
            .retain(|(k, ..)| !matches!(k, Key::Playlist(p) if bare(p) == gone));
    });
}

/// The answer read `ahead` (as `pick` takes it out), or else `fetch`'s. Only a request read
/// ahead that was dropped, or failed to reach the service, is asked again: any other error
/// would come back the same.
pub async fn or_fetch<T>(
    ahead: Option<Ahead>,
    pick: fn(Answer) -> Option<Result<T, ApiError>>,
    fetch: impl Future<Output = Result<T, ApiError>>,
) -> Result<T, ApiError> {
    if let Some(ahead) = ahead {
        match ahead.await.map(pick) {
            Ok(Some(Err(ApiError::Network(e)))) => log!("read ahead: {e}; asking again"),
            Ok(Some(answer)) => return answer,
            // Taken by its key, so never another page's answer.
            Ok(None) => {}
            Err(e) => log!("read ahead: {e}; asking again"),
        }
    }
    fetch.await
}

/// `page` is shown: what's still in flight for other pages is dropped, and so is what's
/// expired.
pub fn shown(page: &Page) {
    let now = Instant::now();
    KEPT_AHEAD.with(|k| k.borrow_mut().keep_only(page, now));
    SHOWN.with(|s| s.borrow_mut().expire(now));
}

/// Drops everything read ahead, and the pages left: the account changed, or the window
/// shows nothing.
pub fn clear() {
    KEPT_AHEAD.with(|k| k.borrow_mut().clear());
    SHOWN.with(|s| s.borrow_mut().pages.clear());
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// A request that's in flight until `landed` is set.
    #[derive(Clone, Default)]
    struct Fake {
        landed: Rc<Cell<bool>>,
        aborted: Rc<Cell<bool>>,
    }

    impl Request for Fake {
        fn in_flight(&self) -> bool {
            !self.landed.get()
        }

        fn abort(&self) {
            self.aborted.set(true);
        }
    }

    fn album(id: &str) -> Key {
        Key::Album(AlbumId(id.into()))
    }

    #[test]
    fn an_answer_older_than_a_minute_is_not_used() {
        let mut kept = Kept::new();
        let t0 = Instant::now();
        kept.insert(album("a"), Fake::default(), t0);
        kept.insert(album("b"), Fake::default(), t0);

        assert!(
            kept.take(&album("a"), t0 + KEEP - Duration::from_secs(1))
                .is_some()
        );
        assert!(
            kept.take(&album("b"), t0 + KEEP).is_none(),
            "expired, yet used"
        );
    }

    #[test]
    fn a_third_request_drops_the_oldest_still_in_flight() {
        let mut kept = Kept::new();
        let t0 = Instant::now();
        let (landed, first, second) = (Fake::default(), Fake::default(), Fake::default());
        landed.landed.set(true);
        kept.insert(album("in"), landed.clone(), t0);
        kept.insert(album("first"), first.clone(), t0);
        kept.insert(album("second"), second.clone(), t0);
        kept.insert(album("third"), Fake::default(), t0);

        assert!(first.aborted.get(), "three requests in flight");
        assert!(!second.aborted.get() && !landed.aborted.get());
        assert!(kept.has(&album("in"), t0), "an answer in was dropped");
        assert!(!kept.has(&album("first"), t0));
    }

    #[test]
    fn another_page_shown_drops_the_requests_in_flight_for_others() {
        let mut kept = Kept::new();
        let t0 = Instant::now();
        let (landed, flying, opened) = (Fake::default(), Fake::default(), Fake::default());
        landed.landed.set(true);
        kept.insert(album("in"), landed, t0);
        kept.insert(album("flying"), flying.clone(), t0);
        kept.insert(album("opened"), opened.clone(), t0);

        kept.keep_only(&album("opened").page(), t0);

        assert!(flying.aborted.get());
        assert!(!opened.aborted.get());
        assert!(kept.has(&album("in"), t0) && kept.has(&album("opened"), t0));
    }
}
