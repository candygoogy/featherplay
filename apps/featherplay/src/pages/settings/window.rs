//! Settings' Window section (ui/pages/settings/window.slint): the close choice, the theme, the
//! language, the accent with its custom colour picker, and the global hotkeys.

use std::rc::Rc;

use featherplay_core::tr;
use featherplay_integrations::parse_hotkey;
use featherplay_store::{Accent, AccentColour, CloseAction, Hotkeys, ThemeMode};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{SETTINGS, commit};
use crate::app::Ctx;
use crate::controller::Change;
use crate::ui::{AccentSwatch, Appearance, SettingsData, Theme};
use crate::{follow, language, shell};

/// Offered when the hotkeys are first switched on.
pub(super) const DEFAULT_HOTKEYS: [&str; 3] = ["Ctrl+Alt+Space", "Ctrl+Alt+Right", "Ctrl+Alt+Left"];
/// The close choices in the order the page lists them.
pub(super) const CLOSE_ACTIONS: [CloseAction; 3] =
    [CloseAction::Ask, CloseAction::Quit, CloseAction::Tray];

/// Shows `mode` (live, without saving): `--snapshot --theme` forces Dark or Light this way.
pub fn apply_theme(ctx: &Ctx, mode: ThemeMode) {
    let Some(ui) = ctx.window() else { return };
    // Until the flip's crossfade starts (`follow::theme_changed`).
    follow::hold(&ui);
    ui.global::<Theme>().invoke_apply(appearance(mode));
    ui.global::<SettingsData>().set_appearance(appearance(mode));
}

pub(super) fn pick_theme(ctx: &Ctx, mode: ThemeMode) {
    if SETTINGS.with(|s| s.borrow().theme == mode) {
        return;
    }
    apply_theme(ctx, mode);
    commit(ctx, Change::Theme(mode));
}

/// The Language setting's list, in the language shown now ("Windows default" is
/// translated), with the saved choice picked. The test language is listed in debug builds
/// only.
pub(super) fn show_languages(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    let setting = SETTINGS.with(|s| s.borrow().language.clone());
    let choices = language::choices(cfg!(debug_assertions));
    let names: Vec<SharedString> = choices.iter().map(|c| c.name.as_str().into()).collect();
    let codes: Vec<SharedString> = choices.iter().map(|c| c.code.into()).collect();
    let at = choices.iter().position(|c| c.code == setting).unwrap_or(0);
    data.set_language_names(ModelRc::new(VecModel::from(names)));
    data.set_language_codes(ModelRc::new(VecModel::from(codes)));
    data.set_language(i32::try_from(at).unwrap_or(0));
}

/// Applies the language at `i` in the list on the spot and saves it.
pub(super) fn pick_language(ctx: &Ctx, i: i32) {
    let choices = language::choices(cfg!(debug_assertions));
    let Some(code) = usize::try_from(i)
        .ok()
        .and_then(|i| choices.get(i))
        .map(|c| c.code)
    else {
        return;
    };
    if SETTINGS.with(|s| s.borrow().language == code) {
        return;
    }
    // Saved first: the change refills the list with the new choice picked.
    commit(ctx, Change::Language(code.into()));
    language::apply(&language::for_setting(code));
}

/// The preset accents for the page's swatches, in `Accent::ALL`'s order between "From the
/// cover" and the custom colour, which the page draws itself.
pub(super) fn accent_swatches() -> ModelRc<AccentSwatch> {
    let colour = |hex: u32| slint::Color::from_argb_encoded(0xFF00_0000 | hex);
    let swatches: Vec<AccentSwatch> = Accent::ALL[1..Accent::ALL.len() - 1]
        .iter()
        .map(|&a| {
            let (dark, light) = follow::preset(a);
            AccentSwatch {
                name: accent_name(a).into(),
                dark: colour(dark),
                light: colour(light),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(swatches))
}

fn accent_name(a: Accent) -> &'static str {
    match a {
        Accent::Cover => tr!("From the cover"),
        Accent::Red => tr!("Red"),
        Accent::Rose => tr!("Rose"),
        Accent::Amber => tr!("Amber"),
        Accent::Teal => tr!("Teal"),
        Accent::Violet => tr!("Violet"),
        Accent::Custom => tr!("Custom colour"),
    }
}

/// The accent's place on the page: 0 is "From the cover", then the swatches.
pub(super) fn accent_index(a: Accent) -> i32 {
    Accent::ALL.iter().position(|&x| x == a).unwrap_or(0) as i32
}

/// The close choice's place in the page's list.
pub(super) fn close_action_index(action: CloseAction) -> i32 {
    CLOSE_ACTIONS.iter().position(|&a| a == action).unwrap_or(0) as i32
}

pub(super) fn pick_accent(ctx: &Rc<Ctx>, i: i32) {
    let Some(&accent) = usize::try_from(i).ok().and_then(|i| Accent::ALL.get(i)) else {
        return;
    };
    if SETTINGS.with(|s| s.borrow().accent == accent) {
        return;
    }
    let custom = SETTINGS.with(|s| s.borrow().custom_accent);
    if let Some(ui) = ctx.window() {
        let data = ui.global::<SettingsData>();
        data.set_accent(i);
        // The picker opens on the saved colour, not a half-typed one.
        show_custom(&data, custom);
    }
    follow::set_accent(ctx, accent, custom);
    commit(ctx, Change::Accent(accent));
}

/// The custom picker's fields for `c`.
pub(super) fn show_custom(data: &SettingsData, c: AccentColour) {
    data.set_custom_hex(c.to_string().into());
    data.set_custom_hue(follow::hue(c));
    data.set_custom_error("".into());
    show_custom_notes(data, c);
}

fn show_custom_notes(data: &SettingsData, c: AccentColour) {
    data.set_custom_note_dark(custom_note(c, true).into());
    data.set_custom_note_light(custom_note(c, false).into());
}

/// What the picker says under the colour when the theme shows it differently: empty, or
/// the colour it shows and why.
fn custom_note(c: AccentColour, dark: bool) -> String {
    let (d, l) = follow::custom(c);
    let shown = AccentColour(if dark { d } else { l });
    if shown == c {
        return String::new();
    }
    if dark {
        tr!(
            "On the dark theme it shows as {}, so it stays easy to see.",
            shown
        )
    } else {
        tr!(
            "On the light theme it shows as {}, so it stays easy to see.",
            shown
        )
    }
}

/// The hue slider moved to `degrees`: the accent follows at once; `done` (the button let
/// go, or a key step) saves it.
pub(super) fn custom_hue_moved(ctx: &Rc<Ctx>, degrees: f32, done: bool) {
    let c = follow::hue_colour(degrees);
    if let Some(ui) = ctx.window() {
        let data = ui.global::<SettingsData>();
        data.set_custom_hue(degrees.rem_euclid(360.0));
        data.set_custom_hex(c.to_string().into());
        data.set_custom_error("".into());
        show_custom_notes(&data, c);
    }
    follow::set_accent(ctx, Accent::Custom, c);
    if done {
        save_custom(ctx, c);
    }
}

/// A colour typed into the hex field (Enter, or leaving it).
pub(super) fn custom_hex_entered(ctx: &Rc<Ctx>, text: &str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    let Some(c) = AccentColour::parse(text) else {
        data.set_custom_error(
            tr!("Type a colour as # and 6 letters or digits, like #4390EE.").into(),
        );
        return;
    };
    show_custom(&data, c);
    follow::set_accent(ctx, Accent::Custom, c);
    save_custom(ctx, c);
}

fn save_custom(ctx: &Ctx, c: AccentColour) {
    let saved = SETTINGS.with(|s| {
        let s = s.borrow();
        s.accent == Accent::Custom && s.custom_accent == c
    });
    if !saved {
        commit(ctx, Change::CustomAccent(c));
    }
}

pub(super) fn appearance(mode: ThemeMode) -> Appearance {
    match mode {
        ThemeMode::Dark => Appearance::Dark,
        ThemeMode::Light => Appearance::Light,
        ThemeMode::System => Appearance::System,
    }
}

pub(super) fn theme_mode(a: Appearance) -> ThemeMode {
    match a {
        Appearance::Dark => ThemeMode::Dark,
        Appearance::Light => ThemeMode::Light,
        Appearance::System => ThemeMode::System,
    }
}

/// Fills the empty hotkey fields with the suggested keys.
pub(super) fn fill_default_hotkeys(data: &SettingsData) {
    let [play_pause, next, previous] = DEFAULT_HOTKEYS.map(SharedString::from);
    if data.get_hotkey_play_pause().trim().is_empty() {
        data.set_hotkey_play_pause(play_pause);
    }
    if data.get_hotkey_next().trim().is_empty() {
        data.set_hotkey_next(next);
    }
    if data.get_hotkey_previous().trim().is_empty() {
        data.set_hotkey_previous(previous);
    }
}

/// A hotkey field was committed (Enter or leaving it): check all three, register them,
/// and save them once they work.
pub(super) fn hotkeys_edited(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    if !data.get_hotkeys() {
        return;
    }
    let wanted = Hotkeys {
        play_pause: data.get_hotkey_play_pause().trim().to_string(),
        next: data.get_hotkey_next().trim().to_string(),
        previous: data.get_hotkey_previous().trim().to_string(),
    };
    let previous = SETTINGS.with(|s| s.borrow().hotkeys.clone());
    if previous.as_ref() == Some(&wanted) {
        data.set_hotkeys_error("".into());
        return;
    }
    if let Err(e) = check_hotkeys(&wanted) {
        data.set_hotkeys_error(e.into());
        return;
    }
    match shell::set_hotkeys(Some(&wanted), previous.as_ref()) {
        Ok(()) => {
            data.set_hotkeys_error("".into());
            commit(ctx, Change::Hotkeys(Some(wanted)));
        }
        Err(_) => data.set_hotkeys_error(
            tr!("Another app already uses one of these keys. Pick other keys.").into(),
        ),
    }
}

/// Each key parses (`parse_hotkey`: it needs a modifier) and the three differ.
pub fn check_hotkeys(h: &Hotkeys) -> Result<(), String> {
    let mut ids = Vec::new();
    for (label, text) in [
        (tr!("Play or pause"), &h.play_pause),
        (tr!("Next"), &h.next),
        (tr!("Previous"), &h.previous),
    ] {
        // The fourth modifier is the Windows key; elsewhere it's called Super.
        let key = parse_hotkey(text).map_err(|_| {
            if cfg!(windows) {
                tr!("{0}: \"{1}\" isn't a key featherplay can use. Write it like Ctrl+Alt+Space; it needs Ctrl, Alt, Shift or Win.", label, text)
            } else {
                tr!("{0}: \"{1}\" isn't a key featherplay can use. Write it like Ctrl+Alt+Space; it needs Ctrl, Alt, Shift or Super.", label, text)
            }
        })?;
        if ids.contains(&key.id()) {
            return Err(tr!("{} uses the same key as another action.", label));
        }
        ids.push(key.id());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(a: &str, b: &str, c: &str) -> Hotkeys {
        Hotkeys {
            play_pause: a.into(),
            next: b.into(),
            previous: c.into(),
        }
    }

    #[test]
    fn every_accent_has_a_place_on_the_page() {
        for (i, &a) in Accent::ALL.iter().enumerate() {
            assert_eq!(accent_index(a), i as i32);
        }
        assert_eq!(accent_index(Accent::Cover), 0, "From the cover comes first");
        use slint::Model;
        assert_eq!(accent_swatches().row_count(), Accent::ALL.len() - 2);
        assert_eq!(
            accent_index(Accent::Custom),
            Accent::ALL.len() as i32 - 1,
            "the custom colour comes last, after the swatches"
        );
    }

    #[test]
    fn the_picker_says_when_the_colour_is_shown_differently() {
        // The teal preset's dark shade reads on dark as it is.
        assert_eq!(custom_note(AccentColour(0x2DD4BF), true), "");
        let navy = custom_note(AccentColour(0x1A237E), true);
        let (shown, _) = follow::custom(AccentColour(0x1A237E));
        assert_eq!(
            navy,
            format!(
                "On the dark theme it shows as {}, so it stays easy to see.",
                AccentColour(shown)
            )
        );
        assert!(custom_note(AccentColour(0xFFEB3B), false).starts_with("On the light theme"));
    }

    #[test]
    fn hotkeys_are_checked_before_registering() {
        assert!(check_hotkeys(&keys("Ctrl+Alt+Space", "Ctrl+Alt+Right", "Ctrl+Alt+Left")).is_ok());
        let no_modifier = check_hotkeys(&keys("Space", "Ctrl+Alt+Right", "Ctrl+Alt+Left"));
        assert!(no_modifier.unwrap_err().starts_with("Play or pause"));
        let garbage = check_hotkeys(&keys("Ctrl+Alt+Space", "Ctrl+Nope", "Ctrl+Alt+Left"));
        assert!(garbage.unwrap_err().starts_with("Next"));
        let twice = check_hotkeys(&keys("Ctrl+Alt+Space", "Ctrl+Alt+Right", "ctrl+alt+right"));
        assert!(twice.unwrap_err().contains("same key"));
    }
}
