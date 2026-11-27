//! The session (`config/session.json`) and the settings (`config/settings.toml`), written
//! off the controller's loop.
//!
//! The loop only copies what to save (`Player::session`, the settings); the serialising, the
//! atomic write and its fsync happen on the saves' thread (`task::write_in_order`). One write
//! of a file runs at a time, and each takes the newest copy waiting when it starts, so copies
//! given while one waits or writes come down to one write of the last (an equalizer band
//! dragged gives a change per step): an older copy never lands after a newer one. At quit,
//! and for a setting another reader of the file needs at once, the last copy is written on
//! the caller's thread, after any write running, and a write still queued then does nothing.

use std::io;
use std::sync::{Arc, Mutex};

use featherplay_core::model::Session;
use featherplay_core::sync::LockExt;
use featherplay_store::{Paths, Settings, save_session};

/// One file written in the background, newest copy first (see the module docs).
pub(super) struct Later<T>(Arc<Shared<T>>);

pub(super) type SessionFile = Later<Session>;
pub(super) type SettingsFile = Later<Settings>;

struct Shared<T> {
    paths: Paths,
    /// Writes a copy and logs how that went.
    write: fn(&T, &Paths),
    /// The newest copy not written yet.
    waiting: Mutex<Option<T>>,
    /// Held for each write, so two never overlap.
    writing: Mutex<()>,
}

impl<T> Clone for Later<T> {
    fn clone(&self) -> Self {
        Later(self.0.clone())
    }
}

impl SessionFile {
    pub fn session(paths: Paths) -> SessionFile {
        Later::new(paths, |session, paths| {
            report(session, save_session(session, paths));
        })
    }
}

impl SettingsFile {
    pub fn settings(paths: Paths) -> SettingsFile {
        Later::new(paths, |settings, paths| {
            if let Err(e) = settings.save(paths) {
                log!("saving the settings failed: {}", e.kind());
            }
        })
    }
}

impl<T: Send + 'static> Later<T> {
    fn new(paths: Paths, write: fn(&T, &Paths)) -> Later<T> {
        Later(Arc::new(Shared {
            paths,
            write,
            waiting: Mutex::new(None),
            writing: Mutex::new(()),
        }))
    }

    /// Saves `copy` in the background.
    pub fn save_later(&self, copy: T) {
        if let Some(write) = self.queue(copy) {
            crate::task::write_in_order(write);
        }
    }

    /// Makes `copy` the one to write next, replacing an older one still waiting. Returns
    /// the write to start when none is queued to take it.
    fn queue(&self, copy: T) -> Option<impl FnOnce() + Send + 'static> {
        let queued = self.0.waiting.lock_or_recover().replace(copy).is_some();
        let me = self.clone();
        (!queued).then_some(move || me.write_waiting())
    }

    fn write_waiting(&self) {
        let _writing = self.0.writing.lock_or_recover();
        // Taken once this write's turn has come, so it's the newest.
        let Some(copy) = self.0.waiting.lock_or_recover().take() else {
            return;
        };
        (self.0.write)(&copy, &self.0.paths);
    }

    /// Writes `copy` now, on this thread: after the write running, if any. A copy still
    /// waiting is older, so it's dropped.
    pub fn save_now(&self, copy: &T) {
        let _writing = self.0.writing.lock_or_recover();
        self.0.waiting.lock_or_recover().take();
        (self.0.write)(copy, &self.0.paths);
    }

    /// Writes the copy still waiting, if any, now (at quit).
    pub fn flush(&self) {
        self.write_waiting();
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
    use featherplay_store::{Paths, Settings, load_session};

    use super::{SessionFile, SettingsFile};

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

    /// A fresh home of its own (`crate::test_home`); `None` (the test skips) without
    /// `FEATHERPLAY_HOME`.
    fn home(name: &str) -> Option<(PathBuf, Paths)> {
        let root = crate::test_home(&format!("session-file-{name}"))?;
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
        let file = SessionFile::session(paths.clone());
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
        let file = SessionFile::session(paths.clone());
        let late = file.queue(session(1)).expect("starts a write");
        file.save_now(&session(2));
        assert_eq!(saved_at(&paths), Some(2), "written before save_now returns");
        late();
        assert_eq!(saved_at(&paths), Some(2));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Settings changed in a burst (a band dragged) are written once, the last; one still
    /// waiting at quit is written then.
    #[test]
    fn a_burst_of_settings_comes_down_to_the_last_and_quit_writes_what_waits() {
        let Some((root, paths)) = home("settings") else {
            return;
        };
        let file = SettingsFile::settings(paths.clone());
        let volume = |v: f32| Settings {
            volume: v,
            ..Settings::default()
        };
        let write = file.queue(volume(0.1)).expect("the first starts a write");
        assert!(file.queue(volume(0.2)).is_none());
        assert!(file.queue(volume(0.3)).is_none());
        write();
        assert_eq!(Settings::load(&paths).volume, 0.3);
        assert!(file.queue(volume(0.4)).is_some(), "a write of its own");
        file.flush();
        assert_eq!(Settings::load(&paths).volume, 0.4, "written at quit");
        let _ = std::fs::remove_dir_all(root);
    }
}
