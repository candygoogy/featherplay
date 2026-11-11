//! The install location and starting the setup. Stubs: no per-user install is known on
//! Linux, so an update never installs over the running copy
//! (`supports(Feature::InPlaceUpdate)` is false).

use std::io;
use std::path::PathBuf;
use std::process::Command;

use crate::{Feature, Unsupported};

/// The per-user install's folder. `None`: there's no per-user install on Linux yet.
pub fn install_location() -> Option<PathBuf> {
    None
}

/// Would start the setup `command()` makes. Always `Unsupported`: there's no setup to run.
pub fn start_setup(_command: impl Fn() -> Command) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        Unsupported(Feature::InPlaceUpdate),
    ))
}
