//! Artist page (ui/pages/artist.slint): the profile header, top songs, albums, singles and
//! EPs, videos (song rows), the playlists it's featured on and related artists. The live
//! performances and the artist's own playlists are read (`Artist`) but not shown: the page's
//! design has no row for them. One call per visit, started early when the pointer rested on
//! its card (`ahead`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use featherplay_controller::place::Place;
use featherplay_core::model::ArtistId;
use featherplay_core::tr;
use featherplay_ytm::{AlbumSummary, ArtistSummary, ArtistVideo, PlaylistSummary};
use slint::{ComponentHandle, Model};

use crate::app::Ctx;
use crate::controller::{Action, PlayerState, PlayingFrom};
use crate::pages::ahead;
use crate::pages::content::{self, CARD_COVER, TrackList};
use crate::router::Page;

/// The banner photo's width, in logical pixels: the content width of a large window. The
/// photo is square, so the banner crops it from the top.
const BANNER: u32 = 1200;
use crate::ui::{ArtistData, CardItem, PageKind};
use crate::ui_bridge::{album_card, artist_card, model, playlist_card, shown};

thread_local! {
    static TOP: RefCell<Option<TrackList>> = const { RefCell::new(None) };
    static VIDEOS: RefCell<Option<TrackList>> = const { RefCell::new(None) };
}

/// Once at startup: connect `ArtistData`'s callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<ArtistData>();
    let c = ctx.clone();
    data.on_play(move |i| with_top(|l| l.play(&c, i.max(0) as usize, false)));
    let c = ctx.clone();
    data.on_radio(move || {
        with_top(|l| {
            if let Some(t) = l.tracks.first() {
                c.send(Action::Radio(t.id.clone()));
            }
        })
    });
    let c = ctx.clone();
    data.on_like(move |i| with_top(|l| l.toggle_like(&c, i.max(0) as usize)));
    let c = ctx.clone();
    data.on_play_video(move |i| with_videos(|l| l.play(&c, i.max(0) as usize, false)));
    let c = ctx.clone();
    data.on_like_video(move |i| with_videos(|l| l.toggle_like(&c, i.max(0) as usize)));
    let c = ctx.clone();
    data.on_play_card(move |it| content::play_card(&c, &it));
    let c = ctx.clone();
    data.on_retry(move || {
        // While the stored session can't connect, this load waits for a reconnect.
        c.reconnect();
        if let Some(ui) = c.window() {
            let id = ArtistId(ui.global::<ArtistData>().get_id().to_string());
            open(&c, &id);
        }
    });
}

fn with_top(f: impl FnOnce(&mut TrackList)) {
    TOP.with(|l| {
        if let Some(list) = l.borrow_mut().as_mut() {
            f(list);
        }
    });
}

fn with_videos(f: impl FnOnce(&mut TrackList)) {
    VIDEOS.with(|l| {
        if let Some(list) = l.borrow_mut().as_mut() {
            f(list);
        }
    });
}

/// The page is being shown.
pub fn open(ctx: &Rc<Ctx>, id: &ArtistId) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<ArtistData>();
    data.set_id(id.0.as_str().into());
    data.set_loading(true);
    data.set_error("".into());
    data.set_name("".into());
    data.set_meta("".into());
    data.set_photo(Default::default());
    data.set_has_photo(false);
    data.set_top_songs(Default::default());
    data.set_albums(Default::default());
    data.set_singles(Default::default());
    data.set_videos(Default::default());
    data.set_featured(Default::default());
    data.set_related(Default::default());
    TOP.with(|l| *l.borrow_mut() = None);
    VIDEOS.with(|l| *l.borrow_mut() = None);

    let ahead = ahead::artist(ctx, id);
    let read_ahead = if ahead.is_some() { " (read ahead)" } else { "" };
    let (page, asked) = (Page::Artist { id: id.clone() }, ahead::changes());
    let id = id.clone();
    ctx.load_for_page(
        move |source| async move {
            let started = Instant::now();
            let artist = ahead::or_fetch(ahead, ahead::Answer::artist, source.artist(&id)).await;
            (artist, started.elapsed())
        },
        move |ctx, ui, (result, took)| {
            let data = ui.global::<ArtistData>();
            data.set_loading(false);
            let artist = match result {
                Ok(a) => {
                    log!("artist: fetched in {} ms{read_ahead}", took.as_millis());
                    ahead::showing(&page, ahead::Answer::Artist(Ok(a.clone())), asked);
                    a
                }
                Err(e) => {
                    data.set_error(content::error_text("artist", &e).into());
                    return;
                }
            };
            crate::sidebar::opened(ctx, crate::sidebar::artist_entry(&artist.summary));
            data.set_name(shown(&artist.summary.name));
            data.set_meta(tr!("Artist").into());
            data.set_has_photo(artist.summary.thumbnail.is_some());
            if let Some(url) = &artist.summary.thumbnail {
                let weak = ui.as_weak();
                let wanted = data.get_id();
                // 1200 px across at 150 %: 13 MB decoded, let go when the page is.
                ctx.covers
                    .single(PageKind::Artist, "artist photo", url, BANNER, move |img| {
                        if let Some(ui) = weak.upgrade() {
                            let data = ui.global::<ArtistData>();
                            if data.get_id() == wanted {
                                data.set_photo(img);
                            }
                        }
                    });
            }
            let (mut list, model) = TrackList::new(artist.top_songs, ctx);
            // Up next names the artist, and Shortcuts learns a play from here.
            list.from = PlayingFrom::on_page(
                artist.summary.name.clone(),
                Place::Artist(artist.summary.id.clone()),
            );
            list.hint_first(ctx);
            list.fill_covers(&ctx.covers);
            data.set_top_songs(model);
            TOP.with(|l| *l.borrow_mut() = Some(list));
            data.set_albums(cards(ctx, &artist.albums));
            data.set_singles(cards(ctx, &artist.singles));
            let from = PlayingFrom::on_page(
                artist.summary.name.clone(),
                Place::Artist(artist.summary.id.clone()),
            );
            let (videos, model) = video_list(ctx, &artist.videos, from);
            data.set_videos(model);
            VIDEOS.with(|l| *l.borrow_mut() = Some(videos));
            data.set_featured(playlist_cards(ctx, &artist.featured_on));
            data.set_related(artist_cards(ctx, &artist.related));
        },
    );
}

/// The videos as song rows, their views where an album would be (the cards give no
/// length), playing from the artist's page.
fn video_list(
    ctx: &Ctx,
    videos: &[ArtistVideo],
    from: PlayingFrom,
) -> (TrackList, slint::ModelRc<crate::ui::TrackRow>) {
    let tracks = videos.iter().map(|v| v.track.clone()).collect();
    let (mut list, model) = TrackList::new(tracks, ctx);
    for (i, v) in videos.iter().enumerate() {
        if let (Some(views), Some(mut row)) = (&v.views, list.rows.row_data(i)) {
            row.album = views.as_str().into();
            list.rows.set_row_data(i, row);
        }
    }
    list.from = from;
    list.fill_covers(&ctx.covers);
    (list, model)
}

fn playlist_cards(ctx: &Ctx, playlists: &[PlaylistSummary]) -> slint::ModelRc<CardItem> {
    crate::sidebar::seen(playlists.iter().map(crate::sidebar::playlist_entry));
    let (rows, model) = model(playlists.iter().map(playlist_card).collect());
    let urls = playlists.iter().map(|p| p.thumbnail.clone()).collect();
    ctx.covers.cards(PageKind::Artist, &rows, urls, CARD_COVER);
    model
}

fn artist_cards(ctx: &Ctx, artists: &[ArtistSummary]) -> slint::ModelRc<CardItem> {
    crate::sidebar::seen(artists.iter().map(crate::sidebar::artist_entry));
    let (rows, model) = model(artists.iter().map(artist_card).collect());
    let urls = artists.iter().map(|a| a.thumbnail.clone()).collect();
    ctx.covers.cards(PageKind::Artist, &rows, urls, CARD_COVER);
    model
}

fn cards(ctx: &Ctx, albums: &[AlbumSummary]) -> slint::ModelRc<CardItem> {
    // For a pin from a card (`item_menu`): its title and cover in Shortcuts.
    crate::sidebar::seen(albums.iter().map(crate::sidebar::album_entry));
    let (rows, model) = model(albums.iter().map(album_card).collect());
    let urls = albums.iter().map(|a| a.thumbnail.clone()).collect();
    ctx.covers.cards(PageKind::Artist, &rows, urls, CARD_COVER);
    model
}

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(_ctx: &Rc<Ctx>, state: &PlayerState) {
    let playing = content::playing(Some(state));
    with_top(|l| l.mark_playing(playing.as_ref()));
    with_videos(|l| l.mark_playing(playing.as_ref()));
}

/// Signed in or out.
pub fn account_changed(_ctx: &Rc<Ctx>, _signed_in: bool) {}
