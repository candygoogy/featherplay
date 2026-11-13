//! The sleep timer (Settings › Playback › Sleep timer, off by default): pauses the music in
//! 15, 30 or 45 minutes or an hour, when the song playing ends, or when the list playing
//! ends (before autoplay carries on). Over the last seconds (the Fade out setting) the
//! volume fades to silence; then playback pauses, and the volume comes back with the next
//! Play, so that one isn't silent. The queue and the position stay as they are.
//!
//! A manual pause doesn't stop the timer: paused when it ends, it just goes away. A pause
//! while it fades ends it then. Quitting drops it; it isn't restored at the next start.
//!
//! One timer: for a time, the loop wakes when the minutes left change, when the fade
//! starts, and at the end (`Controller::sleep_wake`); each `FADE_STEP` while it fades. A
//! song's or list's end follows the position on the ticks that run while it plays, and
//! the song's end itself is the player's (`Input::PauseAtEnd`). Off, nothing runs.

use std::time::{Duration, Instant};

use featherplay_core::player::Input;

use super::{Controller, UiEvent};

/// How often the volume steps down while the music fades out, for a time.
const FADE_STEP: Duration = Duration::from_millis(250);
/// What "+15 min" adds.
const MORE: Duration = Duration::from_secs(15 * 60);
/// The longest a timer runs: "+15 min" stops adding there.
const MOST: Duration = Duration::from_secs(12 * 60 * 60);

/// What the bar's popup and the tray menu ask of the timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SleepAction {
    /// Pause in this many minutes, from now.
    Minutes(u16),
    /// Pause when the song playing ends.
    SongEnd,
    /// Pause when the list playing ends.
    ListEnd,
    /// 15 minutes more than what's left; while it fades, the volume comes back.
    AddQuarter,
    /// Turn it off; while it fades, the volume comes back.
    Off,
}

/// What the timer waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SleepKind {
    Time,
    SongEnd,
    ListEnd,
}

/// The running timer, as the bar and the tray show it. Sent (`UiEvent::Sleep`) when the
/// kind, the minutes or the fade change, so at most once a minute between choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SleepState {
    pub kind: SleepKind,
    /// The minutes left, rounded up: at least 1.
    pub minutes: u32,
    /// The music is fading out.
    pub fading: bool,
    /// The time left when this was sent, for the popup's clock time.
    pub left: Duration,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Off,
    At(Instant),
    SongEnd,
    ListEnd,
}

/// The controller's side of the timer.
#[derive(Debug, Default)]
pub(super) struct SleepTimer {
    mode: Mode,
    /// The player was told to pause at the song's end (`Input::PauseAtEnd`).
    armed: bool,
    /// The fade turned the engine's volume down: it comes back by the next Play.
    quiet: bool,
    fading: bool,
    /// How long this fade lasts: the Fade out setting, or less when less was left as it
    /// started, so it always starts from the full volume.
    fade_span: Duration,
    /// The kind, minutes and fade last sent.
    shown: Option<(SleepKind, u32, bool)>,
}

/// The minutes `left`, rounded up, at least 1: 24 min 1 s is "25 min".
pub fn minutes_left(left: Duration) -> u32 {
    u32::try_from(left.as_secs().div_ceil(60))
        .unwrap_or(u32::MAX)
        .max(1)
}

/// The volume's share (1 is the slider's, 0 silence) with `left` to go and a fade of
/// `fade`; `None` before the fade.
pub fn fade_level(left: Duration, fade: Duration) -> Option<f32> {
    (!fade.is_zero() && left < fade)
        .then(|| (left.as_secs_f32() / fade.as_secs_f32()).clamp(0.0, 1.0))
}

/// What's left after "+15 min" with `left` to go: 15 minutes more, 12 hours at most.
pub fn more(left: Duration) -> Duration {
    (left + MORE).min(MOST)
}

/// How long until a timer with `left` to go next needs the loop: the minutes shown change,
/// the fade (`fade` long) starts, or it ends; `FADE_STEP` while it `fading`.
pub fn next_wake(left: Duration, fade: Duration, fading: bool) -> Duration {
    if fading {
        return FADE_STEP.min(left);
    }
    let minute = Duration::from_secs(60);
    let next_minute = left.saturating_sub(minute * (minutes_left(left) - 1));
    let fade_start = left.checked_sub(fade).filter(|d| !d.is_zero());
    [Some(next_minute), fade_start, Some(left)]
        .into_iter()
        .flatten()
        .filter(|d| !d.is_zero())
        .min()
        .unwrap_or_default()
}

impl Controller {
    pub(super) fn sleep_action(&mut self, action: SleepAction) {
        log!("sleep timer: {action:?}");
        let now = Instant::now();
        let left = self.sleep_left(now);
        self.sleep_reset();
        self.sleep.mode = match action {
            SleepAction::Minutes(m) => {
                Mode::At(now + Duration::from_secs(u64::from(m) * 60).min(MOST))
            }
            SleepAction::SongEnd => Mode::SongEnd,
            SleepAction::ListEnd => Mode::ListEnd,
            SleepAction::AddQuarter => Mode::At(now + more(left.unwrap_or_default())),
            SleepAction::Off => Mode::Off,
        };
        self.sleep_check();
    }

    /// When the loop must wake for the timer; `None` when it needn't (off, or a song's or
    /// list's end, which the ticks follow while it plays).
    pub(super) fn sleep_wake(&self) -> Option<Instant> {
        let Mode::At(end) = self.sleep.mode else {
            return None;
        };
        let now = Instant::now();
        let left = end.saturating_duration_since(now);
        Some(now + next_wake(left, self.sleep_fade(), self.sleep.fading))
    }

    /// Brings the timer up to date: run after everything the loop does. Pauses at its end,
    /// fades, sends the window what changed, and gives the volume back once the music plays
    /// again after a pause it made.
    pub(super) fn sleep_check(&mut self) {
        let playing = self.player.wants_play();
        if self.sleep.mode == Mode::Off {
            if self.sleep.quiet && playing {
                self.sleep_volume_back();
            }
            return;
        }
        // The player paused at the song's end.
        if self.sleep.armed && !self.player.pauses_at_end() {
            self.sleep.armed = false;
            log!("sleep timer: paused at the end");
            self.sleep_done();
            return;
        }
        let now = Instant::now();
        let left = self.sleep_left(now).unwrap_or_default();
        match self.sleep.mode {
            Mode::At(end) if now >= end => {
                if playing {
                    log!("sleep timer: pausing");
                    self.input(Input::Toggle);
                }
                self.sleep_done();
                return;
            }
            Mode::SongEnd | Mode::ListEnd => {
                let arm = self.sleep.mode == Mode::SongEnd || self.list_left().0 == 0;
                if arm != self.sleep.armed {
                    self.sleep.armed = arm;
                    self.feed(Input::PauseAtEnd(arm));
                }
            }
            _ => {}
        }
        if self.sleep.fading && !playing {
            // Play/Pause while it fades: paused at once, and the timer has done its job.
            self.sleep_done();
            return;
        }
        if playing && !self.sleep.fading && fade_level(left, self.sleep_fade()).is_some() {
            // The fade starts from the full volume, over what's left when that's less than
            // the setting (a song's end picked in its last seconds).
            self.sleep.fade_span = left.max(Duration::from_millis(1));
            self.sleep.fading = true;
        }
        if self.sleep.fading {
            if left > self.sleep.fade_span {
                // Back before the fade (a seek back in the song): the volume comes back.
                self.sleep.fading = false;
                self.sleep_volume_back();
            } else {
                let level = fade_level(left, self.sleep.fade_span).unwrap_or(1.0);
                self.sleep.quiet = true;
                // The slider's value scaled: the engine's curve makes it an even fade.
                self.audio.set_volume(self.slider_volume() * level);
            }
        }
        self.push_sleep(left);
    }

    /// Stops the timer: the window's pill goes, and so does a pause at the song's end not
    /// used yet. A volume a fade turned down comes back with the next Play.
    fn sleep_done(&mut self) {
        self.sleep_reset_keeping_quiet();
        if self.sleep.quiet && self.player.wants_play() {
            self.sleep_volume_back();
        }
        self.push_sleep(Duration::ZERO);
    }

    /// Before a new choice: no fade (the volume back), no pause at the song's end.
    fn sleep_reset(&mut self) {
        self.sleep_reset_keeping_quiet();
        if self.sleep.quiet {
            self.sleep_volume_back();
        }
    }

    fn sleep_reset_keeping_quiet(&mut self) {
        self.sleep.mode = Mode::Off;
        self.sleep.fading = false;
        if std::mem::take(&mut self.sleep.armed) {
            self.feed(Input::PauseAtEnd(false));
        }
    }

    fn sleep_volume_back(&mut self) {
        self.sleep.quiet = false;
        self.audio.set_volume(self.slider_volume());
    }

    /// The Fade out setting.
    fn sleep_fade(&self) -> Duration {
        Duration::from_secs(u64::from(self.settings.sleep_fade))
    }

    /// The time left in listening time (at the speed), or `None` while off. For a song or a
    /// list, from the lengths known (a track without one counts as 0).
    fn sleep_left(&self, now: Instant) -> Option<Duration> {
        let song = || {
            let length = self.player.queue().current().and_then(|t| t.duration);
            length
                .unwrap_or_default()
                .saturating_sub(self.player.position())
        };
        let song_time = match self.sleep.mode {
            Mode::Off => return None,
            Mode::At(end) => return Some(end.saturating_duration_since(now)),
            Mode::SongEnd => song(),
            Mode::ListEnd => song() + self.list_left().1,
        };
        let speed = super::playback_speed(&self.settings).max(0.25);
        Some(song_time.div_f32(speed))
    }

    /// The list's songs still to come after the current one (before autoplay's), and their
    /// length.
    fn list_left(&self) -> (usize, Duration) {
        let queue = self.player.queue();
        let upcoming = queue.upcoming();
        let list = &upcoming[..queue.autoplay_start().unwrap_or(upcoming.len())];
        let length = list.iter().filter_map(|t| t.duration).sum();
        (list.len(), length)
    }

    /// Sends the window the timer's state when its kind, minutes or fade changed.
    fn push_sleep(&mut self, left: Duration) {
        let state = match self.sleep.mode {
            Mode::Off => None,
            Mode::At(_) => Some(SleepKind::Time),
            Mode::SongEnd => Some(SleepKind::SongEnd),
            Mode::ListEnd => Some(SleepKind::ListEnd),
        }
        .map(|kind| SleepState {
            kind,
            minutes: minutes_left(left),
            fading: self.sleep.fading,
            left,
        });
        let key = state.map(|s| (s.kind, s.minutes, s.fading));
        if key != self.sleep.shown {
            self.sleep.shown = key;
            (self.ui.events)(UiEvent::Sleep(state));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn the_minutes_left_round_up() {
        assert_eq!(minutes_left(secs(24 * 60)), 24);
        assert_eq!(minutes_left(secs(24 * 60 + 1)), 25);
        assert_eq!(minutes_left(secs(3)), 1);
        assert_eq!(minutes_left(Duration::ZERO), 1);
    }

    #[test]
    fn the_volume_fades_over_the_last_seconds_only() {
        assert_eq!(fade_level(secs(21), secs(20)), None);
        assert_eq!(fade_level(secs(10), secs(20)), Some(0.5));
        assert_eq!(fade_level(Duration::ZERO, secs(20)), Some(0.0));
        assert_eq!(fade_level(secs(5), Duration::ZERO), None, "no fade");
    }

    #[test]
    fn more_time_stops_at_twelve_hours() {
        assert_eq!(more(secs(10 * 60)), secs(25 * 60));
        assert_eq!(more(secs(12 * 3600 - 60)), secs(12 * 3600));
    }

    #[test]
    fn the_loop_wakes_once_a_minute_then_for_the_fade_and_the_end() {
        // 24 min 30 s: "25 min" until 24 min are left.
        assert_eq!(next_wake(secs(24 * 60 + 30), secs(20), false), secs(30));
        assert_eq!(next_wake(secs(24 * 60), secs(20), false), secs(60));
        // The last minute: the fade's start, then its steps.
        assert_eq!(next_wake(secs(50), secs(20), false), secs(30));
        assert_eq!(next_wake(secs(15), secs(20), true), FADE_STEP);
        // No fade: the end.
        assert_eq!(next_wake(secs(50), Duration::ZERO, false), secs(50));
    }
}
