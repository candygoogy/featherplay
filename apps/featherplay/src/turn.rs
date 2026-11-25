//! A song's colours, art and title at a crossfade: they stay the song being left's while it's
//! the louder, then turn to the new song's over `TURN`, centred on the crossing, the
//! crossfade's middle, where the two songs are equally loud (the fade is equal power). A
//! shorter fade ("Fade on skip", a crossfade that starts late) turns over its whole length. A
//! cut (Next, Previous, a click without "Fade on skip") isn't a crossfade: it shows at once,
//! as before. Nothing moves before the crossfade starts or after it ends.
//!
//! Meanwhile the window shows the last song's state: `arrived` holds the new one's, and shows
//! the last one's again with its position running on. The new cover's colours, its field and
//! its art are worked out ahead (`follow::prepare`). From the turn's start one timer runs it,
//! a frame at a time: the cover colours (`follow::turn_step`) and the field under Now playing,
//! the bar and Home's band (`backdrop::turn_step`) move `ease` of the way to the new song's,
//! and at its middle the new song's state shows (the title, the art, the lyrics, Up next),
//! the inks switching with it, once. A pause, a seek, another song or the window hidden while
//! it holds or turns shows the latest state at once, as a cut would.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_core::animations::Instant;
use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::Ctx;
use crate::controller::PlayerState;

/// How long the colours take to turn: long enough not to read as a cut on far-apart colours,
/// short enough to stay one moment (the board of 2026-11-25).
pub const TURN: Duration = Duration::from_millis(1200);
/// About one frame at 60 Hz: the cover colours (`follow-*`) move every frame.
const FRAME: Duration = Duration::from_millis(16);
/// The field moves about 20 times a second, every third frame: each of its steps repaints
/// the whole window (~13 ms at 4K, `crossfade_field`), a single colour only its marks.
const FIELD_STEP: Duration = Duration::from_millis(45);

/// When a crossfade's turn runs, from the crossfade's start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The colours start to move.
    pub start: Duration,
    /// The crossing: the new song's state shows, and the inks switch.
    pub mid: Duration,
    /// The colours are the new song's.
    pub end: Duration,
}

/// The turn for a crossfade of `fade`: `TURN` centred on its middle, or the whole fade if
/// that's shorter.
pub fn plan(fade: Duration) -> Plan {
    let turn = TURN.min(fade);
    let mid = fade / 2;
    let start = mid.saturating_sub(turn / 2);
    Plan {
        start,
        mid,
        end: start + turn,
    }
}

/// Ease in and out (a half sine): the turn starts and lands softly.
pub fn ease(t: f32) -> f32 {
    0.5 - 0.5 * (std::f32::consts::PI * t.clamp(0.0, 1.0)).cos()
}

/// Where a turn is, some time after its crossfade started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct At {
    /// How far the colours are towards the new song's (eased), once they've started.
    pub amount: Option<f32>,
    /// The new song's state shows.
    pub show_new: bool,
    /// The turn is over.
    pub done: bool,
}

pub fn at(plan: Plan, elapsed: Duration) -> At {
    let span = plan.end.saturating_sub(plan.start).as_secs_f32();
    let amount = (elapsed >= plan.start).then(|| {
        if span <= 0.0 {
            1.0
        } else {
            ease((elapsed - plan.start).as_secs_f32() / span)
        }
    });
    At {
        amount,
        show_new: elapsed >= plan.mid,
        done: elapsed >= plan.end,
    }
}

/// What a state the controller sent means for the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrival {
    /// Show it, as ever.
    Show,
    /// A new song coming in by a crossfade: hold it, the last one's showing on.
    Hold,
    /// The held song's state again (its position, its sound): it stays held.
    Keep,
    /// Something else happened while a song was held (a pause, a seek, another song): show
    /// this now, and the turn ends.
    Land,
}

fn same_song(a: &PlayerState, b: &PlayerState) -> bool {
    match (&a.current, &b.current) {
        (Some(a), Some(b)) => a.id == b.id,
        (None, None) => true,
        _ => false,
    }
}

/// What `new` means: `shown` is the state the window shows, `held` the one it holds, if any.
pub fn arrival(
    shown: Option<&PlayerState>,
    held: Option<&PlayerState>,
    new: &PlayerState,
) -> Arrival {
    let fading = new.crossfade.is_some() && new.playing;
    if let Some(held) = held {
        return if fading && same_song(held, new) {
            Arrival::Keep
        } else {
            Arrival::Land
        };
    }
    match shown {
        Some(shown) if fading && shown.current.is_some() && !same_song(shown, new) => Arrival::Hold,
        _ => Arrival::Show,
    }
}

/// The last song's state as it shows while the new one is held, `elapsed` after the hold
/// began: its position running on at the speed (to its end at most), the sound's and the
/// buttons' state the new one's.
pub fn held(last: &PlayerState, new: &PlayerState, elapsed: Duration) -> PlayerState {
    let ran = elapsed.mul_f32(last.speed.max(0.0));
    let end = last
        .current
        .as_ref()
        .and_then(|t| t.duration)
        .unwrap_or(Duration::MAX);
    PlayerState {
        position: (last.position + ran).min(end),
        playing: new.playing,
        audible: new.audible,
        loading: false,
        volume: new.volume,
        shuffle: new.shuffle,
        repeat: new.repeat,
        crossfade: None,
        ..last.clone()
    }
}

/// A crossfade on its way.
struct Hold {
    /// The last song's state, as it was when the crossfade started.
    last: PlayerState,
    /// The new song's, as the controller sent it last.
    new: PlayerState,
    plan: Plan,
    /// When the crossfade started, on the animation clock (which the offscreen tests step).
    began: Instant,
    /// The turn's first frame has run: the colours and the field started to move, if the
    /// new cover's were known by then.
    begun: bool,
    /// The new song's state shows.
    shown_new: bool,
    /// When the field last moved, from the crossfade's start.
    field_at: Option<Duration>,
}

thread_local! {
    static HOLD: RefCell<Option<Hold>> = const { RefCell::new(None) };
    static TIMER: Timer = Timer::default();
}

fn now() -> Instant {
    i_slint_core::animations::current_tick()
}

/// A crossfade's turn is on its way (held, or turning).
pub fn running() -> bool {
    HOLD.with(|h| h.borrow().is_some())
}

/// The controller sent `state`: what the window shows now, if anything (`app::on_event`).
pub fn arrived(ctx: &Rc<Ctx>, state: PlayerState) -> Option<PlayerState> {
    let shown = ctx.player();
    let on_screen = ctx.window().is_some_and(|ui| {
        let w = ui.window();
        (w.is_visible() && !w.is_minimized()) || crate::mini::is_open()
    });
    let what = HOLD.with(|h| {
        let h = h.borrow();
        arrival(shown.as_ref(), h.as_ref().map(|h| &h.new), &state)
    });
    match what {
        Arrival::Show => Some(state),
        Arrival::Hold if !on_screen => Some(state),
        Arrival::Hold => {
            let (Some(last), Some(fade)) = (shown, state.crossfade) else {
                return Some(state);
            };
            let plan = plan(fade);
            if let Some(url) = cover_url(&state) {
                crate::follow::prepare(ctx, &url);
                crate::pages::now_playing::prefetch_cover(ctx, &url);
            }
            let began = now();
            HOLD.with(|h| {
                *h.borrow_mut() = Some(Hold {
                    last: last.clone(),
                    new: state.clone(),
                    plan,
                    began,
                    begun: false,
                    shown_new: false,
                    field_at: None,
                })
            });
            schedule(ctx, plan.start);
            Some(held(&last, &state, Duration::ZERO))
        }
        Arrival::Keep => HOLD.with(|h| {
            let mut h = h.borrow_mut();
            let hold = h.as_mut()?;
            hold.new = state;
            // Once the new song shows, its own state does.
            Some(if hold.shown_new {
                hold.new.clone()
            } else {
                held(&hold.last, &hold.new, now() - hold.began)
            })
        }),
        Arrival::Land => {
            end(ctx);
            Some(state)
        }
    }
}

/// The cover the state's song shows.
fn cover_url(state: &PlayerState) -> Option<String> {
    state
        .current
        .as_ref()
        .and_then(|t| t.thumbnail.clone())
        .filter(|u| !u.is_empty())
}

/// Runs `step` once the crossfade has run `from` (a frame at a time from then on).
fn schedule(ctx: &Rc<Ctx>, from: Duration) {
    let wait = HOLD.with(|h| {
        h.borrow()
            .as_ref()
            .map(|h| from.saturating_sub(now() - h.began))
    });
    let Some(wait) = wait else { return };
    let c = ctx.clone();
    TIMER.with(|t| {
        t.start(TimerMode::SingleShot, wait, move || {
            let c = c.clone();
            let each = c.clone();
            TIMER.with(|t| t.start(TimerMode::Repeated, FRAME, move || step(&each)));
            step(&c);
        })
    });
}

/// One frame of the turn.
fn step(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let Some((plan, began, begun, shown_new, field_at, url)) = HOLD.with(|h| {
        let mut h = h.borrow_mut();
        let h = h.as_mut()?;
        let begun = std::mem::replace(&mut h.begun, true);
        Some((
            h.plan,
            h.began,
            begun,
            h.shown_new,
            h.field_at,
            cover_url(&h.new),
        ))
    }) else {
        TIMER.with(Timer::stop);
        return;
    };
    let elapsed = now() - began;
    let at = at(plan, elapsed);
    let Some(amount) = at.amount else { return };
    // The first frame: the colours and the field start, if the new cover's are known by now;
    // if not, they land with the new song's state at the crossing, as on a cut.
    if !begun && let Some(url) = url.as_deref() {
        crate::follow::turn_begin(ctx, url);
        crate::pages::now_playing::backdrop_turn_begin(ctx, url);
    }
    if at.show_new && !shown_new {
        show_new(ctx);
    }
    crate::follow::turn_step(&ui, amount);
    if field_due(field_at, elapsed, at.done) {
        HOLD.with(|h| {
            if let Some(h) = h.borrow_mut().as_mut() {
                h.field_at = Some(elapsed);
            }
        });
        crate::pages::now_playing::backdrop_turn_step(ctx, amount);
    }
    if at.done {
        finish(ctx);
    }
}

/// Whether the field moves in a frame `elapsed` into the crossfade: its first, then
/// `FIELD_STEP` after the last, and the last.
pub fn field_due(last: Option<Duration>, elapsed: Duration, done: bool) -> bool {
    done || last.is_none_or(|last| elapsed >= last + FIELD_STEP)
}

/// The colours that ease in Slint (`Theme.t-turn`: Home's band, the lyrics' panel, the
/// desktop lyrics' strip) take the turn's length while it's on, from the crossing.
fn turning(ctx: &Rc<Ctx>, on: bool) {
    if let Some(ui) = ctx.window() {
        ui.global::<crate::ui::Theme>().set_turning(on);
    }
    crate::desktop_lyrics::set_turning(on);
}

/// The crossing: the new song's state shows.
fn show_new(ctx: &Rc<Ctx>) {
    let new = HOLD.with(|h| {
        let mut h = h.borrow_mut();
        let h = h.as_mut()?;
        h.shown_new = true;
        Some(h.new.clone())
    });
    if let Some(new) = new {
        // Its colours that ease in Slint ease over a turn's length; then back to a cut's.
        turning(ctx, true);
        crate::app::show_player(ctx, new);
        let c = ctx.clone();
        Timer::single_shot(TURN, move || turning(&c, false));
    }
}

/// The turn is over: everything is the new song's.
fn finish(ctx: &Rc<Ctx>) {
    TIMER.with(Timer::stop);
    let hold = HOLD.with(|h| h.borrow_mut().take());
    let Some(hold) = hold else { return };
    if !hold.shown_new {
        crate::app::show_player(ctx, hold.new);
    }
    crate::follow::turn_end(ctx);
    crate::pages::now_playing::backdrop_turn_end(ctx);
}

/// Something changed under a turn on its way (the theme): the new song's state shows now,
/// and its colours and field land.
pub fn interrupt(ctx: &Rc<Ctx>) {
    if running() {
        finish(ctx);
    }
}

/// Ends a turn at once (another state came): the colours and the field land where they were
/// going, and the state that came shows next, as on a cut.
fn end(ctx: &Rc<Ctx>) {
    TIMER.with(Timer::stop);
    if HOLD.with(|h| h.borrow_mut().take()).is_none() {
        return;
    }
    crate::follow::turn_end(ctx);
    crate::pages::now_playing::backdrop_turn_end(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use featherplay_core::model::{Track, TrackId};

    const S: fn(u64) -> Duration = Duration::from_secs;
    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn song(id: &str) -> Track {
        Track {
            id: TrackId(id.into()),
            title: id.into(),
            artists: Vec::new(),
            album: None,
            duration: Some(S(200)),
            video_type: featherplay_core::model::VideoType::Song,
            thumbnail: None,
            like: featherplay_core::model::LikeStatus::Indifferent,
            counterpart: None,
        }
    }

    fn state(id: &str, crossfade: Option<Duration>, playing: bool) -> PlayerState {
        PlayerState {
            current: Some(song(id)),
            playing,
            audible: playing,
            loading: false,
            position: Duration::ZERO,
            upcoming: Vec::new().into(),
            shuffle: false,
            repeat: Default::default(),
            radio: false,
            volume: 1.0,
            queue: Default::default(),
            crossfade,
            speed: 1.0,
        }
    }

    #[test]
    fn the_turn_is_centred_on_the_crossing() {
        assert_eq!(
            plan(S(12)),
            Plan {
                start: MS(5400),
                mid: S(6),
                end: MS(6600)
            }
        );
        assert_eq!(
            plan(S(5)),
            Plan {
                start: MS(1900),
                mid: MS(2500),
                end: MS(3100)
            }
        );
    }

    #[test]
    fn a_short_fade_turns_inside_it() {
        // "Fade on skip": two seconds at most.
        assert_eq!(
            plan(S(2)),
            Plan {
                start: MS(400),
                mid: S(1),
                end: MS(1600)
            }
        );
        // The shortest crossfade, one that starts late: the turn is the whole fade.
        assert_eq!(
            plan(S(1)),
            Plan {
                start: S(0),
                mid: MS(500),
                end: S(1)
            }
        );
        for fade in [MS(300), S(1), MS(1500), S(2), S(5), S(8), S(12)] {
            let p = plan(fade);
            assert!(
                p.end <= fade && p.start <= p.mid && p.mid <= p.end,
                "{fade:?}: {p:?}"
            );
        }
    }

    #[test]
    fn colours_hold_until_the_turn_then_cross_half_way_at_the_crossing() {
        let p = plan(S(12));
        for t in [S(0), S(3), MS(5399)] {
            assert_eq!(
                at(p, t),
                At {
                    amount: None,
                    show_new: false,
                    done: false
                },
                "{t:?}"
            );
        }
        let start = at(p, MS(5400));
        assert_eq!(start.amount, Some(0.0));
        let mid = at(p, S(6));
        assert!((mid.amount.unwrap() - 0.5).abs() < 1e-4 && mid.show_new && !mid.done);
        assert!(!at(p, MS(5999)).show_new);
        let end = at(p, MS(6600));
        assert!((end.amount.unwrap() - 1.0).abs() < 1e-6 && end.done);
        // Eased: slow at both ends, never back.
        let steps: Vec<f32> = (0..=60)
            .map(|i| at(p, MS(5400 + i * 20)).amount.unwrap())
            .collect();
        assert!(steps.windows(2).all(|w| w[1] >= w[0]));
        assert!(steps[1] - steps[0] < steps[31] - steps[30]);
    }

    #[test]
    fn the_field_moves_about_twenty_times_a_second() {
        let mut last = None;
        let mut moved = Vec::new();
        for frame in 0..=75u64 {
            let elapsed = MS(frame * 16);
            let done = frame == 75;
            if field_due(last, elapsed, done) {
                last = Some(elapsed);
                moved.push(frame);
            }
        }
        // Every third frame of 60 Hz from the first, and the last.
        assert_eq!(moved.first(), Some(&0));
        assert_eq!(moved.last(), Some(&75));
        assert!(moved.windows(2).all(|w| w[1] - w[0] == 3), "{moved:?}");
        assert_eq!(moved.len(), 26);
    }

    #[test]
    fn only_a_crossfade_into_another_song_holds() {
        let a = state("a", None, true);
        // A cut: shown at once.
        assert_eq!(
            arrival(Some(&a), None, &state("b", None, true)),
            Arrival::Show
        );
        // A crossfade.
        assert_eq!(
            arrival(Some(&a), None, &state("b", Some(S(5)), true)),
            Arrival::Hold
        );
        // Paused as it comes in, or the same song (a state resent): shown.
        assert_eq!(
            arrival(Some(&a), None, &state("b", Some(S(5)), false)),
            Arrival::Show
        );
        assert_eq!(
            arrival(Some(&a), None, &state("a", Some(S(5)), true)),
            Arrival::Show
        );
        // Nothing was showing: shown.
        assert_eq!(
            arrival(None, None, &state("b", Some(S(5)), true)),
            Arrival::Show
        );
    }

    #[test]
    fn a_pause_a_seek_or_another_song_ends_the_hold() {
        let a = state("a", None, true);
        let b = state("b", Some(S(12)), true);
        // Its position moving on: still held.
        assert_eq!(arrival(Some(&a), Some(&b), &b), Arrival::Keep);
        // Paused.
        assert_eq!(
            arrival(Some(&a), Some(&b), &state("b", Some(S(12)), false)),
            Arrival::Land
        );
        // A seek (the controller ends the crossfade's record).
        assert_eq!(
            arrival(Some(&a), Some(&b), &state("b", None, true)),
            Arrival::Land
        );
        // Next: another song, cut or crossfaded.
        assert_eq!(
            arrival(Some(&a), Some(&b), &state("c", None, true)),
            Arrival::Land
        );
        assert_eq!(
            arrival(Some(&a), Some(&b), &state("c", Some(S(2)), true)),
            Arrival::Land
        );
    }

    #[test]
    fn the_last_song_runs_on_while_held() {
        let mut a = state("a", None, true);
        a.position = S(190);
        let b = state("b", Some(S(12)), true);
        let shown = held(&a, &b, MS(2500));
        assert_eq!(shown.current, a.current);
        assert_eq!(shown.position, MS(192_500));
        // Never past its end.
        assert_eq!(held(&a, &b, S(30)).position, S(200));
        // At double speed, twice as far.
        a.speed = 2.0;
        assert_eq!(held(&a, &b, S(1)).position, S(192));
    }
}
