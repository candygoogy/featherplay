//! The songs YouTube refuses to play now (`UiEvent::Refused`): every song row shows them
//! greyed, with why on hover (`Refused` in `ui/parts/items.slint`, read by the rows in
//! `ui/parts/rows.slint`). The ids are kept here; a change bumps `Refused.changes`, so the
//! rows on screen ask again.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use featherplay_core::model::TrackId;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::ui::Refused;

thread_local! {
    static IDS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    ui.global::<Refused>()
        .on_has(|id| IDS.with(|ids| ids.borrow().contains(id.as_str())));
}

/// The songs YouTube refuses are now `ids`.
pub fn show(ctx: &Ctx, ids: &[TrackId]) {
    IDS.with(|set| *set.borrow_mut() = ids.iter().map(|id| id.0.clone()).collect());
    let Some(ui) = ctx.window() else { return };
    let refused = ui.global::<Refused>();
    refused.set_changes(refused.get_changes().wrapping_add(1));
}
