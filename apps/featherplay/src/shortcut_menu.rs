//! The right-click menu of Home's Shortcuts tiles (ui/shortcut_menu.slint): Play, Shuffle,
//! Play next and Add to queue; Pin or Remove from Shortcuts on a suggestion; Unpin, Move
//! left and Move right on a pin. Liked songs gets Play and Shuffle.
//!
//! Play and Shuffle start as the tile's own play button does (a playlist streams all its
//! pages); Play next, Add to queue and an album's or artist's Shuffle read the songs once,
//! as the sidebar's menu does (`playlist_menu::tracks_of`). The pins and removals are the
//! sidebar's (`sidebar::toggle_pin`, `sidebar::hide`), so the row and the sidebar change at
//! once.

use std::rc::Rc;

use featherplay_core::model::{AlbumId, ArtistId, PlaylistId};
use featherplay_core::tr;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::{Action, QueueAction};
use crate::library_list::Kind;
use crate::pages::content;
use crate::ui::{CardItem, ItemKind, ShortcutMenu};
use crate::{playlist_menu, sidebar, toast};

/// Liked songs' playlist id.
const LIKED: &str = "LM";

/// What a tile is, as the sidebar keeps it.
fn kind_of(item: &CardItem) -> Kind {
    match item.kind {
        ItemKind::Album => Kind::Album,
        ItemKind::Artist => Kind::Artist,
        _ => Kind::Playlist,
    }
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let menu = ui.global::<ShortcutMenu>();

    let c = ctx.clone();
    menu.on_opened(move |item| {
        let Some(ui) = c.window() else { return };
        let place = sidebar::pin_place(kind_of(&item), &item.id);
        let menu = ui.global::<ShortcutMenu>();
        menu.set_can_left(place.is_some_and(|(at, _)| at > 0));
        menu.set_can_right(place.is_some_and(|(at, n)| at + 1 < n));
    });
    let c = ctx.clone();
    menu.on_play(move |item| play(&c, &item));
    let c = ctx.clone();
    menu.on_shuffle(move |item| shuffle(&c, &item));
    let c = ctx.clone();
    menu.on_play_next(move |item| {
        playlist_menu::tracks_of(&c, kind_of(&item), &item.id, item.title.to_string(), {
            move |c, from, tracks| {
                c.toast(&tr!("Playing next: {}", from.label));
                c.send(Action::Queue(QueueAction::PlayNextAll(tracks)));
            }
        });
    });
    let c = ctx.clone();
    menu.on_add_to_queue(move |item| {
        playlist_menu::tracks_of(&c, kind_of(&item), &item.id, item.title.to_string(), {
            move |c, from, tracks| {
                c.toast(&tr!("Added to queue: {}", from.label));
                c.send(Action::Queue(QueueAction::Enqueue(tracks)));
            }
        });
    });
    let c = ctx.clone();
    menu.on_pin(move |item| {
        let pinned = sidebar::toggle_pin(&c, kind_of(&item), &item.id);
        playlist_menu::pin_toast(&c, &item.title, pinned);
    });
    let c = ctx.clone();
    menu.on_move(move |item, later| sidebar::move_pin(&c, kind_of(&item), &item.id, later));
    let c = ctx.clone();
    menu.on_remove(move |item| remove(&c, &item));
}

/// Plays the tile's album, playlist or artist from the start (Liked songs too).
fn play(ctx: &Rc<Ctx>, item: &CardItem) {
    let id = item.id.to_string();
    match kind_of(item) {
        Kind::Album => content::play_album(ctx, AlbumId(id)),
        Kind::Artist => content::play_artist(ctx, ArtistId(id)),
        Kind::Playlist => content::play_playlist(ctx, PlaylistId(id)),
    }
}

/// Plays the tile's songs in a random order.
fn shuffle(ctx: &Rc<Ctx>, item: &CardItem) {
    match kind_of(item) {
        Kind::Playlist => content::shuffle_playlist(ctx, PlaylistId(item.id.to_string())),
        kind => playlist_menu::tracks_of(ctx, kind, &item.id, item.title.to_string(), {
            |c, from, tracks| playlist_menu::play(c, from, content::shuffled(&tracks, seed()))
        }),
    }
}

fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

/// Remove from Shortcuts: gone until it's played again, with Undo in the toast.
fn remove(ctx: &Rc<Ctx>, item: &CardItem) {
    if item.id == LIKED {
        return;
    }
    let (kind, id) = (kind_of(item), item.id.to_string());
    sidebar::hide(ctx, kind, &id, true);
    toast::show_with(
        ctx,
        &tr!("Removed {} from Shortcuts", item.title),
        tr!("Undo"),
        move |c| sidebar::hide(c, kind, &id, false),
    );
}
