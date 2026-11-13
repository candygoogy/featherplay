//! The lyrics' text size: Settings > Lyrics' chips, the lyrics menu's A's and Ctrl+= / Ctrl+-
//! all pick it through `LyricsData.pick-size` (src/lyrics.rs passes it here); and "Reset
//! lyrics settings", which puts everything of the lyrics back to the defaults.

use std::rc::Rc;

use featherplay_store::{DesktopLyrics, LyricsSize};
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
    show_lyrics_resettable(ctx);
}

/// Shows Settings' "Reset lyrics settings" only while something of the lyrics differs from
/// the defaults: the text size, the desktop lyrics' size, place or lock, a song's choices.
pub fn show_lyrics_resettable(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let (size, strip) = SETTINGS.with(|s| {
        let s = s.borrow();
        (s.lyrics_size, s.desktop_lyrics.clone())
    });
    let defaults = DesktopLyrics {
        on: strip.on,
        hinted: strip.hinted,
        ..DesktopLyrics::default()
    };
    let differs =
        size != LyricsSize::default() || strip != defaults || crate::lyrics::songs_chosen();
    ui.global::<LyricsData>().set_resettable(differs);
}

/// "Reset lyrics settings", confirmed: the text size back to Medium, the desktop lyrics'
/// size, place and lock to their defaults, every song's timing and pick forgotten. Whether
/// lyrics and desktop lyrics are on stays as it is.
pub fn reset_lyrics(ctx: &Rc<Ctx>) {
    log!("lyrics settings reset");
    pick_lyrics_size(ctx, 1);
    crate::desktop_lyrics::reset(ctx);
    crate::lyrics::forget_choices(ctx);
    show_lyrics_resettable(ctx);
}
