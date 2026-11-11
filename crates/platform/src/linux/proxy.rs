//! The system proxy lookup. A stub until phase 5: no settings are read and no script is
//! asked, so requests go by the environment's proxy alone.

/// What the system's proxy settings say to ask. No `Debug`: the script's address may name
/// a user or carry a key.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// Proxy auto-detection is on.
    pub auto_detect: bool,
    /// A proxy script is set: its address.
    pub script: Option<String>,
}

/// The system's proxy settings. `None`: not read on Linux yet.
pub fn settings() -> Option<Settings> {
    None
}

/// The proxies for `url` from a script or auto-detection. `Err(0)`: not looked up on
/// Linux yet.
pub fn ask(_url: &str, _script: Option<&str>) -> Result<Option<String>, u32> {
    Err(0)
}
