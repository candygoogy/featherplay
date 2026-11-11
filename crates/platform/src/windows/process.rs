//! Child processes: no console window, and the job that ends them with the app.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows::core::PCWSTR;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Starts `cmd`'s process without a console window: a console program started from a GUI
/// app would open one.
pub fn no_console_window(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// What names a started child to [`kill_on_close`]: its process handle, as
/// `tokio::process::Child::raw_handle` gives it.
pub type ChildHandle = RawHandle;

/// A job that ends the process in it when the job's last handle closes.
pub struct Job {
    handle: OwnedHandle,
}

impl Job {
    /// Ends every process in the job now, as closing it would.
    pub(crate) fn end(&self) {
        // SAFETY: `handle` is a live job handle we own.
        unsafe {
            let _ = TerminateJobObject(HANDLE(self.handle.as_raw_handle()), 1);
        }
    }
}

/// A job that ends `process` when its last handle closes: when it's dropped, or when this
/// process ends, however it ends. `process` must be a live child's handle.
pub fn kill_on_close(process: ChildHandle) -> io::Result<Job> {
    // SAFETY: no security attributes (not inheritable) and no name.
    let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }?;
    // SAFETY: CreateJobObjectW succeeded; the handle is ours alone.
    let job = unsafe { OwnedHandle::from_raw_handle(job.0) };
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `info` is the structure JobObjectExtendedLimitInformation expects, with its
    // size; `job` is ours, and `process` is the live child's handle, owned by its `Child`.
    unsafe {
        SetInformationJobObject(
            HANDLE(job.as_raw_handle()),
            JobObjectExtendedLimitInformation,
            (&raw const info).cast(),
            size_of_val(&info) as u32,
        )?;
        AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(process))?;
    }
    Ok(Job { handle: job })
}

#[cfg(test)]
mod tests {
    use std::os::windows::io::AsRawHandle;
    use std::time::Duration;

    /// The preprocessing child's job ends it when the job's handle closes, as it does when
    /// the app exits or crashes.
    #[test]
    fn the_preprocessor_ends_when_its_job_closes() {
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let job = super::kill_on_close(child.as_raw_handle()).unwrap();
        assert!(child.try_wait().unwrap().is_none(), "running in its job");
        drop(job);
        for _ in 0..250 {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        panic!("the child outlived its job");
    }
}
