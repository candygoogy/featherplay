//! One featherplay at a time. It hides in the notification area, so starting it again from
//! the Start menu would otherwise run a second copy: two tray icons, two sets of hotkeys,
//! two writers of the queue and settings, double scrobbles. `start` runs first in a windowed
//! run, before any window, tray icon, hotkey or store: it claims the home's guard, and if
//! another featherplay holds it, asks that one to show its window and exits. How the guard
//! and the asking work is `featherplay_platform::instance`'s.
//!
//! One guard per home: a run with `FEATHERPLAY_HOME` set derives its names from that home,
//! so a dev build and the installed app don't stop each other, and tests don't meet the
//! user's app.

use std::time::{Duration, Instant};

use featherplay_platform::instance;
pub use featherplay_platform::instance::{
    Claim, Guard, Running, claim, key, mutex_name, show_message,
};
pub use featherplay_platform::session_end::Ask;

/// How long a second launch keeps trying: the first may still be starting (its end-session
/// window not made yet) or closing (saving its session).
const HANDOFF: Duration = Duration::from_secs(10);
/// How long it waits for the first to let go between two asks.
const RETRY: Duration = Duration::from_millis(250);

/// The key for this run's environment.
pub fn key_here() -> String {
    key(std::env::var_os("FEATHERPLAY_HOME").as_deref())
}

/// Asks the featherplay of this key to show its window. True once one answered that it
/// did; false if none did (none is running, it is still starting, or it is closing).
pub fn ask_to_show(key: &str) -> bool {
    ask(key, Ask::Show)
}

/// Asks the featherplay of this key for `what`, as `ask_to_show` does.
pub fn ask(key: &str, what: Ask) -> bool {
    instance::ask(key, what).unwrap_or_else(|e| {
        log!("single instance: {e}");
        false
    })
}

/// What `main` does after `start`.
pub enum Start {
    /// The first featherplay of this home: run, keeping the guard (if the mutex could be
    /// made at all) on the main thread until the end.
    Run(Option<Guard>),
    /// Another one showed its window: exit without creating anything.
    Exit,
}

/// Call on the main thread, first thing in a windowed run. Another featherplay of this home
/// is asked for `what` (its window; with `--quick-search`, the quick search box). With none
/// running, this one starts as usual.
pub fn start(what: Ask) -> Start {
    let key = key_here();
    let mut running = match claim(&mutex_name(&key)) {
        Ok(Claim::First(guard)) => return Start::Run(Some(guard)),
        Ok(Claim::Running(running)) => running,
        Err(e) => {
            log!("single instance: {e}; starting anyway");
            return Start::Run(None);
        }
    };
    let deadline = Instant::now() + HANDOFF;
    loop {
        if ask(&key, what) {
            log!("featherplay is already running; it answered ({what:?})");
            return Start::Exit;
        }
        // Starting (no window yet) or closing (it said no): if it ends, this run takes over.
        running = match running.wait(RETRY) {
            Ok(guard) => return Start::Run(Some(guard)),
            Err(running) => running,
        };
        if Instant::now() >= deadline {
            log!("featherplay is already running but didn't answer; not starting another");
            return Start::Exit;
        }
    }
}
