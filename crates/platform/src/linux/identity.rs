//! The app's identity: its desktop entry, written in phase 5. Until then nothing is
//! registered: `register` and `unregister` say so.

use std::path::{Path, PathBuf};

pub use crate::common::identity::*;

/// Why registering fails here.
const NOT_YET: &str = "registering the app isn't supported on this system";

/// Nothing to name: the window's app ID comes with the window.
pub fn set_process_app_id() -> Result<(), String> {
    Ok(())
}

impl Registration {
    /// Always an error: there's no registration here yet.
    pub fn for_current_user(_exe: PathBuf, _data_dir: &Path) -> Result<Registration, String> {
        Err(NOT_YET.into())
    }
}

/// Always an error: there's no registration here yet.
pub fn register(_reg: &Registration, _icon: &[u8]) -> Result<Changes, String> {
    Err(NOT_YET.into())
}

/// Always an error: there's no registration here yet.
pub fn unregister(_reg: &Registration) -> Result<(), String> {
    Err(NOT_YET.into())
}
