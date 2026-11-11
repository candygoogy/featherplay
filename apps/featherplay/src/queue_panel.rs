//! The Up next panel (ui/queue_panel.slint); the now-playing page's Up next
//! tab shows the same `QueueData`. Edits are applied to the list at once and sent to the
//! controller, whose state then confirms them; the panel's open state is saved in the
//! settings and restored at startup. Now playing's "Playing from" opens the queue's source
//! from here too.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use featherplay_core::model::TrackId;
use featherplay_store::{Paths, Settings};
use slint::{ComponentHandle, Image, Model, SharedString, VecModel};

use crate::app::Ctx;
use crate::controller::{Action, PlayerState, QueueAction};
use crate::router::Page;
use crate::ui::{Frame, NowPlayingData, QueueData, TrackRow};
use crate::ui_bridge::{model, shown, to_rows};

/// Row covers, in logical pixels.
const COVER: u32 = 40;

#[derive(Default)]
struct Shown {
    /// The upcoming ids shown, so an unchanged queue isn't rebuilt on every state change.
    ids: Vec<TrackId>,
    rows: Option<Rc<VecModel<TrackRow>>>,
}

thread_local! {
    static SHOWN: RefCell<Shown> = RefCell::new(Shown::default());
    /// The page that shows where the queue came from, if one does.
    static SOURCE: RefCell<Option<Page>> = const { RefCell::new(None) };
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<QueueData>();
    let c = ctx.clone();
    data.on_move(move |from, to| {
        let (Ok(from), Ok(to)) = (usize::try_from(from), usize::try_from(to)) else {
            return;
        };
        if edit(|ids, rows| {
            if from >= ids.len() || to >= ids.len() {
                return false;
            }
            let id = ids.remove(from);
            ids.insert(to, id);
            let row = rows.remove(from);
            rows.insert(to, row);
            true
        }) {
            c.send(Action::Queue(QueueAction::Move { from, to }));
        }
    });
    let c = ctx.clone();
    data.on_remove(move |i| {
        let Ok(i) = usize::try_from(i) else { return };
        if edit(|ids, rows| {
            if i >= ids.len() {
                return false;
            }
            ids.remove(i);
            rows.remove(i);
            true
        }) {
            c.send(Action::Queue(QueueAction::Remove(i)));
        }
    });
    let c = ctx.clone();
    data.on_clear(move || {
        edit(|ids, rows| {
            ids.clear();
            rows.clear();
            true
        });
        c.send(Action::Queue(QueueAction::Clear));
    });
    let c = ctx.clone();
    data.on_play(move |i| {
        if let Ok(i) = usize::try_from(i) {
            c.send(Action::Queue(QueueAction::SkipTo(i)));
        }
    });
    let c = ctx.clone();
    data.on_open_source(move || open_source(&c));
    let c = ctx.clone();
    ui.global::<Frame>().on_queue_toggled(move |open| {
        log!("queue panel {open}");
        c.send(Action::Queue(QueueAction::PanelOpen(open)));
    });
    let c = ctx.clone();
    ui.global::<Frame>().on_side_toggled(move |open| {
        log!("now playing side panel {open}");
        c.send(Action::Queue(QueueAction::SidePanelOpen(open)));
    });
}

/// "Playing from" was clicked: the source's page; for one without a page (a radio,
/// Autoplay), Up next, in Now playing's side panel there (shown if it was hidden) and in the
/// frame's panel elsewhere.
fn open_source(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    if let Some(page) = SOURCE.with(|s| s.borrow().clone()) {
        ctx.navigate(page);
        return;
    }
    ui.global::<NowPlayingData>().set_tab(0);
    let frame = ui.global::<Frame>();
    if ctx.current_page() == Page::NowPlaying {
        if !frame.get_side_open() {
            frame.set_side_open(true);
            frame.invoke_side_toggled(true);
        }
    } else if !frame.get_queue_open() {
        frame.set_queue_open(true);
        frame.invoke_queue_toggled(true);
    }
}

/// Opens the panel if it was open when the app last ran, and hides Now playing's side panel
/// if it was hidden. Not called for `--snapshot`.
pub fn restore(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let settings = Settings::load(&Paths::resolve());
    if settings.queue_open {
        ui.global::<Frame>().set_queue_open(true);
    }
    if !settings.nowplaying_side_open {
        ui.global::<Frame>().set_side_open(false);
    }
}

/// Applies `f` to the shown ids and rows; the autoplay divider moves with the rows above it.
fn edit(f: impl FnOnce(&mut Vec<TrackId>, &VecModel<TrackRow>) -> bool) -> bool {
    SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        let Some(rows) = s.rows.clone() else {
            return false;
        };
        f(&mut s.ids, &rows)
    })
}

pub fn update(ctx: &Rc<Ctx>, state: &PlayerState) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<QueueData>();
    let from = &state.queue.playing_from;
    data.set_playing_from(shown(&from.label));
    SOURCE.with(|s| *s.borrow_mut() = from.page.clone().map(Page::from));
    data.set_autoplay_at(
        state
            .queue
            .autoplay_at
            .and_then(|i| i32::try_from(i).ok())
            .unwrap_or(-1),
    );
    if let Some(t) = &state.current {
        let mut row = to_rows(std::slice::from_ref(t)).remove(0);
        row.playing = true;
        // Keep the cover already shown for the same track.
        let old = data.get_current();
        let load = if old.id == row.id {
            row.cover = old.cover;
            None
        } else {
            t.thumbnail.clone()
        };
        let id = row.id.clone();
        // Set before loading: a cover already in memory arrives at once and finds this row.
        data.set_current(row);
        if let Some(url) = load {
            let weak = ui.as_weak();
            ctx.covers.load(&url, COVER, move |img| {
                let Some(ui) = weak.upgrade() else { return };
                let data = ui.global::<QueueData>();
                let mut row: TrackRow = data.get_current();
                if row.id == id {
                    row.cover = img;
                    data.set_current(row);
                }
            });
        }
    }
    data.set_has_current(state.current.is_some());

    let ids: Vec<TrackId> = state.upcoming.iter().map(|t| t.id.clone()).collect();
    let (old_rows, same) = SHOWN.with(|s| {
        let s = s.borrow();
        (s.rows.clone(), s.ids == ids)
    });
    if same && old_rows.is_some() {
        return;
    }
    // Covers already shown are kept, so a reorder or a removal doesn't blink.
    let mut covers: HashMap<SharedString, Image> = HashMap::new();
    if let Some(rows) = &old_rows {
        for row in rows.iter() {
            covers.entry(row.id.clone()).or_insert(row.cover);
        }
    }
    let mut new_rows = to_rows(&state.upcoming);
    let mut missing = Vec::new();
    for (row, t) in new_rows.iter_mut().zip(&state.upcoming) {
        match covers.get(&row.id) {
            Some(img) if img.size().width > 0 => row.cover = img.clone(),
            _ => {
                if let Some(url) = &t.thumbnail {
                    missing.push((row.id.clone(), url.clone()));
                }
            }
        }
    }
    let (rows, rows_model) = model(new_rows);
    data.set_upcoming(rows_model);
    crate::prepare::rows_changed(crate::ui::SongList::UpNext);
    SHOWN.with(|s| {
        *s.borrow_mut() = Shown {
            ids,
            rows: Some(rows.clone()),
        }
    });
    for (id, url) in missing {
        let weak = Rc::downgrade(&rows);
        ctx.covers.load(&url, COVER, move |img| {
            // By id: the row may have moved since it was asked for.
            let Some(rows) = weak.upgrade() else { return };
            for i in 0..rows.row_count() {
                if let Some(mut row) = rows.row_data(i)
                    && row.id == id
                {
                    row.cover = img.clone();
                    rows.set_row_data(i, row);
                }
            }
        });
    }
}
