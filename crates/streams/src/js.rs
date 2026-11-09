//! YouTube's player challenges, answered in QuickJS inside the app (QuickJS-NG, through
//! `rquickjs`) with yt-dlp's EJS scripts (`ejs/`). Until 0.2.1 a deno process did this,
//! ~185 ms of it per song.
//!
//! The player is YouTube's code, so each run gets a runtime of its own on a thread of its
//! own, with what deno's sandbox gave it:
//! - no host functions: the context holds the language's own objects and nothing else, so
//!   no code in it can read a file, the network or the environment;
//! - a memory limit, a stack limit (on a thread whose own stack is larger), and a time
//!   limit (the interrupt handler), which also ends a run nobody waits for any more.
//!
//! Nothing outlives a run: the runtime goes with its thread, and its memory with it.
//!
//! Compiling a preprocessed player (4 MB of JS) takes ~190 ms, so its bytecode is kept
//! beside it (`{id}.bytecode`) and a resolve loads that. QuickJS doesn't check bytecode
//! it reads, so the file is trusted like the helpers beside it: it's the app's own, in the
//! user's profile. Its first line ties it to the app version, the engine and the player
//! file, and carries a checksum, so one that's stale or damaged is made again from the
//! source.

use std::ffi::CStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use rquickjs::{Context, Ctx, Function, Object, Runtime, Value, qjs};
use serde_json::json;

use crate::native::Answers;

/// yt-dlp's EJS challenge solver, v0.8.0 (Unlicense; the lib bundles meriyah, ISC, and
/// astring, MIT). Only preprocessing a new player needs them. The lib is deflated by
/// `build.rs` ([`ejs_lib`]).
const EJS_LIB_DEFLATED: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/yt.solver.lib.min.js.deflate"));
const EJS_CORE: &str = include_str!("../ejs/yt.solver.core.min.js");
/// Preprocessed players carry it; one made by other scripts is made again.
pub(crate) const EJS_VERSION: &str = "0.8.0";

/// What a run may use.
#[derive(Clone, Copy)]
struct Limits {
    /// The runtime's heap, in bytes.
    memory: usize,
    /// The engine's stack, in bytes.
    stack: usize,
    /// The thread's stack: larger, as the engine checks its depth between calls and a
    /// native call in between needs room too.
    thread_stack: usize,
    /// For the load, and again for the challenges.
    time: Duration,
}

/// A loaded player holds 11-24 MB of heap (stripped or whole, `STRIPPED`), and compiling
/// one from its source takes between 128 and 256 KB of stack (2026-11-09).
const SOLVE: Limits = Limits {
    memory: 256 << 20,
    stack: 6 << 20,
    thread_stack: 8 << 20,
    time: Duration::from_secs(10),
};
/// EJS parses the whole player (3 MB) with a JS parser written in JS: ~3.8 s, ~220 MB of
/// the process at its peak, and under 1 MB of stack (2026-11-09).
const PREPROCESS: Limits = Limits {
    memory: 768 << 20,
    stack: 16 << 20,
    thread_stack: 24 << 20,
    time: Duration::from_secs(30),
};

/// The engine's name and version, as the diagnostics show it: `QuickJS-NG 0.16.2`.
pub(crate) fn engine() -> String {
    // SAFETY: `JS_GetVersion` returns a pointer to a static NUL-terminated string.
    let version = unsafe { CStr::from_ptr(qjs::JS_GetVersion()) };
    format!("QuickJS-NG {}", version.to_string_lossy())
}

/// When a run must stop: its time is up, or nobody waits for it any more.
struct Watch {
    started: Instant,
    /// Milliseconds after `started`.
    deadline: AtomicU64,
    cancelled: AtomicBool,
    /// `go_on_alone`.
    alone: AtomicBool,
}

impl Watch {
    fn new(time: Duration) -> Arc<Self> {
        let watch = Self {
            started: Instant::now(),
            deadline: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
            alone: AtomicBool::new(false),
        };
        watch.allow(time);
        Arc::new(watch)
    }

    /// `time` from now.
    fn allow(&self, time: Duration) {
        let until = self.started.elapsed() + time;
        self.deadline
            .store(until.as_millis() as u64, Ordering::Relaxed);
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// From now on the run goes on for itself (it saves a player's bytecode): only its time
    /// stops it, not a caller that stopped waiting.
    fn go_on_alone(&self) {
        self.alone.store(true, Ordering::Relaxed);
    }

    fn timed_out(&self) -> bool {
        self.started.elapsed().as_millis() as u64 >= self.deadline.load(Ordering::Relaxed)
    }

    /// The time left before the deadline.
    fn left(&self) -> Duration {
        Duration::from_millis(self.deadline.load(Ordering::Relaxed))
            .saturating_sub(self.started.elapsed())
    }

    /// The interrupt handler's answer: stop now.
    fn over(&self) -> bool {
        (self.cancelled.load(Ordering::Relaxed) && !self.alone.load(Ordering::Relaxed))
            || self.timed_out()
    }
}

/// Runs `job` in a fresh runtime on a new thread. The runtime is dropped before the
/// thread ends, after `job` returned.
fn spawn<T: Send + 'static>(
    limits: Limits,
    watch: Arc<Watch>,
    job: impl for<'js> FnOnce(&Ctx<'js>) -> T + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<Result<T, String>>> {
    std::thread::Builder::new()
        .name("player-js".into())
        .stack_size(limits.thread_stack)
        .spawn(move || {
            let failed = |e: rquickjs::Error| format!("starting QuickJS: {e}");
            #[cfg(windows)]
            let runtime = {
                let heap = heap::RunHeap::new().map_err(|e| format!("starting QuickJS: {e}"))?;
                Runtime::new_with_alloc(heap).map_err(failed)?
            };
            #[cfg(not(windows))]
            let runtime = Runtime::new().map_err(failed)?;
            runtime.set_memory_limit(limits.memory);
            runtime.set_max_stack_size(limits.stack);
            runtime.set_interrupt_handler(Some(Box::new(move || watch.over())));
            // Every intrinsic of the language, and no host function.
            let context = Context::full(&runtime).map_err(failed)?;
            Ok(context.with(|ctx| job(&ctx)))
        })
}

/// Each run's memory in a Windows heap of its own, destroyed with its runtime. In the process
/// heap, what a run frees mostly stays committed: 50 resolves (a loaded player each) left the
/// process 9.4 MB larger (8.2 to 17.6 MB private) and still growing ~30 KB a resolve; in a
/// heap of their own, 0.5 MB (8.3 to 8.8 MB), flat after the first 20 (2026-11-09).
#[cfg(windows)]
mod heap {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::ptr;

    use rquickjs::allocator::Allocator;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Memory::{
        HEAP_FLAGS, HEAP_ZERO_MEMORY, HeapAlloc, HeapCreate, HeapDestroy, HeapFree, HeapReAlloc,
        HeapSize,
    };

    thread_local! {
        /// This thread's run heap: `usable_size` gets no `self`, and a runtime lives and
        /// dies on one thread (`spawn`).
        static HEAP: Cell<HANDLE> = const { Cell::new(HANDLE(ptr::null_mut())) };
    }

    /// Only this thread allocates from it, but a heap without its lock gets no low-fragmentation
    /// front end either, and was slower.
    const FLAGS: HEAP_FLAGS = HEAP_FLAGS(0);

    pub(super) struct RunHeap(HANDLE);

    impl RunHeap {
        pub(super) fn new() -> windows::core::Result<Self> {
            // SAFETY: a growable heap with no initial size; destroyed in `drop`.
            let heap = unsafe { HeapCreate(FLAGS, 0, 0)? };
            HEAP.set(heap);
            Ok(Self(heap))
        }
    }

    impl Drop for RunHeap {
        fn drop(&mut self) {
            // The runtime drops its allocator after `JS_FreeRuntime`: nothing in the heap is
            // used any more.
            HEAP.set(HANDLE(ptr::null_mut()));
            // SAFETY: the heap is this value's own, and destroyed once.
            let _ = unsafe { HeapDestroy(self.0) };
        }
    }

    // SAFETY: `HeapAlloc` aligns to 16 bytes on x64 and ARM64, more than QuickJS needs (8),
    // and `HeapSize` gives each block's usable size.
    unsafe impl Allocator for RunHeap {
        fn alloc(&mut self, size: usize) -> *mut u8 {
            // SAFETY: the heap is alive while the runtime is.
            unsafe { HeapAlloc(self.0, FLAGS, size).cast() }
        }

        fn calloc(&mut self, count: usize, size: usize) -> *mut u8 {
            let Some(size) = count.checked_mul(size) else {
                return ptr::null_mut();
            };
            // SAFETY: as for `alloc`.
            unsafe { HeapAlloc(self.0, FLAGS | HEAP_ZERO_MEMORY, size).cast() }
        }

        unsafe fn dealloc(&mut self, block: *mut u8) {
            // SAFETY: `block` is this heap's, as the trait asks of the caller.
            let _ = unsafe { HeapFree(self.0, FLAGS, Some(block as *const c_void)) };
        }

        unsafe fn realloc(&mut self, block: *mut u8, new_size: usize) -> *mut u8 {
            if block.is_null() {
                return self.alloc(new_size);
            }
            // SAFETY: as for `dealloc`.
            unsafe { HeapReAlloc(self.0, FLAGS, Some(block as *const c_void), new_size).cast() }
        }

        unsafe fn usable_size(block: *mut u8) -> usize {
            // SAFETY: `block` is this thread's run heap's, as the trait asks of the caller.
            let size = unsafe { HeapSize(HEAP.get(), FLAGS, block as *const c_void) };
            if size == usize::MAX { 0 } else { size }
        }
    }
}

/// What a failed call threw, cut short for a log line.
fn thrown(ctx: &Ctx<'_>, e: &rquickjs::Error) -> String {
    let text = if e.is_exception() {
        let value = ctx.catch();
        match value.as_exception() {
            Some(x) => x.message().unwrap_or_default(),
            None => value
                .as_string()
                .and_then(|s| s.to_string().ok())
                .unwrap_or_else(|| format!("{value:?}")),
        }
    } else {
        e.to_string()
    };
    text.chars().take(300).collect()
}

/// `raw`, a value this context just returned and the caller owns, or what was thrown
/// instead of it.
fn owned<'js>(ctx: &Ctx<'js>, raw: qjs::JSValue) -> Result<Value<'js>, String> {
    // SAFETY: `raw` comes from this context's runtime and its reference is ours, as the
    // callers say.
    let value = unsafe { Value::from_raw(ctx.clone(), raw) };
    if value.is_exception() {
        Err(thrown(ctx, &rquickjs::Error::Exception))
    } else {
        Ok(value)
    }
}

/// A global script compiled and not run: its function bytecode.
fn compile<'js>(ctx: &Ctx<'js>, source: &str) -> Result<Value<'js>, String> {
    // `JS_Eval` reads a NUL after the source.
    let mut text = Vec::with_capacity(source.len() + 1);
    text.extend_from_slice(source.as_bytes());
    text.push(0);
    let flags = qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY;
    // SAFETY: `text` is `source.len()` bytes and a NUL, alive for the call; the name is a C
    // string; the context is this thread's own.
    let raw = unsafe {
        qjs::JS_Eval(
            ctx.as_raw().as_ptr(),
            text.as_ptr().cast(),
            source.len() as qjs::size_t,
            c"player".as_ptr(),
            flags as i32,
        )
    };
    owned(ctx, raw)
}

/// Runs compiled code (`compile`, `read_bytecode`): the script's value.
fn run<'js>(ctx: &Ctx<'js>, code: &Value<'js>) -> Result<Value<'js>, String> {
    // SAFETY: `code` is this context's function bytecode. `JS_EvalFunction` frees the
    // reference it's given, so it gets one of its own.
    let raw = unsafe {
        let ptr = ctx.as_raw().as_ptr();
        qjs::JS_EvalFunction(ptr, qjs::JS_DupValue(ptr, code.as_raw()))
    };
    owned(ctx, raw)
}

/// Where the bytecode of the player file `player` is kept.
pub(crate) fn bytecode_path(player: &Path) -> PathBuf {
    player.with_extension("bytecode")
}

/// A bytecode file's first line: what wrote it, for which player file (its length), and
/// the payload's CRC-32.
fn bytecode_header(player_len: u64, payload: &[u8]) -> String {
    let mut crc = flate2::Crc::new();
    crc.update(payload);
    format!(
        "featherplay bytecode {} {} {} js {player_len} crc {:08x}",
        env!("CARGO_PKG_VERSION"),
        engine(),
        std::env::consts::ARCH,
        crc.sum()
    )
}

/// Bytecode without the functions' source text and line tables. For a 4.2 MB player
/// (2026-11-09): 3.0 MB instead of 16.3 MB, 10.7 MB instead of 23.7 MB once loaded, and
/// 13 ms instead of 19 ms to load. Without their text, functions print as native ones
/// (`Function.prototype.toString`), so it's only for a player that answers the same
/// without it (`save_bytecode`).
const STRIPPED: u32 = qjs::JS_WRITE_OBJ_BYTECODE | qjs::JS_WRITE_OBJ_STRIP_DEBUG;
const WHOLE: u32 = qjs::JS_WRITE_OBJ_BYTECODE;

/// `code` (compiled, not run) as bytecode, written with `flags`.
fn to_bytecode(ctx: &Ctx<'_>, code: &Value<'_>, flags: u32) -> Option<Vec<u8>> {
    let mut size: qjs::size_t = 0;
    // SAFETY: `code` is this context's value, and `size` outlives the call.
    let buffer = unsafe {
        qjs::JS_WriteObject(
            ctx.as_raw().as_ptr(),
            &mut size,
            code.as_raw(),
            flags as i32,
        )
    };
    if buffer.is_null() {
        let why = thrown(ctx, &rquickjs::Error::Exception);
        log::warn!("a player's bytecode wasn't written: {why}");
        return None;
    }
    // SAFETY: `JS_WriteObject` returned `size` bytes at `buffer`, allocated by this context
    // and not used after they're copied and freed here.
    unsafe {
        let bytes = std::slice::from_raw_parts(buffer, size as usize).to_vec();
        qjs::js_free(ctx.as_raw().as_ptr(), buffer.cast());
        Some(bytes)
    }
}

/// The compiled code in `bytecode`, which must be this build's own (`to_bytecode`): QuickJS
/// doesn't check the bytecode it reads.
fn from_bytecode<'js>(ctx: &Ctx<'js>, bytecode: &[u8]) -> Result<Value<'js>, String> {
    // SAFETY: `bytecode` is alive for the call, which copies what it keeps, and was written
    // by this build, as the callers see to.
    let raw = unsafe {
        qjs::JS_ReadObject(
            ctx.as_raw().as_ptr(),
            bytecode.as_ptr(),
            bytecode.len() as qjs::size_t,
            qjs::JS_READ_OBJ_BYTECODE as i32,
        )
    };
    owned(ctx, raw)
}

/// The bytecode in the file beside the player file `player`, if that file was written by
/// this build for this player file and is whole.
fn bytecode_file(player: &Path) -> Option<Vec<u8>> {
    let mut file = std::fs::read(bytecode_path(player)).ok()?;
    let line = file.iter().position(|b| *b == b'\n')?;
    let player_len = std::fs::metadata(player).ok()?.len();
    if file[..line] != *bytecode_header(player_len, &file[line + 1..]).as_bytes() {
        return None;
    }
    file.drain(..=line);
    Some(file)
}

/// Whether the player file `player` has its bytecode beside it, from this build.
pub(crate) fn has_bytecode(player: &Path) -> bool {
    bytecode_file(player).is_some()
}

/// The player's compiled wrapper from its bytecode file (`bytecode_file`).
fn read_bytecode<'js>(ctx: &Ctx<'js>, player: &Path) -> Option<Value<'js>> {
    from_bytecode(ctx, &bytecode_file(player)?).ok()
}

/// Numbers the temporary files, so two runs saving one player's bytecode at once don't
/// write the same one.
static SAVES: AtomicU64 = AtomicU64::new(0);

/// A player compiled from its source, its bytecode not saved yet (`Unsaved::save`).
struct Unsaved<'js> {
    /// The player's wrapper, compiled and not run.
    code: Value<'js>,
    /// The player file's length, for the bytecode's header.
    player_len: u64,
}

impl<'js> Unsaved<'js> {
    /// Saves the bytecode beside the player file `player`: stripped, if a copy loaded from
    /// that answers the test challenges as `loaded` does, which came from the source; else
    /// whole. Nothing when `watch` stopped the check (it says nothing about the player). A
    /// failure is logged, and the next run compiles again.
    fn save(&self, ctx: &Ctx<'js>, loaded: &Loaded<'js>, player: &Path, watch: &Watch) {
        save_bytecode(ctx, &self.code, loaded, player, self.player_len, watch);
    }
}

fn save_bytecode<'js>(
    ctx: &Ctx<'js>,
    code: &Value<'js>,
    loaded: &Loaded<'js>,
    player: &Path,
    player_len: u64,
    watch: &Watch,
) {
    let stripped = to_bytecode(ctx, code, STRIPPED).filter(|bytecode| {
        from_bytecode(ctx, bytecode)
            .and_then(|copy| Loaded::new(ctx, &copy))
            .is_ok_and(|copy| copy.test_answers(ctx) == loaded.test_answers(ctx))
    });
    if stripped.is_none() && watch.over() {
        log::warn!("a player's bytecode wasn't saved: its check was stopped");
        return;
    }
    let payload = match stripped {
        Some(bytecode) => bytecode,
        None => {
            log::warn!("a player answers otherwise without its source text: kept whole");
            match to_bytecode(ctx, code, WHOLE) {
                Some(bytecode) => bytecode,
                None => return,
            }
        }
    };
    let path = bytecode_path(player);
    let part = path.with_extension(format!(
        "{}-{}.part",
        std::process::id(),
        SAVES.fetch_add(1, Ordering::Relaxed)
    ));
    let saved = std::fs::File::create(&part)
        .and_then(|mut file| {
            file.write_all(bytecode_header(player_len, &payload).as_bytes())?;
            file.write_all(b"\n")?;
            file.write_all(&payload)
        })
        .and_then(|()| std::fs::rename(&part, &path));
    if let Err(e) = saved {
        let _ = std::fs::remove_file(&part);
        log::warn!("saving a player's bytecode: {e}");
    }
}

/// The challenges a player is checked on: EJS's when it preprocesses one, and a stripped
/// copy's against its source (`save_bytecode`).
pub(crate) const TEST_N: &str = "featherplayTest0";

fn test_sig() -> String {
    (0u8..100).map(|b| (b'0' + b % 75) as char).collect()
}

/// A loaded player: the object its code filled with the `n` and `sig` functions.
struct Loaded<'js> {
    fns: Object<'js>,
    /// What the player threw while it loaded, said with a missing function.
    threw: Option<String>,
}

/// Loads the preprocessed player at `player`: from its bytecode, else from its source, with
/// the bytecode to save once the challenges are answered. The player's code is the body of
/// a function taking `_result`, as EJS makes it.
fn load<'js>(ctx: &Ctx<'js>, player: &Path) -> Result<(Loaded<'js>, Option<Unsaved<'js>>), String> {
    if let Some(code) = read_bytecode(ctx, player) {
        return Ok((Loaded::new(ctx, &code)?, None));
    }
    let source = std::fs::read_to_string(player).map_err(|e| format!("reading the player: {e}"))?;
    let code = compile(ctx, &format!("(function(_result){{\n{source}\n}})"))?;
    let loaded = Loaded::new(ctx, &code)?;
    let player_len = source.len() as u64;
    Ok((loaded, Some(Unsaved { code, player_len })))
}

impl<'js> Loaded<'js> {
    /// Runs a player's compiled wrapper (`compile`, `from_bytecode`), then the player.
    fn new(ctx: &Ctx<'js>, code: &Value<'js>) -> Result<Self, String> {
        let wrapper = run(ctx, code)?
            .into_function()
            .ok_or("the player didn't compile to a function")?;
        let fns = Object::new(ctx.clone()).map_err(|e| thrown(ctx, &e))?;
        let threw = wrapper
            .call::<_, Value>((fns.clone(),))
            .err()
            .map(|e| thrown(ctx, &e));
        Ok(Self { fns, threw })
    }

    /// What the player answers to the test challenges, each kind apart: an error there
    /// is an answer too.
    fn test_answers(&self, ctx: &Ctx<'js>) -> [Result<Answers, String>; 2] {
        [
            self.answers(ctx, &[TEST_N.into()], &[]),
            self.answers(ctx, &[], &[test_sig()]),
        ]
    }

    fn answers(&self, ctx: &Ctx<'js>, n: &[String], sig: &[String]) -> Result<Answers, String> {
        let mut out = Answers::default();
        for (kind, challenges, answers) in [("n", n, &mut out.n), ("sig", sig, &mut out.sig)] {
            if challenges.is_empty() {
                continue;
            }
            let solve = self
                .fns
                .get::<_, Value>(kind)
                .ok()
                .and_then(Value::into_function)
                .ok_or_else(|| {
                    let why = self.threw.as_ref().map(|t| format!(": {t}"));
                    format!(
                        "the player has no {kind} function{}",
                        why.unwrap_or_default()
                    )
                })?;
            for challenge in challenges {
                let answer: Value = solve
                    .call((challenge.as_str(),))
                    .map_err(|e| format!("{kind}: {}", thrown(ctx, &e)))?;
                let answer = answer
                    .as_string()
                    .and_then(|s| s.to_string().ok())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("no {kind} answer for {challenge}"))?;
                answers.insert(challenge.clone(), answer);
            }
        }
        Ok(out)
    }
}

/// The `n` and `sig` challenges of one answer.
type Challenges = (Vec<String>, Vec<String>);

/// A thread loading a preprocessed player, waiting for its challenges. A resolve starts it
/// before its request, so the player is loaded by the time the answer is in. Stopped if
/// dropped before it answers.
pub(crate) struct Solver {
    ask: mpsc::Sender<Challenges>,
    answer: tokio::sync::oneshot::Receiver<Result<Answers, String>>,
    watch: Arc<Watch>,
}

impl Solver {
    pub(crate) fn start(player: &Path) -> std::io::Result<Self> {
        Self::with_limits(player, SOLVE)
    }

    fn with_limits(player: &Path, limits: Limits) -> std::io::Result<Self> {
        let (ask, asked) = mpsc::channel::<Challenges>();
        let (tell, answer) = tokio::sync::oneshot::channel();
        let watch = Watch::new(limits.time);
        let (timer, player) = (watch.clone(), player.to_path_buf());
        // Not joined: the thread ends once it answered, or once nobody waits.
        spawn(limits, watch.clone(), move |ctx| {
            let loaded = load(ctx, &player);
            // Nobody asking (the request failed): the bytecode is still worth saving.
            if let Ok((n, sig)) = asked.recv() {
                timer.allow(limits.time);
                let result = match &loaded {
                    Ok((player, _)) => player.answers(ctx, &n, &sig),
                    Err(e) => Err(e.clone()),
                };
                let result = result.map_err(|e| match timer.timed_out() {
                    true => "the solver timed out".to_string(),
                    false => e,
                });
                let _ = tell.send(result);
            }
            // After the answer: the song doesn't wait for the save (~0.1 s and a 3 MB write).
            if let Ok((loaded, Some(unsaved))) = &loaded {
                timer.go_on_alone();
                timer.allow(limits.time);
                unsaved.save(ctx, loaded, &player, &timer);
            }
        })?;
        Ok(Self { ask, answer, watch })
    }

    pub(crate) async fn solve(mut self, n: &[String], sig: &[String]) -> Result<Answers, String> {
        let stopped = || "the solver stopped".to_string();
        self.ask
            .send((n.to_vec(), sig.to_vec()))
            .map_err(|_| stopped())?;
        // The thread's own limit ends a run; this one is for a thread that never got to it.
        let limit = self.watch.left() + Duration::from_secs(20);
        match tokio::time::timeout(limit, &mut self.answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(stopped()),
            Err(_) => Err("the solver timed out".into()),
        }
    }
}

impl Drop for Solver {
    fn drop(&mut self) {
        self.watch.cancel();
    }
}

/// Loads the player at `player` once, answers a test challenge, and saves its bytecode if
/// this build's isn't there: the first song it answers then loads it in ~15 ms instead of
/// compiling 4 MB of JS (~0.2 s). Blocks until the bytecode is saved, on a thread of its own.
pub(crate) fn warm(player: &Path) -> Result<(), String> {
    let watch = Watch::new(SOLVE.time);
    let (timer, player) = (watch.clone(), player.to_path_buf());
    let run = spawn(SOLVE, watch, move |ctx| {
        let (loaded, unsaved) = load(ctx, &player)?;
        loaded.answers(ctx, &[TEST_N.into()], &[])?;
        if let Some(unsaved) = unsaved {
            timer.allow(SOLVE.time);
            unsaved.save(ctx, &loaded, &player, &timer);
        }
        Ok(())
    })
    .map_err(|e| format!("starting the QuickJS thread: {e}"))?;
    run.join()
        .map_err(|_| "QuickJS panicked".to_string())
        .and_then(|started| started)
        .and_then(|ran| ran)
}

/// The EJS library's source, inflated from what `build.rs` deflated.
fn ejs_lib() -> String {
    let mut js = String::new();
    flate2::read::DeflateDecoder::new(EJS_LIB_DEFLATED)
        .read_to_string(&mut js)
        .expect("build.rs deflates the EJS library from a UTF-8 file");
    js
}

/// Runs EJS on a player's JS once: the preprocessed player, which sets `_result.n` and
/// `_result.sig` when run as a function's body taking `_result`. EJS checks it on a test
/// challenge each. Blocks for seconds (3.8 s for a 3 MB player, 2026-11-09) on a thread of
/// its own, and peaks at ~220 MB: for a short-lived process (`crate::preprocess_stdio`).
pub(crate) fn preprocess(js: &str) -> Result<String, String> {
    let input = json!({
        "type": "player",
        "player": js,
        "requests": [
            { "type": "n", "challenges": [TEST_N] },
            { "type": "sig", "challenges": [test_sig()] },
        ],
        "output_preprocessed": true,
    })
    .to_string();
    let watch = Watch::new(PREPROCESS.time);
    let timer = watch.clone();
    let run = spawn(PREPROCESS, watch, move |ctx| {
        let failed = |e: rquickjs::Error| format!("EJS: {}", thrown(ctx, &e));
        let script = format!(
            "{}\nObject.assign(globalThis, lib);\n{EJS_CORE}\njsc",
            ejs_lib()
        );
        let jsc: Function = ctx.eval(script).map_err(failed)?;
        let input = ctx.json_parse(input).map_err(failed)?;
        let out: Object = jsc.call((input,)).map_err(failed)?;
        let responses: Vec<Object> = out.get("responses").map_err(failed)?;
        for response in responses {
            let kind: String = response.get("type").map_err(failed)?;
            if kind != "result" {
                let error: Option<String> = response.get("error").ok();
                return Err(format!("EJS: {}", error.as_deref().unwrap_or("error")));
            }
        }
        let player: Option<String> = out.get("preprocessed_player").map_err(failed)?;
        player
            .filter(|p| !p.is_empty())
            .ok_or_else(|| "EJS gave no preprocessed player".to_string())
    })
    .map_err(|e| format!("starting the QuickJS thread: {e}"))?;
    let result = run
        .join()
        .map_err(|_| "QuickJS panicked".to_string())
        .and_then(|started| started)
        .and_then(|ran| ran);
    result.map_err(|e| match timer.timed_out() {
        true => "preprocessing the player timed out".to_string(),
        false => e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stands in for a preprocessed player: `n` reversed, `sig` without its first two
    /// characters and reversed.
    const FAKE_PLAYER: &str = r#"_result.n = (n) => n.split("").reverse().join("");
_result.sig = (s) => s.slice(2).split("").reverse().join("");"#;

    /// A player file with this code, in a folder of its own.
    fn player(name: &str, code: &str) -> PathBuf {
        let base = std::env::var("FEATHERPLAY_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"));
        let dir = base
            .join("test-streams")
            .join(format!("js-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("player.js");
        std::fs::write(&path, code).unwrap();
        path
    }

    /// The bytecode, once the run that answered has saved it.
    fn saved_bytecode(player: &Path) -> Vec<u8> {
        for _ in 0..500 {
            if has_bytecode(player) {
                return std::fs::read(bytecode_path(player)).unwrap();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no bytecode saved beside {}", player.display());
    }

    /// Removes the player's folder, once a save still running beside the answer let go of it.
    fn done(player: &Path) {
        let dir = player.parent().unwrap();
        for _ in 0..100 {
            if std::fs::remove_dir_all(dir).is_ok() || !dir.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The player's answer to one `n` challenge.
    async fn n(player: &Path, challenge: &str) -> Result<String, String> {
        let solver = Solver::start(player).unwrap();
        let mut answers = solver.solve(&[challenge.into()], &[]).await?;
        Ok(answers.n.remove(challenge).unwrap())
    }

    /// Preprocessing a real player (`MEASURE_BASE`, as YouTube serves it), 3 times.
    #[test]
    #[ignore = "a measurement: set MEASURE_BASE"]
    fn measure_preprocess() {
        let js = std::fs::read_to_string(std::env::var("MEASURE_BASE").unwrap()).unwrap();
        for _ in 0..3 {
            let t = Instant::now();
            let out = preprocess(&js).unwrap();
            let took = t.elapsed().as_secs_f64();
            println!("preprocessed in {took:.2} s: {} B", out.len());
        }
    }

    #[test]
    fn the_ejs_library_inflates_to_its_source() {
        assert_eq!(ejs_lib(), include_str!("../ejs/yt.solver.lib.min.js"));
    }

    #[tokio::test]
    async fn the_solver_answers_from_the_source_then_from_its_bytecode() {
        let p = player("solver", FAKE_PLAYER);
        let solver = Solver::start(&p).unwrap();
        let a = solver
            .solve(&["abc".into()], &["xy123".into()])
            .await
            .unwrap();
        assert_eq!(a.n["abc"], "cba");
        assert_eq!(a.sig["xy123"], "321");
        saved_bytecode(&p);
        // The source now says otherwise (in as many bytes, so the bytecode is still its
        // own): the answer comes from the bytecode.
        std::fs::write(&p, FAKE_PLAYER.replace("reverse", "REVERSE")).unwrap();
        assert_eq!(n(&p, "abc").await.unwrap(), "cba");
        done(&p);
    }

    #[tokio::test]
    async fn bytecode_that_is_damaged_or_another_builds_is_made_again() {
        let p = player("stale", FAKE_PLAYER);
        assert_eq!(n(&p, "abc").await.unwrap(), "cba");
        let bytecode = bytecode_path(&p);
        let good = saved_bytecode(&p);
        let payload = &good[good.iter().position(|b| *b == b'\n').unwrap() + 1..];
        let mut damaged = good.clone();
        *damaged.last_mut().unwrap() ^= 0xff;
        let other_build = [b"featherplay bytecode 0.0.0\n", payload].concat();
        for bad in [damaged, other_build, Vec::new()] {
            std::fs::write(&bytecode, &bad).unwrap();
            assert_eq!(n(&p, "abc").await.unwrap(), "cba");
            assert_eq!(saved_bytecode(&p), good, "made again");
        }
        done(&p);
    }

    #[tokio::test]
    async fn a_player_without_the_functions_says_so() {
        let p = player("no-functions", "_result.x = 1; throw new Error('boom');");
        let e = n(&p, "abc").await.unwrap_err();
        assert!(e.contains("no n function") && e.contains("boom"), "{e}");
        done(&p);
    }

    /// The player is YouTube's code: it gets the language, and nothing of the machine.
    #[tokio::test]
    async fn a_player_sees_no_host_functions() {
        let host = "Deno,process,require,fetch,XMLHttpRequest,WebSocket,Worker,std,os,bjson,\
                    console,print,scriptArgs,navigator,setTimeout,setInterval,importScripts,\
                    WebAssembly,localStorage";
        let p = player(
            "sandbox",
            r#"_result.n = (names) => names.split(",")
                .filter((name) => typeof globalThis[name] !== "undefined").join(",") || "none";"#,
        );
        assert_eq!(n(&p, host).await.unwrap(), "none");
        done(&p);
    }

    /// Without their source text, functions print as native ones. A player that looks at
    /// that keeps its bytecode whole, and answers from it as it does from its source.
    #[tokio::test]
    async fn a_player_that_reads_its_own_text_keeps_it_in_its_bytecode() {
        let reads_itself = "_result.n = (n) => n + (function f() { return 1; }).toString().length;";
        let p = player("own-text", reads_itself);
        let from_source = n(&p, "abc").await.unwrap();
        saved_bytecode(&p);
        assert_eq!(from_source, "abc26");
        assert_eq!(
            n(&p, "abc").await.unwrap(),
            from_source,
            "from its bytecode"
        );
        done(&p);
    }

    /// A player that never ends, recurses without end or fills the memory is stopped with
    /// an error, and the app goes on.
    #[tokio::test]
    async fn a_run_past_its_limits_is_stopped() {
        let limits = Limits {
            memory: 32 << 20,
            time: Duration::from_secs(1),
            ..SOLVE
        };
        for (code, said) in [
            ("_result.n = () => { for (;;) {} };", "timed out"),
            ("_result.n = function f(x) { return f(x) + 1; };", "stack"),
            (
                "_result.n = () => { const all = []; for (;;) all.push(new Array(1e5).fill(1)); };",
                "memory",
            ),
        ] {
            let p = player("limits", code);
            let solver = Solver::with_limits(&p, limits).unwrap();
            let e = solver.solve(&["abc".into()], &[]).await.unwrap_err();
            assert!(e.to_lowercase().contains(said), "{code}: {e}");
            done(&p);
        }
    }
}
