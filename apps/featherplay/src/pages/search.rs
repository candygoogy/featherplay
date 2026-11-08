//! Search page (ui/pages/search.slint): the query from the top bar, a filter chip per kind.
//! Each filter is fetched once per query and kept while the page shows that query, so
//! switching chips back and forth doesn't call again. A song link (`link::parse`) shows
//! that one song instead, from one lookup of its id, without chips; it plays only when asked.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay_ytm::{SearchFilter, SearchResults};
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::PlayerState;
use crate::link::{self, Link};
use crate::pages::content::{self, CARD_COVER, TrackList};
use crate::ui::{PageKind, SearchData};
use crate::ui_bridge::{album_card, artist_card, model, playlist_card};

const FILTERS: [SearchFilter; 4] = [
    SearchFilter::Songs,
    SearchFilter::Albums,
    SearchFilter::Artists,
    SearchFilter::Playlists,
];

#[derive(Default)]
struct State {
    query: String,
    filter: usize,
    results: [Option<SearchResults>; 4],
    songs: Option<TrackList>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Once at startup: connect `SearchData`'s callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SearchData>();
    let c = ctx.clone();
    data.on_set_filter(move |i| {
        let i = (i.max(0) as usize).min(FILTERS.len() - 1);
        STATE.with(|s| s.borrow_mut().filter = i);
        show(&c);
    });
    // A song from search plays with its radio after it.
    let c = ctx.clone();
    data.on_play(move |i| with_songs(|l| l.play_one(&c, i.max(0) as usize)));
    let c = ctx.clone();
    data.on_like(move |i| with_songs(|l| l.toggle_like(&c, i.max(0) as usize)));
    let c = ctx.clone();
    data.on_play_card(move |it| content::play_card(&c, &it));
    let c = ctx.clone();
    data.on_retry(move || {
        // While the stored session can't connect, this load waits for a reconnect.
        c.reconnect();
        show(&c);
    });
}

fn with_songs(f: impl FnOnce(&mut TrackList)) {
    STATE.with(|s| {
        if let Some(list) = s.borrow_mut().songs.as_mut() {
            f(list);
        }
    });
}

/// The page is being shown.
pub fn open(ctx: &Rc<Ctx>, query: &str) {
    let Some(ui) = ctx.window() else { return };
    ui.global::<SearchData>().set_query(query.into());
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.query != query {
            *s = State {
                query: query.to_string(),
                ..State::default()
            };
        }
    });
    show(ctx);
}

/// Shows the current filter's results, fetching them first if needed.
fn show(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SearchData>();
    // A song link has its one result, under the Songs filter.
    let track = match link::parse(&STATE.with(|s| s.borrow().query.clone())) {
        Some(Link::Track(id)) => Some(id),
        _ => None,
    };
    if track.is_some() {
        STATE.with(|s| s.borrow_mut().filter = 0);
    }
    let (query, filter, cached) = STATE.with(|s| {
        let s = s.borrow();
        (s.query.clone(), s.filter, s.results[s.filter].clone())
    });
    data.set_link(track.is_some());
    data.set_filter(filter as i32);
    data.set_error("".into());
    data.set_songs(Default::default());
    data.set_cards(Default::default());
    STATE.with(|s| s.borrow_mut().songs = None);
    if let Some(results) = cached {
        data.set_loading(false);
        fill(ctx, filter, results);
        return;
    }
    data.set_loading(true);
    let kind = FILTERS[filter];
    ctx.load_for_page(
        move |source| async move {
            match track {
                Some(id) => Ok(SearchResults {
                    songs: source.track(&id).await?.into_iter().collect(),
                    ..SearchResults::default()
                }),
                None => source.search(&query, kind).await,
            }
        },
        move |ctx, ui, result| {
            let current = STATE.with(|s| s.borrow().filter) == filter;
            match result {
                Ok(results) => {
                    STATE.with(|s| s.borrow_mut().results[filter] = Some(results.clone()));
                    if current {
                        ui.global::<SearchData>().set_loading(false);
                        fill(ctx, filter, results);
                    }
                }
                Err(e) if current => {
                    let data = ui.global::<SearchData>();
                    data.set_loading(false);
                    data.set_error(content::error_text("search", &e).into());
                }
                Err(e) => log!("search: {e}"),
            }
        },
    );
}

fn fill(ctx: &Rc<Ctx>, filter: usize, r: SearchResults) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SearchData>();
    let cards: Vec<_> = match FILTERS[filter] {
        SearchFilter::Songs => {
            let (mut list, model) = TrackList::new(r.songs, ctx);
            list.fill_covers(&ctx.covers);
            data.set_songs(model);
            STATE.with(|s| s.borrow_mut().songs = Some(list));
            return;
        }
        SearchFilter::Albums => r
            .albums
            .iter()
            .map(|a| (album_card(a), a.thumbnail.clone()))
            .collect(),
        SearchFilter::Artists => r
            .artists
            .iter()
            .map(|a| (artist_card(a), a.thumbnail.clone()))
            .collect(),
        SearchFilter::Playlists => r
            .playlists
            .iter()
            .map(|p| (playlist_card(p), p.thumbnail.clone()))
            .collect(),
    };
    let (items, urls): (Vec<_>, Vec<_>) = cards.into_iter().unzip();
    let (rows, model) = model(items);
    data.set_cards(model);
    ctx.covers.cards(PageKind::Search, &rows, urls, CARD_COVER);
}

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(_ctx: &Rc<Ctx>, state: &PlayerState) {
    let playing = content::playing(Some(state));
    with_songs(|l| l.mark_playing(playing.as_ref()));
}

/// Signed in or out.
pub fn account_changed(_ctx: &Rc<Ctx>, signed_in: bool) {
    // Results may differ per account; fetch again next time.
    if !signed_in {
        STATE.with(|s| s.borrow_mut().query.clear());
    }
}
