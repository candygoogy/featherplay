//! The keyboard shortcuts' dialog (ui/pages/settings/shortcuts.slint): the saved hotkeys as
//! its keycaps.

use std::rc::Rc;

use featherplay_store::Hotkeys;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::window::DEFAULT_HOTKEYS;
use super::{SETTINGS, open_dialog};
use crate::app::Ctx;
use crate::ui::{SettingsData, SettingsDialog};

/// `--snapshot shortcuts` and `shortcuts-hotkeys`: the keyboard shortcuts' list, with the
/// global hotkeys off, or on with the suggested keys (shown, not saved).
pub fn show_shortcuts_for_snapshot(ctx: &Rc<Ctx>, hotkeys: bool) {
    let Some(ui) = ctx.window() else { return };
    let [play_pause, next, previous] = DEFAULT_HOTKEYS.map(String::from);
    SETTINGS.with(|s| {
        s.borrow_mut().hotkeys = hotkeys.then_some(Hotkeys {
            play_pause,
            next,
            previous,
        });
    });
    ui.global::<SettingsData>().set_hotkeys(hotkeys);
    open_dialog(ctx, SettingsDialog::Shortcuts);
}

/// A hotkey as the shortcuts' list draws it.
pub(super) fn keycaps(hotkey: &str) -> ModelRc<SharedString> {
    let caps: Vec<SharedString> = keycap_names(hotkey).into_iter().map(Into::into).collect();
    ModelRc::new(VecModel::from(caps))
}

/// A keycap per key of a hotkey, in the names the list uses whatever spelling the parser
/// took: "ctrl+alt+ArrowRight" is Ctrl, Alt, →; "Super+KeyP" is Win, P.
pub(super) fn keycap_names(hotkey: &str) -> Vec<String> {
    hotkey
        .split('+')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| {
            let lower = k.to_ascii_lowercase();
            let named = match lower.as_str() {
                "ctrl" | "control" => "Ctrl",
                "alt" | "option" => "Alt",
                "shift" => "Shift",
                "super" | "meta" | "cmd" | "command" | "win" => "Win",
                "left" | "arrowleft" => "←",
                "right" | "arrowright" => "→",
                "up" | "arrowup" => "↑",
                "down" | "arrowdown" => "↓",
                _ => "",
            };
            if !named.is_empty() {
                return named.to_string();
            }
            let bare = ["key", "digit", "numpad", "num"]
                .iter()
                .find_map(|p| lower.strip_prefix(p).filter(|rest| rest.len() == 1))
                .map_or(k, |rest| &k[k.len() - rest.len()..]);
            if bare.len() == 1 {
                bare.to_ascii_uppercase()
            } else {
                bare.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hotkey_shows_as_the_lists_keycaps_however_it_was_typed() {
        assert_eq!(keycap_names("Ctrl+Alt+Right"), ["Ctrl", "Alt", "→"]);
        assert_eq!(
            keycap_names("control + shift+ArrowLeft"),
            ["Ctrl", "Shift", "←"]
        );
        assert_eq!(keycap_names("Super+KeyP"), ["Win", "P"]);
        assert_eq!(keycap_names("Ctrl+Alt+Digit1"), ["Ctrl", "Alt", "1"]);
        assert_eq!(keycap_names("Ctrl+Alt+Space"), ["Ctrl", "Alt", "Space"]);
        assert_eq!(keycap_names("ctrl+m"), ["Ctrl", "M"]);
    }
}
