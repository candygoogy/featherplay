//! The song rows on screen, sent to the controller to resolve ahead of a click
//! (`Action::Prepare`, `controller/streams.rs`): a playlist's, an album's or Liked songs'
//! rows, search results, Home's song shelves, an artist's top songs, Related and Up next.
//!
//! The lists tell which rows show through `ShownRows` (`RowsShown`, ui/parts.slint), as they
//! open and whenever that changes. Once nothing has changed for `SETTLE` (a list opened, a
//! scroll ended), the rows go to the controller, the top of each list first, page lists
//! before the side panel's. Another page sends at once what's left (the frame's side
//! panel), so what the page showed stops; minimised or hidden in the tray, the window sends
//! none until it shows again. The settle is one single-shot timer, restarted by a change:
//! at rest nothing runs.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use featherplay_core::model::{Track, TrackId};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode};

use crate::app::Ctx;
use crate::controller::Action;
use crate::song_menu;
use crate::ui::{
    AlbumData, AppWindow, ArtistData, HomeData, LibraryData, NowPlayingData, PageKind,
    PlaylistData, QueueData, SearchData, ShownRows, SongList, TrackRow,
};

/// How long the rows on screen stay the same before they're sent: a scroll has ended.
const SETTLE: Duration = Duration::from_millis(300);
/// The rows sent at most: the controller resolves no more for one screen.
const ROWS: usize = crate::controller::PREPARE_ROWS;

/// A list's rows on screen: a list, its part (a Home shelf, a side panel), first, last.
type Shown = (SongList, i32, i32, i32);

#[derive(Default)]
struct State {
    /// What each list shows, in the order sent (`rank`).
    lists: BTreeMap<(u8, i32), Shown>,
    hidden: bool,
    /// The rows last sent.
    sent: Vec<TrackId>,
    settle: Timer,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// Page lists first, top to bottom, then the side panel's.
fn rank(list: SongList) -> u8 {
    match list {
        SongList::Playlist => 0,
        SongList::Album => 1,
        SongList::Liked => 2,
        SongList::Search => 3,
        SongList::Artist => 4,
        SongList::Home => 5,
        SongList::Related => 6,
        SongList::UpNext => 7,
    }
}

/// The frame's side panel stays across pages; Now playing's goes with its page.
fn outlives_pages(list: SongList, part: i32) -> bool {
    matches!(list, SongList::Related | SongList::UpNext) && part == 0
}

/// Whether `list` is on page `page`, or on every page.
fn is_on(list: SongList, part: i32, page: PageKind) -> bool {
    outlives_pages(list, part)
        || page
            == match list {
                SongList::Playlist => PageKind::Playlist,
                SongList::Album => PageKind::Album,
                SongList::Liked => PageKind::Library,
                SongList::Search => PageKind::Search,
                SongList::Artist => PageKind::Artist,
                SongList::Home => PageKind::Home,
                SongList::Related | SongList::UpNext => PageKind::NowPlaying,
            }
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let c = Rc::downgrade(ctx);
    ui.global::<ShownRows>()
        .on_shown(move |list, part, first, last| {
            // A page on its way out may still say what it shows as it goes.
            let Some(ctx) = c.upgrade() else { return };
            let on = is_on(list, part, ctx.current_page().kind());
            shown(list, part, first, if on { last } else { -1 });
        });
}

/// `list`'s part `part` shows rows `first` to `last` (none when `last < first`).
fn shown(list: SongList, part: i32, first: i32, last: i32) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let key = (rank(list), part);
        let was = if last < first {
            s.lists.remove(&key)
        } else {
            s.lists.insert(key, (list, part, first, last))
        };
        // The same as before, or nothing to send while hidden (`hidden` sends on showing).
        if was == s.lists.get(&key).copied() || s.hidden {
            return;
        }
        s.settle.start(TimerMode::SingleShot, SETTLE, || {
            if let Some(ctx) = Ctx::get() {
                send(&ctx);
            }
        });
    });
}

/// Another page shows: its lists say what they show once they're in; what the last page
/// showed stops now.
pub fn page_changed(ctx: &Ctx) {
    STATE.with(|s| {
        s.borrow_mut()
            .lists
            .retain(|_, &mut (list, part, ..)| outlives_pages(list, part));
    });
    send(ctx);
}

/// The window was minimised or hidden in the tray (`true`), or shows again (`memory`).
pub fn hidden(ctx: &Ctx, hidden: bool) {
    STATE.with(|s| s.borrow_mut().hidden = hidden);
    send(ctx);
}

/// Sends the rows on screen, unless they're the ones sent last.
fn send(ctx: &Ctx) {
    let rows = match ctx.window() {
        Some(ui) if !STATE.with(|s| s.borrow().hidden) => on_screen(&ui),
        _ => Vec::new(),
    };
    let ids: Vec<TrackId> = rows.iter().map(|(id, _)| id.clone()).collect();
    let same = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.settle.stop();
        if s.sent == ids {
            return true;
        }
        s.sent = ids;
        false
    });
    if !same {
        ctx.send(Action::Prepare(rows));
    }
}

/// The rows the lists show, each once, at most `ROWS`, with their tracks when known.
fn on_screen(ui: &AppWindow) -> Vec<(TrackId, Option<Track>)> {
    let lists: Vec<Shown> = STATE.with(|s| s.borrow().lists.values().copied().collect());
    let mut out: Vec<(TrackId, Option<Track>)> = Vec::new();
    for (list, part, first, last) in lists {
        let Some(rows) = rows(ui, list, part) else {
            continue;
        };
        let (Ok(first), Ok(last)) = (usize::try_from(first), usize::try_from(last)) else {
            continue;
        };
        for row in (first..=last).filter_map(|i| rows.row_data(i)) {
            if out.len() == ROWS {
                return out;
            }
            if row.id.is_empty() || out.iter().any(|(id, _)| id.0 == row.id.as_str()) {
                continue;
            }
            // Up next's rows are the queue's tracks: the controller has them, as they play.
            let track = match list {
                SongList::UpNext => None,
                _ => song_menu::find(&row.id),
            };
            out.push((TrackId(row.id.to_string()), track));
        }
    }
    out
}

fn rows(ui: &AppWindow, list: SongList, part: i32) -> Option<ModelRc<TrackRow>> {
    Some(match list {
        SongList::Playlist => ui.global::<PlaylistData>().get_tracks(),
        SongList::Album => ui.global::<AlbumData>().get_tracks(),
        SongList::Liked => ui.global::<LibraryData>().get_liked(),
        SongList::Search => ui.global::<SearchData>().get_songs(),
        SongList::Artist => ui.global::<ArtistData>().get_top_songs(),
        SongList::Home => {
            let shelf = usize::try_from(part).ok()?;
            ui.global::<HomeData>().get_shelves().row_data(shelf)?.songs
        }
        SongList::Related => ui.global::<NowPlayingData>().get_related(),
        SongList::UpNext => ui.global::<QueueData>().get_upcoming(),
    })
}
