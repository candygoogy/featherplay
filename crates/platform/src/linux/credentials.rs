//! The secret store behind `keyring-core`. None until phase 5: no secret is kept.

use std::sync::Arc;

use keyring_core::{CredentialStore, Error};

use crate::{Feature, Unsupported};

/// The modifiers every entry is made with: none.
pub const MODIFIERS: &[(&str, &str)] = &[];

/// The most bytes one entry's secret holds. Windows' limit, so a value splits the same way
/// on every OS.
pub const MAX_SECRET_BYTES: usize = 2560;

/// The OS's credential store. Always an error for now: there's none yet.
pub fn store() -> Result<Arc<CredentialStore>, Error> {
    Err(Error::NoStorageAccess(Box::new(Unsupported(
        Feature::SecretStorage,
    ))))
}
