//! The sleep timer's button and popup in the player bar (`Sleep` in ui/sleep_timer.slint)
//! and its tray submenu, while Settings › Playback › Sleep timer is on. A choice goes to the
//! controller as `Action::Sleep`; the controller runs the timer and its fade
//! (`controller/sleep.rs`) and sends `UiEvent::Sleep` when what's shown changes, at most
//! once a minute. Nothing here keeps time: the popup's clock time and the tray's labels are
//! worked out when they open.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay_controller::place::Place;
use featherplay_core::tr;
use featherplay_platform::system::local_time;
use featherplay_platform::tray::SleepMenu;
use slint::ComponentHandle;

use crate::app::Ctx;
use crate::controller::{Action, PlayerState, SleepAction, SleepKind, SleepState};
use crate::pages::settings;
use crate::shell;
use crate::ui::{Player, Sleep};

#[derive(Default)]
struct Shown {
    state: Option<SleepState>,
    /// When `state` came.
    since: Option<Instant>,
    /// The list playing, for its end: an album (`Some(true)`) or a playlist; `None` for a
    /// radio or nothing.
    list: Option<bool>,
}

thread_local! {
    static SHOWN: RefCell<Shown> = RefCell::default();
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let sleep = ui.global::<Sleep>();
    let c = ctx.clone();
    sleep.on_pick(move |i| {
        if let Some(a) = choice(i) {
            c.send(Action::Sleep(a));
        }
    });
    let c = ctx.clone();
    sleep.on_add_quarter(move || c.send(Action::Sleep(SleepAction::AddQuarter)));
    let c = ctx.clone();
    sleep.on_turn_off(move || c.send(Action::Sleep(SleepAction::Off)));
    let c = ctx.clone();
    sleep.on_opening(move || show_line(&c));
}

/// The popup's chip `i`: the times, then the song's and the list's end.
fn choice(i: i32) -> Option<SleepAction> {
    Some(match i {
        0 => SleepAction::Minutes(15),
        1 => SleepAction::Minutes(30),
        2 => SleepAction::Minutes(45),
        3 => SleepAction::Minutes(60),
        4 => SleepAction::SongEnd,
        5 => SleepAction::ListEnd,
        _ => return None,
    })
}

/// The tray submenu's entries, as the popup's.
pub fn from_tray(ctx: &Ctx, action: SleepAction) {
    ctx.send(Action::Sleep(action));
}

/// The controller's news: the bar's button, the popup's line, the tray's submenu.
pub fn apply(ctx: &Ctx, state: Option<SleepState>) {
    SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        s.state = state;
        s.since = Some(Instant::now());
    });
    let Some(ui) = ctx.window() else { return };
    let sleep = ui.global::<Sleep>();
    sleep.set_running(state.is_some());
    sleep.set_fading(state.is_some_and(|s| s.fading));
    if let Some(s) = state {
        sleep.set_minutes(i32::try_from(s.minutes).unwrap_or(i32::MAX));
    }
    show_line(ctx);
    shell::set_tray_menu(&ui.global::<Player>());
}

/// What's playing, for the list's end: named after an album or a playlist, and not offered
/// for a radio (it has no end) or with nothing to play.
pub fn follow(ctx: &Ctx, state: &PlayerState) {
    let list = (state.current.is_some() && !state.radio).then_some(matches!(
        state.queue.playing_from.page,
        Some(Place::Album(_))
    ));
    if SHOWN.with(|s| std::mem::replace(&mut s.borrow_mut().list, list)) == list {
        return;
    }
    let Some(ui) = ctx.window() else { return };
    ui.global::<Sleep>().set_list_chip(
        match list {
            Some(true) => tr!("End of album"),
            Some(false) => tr!("End of playlist"),
            None => "",
        }
        .into(),
    );
    show_line(ctx);
}

/// The setting was turned on or off: the tray's submenu comes or goes, and so does the
/// popup (the controller turns a running timer off).
pub fn setting_changed(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    if !settings::sleep_timer() {
        ui.global::<Sleep>().set_open(false);
    }
    shell::set_tray_menu(&ui.global::<Player>());
}

/// The tray's submenu, while the setting is on: what's left while it runs, and the list.
pub fn menu() -> Option<SleepMenu> {
    if !settings::sleep_timer() {
        return None;
    }
    SHOWN.with(|s| {
        let s = s.borrow();
        Some(SleepMenu {
            left: s.state.map(|st| left_text(&st)),
            list: s.list,
        })
    })
}

/// "24 min left", or "Fading out".
fn left_text(s: &SleepState) -> String {
    if s.fading {
        tr!("Fading out").into()
    } else {
        tr!("{} min left", s.minutes)
    }
}

/// Sets the popup's line for the timer shown, with the clock time now.
fn show_line(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let text = SHOWN.with(|s| {
        let s = s.borrow();
        let state = s.state?;
        let gone = s.since.map(|t| t.elapsed()).unwrap_or_default();
        let t = local_time();
        let now = Duration::from_secs(
            u64::from(t.hour) * 3600 + u64::from(t.minute) * 60 + u64::from(t.second),
        );
        Some(line(&state, s.list, now, gone))
    });
    ui.global::<Sleep>()
        .set_line(text.unwrap_or_default().into());
}

/// The popup's line for `state`, sent `gone` ago, at `now` into the local day: when it
/// pauses, and in how many minutes.
pub fn line(state: &SleepState, list: Option<bool>, now: Duration, gone: Duration) -> String {
    if state.fading {
        return tr!("Fading out").into();
    }
    let minutes = state.minutes;
    match state.kind {
        SleepKind::Time => {
            let at = (now + state.left.saturating_sub(gone)).as_secs() / 60 % (24 * 60);
            let clock = format!("{}:{:02}", at / 60, at % 60);
            tr!("Pauses at {} (in {} min)", clock, minutes)
        }
        SleepKind::SongEnd => tr!("Pauses when this song ends (in {} min)", minutes),
        SleepKind::ListEnd if list == Some(true) => {
            tr!("Pauses when this album ends (in {} min)", minutes)
        }
        SleepKind::ListEnd => tr!("Pauses when this playlist ends (in {} min)", minutes),
    }
}

/// Shows the timer as the controller would: for the snapshots.
pub fn show_for_snapshot(ctx: &Ctx, state: Option<SleepState>, open: bool) {
    apply(ctx, state);
    if let Some(ui) = ctx.window() {
        ui.global::<Sleep>().set_open(open);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(kind: SleepKind, minutes: u32) -> SleepState {
        SleepState {
            kind,
            minutes,
            fading: false,
            left: Duration::from_secs(u64::from(minutes) * 60),
        }
    }

    #[test]
    fn the_line_says_when_it_pauses_and_in_how_long() {
        let at = |h: u64, m: u64| Duration::from_secs(h * 3600 + m * 60);
        let s = state(SleepKind::Time, 24);
        assert_eq!(
            line(&s, None, at(23, 23), Duration::ZERO),
            "Pauses at 23:47 (in 24 min)"
        );
        // Past midnight.
        assert_eq!(
            line(&s, None, at(23, 50), Duration::from_secs(60)),
            "Pauses at 0:13 (in 24 min)"
        );
        assert_eq!(
            line(
                &state(SleepKind::ListEnd, 12),
                Some(true),
                at(1, 0),
                Duration::ZERO
            ),
            "Pauses when this album ends (in 12 min)"
        );
    }
}
