//! The Credential Manager store behind `keyring-core`, and its size limits.
//!
//! Credential Manager encrypts each entry with the user's DPAPI key, so other Windows
//! users can't read it, but any program running as this user can. Entries are written
//! with `Local` persistence (this computer only) rather than the Windows store's default,
//! `Enterprise`, which copies them to other computers along with a roaming profile.

use std::sync::Arc;

use keyring_core::{CredentialStore, Error};

/// The modifiers every entry is made with: the Windows store's `persistence` set to
/// `Local` (`CRED_PERSIST_LOCAL_MACHINE`).
pub const MODIFIERS: &[(&str, &str)] = &[("persistence", "Local")];

/// The most bytes one entry's secret holds: `CRED_MAX_CREDENTIAL_BLOB_SIZE`.
pub const MAX_SECRET_BYTES: usize = 2560;

/// The OS's credential store, Credential Manager, to make `keyring-core`'s default.
pub fn store() -> Result<Arc<CredentialStore>, Error> {
    Ok(windows_native_keyring_store::Store::new()?)
}
