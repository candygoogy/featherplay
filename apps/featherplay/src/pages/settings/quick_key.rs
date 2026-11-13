//! Settings › Window's "Quick search from anywhere" (ui/pages/settings/quick_key.slint): the
//! switch registers the key at once (`quick_search::set_key`), on its own, apart from the
//! three global hotkeys. The key is picked by pressing it: while the field listens, featherplay
//! lets go of its own keys, so pressing one of them works, and takes them back after. A key
//! another app holds stays picked and saved, with a calm line under it: the next start or
//! change tries again. One of featherplay's own global hotkeys isn't registered twice.

use std::rc::Rc;

use featherplay_core::tr;
use featherplay_integrations::{KeyError, same_hotkey};
use featherplay_store::Settings;
use slint::platform::Key;
use slint::{ComponentHandle, SharedString};

use super::shortcuts::{keycap_names, keycaps};
use super::{SETTINGS, commit};
use crate::app::Ctx;
use crate::controller::Change;
use crate::ui::QuickKeyData;
use crate::{quick_search, shell, toast};

/// The command a desktop shortcut runs where featherplay can't hear keys (Wayland).
pub const COMMAND: &str = "featherplay --quick-search";

/// Connects `QuickKeyData`.
pub(super) fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<QuickKeyData>();
    let c = ctx.clone();
    data.on_toggled(move |on| toggled(&c, on));
    let c = ctx.clone();
    data.on_listen(move |on| listen(&c, on));
    let c = ctx.clone();
    data.on_captured(move |text, ctrl, alt, shift, meta| {
        match capture_text(&text, ctrl, alt, shift, meta) {
            Some(key) => picked(&c, &key),
            // A bare key: the field keeps listening and says why it wasn't taken.
            None if !(ctrl || alt || shift || meta) => {
                if let Some(ui) = c.window() {
                    let line = if cfg!(windows) {
                        tr!("A shortcut needs Ctrl, Alt, Shift or Win.")
                    } else {
                        tr!("A shortcut needs Ctrl, Alt, Shift or Super.")
                    };
                    ui.global::<QuickKeyData>().set_error(line.into());
                }
            }
            None => {}
        }
    });
    let c = ctx.clone();
    data.on_reset(move || picked(&c, Settings::QUICK_SEARCH_KEY));
    let c = ctx.clone();
    data.on_copy_command(
        move || match featherplay_platform::clipboard::copy_text(COMMAND) {
            Ok(()) => toast::show(&c, tr!("Command copied")),
            Err(e) => {
                log!("copying the quick search command: {e}");
                toast::show_error(&c, tr!("Couldn't copy the command. Try again."));
            }
        },
    );
}

/// Fills the row from `s`, with the key's state as the last registration left it. A saved
/// key featherplay can't use (hand-edited, from a newer version) shows as the default it fell
/// back to, with a line saying so.
pub(super) fn show(ctx: &Ctx, s: &Settings) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<QuickKeyData>();
    data.set_on(s.quick_search);
    let key = quick_search::usable_key(&s.quick_search_key);
    show_key(&data, key);
    let error = if !s.quick_search || data.get_listening() {
        String::new()
    } else if let Some(e) = quick_search::key_error() {
        error_text(key, &e)
    } else if key != s.quick_search_key {
        fell_back()
    } else {
        String::new()
    };
    data.set_error(error.into());
}

fn fell_back() -> String {
    tr!(
        "The saved shortcut couldn't be used, so it's back to {}.",
        key_text(Settings::QUICK_SEARCH_KEY)
    )
}

fn show_key(data: &QuickKeyData, key: &str) {
    data.set_keys(keycaps(key));
    data.set_key_text(key_text(key).into());
    data.set_is_default(same_hotkey(key, Settings::QUICK_SEARCH_KEY));
}

/// The key as the line under the field writes it: "Ctrl+Alt+K".
fn key_text(key: &str) -> String {
    keycap_names(key).join("+")
}

fn toggled(ctx: &Rc<Ctx>, on: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<QuickKeyData>();
    data.set_on(on);
    data.set_listening(false);
    commit(ctx, Change::QuickSearch(on));
    if on {
        let stored = SETTINGS.with(|s| s.borrow().quick_search_key.clone());
        let key = quick_search::usable_key(&stored);
        register(ctx, key);
        if key != stored && data.get_error().is_empty() {
            data.set_error(fell_back().into());
        }
    } else {
        let _ = quick_search::set_key(None);
        data.set_error(SharedString::new());
    }
}

/// The field starts listening (`on`), or stops without a key (Esc, leaving it).
fn listen(ctx: &Rc<Ctx>, on: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<QuickKeyData>();
    if data.get_listening() == on {
        return;
    }
    data.set_listening(on);
    let (key, hotkeys) = SETTINGS.with(|s| {
        let s = s.borrow();
        (s.quick_search_key.clone(), s.hotkeys.clone())
    });
    if on {
        data.set_error(SharedString::new());
        // featherplay's own keys would be taken before the field hears them.
        let _ = quick_search::set_key(None);
        if hotkeys.is_some() {
            let _ = shell::set_hotkeys(None, None);
        }
    } else {
        give_back_hotkeys(hotkeys.as_ref());
        register(ctx, quick_search::usable_key(&key));
    }
}

fn give_back_hotkeys(hotkeys: Option<&featherplay_store::Hotkeys>) {
    if let Some(h) = hotkeys
        && let Err(e) = shell::set_hotkeys(Some(h), None)
    {
        log!("hotkeys: {e}");
    }
}

/// Keys the system keeps for itself: never offered to the quick search.
const RESERVED: [&str; 6] = [
    "Ctrl+Alt+Delete",
    "Alt+Tab",
    "Alt+F4",
    "Alt+Space",
    "Super+L",
    "Super+D",
];

/// Why `key` can't be the quick search's at all, if it can't: it isn't a key with a
/// modifier, the system keeps it, or one of featherplay's own global hotkeys uses it.
fn refusal(key: &str) -> Option<String> {
    if featherplay_integrations::parse_hotkey(key).is_err() {
        return Some(tr!(
            "{} isn't a key featherplay can use. Pick another key.",
            key
        ));
    }
    if RESERVED.iter().any(|r| same_hotkey(r, key)) {
        return Some(tr!(
            "{} belongs to the system. Pick another key.",
            key_text(key)
        ));
    }
    let hotkeys = SETTINGS.with(|s| s.borrow().hotkeys.clone());
    let (_, action) = hotkeys.as_ref().and_then(|h| {
        [
            (h.play_pause.as_str(), tr!("Play or pause")),
            (h.next.as_str(), tr!("Next")),
            (h.previous.as_str(), tr!("Previous")),
        ]
        .into_iter()
        .find(|(k, _)| same_hotkey(k, key))
    })?;
    Some(tr!(
        "featherplay already uses {0} for {1}.",
        key_text(key),
        action
    ))
}

/// `key` was pressed in the field, or Reset puts the default back. It's saved only once it
/// has registered: one that can't be the quick search's (`refusal`) or that another app
/// holds is refused with a line saying why, and the key before stays.
fn picked(ctx: &Rc<Ctx>, key: &str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<QuickKeyData>();
    let (stored, hotkeys) = SETTINGS.with(|s| {
        let s = s.borrow();
        (s.quick_search_key.clone(), s.hotkeys.clone())
    });
    let before = quick_search::usable_key(&stored).to_string();
    if data.get_listening() {
        data.set_listening(false);
        give_back_hotkeys(hotkeys.as_ref());
    }
    let refused = refusal(key).or_else(|| {
        quick_search::set_key(Some(key))
            .err()
            .map(|e| error_text(key, &e))
    });
    if let Some(line) = refused {
        log!("quick search: {key} refused");
        register(ctx, &before);
        data.set_error(line.into());
        return;
    }
    show_key(&data, key);
    data.set_error(SharedString::new());
    if stored != key {
        commit(ctx, Change::QuickSearchKey(key.to_string()));
    }
}

/// Registers `key` and shows it, with the line under it if it couldn't be: featherplay's own
/// global hotkeys aren't registered twice.
fn register(ctx: &Ctx, key: &str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<QuickKeyData>();
    show_key(&data, key);
    if let Some(line) = refusal(key) {
        let _ = quick_search::set_key(None);
        data.set_error(line.into());
        return;
    }
    let error = match quick_search::set_key(Some(key)) {
        Ok(()) => String::new(),
        Err(e) => error_text(key, &e),
    };
    data.set_error(error.into());
}

fn error_text(key: &str, e: &KeyError) -> String {
    match e {
        KeyError::Taken(_) => tr!(
            "Another app already uses {}. Pick another key.",
            key_text(key)
        ),
        // `refusal` catches these first.
        KeyError::Invalid(_) => tr!("{} isn't a key featherplay can use. Pick another key.", key),
    }
}

/// The hotkey text for a key pressed in the field (`text` as Slint's key event has it) with
/// these modifiers: "Ctrl+Shift+Space". `None` for a key without Ctrl, Alt, Shift or Win
/// (it isn't taken), and for one a hotkey can't be (a modifier alone, Esc, Tab).
pub fn capture_text(text: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Option<String> {
    if !(ctrl || alt || shift || meta) {
        return None;
    }
    let named = |k: Key| SharedString::from(k) == text;
    let key = if named(Key::Space) || text == " " {
        "Space".to_string()
    } else if named(Key::Return) {
        "Enter".to_string()
    } else if named(Key::Backspace) {
        "Backspace".to_string()
    } else if named(Key::Delete) {
        "Delete".to_string()
    } else if named(Key::Insert) {
        "Insert".to_string()
    } else if named(Key::Home) {
        "Home".to_string()
    } else if named(Key::End) {
        "End".to_string()
    } else if named(Key::PageUp) {
        "PageUp".to_string()
    } else if named(Key::PageDown) {
        "PageDown".to_string()
    } else if named(Key::UpArrow) {
        "Up".to_string()
    } else if named(Key::DownArrow) {
        "Down".to_string()
    } else if named(Key::LeftArrow) {
        "Left".to_string()
    } else if named(Key::RightArrow) {
        "Right".to_string()
    } else if let Some(n) = function_key(text) {
        format!("F{n}")
    } else {
        let mut chars = text.chars();
        let c = chars.next()?;
        if chars.next().is_some() || !(c.is_ascii_alphanumeric() || "`-=[]\\;',./".contains(c)) {
            return None;
        }
        c.to_ascii_uppercase().to_string()
    };
    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    if meta {
        parts.push("Super");
    }
    Some(format!("{}+{key}", parts.join("+")))
}

/// F1 to F24 by Slint's key text.
fn function_key(text: &str) -> Option<u8> {
    const KEYS: [Key; 24] = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
        Key::F13,
        Key::F14,
        Key::F15,
        Key::F16,
        Key::F17,
        Key::F18,
        Key::F19,
        Key::F20,
        Key::F21,
        Key::F22,
        Key::F23,
        Key::F24,
    ];
    KEYS.iter()
        .position(|k| SharedString::from(*k) == text)
        .map(|i| i as u8 + 1)
}

/// `--snapshot settings-quick-search…`: the switch on with `key`, the field listening with
/// Ctrl and Shift held, or the key taken by another app.
pub fn show_for_snapshot(ctx: &Rc<Ctx>, key: &str, state: &str) {
    let Some(ui) = ctx.window() else { return };
    SETTINGS.with(|s| {
        let mut s = s.borrow_mut();
        s.quick_search = true;
        s.quick_search_key = key.to_string();
    });
    if state == "taken" {
        quick_search::register_keys_with(|_| Err(KeyError::Taken("held by another app".into())));
    }
    let data = ui.global::<QuickKeyData>();
    data.set_on(true);
    register(ctx, key);
    if state == "listening" {
        data.set_listening(true);
        data.set_held_ctrl(true);
        data.set_held_shift(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pressed_combination_becomes_the_hotkeys_text() {
        assert_eq!(
            capture_text("k", true, true, false, false).as_deref(),
            Some("Ctrl+Alt+K")
        );
        assert_eq!(
            capture_text(" ", true, false, true, false).as_deref(),
            Some("Ctrl+Shift+Space")
        );
        let f5: SharedString = Key::F5.into();
        assert_eq!(
            capture_text(&f5, false, false, false, true).as_deref(),
            Some("Super+F5")
        );
        let ret: SharedString = Key::Return.into();
        assert_eq!(
            capture_text(&ret, true, true, false, false).as_deref(),
            Some("Ctrl+Alt+Enter")
        );
        // Every one of them is a key the hotkeys take.
        for k in [
            "Ctrl+Alt+K",
            "Ctrl+Shift+Space",
            "Super+F5",
            "Ctrl+Alt+Enter",
        ] {
            assert!(featherplay_integrations::parse_hotkey(k).is_ok(), "{k}");
        }
    }

    #[test]
    fn a_key_without_a_modifier_or_a_modifier_alone_isnt_taken() {
        assert_eq!(capture_text("k", false, false, false, false), None);
        let ctrl: SharedString = Key::Control.into();
        assert_eq!(capture_text(&ctrl, true, false, false, false), None);
        let esc: SharedString = Key::Escape.into();
        assert_eq!(capture_text(&esc, true, false, false, false), None);
    }
}
