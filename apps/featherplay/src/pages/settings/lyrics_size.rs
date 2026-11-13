//! The lyrics' text size: Settings > Lyrics' chips, the lyrics menu's A's and Ctrl+= / Ctrl+-
//! all pick it through `LyricsData.pick-size` (src/lyrics.rs passes it here).

use featherplay_store::LyricsSize;
use slint::ComponentHandle;

use super::{SETTINGS, commit};
use crate::app::Ctx;
use crate::controller::Change;
use crate::ui::LyricsData;

/// Shows `size` on the lyrics views, the menu and the Settings row.
pub(super) fn show_lyrics_size(ctx: &Ctx, size: LyricsSize) {
    let Some(ui) = ctx.window() else { return };
    let at = LyricsSize::ALL.iter().position(|&s| s == size).unwrap_or(1);
    ui.global::<LyricsData>()
        .set_size(i32::try_from(at).unwrap_or(1));
}

/// The size at `i` (0 Small, 1 Medium, 2 Large), shown at once and saved.
pub fn pick_lyrics_size(ctx: &Ctx, i: i32) {
    let Some(&size) = usize::try_from(i).ok().and_then(|i| LyricsSize::ALL.get(i)) else {
        return;
    };
    show_lyrics_size(ctx, size);
    if SETTINGS.with(|s| s.borrow().lyrics_size != size) {
        commit(ctx, Change::LyricsSize(size));
    }
}
