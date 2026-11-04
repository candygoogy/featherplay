//! The album page's ⋯ menu (ui/album_menu.slint): Like all songs, Remove likes, Dislike all
//! songs, Remove dislikes. Only the songs that need it get a write: Like all skips the songs
//! already liked, Remove likes touches only the liked ones, and so on; when none do, a toast
//! says so and nothing is sent. Each action asks first.
//!
//! Then a job rates the songs in the album's order, one write every half second (`GAP`), and
//! stops at the first that fails. One job runs at a time, and leaving the page doesn't stop
//! it. A toast shows how far it has got, then how it ended. Each song's heart changes as its
//! write goes through, as a single like's does (`song_menu::show_rating`), and the controller
//! hears of it for the bar and the queue (`Action::Rated`).
//!
//! The writes go through `Rater`: the account's in the app (`Source::rate`), a fake in tests.

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use featherplay_core::model::{LikeStatus, Track, TrackId};
use featherplay_core::{tr, trn};
use featherplay_ytm::ApiError;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::busy::Busy;
use crate::controller::Action;
use crate::pages::album;
use crate::song_menu;
use crate::ui::{AlbumAction, AlbumData, AlbumMenu};

/// Between two writes, so a long album isn't a burst of requests.
const GAP: Duration = Duration::from_millis(500);

/// What a job needs from outside: one song's write, and the wait before the next.
pub trait Rater {
    fn rate(&self, id: &TrackId, status: LikeStatus) -> impl Future<Output = Result<(), ApiError>>;
    fn pause(&self) -> impl Future<Output = ()>;
}

/// The account's: each write runs on the runtime with the data source of the moment (so a
/// sign-out mid-job stops it), and the wait is a timer on the UI thread.
struct Account(Rc<Ctx>);

impl Rater for Account {
    async fn rate(&self, id: &TrackId, status: LikeStatus) -> Result<(), ApiError> {
        let source = self.0.catalog.source();
        let id = id.clone();
        self.0
            .catalog
            .runtime()
            .spawn(async move { source.rate(&id, status).await })
            .await
            .unwrap_or_else(|e| Err(ApiError::Network(e.to_string())))
    }

    async fn pause(&self) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        slint::Timer::single_shot(GAP, move || {
            let _ = tx.send(());
        });
        let _ = rx.await;
    }
}

/// What the confirmation asks about.
struct Job {
    action: AlbumAction,
    ids: Vec<TrackId>,
}

thread_local! {
    static ASKED: RefCell<Option<Job>> = const { RefCell::new(None) };
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let menu = ui.global::<AlbumMenu>();
    let c = ctx.clone();
    menu.on_pick(move |action| pick(&c, action));
    let c = ctx.clone();
    menu.on_confirm(move || confirm_with(&c, Account(c.clone())));
    let c = ctx.clone();
    menu.on_cancel(move || cancel(&c));
}

/// What `action` sets a song to.
fn target(action: AlbumAction) -> LikeStatus {
    match action {
        AlbumAction::LikeAll => LikeStatus::Like,
        AlbumAction::DislikeAll => LikeStatus::Dislike,
        AlbumAction::RemoveLikes | AlbumAction::RemoveDislikes => LikeStatus::Indifferent,
    }
}

/// Whether a song rated `now` needs a write for `action`.
fn needs(action: AlbumAction, now: LikeStatus) -> bool {
    match action {
        AlbumAction::LikeAll => now != LikeStatus::Like,
        AlbumAction::RemoveLikes => now == LikeStatus::Like,
        AlbumAction::DislikeAll => now != LikeStatus::Dislike,
        AlbumAction::RemoveDislikes => now == LikeStatus::Dislike,
    }
}

/// A song's rating as the app knows it now: the controller's for a song in the queue (the
/// bar's heart rates it there), else the song menu's (the rows' hearts), else as loaded.
fn rating_now(ctx: &Ctx, track: &Track) -> LikeStatus {
    let queued = ctx.player().and_then(|s| {
        s.current
            .iter()
            .chain(&s.upcoming)
            .find(|q| q.id == track.id)
            .map(|q| q.like)
    });
    queued
        .or_else(|| song_menu::like_of(&track.id))
        .unwrap_or(track.like)
}

/// A menu row: works out the songs, then asks, or says there's nothing to do.
fn pick(ctx: &Ctx, action: AlbumAction) {
    let Some(ui) = ctx.window() else { return };
    let menu = ui.global::<AlbumMenu>();
    if menu.get_busy() {
        return;
    }
    let ids: Vec<TrackId> = album::tracks()
        .iter()
        .filter(|t| needs(action, rating_now(ctx, t)))
        .map(|t| t.id.clone())
        .collect();
    if ids.is_empty() {
        ctx.toast(none_text(action));
        return;
    }
    let title = ui.global::<AlbumData>().get_title();
    menu.set_question(question(action, ids.len(), &title).into());
    menu.set_ok_label(ok_label(action).into());
    ASKED.with(|a| *a.borrow_mut() = Some(Job { action, ids }));
    menu.set_dialog(true);
}

fn cancel(ctx: &Ctx) {
    ASKED.with(|a| a.borrow_mut().take());
    if let Some(ui) = ctx.window() {
        ui.global::<AlbumMenu>().set_dialog(false);
    }
}

/// The confirmation's button: starts the job asked about, with `rater`'s writes.
pub fn confirm_with(ctx: &Rc<Ctx>, rater: impl Rater + 'static) {
    let Some(ui) = ctx.window() else { return };
    let menu = ui.global::<AlbumMenu>();
    menu.set_dialog(false);
    let Some(job) = ASKED.with(|a| a.borrow_mut().take()) else {
        return;
    };
    if menu.get_busy() {
        return;
    }
    menu.set_busy(true);
    // `--snapshot` and the tests wait for it as for a load.
    let busy = Busy::start();
    let c = ctx.clone();
    let spawned = slint::spawn_local(async move {
        let said = run(&c, &job, &rater).await;
        c.toast(&said);
        if let Some(ui) = c.window() {
            ui.global::<AlbumMenu>().set_busy(false);
        }
        drop(busy);
    });
    if let Err(e) = spawned {
        log!("album menu: {e}");
        menu.set_busy(false);
    }
}

/// Rates the job's songs in order, stopping at the first write that fails, and says how it
/// went.
async fn run(ctx: &Ctx, job: &Job, rater: &impl Rater) -> String {
    let status = target(job.action);
    let total = job.ids.len();
    log!("album menu: rating {total} songs {status:?}");
    for (done, id) in job.ids.iter().enumerate() {
        if done > 0 {
            rater.pause().await;
        }
        ctx.toast(&progress_text(job.action, done + 1, total));
        if let Err(e) = rater.rate(id, status).await {
            log!(
                "album menu: rating {} failed after {done} of {total}: {e}",
                id.0
            );
            let why = reason(&e);
            return if done == 0 {
                tr!("Nothing changed: {}", why)
            } else {
                tr!("Stopped after {} of {}: {}", done, total, why)
            };
        }
        song_menu::show_rating(ctx, id, status);
        ctx.send(Action::Rated(id.clone(), status));
    }
    done_text(job.action, total)
}

/// `“Name”`, as the dialogs quote a title.
fn quoted(name: &str) -> String {
    format!("\u{201C}{name}\u{201D}")
}

fn question(action: AlbumAction, n: usize, album: &str) -> String {
    let album = quoted(album);
    match action {
        AlbumAction::LikeAll => trn!("Like {n} song on {}?" | "Like {n} songs on {}?" % n, album),
        AlbumAction::RemoveLikes => trn!(
            "Remove the like from {n} song on {}?" | "Remove likes from {n} songs on {}?" % n,
            album
        ),
        AlbumAction::DislikeAll => {
            trn!(
                "Dislike {n} song on {}?" | "Dislike {n} songs on {}?" % n,
                album
            )
        }
        AlbumAction::RemoveDislikes => trn!(
            "Remove the dislike from {n} song on {}?" | "Remove dislikes from {n} songs on {}?" % n,
            album
        ),
    }
}

fn ok_label(action: AlbumAction) -> &'static str {
    match action {
        AlbumAction::LikeAll => tr!("Like"),
        AlbumAction::RemoveLikes => tr!("Remove likes"),
        AlbumAction::DislikeAll => tr!("Dislike"),
        AlbumAction::RemoveDislikes => tr!("Remove dislikes"),
    }
}

fn none_text(action: AlbumAction) -> &'static str {
    match action {
        AlbumAction::LikeAll => tr!("Every song on this album is already liked"),
        AlbumAction::RemoveLikes => tr!("No liked songs on this album"),
        AlbumAction::DislikeAll => tr!("Every song on this album is already disliked"),
        AlbumAction::RemoveDislikes => tr!("No disliked songs on this album"),
    }
}

/// While the `at`th write of `total` is on its way.
fn progress_text(action: AlbumAction, at: usize, total: usize) -> String {
    match action {
        AlbumAction::LikeAll => tr!("Liking songs… {} of {}", at, total),
        AlbumAction::RemoveLikes => tr!("Removing likes… {} of {}", at, total),
        AlbumAction::DislikeAll => tr!("Disliking songs… {} of {}", at, total),
        AlbumAction::RemoveDislikes => tr!("Removing dislikes… {} of {}", at, total),
    }
}

fn done_text(action: AlbumAction, n: usize) -> String {
    match action {
        AlbumAction::LikeAll => trn!("Liked {n} song" | "Liked {n} songs" % n),
        AlbumAction::RemoveLikes => trn!("Removed {n} like" | "Removed {n} likes" % n),
        AlbumAction::DislikeAll => trn!("Disliked {n} song" | "Disliked {n} songs" % n),
        AlbumAction::RemoveDislikes => {
            trn!("Removed {n} dislike" | "Removed {n} dislikes" % n)
        }
    }
}

/// Why a write failed, short enough to follow "Stopped after 4 of 12: ".
fn reason(e: &ApiError) -> &'static str {
    match e {
        ApiError::AuthExpired => tr!("sign in again"),
        ApiError::Network(_) => tr!("no connection"),
        ApiError::NotFound => tr!("a song wasn't found"),
        ApiError::Parse(_) => tr!("the music service said no"),
    }
}

/// `--snapshot album-actions`: the menu open below the header's ⋯; with `confirm`, Like all
/// songs asking first.
pub fn show_for_snapshot(ctx: &Ctx, confirm: bool) {
    let Some(ui) = ctx.window() else { return };
    let menu = ui.global::<AlbumMenu>();
    if confirm {
        pick(ctx, AlbumAction::LikeAll);
        return;
    }
    // The header rides in the list's first row, which the list makes as it lays out for a
    // frame: one first, so the ⋯ is there to open its menu.
    if let Err(e) = ui.window().take_snapshot() {
        log!("album menu: snapshot: {e}");
    }
    menu.set_open_asked(menu.get_open_asked() + 1);
}
