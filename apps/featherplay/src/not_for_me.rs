//! "Not for me": the thumbs-down beside the heart, in the bar, on Now playing and in the mini
//! player. It rates the song Dislike (one write, as the song menu's Dislike) and skips to the
//! next one, then a toast offers Undo: the rating goes back to what it was (no rating, or a
//! like) and the player returns to the song where it was (`Action::BackTo`). On a song that
//! shows it already (the thumbs-down filled) it's a toggle, as the heart is: the press only
//! takes the dislike off, and the song plays on.

use std::rc::Rc;
use std::time::Duration;

use featherplay_core::model::{LikeStatus, Track};
use featherplay_core::tr;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::Action;
use crate::ui::{Frame, Player};
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
/// already disliked is only skipped (the tray's item, which doesn't show the rating): rating
/// it again would be one more write to the account for nothing. The `Undo` keeps its rating
/// from before.
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

/// What a press of the thumbs-down button does.
#[derive(Debug)]
pub enum Press {
    /// Rate it Dislike and skip it (`actions`); the toast offers the `Undo`.
    Mark(Vec<Action>, Undo),
    /// The song shows the dislike: take it off (no rating), and stay on the song.
    Unmark(Action),
}

/// What the thumbs-down button does to `track`, its `like` as the button shows it, playing
/// at `at`.
pub fn press(track: &Track, at: Duration) -> Press {
    if track.like == LikeStatus::Dislike {
        Press::Unmark(Action::Rate(track.id.clone(), LikeStatus::Indifferent))
    } else {
        let (now, undo) = actions(track, at);
        Press::Mark(now, undo)
    }
}

/// The thumbs-down button (the bar, Now playing, the mini player): the current track is not
/// for me or, when the button shows it is already, no longer.
pub fn start(ctx: &Rc<Ctx>) {
    if ctx.needs_account(tr!("Sign in to rate songs")) {
        return;
    }
    let Some(state) = ctx.player() else { return };
    let Some(mut track) = state.current else {
        return;
    };
    // The rating as the button shows it: one just made is there at once, before its write
    // comes back to the controller's state.
    if let Some(ui) = ctx.window() {
        let p = ui.global::<Player>();
        if p.get_track_id().as_str() == track.id.0 {
            track.like = if p.get_disliked() {
                LikeStatus::Dislike
            } else if p.get_liked() {
                LikeStatus::Like
            } else {
                LikeStatus::Indifferent
            };
        }
    }
    match press(&track, bar::position().unwrap_or(state.position)) {
        Press::Mark(now, undo) => mark_with_undo(ctx, now, undo),
        Press::Unmark(rate) => send(ctx, vec![rate]),
    }
}

/// The mini player's thumbs-down: a toggle as the bar's; its first press goes the tray's way
/// (the main window may be hidden behind it, with nobody to see the toast).
pub fn from_mini(ctx: &Rc<Ctx>) {
    let shown = ctx.window().is_some_and(|ui| {
        let p = ui.global::<Player>();
        p.get_has_track() && p.get_disliked()
    });
    if shown {
        start(ctx);
    } else {
        from_tray(ctx);
    }
}

/// "Not for me" in the tray icon's menu: as the bar's first press while the window is on
/// screen, toast and Undo included; hidden in the tray or minimized, nobody would see the
/// toast, so the song is just rated and skipped. Never a toggle: the menu doesn't show the
/// rating, so it always sets the song aside.
pub fn from_tray(ctx: &Rc<Ctx>) {
    let on_screen = ctx
        .window()
        .is_some_and(|ui| !ui.global::<Frame>().get_hidden() && !ui.window().is_minimized());
    if on_screen {
        if ctx.needs_account(tr!("Sign in to rate songs")) {
            return;
        }
        let Some(state) = ctx.player() else { return };
        let Some(track) = state.current else { return };
        let (now, undo) = actions(&track, bar::position().unwrap_or(state.position));
        mark_with_undo(ctx, now, undo);
        return;
    }
    // Hidden, nobody would see the toast asking to sign in.
    if ctx.signed_out() {
        return;
    }
    let Some(state) = ctx.player() else { return };
    let Some(track) = state.current else { return };
    let (now, _) = actions(&track, bar::position().unwrap_or(state.position));
    send(ctx, now);
}

/// Sends the first press's actions, then the toast with Undo.
fn mark_with_undo(ctx: &Rc<Ctx>, now: Vec<Action>, undo: Undo) {
    send(ctx, now);
    toast::show_with(ctx, tr!("Marked as not for me"), tr!("Undo"), move |c| {
        send(c, undo.actions())
    });
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
