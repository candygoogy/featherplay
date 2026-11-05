//! Starts the browser with a DevTools pipe and nothing else, inside a job object.
//!
//! # How Chromium on Windows gets the pipe
//! `--remote-debugging-pipe` makes Chromium talk CDP over file descriptors 3 (it reads) and
//! 4 (it writes): `DevToolsAgentHost::StartRemoteDebuggingPipeHandler`
//! (content/browser/devtools/devtools_agent_host_impl.cc) passes `kReadFD = 3` and
//! `kWriteFD = 4` to `DevToolsPipeHandler`, which on Windows turns them into handles with
//! `_get_osfhandle` and does blocking `ReadFile`/`WriteFile` on them
//! (content/browser/devtools/devtools_pipe_handler.cc). Checked against Chromium's main
//! branch, 2026-11-05.
//!
//! A Windows process gets C runtime descriptors beyond 0-2 from its parent through
//! `STARTUPINFOW.cbReserved2`/`lpReserved2`: an `int` count, then one flag byte per
//! descriptor, then one `HANDLE` per descriptor, packed. The UCRT reads that block at
//! startup (`__acrt_initialize_lowio`) and opens each descriptor whose flags have `FOPEN`.
//! This is how Node (libuv's `uv__stdio_create`, src/win/process-stdio.c) hands a child
//! extra descriptors, and so how Playwright and Puppeteer reach Chromium's pipe on Windows
//! with `stdio: [..., 'pipe', 'pipe']`. `crt_block` builds it: 0-2 not open, 3 and 4
//! `FOPEN | FPIPE` with the browser's two pipe ends.
//!
//! (Chromium also has `--remote-debugging-io-pipes=<read>,<write>`, handle values on the
//! command line. The descriptor block works on every Chromium that has the pipe, so this
//! uses that.)
//!
//! # What the browser inherits
//! Only its two pipe ends. Both pipes are made non-inheritable (`CreatePipe` without
//! security attributes); the browser's two ends are made inheritable just for
//! `CreateProcessW`, which gets them as its `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, so no other
//! inheritable handle of the app reaches it; and the app closes its copies of those ends
//! right after. Std's `Command` (yt-dlp, Deno) inherits every inheritable handle, so a
//! process it starts in that instant could hold a copy too: then the pipe stays open after
//! the browser exits, which is why the app watches the browser's process and never takes
//! the pipe closing as the only sign that it's gone. Its standard handles are none
//! (`STARTF_USESTDHANDLES` with null handles).
//!
//! # The job object
//! The browser starts suspended, joins a job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
//! and only then runs, so it and every helper process it starts are in the job. When the
//! app lets go of the job (`Edge` dropped), or the app crashes or is killed and Windows
//! closes the handle, everything in the job ends.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::time::Duration;

use windows::Win32::Foundation::{
    HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS, SetHandleInformation, WAIT_OBJECT_0,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

/// The C runtime's descriptor flags (UCRT `lowio`): open, and a pipe.
const FOPEN: u8 = 0x01;
const FPIPE: u8 = 0x08;
/// The descriptors Chromium's pipe handler uses.
const READ_FD: usize = 3;
const WRITE_FD: usize = 4;
/// `INVALID_HANDLE_VALUE`, for the descriptors that aren't open.
const NO_HANDLE: usize = usize::MAX;

/// What `launch` passes to `CreateProcessW`, worked out before any handle is touched.
#[derive(Debug)]
pub struct Plan {
    /// The whole command line, NUL-terminated.
    pub command_line: Vec<u16>,
    /// The C runtime's descriptor block (`lpReserved2`).
    pub crt: Vec<u8>,
    /// The handles the browser inherits: its read end, then its write end.
    pub inherit: [usize; 2],
}

/// The launch of `exe` with `args`, where `read` and `write` are the browser's ends of the
/// two pipes.
pub fn plan(exe: &OsStr, args: &[OsString], read: usize, write: usize) -> Plan {
    let mut command_line = quote(exe);
    for a in args {
        command_line.push(u16::from(b' '));
        command_line.extend(quote(a));
    }
    command_line.push(0);
    Plan {
        command_line,
        crt: crt_block(read, write),
        inherit: [read, write],
    }
}

/// The UCRT's inherited-descriptor block: 0-2 not open, 3 = `read`, 4 = `write`.
pub fn crt_block(read: usize, write: usize) -> Vec<u8> {
    const COUNT: usize = WRITE_FD + 1;
    let mut flags = [0u8; COUNT];
    let mut handles = [NO_HANDLE; COUNT];
    flags[READ_FD] = FOPEN | FPIPE;
    flags[WRITE_FD] = FOPEN | FPIPE;
    handles[READ_FD] = read;
    handles[WRITE_FD] = write;
    let mut block = Vec::with_capacity(size_of::<i32>() + COUNT * (1 + size_of::<usize>()));
    block.extend_from_slice(&(COUNT as i32).to_ne_bytes());
    block.extend_from_slice(&flags);
    for h in handles {
        block.extend_from_slice(&h.to_ne_bytes());
    }
    block
}

/// One argument quoted the way `CommandLineToArgvW` (which Chromium parses with) reads it
/// back unchanged.
fn quote(arg: &OsStr) -> Vec<u16> {
    let wide: Vec<u16> = arg.encode_wide().collect();
    let plain = !wide.is_empty()
        && !wide
            .iter()
            .any(|&c| c == u16::from(b' ') || c == u16::from(b'\t') || c == u16::from(b'"'));
    if plain {
        return wide;
    }
    let (bs, dq) = (u16::from(b'\\'), u16::from(b'"'));
    let mut out = vec![dq];
    let mut slashes = 0;
    for c in wide {
        if c == bs {
            slashes += 1;
            continue;
        }
        // Backslashes before a quote are doubled, and the quote escaped.
        let n = if c == dq { slashes * 2 + 1 } else { slashes };
        out.extend(std::iter::repeat_n(bs, n));
        out.push(c);
        slashes = 0;
    }
    // Before the closing quote, too.
    out.extend(std::iter::repeat_n(bs, slashes * 2));
    out.push(dq);
    out
}

/// The running browser. Dropping it closes the job, which ends whatever is still in it.
pub struct Edge {
    process: OwnedHandle,
    job: OwnedHandle,
    pub pid: u32,
}

impl Edge {
    /// The browser's own process has exited (its helpers may linger a moment).
    pub fn exited(&self) -> bool {
        self.wait(Duration::ZERO)
    }

    /// Waits up to `limit` for the browser's process to exit; says whether it did.
    pub fn wait(&self, limit: Duration) -> bool {
        let ms = u32::try_from(limit.as_millis()).unwrap_or(u32::MAX - 1);
        // SAFETY: `process` is a live process handle we own; waiting reads nothing else.
        unsafe { WaitForSingleObject(handle(&self.process), ms) == WAIT_OBJECT_0 }
    }

    /// Ends the browser and every process in its job.
    pub fn kill(&self) {
        // SAFETY: `job` is a live job handle we own.
        unsafe {
            let _ = TerminateJobObject(handle(&self.job), 1);
        }
    }
}

/// The app's ends of the two pipes.
pub struct Pipes {
    /// Commands go in here; the browser reads them on its descriptor 3.
    pub to_browser: File,
    /// The browser writes on its descriptor 4; replies come out here.
    pub from_browser: File,
}

/// Starts `exe` with `args` and the pipe, suspended, in a kill-on-close job, then lets it
/// run.
pub fn launch(exe: &Path, args: &[OsString]) -> io::Result<(Edge, Pipes)> {
    let (browser_read, to_browser) = pipe()?;
    let (from_browser, browser_write) = pipe()?;
    let plan = plan(
        exe.as_os_str(),
        args,
        browser_read.as_raw_handle() as usize,
        browser_write.as_raw_handle() as usize,
    );
    let started = spawn_suspended(exe, &plan, [&browser_read, &browser_write]);
    // The browser has its own copies now; ours would keep the pipe open after it exits.
    drop(browser_read);
    drop(browser_write);
    let (process, thread, pid) = started?;

    let job = match kill_on_close_job(&process) {
        Ok(job) => job,
        Err(e) => {
            // SAFETY: our live, suspended child; it never ran.
            unsafe {
                let _ = TerminateProcess(handle(&process), 1);
            }
            return Err(e);
        }
    };
    // SAFETY: the live, suspended main thread of our child.
    if unsafe { ResumeThread(handle(&thread)) } == u32::MAX {
        let e = io::Error::last_os_error();
        // SAFETY: our live job handle; this ends the child that never ran.
        unsafe {
            let _ = TerminateJobObject(handle(&job), 1);
        }
        return Err(e);
    }
    Ok((
        Edge { process, job, pid },
        Pipes {
            to_browser: File::from(to_browser),
            from_browser: File::from(from_browser),
        },
    ))
}

fn handle(h: &OwnedHandle) -> HANDLE {
    HANDLE(h.as_raw_handle())
}

/// An anonymous pipe, both ends non-inheritable: (read end, write end).
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
    // SAFETY: two live out pointers; no security attributes, so neither end is inheritable.
    unsafe { CreatePipe(&mut read, &mut write, None, 0) }?;
    // SAFETY: CreatePipe succeeded, so both are fresh handles that nothing else owns.
    unsafe {
        Ok((
            OwnedHandle::from_raw_handle(read.0),
            OwnedHandle::from_raw_handle(write.0),
        ))
    }
}

/// `CreateProcessW`, suspended, inheriting exactly `inherit` (the handles `plan` names).
fn spawn_suspended(
    exe: &Path,
    plan: &Plan,
    inherit: [&OwnedHandle; 2],
) -> io::Result<(OwnedHandle, OwnedHandle, u32)> {
    for h in inherit {
        // SAFETY: a live handle we own; only its inherit flag changes.
        unsafe { SetHandleInformation(handle(h), HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }?;
    }
    let list = HandleList::new(plan.inherit.map(|h| HANDLE(h as *mut _)))?;

    let mut si = STARTUPINFOEXW::default();
    si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.cbReserved2 = u16::try_from(plan.crt.len()).map_err(io::Error::other)?;
    si.StartupInfo.lpReserved2 = plan.crt.as_ptr().cast_mut();
    si.lpAttributeList = list.ptr();
    let mut command_line = plan.command_line.clone();
    let mut pi = PROCESS_INFORMATION::default();
    let app = HSTRING::from(exe.as_os_str());
    // SAFETY: `app` and `command_line` are NUL-terminated and live across the call (the
    // command line is a mutable copy, as CreateProcessW may write to it); `si` points at
    // `plan.crt` and the attribute list, both alive until this returns; `pi` is a live out
    // pointer. Inheritance is on, limited by the handle list to the two pipe ends.
    unsafe {
        CreateProcessW(
            &app,
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
            None,
            PCWSTR::null(),
            &si.StartupInfo,
            &mut pi,
        )
    }?;
    for h in inherit {
        // SAFETY: as above. Closed right after anyway; this only narrows the window.
        unsafe {
            let _ = SetHandleInformation(handle(h), HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0));
        }
    }
    // SAFETY: CreateProcessW succeeded, so both are fresh handles that only we own.
    unsafe {
        Ok((
            OwnedHandle::from_raw_handle(pi.hProcess.0),
            OwnedHandle::from_raw_handle(pi.hThread.0),
            pi.dwProcessId,
        ))
    }
}

/// A `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` attribute list naming exactly two handles. One
/// exists only once its list is initialised, so `Drop` always has a list to delete.
struct HandleList {
    // usize-aligned storage for the opaque list; boxed, so `list` stays valid.
    _storage: Box<[usize]>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
    // The list points into this; boxed, so it stays put while the list lives.
    handles: Box<[HANDLE; 2]>,
}

impl HandleList {
    fn new(handles: [HANDLE; 2]) -> io::Result<Self> {
        let mut size = 0usize;
        // SAFETY: the documented size query: no list, a live out pointer. It "fails" with
        // ERROR_INSUFFICIENT_BUFFER and fills `size`.
        unsafe {
            let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
        }
        let mut storage = vec![0usize; size.div_ceil(size_of::<usize>()).max(1)].into_boxed_slice();
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        // SAFETY: `storage` holds at least `size` bytes, usize-aligned, and is written only
        // through `list`.
        unsafe { InitializeProcThreadAttributeList(Some(list), 1, None, &mut size) }?;
        let me = Self {
            _storage: storage,
            list,
            handles: Box::new(handles),
        };
        // SAFETY: an initialised list with room for one attribute; `handles` is boxed, so
        // the pointer stays valid for as long as `me` (which deletes the list on drop).
        unsafe {
            UpdateProcThreadAttribute(
                me.list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(me.handles.as_ptr().cast()),
                size_of::<[HANDLE; 2]>(),
                None,
                None,
            )
        }?;
        Ok(me)
    }

    fn ptr(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.list
    }
}

impl Drop for HandleList {
    fn drop(&mut self) {
        // SAFETY: the list was initialised in `new` before `self` existed, and its storage
        // is still owned by `self`.
        unsafe { DeleteProcThreadAttributeList(self.list) };
    }
}

/// A job that ends everything in it when its last handle closes, with `process` in it.
fn kill_on_close_job(process: &OwnedHandle) -> io::Result<OwnedHandle> {
    // SAFETY: no security attributes (not inheritable) and no name.
    let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }?;
    // SAFETY: CreateJobObjectW succeeded; the handle is ours alone.
    let job = unsafe { OwnedHandle::from_raw_handle(job.0) };
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `info` is the structure JobObjectExtendedLimitInformation expects, with its
    // size; `job` and `process` are live handles we own.
    unsafe {
        SetInformationJobObject(
            handle(&job),
            JobObjectExtendedLimitInformation,
            (&raw const info).cast(),
            size_of_val(&info) as u32,
        )?;
        AssignProcessToJobObject(handle(&job), handle(process))?;
    }
    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn the_crt_block_opens_only_descriptors_3_and_4() {
        let b = crt_block(0x1234, 0x5678);
        let n = size_of::<usize>();
        assert_eq!(b.len(), 4 + 5 + 5 * n);
        assert_eq!(i32::from_ne_bytes(b[..4].try_into().unwrap()), 5);
        assert_eq!(&b[4..9], &[0, 0, 0, FOPEN | FPIPE, FOPEN | FPIPE]);
        let handles: Vec<usize> = b[9..]
            .chunks(n)
            .map(|c| usize::from_ne_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(
            handles,
            [usize::MAX, usize::MAX, usize::MAX, 0x1234, 0x5678]
        );
    }

    #[test]
    fn the_browser_inherits_only_the_two_pipe_ends() {
        let p = plan(OsStr::new(r"C:\Edge\msedge.exe"), &[], 0x1234, 0x5678);
        assert_eq!(p.inherit, [0x1234, 0x5678]);
        // The descriptor block hands out those same two, as 3 and 4.
        assert_eq!(p.crt, crt_block(0x1234, 0x5678));
    }

    #[test]
    fn the_command_line_quotes_what_needs_it() {
        let p = plan(
            OsStr::new(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
            &[
                r"--user-data-dir=C:\Users\Ada Lovelace\cache\signin-1".into(),
                "--remote-debugging-pipe".into(),
                "https://example.com/?a=1&b=2".into(),
            ],
            1,
            2,
        );
        assert_eq!(
            String::from_utf16(&p.command_line).unwrap(),
            "\"C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe\" \
             \"--user-data-dir=C:\\Users\\Ada Lovelace\\cache\\signin-1\" \
             --remote-debugging-pipe https://example.com/?a=1&b=2\0"
        );
    }

    #[test]
    fn quoting_keeps_backslashes_and_quotes_intact() {
        assert_eq!(quote(OsStr::new(r"plain\path")), wide(r"plain\path"));
        assert_eq!(quote(OsStr::new("")), wide(r#""""#));
        assert_eq!(quote(OsStr::new(r"C:\a b\")), wide(r#""C:\a b\\""#));
        assert_eq!(quote(OsStr::new(r#"say "hi""#)), wide(r#""say \"hi\"""#));
        assert_eq!(quote(OsStr::new(r#"a\"b"#)), wide(r#""a\\\"b""#));
    }

    #[test]
    fn the_pipes_are_not_inheritable() {
        use windows::Win32::Foundation::GetHandleInformation;
        let (r, w) = pipe().unwrap();
        for h in [&r, &w] {
            let mut flags = 0u32;
            // SAFETY: a live handle and a live out pointer.
            unsafe { GetHandleInformation(handle(h), &mut flags) }.unwrap();
            assert_eq!(flags & HANDLE_FLAG_INHERIT.0, 0);
        }
    }

    /// A harmless long-running Windows program: ping the loopback address.
    fn ping() -> (std::path::PathBuf, Vec<OsString>) {
        let windir = std::env::var_os("SystemRoot").unwrap();
        let exe = Path::new(&windir).join(r"System32\PING.EXE");
        (exe, vec!["-n".into(), "60".into(), "127.0.0.1".into()])
    }

    #[test]
    fn a_launched_process_runs_and_its_job_ends_it() {
        let (exe, args) = ping();
        let (child, _pipes) = launch(&exe, &args).unwrap();
        assert!(!child.exited(), "it should be running");
        child.kill();
        assert!(child.wait(Duration::from_secs(5)), "the job didn't end it");
    }

    #[test]
    fn dropping_the_browser_closes_its_job_and_ends_it() {
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
        let (exe, args) = ping();
        let (child, _pipes) = launch(&exe, &args).unwrap();
        // SAFETY: opens a fresh handle to our child, which is alive (asserted below).
        let watch = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, child.pid) }.unwrap();
        // SAFETY: OpenProcess succeeded; the handle is ours alone.
        let watch = unsafe { OwnedHandle::from_raw_handle(watch.0) };
        assert!(!child.exited());
        drop(child);
        // SAFETY: a live process handle we own.
        let ended = unsafe { WaitForSingleObject(handle(&watch), 5000) } == WAIT_OBJECT_0;
        assert!(ended, "closing the job didn't end the process");
    }

    #[test]
    fn a_missing_exe_is_an_error() {
        let gone = Path::new(r"C:\definitely\not\here\msedge.exe");
        assert!(launch(gone, &[]).is_err());
    }
}
