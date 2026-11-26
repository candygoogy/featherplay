//! Artist page (ui/pages/artist.slint): the profile header, top songs, albums, singles and
//! EPs, videos (song rows), the playlists it's featured on and related artists. The live
//! performances and the artist's own playlists are read (`Artist`) but not shown: the page's
//! design has no row for them. One call per visit, started early when the pointer rested on
//! its card (`ahead`).
//!
//! The four card shelves' covers are asked for as their cards come within a screen of what
//! shows, down the page and along each shelf (`ArtistData.near`), each once while the page is
//! open, at the width the cards are drawn at up to `CARD_COVER`: a shelf far down the page
//! decodes nothing until it's scrolled near.
//!
//! Top songs, Albums, Singles & EPs and Videos have a "Show all" when the page links to the
//! rest (`Artist::songs_all`, …): the songs and videos open on the playlist page under the
//! artist's header, the albums and singles on the discography page.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay_controller::place::Place;
use featherplay_core::model::ArtistId;
use featherplay_core::tr;
use featherplay_ytm::{
    AlbumSummary, Artist, ArtistSummary, ArtistVideo, BrowseRef, PlaylistSummary,
};
use slint::{ComponentHandle, Model, Timer, VecModel};

use crate::app::Ctx;
use crate::busy::Busy;
use crate::controller::{Action, PlayerState, PlayingFrom};
use crate::pages::ahead;
use crate::pages::content::{self, CARD_COVER, ROW_COVER, TrackList};
use crate::router::{Discography, FromArtist, Page};
use crate::ui::{ArtistData, CardItem, PageKind};
use crate::ui_bridge::{album_card, artist_card, model, playlist_card, shown};

thread_local! {
    static TOP: RefCell<Option<TrackList>> = const { RefCell::new(None) };
    static VIDEOS: RefCell<Option<TrackList>> = const { RefCell::new(None) };
    /// Albums, Singles & EPs, Featured on, Fans might also like (`ArtistData.near`'s order).
    static SHELVES: RefCell<[Option<NearShelf>; 4]> = const { RefCell::new([None, None, None, None]) };
    /// The cards' covers' size, in logical pixels: the width they're drawn at, at most
    /// `CARD_COVER` (`ArtistData.card-size`).
    static COVER_PX: Cell<u32> = const { Cell::new(CARD_COVER) };
    /// Where the page's shelves go on (`ArtistData.show-all`).
    static MORE: RefCell<Option<More>> = const { RefCell::new(None) };
    /// The banner: the artist's photo, and the size and ground it's made for
    /// (`ArtistData.banner-size`), once each is known; the wait before a new size is made.
    static BANNER: RefCell<Banner> = RefCell::new(Banner::default());
}

/// The banner's photo is made again this long after its size last changed (a window being
/// resized), not at every step.
const BANNER_SETTLE: Duration = Duration::from_millis(150);

#[derive(Default)]
struct Banner {
    url: Option<String>,
    /// Physical width and height, and the page's ground (`0xRRGGBB`).
    size: Option<((u32, u32), u32)>,
    /// A photo has come since the page opened: a new size then waits until it has settled
    /// (a resize). Before, it's asked for at once (the page's first layouts).
    shown: bool,
    settle: Timer,
    /// Held while `settle` runs: `--snapshot` and the tests wait for the photo it asks for.
    settling: Option<Busy>,
}

/// The banner is `width`×`height` physical pixels on `ground`: its photo is made for that, at
/// once until one has come, then once the size has settled.
fn banner_size(ctx: &Rc<Ctx>, width: i32, height: i32, ground: slint::Color) {
    let size = (
        (width.max(1) as u32, height.max(1) as u32),
        (u32::from(ground.red()) << 16)
            | (u32::from(ground.green()) << 8)
            | u32::from(ground.blue()),
    );
    let now = BANNER.with_borrow_mut(|b| {
        if b.size.replace(size) == Some(size) {
            return None;
        }
        Some(!b.shown)
    });
    match now {
        None => {}
        Some(true) => ask_banner(ctx),
        Some(false) => {
            let c = ctx.clone();
            BANNER.with_borrow_mut(|b| {
                b.settling.get_or_insert_with(Busy::on_timers);
                b.settle
                    .start(slint::TimerMode::SingleShot, BANNER_SETTLE, move || {
                        let settled = BANNER.with_borrow_mut(|b| b.settling.take());
                        ask_banner(&c);
                        drop(settled);
                    })
            });
        }
    }
}

/// Asks for the banner's photo at its size, once both are known: decoded and cut to fit off
/// the UI thread, with the gradient into the ground baked in (`Covers::banner`).
fn ask_banner(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let Some((url, size, ground)) = BANNER.with_borrow_mut(|b| {
        let (url, (size, ground)) = (b.url.clone()?, b.size?);
        Some((url, size, ground))
    }) else {
        return;
    };
    let weak = ui.as_weak();
    let wanted = ui.global::<ArtistData>().get_id();
    ctx.covers.banner(
        PageKind::Artist,
        "artist banner",
        &url,
        size,
        ground,
        move |img| {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<ArtistData>();
                if data.get_id() == wanted {
                    if img.size().width > 0 {
                        BANNER.with_borrow_mut(|b| b.shown = true);
                    }
                    data.set_banner(img);
                }
            }
        },
    );
}

/// Where the shown artist's shelves go on, and who it is.
struct More {
    artist: FromArtist,
    songs: Option<featherplay_core::model::PlaylistId>,
    videos: Option<featherplay_core::model::PlaylistId>,
    kinds: [Option<BrowseRef>; 2],
}

impl More {
    fn of(a: &Artist) -> More {
        More {
            artist: FromArtist {
                id: a.summary.id.clone(),
                name: a.summary.name.clone(),
                photo: a.summary.thumbnail.clone(),
            },
            songs: a.songs_all.clone(),
            videos: a.videos_all.clone(),
            kinds: [a.albums_all.clone(), a.singles_all.clone()],
        }
    }

    /// The page shelf `shelf` goes on to: 0 the songs, 1 the albums, 2 the singles, 3 the
    /// videos.
    fn page(&self, shelf: usize) -> Option<Page> {
        let artist = self.artist.clone();
        match shelf {
            0 | 3 => Some(Page::ArtistSongs {
                id: if shelf == 0 {
                    &self.songs
                } else {
                    &self.videos
                }
                .clone()?,
                artist,
                videos: shelf == 3,
            }),
            1 | 2 => Some(Page::Discography(Box::new(Discography {
                at: self.kinds[shelf - 1].clone()?,
                tab: shelf - 1,
                kinds: self.kinds.clone(),
                artist,
            }))),
            _ => None,
        }
    }
}

/// A shelf's "Show all" (`More::page`).
fn show_all(ctx: &Rc<Ctx>, shelf: usize) {
    if let Some(page) = MORE.with_borrow(|m| m.as_ref()?.page(shelf)) {
        ctx.navigate(page);
    }
}

/// A shelf of cards whose covers are asked for as they come near.
struct NearShelf {
    model: Rc<VecModel<CardItem>>,
    urls: Vec<Option<String>>,
    asked: Vec<bool>,
}

/// A shelf's cards `first` to `last` came near: their covers not asked for yet are.
fn near(ctx: &Ctx, shelf: usize, first: usize, last: usize) {
    let wanted = SHELVES.with_borrow_mut(|shelves| {
        let s = shelves.get_mut(shelf)?.as_mut()?;
        let end = last.min(s.urls.len().checked_sub(1)?);
        let mut urls = vec![None; s.urls.len()];
        for i in first..=end {
            if !std::mem::replace(&mut s.asked[i], true) {
                urls[i] = s.urls[i].clone();
            }
        }
        urls.iter()
            .any(Option::is_some)
            .then(|| (s.model.clone(), urls))
    });
    if let Some((model, urls)) = wanted {
        ctx.covers
            .cards(PageKind::Artist, &model, urls, COVER_PX.get());
    }
}

/// The cards are drawn `width` wide: covers come at that size up to `CARD_COVER`, and those
/// already in are decoded again at it (each shown scaled until then, `Covers::resize_cards`).
fn card_size(ctx: &Ctx, width: f32) {
    let px = (width.round().max(1.0) as u32).min(CARD_COVER);
    if COVER_PX.replace(px) == px {
        return;
    }
    let models: Vec<_> = SHELVES.with_borrow(|shelves| {
        shelves
            .iter()
            .flatten()
            .filter(|s| s.asked.contains(&true))
            .map(|s| s.model.clone())
            .collect()
    });
    for model in models {
        ctx.covers.resize_cards(&model, px);
    }
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
    data.on_near(move |shelf, first, last| {
        near(
            &c,
            shelf.max(0) as usize,
            first.max(0) as usize,
            last.max(0) as usize,
        )
    });
    let c = ctx.clone();
    data.on_card_size(move |width| card_size(&c, width));
    let c = ctx.clone();
    data.on_banner_size(move |width, height, ground| banner_size(&c, width, height, ground));
    let c = ctx.clone();
    data.on_show_all(move |shelf| show_all(&c, shelf.max(0) as usize));
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
    data.set_banner(Default::default());
    data.set_has_photo(false);
    BANNER.with_borrow_mut(|b| {
        b.url = None;
        b.shown = false;
        b.settle.stop();
        b.settling = None;
    });
    data.set_top_songs(Default::default());
    data.set_albums(Default::default());
    data.set_singles(Default::default());
    data.set_videos(Default::default());
    data.set_featured(Default::default());
    data.set_related(Default::default());
    data.set_more(Default::default());
    MORE.with_borrow_mut(|m| *m = None);
    TOP.with(|l| *l.borrow_mut() = None);
    VIDEOS.with(|l| *l.borrow_mut() = None);
    SHELVES.with_borrow_mut(|s| *s = [None, None, None, None]);

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
            let more = More::of(&artist);
            let links: Vec<bool> = (0..4).map(|i| more.page(i).is_some()).collect();
            data.set_more(slint::ModelRc::new(VecModel::from(links)));
            MORE.with_borrow_mut(|m| *m = Some(more));
            data.set_name(shown(&artist.summary.name));
            data.set_meta(tr!("Artist").into());
            data.set_has_photo(artist.summary.thumbnail.is_some());
            if let Some(url) = &artist.summary.thumbnail {
                // The banner's (`ask_banner`), made for its size once the page tells it.
                BANNER.with_borrow_mut(|b| b.url = Some(url.clone()));
                ask_banner(ctx);
                let weak = ui.as_weak();
                let wanted = data.get_id();
                // The ⋯ menu's small round photo.
                ctx.covers.single(
                    PageKind::Artist,
                    "artist photo",
                    url,
                    ROW_COVER,
                    move |img| {
                        if let Some(ui) = weak.upgrade() {
                            let data = ui.global::<ArtistData>();
                            if data.get_id() == wanted {
                                data.set_photo(img);
                            }
                        }
                    },
                );
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
            data.set_albums(cards(0, &artist.albums));
            data.set_singles(cards(1, &artist.singles));
            let from = PlayingFrom::on_page(
                artist.summary.name.clone(),
                Place::Artist(artist.summary.id.clone()),
            );
            let (videos, model) = video_list(ctx, &artist.videos, from);
            data.set_videos(model);
            VIDEOS.with(|l| *l.borrow_mut() = Some(videos));
            data.set_featured(playlist_cards(&artist.featured_on));
            data.set_related(artist_cards(&artist.related));
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

fn playlist_cards(playlists: &[PlaylistSummary]) -> slint::ModelRc<CardItem> {
    crate::sidebar::seen(playlists.iter().map(crate::sidebar::playlist_entry));
    let urls = playlists.iter().map(|p| p.thumbnail.clone()).collect();
    shelf(2, playlists.iter().map(playlist_card).collect(), urls)
}

fn artist_cards(artists: &[ArtistSummary]) -> slint::ModelRc<CardItem> {
    crate::sidebar::seen(artists.iter().map(crate::sidebar::artist_entry));
    let urls = artists.iter().map(|a| a.thumbnail.clone()).collect();
    shelf(3, artists.iter().map(artist_card).collect(), urls)
}

/// Albums (`at` 0) or singles (1).
fn cards(at: usize, albums: &[AlbumSummary]) -> slint::ModelRc<CardItem> {
    // For a pin from a card (`item_menu`): its title and cover in Shortcuts.
    crate::sidebar::seen(albums.iter().map(crate::sidebar::album_entry));
    let urls = albums.iter().map(|a| a.thumbnail.clone()).collect();
    shelf(at, albums.iter().map(album_card).collect(), urls)
}

/// Shelf `at`'s cards, their covers asked for as they come near (`near`).
fn shelf(at: usize, cards: Vec<CardItem>, urls: Vec<Option<String>>) -> slint::ModelRc<CardItem> {
    let (rows, model) = model(cards);
    SHELVES.with_borrow_mut(|s| {
        s[at] = Some(NearShelf {
            model: rows,
            asked: vec![false; urls.len()],
            urls,
        })
    });
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
