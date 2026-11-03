//! Global hotkeys from `settings.hotkeys`, e.g. `"Ctrl+Alt+Space"`.

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::{Sender, UiCommand};

/// Parses `"Ctrl+Alt+Space"`-style text. A key without a modifier is refused: as a global
/// hotkey it would swallow that key in every other app.
pub fn parse_hotkey(s: &str) -> Result<HotKey, String> {
    let key: HotKey = s.trim().parse().map_err(|e| format!("hotkey {s:?}: {e}"))?;
    if key.mods.is_empty() {
        return Err(format!(
            "hotkey {s:?} needs a modifier (Ctrl, Alt, Shift or Super)"
        ));
    }
    Ok(key)
}

/// The registered hotkeys. They stay registered until this is dropped. Create it on the
/// UI thread: the hotkey window belongs to the thread that creates it.
pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    keys: Vec<HotKey>,
}

impl Hotkeys {
    /// Registers all three hotkeys, or none: any parse or registration failure (a key
    /// another app already holds) unregisters the rest and returns the error.
    pub fn register(
        cfg: &featherplay_store::Hotkeys,
        tx: Sender<UiCommand>,
    ) -> Result<Self, String> {
        let wanted = [
            (cfg.play_pause.as_str(), UiCommand::PlayPause),
            (cfg.next.as_str(), UiCommand::Next),
            (cfg.previous.as_str(), UiCommand::Previous),
        ];
        let mut bindings = Vec::with_capacity(wanted.len());
        for (text, cmd) in wanted {
            bindings.push((parse_hotkey(text)?, cmd));
        }
        let manager = GlobalHotKeyManager::new().map_err(|e| format!("hotkey manager: {e}"))?;
        let mut keys = Vec::with_capacity(bindings.len());
        for (key, _) in &bindings {
            if let Err(e) = manager.register(*key) {
                let _ = manager.unregister_all(&keys);
                return Err(format!("registering {key}: {e}"));
            }
            keys.push(*key);
        }
        let ids: Vec<(u32, UiCommand)> = bindings.iter().map(|(k, c)| (k.id(), *c)).collect();
        GlobalHotKeyEvent::set_event_handler(Some(move |ev: GlobalHotKeyEvent| {
            if ev.state() != HotKeyState::Pressed {
                return;
            }
            if let Some((_, cmd)) = ids.iter().find(|(id, _)| *id == ev.id()) {
                let _ = tx.send(*cmd);
            }
        }));
        Ok(Hotkeys { manager, keys })
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        GlobalHotKeyEvent::set_event_handler(None::<fn(GlobalHotKeyEvent)>);
        let _ = self.manager.unregister_all(&self.keys);
    }
}
