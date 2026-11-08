//! Playlist page (ui/pages/playlist.slint), also Liked songs (`"LM"`): the header, then song
//! rows whose covers load as they scroll into view. One call per visit.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use featherplay_core::model::PlaylistId;
use featherplay_core::tr;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::{PlayerState, PlayingFrom};
use crate::follow;
use crate::pages::content::{self, HEADER_COVER, TrackList};
use crate::router::Page;
use crate::ui::PlaylistData;

thread_local! {
    static LIST: RefCell<Option<TrackList>> = const { RefCell::new(None) };
}

/// The page's title and the Up next panel's "Playing from" for Liked songs.
fn liked_songs() -> &'static str {
    tr!("Liked songs")
}

/// Liked songs, as the sidebar and the Home feed name it.
pub fn is_liked_songs(id: &PlaylistId) -> bool {
    matches!(id.0.as_str(), "LM" | "VLLM")
}

/// Once at startup: connect `PlaylistData`'s callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<PlaylistData>();
    let c = ctx.clone();
    data.on_play(move |i| with_list(|l| l.play(&c, i.max(0) as usize, false)));
    let c = ctx.clone();
    data.on_shuffle(move || with_list(|l| l.shuffle(&c)));
    let c = ctx.clone();
    data.on_like(move |i| with_list(|l| l.toggle_like(&c, i.max(0) as usize)));
    let c = ctx.clone();
    data.on_cover_needed(move |i| with_list(|l| l.need_cover(&c.covers, i.max(0) as usize)));
    let c = ctx.clone();
    data.on_retry(move || {
        // While the stored session can't connect, this load waits for a reconnect.
        c.reconnect();
        if let Some(ui) = c.window() {
            let id = PlaylistId(ui.global::<PlaylistData>().get_id().to_string());
            open(&c, &id);
        }
    });
}

fn with_list(f: impl FnOnce(&mut TrackList)) {
    LIST.with(|l| {
        if let Some(list) = l.borrow_mut().as_mut() {
            f(list);
        }
    });
}

/// The page is being shown.
pub fn open(ctx: &Rc<Ctx>, id: &PlaylistId) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<PlaylistData>();
    let liked = is_liked_songs(id);
    data.set_id(id.0.as_str().into());
    data.set_liked_songs(liked);
    data.set_loading(true);
    data.set_error("".into());
    data.set_title(if liked { liked_songs() } else { "" }.into());
    data.set_kind(tr!("Playlist").into());
    data.set_byline("".into());
    data.set_meta("".into());
    data.set_cover(Default::default());
    data.set_wash_dark(Default::default());
    data.set_wash_light(Default::default());
    data.set_tracks(Default::default());
    LIST.with(|l| *l.borrow_mut() = None);
    // Signed out, Liked songs is the account's: the page asks to sign in instead.
    if liked && ctx.signed_out() {
        data.set_loading(false);
        return;
    }

    let page = Page::Playlist { id: id.clone() };
    let id = id.clone();
    ctx.load_for_page(
        // Liked songs loads like any playlist; the header shown for it is ours.
        move |source| async move {
            let started = Instant::now();
            (source.playlist(&id).await, started.elapsed())
        },
        move |ctx, ui, (result, took)| {
            let data = ui.global::<PlaylistData>();
            data.set_loading(false);
            let playlist = match result {
                Ok(p) => {
                    // The page count includes the first: continuations are the rest.
                    log!(
                        "playlist: {} tracks fetched in {} ms ({} continuation(s))",
                        p.tracks.len(),
                        took.as_millis(),
                        p.pages.saturating_sub(1)
                    );
                    p
                }
                Err(e) => {
                    data.set_error(content::error_text("playlist", &e).into());
                    return;
                }
            };
            let s = &playlist.summary;
            crate::sidebar::opened(ctx, crate::sidebar::playlist_entry(s));
            let n = content::songs(playlist.tracks.len());
            let length = content::total_length(&playlist.tracks).unwrap_or_default();
            if liked {
                data.set_meta(content::meta(&[n.as_str(), &length]).into());
            } else {
                data.set_title(crate::ui_bridge::shown(&s.title));
                data.set_byline(crate::ui_bridge::shown(
                    s.author.as_deref().unwrap_or_default(),
                ));
                data.set_meta(content::meta(&[&n, &length]).into());
                // Liked songs shows its own heart instead of the service's artwork.
                if let Some(url) = &s.thumbnail {
                    let weak = ui.as_weak();
                    let wanted = data.get_id();
                    let c = ctx.clone();
                    ctx.covers.load(url, HEADER_COVER, move |img| {
                        let Some(ui) = weak.upgrade() else { return };
                        let data = ui.global::<PlaylistData>();
                        if data.get_id() != wanted {
                            return;
                        }
                        let wash_weak = ui.as_weak();
                        follow::washes(&c, &img, move |w| {
                            let Some(ui) = wash_weak.upgrade() else {
                                return;
                            };
                            let data = ui.global::<PlaylistData>();
                            if data.get_id() == wanted {
                                data.set_wash_dark(w.dark);
                                data.set_wash_light(w.light);
                            }
                        });
                        data.set_cover(img);
                    });
                }
            }
            let (mut list, model) = TrackList::new(playlist.tracks, ctx);
            let label = if liked {
                liked_songs().into()
            } else {
                s.title.clone()
            };
            list.from = PlayingFrom::on_page(label, page);
            list.hint_first(ctx);
            data.set_tracks(model);
            LIST.with(|l| *l.borrow_mut() = Some(list));
        },
    );
}

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(_ctx: &Rc<Ctx>, state: &PlayerState) {
    let playing = content::playing(Some(state));
    with_list(|l| l.mark_playing(playing.as_ref()));
}

/// Signed in or out: Liked songs on screen is another account's, or none, so it opens again.
pub fn account_changed(ctx: &Rc<Ctx>, _signed_in: bool) {
    if let crate::router::Page::Playlist { id } = ctx.current_page()
        && is_liked_songs(&id)
    {
        open(ctx, &id);
    }
}
