//! Finding and starting the sign-in browsers: stubs until phase 5. `ThisPc` finds none, and
//! [`launch`] says it's `Unsupported`. The browsers themselves are in `common`.

use std::convert::Infallible;
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

pub use crate::common::browsers::*;
use crate::window::Handle;

/// This PC, where no browser is looked for yet: every lookup comes back empty.
pub struct ThisPc;

impl Lookup for ThisPc {
    fn app_path(&self, _hive: Hive, _exe: &str) -> Option<String> {
        None
    }

    fn env(&self, _var: &str) -> Option<OsString> {
        None
    }

    fn is_file(&self, _path: &Path) -> bool {
        false
    }

    fn dword(&self, _hive: Hive, _key: &str, _value: &str) -> Option<u32> {
        None
    }

    fn https_handler(&self) -> Option<PathBuf> {
        None
    }
}

/// A running browser. Never started on Linux yet, so none of its methods can be reached.
pub struct Running {
    pub pid: u32,
    never: Infallible,
}

impl Running {
    pub fn exited(&self) -> bool {
        match self.never {}
    }

    pub fn wait(&self, _limit: std::time::Duration) -> bool {
        match self.never {}
    }

    pub fn kill(&self) {
        match self.never {}
    }
}

/// Dropping a running browser ends it, as on Windows (whose job ends with its handle): callers
/// drop it on purpose.
impl Drop for Running {
    fn drop(&mut self) {
        match self.never {}
    }
}

/// The app's ends of the two pipes.
pub struct Pipes {
    pub to_browser: File,
    pub from_browser: File,
}

/// Would start `exe` with the DevTools pipe. Always `Unsupported` for now.
pub fn launch(_exe: &Path, _args: &[OsString]) -> io::Result<(Running, Pipes)> {
    Err(io::ErrorKind::Unsupported.into())
}

/// The visible top-level windows of the process `pid`: none, as no browser is started.
pub fn visible_windows(_pid: u32) -> Vec<Handle> {
    Vec::new()
}

/// Would ask `window` to close; there is none.
pub fn close_window(_window: Handle) {}
