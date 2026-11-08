//! A card grid's lines (`CardGrid`, ui/pages/content.slint): the page's flat model of cards
//! cut into lines of as many cards as fit, each line a model of its own cards.
//!
//! The grid's `ListView` makes a line at a time. Its cards used to read their item out of
//! the page's whole model by index, and Slint tracks a model read that way as one
//! dependency: any card set (a cover handed over) dirtied every card on screen, so each of a
//! tab's ~20 covers repainted the grid and drew every card's cover layer again (Library's
//! Playlists missed ~10 frames on its first draw, 2026-11-08). Here a line's cards are a
//! repeater's own data, which Slint sets only when it changed: a card set reaches its card.

use std::cell::RefCell;
use std::collections::HashMap;
use std::pin::Pin;
use std::rc::{Rc, Weak};

use i_slint_core::model::{ModelChangeListener, ModelChangeListenerBox};
use slint::{ComponentHandle, Model, ModelNotify, ModelRc, ModelTracker};

use crate::app::Ctx;
use crate::ui::{CardItem, CardLine, Grid};

/// Once at startup: answer `Grid.lines`.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    ui.global::<Grid>()
        .on_lines(|cards, per_row| lines(cards, per_row.max(1) as usize));
}

/// `cards` as lines of `per_row`.
pub fn lines(cards: ModelRc<CardItem>, per_row: usize) -> ModelRc<CardLine> {
    let inner = ModelChangeListenerBox::new(Lines {
        cards: cards.clone(),
        per_row: per_row.max(1),
        notify: ModelNotify::default(),
        made: RefCell::new(HashMap::new()),
    });
    cards
        .model_tracker()
        .attach_peer(inner.as_ref().model_peer());
    ModelRc::new(CardLines(inner))
}

struct Lines {
    cards: ModelRc<CardItem>,
    per_row: usize,
    notify: ModelNotify,
    /// The lines handed out and still alive, by index: a card set goes to its line's model,
    /// and the same line asked for again is the same model (so its cards aren't made again).
    made: RefCell<HashMap<usize, Weak<Line>>>,
}

impl Lines {
    fn count(&self) -> usize {
        self.cards.row_count().div_ceil(self.per_row)
    }

    fn line(&self, i: usize) -> Option<Rc<Line>> {
        if i >= self.count() {
            return None;
        }
        let mut made = self.made.borrow_mut();
        if let Some(line) = made.get(&i).and_then(Weak::upgrade) {
            return Some(line);
        }
        made.retain(|_, l| l.strong_count() > 0);
        let start = i * self.per_row;
        let line = Rc::new(Line {
            cards: self.cards.clone(),
            start,
            len: self.per_row.min(self.cards.row_count() - start),
            notify: ModelNotify::default(),
        });
        made.insert(i, Rc::downgrade(&line));
        Some(line)
    }

    /// Cards came or went: every line may have moved, so the grid makes its lines again.
    fn renew(&self) {
        self.made.borrow_mut().clear();
        self.notify.reset();
    }
}

impl ModelChangeListener for Lines {
    fn row_changed(self: Pin<&Self>, row: usize) {
        let line = self
            .made
            .borrow()
            .get(&(row / self.per_row))
            .and_then(Weak::upgrade);
        if let Some(line) = line {
            line.notify.row_changed(row - line.start);
        }
    }

    fn row_added(self: Pin<&Self>, _index: usize, _count: usize) {
        self.renew();
    }

    fn row_removed(self: Pin<&Self>, _index: usize, _count: usize) {
        self.renew();
    }

    fn reset(self: Pin<&Self>) {
        self.renew();
    }
}

struct CardLines(ModelChangeListenerBox<Lines>);

impl Model for CardLines {
    type Data = CardLine;

    fn row_count(&self) -> usize {
        self.0.count()
    }

    fn row_data(&self, row: usize) -> Option<CardLine> {
        let line = self.0.line(row)?;
        Some(CardLine {
            first: line.start as i32,
            cards: ModelRc::from(line),
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.0.notify
    }
}

/// One line: cards `start..start + len` of the page's model.
struct Line {
    cards: ModelRc<CardItem>,
    start: usize,
    len: usize,
    notify: ModelNotify,
}

impl Model for Line {
    type Data = CardItem;

    fn row_count(&self) -> usize {
        self.len
    }

    fn row_data(&self, row: usize) -> Option<CardItem> {
        (row < self.len)
            .then(|| self.cards.row_data(self.start + row))
            .flatten()
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::VecModel;

    fn card(id: &str) -> CardItem {
        CardItem {
            id: id.into(),
            ..Default::default()
        }
    }

    fn ids(lines: &ModelRc<CardLine>) -> Vec<Vec<String>> {
        lines
            .iter()
            .map(|l| l.cards.iter().map(|c| c.id.to_string()).collect())
            .collect()
    }

    #[test]
    fn cards_are_cut_into_lines_the_last_one_short() {
        let cards = Rc::new(VecModel::from(["a", "b", "c", "d", "e"].map(card).to_vec()));
        let lines = lines(ModelRc::from(cards), 2);
        assert_eq!(ids(&lines), [vec!["a", "b"], vec!["c", "d"], vec!["e"]]);
        assert_eq!(lines.row_data(2).map(|l| l.first), Some(4));
    }

    #[test]
    fn a_card_set_shows_in_its_line_which_stays_the_same_model() {
        let cards = Rc::new(VecModel::from(["a", "b", "c"].map(card).to_vec()));
        let lines = lines(ModelRc::from(cards.clone()), 2);
        let first = lines.row_data(0).expect("line 0").cards;
        cards.set_row_data(1, card("B"));
        assert_eq!(
            first.row_data(1).map(|c| c.id.to_string()),
            Some("B".into())
        );
        assert!(lines.row_data(0).expect("line 0").cards == first);
    }

    #[test]
    fn cards_added_make_the_lines_again() {
        let cards = Rc::new(VecModel::from(["a", "b", "c"].map(card).to_vec()));
        let lines = lines(ModelRc::from(cards.clone()), 2);
        let _held = lines.row_data(1);
        cards.push(card("d"));
        assert_eq!(ids(&lines), [vec!["a", "b"], vec!["c", "d"]]);
    }
}
