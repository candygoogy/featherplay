//! The Up next panel (ui/queue_panel.slint); the now-playing page's Up next
//! tab shows the same `QueueData`. Edits are applied to the list at once and sent to the
//! controller, whose state then confirms them; the panel's open state is saved in the
//! settings and restored at startup. Now playing's "Playing from" opens the queue's source
//! from here too.
//!
//! A new state edits the rows in place when only a few changed (the next song drops the first
//! row), and covers are asked for only for the rows on screen and a few either side
//! (`ShownRows`, through `prepare`): a queue of thousands decodes a screenful.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use featherplay_core::model::Track;
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};

use crate::app::Ctx;
use crate::controller::{Action, PlayerState, QueueAction};
use crate::router::Page;
use crate::ui::{Frame, NowPlayingData, QueueData, SongList, TrackRow};
use crate::ui_bridge::{shown, to_rows};

/// Row covers, in logical pixels.
const COVER: u32 = 40;
/// Rows either side of those on screen whose covers are asked for too: a scroll's next.
const AHEAD: usize = 10;
/// A change of more rows than this (gone and added, not counting songs added at the end)
/// builds the list again rather than editing it row by row.
const EDIT_MAX: usize = 32;

/// An upcoming row as shown.
struct Slot {
    id: SharedString,
    url: Option<String>,
    /// Its cover was asked for, or it shows one.
    asked: bool,
}

/// The upcoming rows shown. A cover finds its row through `at`, wherever the row has moved
/// since it was asked for.
#[derive(Default)]
struct Shown {
    slots: Vec<Slot>,
    /// Where each id's row is (one of them, for a song queued twice).
    at: HashMap<SharedString, usize>,
    rows: Option<Rc<VecModel<TrackRow>>>,
    /// The rows on screen: the frame's panel or the sheet (0), Now playing's side panel (1).
    on_screen: [Option<(usize, usize)>; 2],
}

impl Shown {
    /// Takes row `i` out.
    fn take(&mut self, rows: &VecModel<TrackRow>, i: usize) -> (Slot, TrackRow) {
        let row = rows.remove(i);
        let slot = self.slots.remove(i);
        if self.at.get(&slot.id) == Some(&i) {
            self.at.remove(&slot.id);
        }
        self.shift(i, false);
        (slot, row)
    }

    /// Puts `row` in at `i`.
    fn put(&mut self, rows: &VecModel<TrackRow>, i: usize, slot: Slot, row: TrackRow) {
        self.shift(i, true);
        self.at.insert(slot.id.clone(), i);
        self.slots.insert(i, slot);
        rows.insert(i, row);
    }

    /// Shows `row` in row `i`'s place.
    fn replace(&mut self, rows: &VecModel<TrackRow>, i: usize, slot: Slot, row: TrackRow) {
        if self.at.get(&self.slots[i].id) == Some(&i) {
            self.at.remove(&self.slots[i].id);
        }
        self.at.insert(slot.id.clone(), i);
        self.slots[i] = slot;
        rows.set_row_data(i, row);
    }

    /// The rows from `from` on move one place down (a row put in before them) or up (the
    /// row before them taken out).
    fn shift(&mut self, from: usize, down: bool) {
        // Nothing comes after a row added at the end.
        if from >= self.slots.len() {
            return;
        }
        for i in self.at.values_mut() {
            if *i >= from {
                *i = if down { *i + 1 } else { *i - 1 };
            }
        }
    }

    fn clear(&mut self, rows: &VecModel<TrackRow>) {
        self.slots.clear();
        self.at.clear();
        rows.clear();
    }

    /// Shows `upcoming` in `rows`. Unchanged, nothing changes. A few rows gone or added (the
    /// next song, Previous, Play next, Add to queue, songs added at the end) are edited in
    /// place, so the list keeps its rows and its scroll; anything more builds it again.
    /// Returns whether it changed.
    fn follow(&mut self, rows: &VecModel<TrackRow>, upcoming: &[Track]) -> bool {
        let same = |slot: &Slot, t: &Track| slot.id.as_str() == t.id.0.as_str();
        let (n, m) = (self.slots.len(), upcoming.len());
        let prefix = self
            .slots
            .iter()
            .zip(upcoming)
            .take_while(|(s, t)| same(s, t))
            .count();
        if prefix == n && n == m {
            return false;
        }
        let suffix = self
            .slots
            .iter()
            .rev()
            .zip(upcoming.iter().rev())
            .take(n.min(m) - prefix)
            .take_while(|(s, t)| same(s, t))
            .count();
        let (gone, added) = (n - prefix - suffix, m - prefix - suffix);
        let appended = gone == 0 && suffix == 0;
        if gone + added > EDIT_MAX && !appended {
            self.rebuild(rows, upcoming);
            return true;
        }
        let new = &upcoming[prefix..prefix + added];
        let mut new_rows = to_rows(new);
        let mut new_slots = Vec::with_capacity(new.len());
        // A song already in the list keeps the cover it showed there.
        for (row, t) in new_rows.iter_mut().zip(new) {
            let cover = self
                .at
                .get(&row.id)
                .filter(|&&j| self.slots[j].asked)
                .and_then(|&j| rows.row_data(j))
                .map(|r| r.cover)
                .filter(|c| c.size().width > 0);
            let asked = cover.is_some();
            if let Some(cover) = cover {
                row.cover = cover;
            }
            new_slots.push(Slot {
                id: row.id.clone(),
                url: t.thumbnail.clone(),
                asked,
            });
        }
        let replaced = gone.min(added);
        let mut new = new_rows.into_iter().zip(new_slots);
        for (k, (row, slot)) in new.by_ref().take(replaced).enumerate() {
            self.replace(rows, prefix + k, slot, row);
        }
        for _ in replaced..gone {
            self.take(rows, prefix + replaced);
        }
        for (k, (row, slot)) in new.enumerate() {
            self.put(rows, prefix + replaced + k, slot, row);
        }
        true
    }

    /// Builds the rows again for `upcoming`, keeping the covers already shown, so a shuffle
    /// or a new queue with songs from the last doesn't blink.
    fn rebuild(&mut self, rows: &VecModel<TrackRow>, upcoming: &[Track]) {
        let mut covers: HashMap<SharedString, Image> = HashMap::new();
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.asked
                && let Some(row) = rows.row_data(i)
                && row.cover.size().width > 0
            {
                covers.entry(row.id).or_insert(row.cover);
            }
        }
        let mut new_rows = to_rows(upcoming);
        self.slots = new_rows
            .iter_mut()
            .zip(upcoming)
            .map(|(row, t)| {
                let cover = covers.get(&row.id);
                if let Some(img) = cover {
                    row.cover = img.clone();
                }
                Slot {
                    id: row.id.clone(),
                    url: t.thumbnail.clone(),
                    asked: cover.is_some(),
                }
            })
            .collect();
        self.at = self
            .slots
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.clone(), i))
            .collect();
        rows.set_vec(new_rows);
    }

    /// The rows on screen, and `AHEAD` either side, whose covers weren't asked for yet:
    /// marked asked, with their urls. Now playing's side panel counts only on its page.
    fn wanted(&mut self, now_playing: bool) -> Vec<(usize, SharedString, String)> {
        let len = self.slots.len();
        let mut wanted = Vec::new();
        for (part, range) in self.on_screen.into_iter().enumerate() {
            let Some((first, last)) = range else { continue };
            if len == 0 || (part == 1 && !now_playing) {
                continue;
            }
            for i in first.saturating_sub(AHEAD)..=(last + AHEAD).min(len - 1) {
                let slot = &mut self.slots[i];
                if slot.asked {
                    continue;
                }
                slot.asked = true;
                if let Some(url) = &slot.url {
                    wanted.push((i, slot.id.clone(), url.clone()));
                }
            }
        }
        wanted
    }

    /// Where `id`'s row is now: row `i`, where it was asked for, if it's still there.
    fn find(&self, i: usize, id: &str) -> Option<usize> {
        if self.slots.get(i).is_some_and(|s| s.id == id) {
            return Some(i);
        }
        self.at.get(id).copied()
    }
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
        if edit(|s, rows| {
            if from >= s.slots.len() || to >= s.slots.len() {
                return false;
            }
            let (slot, row) = s.take(rows, from);
            s.put(rows, to, slot, row);
            true
        }) {
            c.send(Action::Queue(QueueAction::Move { from, to }));
            load_shown(&c);
        }
    });
    let c = ctx.clone();
    data.on_remove(move |i| {
        let Ok(i) = usize::try_from(i) else { return };
        if edit(|s, rows| {
            if i >= s.slots.len() {
                return false;
            }
            s.take(rows, i);
            true
        }) {
            c.send(Action::Queue(QueueAction::Remove(i)));
            // A row comes up from below the list's end.
            load_shown(&c);
        }
    });
    let c = ctx.clone();
    data.on_clear(move || {
        edit(|s, rows| {
            s.clear(rows);
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
    if let Some(page) = SOURCE.with(|s| s.borrow().clone()) {
        ctx.navigate(page);
        return;
    }
    show_up_next(ctx);
}

/// Shows Up next: in Now playing's side panel there (shown if it was hidden), in the frame's
/// panel elsewhere (Home's "Show all"), and in a window too narrow for either, in the sheet.
pub fn show_up_next(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    ui.global::<NowPlayingData>().set_tab(0);
    let frame = ui.global::<Frame>();
    if ui.get_sheet() {
        frame.set_sheet_open(true);
    } else if ctx.current_page() == Page::NowPlaying {
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
    let (queue_open, side_open) = crate::pages::settings::panels_at_start();
    if queue_open {
        ui.global::<Frame>().set_queue_open(true);
    }
    if !side_open {
        ui.global::<Frame>().set_side_open(false);
    }
}

/// Applies `f` to the shown rows; the autoplay divider moves with the rows above it.
fn edit(f: impl FnOnce(&mut Shown, &VecModel<TrackRow>) -> bool) -> bool {
    SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        let Some(rows) = s.rows.clone() else {
            return false;
        };
        f(&mut s, &rows)
    })
}

/// Up next in the frame's panel or the sheet (`part` 0) or in Now playing's side panel (1)
/// shows rows `first` to `last` (none when `last < first`): their covers are asked for, and
/// a few either side.
pub fn rows_shown(ctx: &Rc<Ctx>, part: i32, first: i32, last: i32) {
    let Some(part) = usize::try_from(part).ok().filter(|&p| p < 2) else {
        return;
    };
    let range = match (usize::try_from(first), usize::try_from(last)) {
        (Ok(first), Ok(last)) if first <= last => Some((first, last)),
        _ => None,
    };
    SHOWN.with(|s| s.borrow_mut().on_screen[part] = range);
    if range.is_some() {
        load_shown(ctx);
    }
}

/// Asks for the covers of the rows on screen (and `AHEAD` either side) not asked for yet.
fn load_shown(ctx: &Rc<Ctx>) {
    let now_playing = ctx.current_page() == Page::NowPlaying;
    let (wanted, rows) = SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        (s.wanted(now_playing), s.rows.as_ref().map(Rc::downgrade))
    });
    let Some(rows) = rows else { return };
    // Outside the borrow: a cover already in memory comes at once.
    for (i, id, url) in wanted {
        let rows = rows.clone();
        ctx.covers.load(&url, COVER, move |img| {
            let Some(rows) = rows.upgrade() else { return };
            let Some(i) = SHOWN.with(|s| s.borrow().find(i, &id)) else {
                return;
            };
            if let Some(mut row) = rows.row_data(i)
                && row.id == id
            {
                row.cover = img;
                rows.set_row_data(i, row);
            }
        });
    }
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

    let changed = SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        match s.rows.clone() {
            Some(rows) => s.follow(&rows, &state.upcoming),
            None => {
                let rows = Rc::new(VecModel::default());
                s.rebuild(&rows, &state.upcoming);
                data.set_upcoming(ModelRc::from(rows.clone()));
                s.rows = Some(rows);
                true
            }
        }
    });
    if changed {
        crate::prepare::rows_changed(SongList::UpNext);
        load_shown(ctx);
    }
}
