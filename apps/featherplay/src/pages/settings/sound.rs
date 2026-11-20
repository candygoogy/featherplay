//! Settings' Sound section (ui/pages/settings/sound.slint): the Equalizer and its dialog's
//! bands, Even out loudness, Streaming quality and the output device.

use std::rc::Rc;

use featherplay_core::tr;
use featherplay_store::{Equalizer, Loudness, Settings, SkipSilence, StreamingQuality};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::playback::{crossfade_index, skip_silence_index};
use super::{DEVICES, SETTINGS, commit, open_dialog};
use crate::app::Ctx;
use crate::controller::Change;
use crate::ui::{ChoiceMenu, SettingId, SettingsData, SettingsDialog};

/// Shows the Equalizer row's choice, and the bands it plays (all 0 dB when off) for the
/// dialog.
pub(super) fn show_equalizer(data: &SettingsData, s: &Settings) {
    data.set_equalizer(
        Equalizer::ALL
            .iter()
            .position(|&e| e == s.equalizer)
            .unwrap_or(0) as i32,
    );
    let bands = s.equalizer.bands(s.equalizer_bands).unwrap_or_default();
    let model = data.get_eq_bands();
    let Some(rows) = model.as_any().downcast_ref::<VecModel<i32>>() else {
        return;
    };
    for (i, &db) in bands.iter().enumerate() {
        if rows.row_data(i) != Some(i32::from(db)) {
            rows.set_row_data(i, i32::from(db));
        }
    }
}

/// The Equalizer chip at `i`. Custom… opens the bands as they play now; from Off or a preset
/// it brings back the user's own bands first, if they set any, so a preset tried in between
/// doesn't cost them.
pub(super) fn pick_equalizer(ctx: &Ctx, i: i32) {
    let Some(&choice) = usize::try_from(i).ok().and_then(|i| Equalizer::ALL.get(i)) else {
        return;
    };
    let (now, custom) = SETTINGS.with(|s| {
        let s = s.borrow();
        (s.equalizer, s.equalizer_bands)
    });
    if choice != Equalizer::Custom {
        set_equalizer(ctx, choice, custom);
        return;
    }
    if now != Equalizer::Custom && custom != [0; 5] {
        set_equalizer(ctx, Equalizer::Custom, custom);
    }
    open_dialog(ctx, SettingsDialog::Equalizer);
}

/// A band in the dialog moved to `db`: the bands playing now, with that one changed,
/// become the Custom bands.
pub(super) fn eq_band_moved(ctx: &Ctx, band: i32, db: i32) {
    let mut bands = SETTINGS.with(|s| {
        let s = s.borrow();
        s.equalizer.bands(s.equalizer_bands).unwrap_or_default()
    });
    let Some(slot) = usize::try_from(band).ok().and_then(|b| bands.get_mut(b)) else {
        return;
    };
    let max = i32::from(Equalizer::MAX_DB);
    *slot = db.clamp(-max, max) as i8;
    set_equalizer(ctx, Equalizer::Custom, bands);
}

/// Shows and saves the Equalizer's choice and Custom bands; the controller applies them.
pub(super) fn set_equalizer(ctx: &Ctx, choice: Equalizer, custom: [i8; 5]) {
    let same = SETTINGS.with(|s| {
        let s = s.borrow();
        (s.equalizer, s.equalizer_bands) == (choice, custom)
    });
    if !same {
        commit(ctx, Change::Equalizer(choice, custom));
    }
    if let Some(ui) = ctx.window() {
        SETTINGS.with(|s| show_equalizer(&ui.global::<SettingsData>(), &s.borrow()));
    }
}

/// The Even out loudness chips, in the page's order.
const LOUDNESS: [Loudness; 3] = [Loudness::Off, Loudness::Light, Loudness::Strong];

pub(super) fn loudness_index(l: Loudness) -> i32 {
    LOUDNESS.iter().position(|&x| x == l).unwrap_or(0) as i32
}

/// The Even out loudness chip at `i`, shown and saved; the controller applies it.
pub(super) fn pick_loudness(ctx: &Ctx, i: i32) {
    let Some(&l) = usize::try_from(i).ok().and_then(|i| LOUDNESS.get(i)) else {
        return;
    };
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>().set_loudness(loudness_index(l));
    if SETTINGS.with(|s| s.borrow().loudness != l) {
        commit(ctx, Change::Loudness(l));
    }
}

/// `--snapshot settings-sound` and `settings-equalizer`: the new sound rows on (Skip
/// silence at Start and end, Crossfade at 5 s with Fade on skip, the Bass preset, Light), or
/// the Equalizer dialog over the page with Custom bands. Shown only, not saved.
pub fn show_sound_for_snapshot(ctx: &Rc<Ctx>, dialog: bool) {
    let Some(ui) = ctx.window() else { return };
    SETTINGS.with(|s| {
        let mut s = s.borrow_mut();
        if dialog {
            s.equalizer = Equalizer::Custom;
            s.equalizer_bands = [6, 2, 0, 3, 5];
        } else {
            s.equalizer = Equalizer::Bass;
            s.loudness = Loudness::Light;
            s.skip_silence = SkipSilence::StartAndEnd;
            s.crossfade = 5;
            s.fade_on_skip = true;
        }
    });
    let data = ui.global::<SettingsData>();
    SETTINGS.with(|s| show_equalizer(&data, &s.borrow()));
    data.set_loudness(SETTINGS.with(|s| loudness_index(s.borrow().loudness)));
    data.set_skip_silence(SETTINGS.with(|s| skip_silence_index(s.borrow().skip_silence)));
    SETTINGS.with(|s| {
        let s = s.borrow();
        data.set_crossfade(crossfade_index(s.crossfade));
        data.set_fade_on_skip(s.fade_on_skip);
    });
    if dialog {
        open_dialog(ctx, SettingsDialog::Equalizer);
    }
}

/// The Streaming quality chips, in the page's order.
const STREAMING_QUALITIES: [StreamingQuality; 2] =
    [StreamingQuality::High, StreamingQuality::Standard];

/// The quality's place in the page's chips.
pub(super) fn streaming_quality_index(q: StreamingQuality) -> i32 {
    STREAMING_QUALITIES
        .iter()
        .position(|&x| x == q)
        .unwrap_or(0) as i32
}

/// The quality at `i` in the page's chips, shown and saved; the controller applies it from
/// the next resolve.
pub(super) fn pick_streaming_quality(ctx: &Ctx, i: i32) {
    let Some(&q) = usize::try_from(i)
        .ok()
        .and_then(|i| STREAMING_QUALITIES.get(i))
    else {
        return;
    };
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>()
        .set_streaming_quality(streaming_quality_index(q));
    if SETTINGS.with(|s| s.borrow().streaming_quality != q) {
        commit(ctx, Change::StreamingQuality(q));
    }
}

/// Shows the device list with the saved one selected, each device's name over its adapter;
/// a saved device that isn't connected stays in the list, marked. A menu open on the old list
/// closes: its rows would no longer match.
pub(super) fn set_devices(ctx: &Ctx, names: Vec<String>) {
    let Some(ui) = ctx.window() else { return };
    let saved = SETTINGS.with(|s| s.borrow().output_device.clone());
    let mut devices: Vec<Option<String>> = vec![None];
    let mut labels: Vec<SharedString> = vec![tr!("System default").into()];
    let mut details: Vec<SharedString> = vec![SharedString::new()];
    for n in names {
        let (name, adapter) = split_device(&n);
        labels.push(name.into());
        details.push(adapter.into());
        devices.push(Some(n));
    }
    if let Some(s) = &saved
        && !devices.contains(&Some(s.clone()))
    {
        labels.push(split_device(s).0.into());
        details.push(tr!("Not connected").into());
        devices.push(Some(s.clone()));
    }
    let index = devices.iter().position(|d| *d == saved).unwrap_or(0);
    DEVICES.with(|d| *d.borrow_mut() = devices);
    let menu = ui.global::<ChoiceMenu>();
    if menu.get_open() && menu.get_setting() == SettingId::OutputDevice {
        menu.invoke_close(false);
    }
    let data = ui.global::<SettingsData>();
    data.set_devices(ModelRc::new(VecModel::from(labels)));
    data.set_device_details(ModelRc::new(VecModel::from(details)));
    data.set_device_index(index as i32);
}

/// A device's name and its adapter, as Windows names an endpoint: "Speakers (Realtek(R)
/// Audio)" is "Speakers" on "Realtek(R) Audio", the brackets closing last. Any other name is
/// all name. Windows' instance number before the adapter ("3- ") goes; an adapter that only
/// repeats the name's start ("Fosi Audio ZH3 (3- Fosi Audio)") is no adapter.
fn split_device(full: &str) -> (&str, &str) {
    let (name, adapter) = split_brackets(full);
    let adapter = drop_instance(adapter);
    let repeats = name
        .get(..adapter.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(adapter));
    (name, if repeats { "" } else { adapter })
}

/// "3- Fosi Audio" without its leading "3- "; anything else as it is.
fn drop_instance(adapter: &str) -> &str {
    let rest = adapter.trim_start_matches(|c: char| c.is_ascii_digit());
    match rest.strip_prefix("- ") {
        Some(rest) if rest.len() < adapter.len() - 2 => rest,
        _ => adapter,
    }
}

fn split_brackets(full: &str) -> (&str, &str) {
    let Some(inner) = full.strip_suffix(')') else {
        return (full, "");
    };
    let mut depth = 0;
    for (i, c) in inner.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth > 0 => depth -= 1,
            '(' => {
                let name = inner[..i].strip_suffix(' ').unwrap_or("").trim_end();
                let adapter = &inner[i + 1..];
                if name.is_empty() || adapter.is_empty() {
                    break;
                }
                return (name, adapter);
            }
            _ => {}
        }
    }
    (full, "")
}

pub(super) fn pick_device(ctx: &Ctx, i: i32) {
    let Some(device) = DEVICES.with(|d| d.borrow().get(i as usize).cloned()) else {
        return;
    };
    if SETTINGS.with(|s| s.borrow().output_device == device) {
        return;
    }
    if let Some(ui) = ctx.window() {
        ui.global::<SettingsData>().set_device_index(i);
    }
    commit(ctx, Change::OutputDevice(device));
}

#[cfg(test)]
mod tests {
    use super::split_device;

    #[test]
    fn a_device_splits_into_its_name_and_adapter_at_the_last_brackets() {
        let cases = [
            (
                "Speakers (Realtek(R) Audio)",
                ("Speakers", "Realtek(R) Audio"),
            ),
            (
                "LG ULTRAGEAR (NVIDIA High Definition Audio)",
                ("LG ULTRAGEAR", "NVIDIA High Definition Audio"),
            ),
            (
                "Headphones (2- WH-1000XM4 (Stereo))",
                ("Headphones", "WH-1000XM4 (Stereo)"),
            ),
            // Windows' instance number goes; then the adapter only repeats the name.
            ("Fosi Audio ZH3 (3- Fosi Audio)", ("Fosi Audio ZH3", "")),
            ("Speakers (12- USB Audio)", ("Speakers", "USB Audio")),
            ("Speakers (3- )", ("Speakers", "")),
            // Digits that aren't an instance number stay.
            ("Speakers (2-Channel DAC)", ("Speakers", "2-Channel DAC")),
            ("Speakers (- USB Audio)", ("Speakers", "- USB Audio")),
            ("Speakers", ("Speakers", "")),
            ("(Realtek)", ("(Realtek)", "")),
            ("Speakers ()", ("Speakers ()", "")),
            ("Speakers(Realtek)", ("Speakers(Realtek)", "")),
            ("Speakers Realtek)", ("Speakers Realtek)", "")),
        ];
        for (full, want) in cases {
            assert_eq!(split_device(full), want, "{full}");
        }
    }
}
