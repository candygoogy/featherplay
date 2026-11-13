//! Global hotkeys: the three of `settings.hotkeys` (`Hotkeys`, e.g. `"Ctrl+Alt+Space"`) and
//! the quick search's one (`QuickKey`), each registered on its own, so a taken key of one
//! never drops the other.
//!
//! `global-hotkey` takes one event handler for the whole process, once: the first set stays.
//! So it's installed here once, and it looks each press up in `ROUTES`, which the registered
//! keys add themselves to and leave when they're dropped.

use std::sync::{Arc, Mutex, Once, PoisonError};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::{Sender, UiCommand};

/// What a registered key's press runs, on the thread that registered it (its window's).
type Route = Arc<dyn Fn() + Send + Sync>;

/// The registered keys' ids and what their presses run.
static ROUTES: Mutex<Vec<(u32, Route)>> = Mutex::new(Vec::new());
static HANDLER: Once = Once::new();

/// Installs the one event handler, the first time a key is routed.
fn route(id: u32, on_press: Route) {
    HANDLER.call_once(|| {
        GlobalHotKeyEvent::set_event_handler(Some(|ev: GlobalHotKeyEvent| {
            if ev.state() != HotKeyState::Pressed {
                return;
            }
            // Cloned out first: what it runs may register or drop keys.
            let found = ROUTES
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .find(|(id, _)| *id == ev.id())
                .map(|(_, r)| r.clone());
            if let Some(run) = found {
                run();
            }
        }));
    });
    ROUTES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((id, on_press));
}

fn unroute(ids: &[u32]) {
    ROUTES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|(id, _)| !ids.contains(id));
}

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

/// Whether two hotkey texts name the same keys, however each is spelled ("ctrl+alt+k" and
/// "Ctrl+Alt+KeyK"). Text that doesn't parse is the same as nothing.
pub fn same_hotkey(a: &str, b: &str) -> bool {
    match (parse_hotkey(a), parse_hotkey(b)) {
        (Ok(a), Ok(b)) => a.id() == b.id(),
        _ => false,
    }
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
        for (key, cmd) in bindings {
            let tx = tx.clone();
            route(
                key.id(),
                Arc::new(move || {
                    let _ = tx.send(cmd);
                }),
            );
        }
        Ok(Hotkeys { manager, keys })
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        let ids: Vec<u32> = self.keys.iter().map(HotKey::id).collect();
        unroute(&ids);
        let _ = self.manager.unregister_all(&self.keys);
    }
}

/// Why a `QuickKey` couldn't be registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// The text isn't a key with a modifier (`parse_hotkey`).
    Invalid(String),
    /// The system wouldn't register it: another app holds it.
    Taken(String),
}

/// One hotkey of its own (the quick search's), registered until this is dropped. Create it
/// on the UI thread, as `Hotkeys`.
pub struct QuickKey {
    manager: GlobalHotKeyManager,
    key: HotKey,
}

impl QuickKey {
    /// Registers `text`; each press runs `on_press` on this thread.
    pub fn register(
        text: &str,
        on_press: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, KeyError> {
        let key = parse_hotkey(text).map_err(KeyError::Invalid)?;
        let manager = GlobalHotKeyManager::new()
            .map_err(|e| KeyError::Taken(format!("hotkey manager: {e}")))?;
        manager
            .register(key)
            .map_err(|e| KeyError::Taken(format!("registering {key}: {e}")))?;
        route(key.id(), Arc::new(on_press));
        Ok(QuickKey { manager, key })
    }
}

impl Drop for QuickKey {
    fn drop(&mut self) {
        unroute(&[self.key.id()]);
        let _ = self.manager.unregister(self.key);
    }
}
