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

use std::cell::RefCell;
use std::sync::{Mutex, OnceLock};
use std::task::Waker;
use std::time::{Duration, Instant};

use crate::controller::PlayerState;

mod process;
mod report;
mod script;
mod wait;

pub use script::start;

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
