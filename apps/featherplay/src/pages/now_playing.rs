//! Now playing page (ui/pages/now_playing.slint). The track itself comes
//! from the `Player` global (`bar.rs`) and the Up next tab from `QueueData`
//! (`queue_panel.rs`); this module fills the large cover, the album's year and the Related
//! tab, and opens the song menu for the current track.
//!
//! Requests stay small: Related is the current track's radio, fetched once per track and
//! only when the tab is shown; the year comes from the album, fetched once per album while
//! the page is open. Results are kept, so reopening the page asks for nothing again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use featherplay_core::model::{AlbumId, Track, TrackId};
use featherplay_core::tr;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::{Action, PlayerState, QueueAction};
use crate::router::Page;
use crate::song_menu;
use crate::ui::{AppWindow, Frame, NowPlayingData, Player, SongMenu};
use crate::ui_bridge::{model, to_rows};
use crate::{follow, lyrics};

/// The cover's largest size beside the side panel, in logical pixels (`Theme.nowplaying-cover`).
const COVER: u32 = 440;
/// Its largest with the side panel hidden (`Theme.nowplaying-cover-wide`), decoded while the
/// page draws it larger than `COVER` (`NowPlayingData.cover-wide`).
const COVER_WIDE: u32 = 560;
/// Related row covers.
const ROW_COVER: u32 = 40;

#[derive(Default)]
struct State {
    /// The track the page shows.
    track: Option<TrackId>,
    /// The track the frame's Up next panel last showed Related for.
    side_track: Option<TrackId>,
    cover_url: String,
    /// Related tracks by seed; `None` while the request runs.
    related: HashMap<TrackId, Option<Vec<Track>>>,
    /// Album years; `None` while the request runs or when the album has none.
    years: HashMap<AlbumId, Option<String>>,
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
        let Some(seed) = c.player().and_then(|s| s.current) else {
            return;
        };
        let Some(tracks) = STATE.with(|s| s.borrow().related.get(&seed.id).cloned().flatten())
        else {
            return;
        };
        let Ok(start) = usize::try_from(i) else {
            return;
        };
        if start < tracks.len() {
            c.send(Action::Queue(QueueAction::PlayFrom {
                tracks,
                start,
                radio: true,
                from: format!("{} radio", seed.title),
            }));
        }
    });
    let c = ctx.clone();
    data.on_more(move |x, y, top| show_menu(&c, x, y, top));
    let c = ctx.clone();
    data.on_cover_wide_changed(move || {
        let url = STATE.with(|s| s.borrow().cover_url.clone());
        load_cover(&c, url);
    });
    lyrics::init(ctx);
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
fn load_cover(ctx: &Ctx, url: String) {
    let Some(ui) = ctx.window() else { return };
    if url.is_empty() {
        return;
    }
    let px = cover_px(&ui);
    let weak = ui.as_weak();
    let wanted = url.clone();
    ctx.covers.load(&url, px, move |img| {
        let Some(ui) = weak.upgrade() else { return };
        let still = STATE.with(|s| s.borrow().cover_url == wanted) && cover_px(&ui) == px;
        if still {
            ui.global::<NowPlayingData>().set_cover(img);
        }
    });
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
    if !ui.global::<Frame>().get_queue_open() || ui.global::<NowPlayingData>().get_tab() != 1 {
        return;
    }
    let id = state.current.as_ref().map(|t| t.id.clone());
    let changed = STATE.with(|s| s.borrow().side_track != id);
    if changed {
        STATE.with(|s| s.borrow_mut().side_track = id);
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
    let id = track.map(|t| t.id.clone());
    let changed = STATE.with(|s| s.borrow().track != id);
    if !changed {
        return;
    }
    STATE.with(|s| s.borrow_mut().track = id);

    let url = track.and_then(|t| t.thumbnail.clone()).unwrap_or_default();
    if STATE.with(|s| s.borrow().cover_url != url) {
        STATE.with(|s| s.borrow_mut().cover_url = url.clone());
        data.set_cover(Default::default());
        // The wash comes from the cover's colour, which `follow` works out once per song
        // whatever page is shown (`set_cover_colour`).
        load_cover(ctx, url);
    }

    show_year(ctx, track);
    if data.get_tab() == 1 && ui.global::<Frame>().get_side_open() {
        show_related(ctx);
    } else {
        // Shown when the tab is picked, or the hidden side panel shown again.
        data.set_related(Default::default());
    }
}

/// The playing song's cover colour is known (`follow`; `None`: no cover): the stage's
/// ambient wash, whatever the Accent setting is. Set whatever page is shown, so the page
/// opens on the right colour; until the next colour is known the last one stays, and the
/// stage crossfades from one to the other.
pub fn set_cover_colour(ui: &AppWindow, washes: Option<follow::Washes>) {
    let data = ui.global::<NowPlayingData>();
    let (dark, light) = washes.map_or_else(Default::default, |w| (w.dark, w.light));
    data.set_wash_dark(dark);
    data.set_wash_light(light);
}

fn show_year(ctx: &Rc<Ctx>, track: Option<&Track>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<NowPlayingData>();
    let Some(album) = track.and_then(|t| t.album.as_ref()).map(|a| a.id.clone()) else {
        data.set_year("".into());
        return;
    };
    let known = STATE.with(|s| s.borrow().years.get(&album).cloned());
    match known {
        Some(year) => data.set_year(year.unwrap_or_default().into()),
        None => {
            data.set_year("".into());
            STATE.with(|s| s.borrow_mut().years.insert(album.clone(), None));
            let id = album.clone();
            ctx.load(
                move |source| async move { source.album(&id).await },
                move |ctx, ui, result| {
                    let year = match result {
                        Ok(a) => a.summary.year,
                        Err(e) => {
                            log!("now playing: album {}: {e}", album.0);
                            None
                        }
                    };
                    STATE.with(|s| s.borrow_mut().years.insert(album.clone(), year.clone()));
                    let current = ctx
                        .player()
                        .and_then(|s| s.current)
                        .and_then(|t| t.album)
                        .map(|a| a.id);
                    if current.as_ref() == Some(&album) {
                        ui.global::<NowPlayingData>()
                            .set_year(year.unwrap_or_default().into());
                    }
                },
            );
        }
    }
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
                            // Asked again next time the tab is shown for this track.
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
                    let current = ctx.player().and_then(|s| s.current).map(|t| t.id);
                    let data = ui.global::<NowPlayingData>();
                    if current.as_ref() == Some(&seed.id) && data.get_tab() == 1 {
                        data.set_loading(false);
                        fill_related(ctx, &tracks);
                    }
                },
            );
        }
    }
}

fn fill_related(ctx: &Rc<Ctx>, tracks: &[Track]) {
    let Some(ui) = ctx.window() else { return };
    let urls = tracks.iter().map(|t| t.thumbnail.clone()).collect();
    let (rows, model) = model(to_rows(tracks));
    ui.global::<NowPlayingData>().set_related(model);
    ctx.covers
        .fill(&rows, urls, ROW_COVER, |row, img| row.cover = img);
}
