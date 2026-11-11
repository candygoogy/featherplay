//! One instance per home: not guarded yet (phase 5 makes it a Unix socket). `claim` says
//! so, and the app starts anyway, as it does on Windows when the mutex can't be made.

use std::ffi::OsStr;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::time::Duration;

use crate::common::identity::AUMID;
use crate::common::instance::fnv1a;

/// Why the guard can't be had here.
const NOT_YET: &str = "one instance per home isn't guarded on this system yet";

/// The guard's key: the app ID, plus a hash of the home when `FEATHERPLAY_HOME` is set (an
/// empty one counts as unset). It holds no path separator.
pub fn key(home: Option<&OsStr>) -> String {
    match home.filter(|h| !h.is_empty()) {
        None => AUMID.to_string(),
        Some(home) => format!("{AUMID}-home-{:016x}", fnv1a(normalised(home).as_bytes())),
    }
}

/// The guard's name for `key`.
pub fn mutex_name(key: &str) -> String {
    format!("featherplay-{key}")
}

/// Always an error: there's no message to ask with here yet.
pub fn show_message(_key: &str) -> Result<u32, String> {
    Err(NOT_YET.into())
}

/// The absolute form of the path, without a trailing separator.
fn normalised(home: &OsStr) -> String {
    let path = std::path::absolute(home).unwrap_or_else(|_| PathBuf::from(home));
    let text = path.to_string_lossy().into_owned();
    text.trim_end_matches('/').to_string()
}

/// Holding the guard, which this OS never gives yet.
pub struct Guard {
    _owning_thread: PhantomData<*const ()>,
}

/// Another featherplay of this home holds the guard, which this OS never says yet.
pub struct Running {
    _none: (),
}

impl Running {
    /// Gives itself back: there's nothing to wait for here.
    pub fn wait(self, _timeout: Duration) -> Result<Guard, Running> {
        Err(self)
    }
}

/// Whether this run is the first of its home.
pub enum Claim {
    First(Guard),
    Running(Running),
}

/// Always an error: the guard isn't made here yet.
pub fn claim(_name: &str) -> Result<Claim, String> {
    Err(NOT_YET.into())
}

/// Always an error: there's no running copy to ask here yet.
pub fn ask_to_show(_key: &str) -> Result<bool, String> {
    Err(NOT_YET.into())
}
