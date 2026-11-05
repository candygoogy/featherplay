//! "Not for me": the thumbs-down beside the heart, in the bar and on Now playing. It rates
//! the song Dislike (one write, as the song menu's Dislike) and skips to the next one, then a
//! toast offers Undo: the rating goes back to what it was (no rating, or a like) and the
//! player returns to the song where it was (`Action::BackTo`).

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
    /// The rating before "Not for me".
    was: LikeStatus,
    /// Where the song was when "Not for me" skipped it.
    at: Duration,
}

impl Undo {
    /// The rating back (when "Not for me" changed it), then the song at `at`.
    pub fn actions(&self) -> Vec<Action> {
        let rate = (self.was != LikeStatus::Dislike)
            .then(|| Action::Rate(self.track.id.clone(), self.was));
        let back = Action::BackTo {
            track: self.track.clone(),
            at: self.at,
        };
        rate.into_iter().chain([back]).collect()
    }
}

/// What "Not for me" does to `track`, playing at `at`: rate it Dislike, then skip it. A song
/// already disliked (the thumbs-down shows filled) is only skipped: rating it again would be
/// one more write to the account for nothing. The `Undo` keeps its rating from before.
pub fn actions(track: &Track, at: Duration) -> (Vec<Action>, Undo) {
    let undo = Undo {
        track: track.clone(),
        was: track.like,
        at,
    };
    let rate = (track.like != LikeStatus::Dislike)
        .then(|| Action::Rate(track.id.clone(), LikeStatus::Dislike));
    (rate.into_iter().chain([Action::Next]).collect(), undo)
}

/// The thumbs-down button: the current track is not for me.
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

/// "Not for me" in the tray icon's menu: as the bar's while the window is on screen, toast
/// and Undo included; hidden in the tray or minimized, nobody would see the toast, so the
/// song is just rated and skipped.
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
