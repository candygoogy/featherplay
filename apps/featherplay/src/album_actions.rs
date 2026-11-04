//! The album page's ⋯ menu (ui/album_menu.slint): Like all songs, Remove likes, Dislike all
//! songs, Remove dislikes. Only the songs that need it get a write: Like all skips the songs
//! already liked, Remove likes touches only the liked ones, and so on; when none do, a toast
//! says so and nothing is sent. Each action asks first.
//!
//! Then a job rates the songs, starting the writes in the album's order with up to three on
//! their way at once (`AT_ONCE`): the next starts as soon as one ends. After the first that
//! fails no new write starts, and the ones on their way finish. One job runs at a time, and
//! leaving the page doesn't stop it. A toast counts the songs done, then says how it ended.
//! Each song's heart changes as its write goes through, as a single like's does
//! (`song_menu::show_rating`), and the controller hears of it for the bar and the queue
//! (`Action::Rated`).
//!
//! The writes go through `Rater`: the account's in the app (`Source::rate`), a fake in tests.

use std::cell::RefCell;
use std::future::{self, Future};
use std::pin::Pin;
use std::rc::Rc;
use std::task::Poll;

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

/// The most writes on their way at once: an album's worth goes out a few at a time, never
/// as one burst.
const AT_ONCE: usize = 3;

/// What a job needs from outside: one song's write.
pub trait Rater {
    /// Starts the write at once, not when the future is first polled, so the job knows
    /// what it has sent; the future is its outcome.
    fn rate(&self, id: &TrackId, status: LikeStatus) -> impl Future<Output = Result<(), ApiError>>;
}

/// The account's: each write runs on the runtime with the data source of the moment (so a
/// sign-out mid-job stops it).
struct Account(Rc<Ctx>);

impl Rater for Account {
    fn rate(&self, id: &TrackId, status: LikeStatus) -> impl Future<Output = Result<(), ApiError>> {
        let source = self.0.catalog.source();
        let id = id.clone();
        let write = self
            .0
            .catalog
            .runtime()
            .spawn(async move { source.rate(&id, status).await });
        async move {
            write
                .await
                .unwrap_or_else(|e| Err(ApiError::Network(e.to_string())))
        }
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

/// Rates the job's songs, starting the writes in order with up to `AT_ONCE` on their way.
/// After the first that fails it starts no more and lets the others finish; then it says
/// how it went.
async fn run(ctx: &Ctx, job: &Job, rater: &impl Rater) -> String {
    let status = target(job.action);
    let total = job.ids.len();
    log!("album menu: rating {total} songs {status:?}");
    ctx.toast(&progress_text(job.action, 0, total));
    let mut waiting = job.ids.iter();
    let mut flying = Vec::with_capacity(AT_ONCE);
    let mut done = 0;
    let mut failed = None;
    loop {
        while failed.is_none() && flying.len() < AT_ONCE {
            let Some(id) = waiting.next() else { break };
            flying.push((id, Box::pin(rater.rate(id, status))));
        }
        if flying.is_empty() {
            break;
        }
        let (id, outcome) = next_to_end(&mut flying).await;
        match outcome {
            Ok(()) => {
                done += 1;
                song_menu::show_rating(ctx, id, status);
                ctx.send(Action::Rated(id.clone(), status));
                if failed.is_none() {
                    ctx.toast(&progress_text(job.action, done, total));
                }
            }
            Err(e) => {
                log!("album menu: rating {} failed: {e}", id.0);
                failed.get_or_insert(e);
            }
        }
    }
    let Some(e) = failed else {
        return done_text(job.action, total);
    };
    log!("album menu: stopped after {done} of {total}");
    let why = reason(&e);
    if done == 0 {
        tr!("Nothing changed: {}", why)
    } else {
        tr!("Stopped after {} of {}: {}", done, total, why)
    }
}

/// Waits for the first of the writes on their way to end, and takes it out of `flying`.
async fn next_to_end<'id, F: Future>(
    flying: &mut Vec<(&'id TrackId, Pin<Box<F>>)>,
) -> (&'id TrackId, F::Output) {
    future::poll_fn(|cx| {
        let ended = flying.iter_mut().enumerate().find_map(|(i, (_, write))| {
            match write.as_mut().poll(cx) {
                Poll::Ready(outcome) => Some((i, outcome)),
                Poll::Pending => None,
            }
        });
        match ended {
            Some((i, outcome)) => Poll::Ready((flying.remove(i).0, outcome)),
            None => Poll::Pending,
        }
    })
    .await
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

/// While the job runs, with `done` of its `total` songs rated so far.
fn progress_text(action: AlbumAction, done: usize, total: usize) -> String {
    match action {
        AlbumAction::LikeAll => tr!("Liking songs… {} of {}", done, total),
        AlbumAction::RemoveLikes => tr!("Removing likes… {} of {}", done, total),
        AlbumAction::DislikeAll => tr!("Disliking songs… {} of {}", done, total),
        AlbumAction::RemoveDislikes => tr!("Removing dislikes… {} of {}", done, total),
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
