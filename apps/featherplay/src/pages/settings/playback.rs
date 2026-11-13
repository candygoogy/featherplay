//! Settings' Playback section (ui/pages/settings/playback.slint): the chips' choices (Skip
//! silence, Crossfade, the sleep timer's Fade out, Playback speed, Volume step); its switches
//! go through `toggled`.

use std::rc::Rc;

use featherplay_store::{Settings, SkipSilence};
use slint::ComponentHandle;

use super::{SETTINGS, commit};
use crate::app::Ctx;
use crate::controller::Change;
use crate::ui::{Player, SettingsData};

/// Shows the volume step (percent) on the page and hands it to the bar's slider, which the
/// wheel and the window's Up and Down keys move by it.
pub(super) fn show_volume_step(ctx: &Ctx, step: u8) {
    let Some(ui) = ctx.window() else { return };
    let place = |step: u8| Settings::VOLUME_STEPS.iter().position(|&s| s == step);
    let at = place(step)
        .or_else(|| place(Settings::VOLUME_STEP))
        .unwrap_or_default();
    ui.global::<SettingsData>()
        .set_volume_step(i32::try_from(at).unwrap_or_default());
    ui.global::<Player>().set_volume_step(i32::from(step));
}

/// The volume step at `i` in the page's chips, applied at once and saved.
pub(super) fn pick_volume_step(ctx: &Ctx, i: i32) {
    let Some(&step) = usize::try_from(i)
        .ok()
        .and_then(|i| Settings::VOLUME_STEPS.get(i))
    else {
        return;
    };
    show_volume_step(ctx, step);
    if SETTINGS.with(|s| s.borrow().volume_step != step) {
        commit(ctx, Change::VolumeStep(step));
    }
}

/// The Skip silence chips, in the page's order.
const SKIP_SILENCE: [SkipSilence; 3] = [
    SkipSilence::Off,
    SkipSilence::StartAndEnd,
    SkipSilence::Everywhere,
];

pub(super) fn skip_silence_index(m: SkipSilence) -> i32 {
    SKIP_SILENCE.iter().position(|&x| x == m).unwrap_or(0) as i32
}

/// The Skip silence chip at `i`, shown and saved; the controller applies it.
pub(super) fn pick_skip_silence(ctx: &Ctx, i: i32) {
    let Some(&m) = usize::try_from(i).ok().and_then(|i| SKIP_SILENCE.get(i)) else {
        return;
    };
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>()
        .set_skip_silence(skip_silence_index(m));
    if SETTINGS.with(|s| s.borrow().skip_silence != m) {
        commit(ctx, Change::SkipSilence(m));
    }
}

pub(super) fn crossfade_index(secs: u8) -> i32 {
    Settings::CROSSFADES
        .iter()
        .position(|&x| x == secs)
        .unwrap_or(0) as i32
}

/// The Crossfade chip at `i`, shown and saved; the controller applies it.
pub(super) fn pick_crossfade(ctx: &Ctx, i: i32) {
    let Some(&secs) = usize::try_from(i)
        .ok()
        .and_then(|i| Settings::CROSSFADES.get(i))
    else {
        return;
    };
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>()
        .set_crossfade(crossfade_index(secs));
    if SETTINGS.with(|s| s.borrow().crossfade != secs) {
        commit(ctx, Change::Crossfade(secs));
    }
}

pub(super) fn sleep_fade_index(secs: u8) -> i32 {
    let place = |secs: u8| Settings::SLEEP_FADES.iter().position(|&x| x == secs);
    place(secs)
        .or_else(|| place(Settings::SLEEP_FADE))
        .unwrap_or_default() as i32
}

/// The sleep timer's Fade out chip at `i`, shown and saved; the controller reads it when
/// the timer ends.
pub(super) fn pick_sleep_fade(ctx: &Ctx, i: i32) {
    let Some(&secs) = usize::try_from(i)
        .ok()
        .and_then(|i| Settings::SLEEP_FADES.get(i))
    else {
        return;
    };
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>()
        .set_sleep_fade(sleep_fade_index(secs));
    if SETTINGS.with(|s| s.borrow().sleep_fade != secs) {
        commit(ctx, Change::SleepFade(secs));
    }
}

/// Shows the Playback speed (percent) on its chips and the bar's pill.
pub(super) fn show_speed(ctx: &Ctx, percent: u8) {
    let Some(ui) = ctx.window() else { return };
    let place = |p: u8| Settings::SPEEDS.iter().position(|&s| s == p);
    let at = place(percent)
        .or_else(|| place(Settings::NORMAL_SPEED))
        .unwrap_or_default();
    ui.global::<Player>()
        .set_speed(i32::try_from(at).unwrap_or_default());
}

/// The speed chip at `i` (Settings or the bar's popup), shown and saved; the controller
/// applies it.
pub(super) fn pick_speed(ctx: &Ctx, i: i32) {
    let Some(&percent) = usize::try_from(i)
        .ok()
        .and_then(|i| Settings::SPEEDS.get(i))
    else {
        return;
    };
    show_speed(ctx, percent);
    if SETTINGS.with(|s| s.borrow().playback_speed != percent) {
        commit(ctx, Change::PlaybackSpeed(percent));
    }
}

/// `--snapshot` at a playback speed (`--speed`): the row and the bar's pill show it, and
/// `open` opens the pill's popup. Shown only, not saved.
/// The Sleep timer switch on, as if turned on: for the snapshots.
pub fn show_sleep_timer_for_snapshot(ctx: &Ctx) {
    SETTINGS.with(|s| s.borrow_mut().sleep_timer = true);
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>().set_sleep_timer(true);
}

pub fn show_speed_for_snapshot(ctx: &Rc<Ctx>, percent: u8, open: bool) {
    show_speed(ctx, percent);
    let Some(ui) = ctx.window() else { return };
    ui.global::<Player>().set_speed_open(open);
}
