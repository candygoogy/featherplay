//! A scripted run of the real window, for the performance harness: built only with the
//! `perf-harness` feature, and active only with `FEATHERPLAY_PERF_SCRIPT` naming a script.
//! A build without the feature has none of it; a build with it and no script pays one cached
//! flag read per paint.
//!
//! The script drives the window through the same callbacks its clicks, keys and wheel reach
//! (`Nav`, the pages' globals, `Player`, the queue), one step at a time, and times each step
//! from the app's own signals: the window's paints (the paint subclass in
//! `crates/platform/src/windows/window.rs`, through `frame_log`), page loads applied (`app`),
//! covers decoded (`covers`), suggestions shown (`suggest`), the background work still
//! pending (`busy`) and the player's state (`app::on_event`). Each step writes one JSON line
//! to `FEATHERPLAY_PERF_OUT` (or `{cache}/logs/perf.jsonl`). Nothing moves the real pointer
//! or takes the keyboard.
//!
//! `FEATHERPLAY_PERF_FIXTURES=1` shows the recorded fixtures (`fixtures`) instead of the
//! account, so page tours repeat with no network: no sign-in is read, nothing is asked of
//! Google. `FEATHERPLAY_PERF_ROWS=<n>` lengthens the fixture playlists to `n` rows.
//!
//! The runner script and the script language are documented with the measurements, which
//! are kept outside the repo.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::RefCell;
use std::future::Future;
use std::io::Write;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use featherplay_platform::perf::{cpu_ms, since_creation, thread_cycles, thread_name};
use featherplay_platform::window::Handle;
use i_slint_core::accessibility::{AccessibilityAction, AccessibleStringProperty};
use i_slint_core::item_tree::ItemRc;
use i_slint_core::window::WindowInner;
use serde_json::{Map, Value, json};
use slint::platform::WindowEvent;
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, SharedString};

use crate::app::Ctx;
use crate::controller::PlayerState;
use crate::ui::{
    AlbumData, ArtistData, Frame, HistoryData, HomeData, ItemKind, LibraryData, Nav,
    NowPlayingData, PageHover, PageKind, Player, PlaylistData, QueueData, RowHover, SearchData,
    TopBarData, TrackRow,
};

/// A paint gap longer than this starts a new run of frames: the window was idle between.
const RUN_GAP: Duration = Duration::from_millis(100);
/// No paint for this long after a step's work is in: its animations are over.
const QUIET: Duration = Duration::from_millis(250);
const STEP_TIMEOUT: Duration = Duration::from_secs(20);
const PLAY_TIMEOUT: Duration = Duration::from_secs(40);
/// How often a wait on time alone looks again.
const TICK: Duration = Duration::from_millis(20);

/// Whether a script runs: `FEATHERPLAY_PERF_SCRIPT` is set.
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FEATHERPLAY_PERF_SCRIPT").is_some_and(|v| !v.is_empty()))
}

/// Whether the window shows the fixtures instead of the account.
pub fn fixtures() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| on() && std::env::var_os("FEATHERPLAY_PERF_FIXTURES").is_some())
}

/// With the fixtures, every song plays the local file `FEATHERPLAY_PERF_AUDIO` names (an
/// `.m4a` or `.webm`), so the window can be measured while sound plays and after, with no
/// stream asked of YouTube. Unset, the fixtures play nothing.
pub fn fixture_stream() -> Option<featherplay_streams::ResolvedStream> {
    if !fixtures() {
        return None;
    }
    let path = std::path::absolute(std::env::var_os("FEATHERPLAY_PERF_AUDIO")?).ok()?;
    let len = std::fs::metadata(&path).ok()?.len();
    let webm = path.extension().is_some_and(|e| e == "webm");
    Some(featherplay_streams::ResolvedStream {
        url: format!("file:///{}", path.display().to_string().replace('\\', "/")),
        mime: if webm { "audio/webm" } else { "audio/mp4" }.into(),
        itag: if webm { 251 } else { 140 },
        bitrate_kbps: 128,
        content_length: Some(len),
        expires_at: std::time::SystemTime::now() + Duration::from_secs(6 * 3600),
        duration: None,
        available_at: None,
        video_title: None,
        first_tag: None,
        loudness_db: None,
        via: featherplay_streams::Via::Fixture,
    })
}

/// The rows a fixture playlist is lengthened to (`FEATHERPLAY_PERF_ROWS`), if set.
pub fn rows() -> Option<usize> {
    if !fixtures() {
        return None;
    }
    std::env::var("FEATHERPLAY_PERF_ROWS").ok()?.parse().ok()
}

/// `tracks` repeated to `rows()` rows, when that's set.
pub fn lengthen<T: Clone>(tracks: &mut Vec<T>) {
    let Some(n) = rows() else { return };
    if tracks.is_empty() || tracks.len() >= n {
        return;
    }
    let base = tracks.clone();
    while tracks.len() < n {
        tracks.push(base[tracks.len() % base.len()].clone());
    }
}

// ----- What the app reports -------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    /// A page load's result was applied (`Ctx::load`).
    Load,
    /// A cover was decoded and handed to the UI thread.
    Cover,
    /// The covers' pacer handed a card its cover (`covers`).
    CoverShown,
    /// Search suggestions were shown.
    Suggest,
    /// Search suggestions for the field's whole text were shown (not an earlier prefix's).
    SuggestTyped,
    /// The account is known (signed in or out).
    Account,
}

#[derive(Clone, Default)]
struct PlayerSeen {
    id: String,
    playing: bool,
    audible: bool,
}

#[derive(Default)]
struct Seen {
    /// Paints since the step began: start and end.
    paints: Vec<(Instant, Instant)>,
    last_paint: Option<Instant>,
    marks: Vec<(Mark, Instant)>,
    /// When the pending background work last dropped to none.
    first_paint_ever: Option<Instant>,
    account_ever: Option<Instant>,
    restored_ever: Option<Instant>,
    player: PlayerSeen,
    wakers: Vec<Waker>,
}

thread_local! {
    static SEEN: RefCell<Seen> = RefCell::new(Seen::default());
}

/// Cover decodes since the step began, from tokio's blocking pool: count, total and slowest ms,
/// and of the total, the image decode alone (the rest is the crop and the resize).
static DECODES: Mutex<(u32, f64, f64, f64)> = Mutex::new((0, 0.0, 0.0, 0.0));

fn wake() {
    let wakers = SEEN.with_borrow_mut(|s| std::mem::take(&mut s.wakers));
    for w in wakers {
        w.wake();
    }
}

/// A paint of the main window ended (`frame_log::around_paint`, from the paint subclass in
/// `crates/platform/src/windows/window.rs`), on the UI thread.
pub fn painted(started: Instant) {
    let ended = Instant::now();
    SEEN.with_borrow_mut(|s| {
        s.first_paint_ever.get_or_insert(ended);
        s.paints.push((started, ended));
        s.last_paint = Some(ended);
    });
    wake();
}

/// Something the app did, on the UI thread.
pub fn mark(mark: Mark) {
    if !on() {
        return;
    }
    let now = Instant::now();
    SEEN.with_borrow_mut(|s| {
        s.marks.push((mark, now));
        if mark == Mark::Account {
            s.account_ever.get_or_insert(now);
        }
    });
    wake();
}

/// The player's state reached the window.
pub fn player(state: &PlayerState) {
    if !on() {
        return;
    }
    let now = Instant::now();
    SEEN.with_borrow_mut(|s| {
        if state.current.is_some() {
            s.restored_ever.get_or_insert(now);
        }
        s.player = PlayerSeen {
            id: state
                .current
                .as_ref()
                .map(|t| t.id.0.clone())
                .unwrap_or_default(),
            playing: state.playing,
            audible: state.audible,
        };
    });
    wake();
}

/// The pending background work dropped to none (`busy`), on any thread.
pub fn idle() {
    // Kept here at once, not on the UI thread's next turn: a wait that finds nothing pending
    // must also find when that began.
    if let Ok(mut at) = IDLE_AT.lock() {
        *at = Some(Instant::now());
    }
    let _ = slint::invoke_from_event_loop(wake);
}

/// When the pending background work last dropped to none.
static IDLE_AT: Mutex<Option<Instant>> = Mutex::new(None);

fn idle_at() -> Option<Instant> {
    IDLE_AT.lock().ok().and_then(|at| *at)
}

/// A cover decode took `ms`, on tokio's blocking pool.
pub fn decoded(ms: f64) {
    if !on() {
        return;
    }
    if let Ok(mut d) = DECODES.lock() {
        d.0 += 1;
        d.1 += ms;
        d.2 = d.2.max(ms);
    }
}

/// Of a cover's decode, the image decode alone took `ms`.
pub fn decoded_alone(ms: f64) {
    if !on() {
        return;
    }
    if let Ok(mut d) = DECODES.lock() {
        d.3 += ms;
    }
}

// ----- Waiting --------------------------------------------------------------------------

/// Ready with `true` once `cond` holds (checked whenever the app reports something), or with
/// `false` at `deadline`.
struct Until<F> {
    cond: F,
    deadline: Instant,
    timer: Option<slint::Timer>,
    /// Checked again at least this often, for conditions that time alone makes true.
    tick: Option<Duration>,
    /// Checked again whenever the app reports something. Off for a plain wait (`sleep`),
    /// which time alone ends: woken by every paint, the script would cost an idle spell's
    /// frames a posted message and a turn of the event loop each.
    on_reports: bool,
}

impl<F: FnMut() -> bool + Unpin> Future for Until<F> {
    type Output = bool;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<bool> {
        if (self.cond)() {
            return Poll::Ready(true);
        }
        let now = Instant::now();
        if now >= self.deadline {
            return Poll::Ready(false);
        }
        // Once: each wake polls the script again, and each poll lands here. Queued on every
        // poll, a copy was never dropped and every timer wake added one, so over a run every
        // paint came to post hundreds of wakes to the UI thread (half its time in the tour).
        SEEN.with_borrow_mut(|s| {
            if self.on_reports && !s.wakers.iter().any(|w| w.will_wake(cx.waker())) {
                s.wakers.push(cx.waker().clone());
            }
        });
        // Slint's timers count whole milliseconds and may fire a little early: armed again
        // for what's left whenever it isn't running.
        if !self.timer.as_ref().is_some_and(slint::Timer::running) {
            let waker = cx.waker().clone();
            let left = self.deadline - now + Duration::from_millis(1);
            let left = self.tick.map_or(left, |t| t.min(left));
            let timer = self.timer.get_or_insert_with(slint::Timer::default);
            timer.start(slint::TimerMode::SingleShot, left, move || {
                waker.wake_by_ref()
            });
        }
        Poll::Pending
    }
}

fn until(
    timeout: Duration,
    cond: impl FnMut() -> bool + Unpin,
) -> Until<impl FnMut() -> bool + Unpin> {
    Until {
        cond,
        deadline: Instant::now() + timeout,
        timer: None,
        tick: None,
        on_reports: true,
    }
}

/// `until`, for a condition that may come true with nothing reported (a quiet spell): checked
/// every `TICK` too. Never used for `idle`, whose wakeups are measured.
fn ticking(
    timeout: Duration,
    cond: impl FnMut() -> bool + Unpin,
) -> Until<impl FnMut() -> bool + Unpin> {
    Until {
        tick: Some(TICK),
        ..until(timeout, cond)
    }
}

async fn sleep(d: Duration) {
    Until {
        on_reports: false,
        ..until(d, || false)
    }
    .await;
}

/// Lets the event loop run once (input dispatched, a frame asked for).
async fn turn() {
    sleep(Duration::from_millis(1)).await;
}

// ----- The process --------------------------------------------------------------------

// Windows charges a thread's time a clock tick at a time; the spells also report cycles
// (`featherplay_platform::perf` says why).

/// This process's cycles so far, its ended threads' included, or 0.
fn process_cycles() -> u64 {
    featherplay_platform::thread::process_cycles().unwrap_or(0)
}

/// The time-stamp counter, the clock the cycle counts are in on x86-64. Elsewhere (ARM64
/// counts cycles in other units) 0, and the spells report no cycles.
fn tsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: rdtsc has no preconditions on x86-64.
    return unsafe { core::arch::x86_64::_rdtsc() };
    #[cfg(not(target_arch = "x86_64"))]
    return 0;
}

/// One look at the process from the system's process list.
#[derive(Clone, Default)]
struct Sample {
    cpu_ms: f64,
    private_mb: f64,
    working_set_mb: f64,
    /// Each thread's id, its context switches so far (each one is the thread waking) and its
    /// CPU time so far (100 ns units).
    switches: Vec<(u64, u64, u64)>,
    threads: u32,
    /// Each thread's id and cycles so far; the process's cycles; the TSC.
    cycles: Vec<(u64, u64)>,
    proc_cycles: u64,
    tsc: u64,
}

impl Sample {
    /// The context switches from `self` to `later`, of the threads alive at `later` (a thread
    /// that ended in between is lost; one that began counts from zero).
    fn switches_until(&self, later: &Sample) -> u64 {
        later
            .switches
            .iter()
            .map(|(tid, n, _)| {
                let before = self
                    .switches
                    .iter()
                    .find(|(t, _, _)| t == tid)
                    .map_or(0, |(_, n, _)| *n);
                n.saturating_sub(before)
            })
            .sum()
    }

    /// The threads that used the most cycles from `self` to `later`: name (or id), CPU ms
    /// as Windows charges it, CPU ms from the cycles, and wakeups per second over `secs`.
    fn busiest_until(&self, later: &Sample, secs: f64) -> Vec<Value> {
        // No rate without both TSC readings (a failed sample, an ARM64 build): no cycles.
        let tsc_per_ms = if self.tsc > 0 {
            later.tsc.saturating_sub(self.tsc) as f64 / (secs * 1000.0)
        } else {
            0.0
        };
        let cycles = |s: &Sample, tid: u64| {
            s.cycles
                .iter()
                .find(|(t, _)| *t == tid)
                .map_or(0, |(_, c)| *c)
        };
        let mut rows: Vec<(u64, u64, u64, f64)> = later
            .switches
            .iter()
            .map(|&(tid, n, cpu)| {
                let (n0, cpu0) = self
                    .switches
                    .iter()
                    .find(|(t, _, _)| *t == tid)
                    .map_or((0, 0), |(_, n, c)| (*n, *c));
                let ran = cycles(later, tid).saturating_sub(cycles(self, tid)) as f64;
                let cycle_ms = if tsc_per_ms > 0.0 {
                    ran / tsc_per_ms
                } else {
                    0.0
                };
                (
                    tid,
                    cpu.saturating_sub(cpu0),
                    n.saturating_sub(n0),
                    cycle_ms,
                )
            })
            .collect();
        rows.sort_by(|a, b| b.3.total_cmp(&a.3).then(b.1.cmp(&a.1)));
        rows.into_iter()
            .take(5)
            .filter(|(_, cpu, n, cycle_ms)| *cpu > 0 || *n > 0 || *cycle_ms > 0.0)
            .map(|(tid, cpu, n, cycle_ms)| {
                let name = thread_name(tid);
                json!({
                    "thread": if name.is_empty() { tid.to_string() } else { name },
                    "cpu_ms": round(cpu as f64 / 1e4),
                    "cycle_ms": round(cycle_ms),
                    "wakeups_per_s": round(n as f64 / secs),
                })
            })
            .collect()
    }
}

fn sample() -> Sample {
    let Some(s) = featherplay_platform::perf::process_sample() else {
        return Sample::default();
    };
    let switches: Vec<(u64, u64, u64)> = s
        .threads
        .iter()
        .map(|t| (t.id, t.switches, t.cpu))
        .collect();
    let cycles = switches
        .iter()
        .map(|&(tid, _, _)| (tid, thread_cycles(tid)))
        .collect();
    Sample {
        cpu_ms: s.cpu as f64 / 1e4,
        private_mb: s.private_bytes as f64 / 1e6,
        working_set_mb: s.working_set_bytes as f64 / 1e6,
        threads: s.threads.len() as u32,
        switches,
        cycles,
        proc_cycles: process_cycles(),
        tsc: tsc(),
    }
}

fn logical_cpus() -> f64 {
    std::thread::available_parallelism().map_or(1.0, |n| n.get() as f64)
}

/// The Rust heap's live bytes: every allocation through the global allocator, counted.
/// What the C and C++ libraries (skia, the system's DLLs) allocate isn't in it.
struct Counting;

static RUST_LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call goes to `System` as it came; only a counter is kept beside it.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's layout, passed on.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            RUST_LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as `alloc`.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            RUST_LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        // SAFETY: a block this allocator handed out, with its layout.
        unsafe { System.dealloc(p, layout) };
        RUST_LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: as `dealloc`, with the new size the caller asked for.
        let q = unsafe { System.realloc(p, layout, size) };
        if !q.is_null() {
            RUST_LIVE.fetch_add(size, Ordering::Relaxed);
            RUST_LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        q
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Into `m`, each name after `prefix`: Rust's live bytes, the heaps' parts, the committed
/// private regions and the decoded covers in the in-memory cache.
fn heap_parts(m: &mut Map<String, Value>, prefix: &str, covers: &crate::covers::Covers) {
    let h = featherplay_platform::perf::heap_walk();
    let (private, private_big) = featherplay_platform::perf::private_regions();
    let rust = RUST_LIVE.load(Ordering::Relaxed) as u64;
    for (name, bytes) in [
        ("rust", rust),
        ("heap_busy", h.busy),
        ("heap_committed", h.committed),
        ("heap_large", h.large),
        ("regions_private", private),
        ("regions_private_1mb", private_big),
        ("covers_cache", covers.cached_bytes() as u64),
    ] {
        m.insert(
            format!("{prefix}{name}_mb"),
            json!(round(bytes as f64 / 1e6)),
        );
    }
}

// ----- The report -----------------------------------------------------------------------

struct Out {
    file: Option<std::fs::File>,
    /// The process's creation, as an `Instant`.
    epoch: Instant,
    /// The display's frame interval, ms.
    period: f64,
}

impl Out {
    fn ms(&self, t: Instant) -> f64 {
        round((t - self.epoch).as_secs_f64() * 1000.0)
    }

    fn write(&mut self, mut record: Map<String, Value>) {
        record.insert("t".into(), json!(self.ms(Instant::now())));
        let line = Value::Object(record).to_string();
        log!("perf: {line}");
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}

fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn quantile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// The frames of a step: how many, their paint times, and the gaps inside runs of them
/// (a gap longer than 1.5 display frames is a frame missed).
fn frames(record: &mut Map<String, Value>, period: f64) {
    let paints = SEEN.with_borrow(|s| s.paints.clone());
    let mut cost: Vec<f64> = paints
        .iter()
        .map(|(a, b)| (*b - *a).as_secs_f64() * 1000.0)
        .collect();
    let mut gaps: Vec<f64> = paints
        .windows(2)
        .map(|w| w[1].0 - w[0].0)
        .filter(|g| *g < RUN_GAP)
        .map(|g| g.as_secs_f64() * 1000.0)
        .collect();
    let missed = gaps.iter().filter(|g| **g > period * 1.5).count();
    record.insert("frames".into(), json!(paints.len()));
    record.insert("paint_ms_total".into(), json!(round(cost.iter().sum())));
    record.insert(
        "paint_ms_p50".into(),
        json!(round(quantile(&mut cost, 0.5))),
    );
    record.insert(
        "paint_ms_p95".into(),
        json!(round(quantile(&mut cost, 0.95))),
    );
    record.insert(
        "paint_ms_max".into(),
        json!(round(quantile(&mut cost, 1.0))),
    );
    record.insert("gap_ms_max".into(), json!(round(quantile(&mut gaps, 1.0))));
    record.insert("frames_missed".into(), json!(missed));
}

/// Starts a step: forgets what earlier steps saw.
fn begin() -> (Instant, f64) {
    SEEN.with_borrow_mut(|s| {
        s.paints.clear();
        s.marks.clear();
    });
    if let Ok(mut d) = DECODES.lock() {
        *d = (0, 0.0, 0.0, 0.0);
    }
    (Instant::now(), cpu_ms())
}

/// The threads that used the most CPU since `before`, taken as the step began at `t0` (as
/// `idle` lists them; outside the step's `cpu_ms`).
fn threads(record: &mut Map<String, Value>, before: &Sample, t0: Instant) {
    let wall = t0.elapsed().as_secs_f64();
    record.insert(
        "busiest".into(),
        Value::Array(before.busiest_until(&sample(), wall)),
    );
}

fn decodes(record: &mut Map<String, Value>) {
    if let Ok(d) = DECODES.lock() {
        record.insert("covers_decoded".into(), json!(d.0));
        record.insert("decode_ms_total".into(), json!(round(d.1)));
        record.insert("decode_ms_max".into(), json!(round(d.2)));
        record.insert("decode_alone_ms_total".into(), json!(round(d.3)));
    }
}

fn first_mark(kind: Mark, after: Instant) -> Option<Instant> {
    SEEN.with_borrow(|s| {
        s.marks
            .iter()
            .find(|(m, t)| *m == kind && *t >= after)
            .map(|(_, t)| *t)
    })
}

fn first_paint_after(after: Instant) -> Option<Instant> {
    SEEN.with_borrow(|s| s.paints.iter().find(|(a, _)| *a >= after).map(|(_, b)| *b))
}

fn ms_since(t0: Instant, t: Option<Instant>) -> Value {
    t.map_or(Value::Null, |t| {
        json!(round((t - t0).as_secs_f64() * 1000.0))
    })
}

/// Waits for a step's work to be in and drawn: no background work pending, and a paint since
/// it dropped to none (or nothing to paint for `QUIET`); then for its animations to end.
/// Writes `first_paint`, `loaded` (first page load applied), `first_cover`, `settled` and
/// `quiet`, in ms from `t0`.
async fn settle(record: &mut Map<String, Value>, t0: Instant, covers: &crate::covers::Covers) {
    // When the work was in, and the end of the first paint after that, if any.
    let mut done: (Instant, Option<Instant>) = (t0, None);
    let settled = ticking(STEP_TIMEOUT, || {
        if crate::busy::pending() > 0 || covers.pacing() {
            return false;
        }
        SEEN.with_borrow(|s| {
            // The last of: the work pending dropped to none, a card got its cover.
            let shown = s
                .marks
                .iter()
                .rev()
                .find(|(m, _)| *m == Mark::CoverShown)
                .map(|(_, t)| *t);
            let zero = idle_at().max(shown).filter(|z| *z >= t0).unwrap_or(t0);
            let painted = s.paints.iter().find(|(a, _)| *a >= zero).map(|(_, b)| *b);
            done = (zero, painted);
            painted.is_some() || Instant::now() - zero >= QUIET
        })
    })
    .await;
    let settled_at = if settled {
        done.1.unwrap_or(done.0)
    } else {
        record.insert("timeout".into(), json!(true));
        Instant::now()
    };
    // Then the frames stop.
    ticking(Duration::from_secs(10), || {
        SEEN.with_borrow(|s| s.last_paint.is_none_or(|p| Instant::now() - p >= QUIET))
    })
    .await;
    let quiet_at = SEEN.with_borrow(|s| s.last_paint).filter(|p| *p >= t0);
    record.insert("first_paint".into(), ms_since(t0, first_paint_after(t0)));
    record.insert("loaded".into(), ms_since(t0, first_mark(Mark::Load, t0)));
    record.insert(
        "first_cover".into(),
        ms_since(t0, first_mark(Mark::Cover, t0)),
    );
    record.insert("settled".into(), ms_since(t0, Some(settled_at)));
    record.insert(
        "quiet".into(),
        ms_since(t0, Some(quiet_at.unwrap_or(settled_at).max(settled_at))),
    );
}

// ----- The script -----------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Cmd {
    Line(usize, Vec<String>),
    Repeat(u32, Vec<Cmd>),
}

fn parse(text: &str) -> Result<Vec<Cmd>, String> {
    let mut stack: Vec<(u32, Vec<Cmd>)> = vec![(1, Vec::new())];
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        match words[0].as_str() {
            "repeat" => {
                let times = words
                    .get(1)
                    .and_then(|w| w.parse().ok())
                    .ok_or(format!("line {}: repeat takes a count", n + 1))?;
                stack.push((times, Vec::new()));
            }
            "end" => {
                let (times, body) = stack
                    .pop()
                    .filter(|_| !stack.is_empty())
                    .ok_or(format!("line {}: end without repeat", n + 1))?;
                if let Some(top) = stack.last_mut() {
                    top.1.push(Cmd::Repeat(times, body));
                }
            }
            _ => {
                if let Some(top) = stack.last_mut() {
                    top.1.push(Cmd::Line(n + 1, words));
                }
            }
        }
    }
    if stack.len() != 1 {
        return Err("a repeat has no end".into());
    }
    Ok(stack.pop().map(|s| s.1).unwrap_or_default())
}

/// Plays row `i` of a page's song list.
type PlayRow = Box<dyn Fn(i32)>;

struct Driver {
    ctx: Rc<Ctx>,
    handle: Handle,
    out: Out,
    reps: Vec<u32>,
}

/// Starts the script, once the window exists (`app::run_window`).
pub fn start(ctx: &Rc<Ctx>, handle: Handle) {
    if !on() {
        return;
    }
    let ready = Instant::now();
    let epoch = ready - since_creation();
    let path = std::env::var_os("FEATHERPLAY_PERF_SCRIPT").map(PathBuf::from);
    let script = match path.as_ref().map(std::fs::read_to_string) {
        Some(Ok(text)) => parse(&text),
        Some(Err(e)) => Err(format!("can't read the script: {e}")),
        None => Err("no script".into()),
    };
    let out_path = std::env::var_os("FEATHERPLAY_PERF_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            featherplay_store::Paths::resolve()
                .cache
                .join("logs/perf.jsonl")
        });
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&out_path)
        .map_err(|e| log!("perf: {}: {e}", out_path.display()))
        .ok();
    let period = ctx
        .window()
        .and_then(|ui| {
            ui.window()
                .with_winit_window(|w| w.current_monitor()?.refresh_rate_millihertz())
                .flatten()
        })
        .map_or(1000.0 / 60.0, |mhz| 1e6 / f64::from(mhz));
    let mut driver = Driver {
        ctx: ctx.clone(),
        handle,
        out: Out {
            file,
            epoch,
            period,
        },
        reps: Vec::new(),
    };
    let script = match script {
        Ok(s) => s,
        Err(e) => {
            log!("perf: {e}");
            driver.record("error", &[], Map::new(), Some(e));
            return;
        }
    };
    let mut info = Map::new();
    info.insert("window_ready".into(), json!(driver.out.ms(ready)));
    info.insert("display_hz".into(), json!(round(1000.0 / period)));
    info.insert("cpus".into(), json!(logical_cpus()));
    info.insert("fixtures".into(), json!(fixtures()));
    info.insert("version".into(), json!(env!("CARGO_PKG_VERSION")));
    driver.record("begin", &[], info, None);
    let spawned = slint::spawn_local(async move {
        driver.run(&script).await;
        driver.record("end", &[], Map::new(), None);
    });
    if let Err(e) = spawned {
        log!("perf: {e}");
    }
}

impl Driver {
    fn record(
        &mut self,
        step: &str,
        words: &[String],
        mut m: Map<String, Value>,
        error: Option<String>,
    ) {
        m.insert("step".into(), json!(step));
        m.insert("arg".into(), json!(words.get(1..).unwrap_or(&[]).join(" ")));
        if !self.reps.is_empty() {
            m.insert("rep".into(), json!(self.reps));
        }
        if let Some(e) = error {
            m.insert("error".into(), json!(e));
        }
        self.out.write(m);
    }

    fn run<'a>(&'a mut self, script: &'a [Cmd]) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
        Box::pin(async move {
            for cmd in script {
                match cmd {
                    Cmd::Line(n, words) => {
                        if let Err(e) = self.step(words).await {
                            log!("perf: line {n}: {e}");
                            self.record(&words[0], words, Map::new(), Some(e));
                        }
                    }
                    Cmd::Repeat(times, body) => {
                        for i in 0..*times {
                            self.reps.push(i);
                            self.run(body).await;
                            self.reps.pop();
                        }
                    }
                }
            }
        })
    }

    fn ui(&self) -> Result<crate::ui::AppWindow, String> {
        self.ctx.window().ok_or_else(|| "no window".to_string())
    }

    async fn step(&mut self, words: &[String]) -> Result<(), String> {
        let ui = self.ui()?;
        let arg = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
        let num = |i: usize| -> Result<i32, String> {
            arg(i)
                .parse()
                .map_err(|_| format!("{}: a number expected", words[0]))
        };
        let rest = words.get(1..).unwrap_or(&[]).join(" ");
        match words[0].as_str() {
            "startup" => self.startup(words).await,
            "go" => {
                let kind = match arg(1) {
                    "home" => PageKind::Home,
                    "explore" => PageKind::Explore,
                    "history" => PageKind::History,
                    "library" => PageKind::Library,
                    "now-playing" => PageKind::NowPlaying,
                    "settings" => PageKind::Settings,
                    "about" => PageKind::About,
                    other => return Err(format!("go: no page {other:?}")),
                };
                self.nav(words, || ui.global::<Nav>().invoke_go(kind)).await
            }
            "library" => {
                let tab = num(1)?;
                self.nav(words, || ui.global::<Nav>().invoke_open_library(tab))
                    .await
            }
            "explore" => {
                let tab = num(1)?;
                self.nav(words, || ui.global::<Nav>().invoke_open_explore(tab))
                    .await
            }
            "playlist" => {
                let id = SharedString::from(arg(1));
                self.nav(words, || ui.global::<Nav>().invoke_open_playlist(id))
                    .await
            }
            "open-card" => {
                let card = self.card(num(1)? as usize)?;
                let nav = ui.global::<Nav>();
                self.nav(words, || match card.kind {
                    ItemKind::Album => nav.invoke_open_album(card.id),
                    ItemKind::Artist => nav.invoke_open_artist(card.id),
                    _ => nav.invoke_open_playlist(card.id),
                })
                .await
            }
            "back" => self.nav(words, || ui.global::<Nav>().invoke_back()).await,
            "forward" => {
                self.nav(words, || ui.global::<Nav>().invoke_forward())
                    .await
            }
            "search" => {
                let q = SharedString::from(rest.as_str());
                ui.global::<TopBarData>().set_query(q.clone());
                self.nav(words, || ui.global::<Nav>().invoke_search(q))
                    .await
            }
            "press" => {
                let item = find(&ui, &rest).ok_or(format!("nothing visible says {rest:?}"))?;
                self.nav(words, || {
                    item.accessible_action(&AccessibilityAction::Default);
                })
                .await
            }
            "np-tab" => {
                let tab = num(1)?;
                let np = ui.global::<NowPlayingData>();
                self.nav(words, || {
                    np.set_tab(tab);
                    np.invoke_tab_selected(tab);
                })
                .await
            }
            "size" => {
                let (w, h) = arg(1)
                    .split_once('x')
                    .and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?)))
                    .ok_or("size <w>x<h>")?;
                self.nav(words, || ui.window().set_size(LogicalSize::new(w, h)))
                    .await
            }
            "type" => self.typing(words).await,
            "scroll" => self.scroll(words).await,
            "resize" => self.resize(words).await,
            "play-row" => {
                let (rows, play) = self.page_rows()?;
                let i = num(1)?;
                let row = rows.row_data(i as usize).ok_or("no such row")?;
                self.song_start(words, Some(row.id.to_string()), || play(i))
                    .await
            }
            "queue" => {
                let i = num(1)?;
                let q = ui.global::<QueueData>();
                let row = q
                    .get_upcoming()
                    .row_data(i as usize)
                    .ok_or("no such queue row")?;
                self.song_start(words, Some(row.id.to_string()), || q.invoke_play(i))
                    .await
            }
            "next" => {
                self.song_start(words, None, || ui.global::<Player>().invoke_next())
                    .await
            }
            "previous" => {
                self.song_start(words, None, || ui.global::<Player>().invoke_previous())
                    .await
            }
            "play" => {
                if SEEN.with_borrow(|s| s.player.playing) {
                    return Err("already playing".into());
                }
                let id = SEEN.with_borrow(|s| s.player.id.clone());
                self.song_start(words, Some(id), || ui.global::<Player>().invoke_toggle())
                    .await
            }
            "pause" => {
                let (t0, _) = begin();
                ui.global::<Player>().invoke_toggle();
                let done = until(STEP_TIMEOUT, || SEEN.with_borrow(|s| !s.player.playing)).await;
                let mut m = Map::new();
                m.insert(
                    "paused".into(),
                    json!(round(t0.elapsed().as_secs_f64() * 1000.0)),
                );
                if !done {
                    m.insert("timeout".into(), json!(true));
                }
                self.record("pause", words, m, None);
                Ok(())
            }
            "rest-card" => {
                // `PageIntent`'s report once the pointer has stood still over the card.
                let card = self.card(num(1)? as usize)?;
                ui.global::<PageHover>().invoke_rested(card.kind, card.id);
                Ok(())
            }
            "rest" => {
                let (rows, _) = self.page_rows()?;
                let row = rows.row_data(num(1)? as usize).ok_or("no such row")?;
                ui.global::<RowHover>().invoke_rested(row.id);
                Ok(())
            }
            "wait" => {
                sleep(Duration::from_millis(num(1)? as u64)).await;
                Ok(())
            }
            "idle" => self.idle(words).await,
            "mem" => {
                let s = sample();
                let mut m = Map::new();
                m.insert("private_mb".into(), json!(round(s.private_mb)));
                m.insert("working_set_mb".into(), json!(round(s.working_set_mb)));
                m.insert("threads".into(), json!(s.threads));
                heap_parts(&mut m, "", &self.ctx.covers);
                self.record("mem", words, m, None);
                Ok(())
            }
            "mem-parts" => {
                // What each holder lets go of, in turn: the renderer's caches and buffer,
                // the covers, the heaps' free pages (`memory::hidden` lets go of the same),
                // then everything back as a show brings it. Shown windows only.
                let mut m = Map::new();
                let covers = self.ctx.covers.clone();
                let at = |m: &mut Map<String, Value>, part: &str| {
                    m.insert(
                        format!("{part}private_mb"),
                        json!(round(sample().private_mb)),
                    );
                    heap_parts(m, part, &covers);
                };
                at(&mut m, "");
                crate::memory::release_drawing(ui.window());
                at(&mut m, "no_drawing_");
                covers.show(None);
                at(&mut m, "no_covers_");
                crate::memory::compact_heaps();
                at(&mut m, "compacted_");
                let t0 = Instant::now();
                covers.show(Some(self.ctx.current_page().kind()));
                crate::memory::resume_drawing(ui.window());
                settle(&mut m, t0, &covers).await;
                self.record("mem-parts", words, m, None);
                Ok(())
            }
            "minimize" | "restore" => {
                let minimise = words[0] == "minimize";
                let handle = self.handle;
                self.nav(words, || {
                    featherplay_platform::perf::show_minimised(handle, minimise);
                })
                .await
            }
            "tray" => {
                self.nav(words, || {
                    ui.window().dispatch_event(WindowEvent::CloseRequested)
                })
                .await
            }
            "show" => {
                let ctx = self.ctx.clone();
                self.nav(words, || crate::app::show_window(&ctx)).await
            }
            "mini" => {
                let (w, h) = featherplay_platform::perf::screen_size();
                let (t0, c0) = begin();
                crate::mini::toggle(w - 200, h - 20);
                let call = t0.elapsed();
                turn().await;
                turn().await;
                let mut m = Map::new();
                m.insert("call".into(), json!(round(call.as_secs_f64() * 1000.0)));
                m.insert("open".into(), json!(crate::mini::is_open()));
                m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
                self.record("mini", words, m, None);
                Ok(())
            }
            "quit" => {
                self.record("quit", words, Map::new(), None);
                self.ctx.quit();
                Ok(())
            }
            other => Err(format!("unknown step {other:?}")),
        }
    }

    /// The process's start, measured from its creation: the window, its first frame, the
    /// account, the session restored, Home's load, everything in and drawn, and quiet.
    async fn startup(&mut self, words: &[String]) -> Result<(), String> {
        let epoch = self.out.epoch;
        until(Duration::from_secs(60), || {
            SEEN.with_borrow(|s| s.account_ever.is_some())
        })
        .await;
        let mut m = Map::new();
        settle(&mut m, epoch, &self.ctx.covers).await;
        // `settle` saw only what came since this step began; the start's marks are kept.
        let (first_paint, account, restored) =
            SEEN.with_borrow(|s| (s.first_paint_ever, s.account_ever, s.restored_ever));
        m.insert("first_paint".into(), ms_since(epoch, first_paint));
        m.insert("account".into(), ms_since(epoch, account));
        m.insert("restored".into(), ms_since(epoch, restored));
        let s = sample();
        m.insert("cpu_ms".into(), json!(round(s.cpu_ms)));
        m.insert("private_mb".into(), json!(round(s.private_mb)));
        m.insert("working_set_mb".into(), json!(round(s.working_set_mb)));
        frames(&mut m, self.out.period);
        decodes(&mut m);
        self.record("startup", words, m, None);
        Ok(())
    }

    /// A navigation or any step whose result is a page: runs `act`, then waits for it to
    /// settle.
    async fn nav(&mut self, words: &[String], act: impl FnOnce()) -> Result<(), String> {
        // Taken before the step's clock starts, so it isn't in the step's times.
        let before = sample();
        let (t0, c0) = begin();
        act();
        let mut m = Map::new();
        settle(&mut m, t0, &self.ctx.covers).await;
        m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
        frames(&mut m, self.out.period);
        decodes(&mut m);
        threads(&mut m, &before, t0);
        self.record(&words[0], words, m, None);
        Ok(())
    }

    /// A song start: from `act` until `want` (or, with none, any other song) is audible.
    async fn song_start(
        &mut self,
        words: &[String],
        want: Option<String>,
        act: impl FnOnce(),
    ) -> Result<(), String> {
        let before = SEEN.with_borrow(|s| s.player.id.clone());
        let (t0, c0) = begin();
        act();
        let playing = until(PLAY_TIMEOUT, || {
            SEEN.with_borrow(|s| {
                let p = &s.player;
                let right = match &want {
                    Some(id) => p.id == *id,
                    None => !p.id.is_empty() && p.id != before,
                };
                right && p.audible
            })
        })
        .await;
        let mut m = Map::new();
        m.insert("audible".into(), ms_since(t0, playing.then(Instant::now)));
        m.insert("first_paint".into(), ms_since(t0, first_paint_after(t0)));
        m.insert(
            "id".into(),
            json!(SEEN.with_borrow(|s| s.player.id.clone())),
        );
        m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
        if !playing {
            m.insert("timeout".into(), json!(true));
        }
        frames(&mut m, self.out.period);
        self.record(&words[0], words, m, None);
        Ok(())
    }

    /// `type <ms per key> <text>`: the field's text grows a letter every `ms`, as typed; then
    /// the suggestions for the whole text. Writes the time from the first key to the first
    /// suggestions shown, and from the last key to the whole text's suggestions.
    async fn typing(&mut self, words: &[String]) -> Result<(), String> {
        let ui = self.ui()?;
        let every: u64 = words
            .get(1)
            .and_then(|w| w.parse().ok())
            .ok_or("type <ms per key> <text>")?;
        let text = words.get(2..).unwrap_or(&[]).join(" ");
        let (t0, c0) = begin();
        let field = ui.global::<TopBarData>();
        field.set_query(SharedString::new());
        let mut keys = Vec::new();
        for end in text.char_indices().map(|(i, c)| i + c.len_utf8()) {
            let prefix = SharedString::from(&text[..end]);
            keys.push(Instant::now());
            field.set_query(prefix.clone());
            field.invoke_edited(prefix);
            sleep(Duration::from_millis(every)).await;
        }
        let last = keys.last().copied().unwrap_or(t0);
        let shown = until(Duration::from_secs(10), || {
            first_mark(Mark::SuggestTyped, last).is_some()
        })
        .await;
        let mut m = Map::new();
        m.insert("keys".into(), json!(keys.len()));
        m.insert(
            "first_suggestions".into(),
            ms_since(t0, first_mark(Mark::Suggest, t0)),
        );
        m.insert(
            "last_key_to_suggestions".into(),
            ms_since(last, first_mark(Mark::SuggestTyped, last)),
        );
        m.insert(
            "suggestions_shown".into(),
            json!(
                SEEN.with_borrow(|s| s.marks.iter().filter(|(k, _)| *k == Mark::Suggest).count())
            ),
        );
        if !shown {
            m.insert("timeout".into(), json!(true));
        }
        m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
        frames(&mut m, self.out.period);
        self.record("type", words, m, None);
        Ok(())
    }

    /// `scroll <events> <notches> [ms between]`: wheel notches at the middle of the content,
    /// as a wheel sends them; then until the list stops.
    async fn scroll(&mut self, words: &[String]) -> Result<(), String> {
        let ui = self.ui()?;
        let get = |i: usize, d: f32| words.get(i).and_then(|w| w.parse().ok()).unwrap_or(d);
        let (events, notches, every) = (get(1, 20.0) as u32, get(2, -1.0), get(3, 50.0));
        let w = ui.window();
        let size = w.size().to_logical(w.scale_factor());
        let at = LogicalPosition::new(size.width * 0.55, size.height * 0.5);
        let before = sample();
        let (t0, c0) = begin();
        w.dispatch_event(WindowEvent::PointerMoved { position: at });
        // A wheel under a still pointer, as `wheel` sets it from winit's events: the rows
        // passing under the pointer don't take the hover.
        ui.global::<Frame>().set_scrolling(true);
        for _ in 0..events {
            crate::offscreen::wheel(w, at, notches);
            sleep(Duration::from_millis(every as u64)).await;
        }
        let mut m = Map::new();
        settle(&mut m, t0, &self.ctx.covers).await;
        m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
        threads(&mut m, &before, t0);
        frames(&mut m, self.out.period);
        decodes(&mut m);
        self.record("scroll", words, m, None);
        ui.global::<Frame>().set_scrolling(false);
        w.dispatch_event(WindowEvent::PointerExited);
        Ok(())
    }

    /// `resize <w>x<h> [steps]`: the window's size moves there in `steps` frames and back,
    /// as a drag of its edge would.
    async fn resize(&mut self, words: &[String]) -> Result<(), String> {
        let ui = self.ui()?;
        let (tw, th) = words
            .get(1)
            .and_then(|v| v.split_once('x'))
            .and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?)))
            .ok_or("resize <w>x<h> [steps]")?;
        let steps: u32 = words.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
        let w = ui.window();
        let from = w.size().to_logical(w.scale_factor());
        let before = sample();
        let (t0, c0) = begin();
        // As a drag of the window's edge sets it (`featherplay_platform::window::when_sizing`).
        crate::live_resize::set(w, true);
        for (a, b) in [
            ((from.width, from.height), (tw, th)),
            ((tw, th), (from.width, from.height)),
        ] {
            for i in 1..=steps {
                let f = i as f32 / steps as f32;
                w.set_size(LogicalSize::new(
                    a.0 + (b.0 - a.0) * f,
                    a.1 + (b.1 - a.1) * f,
                ));
                sleep(Duration::from_millis(16)).await;
            }
        }
        crate::live_resize::set(w, false);
        let mut m = Map::new();
        settle(&mut m, t0, &self.ctx.covers).await;
        m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
        threads(&mut m, &before, t0);
        frames(&mut m, self.out.period);
        decodes(&mut m);
        self.record("resize", words, m, None);
        Ok(())
    }

    /// `idle <seconds> <name>`: nothing done; the process's CPU (as Windows charges it, and
    /// from its cycles), wakeups and memory over it.
    async fn idle(&mut self, words: &[String]) -> Result<(), String> {
        let secs: f64 = words
            .get(1)
            .and_then(|w| w.parse().ok())
            .ok_or("idle <seconds> <name>")?;
        let (t0, _) = begin();
        let a = sample();
        sleep(Duration::from_secs_f64(secs)).await;
        let b = sample();
        let wall = t0.elapsed().as_secs_f64();
        let cpu = b.cpu_ms - a.cpu_ms;
        let mut m = Map::new();
        m.insert("seconds".into(), json!(round(wall)));
        m.insert("cpu_ms".into(), json!(round(cpu)));
        m.insert(
            "cpu_pct_machine".into(),
            json!((cpu / (wall * 1000.0 * logical_cpus()) * 1e5).round() / 1e3),
        );
        m.insert(
            "wakeups_per_s".into(),
            json!(round(a.switches_until(&b) as f64 / wall)),
        );
        m.insert("busiest".into(), Value::Array(a.busiest_until(&b, wall)));
        // The same from the cycles: what the process ran, not the clock ticks it was charged.
        let ticked = b.tsc.saturating_sub(a.tsc) as f64;
        // A failed sample has no TSC (0), nor has an ARM64 build.
        if a.tsc > 0 && ticked > 0.0 {
            let ran = b.proc_cycles.saturating_sub(a.proc_cycles) as f64;
            m.insert(
                "cycle_ms".into(),
                json!(round(ran / ticked * wall * 1000.0)),
            );
            m.insert(
                "cycle_pct_machine".into(),
                json!((ran / (ticked * logical_cpus()) * 1e5).round() / 1e3),
            );
        }
        m.insert("private_mb".into(), json!(round(b.private_mb)));
        m.insert("working_set_mb".into(), json!(round(b.working_set_mb)));
        m.insert("threads".into(), json!(b.threads));
        m.insert(
            "playing".into(),
            json!(SEEN.with_borrow(|s| s.player.audible)),
        );
        frames(&mut m, self.out.period);
        // When the spell's first paints began, in ms: what draws while nothing should.
        let at: Vec<Value> = SEEN.with_borrow(|s| {
            s.paints
                .iter()
                .take(64)
                .map(|(start, _)| json!(round((*start - t0).as_secs_f64() * 1000.0)))
                .collect()
        });
        m.insert("paints_at_ms".into(), Value::Array(at));
        self.record("idle", words, m, None);
        Ok(())
    }

    /// Card `i` of the page shown (Library's tabs, search results, an artist's albums, and
    /// Home's cards that open a page, counted across its shelves, top first).
    fn card(&self, i: usize) -> Result<crate::ui::CardItem, String> {
        let ui = self.ui()?;
        let cards = match self.ctx.current_page().kind() {
            PageKind::Home => {
                let shelves = ui.global::<HomeData>().get_shelves();
                return shelves
                    .iter()
                    .flat_map(|shelf| shelf.items.iter().collect::<Vec<_>>())
                    .filter(|card| card.kind != ItemKind::Song)
                    .nth(i)
                    .ok_or_else(|| format!("open-card: no card {i} on Home"));
            }
            PageKind::Library => ui.global::<LibraryData>().get_cards(),
            PageKind::Search => ui.global::<SearchData>().get_cards(),
            PageKind::Artist => ui.global::<ArtistData>().get_albums(),
            other => return Err(format!("open-card: no cards on {other:?}")),
        };
        cards
            .row_data(i)
            .ok_or_else(|| format!("open-card: no card {i}"))
    }

    /// The song rows of the page shown, and how one of them is played.
    fn page_rows(&self) -> Result<(ModelRc<TrackRow>, PlayRow), String> {
        let ui = self.ui()?;
        let w = ui.as_weak();
        macro_rules! rows {
            ($global:ident, $get:ident, $play:ident) => {{
                let rows = ui.global::<$global>().$get();
                let play: Box<dyn Fn(i32)> = Box::new(move |i| {
                    if let Some(ui) = w.upgrade() {
                        ui.global::<$global>().$play(i);
                    }
                });
                (rows, play)
            }};
        }
        Ok(match self.ctx.current_page().kind() {
            PageKind::Playlist => rows!(PlaylistData, get_tracks, invoke_play),
            PageKind::Album => rows!(AlbumData, get_tracks, invoke_play),
            PageKind::Library => rows!(LibraryData, get_liked, invoke_play),
            PageKind::History => rows!(HistoryData, get_tracks, invoke_play),
            PageKind::Search => rows!(SearchData, get_songs, invoke_play),
            PageKind::Artist => rows!(ArtistData, get_top_songs, invoke_play),
            PageKind::Home => rows!(HomeData, get_next, invoke_play_next),
            other => return Err(format!("no song rows on {other:?}")),
        })
    }
}

/// The first visible element whose accessible label is `label`.
fn find(ui: &crate::ui::AppWindow, label: &str) -> Option<ItemRc> {
    let root = ItemRc::new_root(WindowInner::from_pub(ui.window()).component());
    root.visit_descendants(|item| {
        let says = item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|l| l == label);
        if says && item.is_visible() {
            ControlFlow::Break(item.clone())
        } else {
            ControlFlow::Continue(())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_nests_its_repeats() {
        let s = parse(
            "go home # first\nrepeat 2\n  go explore\n  repeat 3\n    back\n  end\nend\nquit\n",
        )
        .expect("parse");
        assert_eq!(s.len(), 3);
        let Cmd::Repeat(2, body) = &s[1] else {
            panic!("no repeat: {s:?}")
        };
        assert!(matches!(&body[1], Cmd::Repeat(3, inner) if inner.len() == 1));
    }

    #[test]
    fn a_wait_keeps_one_waker_however_often_it_is_polled() {
        struct Task;
        #[expect(
            clippy::manual_noop_waker,
            reason = "Waker::noop() doesn't say its copies wake the same task"
        )]
        impl std::task::Wake for Task {
            fn wake(self: std::sync::Arc<Self>) {}
        }
        // One task's waker, as the script's.
        let waker = Waker::from(std::sync::Arc::new(Task));
        let mut cx = Context::from_waker(&waker);
        let mut wait = until(Duration::from_secs(60), || false);
        for _ in 0..5 {
            assert!(Pin::new(&mut wait).poll(&mut cx).is_pending());
        }
        assert_eq!(SEEN.with_borrow(|s| s.wakers.len()), 1);
    }

    #[test]
    fn an_unclosed_repeat_is_an_error() {
        assert!(parse("repeat 2\ngo home\n").is_err());
        assert!(parse("end\n").is_err());
    }
}
