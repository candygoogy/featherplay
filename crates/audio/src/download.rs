//! An HTTP stream downloaded into a temp file in 1 MiB `Range` requests, readable while it
//! downloads. A single plain GET is throttled to ~2x realtime; Range chunks run at
//! hundreds of times realtime.
//!
//! One task fetches the file front to back. A read blocks until its first byte is there
//! and returns only bytes that are, so it never sees a hole. When a read waits on a byte
//! the task won't reach soon (behind it, or more than `JUMP_AHEAD` in front of it), the
//! task drops its request and goes on from that byte: a seek into the undownloaded part
//! costs one request. Once the end is reached, the holes left behind are filled.

use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use bytes::Bytes;
use featherplay_core::sync::{CondvarExt, LockExt};
use reqwest::StatusCode;
use tempfile::NamedTempFile;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::http::{CHUNK, ErrorSlot, RETRY_DELAYS, get_range};

/// A wanted byte further than this in front of the download is fetched by a new request
/// rather than waited for. A new request costs a connection (~0.3 s); the download covers
/// about 2 MB in that time.
const JUMP_AHEAD: u64 = 2 << 20;

/// How the download begins.
pub(crate) enum First {
    /// An already-sent response for the first chunk (it told the length).
    Response(reqwest::Response),
    /// The start of the file, from memory (`head.rs`).
    Head(Bytes),
    /// The start of the file so far, and the response it came on, which goes on from there
    /// (a head's request, taken over).
    Partial(Bytes, reqwest::Response),
}

#[derive(Default)]
struct State {
    /// Downloaded ranges, sorted, disjoint and not touching.
    have: Vec<Range<u64>>,
    /// The first byte a blocked read waits for.
    wanted: Option<u64>,
    failed: bool,
    cancelled: bool,
}

impl State {
    /// The end of the downloaded run that holds `pos`.
    fn run_end(&self, pos: u64) -> Option<u64> {
        let i = self.have.partition_point(|r| r.end <= pos);
        self.have.get(i).filter(|r| r.start <= pos).map(|r| r.end)
    }

    fn add(&mut self, r: Range<u64>) {
        if r.is_empty() {
            return;
        }
        // Merge with every range it overlaps or touches.
        let lo = self.have.partition_point(|h| h.end < r.start);
        let hi = self.have.partition_point(|h| h.start <= r.end);
        let start = self.have.get(lo).map_or(r.start, |h| h.start.min(r.start));
        let end = if hi > lo {
            self.have[hi - 1].end.max(r.end)
        } else {
            r.end
        };
        self.have.splice(lo..hi, std::iter::once(start..end));
    }

    /// Where the download goes on from `cursor`: the next missing byte at or after it,
    /// else the first missing byte of the file. `None` when it's all there.
    fn next_missing(&self, cursor: u64, len: u64) -> Option<u64> {
        let after = self.run_end(cursor).unwrap_or(cursor);
        if after < len {
            return Some(after);
        }
        let first = self.run_end(0).unwrap_or(0);
        (first < len).then_some(first)
    }

    /// The end of the hole that starts at `pos`.
    fn hole_end(&self, pos: u64, len: u64) -> u64 {
        let i = self.have.partition_point(|r| r.start <= pos);
        self.have.get(i).map_or(len, |r| r.start)
    }
}

struct Shared {
    /// Locked with `lock_or_recover`: every update is a flag, a `wanted` byte or one
    /// `State::add`, whose single `splice` leaves `have` sorted, so a poisoned lock still
    /// holds a state the reads and the task can trust.
    state: Mutex<State>,
    /// Woken when bytes arrive, the download fails or it's cancelled.
    arrived: Condvar,
    /// Wakes the task when a read starts waiting.
    wake: tokio::sync::Notify,
    len: u64,
    jump_ahead: AtomicU64,
}

impl Shared {
    /// Whether the task should leave its position `cursor` for the wanted byte.
    fn should_jump(&self, st: &State, cursor: u64) -> Option<u64> {
        let w = st.wanted?;
        let ahead = self.jump_ahead.load(Ordering::Relaxed);
        (st.run_end(w).is_none() && (w < cursor || w > cursor + ahead)).then_some(w)
    }
}

/// A stream being downloaded; see the module docs. Cancelled on drop.
pub(crate) struct Download {
    shared: Arc<Shared>,
    file: Arc<NamedTempFile>,
    task: JoinHandle<()>,
}

/// Stops a download from any thread, waking its blocked reads.
#[derive(Clone)]
pub(crate) struct Canceller {
    shared: Arc<Shared>,
    task: tokio::task::AbortHandle,
}

impl Canceller {
    pub fn cancel(&self) {
        self.shared.state.lock_or_recover().cancelled = true;
        self.shared.arrived.notify_all();
        self.task.abort();
    }
}

impl Download {
    /// Starts downloading `url` (`len` bytes) into a temp file in `tmp_dir`, beginning with
    /// `first`. Failures go to `error` and fail the reads.
    pub fn start(
        rt: &Handle,
        client: reqwest::Client,
        url: String,
        len: u64,
        first: First,
        tmp_dir: &Path,
        error: ErrorSlot,
    ) -> io::Result<Download> {
        let file = Arc::new(
            tempfile::Builder::new()
                .prefix(crate::tmp::TEMP_PREFIX)
                .tempfile_in(tmp_dir)?,
        );
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            arrived: Condvar::new(),
            wake: tokio::sync::Notify::new(),
            len,
            jump_ahead: AtomicU64::new(JUMP_AHEAD),
        });
        let task = rt.spawn(run(shared.clone(), file.clone(), client, url, first, error));
        Ok(Download { shared, file, task })
    }

    pub fn canceller(&self) -> Canceller {
        Canceller {
            shared: self.shared.clone(),
            task: self.task.abort_handle(),
        }
    }

    #[cfg(test)]
    pub fn set_jump_ahead(&self, n: u64) {
        self.shared.jump_ahead.store(n, Ordering::Relaxed);
    }

    /// Reads at `pos`, blocking until the byte there has arrived. Returns only downloaded
    /// bytes, so possibly fewer than asked for; `Ok(0)` at the end.
    pub fn read_at(&self, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
        let s = &*self.shared;
        if pos >= s.len || buf.is_empty() {
            return Ok(0);
        }
        let mut st = s.state.lock_or_recover();
        let end = loop {
            if st.cancelled {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "download cancelled",
                ));
            }
            if let Some(end) = st.run_end(pos) {
                if st.wanted == Some(pos) {
                    st.wanted = None;
                }
                break end;
            }
            if st.failed {
                return Err(io::Error::other("download failed"));
            }
            if st.wanted != Some(pos) {
                st.wanted = Some(pos);
                s.wake.notify_one();
            }
            st = s.arrived.wait_or_recover(st);
        };
        drop(st);
        let n = buf.len().min((end - pos) as usize);
        read_exact_at(self.file.as_file(), &mut buf[..n], pos)?;
        Ok(n)
    }
}

impl Download {
    /// Another reader over the same bytes (`DownloadView`).
    pub fn view(&self) -> DownloadView {
        DownloadView {
            shared: self.shared.clone(),
            file: self.file.clone(),
        }
    }
}

/// Another reader over a download's bytes, for a second decoder (`tail`). It doesn't keep
/// the download going, and a read where bytes are missing fails instead of waiting.
#[derive(Clone)]
pub(crate) struct DownloadView {
    shared: Arc<Shared>,
    file: Arc<NamedTempFile>,
}

impl DownloadView {
    /// Every byte has arrived.
    pub fn is_complete(&self) -> bool {
        let s = &*self.shared;
        s.state
            .lock_or_recover()
            .run_end(0)
            .is_some_and(|end| end >= s.len)
    }

    pub fn read_at(&self, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
        let s = &*self.shared;
        if pos >= s.len || buf.is_empty() {
            return Ok(0);
        }
        let end = s
            .state
            .lock_or_recover()
            .run_end(pos)
            .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "not downloaded yet"))?;
        let n = buf.len().min((end - pos) as usize);
        read_exact_at(self.file.as_file(), &mut buf[..n], pos)?;
        Ok(n)
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.canceller().cancel();
    }
}

async fn run(
    shared: Arc<Shared>,
    file: Arc<NamedTempFile>,
    client: reqwest::Client,
    url: String,
    first: First,
    error: ErrorSlot,
) {
    let fail = |e: String| {
        error.lock_or_recover().get_or_insert(e);
        shared.state.lock_or_recover().failed = true;
        shared.arrived.notify_all();
    };
    let len = shared.len;
    let mut cursor = 0u64;
    let (start, mut response) = match first {
        First::Response(r) => (None, Some(r)),
        First::Head(b) => (Some(b), None),
        First::Partial(b, r) => (Some(b), Some(r)),
    };
    if let Some(b) = start {
        let n = (b.len() as u64).min(len);
        if let Err(e) = write_all_at(file.as_file(), &b[..n as usize], 0) {
            return fail(format!("writing the temp file: {e}"));
        }
        shared.state.lock_or_recover().add(0..n);
        shared.arrived.notify_all();
        cursor = n;
    }
    let mut failures = 0;
    loop {
        // Where to go on from: the wanted byte if it's out of reach, else the next hole.
        let (start, end) = {
            let st = shared.state.lock_or_recover();
            if st.cancelled {
                return;
            }
            if let Some(w) = shared.should_jump(&st, cursor) {
                cursor = w;
                response = None;
            }
            match st.next_missing(cursor, len) {
                Some(s) => {
                    if s != cursor {
                        response = None;
                    }
                    cursor = s;
                    (s, (s + CHUNK).min(st.hole_end(s, len)))
                }
                None => return, // all downloaded
            }
        };
        let mut resp = match response.take() {
            Some(r) => r,
            None => match get_range(&client, &url, start, end).await {
                Ok(r) => r,
                Err(e) => return fail(e),
            },
        };
        if resp.status() == StatusCode::OK && start > 0 {
            return fail("server ignored the Range header".into());
        }
        loop {
            let piece = tokio::select! {
                p = resp.chunk() => p,
                // A read started waiting: check whether to jump.
                () = shared.wake.notified() => {
                    let st = shared.state.lock_or_recover();
                    if shared.should_jump(&st, cursor).is_some() {
                        break;
                    }
                    continue;
                }
            };
            match piece {
                Ok(Some(b)) => {
                    // Never past the end. A 200 for the first chunk is the whole file: it
                    // runs on past `end`, and is written as it comes.
                    let room = len.saturating_sub(cursor);
                    let b = &b[..(b.len() as u64).min(room) as usize];
                    if let Err(e) = write_all_at(file.as_file(), b, cursor) {
                        return fail(format!("writing the temp file: {e}"));
                    }
                    let next = cursor + b.len() as u64;
                    shared.state.lock_or_recover().add(cursor..next);
                    shared.arrived.notify_all();
                    cursor = next;
                    failures = 0;
                    if cursor >= len {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    // The body broke off: go on from the current position.
                    let Some(delay) = RETRY_DELAYS.get(failures) else {
                        return fail(format!("stream read failed at byte {cursor}: {e}"));
                    };
                    tokio::time::sleep(*delay).await;
                    failures += 1;
                    break;
                }
            }
        }
    }
}

#[cfg(windows)]
fn write_all_at(f: &File, mut buf: &[u8], mut pos: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = f.seek_write(buf, pos)?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        pos += n as u64;
    }
    Ok(())
}

#[cfg(windows)]
fn read_exact_at(f: &File, mut buf: &mut [u8], mut pos: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = f.seek_read(buf, pos)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buf = &mut buf[n..];
        pos += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
fn write_all_at(f: &File, buf: &[u8], pos: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, buf, pos)
}

#[cfg(unix)]
fn read_exact_at(f: &File, buf: &mut [u8], pos: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(f, buf, pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(have: &[Range<u64>]) -> State {
        State {
            have: have.to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn ranges_merge() {
        let mut s = State::default();
        s.add(10..20);
        s.add(30..40);
        s.add(20..25);
        assert_eq!(s.have, vec![10..25, 30..40]);
        s.add(24..31);
        assert_eq!(s.have, vec![10..40]);
        s.add(0..5);
        s.add(50..60);
        assert_eq!(s.have, vec![0..5, 10..40, 50..60]);
        s.add(0..100);
        assert_eq!(s.have, vec![0..100]);
    }

    #[test]
    fn runs_and_holes() {
        let s = state(&[0..10, 20..30]);
        assert_eq!(s.run_end(0), Some(10));
        assert_eq!(s.run_end(9), Some(10));
        assert_eq!(s.run_end(10), None);
        assert_eq!(s.run_end(25), Some(30));
        assert_eq!(s.next_missing(0, 50), Some(10));
        assert_eq!(s.next_missing(22, 50), Some(30));
        assert_eq!(s.next_missing(35, 50), Some(35));
        assert_eq!(s.hole_end(10, 50), 20);
        assert_eq!(s.hole_end(30, 50), 50);
        let s = state(&[0..10, 20..50]);
        assert_eq!(s.next_missing(25, 50), Some(10), "wraps to the hole behind");
        let mut all = State::default();
        all.add(0..50);
        assert_eq!(all.next_missing(10, 50), None);
    }
}
