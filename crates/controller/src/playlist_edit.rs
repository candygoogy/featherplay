//! Moving songs within one of the account's own playlists (`Playlist::editable`), one or
//! several at once, as Up next's reorder works: the page's list changes at once
//! (`move_rows`), the moves go out in one write (`Source::move_in_playlist`), and if the
//! write fails the list goes back as it was (`Moved::put_back`).
//!
//! A move names entries, not songs (`EntryId`: a list may have a song twice): each picked
//! song goes right before the row it's dropped on, or to the end of the list with no
//! successor. Several songs are one move each, in the list's order, all before the same
//! row, so they arrive together and in their order whatever was between them.

use featherplay_ytm::{EntryId, PlaylistMove};

/// A move made on the page's list: what to send, and how to put the list back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    /// One for each song moved, in the list's order (`Source::move_in_playlist`).
    pub moves: Vec<PlaylistMove>,
    /// The list after the move as the rows' places before it: row `i` came from `order[i]`.
    order: Vec<usize>,
}

impl Moved {
    /// Puts the list back as it was before the move, after the write failed. Rows added to
    /// its end since (a later page) stay where they are. `false`, changing nothing, for
    /// lists shorter than the moved one (it was loaded again meanwhile).
    pub fn put_back<T>(&self, rows: &mut Vec<T>, entries: &mut Vec<Option<EntryId>>) -> bool {
        let n = self.order.len();
        if rows.len() < n || entries.len() < n {
            return false;
        }
        let mut back = vec![0; n];
        for (now, &was) in self.order.iter().enumerate() {
            back[was] = now;
        }
        reorder(rows, &back);
        reorder(entries, &back);
        true
    }
}

/// Moves the rows at `picked` (places in the list, in any order) together, in the list's
/// order, to right before the row at `before`, or to the end of the list with `None` (or
/// a place past it). A `before` that's picked itself means the first row after it that
/// isn't. `rows` and `entries` are the page's songs and their entries
/// (`Playlist::entries`), changed alike.
///
/// `None`, changing nothing, when there's nothing to send: no rows picked, a place out of
/// range, lists of different lengths, a picked row or the one it goes before without an
/// entry, or the rows already where they would go.
pub fn move_rows<T>(
    rows: &mut Vec<T>,
    entries: &mut Vec<Option<EntryId>>,
    picked: &[usize],
    before: Option<usize>,
) -> Option<Moved> {
    let n = rows.len();
    if entries.len() != n {
        return None;
    }
    let mut picked = picked.to_vec();
    picked.sort_unstable();
    picked.dedup();
    if picked.is_empty() || picked.last().is_some_and(|&last| last >= n) {
        return None;
    }
    let is_picked = |i: usize| picked.binary_search(&i).is_ok();
    // The first row at or after `before` that stays put; past the end is the end.
    let successor = before.and_then(|b| (b..n).find(|&i| !is_picked(i)));
    let rest: Vec<usize> = (0..n).filter(|&i| !is_picked(i)).collect();
    let at = successor.map_or(rest.len(), |s| {
        rest.iter().position(|&i| i == s).unwrap_or(rest.len())
    });
    let order: Vec<usize> = rest[..at]
        .iter()
        .chain(&picked)
        .chain(&rest[at..])
        .copied()
        .collect();
    if order.iter().enumerate().all(|(i, &was)| i == was) {
        return None;
    }
    let successor = match successor {
        Some(s) => Some(entries[s].clone()?),
        None => None,
    };
    let moves = picked
        .iter()
        .map(|&i| {
            Some(PlaylistMove {
                entry: entries[i].clone()?,
                before: successor.clone(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    reorder(rows, &order);
    reorder(entries, &order);
    Some(Moved { moves, order })
}

/// Reorders the first `order.len()` items of `list` so that item `i` is the one that was
/// at `order[i]`; the rest stay. `order` is a permutation of `0..order.len()`.
fn reorder<T>(list: &mut Vec<T>, order: &[usize]) {
    let rest = list.split_off(order.len());
    let mut old: Vec<Option<T>> = std::mem::take(list).into_iter().map(Some).collect();
    list.extend(order.iter().filter_map(|&was| old[was].take()));
    list.extend(rest);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(n: usize) -> Vec<Option<EntryId>> {
        (0..n).map(|i| Some(EntryId(format!("e{i}")))).collect()
    }

    fn mv(entry: usize, before: Option<usize>) -> PlaylistMove {
        PlaylistMove {
            entry: EntryId(format!("e{entry}")),
            before: before.map(|b| EntryId(format!("e{b}"))),
        }
    }

    #[test]
    fn one_song_goes_before_the_row_it_is_dropped_on_or_to_the_end() {
        let mut rows = vec!['a', 'b', 'c', 'd'];
        let mut ids = entries(4);
        let moved = move_rows(&mut rows, &mut ids, &[3], Some(1)).expect("moved");
        assert_eq!(rows, ['a', 'd', 'b', 'c']);
        assert_eq!(moved.moves, [mv(3, Some(1))]);
        assert_eq!(ids[1], Some(EntryId("e3".into())), "the entries move along");

        let mut rows = vec!['a', 'b', 'c'];
        let mut ids = entries(3);
        let moved = move_rows(&mut rows, &mut ids, &[0], None).expect("to the end");
        assert_eq!(rows, ['b', 'c', 'a']);
        assert_eq!(moved.moves, [mv(0, None)], "the end names no successor");
    }

    #[test]
    fn several_songs_keep_their_order_and_go_together() {
        let mut rows = vec!['a', 'b', 'c', 'd', 'e', 'f'];
        let mut ids = entries(6);
        // Picked in any order, dropped on a picked row: before the next one that isn't.
        let moved = move_rows(&mut rows, &mut ids, &[4, 0, 2], Some(2)).expect("moved");
        assert_eq!(rows, ['b', 'a', 'c', 'e', 'd', 'f']);
        assert_eq!(
            moved.moves,
            [mv(0, Some(3)), mv(2, Some(3)), mv(4, Some(3))]
        );
    }

    #[test]
    fn a_move_that_changes_nothing_or_cant_be_named_sends_nothing() {
        let mut rows = vec!['a', 'b', 'c', 'd'];
        let mut ids = entries(4);
        let before = rows.clone();
        assert_eq!(move_rows(&mut rows, &mut ids, &[1, 2], Some(3)), None);
        assert_eq!(move_rows(&mut rows, &mut ids, &[3], None), None);
        assert_eq!(move_rows(&mut rows, &mut ids, &[], Some(0)), None);
        assert_eq!(move_rows(&mut rows, &mut ids, &[4], Some(0)), None);
        ids[3] = None;
        assert_eq!(move_rows(&mut rows, &mut ids, &[3], Some(0)), None);
        assert_eq!(move_rows(&mut rows, &mut ids, &[0], Some(3)), None);
        assert_eq!(move_rows(&mut rows, &mut entries(3), &[0], None), None);
        assert_eq!(rows, before);
    }

    #[test]
    fn a_failed_move_puts_the_list_back_and_keeps_rows_added_since() {
        let mut rows = vec!['a', 'b', 'c', 'd', 'e'];
        let mut ids = entries(5);
        let (rows_before, ids_before) = (rows.clone(), ids.clone());
        let moved = move_rows(&mut rows, &mut ids, &[1, 3], None).expect("moved");
        assert_eq!(rows, ['a', 'c', 'e', 'b', 'd']);
        rows.push('f');
        ids.push(Some(EntryId("e5".into())));
        assert!(moved.put_back(&mut rows, &mut ids));
        assert_eq!(rows[..5], rows_before[..]);
        assert_eq!(ids[..5], ids_before[..]);
        assert_eq!(rows[5], 'f');
        assert!(
            !moved.put_back(&mut vec!['a'], &mut entries(1)),
            "a list loaded again"
        );
    }
}
