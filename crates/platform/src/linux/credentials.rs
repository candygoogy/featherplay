//! The Secret Service store behind `keyring-core`: GNOME Keyring, KWallet, or whichever
//! program owns `org.freedesktop.secrets` on the session bus (`zbus-secret-service-keyring-store`).
//!
//! Items go in the default collection (the login keyring), which the desktop unlocks at
//! login; a locked one makes the service ask for its password itself. Any program running as
//! this user can read them, as on Windows.
//!
//! The service is reached once, on first use, and the answer kept for the run: with no
//! session bus, or no Secret Service on it, [`store`] is an error that says so, every secret
//! is refused, and the app keeps a sign-in for this run only (in memory: nothing is ever
//! written to a plain file). [`available`] is that answer, and
//! `supports(Feature::SecretStorage)` reads it.

use std::sync::{Arc, OnceLock};

use keyring_core::{CredentialStore, Error};
use zbus_secret_service_keyring_store::Store;

/// The modifiers every entry is made with: none (items get the store's label,
/// `keyring:{user}@{service}`, and go in the default collection).
pub const MODIFIERS: &[(&str, &str)] = &[];

/// The most bytes one entry's secret holds. Windows' limit, so a value splits the same way
/// on every OS.
pub const MAX_SECRET_BYTES: usize = 2560;

/// The Secret Service, reached on first use; or why it couldn't be.
fn opened() -> &'static Result<Arc<Store>, String> {
    static STORE: OnceLock<Result<Arc<Store>, String>> = OnceLock::new();
    STORE.get_or_init(|| {
        Store::new().map_err(|e| {
            let why = no_service(&e);
            log::warn!("{why}");
            why
        })
    })
}

/// What the log and Copy diagnostics say when there's no store: the service's own error,
/// which holds no secret (no secret has been sent yet).
fn no_service(e: &Error) -> String {
    format!("no Secret Service (GNOME Keyring, KWallet) on the session bus: {e}")
}

/// The OS's credential store, the Secret Service, to make `keyring-core`'s default; an
/// error with the reason when there's none.
pub fn store() -> Result<Arc<CredentialStore>, Error> {
    match opened() {
        Ok(store) => Ok(store.clone()),
        Err(why) => Err(Error::NoStorageAccess(why.clone().into())),
    }
}

/// A Secret Service answered on the session bus: secrets are kept across runs.
pub fn available() -> bool {
    opened().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use keyring_core::Entry;

    /// A dummy entry through the real Secret Service: stored, read back, deleted. The
    /// service and user are this test's own, so nothing of the app's is touched.
    /// `cargo nextest run -p featherplay-platform --lib --run-ignored only -E 'test(live_secret_service)'`
    #[test]
    #[ignore = "writes a dummy item to the session's Secret Service"]
    fn live_secret_service_stores_reads_and_deletes_a_dummy_entry() {
        assert!(available(), "no Secret Service: {:?}", store().err());
        // As the store crate makes its entries: keyring-core's default store, the modifiers.
        keyring_core::set_default_store(store().unwrap());
        let modifiers: HashMap<&str, &str> = MODIFIERS.iter().copied().collect();
        let entry = || Entry::new_with_modifiers("featherplay-test", "dummy", &modifiers).unwrap();
        let secret = "k=v; ".repeat(400) + "é";
        entry().set_secret(secret.as_bytes()).unwrap();
        assert_eq!(entry().get_secret().unwrap(), secret.as_bytes());
        // A second write replaces the first, as on Windows.
        entry().set_secret(b"short").unwrap();
        assert_eq!(entry().get_secret().unwrap(), b"short");
        entry().delete_credential().unwrap();
        assert!(matches!(entry().get_secret(), Err(Error::NoEntry)));
        assert!(matches!(entry().delete_credential(), Err(Error::NoEntry)));
    }

    /// The test below runs in a child process of this test program, so the session bus it
    /// can't reach is that process's alone.
    #[test]
    fn with_no_session_bus_nothing_is_kept_and_the_error_says_why() {
        let exe = std::env::current_exe().unwrap();
        let out = std::process::Command::new(exe)
            .args([
                "linux::credentials::tests::without_a_bus",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/featherplay-test-bus",
            )
            .env_remove("XDG_RUNTIME_DIR")
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{said}");
        // The child really ran the test (a filter that matches nothing passes too).
        assert!(said.contains("1 passed"), "{said}");
    }

    #[test]
    #[ignore = "run by the test above, with a session bus that isn't there"]
    fn without_a_bus() {
        assert!(!available());
        assert!(!crate::supports(crate::Feature::SecretStorage));
        let Err(e) = store() else {
            panic!("a store without a session bus")
        };
        assert!(matches!(e, Error::NoStorageAccess(_)), "{e:?}");
        assert!(e.to_string().contains("no Secret Service"), "{e}");
    }
}
