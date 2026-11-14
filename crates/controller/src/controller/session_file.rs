//! The session file (`config/session.json`), written off the controller's loop.
//!
//! The loop only copies the session (`Player::session`); the JSON, the atomic write and its
//! fsync happen on the saves' thread (`task::write_in_order`). One write runs at a time, and
//! each takes the newest session waiting when it starts, so sessions given while one waits
//! or writes come down to one write of the last: an older session never lands after a newer
//! one. At quit the last session is written on the caller's thread, after any write running,
//! and a write still queued then does nothing.

use std::io;
use std::sync::{Arc, Mutex};

use featherplay_core::model::Session;
use featherplay_core::sync::LockExt;
use featherplay_store::{Paths, save_session};

#[derive(Clone)]
pub(super) struct SessionFile(Arc<Shared>);

struct Shared {
    paths: Paths,
    /// The newest session not written yet.
    waiting: Mutex<Option<Session>>,
    /// Held for each write, so two never overlap.
    writing: Mutex<()>,
}

impl SessionFile {
    pub fn new(paths: Paths) -> SessionFile {
        SessionFile(Arc::new(Shared {
            paths,
            waiting: Mutex::new(None),
            writing: Mutex::new(()),
        }))
    }

    /// Saves `session` in the background.
    pub fn save_later(&self, session: Session) {
        if let Some(write) = self.queue(session) {
            crate::task::write_in_order(write);
        }
    }

    /// Makes `session` the one to write next, replacing an older one still waiting. Returns
    /// the write to start when none is queued to take it.
    fn queue(&self, session: Session) -> Option<impl FnOnce() + Send + 'static> {
        let queued = self.0.waiting.lock_or_recover().replace(session).is_some();
        let me = self.clone();
        (!queued).then_some(move || me.write_waiting())
    }

    fn write_waiting(&self) {
        let _writing = self.0.writing.lock_or_recover();
        // Taken once this write's turn has come, so it's the newest.
        let Some(session) = self.0.waiting.lock_or_recover().take() else {
            return;
        };
        report(&session, save_session(&session, &self.0.paths));
    }

    /// Writes `session` now, on this thread (at quit): after the write running, if any. A
    /// session still waiting is older, so it's dropped.
    pub fn save_now(&self, session: &Session) {
        let _writing = self.0.writing.lock_or_recover();
        self.0.waiting.lock_or_recover().take();
        report(session, save_session(session, &self.0.paths));
    }
}

fn report(session: &Session, result: io::Result<()>) {
    match result {
        Ok(()) => log!(
            "session saved: {} tracks, index {}, at {:.1}s",
            session.queue.len(),
            session.index,
            session.position_ms as f64 / 1000.0
        ),
        Err(e) => log!("saving the session failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use featherplay_core::model::{RepeatMode, Session};
    use featherplay_store::{Paths, load_session};

    use super::SessionFile;

    fn session(position_ms: u64) -> Session {
        Session {
            queue: Vec::new(),
            index: 0,
            position_ms,
            radio: false,
            shuffle: false,
            repeat: RepeatMode::Off,
        }
    }

    /// A fresh folder of its own under `FEATHERPLAY_HOME`; `None` (the test skips) when
    /// it isn't set, so nothing is written elsewhere.
    fn home(name: &str) -> Option<(PathBuf, Paths)> {
        let base = std::env::var_os("FEATHERPLAY_HOME")?;
        let root =
            PathBuf::from(base).join(format!("test-session-file-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::from_home(&root);
        Some((root, paths))
    }

    fn saved_at(paths: &Paths) -> Option<u64> {
        load_session(paths).map(|s| s.position_ms)
    }

    #[test]
    fn sessions_given_while_one_waits_come_down_to_one_write_of_the_last() {
        let Some((root, paths)) = home("latest") else {
            return;
        };
        let file = SessionFile::new(paths.clone());
        let write = file.queue(session(1)).expect("the first starts a write");
        assert!(file.queue(session(2)).is_none());
        assert!(file.queue(session(3)).is_none());
        write();
        assert_eq!(saved_at(&paths), Some(3));
        // Nothing waits now: the next session starts a write of its own.
        assert!(file.queue(session(4)).is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_session_written_at_quit_stays_when_an_older_write_runs_after() {
        let Some((root, paths)) = home("quit") else {
            return;
        };
        let file = SessionFile::new(paths.clone());
        let late = file.queue(session(1)).expect("starts a write");
        file.save_now(&session(2));
        assert_eq!(saved_at(&paths), Some(2), "written before save_now returns");
        late();
        assert_eq!(saved_at(&paths), Some(2));
        let _ = std::fs::remove_dir_all(root);
    }
}
