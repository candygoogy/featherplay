//! Child processes, ended with the app. Stubs until phase 5: a child isn't tied to the app
//! yet, and runs out its own time limit if the app ends first.

use std::io;
use std::process::Command;

/// Starts `cmd`'s process without a console window. Linux opens none, so nothing to do.
pub fn no_console_window(_cmd: &mut Command) {}

/// What names a started child to [`kill_on_close`]: its process id, as
/// `tokio::process::Child::id` gives it.
pub type ChildHandle = u32;

/// What would end the child with the app. Holds nothing yet.
pub struct Job(());

/// Would tie `process` to the app. Ties nothing yet: the child goes on as it was.
pub fn kill_on_close(_process: ChildHandle) -> io::Result<Job> {
    Ok(Job(()))
}
