//! The bottom bar: the `Player` global from the controller's state, the transport sent
//! back as `Action`s and `UiCommand`-equivalent inputs, and the integrations (media card,
//! taskbar, tray) fed from the same state.
//!
//! The position advances on a timer aligned to whole seconds of playback, so the bar
//! redraws once a second while sound plays and never while paused. While the window is
//! minimized the timer keeps time but leaves the properties alone. The tray's mini player,
//! when open, follows the same state and tick (`mini`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay_core::model::{LikeStatus, RepeatMode, TrackId};
use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::Ctx;
use crate::controller::{Action, PlayerState, now_playing};
use crate::ui::{Player, Repeat};
use crate::ui_bridge::{artist_names, format_duration};
use crate::{mini, not_for_me, shell};

/// The cover size in the bar, in logical pixels.
const BAR_COVER: u32 = 48;

#[derive(Default)]
struct Clock {
    /// Position at `since`, and whether it advances (sound is playing).
    at: Cell<Duration>,
    since: Cell<Option<Instant>>,
    duration: Cell<Option<Duration>>,
    has_track: Cell<bool>,
    timer: Timer,
    cover_url: RefCell<String>,
}

impl Clock {
    fn now(&self) -> Duration {
        let at = self.at.get() + self.since.get().map(|s| s.elapsed()).unwrap_or_default();
        match self.duration.get() {
            Some(d) => at.min(d),
            None => at,
        }
    }
}

thread_local! {
    static CLOCK: Rc<Clock> = Rc::new(Clock::default());
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let player = ui.global::<Player>();
    let c = ctx.clone();
    // The same inputs as the media keys, through the controller's player.
    player.on_toggle(move || c.send(Action::Toggle));
    let c = ctx.clone();
    player.on_next(move || c.send(Action::Next));
    let c = ctx.clone();
    player.on_previous(move || c.send(Action::Previous));
    let c = ctx.clone();
    player.on_seek(move |fraction| {
        let Some(d) = CLOCK.with(|k| k.duration.get()) else {
            return;
        };
        let to = d.mul_f64(f64::from(fraction.clamp(0.0, 1.0)));
        // Shown at once; the controller's state follows.
        CLOCK.with(|k| {
            k.at.set(to);
            if k.since.get().is_some() {
                k.since.set(Some(Instant::now()));
            }
        });
        show_position(&c, to);
        c.send(Action::SeekTo(to));
    });
    let c = ctx.clone();
    player.on_set_volume(move |value, save| {
        let value = value.clamp(0.0, 1.0);
        if let Some(ui) = c.window() {
            ui.global::<Player>().set_volume(value);
        }
        c.send(Action::SetVolume { value, save });
    });
    let c = ctx.clone();
    player.on_toggle_like(move || {
        let Some(ui) = c.window() else { return };
        let p = ui.global::<Player>();
        if !p.get_has_track() {
            return;
        }
        let liked = !p.get_liked();
        // Flipped at once; the controller sends the state back (reverted on failure).
        p.set_liked(liked);
        // A like replaces a dislike, and removing one leaves no rating.
        p.set_disliked(false);
        let status = if liked {
            LikeStatus::Like
        } else {
            LikeStatus::Indifferent
        };
        c.send(Action::Rate(TrackId(p.get_track_id().into()), status));
    });
    let c = ctx.clone();
    player.on_not_for_me(move || not_for_me::start(&c));
    let c = ctx.clone();
    player.on_toggle_shuffle(move || {
        let on = c.player().is_some_and(|s| !s.shuffle);
        c.send(Action::SetShuffle(on));
    });
    let c = ctx.clone();
    player.on_cycle_repeat(move || {
        let next = match c.player().map(|s| s.repeat).unwrap_or_default() {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        c.send(Action::SetRepeat(next));
    });
}

/// Shows the controller's state in the bar and on the integrations.
pub fn apply(ctx: &Rc<Ctx>, state: &PlayerState) {
    let Some(ui) = ctx.window() else { return };
    let p = ui.global::<Player>();
    p.set_volume(state.volume);
    p.set_playing(state.playing);
    p.set_loading(state.loading);
    p.set_shuffle(state.shuffle);
    p.set_radio(state.radio);
    p.set_repeat(match state.repeat {
        RepeatMode::Off => Repeat::Off,
        RepeatMode::All => Repeat::All,
        RepeatMode::One => Repeat::One,
    });

    let clock = CLOCK.with(Rc::clone);
    let Some(track) = &state.current else {
        p.set_has_track(false);
        p.set_title("".into());
        p.set_artist("".into());
        p.set_cover(Default::default());
        p.set_disliked(false);
        clock.timer.stop();
        clock.cover_url.borrow_mut().clear();
        clock.has_track.set(false);
        shell::idle();
        shell::set_tray_menu(&p);
        mini::update(ctx);
        return;
    };
    p.set_has_track(true);
    p.set_track_id(track.id.0.as_str().into());
    p.set_title(track.title.as_str().into());
    p.set_artist(artist_names(&track.artists).into());
    p.set_artist_id(
        track
            .artists
            .iter()
            .find_map(|a| a.id.as_ref())
            .map(|id| id.0.as_str())
            .unwrap_or_default()
            .into(),
    );
    p.set_album(
        track
            .album
            .as_ref()
            .map(|a| a.title.as_str())
            .unwrap_or_default()
            .into(),
    );
    p.set_album_id(
        track
            .album
            .as_ref()
            .map(|a| a.id.0.as_str())
            .unwrap_or_default()
            .into(),
    );
    p.set_liked(track.like == LikeStatus::Like);
    p.set_disliked(track.like == LikeStatus::Dislike);
    let duration = track.duration;
    p.set_duration_s(duration.map(|d| d.as_secs_f32()).unwrap_or(0.0));
    p.set_total(duration.map(format_duration).unwrap_or_default().into());

    let url = track.thumbnail.clone().unwrap_or_default();
    if *clock.cover_url.borrow() != url {
        *clock.cover_url.borrow_mut() = url.clone();
        p.set_cover(Default::default());
        p.set_cover_url(url.as_str().into());
        let weak = ui.as_weak();
        let wanted = url.clone();
        ctx.covers.load(&url, BAR_COVER, move |img| {
            let still = CLOCK.with(|k| *k.cover_url.borrow() == wanted);
            if still && let Some(ui) = weak.upgrade() {
                ui.global::<Player>().set_cover(img);
            }
        });
    }

    clock.has_track.set(true);
    clock.at.set(state.position);
    clock.since.set(state.audible.then(Instant::now));
    clock.duration.set(duration);
    show_position(ctx, state.position);
    schedule(ctx);

    shell::apply(now_playing(track, state.playing, state.position));
    shell::set_tray_menu(&p);
    mini::update(ctx);
}

/// Where the current track is now (it advances while sound plays), or `None` with nothing
/// to play.
pub fn position() -> Option<Duration> {
    CLOCK.with(|k| k.has_track.get().then(|| k.now()))
}

/// Sets the position properties (one redraw).
fn show_position(ctx: &Ctx, at: Duration) {
    let Some(ui) = ctx.window() else { return };
    let p = ui.global::<Player>();
    p.set_position_s(at.as_secs_f32());
    p.set_elapsed(format_duration(at).into());
}

/// Ticks at the next whole second of playback while sound plays.
fn schedule(ctx: &Rc<Ctx>) {
    let clock = CLOCK.with(Rc::clone);
    if clock.since.get().is_none() {
        clock.timer.stop();
        return;
    }
    let ms = clock.now().as_millis() % 1000;
    let wait = Duration::from_millis((1000 - ms) as u64 + 5);
    let weak = Rc::downgrade(ctx);
    clock.timer.start(TimerMode::SingleShot, wait, move || {
        let Some(ctx) = weak.upgrade() else { return };
        let minimized = ctx.window().is_some_and(|ui| ui.window().is_minimized());
        let at = CLOCK.with(|k| k.now());
        if !minimized {
            show_position(&ctx, at);
        }
        mini::tick(at);
        crate::memory::tick(&ctx);
        schedule(&ctx);
    });
}
