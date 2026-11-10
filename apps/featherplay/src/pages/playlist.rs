//! Playlist page (ui/pages/playlist.slint), also Liked songs (`"LM"`): the header, then song
//! rows whose covers load as they scroll into view. One load per visit, a page at a time:
//! the header and the first songs show as soon as the first page is read, and the songs of
//! each later page follow them as it comes, with the count and length in the header. The
//! first page may have been read already, when the pointer rested on its card (`ahead`).

use std::cell::{Cell, RefCell};
use std::ops::ControlFlow;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay_core::model::{PlaylistId, Track};
use featherplay_core::tr;
use featherplay_ytm::{ApiError, Playlist, PlaylistPart};
use slint::ComponentHandle;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::Ctx;
use crate::catalog::Source;
use crate::controller::{PlayerState, PlayingFrom};
use crate::follow;
use crate::pages::ahead;
use crate::pages::content::{self, Arriving, HEADER_COVER, TrackList};
use crate::router::Page;
use crate::ui::{AppWindow, PlaylistData};

thread_local! {
    static LIST: RefCell<Option<TrackList>> = const { RefCell::new(None) };
    /// `open`s, counted: a load's late pages never land in a later one's list, even on the
    /// same visit (Liked songs opens again for another account).
    static OPENED: Cell<u64> = const { Cell::new(0) };
}

/// What a playlist's load sends from the runtime (`fetch`): each page as it's read, then
/// the end, with the time since the load started.
pub enum Part {
    Page(PlaylistPart, Duration),
    /// The pages read, or the error that ended the load.
    End(Result<usize, ApiError>, Duration),
}

/// Reads playlist `id` a page at a time onto `parts`, the first one from `ahead` when it was
/// read ahead, and asks for no more pages once they're no longer wanted (`parts` is closed).
pub async fn fetch(
    source: Source,
    id: PlaylistId,
    ahead: Option<ahead::Ahead>,
    parts: UnboundedSender<Part>,
) {
    let started = Instant::now();
    let send = |part| match parts.send(Part::Page(part, started.elapsed())) {
        Ok(()) => ControlFlow::Continue(()),
        Err(_) => ControlFlow::Break(()),
    };
    let result =
        match ahead::or_fetch(ahead, ahead::Answer::playlist, source.playlist_first(&id)).await {
            Ok((first, rest)) => {
                let wanted = send(PlaylistPart::First(first)).is_continue();
                match rest {
                    Some(rest) if wanted => source.playlist_rest(rest, send).await,
                    _ => Ok(1),
                }
            }
            Err(e) => Err(e),
        };
    // Nobody may be listening any more.
    let _ = parts.send(Part::End(result, started.elapsed()));
}

/// The page's title and the Up next panel's "Playing from" for Liked songs.
pub fn liked_songs() -> &'static str {
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
    let opened = OPENED.with(|o| {
        o.set(o.get() + 1);
        o.get()
    });
    // Signed out, Liked songs is the account's: the page asks to sign in instead.
    if liked && ctx.signed_out() {
        data.set_loading(false);
        return;
    }

    let page = Page::Playlist { id: id.clone() };
    let visit = ctx.visit();
    // A play from the list while its pages arrive: the ones to come join its queue, even
    // once the page is left.
    let arriving = Arriving::default();
    let mut tracks = 0;
    let wanted = id.clone();
    let ahead = ahead::playlist(ctx, id);
    let read_ahead = if ahead.is_some() { " (read ahead)" } else { "" };
    // Its header, kept with the whole list once it's in, for Back and Forward (`ahead`),
    // unless it's changed here meanwhile.
    let mut summary = None;
    let asked = ahead::changes();
    ctx.stream(
        // Liked songs loads like any playlist; the header shown for it is ours.
        move |source, parts| fetch(source, wanted, ahead, parts),
        move |ctx, ui, part| {
            let here = ctx.visit() == visit && OPENED.with(Cell::get) == opened;
            match part {
                Part::Page(PlaylistPart::First(p), took) => {
                    if !here {
                        return ControlFlow::Break(());
                    }
                    tracks = p.tracks.len();
                    summary = Some(p.summary.clone());
                    log!(
                        "playlist: first page, {tracks} tracks in {} ms{read_ahead}",
                        took.as_millis()
                    );
                    show(ctx, ui, p, page.clone(), &arriving);
                    ControlFlow::Continue(())
                }
                Part::Page(PlaylistPart::More(more), _) => {
                    tracks += more.len();
                    let joined = arriving.join(ctx, &more);
                    if here {
                        with_list(|l| {
                            l.append(more);
                            set_meta(ui, &l.tracks);
                        });
                    }
                    if here || joined {
                        ControlFlow::Continue(())
                    } else {
                        ControlFlow::Break(())
                    }
                }
                Part::End(result, took) => {
                    match result {
                        // The page count includes the first: continuations are the rest.
                        Ok(pages) => {
                            log!(
                                "playlist: {tracks} tracks fetched in {} ms ({} continuation(s))",
                                took.as_millis(),
                                pages.saturating_sub(1)
                            );
                            if here && let Some(summary) = summary.take() {
                                with_list(|l| {
                                    let whole = Playlist {
                                        summary,
                                        tracks: l.tracks.clone(),
                                        pages,
                                    };
                                    ahead::showing(
                                        &page,
                                        ahead::Answer::Playlist(Ok((whole, None))),
                                        asked,
                                    );
                                });
                            }
                        }
                        Err(e) if here && ui.global::<PlaylistData>().get_loading() => {
                            let data = ui.global::<PlaylistData>();
                            data.set_loading(false);
                            data.set_error(content::error_text("playlist", &e).into());
                        }
                        // The songs shown stay; the list ends there.
                        Err(e) => log!("playlist: a later page didn't load: {e}"),
                    }
                    if here {
                        with_list(|l| l.arriving = None);
                    }
                    ControlFlow::Break(())
                }
            }
        },
    );
}

/// The first page has arrived: the header, the songs so far, and the list that plays them.
fn show(ctx: &Rc<Ctx>, ui: &AppWindow, playlist: Playlist, page: Page, arriving: &Arriving) {
    let data = ui.global::<PlaylistData>();
    data.set_loading(false);
    let liked = data.get_liked_songs();
    let s = &playlist.summary;
    crate::sidebar::opened(ctx, crate::sidebar::playlist_entry(s));
    set_meta(ui, &playlist.tracks);
    if !liked {
        data.set_title(crate::ui_bridge::shown(&s.title));
        data.set_byline(crate::ui_bridge::shown(
            s.author.as_deref().unwrap_or_default(),
        ));
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
    let label = if liked {
        liked_songs().into()
    } else {
        s.title.clone()
    };
    let (mut list, model) = TrackList::new(playlist.tracks, ctx);
    list.from = PlayingFrom::on_page(label, page);
    list.arriving = Some(arriving.clone());
    list.hint_first(ctx);
    data.set_tracks(model);
    LIST.with(|l| *l.borrow_mut() = Some(list));
}

/// The header's song count and length, for the songs arrived so far.
fn set_meta(ui: &AppWindow, tracks: &[Track]) {
    let n = content::songs(tracks.len());
    let length = content::total_length(tracks).unwrap_or_default();
    ui.global::<PlaylistData>()
        .set_meta(content::meta(&[n.as_str(), &length]).into());
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
