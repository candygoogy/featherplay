//! Notifications through the desktop's notification server: `org.freedesktop.Notifications`
//! on the session bus (GNOME Shell, KDE Plasma, dunst, mako and the rest). Like the Windows
//! toast: silent, a title and one line of text, no buttons.
//!
//! Each notification replaces the one before it, so a run of track changes leaves one
//! notification in the desktop's list rather than a pile.
//!
//! With no session bus, or no server on it (WSLg has none), [`available`] is false, which
//! `supports(Feature::Notifications)` reads and the settings page hides the switch for, and
//! `notify` shows nothing, as a missing toast on Windows only logs.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use zbus::zvariant::Value;

use crate::common::identity::DISPLAY_NAME;
use crate::linux::bus;

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// The desktop entry the notifications name as theirs (the `desktop-entry` hint), so the
/// desktop can show the app's icon and per-app settings for them.
const DESKTOP_ENTRY: &str = crate::APP_ID;

/// The server's ID of the last notification shown, which the next one replaces; 0 for none.
static LAST: AtomicU32 = AtomicU32::new(0);

/// What the notification server said about itself, asked once.
struct Server {
    /// It reads the body as markup (`body-markup`): `&`, `<` and `>` must be escaped.
    markup: bool,
}

/// The server on the session bus; None (logged once) when there's none.
fn server() -> Option<&'static Server> {
    static SERVER: OnceLock<Option<Server>> = OnceLock::new();
    SERVER
        .get_or_init(|| {
            let bus = bus::session()?;
            if !bus::has_service(bus, SERVICE) {
                log::warn!("no notification server ({SERVICE}) on the session bus");
                return None;
            }
            let caps: Vec<String> = bus
                .call_method(Some(SERVICE), PATH, Some(SERVICE), "GetCapabilities", &())
                .and_then(|reply| reply.body().deserialize())
                .inspect_err(|e| log::warn!("notifications: capabilities: {e}"))
                .unwrap_or_default();
            Some(Server {
                markup: caps.iter().any(|c| c == "body-markup"),
            })
        })
        .as_ref()
}

/// A notification server is there: notifications show. Asked on the session bus the first
/// time (a few milliseconds), then kept for the run.
pub fn available() -> bool {
    server().is_some()
}

/// Shows a silent notification with a title and one line of text, replacing the last one.
/// Blocks for a D-Bus round trip (the first call also asks the server what it supports), so
/// call it off the UI thread. Errors are logged, not returned: a missing notification is not
/// worth interrupting playback for.
pub fn notify(title: &str, text: &str) {
    let (Some(bus), Some(server)) = (bus::session(), server()) else {
        return;
    };
    let body = if server.markup {
        escaped(text)
    } else {
        text.to_string()
    };
    let hints = HashMap::from([
        ("suppress-sound", Value::from(true)),
        ("desktop-entry", Value::from(DESKTOP_ENTRY)),
    ]);
    let no_actions: &[&str] = &[];
    // The server's default time on screen, as the Windows toast's short duration is.
    let expire_default = -1i32;
    let args = (
        DISPLAY_NAME,
        LAST.load(Ordering::Relaxed),
        "",
        title,
        body.as_str(),
        no_actions,
        hints,
        expire_default,
    );
    let shown = bus
        .call_method(Some(SERVICE), PATH, Some(SERVICE), "Notify", &args)
        .and_then(|reply| reply.body().deserialize::<u32>());
    match shown {
        Ok(id) => LAST.store(id, Ordering::Relaxed),
        Err(e) => log::warn!("notify: {e}"),
    }
}

/// `text` with the three characters the notification spec's markup gives a meaning escaped.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_with_markup_characters_shows_as_written() {
        assert_eq!(
            escaped("Simon & Garfunkel <live> > 1970"),
            "Simon &amp; Garfunkel &lt;live&gt; &gt; 1970"
        );
        assert_eq!(escaped("plain"), "plain");
    }

    /// Sends one notification to the session's notification server. Run with a server (or
    /// `dbus-monitor "interface='org.freedesktop.Notifications'"` and a stand-in) on the bus:
    /// `cargo nextest run -p featherplay-platform --lib --run-ignored only -E 'test(live_notification)'`
    #[test]
    #[ignore = "needs a notification server on the session bus"]
    fn live_notification_reaches_the_server() {
        assert!(available(), "no notification server on the session bus");
        notify("featherplay test", "Simon & Garfunkel, <Bookends>");
        let first = LAST.load(Ordering::Relaxed);
        assert_ne!(first, 0, "the server gave no ID");
        // The second replaces the first: the server keeps its ID.
        notify("featherplay test", "the second replaces the first");
        assert_eq!(LAST.load(Ordering::Relaxed), first);
    }
}
