//! Album page (ui/pages/album.slint): the header, then the numbered track list. One call
//! per visit; the tracks stay here for Play, Shuffle and the song menu.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use featherplay_core::model::{AlbumId, Track};
use featherplay_core::tr;
use slint::{ComponentHandle, Model};

use crate::app::Ctx;
use crate::controller::PlayerState;
use crate::follow;
use crate::pages::content::{self, HEADER_COVER, TrackList};
use crate::ui::AlbumData;
use crate::ui_bridge::artist_names;

thread_local! {
    static LIST: RefCell<Option<TrackList>> = const { RefCell::new(None) };
}

/// Once at startup: connect `AlbumData`'s callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<AlbumData>();
    let c = ctx.clone();
    data.on_play(move |i| with_list(|l| l.play(&c, i.max(0) as usize, false)));
    let c = ctx.clone();
    data.on_shuffle(move || with_list(|l| l.shuffle(&c)));
    let c = ctx.clone();
    data.on_like(move |i| with_list(|l| l.toggle_like(&c, i.max(0) as usize)));
    let c = ctx.clone();
    data.on_retry(move || {
        // While the stored session can't connect, this load waits for a reconnect.
        c.reconnect();
        if let Some(ui) = c.window() {
            let id = AlbumId(ui.global::<AlbumData>().get_id().to_string());
            open(&c, &id);
        }
    });
}

/// The album on screen: its tracks in order, as loaded (none until it has loaded).
pub fn tracks() -> Vec<Track> {
    LIST.with(|l| {
        l.borrow()
            .as_ref()
            .map(|list| list.tracks.clone())
            .unwrap_or_default()
    })
}

fn with_list(f: impl FnOnce(&mut TrackList)) {
    LIST.with(|l| {
        if let Some(list) = l.borrow_mut().as_mut() {
            f(list);
        }
    });
}

/// The page is being shown.
pub fn open(ctx: &Rc<Ctx>, id: &AlbumId) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<AlbumData>();
    data.set_id(id.0.as_str().into());
    data.set_loading(true);
    data.set_error("".into());
    data.set_title("".into());
    data.set_artist("".into());
    data.set_artist_id("".into());
    data.set_kind("".into());
    data.set_meta("".into());
    data.set_cover(Default::default());
    data.set_wash_dark(Default::default());
    data.set_wash_light(Default::default());
    data.set_compilation(false);
    data.set_tracks(Default::default());
    LIST.with(|l| *l.borrow_mut() = None);

    let id = id.clone();
    ctx.load_for_page(
        move |source| async move {
            let started = Instant::now();
            (source.album(&id).await, started.elapsed())
        },
        |ctx, ui, (result, took)| {
            let data = ui.global::<AlbumData>();
            data.set_loading(false);
            let album = match result {
                Ok(album) => {
                    // One request: album pages have no continuations.
                    log!(
                        "album: {} tracks fetched in {} ms",
                        album.tracks.len(),
                        took.as_millis()
                    );
                    album
                }
                Err(e) => {
                    data.set_error(content::error_text("album", &e).into());
                    return;
                }
            };
            let s = &album.summary;
            crate::sidebar::opened(ctx, crate::sidebar::album_entry(s));
            data.set_title(s.title.as_str().into());
            data.set_artist(artist_names(&s.artists).into());
            data.set_artist_id(
                s.artists
                    .iter()
                    .find_map(|a| a.id.as_ref())
                    .map(|id| id.0.as_str())
                    .unwrap_or_default()
                    .into(),
            );
            let n = content::songs(album.tracks.len());
            let length = content::total_length(&album.tracks).unwrap_or_default();
            let year = s.year.clone().unwrap_or_default();
            data.set_kind(content::meta(&[tr!("Album"), &year]).into());
            data.set_meta(content::meta(&[&n, &length]).into());
            data.set_compilation(content::is_compilation(&s.artists, &album.tracks));
            let (list, model) = TrackList::new(album.tracks, ctx);
            list.hint_first(ctx);
            data.set_tracks(model);
            LIST.with(|l| *l.borrow_mut() = Some(list));
            if let Some(url) = &album.summary.thumbnail {
                let weak = ui.as_weak();
                let wanted = data.get_id();
                let c = ctx.clone();
                ctx.covers.load(url, HEADER_COVER, move |img| {
                    let Some(ui) = weak.upgrade() else { return };
                    let data = ui.global::<AlbumData>();
                    if data.get_id() != wanted {
                        return;
                    }
                    // The rows show no covers, but the song menu shows the album's.
                    let rows = data.get_tracks();
                    for i in 0..rows.row_count() {
                        if let Some(mut row) = rows.row_data(i) {
                            row.cover = img.clone();
                            rows.set_row_data(i, row);
                        }
                    }
                    let wash_weak = ui.as_weak();
                    let wash_wanted = wanted.clone();
                    follow::washes(&c, &img, move |w| {
                        let Some(ui) = wash_weak.upgrade() else {
                            return;
                        };
                        let data = ui.global::<AlbumData>();
                        if data.get_id() == wash_wanted {
                            data.set_wash_dark(w.dark);
                            data.set_wash_light(w.light);
                        }
                    });
                    data.set_cover(img);
                });
            }
        },
    );
}

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(_ctx: &Rc<Ctx>, state: &PlayerState) {
    let playing = content::playing(Some(state));
    with_list(|l| l.mark_playing(playing.as_ref()));
}

/// Signed in or out.
pub fn account_changed(_ctx: &Rc<Ctx>, _signed_in: bool) {}
