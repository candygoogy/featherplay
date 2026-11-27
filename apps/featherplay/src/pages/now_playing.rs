//! Now playing page (ui/pages/now_playing.slint). The track itself comes
//! from the `Player` global (`bar.rs`) and the Up next tab from `QueueData`
//! (`queue_panel.rs`); this module fills the large cover, the album's year and the Related
//! tab, and opens the song menu for the current track.
//!
//! Requests stay small: Related is the current track's radio, fetched once per track when
//! the tab is shown, or once ahead of it when Home shows a short queue and something on
//! screen would show them (`want_related`); the year comes from the album, fetched once per
//! album while the page is open. Results are kept, so reopening the page asks for nothing
//! again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use featherplay_core::model::{AlbumId, Track, TrackId};
use featherplay_core::tr;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::{Action, PlayerState, PlayingFrom, QueueAction};
use crate::router::Page;
use crate::song_menu;
use crate::ui::{
    AppWindow, FieldBands, Frame, LyricsData, NowPlayingData, Player, SongMenu, Theme,
};
use crate::ui_bridge::{model, to_rows};
use crate::{follow, lyrics};

mod backdrop;
mod landing;

pub use landing::Colours;

/// The cover's largest size beside the side panel, in logical pixels (`Theme.nowplaying-cover`).
const COVER: u32 = 440;
/// Its largest with the side panel hidden (`Theme.nowplaying-cover-wide`), decoded while the
/// page draws it larger than `COVER` (`NowPlayingData.cover-wide`).
const COVER_WIDE: u32 = 560;
/// Related row covers.
const ROW_COVER: u32 = 40;
/// A queue shorter than this gets "More like this" in Home's band (`hero.rs::HERO_COLUMN`).
const SHORT_QUEUE: usize = 3;

#[derive(Default)]
struct State {
    /// The track the page shows.
    track: Option<TrackId>,
    /// The track the frame's Up next panel last showed Related for.
    side_track: Option<TrackId>,
    cover_url: String,
    /// Related tracks by seed; `None` while the request runs.
    related: HashMap<TrackId, Option<Vec<Track>>>,
    /// The songs whose related tracks were asked for ahead of the tab (`ask_ahead`).
    asked_ahead: Vec<TrackId>,
    /// Album years, and whether the album's page calls it a single; the year `None` while the
    /// request runs or when the album has none.
    years: HashMap<AlbumId, (Option<String>, bool)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Once at startup: connect `NowPlayingData`'s callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<NowPlayingData>();
    let c = ctx.clone();
    data.on_tab_selected(move |tab| {
        if tab == 1 {
            show_related(&c);
        }
    });
    let c = ctx.clone();
    data.on_play_related(move |i| {
        if let Ok(start) = usize::try_from(i) {
            play_related(&c, start);
        }
    });
    let c = ctx.clone();
    data.on_more(move |x, y, top| show_menu(&c, x, y, top));
    let c = ctx.clone();
    data.on_cover_wide_changed(move || {
        // After the frame that asked: that's the stage's relayout (Up next coming or going),
        // the costliest frame of the slide, and a decode beside it slowed it by up to a
        // third (16 ms against 12 at 4K, the thread sharing its core); meanwhile the art
        // already in is drawn at the new size.
        let c = c.clone();
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            let url = STATE.with(|s| s.borrow().cover_url.clone());
            load_cover(&c, url);
        });
    });
    lyrics::init(ctx);
    backdrop::init(ctx);
}

/// The current song's related tracks, if they've been loaded (the Related tab or
/// `want_related` asks for them; this doesn't).
pub(crate) fn related_of(id: &TrackId) -> Option<Vec<Track>> {
    STATE.with(|s| s.borrow().related.get(id).cloned().flatten())
}

/// `f` on the current song's related tracks as `related_of` has them (none until loaded),
/// without copying them.
pub(crate) fn with_related<R>(id: &TrackId, f: impl FnOnce(&[Track]) -> R) -> R {
    STATE.with(|s| {
        f(s.borrow()
            .related
            .get(id)
            .and_then(Option::as_deref)
            .unwrap_or_default())
    })
}

/// Plays the current song's related tracks from `start`, as a radio of the song: the Related
/// tab's rows and Home's "More like this".
pub(crate) fn play_related(ctx: &Ctx, start: usize) {
    let Some(seed) = ctx.player().and_then(|s| s.current) else {
        return;
    };
    let Some(tracks) = related_of(&seed.id) else {
        return;
    };
    if start < tracks.len() {
        ctx.send(Action::Queue(QueueAction::PlayFrom {
            tracks,
            start,
            radio: true,
            from: PlayingFrom::named(tr!("{} radio", seed.display_title())),
        }));
    }
}

/// The size the page draws the cover at most, as it last said (`cover-wide`).
fn cover_px(ui: &AppWindow) -> u32 {
    if ui.global::<NowPlayingData>().get_cover_wide() {
        COVER_WIDE
    } else {
        COVER
    }
}

/// Loads the current track's cover at the size the page draws it; `url` is the one the page
/// shows (`State::cover_url`). Nothing is set if the track or the size has changed by the
/// time it's in, so a late load at the other size never replaces the right one.
fn load_cover(ctx: &Rc<Ctx>, url: String) {
    let Some(ui) = ctx.window() else { return };
    if url.is_empty() {
        return;
    }
    let px = cover_px(&ui);
    let c = ctx.clone();
    let wanted = url.clone();
    ctx.covers.load(&url, px, move |img| {
        let Some(ui) = c.window() else { return };
        let still = STATE.with(|s| s.borrow().cover_url == wanted) && cover_px(&ui) == px;
        if still {
            landing::art_in(&c, &wanted, img);
        }
    });
}

/// The colours of the cover at `url` are known (`follow`): set now, or on Now playing with
/// the cover's art, in one frame (`landing`). `with_art`: its art is coming.
pub fn land_colours(ctx: &Rc<Ctx>, url: &str, with_art: bool, colours: Colours) {
    landing::colours_in(ctx, url, with_art, colours);
}

/// A crossfade's next cover (`crate::turn`): its art at the page's size, loaded ahead so it's
/// in the cache when the song shows.
pub fn prefetch_cover(ctx: &Rc<Ctx>, url: &str) {
    if let Some(ui) = ctx.window() {
        ctx.covers.load(url, cover_px(&ui), |_| {});
    }
}

/// A crossfade's next field, made ahead (`backdrop::prepare`).
pub fn backdrop_prepare(
    ctx: &Rc<Ctx>,
    url: &str,
    small: [Vec<follow::colour::Rgb>; 2],
    cards: [follow::colour::Rgb; 2],
) {
    backdrop::prepare(ctx, url, small, cards);
}

/// A crossfade's turn of the field: its start, a frame and its end (`backdrop::turn_*`).
pub fn backdrop_turn_begin(ctx: &Rc<Ctx>, url: &str) {
    backdrop::turn_begin(ctx, url);
}

pub fn backdrop_turn_step(ctx: &Rc<Ctx>, amount: f32) {
    backdrop::turn_step(ctx, amount);
}

pub fn backdrop_turn_end(ctx: &Rc<Ctx>) {
    backdrop::turn_end(ctx);
}

/// The cover's field made for the page as it is now, without waiting for a run of sizes to
/// settle (`--snapshot`, the tests).
pub fn backdrop_settled(ctx: &Rc<Ctx>) {
    backdrop::settled(ctx);
}

/// Opens the song menu for the current track at `x`, `y` (window coordinates); upward, it
/// ends at `top`.
pub fn show_menu(ctx: &Ctx, x: f32, y: f32, top: f32) {
    let Some(ui) = ctx.window() else { return };
    let Some(track) = ctx.player().and_then(|s| s.current) else {
        return;
    };
    // A restored session's track wasn't loaded by any page.
    song_menu::remember(std::slice::from_ref(&track));
    let Some(mut row) = to_rows(std::slice::from_ref(&track)).pop() else {
        return;
    };
    row.cover = ui.global::<Player>().get_cover();
    ui.global::<SongMenu>().invoke_show(row, x, y, top);
}

/// The page is being shown.
pub fn open(ctx: &Rc<Ctx>) {
    if let Some(state) = ctx.player() {
        show(ctx, &state);
    }
}

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(ctx: &Rc<Ctx>, state: &PlayerState) {
    // The Lyrics tab, here or in the Up next panel.
    lyrics::player_changed(ctx);
    if ctx.current_page() == Page::NowPlaying {
        show(ctx, state);
    } else {
        side_changed(ctx, state);
    }
}

/// The frame's Up next panel shows the same tabs (`SideTabs`): while it's open on Related,
/// a new track gets its own related tracks. Only on a track change, so position ticks
/// don't rebuild the list.
fn side_changed(ctx: &Rc<Ctx>, state: &PlayerState) {
    let Some(ui) = ctx.window() else { return };
    let frame = ui.global::<Frame>();
    // In a narrow window the tabs are the Up next sheet's.
    let open = if frame.get_sheet() {
        frame.get_sheet_open()
    } else {
        frame.get_queue_open()
    };
    if !open || ui.global::<NowPlayingData>().get_tab() != 1 {
        return;
    }
    let id = state.current.as_ref().map(|t| &t.id);
    let changed = STATE.with(|s| s.borrow().side_track.as_ref() != id);
    if changed {
        STATE.with(|s| s.borrow_mut().side_track = id.cloned());
        show_related(ctx);
    }
}

/// Signed in or out.
pub fn account_changed(_ctx: &Rc<Ctx>, _signed_in: bool) {}

/// Fills the page for the current track, when it changed.
fn show(ctx: &Rc<Ctx>, state: &PlayerState) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<NowPlayingData>();
    let track = state.current.as_ref();
    let id = track.map(|t| &t.id);
    // The player's state comes with every position: the same track again copies nothing.
    let changed = STATE.with(|s| s.borrow().track.as_ref() != id);
    if !changed {
        return;
    }
    STATE.with(|s| s.borrow_mut().track = id.cloned());

    let url = track.and_then(|t| t.thumbnail.clone()).unwrap_or_default();
    if STATE.with(|s| s.borrow().cover_url != url) {
        STATE.with(|s| s.borrow_mut().cover_url = url.clone());
        // On the page the last song's art stays until the new one's lands with its colours
        // (`landing`); a song with no art has none to wait for.
        if url.is_empty() || !data.get_page_shown() {
            data.set_cover(Default::default());
        }
        // The wash comes from the cover's colour, which `follow` works out once per song
        // whatever page is shown (`set_cover_colour`).
        load_cover(ctx, url);
    }

    show_year(ctx, track);
    let frame = ui.global::<Frame>();
    // The side panel, or in a narrow window the Up next sheet in its place.
    let side = if frame.get_sheet() {
        frame.get_sheet_open()
    } else {
        frame.get_side_open()
    };
    if data.get_tab() == 1 && side {
        show_related(ctx);
    } else {
        // Shown when the tab is picked, or the hidden side panel shown again.
        data.set_related(Default::default());
    }
}

/// The playing song's cover colour is known (`follow`; `None`: no cover): its wash, for the
/// Discord preview's banner.
pub fn set_wash(ui: &AppWindow, washes: Option<follow::Washes>) {
    let data = ui.global::<NowPlayingData>();
    let (dark, light) = washes.map_or_else(Default::default, |w| (w.dark, w.light));
    data.set_wash_dark(dark);
    data.set_wash_light(light);
}

/// The playing song's cover field is known (`follow`; for `url`, or a plain grey one without
/// a cover): the page's bands, the floating card's and the bar's tint, the lyrics' inks on
/// the page (and the page's second level of text, `Theme.text2`), and the picture's source, whatever the Accent setting is. Set whatever page is
/// shown, so the page opens on the right colours; until the next field is known the last one
/// stays. On Now playing it's set with the cover's art, in one frame (`landing`).
pub fn set_field(ctx: &Rc<Ctx>, url: &str, fields: [follow::backdrop::Field; 2]) {
    let Some(ui) = ctx.window() else { return };
    let [dark, light] = fields;
    let bands = |f: &follow::backdrop::Field| {
        let [top, mid, low] = f.stops.map(follow::to_color);
        FieldBands { top, mid, low }
    };
    let data = ui.global::<NowPlayingData>();
    data.set_field_dark(bands(&dark));
    data.set_field_light(bands(&light));
    let theme = ui.global::<Theme>();
    theme.set_field_card_dark(follow::to_color(dark.ink.ground));
    theme.set_field_card_light(follow::to_color(light.ink.ground));
    let (dark_ink, light_ink) = (follow::lyric_ink(dark.ink), follow::lyric_ink(light.ink));
    theme.set_field_text2_dark(dark_ink.next);
    theme.set_field_text2_light(light_ink.next);
    let lyrics = ui.global::<LyricsData>();
    lyrics.set_field_dark(dark_ink);
    lyrics.set_field_light(light_ink);
    backdrop::field_in(ctx, url, [dark.small, light.small]);
}

/// The current album's year into `NowPlayingData.year`, and whether it's a single into
/// `NowPlayingData.single`, read once per album (Home's band shows them too).
pub(crate) fn show_year(ctx: &Rc<Ctx>, track: Option<&Track>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<NowPlayingData>();
    let Some(album) = track.and_then(|t| t.album.as_ref()).map(|a| a.id.clone()) else {
        data.set_year("".into());
        data.set_single(false);
        return;
    };
    let known = STATE.with(|s| s.borrow().years.get(&album).cloned());
    match known {
        Some((year, single)) => {
            data.set_year(year.unwrap_or_default().into());
            data.set_single(single);
        }
        None => {
            data.set_year("".into());
            data.set_single(false);
            STATE.with(|s| s.borrow_mut().years.insert(album.clone(), (None, false)));
            let id = album.clone();
            ctx.load(
                move |source| async move { source.album(&id).await },
                move |ctx, ui, result| {
                    let (year, single) = match result {
                        Ok(a) => (a.summary.year, a.summary.single),
                        Err(e) => {
                            log!("now playing: album {}: {e}", album.0);
                            (None, false)
                        }
                    };
                    STATE.with(|s| {
                        s.borrow_mut()
                            .years
                            .insert(album.clone(), (year.clone(), single))
                    });
                    let current = ctx
                        .player()
                        .and_then(|s| s.current)
                        .and_then(|t| t.album)
                        .map(|a| a.id);
                    if current.as_ref() == Some(&album) {
                        let data = ui.global::<NowPlayingData>();
                        data.set_year(year.unwrap_or_default().into());
                        data.set_single(single);
                    }
                },
            );
        }
    }
}

/// Home shows the playing song (`hero.rs`): with fewer than `SHORT_QUEUE` songs next and
/// something on screen to show them (`shown`: the band's queue column, or the Up next panel
/// or sheet), its related tracks are asked for ahead of the Related tab, through the tab's
/// own request and cache, so "More like this" fills without waiting for the tab to open.
pub(crate) fn want_related(ctx: &Rc<Ctx>, state: &PlayerState, shown: bool) {
    let Some(seed) = &state.current else { return };
    let queued = state.upcoming.len();
    if STATE.with(|s| ask_ahead(&mut s.borrow_mut(), &seed.id, queued, shown)) {
        fetch_related(ctx, seed.clone());
    }
}

/// Whether `seed`'s related tracks are asked for ahead of the tab, with `queued` songs next:
/// only for a short queue something on screen would show them in (`shown`), at most once a
/// song, and not when they're in or on their way. A failed request isn't made again (the
/// tab, opened, still asks).
fn ask_ahead(state: &mut State, seed: &TrackId, queued: usize, shown: bool) -> bool {
    let ask = shown
        && queued < SHORT_QUEUE
        && !state.related.contains_key(seed)
        && !state.asked_ahead.contains(seed);
    if ask {
        state.asked_ahead.push(seed.clone());
    }
    ask
}

/// Fills the Related tab for the current track, asking once per track.
fn show_related(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<NowPlayingData>();
    data.set_error("".into());
    let Some(seed) = ctx.player().and_then(|s| s.current) else {
        data.set_loading(false);
        data.set_related(Default::default());
        return;
    };
    let known = STATE.with(|s| s.borrow().related.get(&seed.id).cloned());
    match known {
        Some(Some(tracks)) => {
            data.set_loading(false);
            fill_related(ctx, &tracks);
        }
        // Already on its way.
        Some(None) => data.set_loading(true),
        None => {
            data.set_loading(true);
            data.set_related(Default::default());
            fetch_related(ctx, seed);
        }
    }
}

/// Asks for `seed`'s related tracks, into the cache; the Related tab shows them if it's open
/// on the song by then, and Home's band fills a short queue with them.
fn fetch_related(ctx: &Rc<Ctx>, seed: Track) {
    STATE.with(|s| s.borrow_mut().related.insert(seed.id.clone(), None));
    let id = seed.id.clone();
    ctx.load(
        move |source| async move { source.radio(&id).await },
        move |ctx, ui, result| {
            let tracks = match result {
                Ok(tracks) => tracks
                    .into_iter()
                    .filter(|t| t.id != seed.id)
                    .collect::<Vec<_>>(),
                Err(e) => {
                    log!("now playing: related for {}: {e}", seed.id.0);
                    // Asked again next time the tab is shown for this track, never
                    // again ahead of it (`asked_ahead`).
                    STATE.with(|s| s.borrow_mut().related.remove(&seed.id));
                    let current = ctx.player().and_then(|s| s.current).map(|t| t.id);
                    if current.as_ref() == Some(&seed.id) {
                        let data = ui.global::<NowPlayingData>();
                        data.set_loading(false);
                        data.set_error(tr!("Couldn't load related tracks").into());
                    }
                    return;
                }
            };
            STATE.with(|s| {
                s.borrow_mut()
                    .related
                    .insert(seed.id.clone(), Some(tracks.clone()))
            });
            // Home's band fills a short queue with them.
            crate::pages::home::related_changed(ctx);
            let current = ctx.player().and_then(|s| s.current).map(|t| t.id);
            let data = ui.global::<NowPlayingData>();
            if current.as_ref() == Some(&seed.id) && data.get_tab() == 1 {
                data.set_loading(false);
                fill_related(ctx, &tracks);
            }
        },
    );
}

fn fill_related(ctx: &Rc<Ctx>, tracks: &[Track]) {
    let Some(ui) = ctx.window() else { return };
    let urls = tracks.iter().map(|t| t.thumbnail.clone()).collect();
    let (rows, model) = model(to_rows(tracks));
    ui.global::<NowPlayingData>().set_related(model);
    crate::prepare::rows_changed(crate::ui::SongList::Related);
    ctx.covers
        .fill(&rows, urls, ROW_COVER, |row, img| row.cover = img);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Home asks for a song's related tracks ahead of the tab only under a short queue and
    /// with something on screen to show them, once a song, and never again after a failure.
    #[test]
    fn related_is_asked_ahead_once_a_song_and_only_for_a_short_queue() {
        let mut s = State::default();
        let (a, b) = (TrackId("a".into()), TrackId("b".into()));
        assert!(!ask_ahead(&mut s, &a, 3, true), "a long queue");
        assert!(ask_ahead(&mut s, &a, 2, true), "a short queue");
        // On its way (`fetch_related`), then in: nothing more.
        s.related.insert(a.clone(), None);
        assert!(!ask_ahead(&mut s, &a, 0, true), "on its way");
        s.related.insert(a.clone(), Some(Vec::new()));
        assert!(!ask_ahead(&mut s, &a, 0, true), "in");
        // Another song asks once and fails (`fetch_related` forgets it): no retry, even when
        // the song comes back.
        assert!(ask_ahead(&mut s, &b, 0, true));
        s.related.insert(b.clone(), None);
        s.related.remove(&b);
        assert!(!ask_ahead(&mut s, &b, 0, true), "asked again");

        // A narrow window with the panel shut: nothing shows them, so nothing is asked; the
        // panel opening (or the window widening) asks once.
        for seed in ["panel", "wider"].map(|id| TrackId(id.into())) {
            assert!(!ask_ahead(&mut s, &seed, 0, false), "nothing shows them");
            assert!(!ask_ahead(&mut s, &seed, 1, false), "nothing shows them");
            assert!(ask_ahead(&mut s, &seed, 0, true), "{} didn't ask", seed.0);
            assert!(!ask_ahead(&mut s, &seed, 0, true), "{} asked twice", seed.0);
        }
    }
}
