//! "Not for me": the ⊘ beside the heart, in the bar and on Now playing. It rates the song
//! Dislike (one write, as the song menu's Dislike) and skips to the next one, then a toast
//! offers Undo: the rating goes back to what it was (no rating, or a like) and the player
//! returns to the song where it was (`Action::BackTo`).

use std::rc::Rc;
use std::time::Duration;

use featherplay_core::model::{LikeStatus, Track};
use featherplay_core::tr;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::Action;
use crate::ui::Frame;
use crate::{bar, song_menu, toast};

/// What Undo puts back.
#[derive(Clone, Debug)]
pub struct Undo {
    track: Track,
    /// The rating before ⊘.
    was: LikeStatus,
    /// Where the song was when ⊘ skipped it.
    at: Duration,
}

impl Undo {
    /// The rating back, then the song at `at`.
    pub fn actions(&self) -> Vec<Action> {
        vec![
            Action::Rate(self.track.id.clone(), self.was),
            Action::BackTo {
                track: self.track.clone(),
                at: self.at,
            },
        ]
    }
}

/// What ⊘ does to `track`, playing at `at`: rate it Dislike, then skip it. The `Undo` keeps
/// its rating from before.
pub fn actions(track: &Track, at: Duration) -> (Vec<Action>, Undo) {
    let undo = Undo {
        track: track.clone(),
        was: track.like,
        at,
    };
    (
        vec![
            Action::Rate(track.id.clone(), LikeStatus::Dislike),
            Action::Next,
        ],
        undo,
    )
}

/// The ⊘ button: the current track is not for me.
pub fn start(ctx: &Rc<Ctx>) {
    let Some(state) = ctx.player() else { return };
    let Some(track) = state.current else { return };
    let at = bar::position().unwrap_or(state.position);
    let (now, undo) = actions(&track, at);
    send(ctx, now);
    toast::show_with(ctx, tr!("Marked as not for me"), tr!("Undo"), move |c| {
        send(c, undo.actions())
    });
}

/// ⊘ in the tray icon's menu: as the bar's while the window is on screen, toast and Undo
/// included; hidden in the tray or minimized, nobody would see the toast, so the song is
/// just rated and skipped.
pub fn from_tray(ctx: &Rc<Ctx>) {
    let on_screen = ctx
        .window()
        .is_some_and(|ui| !ui.global::<Frame>().get_hidden() && !ui.window().is_minimized());
    if on_screen {
        start(ctx);
        return;
    }
    let Some(state) = ctx.player() else { return };
    let Some(track) = state.current else { return };
    let (now, _) = actions(&track, bar::position().unwrap_or(state.position));
    send(ctx, now);
}

/// Ratings go through the song menu's path, so every page's heart follows at once.
fn send(ctx: &Ctx, actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::Rate(id, status) => song_menu::rate(ctx, &id, status),
            other => ctx.send(other),
        }
    }
}
