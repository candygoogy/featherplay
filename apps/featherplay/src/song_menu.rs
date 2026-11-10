//! The song menu (ui/song_menu.slint): Play next, Add to queue, Like or Dislike, Add to
//! playlist, Start radio. Rows open it from Slint with the row's `TrackRow`; the actions
//! need the full `Track`, so pages hand their loaded tracks to `remember` first.
//!
//! Likes from the menu and from the hearts in the rows go through `set_like`, which sends one
//! `Action::Rate` and flips the heart on every page's rows at once.
//!
//! A song row or card the pointer rests on (`RowHover`) sends `Action::Prefetch` with its
//! remembered track, so the controller resolves it before the click.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use featherplay_core::model::{LikeStatus, PlaylistId, Track, TrackId};
use featherplay_core::tr;
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::app::Ctx;
use crate::controller::Action;
use crate::pages::content;
use crate::router::Page;
use crate::toast;
use crate::ui::{
    AlbumData, ArtistData, CardItem, HomeData, LibraryData, Player, PlaylistData, RowHover,
    SearchData, SidebarData, SongMenu, Theme, TrackRow,
};

/// Tracks remembered for the menu; the map starts over past this.
const KNOWN_CAP: usize = 20_000;

thread_local! {
    static KNOWN: RefCell<HashMap<String, Track>> = RefCell::new(HashMap::new());
}

/// Lets the menu find these tracks by id.
pub fn remember(tracks: &[Track]) {
    KNOWN.with(|k| {
        let mut k = k.borrow_mut();
        if k.len() + tracks.len() > KNOWN_CAP {
            k.clear();
        }
        for t in tracks {
            k.insert(t.id.0.clone(), t.clone());
        }
    });
}

/// A remembered track, by id.
pub fn find(id: &str) -> Option<Track> {
    KNOWN.with(|k| k.borrow().get(id).cloned())
}

/// A rating made here, for the remembered track: its rows and the pages shown again from
/// what they showed (`pages::ahead`) read it.
pub fn remember_rating(id: &TrackId, status: LikeStatus) {
    KNOWN.with(|k| {
        if let Some(t) = k.borrow_mut().get_mut(&id.0) {
            t.like = status;
        }
    });
}

/// A remembered track's rating, with the likes made since it was loaded.
pub fn like_of(id: &TrackId) -> Option<LikeStatus> {
    KNOWN.with(|k| k.borrow().get(&id.0).map(|t| t.like))
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let c = ctx.clone();
    ui.global::<RowHover>().on_rested(move |id| {
        let track = find(&id);
        c.send(Action::WakeOutput);
        c.send(Action::Prefetch {
            id: TrackId(id.into()),
            track,
        });
    });
    let menu = ui.global::<SongMenu>();

    let c = ctx.clone();
    menu.on_opened(move |_| {
        let Some(ui) = c.window() else { return };
        // The library playlists the sidebar already shows, without Liked songs.
        let playlists: Vec<CardItem> = ui
            .global::<SidebarData>()
            .get_playlists()
            .iter()
            .filter(|p| !matches!(p.id.as_str(), "LM" | "VLLM"))
            .collect();
        let menu = ui.global::<SongMenu>();
        menu.set_signed_in(!c.signed_out());
        menu.set_playlists(ModelRc::new(VecModel::from(playlists)));
    });

    let c = ctx.clone();
    menu.on_play_next(move |id| match find(&id) {
        Some(t) => {
            c.toast(&tr!("Playing next: {}", t.display_title()));
            c.send(Action::PlayNext(t));
        }
        None => log!("play next: unknown track {id}"),
    });
    let c = ctx.clone();
    menu.on_add_to_queue(move |id| match find(&id) {
        Some(t) => {
            c.toast(&tr!("Added to queue: {}", t.display_title()));
            c.send(Action::AddToQueue(t));
        }
        None => log!("add to queue: unknown track {id}"),
    });
    let c = ctx.clone();
    menu.on_like(move |id, liked| set_like(&c, &TrackId(id.into()), liked));
    let c = ctx.clone();
    menu.on_dislike(move |id| {
        if !c.needs_account(tr!("Sign in to rate songs")) {
            rate(&c, &TrackId(id.into()), LikeStatus::Dislike);
        }
    });
    let c = ctx.clone();
    menu.on_add_signed_out(move || {
        c.needs_account(tr!("Sign in to add songs to your playlists"));
    });
    let c = ctx.clone();
    menu.on_add_to_playlist(move |track, playlist| {
        let name = c.window().and_then(|ui| {
            let playlists = ui.global::<SongMenu>().get_playlists();
            playlists
                .iter()
                .find(|p| p.id == playlist)
                .map(|p| p.title.to_string())
        });
        let id = PlaylistId(playlist.to_string());
        crate::pages::ahead::forget_playlist(&id);
        c.send(Action::AddToPlaylist {
            playlist: id.clone(),
            tracks: vec![TrackId(track.into())],
        });
        if let Some(name) = name {
            toast::show_with(&c, &tr!("Saved to {}", name), tr!("View"), move |c| {
                c.navigate(Page::Playlist { id: id.clone() })
            });
        }
    });
    let c = ctx.clone();
    menu.on_start_radio(move |id| c.send(Action::Radio(TrackId(id.into()))));
}

/// Likes (`true`) or removes the like of a track.
pub fn set_like(ctx: &Ctx, id: &TrackId, liked: bool) {
    if ctx.needs_account(tr!("Sign in to like songs")) {
        return;
    }
    let status = if liked {
        LikeStatus::Like
    } else {
        LikeStatus::Indifferent
    };
    rate(ctx, id, status);
    ctx.toast(if liked {
        tr!("Added to Liked songs")
    } else {
        tr!("Removed from Liked songs")
    });
}

/// One write to the account; the hearts flip at once on every page and in the bar (the
/// controller's state puts the bar back if the write fails).
pub fn rate(ctx: &Ctx, id: &TrackId, status: LikeStatus) {
    ctx.send(Action::Rate(id.clone(), status));
    show_rating(ctx, id, status);
}

/// Shows a rating everywhere the track is: the hearts on every page's rows and in the bar,
/// and the remembered track. `rate` sends the write first; the album menu's job, after its
/// own write went through.
pub fn show_rating(ctx: &Ctx, id: &TrackId, status: LikeStatus) {
    remember_rating(id, status);
    let Some(ui) = ctx.window() else { return };
    let liked = status == LikeStatus::Like;
    let models: [ModelRc<TrackRow>; 6] = [
        ui.global::<AlbumData>().get_tracks(),
        ui.global::<PlaylistData>().get_tracks(),
        ui.global::<LibraryData>().get_liked(),
        ui.global::<SearchData>().get_songs(),
        ui.global::<ArtistData>().get_top_songs(),
        ui.global::<HomeData>().get_next(),
    ];
    for rows in &models {
        content::set_liked(rows, &id.0, liked);
    }
    let player = ui.global::<Player>();
    if player.get_track_id().as_str() == id.0 {
        player.set_liked(liked);
        player.set_disliked(status == LikeStatus::Dislike);
    }
}

/// `--snapshot album-menu`: opens the menu on the album's third track where its ⋯ opens it
/// (ui/pages/album.slint: 8 px in from the row's bottom right; without the Up next panel),
/// so the menu flips to the left of the anchor.
pub fn show_for_snapshot(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let Some(row) = ui.global::<AlbumData>().get_tracks().row_data(2) else {
        return;
    };
    let size = ui.window().size().to_logical(ui.window().scale_factor());
    let t = ui.global::<Theme>();
    let side = if size.width >= t.get_wide_min() {
        t.get_sidebar_w()
    } else {
        t.get_rail_w()
    };
    let cover_h = if size.width - side >= 900.0 {
        232.0
    } else {
        200.0
    };
    // The album page runs up under the top bar, so its header (`header-h`) starts below it,
    // in window coordinates as in the page's.
    let header_h = t.get_top_bar_h() + t.get_page_top() + cover_h + t.get_s6();
    let x = size.width - t.get_page_pad();
    let top = header_h + 2.0 * t.get_track_row_h();
    let y = top + t.get_track_row_h() - 8.0;
    ui.global::<SongMenu>().invoke_show(row, x, y, top);
}
