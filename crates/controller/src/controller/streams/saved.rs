//! The streams outlive a restart (`SAVED_STREAMS`, written when the session is saved and
//! they changed, and at quit): the next session uses one until it nears its expiry, unless
//! a resolve shows another address. The restored track's is buffered at the start, so Play
//! starts it with no resolve. Signing out removes the file.

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use featherplay_core::model::{Track, TrackId};
use featherplay_core::player::Status;
use featherplay_core::sync::LockExt;
use featherplay_streams::{ResolvedStream, Via};
use serde_json::{Value, json};

use super::{Controller, FRESH_FOR, KEEP, Streams, address};

/// The streams kept for the next session, in the cache dir (`Streams::saved`).
pub(crate) const SAVED_STREAMS: &str = "streams.json";

/// Writes `SAVED_STREAMS` one at a time (in the background, and at quit on the controller's
/// thread); an older write that comes after a newer one does nothing. Holds the number of
/// the last one done.
#[derive(Clone, Default)]
pub(crate) struct SavedFile(Arc<Mutex<u64>>);

impl SavedFile {
    /// Writes `text` to `path`, or removes the file for `None`, unless a write numbered
    /// `n` or later was done already.
    pub fn write(&self, path: &Path, n: u64, text: Option<&str>) -> io::Result<()> {
        let mut done = self.0.lock_or_recover();
        if n <= *done {
            return Ok(());
        }
        *done = n;
        match text {
            Some(text) => featherplay_store::write_atomic(path, text.as_bytes()),
            None => match std::fs::remove_file(path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                gone => gone,
            },
        }
    }
}

impl Streams {
    /// Whether a stream came or went since the last call.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// The writer of `SAVED_STREAMS` and the number of a new write, for the streams as
    /// they are now.
    pub fn next_write(&mut self) -> (SavedFile, u64) {
        self.writes += 1;
        (self.file.clone(), self.writes)
    }

    /// Signing out: drops every stream kept; returns the write that removes the file.
    pub fn forget_all(&mut self) -> (SavedFile, u64) {
        self.signed_out += 1;
        self.cache.clear();
        self.restored.clear();
        self.unsaved.clear();
        self.changed = false;
        self.next_write()
    }

    /// Whether any stream kept is one to save (`saved`).
    pub fn keeps_any(&self) -> bool {
        self.cache.iter().any(|(id, _)| !self.unsaved.contains(id))
    }

    /// The streams that stay valid for `FRESH_FOR` from `now`, for `restore` in the next
    /// session (`SAVED_STREAMS`). Not those for another address than this session's, nor
    /// those resolved from before a sign-out (`unsaved`).
    pub fn saved(&self, now: SystemTime) -> String {
        let secs = |t: SystemTime| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
        let ms = |d: Duration| u64::try_from(d.as_millis()).ok();
        let streams: Vec<Value> = self
            .cache
            .iter()
            .filter(|(id, s)| {
                !self.unsaved.contains(id)
                    && s.expires_at > now + FRESH_FOR
                    && self
                        .address
                        .as_deref()
                        .is_none_or(|ip| address(&s.url) == Some(ip))
            })
            .map(|(id, s)| {
                json!({
                    "id": id.0,
                    "url": s.url,
                    "mime": s.mime,
                    "itag": s.itag,
                    "kbps": s.bitrate_kbps,
                    "length": s.content_length,
                    "expires": secs(s.expires_at),
                    "duration_ms": s.duration.and_then(ms),
                    "available_at": s.available_at.and_then(secs),
                    "title": s.video_title,
                    "tag": s.first_tag,
                    "loudness_db": s.loudness_db,
                })
            })
            .collect();
        json!({ "streams": streams }).to_string()
    }

    /// Keeps the streams `saved` gave in the last session that stay valid for `FRESH_FOR`
    /// from `now`, up to `KEEP`. They answer until a resolve shows that this session's URLs
    /// are for another address (their `ip=`): the network may have changed since.
    pub fn restore(&mut self, text: &str, now: SystemTime) {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let at = |v: &Value| v.as_u64().and_then(featherplay_streams::unix_secs);
        let text = |v: &Value| v.as_str().map(str::to_string);
        for s in v["streams"].as_array().into_iter().flatten() {
            let (Some(id), Some(url), Some(expires_at)) =
                (text(&s["id"]), text(&s["url"]), at(&s["expires"]))
            else {
                continue;
            };
            if expires_at <= now + FRESH_FOR || address(&url).is_none() || self.cache.len() >= KEEP
            {
                continue;
            }
            let stream = ResolvedStream {
                url,
                mime: text(&s["mime"]).unwrap_or_default(),
                itag: s["itag"]
                    .as_u64()
                    .and_then(|i| u32::try_from(i).ok())
                    .unwrap_or(0),
                bitrate_kbps: s["kbps"]
                    .as_u64()
                    .and_then(|k| u32::try_from(k).ok())
                    .unwrap_or(0),
                content_length: s["length"].as_u64(),
                expires_at,
                duration: s["duration_ms"].as_u64().map(Duration::from_millis),
                available_at: at(&s["available_at"]),
                video_title: text(&s["title"]),
                first_tag: text(&s["tag"]),
                loudness_db: s["loudness_db"].as_f64().map(|db| db as f32),
                via: Via::Saved,
            };
            let id = TrackId(id);
            self.cache.retain(|(c, _)| *c != id);
            self.cache.push_back((id.clone(), stream));
            self.restored.push(id);
        }
    }

    /// Whether `id`'s stream (or the one just dropped) came from the last session: its
    /// refusal says nothing of this session's URLs, so the resolver isn't told.
    pub fn is_from_last_session(&self, id: &TrackId) -> bool {
        self.restored.contains(id)
    }
}

impl Controller {
    /// At the start, the restored track, so Play starts it at once: buffered from the last
    /// session's stream when that's here (no resolve, nor its ads' wait), else resolved
    /// ahead and buffered then (`Msg::Resolved`). A saved URL refused anyway is resolved
    /// again (`preload_refused`).
    pub(crate) fn prepare_restored(&mut self, track: &Track) {
        // The performance harness's fixtures play nothing, so nothing is asked of YouTube.
        if self.fixture_stream.is_some() {
            return;
        }
        let mut t = track.clone();
        self.song_versions.fill(&mut t);
        let id = self.player.play_id(&t);
        if *self.player.status() == Status::Paused
            && let Some(s) = self.streams.fresh(&id, SystemTime::now())
        {
            log!("resolve {}: kept from the last session", id.0);
            self.audio.preload(super::super::source(s));
            return;
        }
        self.hint(std::slice::from_ref(track));
    }

    /// Writes the streams kept for the next session (`Streams::saved`): when the session is
    /// saved and they changed, in the background; at quit (`at_quit`), at once.
    pub(crate) fn save_streams(&mut self, at_quit: bool) {
        if !self.streams.take_changed() && !at_quit {
            return;
        }
        // None kept (signed out, say): no file.
        let text = self
            .streams
            .keeps_any()
            .then(|| self.streams.saved(SystemTime::now()));
        let (file, n) = self.streams.next_write();
        let path = self.paths.cache.join(SAVED_STREAMS);
        let write = move || {
            if let Err(e) = file.write(&path, n, text.as_deref()) {
                log!("saving the streams failed: {e}");
            }
        };
        if at_quit {
            write();
        } else {
            crate::task::blocking_detached(&tokio::runtime::Handle::current(), write);
        }
    }

    /// Signing out: the kept streams go, and their file with them.
    pub(crate) fn forget_saved_streams(&mut self) {
        let (file, n) = self.streams.forget_all();
        let path = self.paths.cache.join(SAVED_STREAMS);
        crate::task::blocking_detached(&tokio::runtime::Handle::current(), move || {
            if let Err(e) = file.write(&path, n, None) {
                log!("removing the saved streams failed: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::Want;
    use super::super::tests::{HOURS, id, stream};
    use super::*;

    fn from(ip: &str, expires_in: Duration) -> ResolvedStream {
        ResolvedStream {
            url: format!("https://example.invalid/videoplayback?expire=1&ip={ip}&c=X"),
            video_title: Some("Title".into()),
            duration: Some(Duration::from_secs(200)),
            ..stream(expires_in)
        }
    }

    /// A stream kept from the last session may have no start in memory: a rest on its row
    /// fetches it, as for a stream resolved while on screen.
    #[test]
    fn a_stream_from_the_last_session_has_its_start_fetched_when_its_row_is_rested_on() {
        let now = SystemTime::now();
        let mut before = Streams::default();
        before.done(&id("a"), Some(&from("1.2.3.4", HOURS)), &[]);
        let mut s = Streams::default();
        s.restore(&before.saved(now), now);
        assert!(s.head_wanted(&id("a"), now).is_some());
        assert!(s.head_wanted(&id("b"), now).is_none(), "none here");
    }

    /// The streams of the last session answer at once after a restart, before any resolve
    /// of this one: the restored track is the first song asked for, and a resolve of it
    /// waits out its ads again (5-6 s).
    #[test]
    fn saved_streams_answer_at_once_after_a_restart() {
        let now = SystemTime::now();
        let mut before = Streams::default();
        before.done(&id("a"), Some(&from("1.2.3.4", HOURS)), &[]);
        before.done(&id("old"), Some(&from("1.2.3.4", FRESH_FOR / 2)), &[]);
        let mut s = Streams::default();
        s.restore(&before.saved(now), now);
        let parts = |x: &ResolvedStream| (x.url.clone(), x.video_title.clone(), x.duration);
        assert_eq!(
            s.fresh(&id("a"), now).map(parts),
            before.get(&id("a")).map(parts)
        );
        s.done(&id("b"), Some(&from("1.2.3.4", HOURS)), &[]);
        assert!(s.fresh(&id("a"), now).is_some(), "the same address");
        assert!(
            s.get(&id("old")).is_none(),
            "one about to expire isn't kept"
        );
    }

    /// The saved file is written again only when the kept streams changed.
    #[test]
    fn the_kept_streams_count_as_changed_when_one_comes_or_goes() {
        let mut s = Streams::default();
        assert!(!s.take_changed());
        s.done(&id("a"), Some(&stream(HOURS)), &[]);
        assert!(s.take_changed());
        assert!(!s.take_changed(), "taken");
        s.done(&id("b"), None, &[]);
        assert!(!s.take_changed(), "a failed resolve keeps nothing");
        s.forget(&id("a"));
        assert!(s.take_changed());
    }

    /// Signing out drops the file and the streams with it; a write that was on its way
    /// doesn't bring the file back.
    #[test]
    fn signing_out_removes_the_saved_streams() {
        // Under `FEATHERPLAY_HOME`, else the temp folder.
        let base = std::env::var("FEATHERPLAY_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"));
        let dir = base.join(format!("test-saved-streams-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SAVED_STREAMS);
        let mut s = Streams::default();
        s.done(&id("a"), Some(&stream(HOURS)), &[]);
        let (file, older) = s.next_write();
        let text = s.saved(SystemTime::now());
        file.write(&path, older, Some(&text)).unwrap();
        let (file, signed_out) = s.forget_all();
        file.write(&path, signed_out, None).unwrap();
        file.write(&path, older, Some(&text)).unwrap();
        assert!(!path.exists());
        assert!(s.get(&id("a")).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A resolve started before the sign-out that answers after it plays, but isn't saved,
    /// so the removed file doesn't come back with it; one started after it is.
    #[test]
    fn a_resolve_from_before_the_sign_out_is_not_saved() {
        let now = SystemTime::now();
        let url = |n: &str| format!("https://rr1.example.invalid/videoplayback?ip=1.2.3.4&n={n}");
        let at = |n: &str| ResolvedStream {
            url: url(n),
            ..stream(HOURS)
        };
        let mut s = Streams::default();
        assert_eq!(s.want(&id("before"), now), Want::Start);
        let _ = s.forget_all();
        s.done(&id("before"), Some(&at("before")), &[]);
        assert!(s.get(&id("before")).is_some(), "it plays");
        assert!(!s.take_changed(), "nothing to save");
        assert!(!s.keeps_any());

        assert_eq!(s.want(&id("after"), now), Want::Start);
        s.done(&id("after"), Some(&at("after")), &[]);
        assert!(s.take_changed());
        let saved = s.saved(now);
        assert!(saved.contains(&url("after")), "{saved}");
        assert!(!saved.contains(&url("before")), "{saved}");
    }

    #[test]
    fn a_saved_stream_keeps_its_loudness() {
        let now = SystemTime::now();
        let mut before = Streams::default();
        let loud = ResolvedStream {
            loudness_db: Some(7.17),
            ..from("1.2.3.4", HOURS)
        };
        before.done(&id("loud"), Some(&loud), &[]);
        before.done(&id("ytdlp"), Some(&from("1.2.3.4", HOURS)), &[]);
        let mut s = Streams::default();
        s.restore(&before.saved(now), now);
        let db = |t: &str| s.get(&id(t)).map(|x| x.loudness_db);
        assert_eq!((db("loud"), db("ytdlp")), (Some(Some(7.17)), Some(None)));
    }

    /// A saved file with times past what the clock can hold (written by hand, or damaged)
    /// is read without a panic: such a stream isn't kept, and such a gate is left out.
    #[test]
    fn a_saved_time_too_far_ahead_is_read_without_a_panic() {
        let now = SystemTime::now();
        let url = "https://rr1.example.invalid/videoplayback?ip=1.2.3.4";
        let soon = (now + HOURS).duration_since(UNIX_EPOCH).unwrap().as_secs();
        let text = serde_json::json!({ "streams": [
            { "id": "far", "url": url, "expires": u64::MAX },
            { "id": "gate", "url": url, "expires": soon, "available_at": u64::MAX },
        ] })
        .to_string();
        let mut s = Streams::default();
        s.restore(&text, now);
        assert!(s.get(&id("far")).is_none());
        assert_eq!(s.get(&id("gate")).map(|x| x.available_at), Some(None));
    }

    /// Once a resolve shows another address, the last session's streams no longer answer,
    /// nor are they saved again for the next one.
    #[test]
    fn saved_streams_from_another_address_are_not_used() {
        let now = SystemTime::now();
        let mut before = Streams::default();
        before.done(&id("a"), Some(&from("1.2.3.4", HOURS)), &[]);
        let mut s = Streams::default();
        s.restore(&before.saved(now), now);
        s.done(&id("b"), Some(&from("5.6.7.8", HOURS)), &[]);
        assert!(s.fresh(&id("a"), now).is_none());
        assert_eq!(s.want(&id("a"), now), Want::Start);
        let mut next = Streams::default();
        next.restore(&s.saved(now), now);
        assert!(next.get(&id("a")).is_none(), "not saved again");
        assert!(next.get(&id("b")).is_some());
    }
}
