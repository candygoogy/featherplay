//! The session (`config/session.json` and `config/session-position.json`) and the settings
//! (`config/settings.toml`), written off the controller's loop.
//!
//! The loop only copies what to save (`Player::session`, the settings); the serialising, the
//! atomic write and its fsync happen on the saves' thread (`task::write_in_order`). One write
//! of a file runs at a time, and each takes the newest copy waiting when it starts, so copies
//! given while one waits or writes come down to one write of the last (an equalizer band
//! dragged gives a change per step): an older copy never lands after a newer one. At quit,
//! and for a setting another reader of the file needs at once, the last copy is written on
//! the caller's thread, after any write running, and a write still queued then does nothing.
//!
//! The session is copied and written whole (the queue: 2 MB for 5,000 tracks) only when the
//! queue changed since the last whole save (`Queue::revision`), after a failed write, and at
//! quit. In plain playback the loop copies only the position (`SessionPosition`), and it
//! goes to the small file with the last whole save's random tag, so it's read over
//! `session.json` only while that holds the same queue.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use featherplay_core::model::{Session, SessionPosition};
use featherplay_core::player::Player;
use featherplay_core::sync::LockExt;
use featherplay_store::{Paths, Settings, save_session, save_session_position};

/// One file written in the background, newest copy first (see the module docs).
pub(super) struct Later<T>(Arc<Shared<T>>);

pub(super) type SessionFile = Later<SessionCopy>;
pub(super) type SettingsFile = Later<Settings>;

/// A session to save: whole, or only its position while the queue is the one last saved
/// whole. Each with the tag of that whole save.
pub(super) enum SessionCopy {
    Whole(Session, u64),
    Position(SessionPosition, u64),
}

struct Shared<T> {
    paths: Paths,
    /// Writes a copy and logs how that went; `false` when it failed.
    write: fn(&T, &Paths) -> bool,
    /// What to write instead of the copy waiting and a newer one given.
    fold: fn(T, T) -> T,
    /// The newest copy not written yet.
    waiting: Mutex<Option<T>>,
    /// Held for each write, so two never overlap.
    writing: Mutex<()>,
    /// The last write failed.
    failed: AtomicBool,
}

impl<T> Clone for Later<T> {
    fn clone(&self) -> Self {
        Later(self.0.clone())
    }
}

impl SessionFile {
    pub fn session(paths: Paths) -> SessionFile {
        Later::new(paths, write_session, |older, newer| match (older, newer) {
            // A whole session still waiting takes the position of the same queue.
            (SessionCopy::Whole(mut session, tag), SessionCopy::Position(p, of)) if of == tag => {
                session.set_position(p);
                SessionCopy::Whole(session, tag)
            }
            (_, newer) => newer,
        })
    }
}

impl SettingsFile {
    pub fn settings(paths: Paths) -> SettingsFile {
        Later::new(
            paths,
            |settings, paths| match settings.save(paths) {
                Ok(()) => true,
                Err(e) => {
                    log!("saving the settings failed: {}", e.kind());
                    false
                }
            },
            |_, newer| newer,
        )
    }
}

impl<T: Send + 'static> Later<T> {
    fn new(paths: Paths, write: fn(&T, &Paths) -> bool, fold: fn(T, T) -> T) -> Later<T> {
        Later(Arc::new(Shared {
            paths,
            write,
            fold,
            waiting: Mutex::new(None),
            writing: Mutex::new(()),
            failed: AtomicBool::new(false),
        }))
    }

    /// Saves `copy` in the background.
    pub fn save_later(&self, copy: T) {
        if let Some(write) = self.queue(copy) {
            crate::task::write_in_order(write);
        }
    }

    /// Makes `copy` the one to write next, folded into an older one still waiting. Returns
    /// the write to start when none is queued to take it.
    fn queue(&self, copy: T) -> Option<impl FnOnce() + Send + 'static> {
        let queued = {
            let mut waiting = self.0.waiting.lock_or_recover();
            let older = waiting.take();
            let queued = older.is_some();
            *waiting = Some(match older {
                Some(older) => (self.0.fold)(older, copy),
                None => copy,
            });
            queued
        };
        let me = self.clone();
        (!queued).then_some(move || me.write_waiting())
    }

    fn write_waiting(&self) {
        let _writing = self.0.writing.lock_or_recover();
        // Taken once this write's turn has come, so it's the newest.
        let Some(copy) = self.0.waiting.lock_or_recover().take() else {
            return;
        };
        self.write(&copy);
    }

    fn write(&self, copy: &T) {
        let ok = (self.0.write)(copy, &self.0.paths);
        self.0.failed.store(!ok, Ordering::Relaxed);
    }

    /// Writes `copy` now, on this thread: after the write running, if any. A copy still
    /// waiting is older, so it's dropped.
    pub fn save_now(&self, copy: &T) {
        let _writing = self.0.writing.lock_or_recover();
        self.0.waiting.lock_or_recover().take();
        self.write(copy);
    }

    /// Writes the copy still waiting, if any, now (at quit).
    pub fn flush(&self) {
        self.write_waiting();
    }

    /// The last write failed.
    fn failed(&self) -> bool {
        self.0.failed.load(Ordering::Relaxed)
    }
}

/// What each session save copies (see the module docs).
pub(super) struct SessionSaves {
    file: SessionFile,
    /// The queue revision (`Queue::revision`) of the last whole session given to write, or
    /// of the one restored, and its tag.
    saved: Option<(u64, u64)>,
    /// The tag of the `session.json` read at startup, until its session is restored.
    restoring: Option<u64>,
}

impl SessionSaves {
    pub fn new(paths: Paths, restoring: Option<u64>) -> SessionSaves {
        SessionSaves {
            file: SessionFile::session(paths),
            saved: None,
            restoring,
        }
    }

    /// The session read at startup was just restored: its queue is the one on disk.
    pub fn restored(&mut self, player: &Player) {
        if let Some(tag) = self.restoring.take() {
            self.saved = Some((player.queue().revision(), tag));
        }
    }

    /// Saves the session in the background: this loop only copies it, the queue only when
    /// it changed.
    pub fn save_later(&mut self, player: &Player) {
        let copy = self.copy(player);
        self.file.save_later(copy);
    }

    /// Saves the whole session before returning, after any save on its way (at quit).
    pub fn save_now(&mut self, player: &Player) {
        let copy = self.whole(player);
        self.file.save_now(&copy);
    }

    fn copy(&mut self, player: &Player) -> SessionCopy {
        match self.saved {
            Some((revision, tag))
                if revision == player.queue().revision() && !self.file.failed() =>
            {
                SessionCopy::Position(player.session_position(), tag)
            }
            _ => self.whole(player),
        }
    }

    fn whole(&mut self, player: &Player) -> SessionCopy {
        // Random, so no other queue's position (another run's) carries it.
        let tag = RandomState::new().hash_one(SystemTime::now());
        self.saved = Some((player.queue().revision(), tag));
        SessionCopy::Whole(player.session(), tag)
    }
}

/// A whole session goes to `session.json`, then its position to the small file (which may
/// hold an older queue's); a position to the small file alone.
fn write_session(copy: &SessionCopy, paths: &Paths) -> bool {
    let result = match copy {
        SessionCopy::Whole(session, tag) => save_session(session, *tag, paths)
            .and_then(|()| save_session_position(&session.position(), *tag, paths)),
        SessionCopy::Position(position, tag) => save_session_position(position, *tag, paths),
    };
    report(copy, &result);
    result.is_ok()
}

fn report(copy: &SessionCopy, result: &io::Result<()>) {
    match (copy, result) {
        (SessionCopy::Whole(session, _), Ok(())) => log!(
            "session saved: {} tracks, index {}, at {:.1}s",
            session.queue.len(),
            session.index,
            session.position_ms as f64 / 1000.0
        ),
        (SessionCopy::Position(p, _), Ok(())) => log!(
            "session position saved: index {}, at {:.1}s",
            p.index,
            p.position_ms as f64 / 1000.0
        ),
        (_, Err(e)) => log!("saving the session failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    use featherplay_core::model::{LikeStatus, RepeatMode, Session, Track, TrackId, VideoType};
    use featherplay_core::player::{Input, PlaybackPrefs, Player};
    use featherplay_store::{Paths, Settings, load_session, save_session};

    use super::{SessionCopy, SessionFile, SessionSaves, SettingsFile};

    fn session(position_ms: u64) -> SessionCopy {
        let session = Session {
            queue: Vec::new(),
            index: 0,
            position_ms,
            radio: false,
            shuffle: false,
            repeat: RepeatMode::Off,
        };
        SessionCopy::Whole(session, 1)
    }

    fn track(id: &str) -> Track {
        Track {
            id: TrackId(id.into()),
            title: id.into(),
            artists: Vec::new(),
            album: None,
            duration: None,
            video_type: VideoType::Song,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            counterpart: None,
        }
    }

    fn player() -> Player {
        Player::new(PlaybackPrefs {
            prefer_song_versions: false,
            skip_non_music_in_radio: true,
            scrobble_other_media: true,
            list_end: Default::default(),
            skip_disliked: true,
            crossfade: None,
            fade_on_skip: false,
            speed_percent: 100,
        })
    }

    /// A fresh home of its own (`crate::test_home`); `None` (the test skips) without
    /// `FEATHERPLAY_HOME`.
    fn home(name: &str) -> Option<(PathBuf, Paths)> {
        let root = crate::test_home(&format!("session-file-{name}"))?;
        let paths = Paths::from_home(&root);
        Some((root, paths))
    }

    fn saved_at(paths: &Paths) -> Option<u64> {
        load_session(paths).map(|s| s.session.position_ms)
    }

    /// Writes what `saves` copies from `player` now, on this thread.
    fn save(saves: &mut SessionSaves, player: &Player) {
        let copy = saves.copy(player);
        saves.file.queue(copy).expect("nothing waits")();
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

    /// Playback over a restored queue writes only the position file, read back over
    /// `session.json`; a queue edit writes `session.json` again, and the playback after it
    /// the small file only.
    #[test]
    fn playback_writes_the_position_alone_and_a_queue_edit_the_whole_session() {
        let Some((root, paths)) = home("position") else {
            return;
        };
        let restored = Session {
            queue: vec![track("a"), track("b")],
            index: 1,
            position_ms: 5_000,
            radio: false,
            shuffle: false,
            repeat: RepeatMode::Off,
        };
        save_session(&restored, 7, &paths).unwrap();
        let whole = paths.config.join("session.json");
        let on_disk = fs::read(&whole).unwrap();
        let loaded = load_session(&paths).unwrap();
        let mut player = player();
        let mut saves = SessionSaves::new(paths.clone(), loaded.queue_tag);
        player.handle(Input::Restore(loaded.session));
        saves.restored(&player);

        player.handle(Input::SeekTo(Duration::from_secs(20)));
        save(&mut saves, &player);
        assert_eq!(
            fs::read(&whole).unwrap(),
            on_disk,
            "session.json written again"
        );
        assert_eq!(saved_at(&paths), Some(20_000));

        player.handle(Input::Enqueue(vec![track("c")]));
        save(&mut saves, &player);
        let edited = load_session(&paths).unwrap();
        assert_eq!(edited.session.queue.len(), 3);
        assert_ne!(edited.queue_tag, Some(7));

        let on_disk = fs::read(&whole).unwrap();
        player.handle(Input::SeekTo(Duration::from_secs(30)));
        save(&mut saves, &player);
        assert_eq!(fs::read(&whole).unwrap(), on_disk);
        assert_eq!(saved_at(&paths), Some(30_000));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Quit writes `session.json` whole, with the position 0.7.0 reads from it, and the
    /// position file; with nothing changed in the queue since the last whole save too.
    #[test]
    fn quit_writes_both_files() {
        let Some((root, paths)) = home("quit-both") else {
            return;
        };
        let mut player = player();
        player.handle(Input::Enqueue(vec![track("a"), track("b")]));
        let mut saves = SessionSaves::new(paths.clone(), None);
        save(&mut saves, &player);
        player.handle(Input::SeekTo(Duration::from_secs(42)));
        let whole = paths.config.join("session.json");
        let position = paths.config.join("session-position.json");
        fs::remove_file(&whole).unwrap();
        fs::remove_file(&position).unwrap();

        saves.save_now(&player);
        let alone: Session = serde_json::from_slice(&fs::read(&whole).unwrap()).unwrap();
        assert_eq!(alone.position_ms, 42_000, "session.json's own position");
        assert!(position.exists());
        assert_eq!(saved_at(&paths), Some(42_000));
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
