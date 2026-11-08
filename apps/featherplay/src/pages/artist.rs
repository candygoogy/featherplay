//! Artist page (ui/pages/artist.slint): the profile header, top songs, albums and singles.
//! One call per visit.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay_core::model::ArtistId;
use featherplay_core::tr;
use featherplay_ytm::AlbumSummary;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::{Action, PlayerState};
use crate::pages::content::{self, CARD_COVER, TrackList};

/// The banner photo's width, in logical pixels: the content width of a large window. The
/// photo is square, so the banner crops it from the top.
const BANNER: u32 = 1200;
use crate::ui::{ArtistData, CardItem, PageKind};
use crate::ui_bridge::{album_card, model};

thread_local! {
    static TOP: RefCell<Option<TrackList>> = const { RefCell::new(None) };
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
    TOP.with(|l| *l.borrow_mut() = None);

    let id = id.clone();
    ctx.load_for_page(
        move |source| async move { source.artist(&id).await },
        |ctx, ui, result| {
            let data = ui.global::<ArtistData>();
            data.set_loading(false);
            let artist = match result {
                Ok(a) => a,
                Err(e) => {
                    data.set_error(content::error_text("artist", &e).into());
                    return;
                }
            };
            crate::sidebar::opened(ctx, crate::sidebar::artist_entry(&artist.summary));
            data.set_name(artist.summary.name.as_str().into());
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
            list.hint_first(ctx);
            list.fill_covers(&ctx.covers);
            data.set_top_songs(model);
            TOP.with(|l| *l.borrow_mut() = Some(list));
            data.set_albums(cards(ctx, &artist.albums));
            data.set_singles(cards(ctx, &artist.singles));
        },
    );
}

fn cards(ctx: &Ctx, albums: &[AlbumSummary]) -> slint::ModelRc<CardItem> {
    let (rows, model) = model(albums.iter().map(album_card).collect());
    let urls = albums.iter().map(|a| a.thumbnail.clone()).collect();
    ctx.covers.cards(PageKind::Artist, &rows, urls, CARD_COVER);
    model
}

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(_ctx: &Rc<Ctx>, state: &PlayerState) {
    let playing = content::playing(Some(state));
    with_top(|l| l.mark_playing(playing.as_ref()));
}

/// Signed in or out.
pub fn account_changed(_ctx: &Rc<Ctx>, _signed_in: bool) {}
