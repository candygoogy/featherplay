//! The session D-Bus: one connection for the run, which the notifications use and MPRIS is
//! served on. zbus runs its own thread for it, so calls work from any thread with no
//! async runtime around; they block, so make them off the UI thread where it matters.

use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;
use zbus::Message;
use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::zvariant::DynamicType;

/// How long a call may wait for its answer: a service that hangs doesn't hold a thread for
/// longer.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

const DBUS: &str = "org.freedesktop.DBus";
const DBUS_PATH: &str = "/org/freedesktop/DBus";

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

/// Waits on this thread for `op`, one of zbus's async calls. zbus's blocking API inlines its
/// wait loop into every call it makes; boxed and through here, the module's calls share one
/// copy for each kind of answer (36 KB less in the Linux release binary, 2026-11-12).
#[inline(never)]
pub(crate) fn wait<T>(op: Pin<Box<dyn Future<Output = zbus::Result<T>> + '_>>) -> zbus::Result<T> {
    zbus::block_on(op)
}

/// Calls `method` of `service`'s `interface` at `path` and waits for the answer.
pub(crate) fn call<B: Serialize + DynamicType>(
    bus: &Connection,
    service: &str,
    path: &str,
    interface: &str,
    method: &str,
    args: &B,
) -> zbus::Result<Message> {
    wait(Box::pin(bus.inner().call_method(
        Some(service),
        path,
        Some(interface),
        method,
        args,
    )))
}

/// Whether `name` is on the bus: a program owns it, or the bus can start one that will
/// (D-Bus activation, as most notification daemons are started).
pub(crate) fn has_service(bus: &Connection, name: &str) -> bool {
    let answer = || -> zbus::Result<bool> {
        let owned: bool = call(bus, DBUS, DBUS_PATH, DBUS, "NameHasOwner", &(name,))?
            .body()
            .deserialize()?;
        if owned {
            return Ok(true);
        }
        let names: Vec<String> = call(bus, DBUS, DBUS_PATH, DBUS, "ListActivatableNames", &())?
            .body()
            .deserialize()?;
        Ok(names.iter().any(|n| n == name))
    };
    answer()
        .inspect_err(|e| log::warn!("D-Bus: is {name} there? {e}"))
        .unwrap_or(false)
}
