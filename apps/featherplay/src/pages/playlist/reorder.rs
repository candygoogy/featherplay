//! Picking and moving songs on one of the account's own playlists (`TrackList::editable`).
//! A click picks a row, Ctrl+click adds or drops one, Shift+click picks the range from the
//! row clicked last; Esc lets them go, and so does leaving the page or loading it again.
//! Ctrl+A picks them all, and a click on empty space lets them go. Alt+Up and Alt+Down move
//! the picked rows a place, Alt+Home and Alt+End to the top or the end, together and in
//! their order (`key_move`); a drag moves them where it drops them (`drop_picked`).
//!
//! Every move goes through `move_rows`: the list changes at once and the write follows,
//! one at a time in the order they were made. A failed one puts the list back with an
//! error toast, or, with moves made after it, reads the playlist again.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use featherplay_controller::playlist_edit::Moved;
use featherplay_core::model::PlaylistId;
use featherplay_core::tr;
use featherplay_ytm::ApiError;
use featherplay_ytm::convert::bare_playlist_id;
use slint::{ComponentHandle, Model};

use super::{OPENED, with_list};
use crate::app::Ctx;
use crate::pages::ahead;
use crate::pages::content::TrackList;
use crate::router::Page;
use crate::ui::{PlaylistData, ReorderKey};

thread_local! {
    /// Where a Shift+click's range starts: the row clicked last.
    static ANCHOR: Cell<Option<usize>> = const { Cell::new(None) };
    static WRITES: RefCell<Writes> = RefCell::default();
}

/// The moves made and not yet written, sent one at a time.
#[derive(Default)]
struct Writes {
    busy: bool,
    queue: VecDeque<Write>,
}

struct Write {
    id: PlaylistId,
    /// The `open` it was made on (`OPENED`): only that list is put back.
    opened: u64,
    moved: Moved,
}

/// A playlist page opens: nothing is picked.
pub fn opened() {
    ANCHOR.set(None);
}

/// Which rows are picked after a click on row `i` of `picked`, with Ctrl or Shift held, and
/// where a later Shift+click's range starts. Ctrl+Shift adds the range to the rows picked.
pub fn clicked(
    picked: &[bool],
    anchor: Option<usize>,
    i: usize,
    ctrl: bool,
    shift: bool,
) -> (Vec<bool>, Option<usize>) {
    let n = picked.len();
    if i >= n {
        return (picked.to_vec(), anchor);
    }
    match anchor.filter(|&a| shift && a < n) {
        Some(a) => {
            let range = a.min(i)..=a.max(i);
            let now = (0..n)
                .map(|k| range.contains(&k) || (ctrl && picked[k]))
                .collect();
            (now, Some(a))
        }
        None if ctrl => {
            let mut now = picked.to_vec();
            now[i] = !now[i];
            (now, Some(i))
        }
        None => ((0..n).map(|k| k == i).collect(), Some(i)),
    }
}

/// Where a key moves the rows at `picked` (sorted, not empty) to: `move_rows`'s `before`. A
/// place up is before the row above the first; a place down is after the row below the
/// last. Rows picked apart gather there.
pub fn key_target(picked: &[usize], key: ReorderKey) -> Option<usize> {
    let (first, last) = (picked[0], picked[picked.len() - 1]);
    match key {
        ReorderKey::Up => Some(first.saturating_sub(1)),
        ReorderKey::Down => Some(last + 2),
        ReorderKey::Home => Some(0),
        ReorderKey::End => None,
    }
}

/// The places of the rows picked.
fn picked_rows(l: &TrackList) -> Vec<usize> {
    (0..l.rows.row_count())
        .filter(|&i| l.rows.row_data(i).is_some_and(|r| r.selected))
        .collect()
}

/// Shows `now` as the rows picked; only the rows that change are set.
fn show_picked(l: &TrackList, now: impl Fn(usize) -> bool) {
    for i in 0..l.rows.row_count() {
        if let Some(mut row) = l.rows.row_data(i)
            && row.selected != now(i)
        {
            row.selected = !row.selected;
            l.rows.set_row_data(i, row);
        }
    }
}

/// Tells the page how many rows are picked, the first and the last (the drag's line and
/// badge read them).
fn tell(ctx: &Ctx) {
    let (Some(ui), Some(picked)) = (ctx.window(), with_list(|l| picked_rows(l))) else {
        return;
    };
    let data = ui.global::<PlaylistData>();
    let at = |i: Option<&usize>| i.map_or(-1, |&i| i as i32);
    data.set_picked_count(picked.len() as i32);
    data.set_picked_first(at(picked.first()));
    data.set_picked_last(at(picked.last()));
}

/// Row `i` clicked, with Ctrl or Shift held or not.
pub fn pick(ctx: &Ctx, i: usize, ctrl: bool, shift: bool) {
    with_list(|l| {
        if !l.editable {
            return;
        }
        let picked: Vec<bool> = l.rows.iter().map(|r| r.selected).collect();
        let (now, anchor) = clicked(&picked, ANCHOR.get(), i, ctrl, shift);
        ANCHOR.set(anchor);
        show_picked(l, |k| now[k]);
    });
    tell(ctx);
}

/// Ctrl+A: every row picked.
pub fn pick_all(ctx: &Ctx) {
    with_list(|l| {
        if l.editable {
            show_picked(l, |_| true);
        }
    });
    tell(ctx);
}

/// Esc, or a click on empty space: lets the picked rows go. Whether there were any.
pub fn unpick(ctx: &Ctx) -> bool {
    ANCHOR.set(None);
    let any = with_list(|l| {
        let any = l.rows.iter().any(|r| r.selected);
        show_picked(l, |_| false);
        any
    })
    .unwrap_or(false);
    tell(ctx);
    any
}

/// A drag starts on row `i`: a picked row takes the rows picked with it; any other is
/// picked alone, as a click would.
pub fn drag_start(ctx: &Ctx, i: usize) {
    let picked = with_list(|l| l.rows.row_data(i).is_some_and(|r| r.selected));
    if picked == Some(false) {
        pick(ctx, i, false, false);
    }
}

/// The rows dragged are dropped before row `slot`, or past the last at the end.
pub fn drop_picked(ctx: &Rc<Ctx>, slot: usize) {
    let Some((picked, n)) = with_list(|l| (picked_rows(l), l.tracks.len())) else {
        return;
    };
    if !picked.is_empty() {
        move_rows(ctx, &picked, (slot < n).then_some(slot));
    }
}

/// Alt+Up, Alt+Down, Alt+Home or Alt+End: the picked rows move, and the list scrolls to
/// show them.
pub fn key_move(ctx: &Rc<Ctx>, key: ReorderKey) {
    let picked = with_list(|l| picked_rows(l)).unwrap_or_default();
    if picked.is_empty() || !move_rows(ctx, &picked, key_target(&picked, key)) {
        return;
    }
    let Some(now) = with_list(|l| picked_rows(l)) else {
        return;
    };
    if let (Some(&first), Some(&last), Some(ui)) = (now.first(), now.last(), ctx.window()) {
        let data = ui.global::<PlaylistData>();
        data.set_reveal_first(first as i32);
        data.set_reveal_last(last as i32);
        data.set_revealed(data.get_revealed().wrapping_add(1));
    }
}

/// Moves the rows at `picked` of the playlist shown together, in their order, to right
/// before the row at `before`, or to the end with `None` (`playlist_edit::move_rows`):
/// the one way rows move, by key or by drag. The list changes at once; the write goes after
/// the ones before it. Returns whether anything moved.
pub fn move_rows(ctx: &Rc<Ctx>, picked: &[usize], before: Option<usize>) -> bool {
    let Some(ui) = ctx.window() else {
        return false;
    };
    let id = PlaylistId(ui.global::<PlaylistData>().get_id().to_string());
    let moved = with_list(|l| {
        // While later pages still arrive, the end shown isn't the playlist's: nothing goes
        // there yet.
        let n = l.tracks.len();
        let to_end = before.is_none_or(|b| (b..n).all(|k| picked.contains(&k)));
        if !l.editable || (l.arriving.is_some() && to_end) {
            return None;
        }
        let moved = l.move_rows(picked, before)?;
        ANCHOR.set(picked_rows(l).first().copied());
        Some(moved)
    })
    .flatten();
    let Some(moved) = moved else {
        return false;
    };
    tell(ctx);
    // Back to the page shows it as it is now, read again.
    ahead::forget_playlist(&id);
    let opened = OPENED.with(Cell::get);
    WRITES.with_borrow_mut(|w| w.queue.push_back(Write { id, opened, moved }));
    write_next(ctx);
    true
}

/// Sends the oldest move not written yet, unless one is on its way.
fn write_next(ctx: &Rc<Ctx>) {
    let next = WRITES.with_borrow_mut(|w| {
        if w.busy {
            return None;
        }
        let next = w.queue.pop_front();
        w.busy = next.is_some();
        next
    });
    let Some(write) = next else { return };
    let (id, moves) = (write.id.clone(), write.moved.moves.clone());
    ctx.load(
        move |source| async move { source.move_in_playlist(&id, &moves).await },
        move |ctx, _, result| {
            let later = WRITES.with_borrow_mut(|w| {
                w.busy = false;
                // The moves after a failed one were made on a list the service doesn't
                // have: they're dropped, and the list is read again.
                let later = result.is_err() && !w.queue.is_empty();
                if result.is_err() {
                    w.queue.clear();
                }
                later
            });
            if let Err(e) = result {
                failed(ctx, &write, later, &e);
            }
            write_next(ctx);
        },
    );
}

/// A move's write failed: the list shown goes back as the service has it, and a toast says
/// so.
fn failed(ctx: &Rc<Ctx>, write: &Write, later: bool, e: &ApiError) {
    let shown = matches!(ctx.current_page(), Page::Playlist { id }
        if bare_playlist_id(&id.0) == bare_playlist_id(&write.id.0));
    if shown && OPENED.with(Cell::get) == write.opened {
        let back = !later && with_list(|l| l.put_back(&write.moved)).unwrap_or(false);
        if back {
            tell(ctx);
        } else {
            ctx.show_current();
        }
    }
    ctx.toast_error(&crate::playlist_menu::failure_text(
        tr!("Couldn't reorder the playlist."),
        e,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(picked: &[usize], n: usize) -> Vec<bool> {
        (0..n).map(|i| picked.contains(&i)).collect()
    }

    #[test]
    fn a_click_picks_one_ctrl_adds_or_drops_and_shift_picks_a_range() {
        let none = rows(&[], 6);
        assert_eq!(
            clicked(&none, None, 2, false, false),
            (rows(&[2], 6), Some(2))
        );
        let (now, anchor) = clicked(&rows(&[2], 6), Some(2), 4, true, false);
        assert_eq!((now, anchor), (rows(&[2, 4], 6), Some(4)));
        let (now, _) = clicked(&rows(&[2, 4], 6), Some(4), 2, true, false);
        assert_eq!(now, rows(&[4], 6), "Ctrl+click drops a picked row");
        // Shift: from the row clicked last, in either direction; it stays the start.
        assert_eq!(
            clicked(&rows(&[4], 6), Some(4), 1, false, true),
            (rows(&[1, 2, 3, 4], 6), Some(4))
        );
        assert_eq!(
            clicked(&rows(&[0], 6), Some(4), 5, true, true).0,
            rows(&[0, 4, 5], 6),
            "Ctrl+Shift adds the range"
        );
        // Shift with nothing clicked yet picks the one row; a row past the end changes nothing.
        assert_eq!(clicked(&none, None, 3, false, true).0, rows(&[3], 6));
        assert_eq!(
            clicked(&rows(&[1], 6), Some(1), 9, false, false).0,
            rows(&[1], 6)
        );
    }

    #[test]
    fn keys_move_the_picked_rows_a_place_or_to_either_end() {
        assert_eq!(key_target(&[2, 3], ReorderKey::Up), Some(1));
        assert_eq!(key_target(&[0, 3], ReorderKey::Up), Some(0));
        assert_eq!(key_target(&[2, 3], ReorderKey::Down), Some(5));
        assert_eq!(key_target(&[2, 3], ReorderKey::Home), Some(0));
        assert_eq!(key_target(&[2, 3], ReorderKey::End), None);
    }
}
