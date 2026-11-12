//! The session D-Bus: one connection for the run, which the notifications use and MPRIS is
//! served on. zbus runs its own thread for it, so calls work from any thread with no
//! async runtime around; they block, so make them off the UI thread where it matters.

use std::sync::OnceLock;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::blocking::fdo::DBusProxy;
use zbus::names::BusName;

/// How long a call may wait for its answer: a service that hangs doesn't hold a thread for
/// longer.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// The session bus, connected on first use; None (logged once) if there's none.
pub(crate) fn session() -> Option<&'static Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| {
        Builder::session()
            .map(|b| b.method_timeout(CALL_TIMEOUT))
            .and_then(Builder::build)
            .inspect_err(|e| log::warn!("no session D-Bus: {e}"))
            .ok()
    })
    .as_ref()
}

/// Whether `name` is on the bus: a program owns it, or the bus can start one that will
/// (D-Bus activation, as most notification daemons are started).
pub(crate) fn has_service(bus: &Connection, name: &str) -> bool {
    let answer = || -> zbus::Result<bool> {
        let dbus = DBusProxy::new(bus)?;
        Ok(dbus.name_has_owner(BusName::try_from(name)?)?
            || dbus
                .list_activatable_names()?
                .iter()
                .any(|n| n.as_str() == name))
    };
    answer()
        .inspect_err(|e| log::warn!("D-Bus: is {name} there? {e}"))
        .unwrap_or(false)
}
