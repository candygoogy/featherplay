//! The sidebar's playlist menu (ui/playlist_menu.slint): Play, Shuffle, Start radio, Play
//! next, Add to queue and Copy link for any library playlist and Liked songs; Rename and
//! Delete for the account's own playlists; Remove from library for saved ones; Pin to top
//! for every library playlist. The sidebar's albums and artists get Play, Shuffle, Go to
//! album or artist, and Pin to top.
//!
//! Playing fetches the playlist once (one read); the writes go out once each and are never
//! retried. After a rename, delete or removal the sidebar's list is fetched again. Failures
//! show a toast.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use featherplay_controller::place::Place;
use featherplay_core::model::{AlbumId, ArtistId, PlaylistId, Track};
use featherplay_core::tr;
use featherplay_ytm::{ApiError, PlaylistSummary};
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::{Action, PlayingFrom, QueueAction};
use crate::library_list::Kind;
use crate::pages::content;
use crate::router::Page;
use crate::sidebar;
use crate::song_menu;
use crate::ui::{ItemKind, PlaylistDialog, PlaylistKind, PlaylistMenu};
use crate::ui_bridge::shown;

/// Liked songs' playlist id.
const LIKED: &str = "LM";

thread_local! {
    /// The library playlists that are the account's own, as the sidebar last loaded them,
    /// with their names as written (the menu's items show them plain, `ui_bridge::shown`).
    static OWNED: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Notes which of the sidebar's playlists are the account's own (`load_playlists`).
pub fn remember(playlists: &[PlaylistSummary]) {
    let owned = playlists
        .iter()
        .filter(|p| p.owned)
        .map(|p| (p.id.0.clone(), p.title.clone()))
        .collect();
    OWNED.with_borrow_mut(|o| *o = owned);
}

/// Which rows the menu shows for playlist `id`.
pub fn kind(id: &str, owned: bool) -> PlaylistKind {
    if id == LIKED {
        PlaylistKind::Liked
    } else if owned {
        PlaylistKind::Own
    } else {
        PlaylistKind::Saved
    }
}

/// The playlist's link on music.youtube.com, as Share gives it there.
pub fn link(id: &str) -> String {
    let bare = featherplay_ytm::convert::bare_playlist_id(id);
    format!("https://music.youtube.com/playlist?list={bare}")
}

/// What a toast says when a write didn't go through: what failed (`failed`, a sentence:
/// "Couldn't delete “Mix”."), then what to do.
pub fn failure_text(failed: &str, e: &ApiError) -> String {
    log!("playlist menu: {failed} {e}");
    match e {
        ApiError::AuthExpired | ApiError::SignInRequired => {
            tr!("{} Sign in again, then try once more.", failed)
        }
        ApiError::Network(_) => tr!("{} Check the connection, then try again.", failed),
        ApiError::NotFound => tr!("{} The playlist is gone.", failed),
        ApiError::Parse(_) => tr!("{} The music service said no.", failed),
    }
}

/// `“Name”`, as toasts and the Delete dialog quote a playlist.
fn quoted(name: &str) -> String {
    format!("\u{201C}{name}\u{201D}")
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let menu = ui.global::<PlaylistMenu>();

    let c = ctx.clone();
    menu.on_opened(move |id| {
        let Some(ui) = c.window() else { return };
        let written = OWNED.with_borrow(|o| o.get(id.as_str()).cloned());
        let owned = written.is_some();
        let menu = ui.global::<PlaylistMenu>();
        menu.set_written(
            written
                .unwrap_or_else(|| menu.get_item().title.into())
                .into(),
        );
        let entry = entry_kind(menu.get_item().kind);
        menu.set_kind(match entry {
            Kind::Album => PlaylistKind::Album,
            Kind::Artist => PlaylistKind::Artist,
            Kind::Playlist => kind(&id, owned),
        });
        menu.set_pinned(sidebar::is_pinned(entry, &id));
        menu.set_link(link(&id).into());
    });
    let c = ctx.clone();
    menu.on_open_item(move |id| {
        let Some(ui) = c.window() else { return };
        let kind = entry_kind(ui.global::<PlaylistMenu>().get_item().kind);
        c.navigate(place_of(kind, &id).into());
    });
    let c = ctx.clone();
    menu.on_pin(move |id| {
        let Some(ui) = c.window() else { return };
        let item = ui.global::<PlaylistMenu>().get_item();
        let pinned = sidebar::toggle_pin(&c, entry_kind(item.kind), &id);
        c.toast(&if pinned {
            tr!("Pinned {} to the top", quoted(&item.title))
        } else {
            tr!("Unpinned {}", quoted(&item.title))
        });
    });

    let c = ctx.clone();
    menu.on_play(move |id| with_tracks(&c, &id, play));
    let c = ctx.clone();
    menu.on_shuffle(move |id| {
        with_tracks(&c, &id, |c, from, tracks| {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1);
            play(c, from, content::shuffled(&tracks, seed));
        })
    });
    let c = ctx.clone();
    menu.on_play_next(move |id| {
        with_tracks(&c, &id, |c, from, tracks| {
            c.toast(&tr!("Playing next: {}", from.label));
            c.send(Action::Queue(QueueAction::PlayNextAll(tracks)));
        })
    });
    let c = ctx.clone();
    menu.on_add_to_queue(move |id| {
        with_tracks(&c, &id, |c, from, tracks| {
            c.toast(&tr!("Added to queue: {}", from.label));
            c.send(Action::Queue(QueueAction::Enqueue(tracks)));
        })
    });
    let c = ctx.clone();
    menu.on_start_radio(move |id| start_radio(&c, &id));
    let c = ctx.clone();
    menu.on_copied(move |_| c.toast(tr!("Link copied")));

    let c = ctx.clone();
    menu.on_rename(move |id, name| rename(&c, &id, &name));
    let c = ctx.clone();
    menu.on_delete(move |id| delete(&c, &id));
    let c = ctx.clone();
    menu.on_remove(move |id| remove(&c, &id));
}

/// The menu's playlist title (the dialog's, once the menu has closed for it).
fn title_of(ctx: &Ctx, id: &str) -> String {
    let Some(ui) = ctx.window() else {
        return String::new();
    };
    let menu = ui.global::<PlaylistMenu>();
    [menu.get_item(), menu.get_target()]
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.title.to_string())
        .unwrap_or_default()
}

/// What the menu's entry is, as the sidebar's list keeps it.
fn entry_kind(kind: ItemKind) -> Kind {
    match kind {
        ItemKind::Album => Kind::Album,
        ItemKind::Artist => Kind::Artist,
        _ => Kind::Playlist,
    }
}

/// Where the entry is: its page.
fn place_of(kind: Kind, id: &str) -> Place {
    let id = id.to_string();
    match kind {
        Kind::Album => Place::Album(AlbumId(id)),
        Kind::Artist => Place::Artist(ArtistId(id)),
        Kind::Playlist => Place::Playlist(PlaylistId(id)),
    }
}

/// Fetches the entry's tracks (one read: the playlist, the album, or the artist's top
/// songs), then hands them to `then` with its name and page.
fn with_tracks(
    ctx: &Rc<Ctx>,
    id: &str,
    then: impl FnOnce(&Ctx, PlayingFrom, Vec<Track>) + 'static,
) {
    let name = title_of(ctx, id);
    let kind = ctx
        .window()
        .map(|ui| entry_kind(ui.global::<PlaylistMenu>().get_item().kind))
        .unwrap_or(Kind::Playlist);
    let from = PlayingFrom::on_page(name.clone(), place_of(kind, id));
    let id = id.to_string();
    let what = match kind {
        Kind::Playlist => tr!("Couldn't load the playlist."),
        Kind::Album => tr!("Couldn't load the album."),
        Kind::Artist => tr!("Couldn't load the artist."),
    };
    ctx.load(
        move |source| async move {
            match kind {
                Kind::Playlist => source.playlist(&PlaylistId(id)).await.map(|p| p.tracks),
                Kind::Album => source.album(&AlbumId(id)).await.map(|a| a.tracks),
                Kind::Artist => source.artist(&ArtistId(id)).await.map(|a| a.top_songs),
            }
        },
        move |ctx, _, result| match result {
            Ok(tracks) if !tracks.is_empty() => {
                song_menu::remember(&tracks);
                then(ctx, from, tracks);
            }
            Ok(_) => ctx.toast(&tr!("{} has no songs yet", quoted(&name))),
            Err(e) => ctx.toast_error(&failure_text(what, &e)),
        },
    );
}

/// Plays `tracks` from the top, with Up next saying where they're from.
fn play(ctx: &Ctx, from: PlayingFrom, tracks: Vec<Track>) {
    ctx.send(Action::Queue(QueueAction::PlayFrom {
        tracks,
        start: 0,
        radio: false,
        from,
    }));
}

/// The playlist's radio (one read), playing at once.
fn start_radio(ctx: &Rc<Ctx>, id: &str) {
    let name = title_of(ctx, id);
    let pid = PlaylistId(id.to_string());
    ctx.load(
        move |source| async move { source.playlist_radio(&pid).await },
        move |ctx, _, result| match result {
            Ok(tracks) if !tracks.is_empty() => {
                song_menu::remember(&tracks);
                ctx.send(Action::Queue(QueueAction::PlayFrom {
                    tracks,
                    start: 0,
                    radio: true,
                    from: PlayingFrom::named(format!("{name} radio")),
                }));
            }
            Ok(_) => ctx.toast(tr!("This playlist has no radio")),
            Err(e) => ctx.toast_error(&failure_text(tr!("Couldn't start the radio."), &e)),
        },
    );
}

/// Rename's Save: an empty name stays in the dialog with a reason; the same name just
/// closes it; a new one closes it and goes out (one write).
fn rename(ctx: &Rc<Ctx>, id: &str, name: &str) {
    let Some(ui) = ctx.window() else { return };
    let menu = ui.global::<PlaylistMenu>();
    let name = name.trim().to_string();
    if name.is_empty() {
        menu.set_dialog_error(tr!("Enter a name for the playlist.").into());
        return;
    }
    // As written: the field started from it (`written`), so an unchanged name sends nothing.
    let old = OWNED
        .with_borrow(|o| o.get(id).cloned())
        .unwrap_or_else(|| title_of(ctx, id));
    menu.invoke_close_dialog();
    if name == old {
        return;
    }
    let pid = PlaylistId(id.to_string());
    crate::pages::ahead::forget_playlist(&pid);
    let sent = name.clone();
    ctx.load(
        move |source| async move { source.rename_playlist(&pid, &sent).await },
        move |ctx, _, result| match result {
            Ok(()) => {
                ctx.toast(&tr!("Renamed to {}", quoted(&shown(&name))));
                changed(ctx, Change::Renamed);
            }
            Err(e) => ctx.toast_error(&failure_text(
                &tr!("Couldn't rename {}.", quoted(&shown(&old))),
                &e,
            )),
        },
    );
}

/// Delete's confirmation went through: one write, then the list again.
fn delete(ctx: &Rc<Ctx>, id: &str) {
    let name = title_of(ctx, id);
    if let Some(ui) = ctx.window() {
        ui.global::<PlaylistMenu>().invoke_close_dialog();
    }
    let pid = PlaylistId(id.to_string());
    crate::pages::ahead::forget_playlist(&pid);
    ctx.load(
        {
            let pid = pid.clone();
            move |source| async move { source.delete_playlist(&pid).await }
        },
        move |ctx, _, result| match result {
            Ok(()) => {
                ctx.toast(&tr!("Deleted {}", quoted(&name)));
                changed(ctx, Change::Gone(pid));
            }
            Err(e) => ctx.toast_error(&failure_text(
                &tr!("Couldn't delete {}.", quoted(&name)),
                &e,
            )),
        },
    );
}

/// Remove from library, for a saved playlist: one write, then the list again.
fn remove(ctx: &Rc<Ctx>, id: &str) {
    let name = title_of(ctx, id);
    let pid = PlaylistId(id.to_string());
    crate::pages::ahead::forget_playlist(&pid);
    ctx.load(
        {
            let pid = pid.clone();
            move |source| async move { source.remove_playlist_from_library(&pid).await }
        },
        move |ctx, _, result| match result {
            Ok(()) => {
                ctx.toast(&tr!("Removed {} from your library", quoted(&name)));
                changed(ctx, Change::Gone(pid));
            }
            Err(e) => ctx.toast_error(&failure_text(
                &tr!("Couldn't remove {}.", quoted(&name)),
                &e,
            )),
        },
    );
}

/// What a write did to a playlist.
enum Change {
    Renamed,
    /// Deleted, or removed from the library.
    Gone(PlaylistId),
}

/// After a write: the sidebar's list again, and the page if it shows a playlist (a renamed
/// one shows its new name; a deleted or removed one is gone, so Home instead).
fn changed(ctx: &Rc<Ctx>, change: Change) {
    sidebar::load_playlists(ctx);
    match (ctx.current_page(), change) {
        (Page::Playlist { id }, Change::Gone(gone)) if id == gone => ctx.navigate(Page::Home),
        (Page::Playlist { .. }, Change::Renamed) => ctx.show_current(),
        _ => {}
    }
}

/// `--snapshot playlist-menu*`: opens the menu on the sidebar's first playlist (the
/// account's own), its second (saved) or Liked songs, at the row's middle as a right-click
/// there would, and with `dialog` one of its dialogs instead.
pub fn show_for_snapshot(ctx: &Ctx, which: &str) {
    let Some(ui) = ctx.window() else { return };
    let playlists = ui.global::<crate::ui::SidebarData>().get_playlists();
    let (item, y) = match which {
        "liked" => (
            crate::ui::CardItem {
                id: LIKED.into(),
                kind: crate::ui::ItemKind::Playlist,
                title: "Liked songs".into(),
                ..Default::default()
            },
            208.0,
        ),
        "saved" => (
            slint::Model::row_data(&playlists, 1).unwrap_or_default(),
            336.0,
        ),
        _ => (
            slint::Model::row_data(&playlists, 0).unwrap_or_default(),
            296.0,
        ),
    };
    let menu = ui.global::<PlaylistMenu>();
    menu.invoke_show(item, 120.0, y, y);
    match which {
        "rename" => menu.invoke_ask(PlaylistDialog::Rename),
        "delete" => menu.invoke_ask(PlaylistDialog::Delete),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liked_songs_is_liked_whoever_owns_it() {
        assert_eq!(kind("LM", false), PlaylistKind::Liked);
        assert_eq!(kind("LM", true), PlaylistKind::Liked);
    }

    #[test]
    fn library_playlists_are_own_or_saved() {
        assert_eq!(kind("PLmine", true), PlaylistKind::Own);
        assert_eq!(kind("PLtheirs", false), PlaylistKind::Saved);
    }

    #[test]
    fn the_link_is_the_bare_id_on_music_youtube_com() {
        assert_eq!(
            link("VLPLabc"),
            "https://music.youtube.com/playlist?list=PLabc"
        );
        assert_eq!(link("LM"), "https://music.youtube.com/playlist?list=LM");
    }

    #[test]
    fn a_failure_names_what_failed_and_what_to_do() {
        let text = failure_text(
            "Couldn't delete “Mix”.",
            &ApiError::Network("timeout".into()),
        );
        assert_eq!(
            text,
            "Couldn't delete “Mix”. Check the connection, then try again."
        );
        assert!(
            !text.contains("timeout"),
            "the toast echoes the error's details"
        );
    }
}
