//! The tray icon: a StatusNotifierItem on the session bus (ksni, no GTK), which the desktop's
//! tray shows (KDE Plasma, GNOME with the AppIndicator extension, most panels). The desktop
//! draws its own menu from the item's (`menu`), with the same entries as on Windows; their
//! choices come back as `TrayEvent::Chose`. The desktop's activation (a left click, mostly)
//! is a `TrayEvent::Click` at the point it gives, else at the pointer (the app opens its
//! mini player, or the window); its secondary one (a middle click) a `TrayEvent::DoubleClick`
//! (the window comes back). There is no hover: the desktop reads the hover text when it
//! shows it, so the text has no time in it (`TIMED_TOOLTIP`).
//!
//! No tray: without a StatusNotifierWatcher on the session bus (GNOME without the extension,
//! WSL), or without a session bus, `Tray::new` is an error and `supports(Feature::Tray)` is
//! false. A watcher that goes away later (the shell restarting, the extension switched off)
//! is a `TrayEvent::Shown(false)`; the item stays on the bus and registers again when one
//! comes back (`TrayEvent::Shown(true)`). `supports(Feature::Tray)` follows both.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use featherplay_core::tr;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use ksni::{Icon, MenuItem, OfflineReason, ToolTip};

pub use crate::common::tray::*;

use crate::common::tray::drawings;

/// Whether the hover text may say where the song is: no, the desktop reads it whenever it
/// likes (Windows brings it up to date as the pointer comes onto the icon).
pub const TIMED_TOOLTIP: bool = false;

/// The item's id and title, which desktops show in their lists of tray items.
const NAME: &str = "featherplay";

/// Whether a desktop's tray shows the icon now: `supports(Feature::Tray)`.
static SHOWN: AtomicBool = AtomicBool::new(false);

/// Whether a desktop's tray shows the icon now (a `Tray` exists, and the watcher has it).
pub(crate) fn available() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

/// The app's handler for the icon's events (called from ksni's thread).
type OnEvent = Arc<dyn Fn(TrayEvent) + Send + Sync>;

/// What the desktop reads from the item: ksni asks for it on its own thread, and again after
/// each `Handle::update`.
struct Item {
    on_event: OnEvent,
    /// Every size of the drawing, as ARGB32 for the desktop to pick from.
    icons: Vec<Icon>,
    /// The hover text's two lines.
    tip: (String, String),
    state: MenuState,
}

pub struct Tray {
    handle: Handle<Item>,
    /// The hover text and menu state last sent, so an unchanged one costs no D-Bus traffic.
    tip: std::cell::RefCell<String>,
    state: std::cell::RefCell<MenuState>,
}

/// The drawing with the crimson ring round the play: it reads on a light panel and a dark
/// one alike, and nothing says which the desktop's is.
fn icons() -> Vec<Icon> {
    drawings(true)
        .into_iter()
        .map(|(px, rgba)| Icon {
            width: px as i32,
            height: px as i32,
            data: argb(&rgba),
        })
        .collect()
}

/// Straight-alpha RGBA as the item's ARGB32: each pixel's bytes in network order, A R G B.
fn argb(rgba: &[u8]) -> Vec<u8> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| [a, r, g, b])
        .collect()
}

/// The hover text's first line, and the rest.
fn split_tip(s: &str) -> (String, String) {
    match s.split_once('\n') {
        Some((title, rest)) => (title.to_string(), rest.to_string()),
        None => (s.to_string(), String::new()),
    }
}

/// A label as a menu over D-Bus shows it: "_" marks the next letter as the access key, so a
/// plain one is doubled.
fn label(s: &str) -> String {
    s.replace('_', "__")
}

impl Item {
    fn send(&self, ev: TrayEvent) {
        (self.on_event)(ev);
    }

    fn choice(c: TrayChoice) -> Box<dyn Fn(&mut Item) + Send> {
        Box::new(move |item: &mut Item| item.send(TrayEvent::Chose(c)))
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        NAME.into()
    }

    fn title(&self) -> String {
        NAME.into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.tip.0.clone(),
            description: self.tip.1.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, x: i32, y: i32) {
        // Hosts that don't know the point send 0, 0.
        let (x, y) = if (x, y) == (0, 0) {
            crate::window::pointer().unwrap_or((0, 0))
        } else {
            (x, y)
        };
        self.send(TrayEvent::Click { x, y });
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayEvent::DoubleClick);
    }

    fn menu_about_to_show(&mut self) {
        self.send(TrayEvent::Menu);
    }

    /// The menu: the song (greyed), Pause or Play, Previous, Next; Like (checked when liked)
    /// and Not for me; the sleep timer's submenu while its setting is on; while offered,
    /// Desktop lyrics and its Lock or Unlock; Show featherplay and Quit, as on Windows
    /// (`windows::tray::menu`).
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let l = labels(&self.state);
        let item = |c: TrayChoice, text: &str, enabled: bool| -> MenuItem<Self> {
            StandardItem {
                label: label(text),
                enabled,
                activate: Item::choice(c),
                ..Default::default()
            }
            .into()
        };
        let sleep: Option<MenuItem<Self>> = l.sleep.as_ref().map(|s| {
            let submenu = s
                .entries
                .iter()
                .map(|e| match e {
                    Some((c, text)) => item(*c, text, true),
                    None => MenuItem::Separator,
                })
                .collect();
            SubMenu {
                label: label(&s.item),
                enabled: s.enabled,
                submenu,
                ..Default::default()
            }
            .into()
        });
        let mut menu = vec![
            StandardItem {
                label: label(&header_text(&self.state)),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            item(TrayChoice::PlayPause, l.play, l.controls),
            item(TrayChoice::Previous, tr!("Previous"), l.controls),
            item(TrayChoice::Next, tr!("Next"), l.controls),
            MenuItem::Separator,
            CheckmarkItem {
                label: label(l.like),
                enabled: l.controls,
                checked: l.liked,
                activate: Item::choice(TrayChoice::Like),
                ..Default::default()
            }
            .into(),
            item(TrayChoice::NotForMe, tr!("Not for me"), l.controls),
        ];
        if let Some(sleep) = sleep {
            menu.extend([MenuItem::Separator, sleep]);
        }
        menu.push(MenuItem::Separator);
        let desktop = self.state.desktop_lyrics;
        if desktop.offered {
            menu.push(
                CheckmarkItem {
                    label: label(tr!("Desktop lyrics")),
                    checked: desktop.on,
                    activate: Item::choice(TrayChoice::DesktopLyrics),
                    ..Default::default()
                }
                .into(),
            );
            if let Some(lock) = desktop.lock_label() {
                menu.push(item(TrayChoice::LockDesktopLyrics, lock, true));
            }
            menu.push(MenuItem::Separator);
        }
        menu.push(item(TrayChoice::Show, tr!("Show featherplay"), true));
        menu.push(item(TrayChoice::Quit, tr!("Quit"), true));
        menu
    }

    fn watcher_online(&self) {
        log::info!("tray: the desktop's tray is back");
        SHOWN.store(true, Ordering::Relaxed);
        self.send(TrayEvent::Shown(true));
    }

    /// Stays on the bus, to register again if a watcher comes back.
    fn watcher_offline(&self, reason: OfflineReason) -> bool {
        log::warn!("tray: the desktop's tray went away ({reason:?})");
        SHOWN.store(false, Ordering::Relaxed);
        self.send(TrayEvent::Shown(false));
        true
    }
}

impl Tray {
    /// Puts the item on the session bus and registers it with the desktop's tray. An error
    /// when there's no session bus or no tray on it. `on_event` is called from ksni's thread
    /// (so it should only post the event on). `tip` is the first hover text.
    pub fn new(
        tip: &str,
        on_event: impl Fn(TrayEvent) + Send + Sync + 'static,
    ) -> Result<Tray, String> {
        let state = MenuState::default();
        let item = Item {
            on_event: Arc::new(on_event),
            icons: icons(),
            tip: split_tip(tip),
            state: state.clone(),
        };
        let handle = item
            .spawn()
            .map_err(|e| format!("no tray on this desktop: {e}"))?;
        SHOWN.store(true, Ordering::Relaxed);
        Ok(Tray {
            handle,
            tip: std::cell::RefCell::new(tip.to_string()),
            state: std::cell::RefCell::new(state),
        })
    }

    /// Nothing to do: the desktop scales the icon, and the drawing reads on any panel.
    pub fn refresh_icon(&self) {}

    /// Sets the hover text: its first line is the tooltip's title, the rest its text.
    /// Nothing is sent when it's unchanged.
    pub fn set_tooltip(&self, s: &str) {
        if *self.tip.borrow() == s {
            return;
        }
        s.clone_into(&mut self.tip.borrow_mut());
        let tip = split_tip(s);
        self.handle.update(move |item| item.tip = tip);
    }

    /// Sets the menu's labels for the player's state; nothing is sent when it's unchanged.
    pub fn set_menu(&self, s: &MenuState) {
        if *self.state.borrow() == *s {
            return;
        }
        s.clone_into(&mut self.state.borrow_mut());
        let s = s.clone();
        self.handle.update(move |item| item.state = s);
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        SHOWN.store(false, Ordering::Relaxed);
        self.handle.shutdown().wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_argb_in_network_order() {
        assert_eq!(argb(&[1, 2, 3, 4, 5, 6, 7, 8]), [4, 1, 2, 3, 8, 5, 6, 7]);
        let icons = icons();
        assert_eq!(icons.len(), 7);
        for i in icons {
            assert_eq!(i.data.len(), (i.width * i.height * 4) as usize);
        }
    }

    #[test]
    fn labels_keep_their_underscores_and_ampersands() {
        assert_eq!(label("snake_case & co"), "snake__case & co");
        assert_eq!(
            split_tip("Title\nArtist"),
            ("Title".into(), "Artist".into())
        );
    }

    /// A stand-in for the desktop's tray: it keeps the names of the items that register.
    struct Watcher {
        items: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        fn register_status_notifier_item(&self, service: String) {
            self.items.lock().expect("items").push(service);
        }

        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            true
        }
    }

    /// On the session bus, with a stand-in watcher (skipped without a session bus, or when the
    /// desktop's own tray holds the watcher's name): the item registers, its menu lists the
    /// entries, the desktop's activation and a menu choice reach the app, and the watcher going
    /// away is `Shown(false)`.
    #[test]
    fn the_item_registers_lists_its_menu_and_hears_the_desktop() {
        use std::collections::HashMap;
        use std::sync::mpsc;
        use std::time::Duration;

        use zbus::blocking::{Connection, Proxy};
        use zbus::fdo::RequestNameFlags;
        use zbus::zvariant::{OwnedValue, Value};

        const WATCHER: &str = "org.kde.StatusNotifierWatcher";
        let Ok(bus) = Connection::session() else {
            eprintln!("skipped: no session bus");
            return;
        };
        let items = Arc::new(std::sync::Mutex::new(Vec::new()));
        let watcher = Watcher {
            items: items.clone(),
        };
        bus.object_server()
            .at("/StatusNotifierWatcher", watcher)
            .expect("serve the watcher");
        if bus
            .request_name_with_flags(WATCHER, RequestNameFlags::DoNotQueue.into())
            .is_err()
        {
            eprintln!("skipped: the desktop's tray owns {WATCHER}");
            return;
        }

        let (tx, rx) = mpsc::channel();
        let tray = Tray::new("featherplay\nNothing playing", move |ev| {
            let _ = tx.send(ev);
        })
        .expect("the item registers");
        assert!(crate::supports(crate::Feature::Tray));
        let name = items
            .lock()
            .expect("items")
            .first()
            .cloned()
            .expect("an item");
        let next = || rx.recv_timeout(Duration::from_secs(5)).expect("an event");

        let menu = Proxy::new(&bus, name.as_str(), "/MenuBar", "com.canonical.dbusmenu")
            .expect("menu proxy");
        let entries: Vec<(i32, HashMap<String, OwnedValue>)> = menu
            .call("GetGroupProperties", &(Vec::<i32>::new(), vec!["label"]))
            .expect("the menu's labels");
        let labels: Vec<(i32, String)> = entries
            .into_iter()
            .filter_map(|(id, mut props)| {
                Some((id, String::try_from(props.remove("label")?).ok()?))
            })
            .filter(|(_, l)| !l.is_empty())
            .collect();
        let texts: Vec<&str> = labels.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Nothing playing",
                "Play",
                "Previous",
                "Next",
                "Like",
                "Not for me",
                "Show featherplay",
                "Quit"
            ]
        );

        let item = Proxy::new(
            &bus,
            name.as_str(),
            "/StatusNotifierItem",
            "org.kde.StatusNotifierItem",
        )
        .expect("item proxy");
        let () = item.call("Activate", &(10i32, 20i32)).expect("Activate");
        assert_eq!(next(), TrayEvent::Click { x: 10, y: 20 });
        let () = item
            .call("SecondaryActivate", &(0i32, 0i32))
            .expect("SecondaryActivate");
        assert_eq!(next(), TrayEvent::DoubleClick);

        let show = labels
            .iter()
            .find(|(_, l)| l == "Show featherplay")
            .map(|(id, _)| *id)
            .expect("Show featherplay");
        let () = menu
            .call("Event", &(show, "clicked", Value::from(0i32), 0u32))
            .expect("a click on Show featherplay");
        assert_eq!(next(), TrayEvent::Chose(TrayChoice::Show));

        bus.release_name(WATCHER).expect("release the watcher");
        assert_eq!(next(), TrayEvent::Shown(false));
        assert!(!crate::supports(crate::Feature::Tray));
        drop(tray);
    }
}
