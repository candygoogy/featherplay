//! The play queue: history, the current track and Up Next.
//!
//! Shuffle only reorders the upcoming tracks; history and the current track stay put.
//! Turning shuffle off puts the remaining upcoming tracks back in the order they were
//! queued, including tracks added while shuffle was on.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::model::{LikeStatus, ListEnd, RepeatMode, Track, TrackId, VideoType};

/// Pressing "previous" more than this far into a track restarts it instead.
const RESTART_THRESHOLD: Duration = Duration::from_secs(3);

/// Fallback xorshift state, used when a seed would otherwise give the all-zero state.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackAction {
    /// Seek the current track to 0.
    Restart,
    /// The previous track is now current.
    Previous,
}

#[derive(Clone, Debug, Default)]
pub struct Queue {
    /// Play order.
    tracks: Vec<Track>,
    /// One key per entry of `tracks`, so duplicate tracks stay distinguishable.
    keys: Vec<u64>,
    next_key: u64,
    index: usize,
    radio: bool,
    shuffle: bool,
    repeat: RepeatMode,
    /// What a list does when it runs out; a radio continues whatever it is.
    list_end: ListEnd,
    /// Entry keys in queued order. Only kept while shuffle is on; may hold stale keys.
    original: Vec<u64>,
    /// Keys of the entries a radio continuation appended; may hold stale keys.
    continued: HashSet<u64>,
    /// Keys of the entries Add to queue put at the end (`enqueue`), which a list's later
    /// tracks go before (`extend_list`); may hold stale keys.
    added: HashSet<u64>,
    /// Up next was cleared since the list was played: more of it (`extend_list`) is let go.
    list_cleared: bool,
    rng: u64,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the whole queue and makes `tracks[start]` current (clamped to the last track).
    /// If shuffle is on, the new upcoming tracks are shuffled.
    pub fn replace(&mut self, tracks: Vec<Track>, start: usize, radio: bool) {
        self.keys = (0..tracks.len()).map(|_| self.new_key()).collect();
        self.index = start.min(tracks.len().saturating_sub(1));
        self.tracks = tracks;
        self.radio = radio;
        self.original.clear();
        self.continued.clear();
        self.added.clear();
        self.list_cleared = false;
        if self.shuffle {
            self.original = self.keys.clone();
            self.shuffle_upcoming();
        }
    }

    pub fn current(&self) -> Option<&Track> {
        self.tracks.get(self.index)
    }

    /// The track played before the current one, the one "previous" goes back to.
    pub fn previous(&self) -> Option<&Track> {
        self.tracks.get(self.index.checked_sub(1)?)
    }

    /// The track `advance()` would move to, without moving.
    pub fn peek_next(&self) -> Option<&Track> {
        self.peek_next_past(|_| false)
    }

    /// The track `advance_past(skip)` would move to, without moving.
    pub fn peek_next_past(&self, skip: impl Fn(&Track) -> bool) -> Option<&Track> {
        self.tracks.get(self.next_index(skip)?)
    }

    /// Moves to the next track and returns it. Repeat One stays on the current track,
    /// Repeat All (or a list set to `ListEnd::Restart`) wraps to the first track, and
    /// otherwise it returns `None` at the end (the last track stays current).
    pub fn advance(&mut self) -> Option<&Track> {
        self.advance_past(|_| false)
    }

    /// `advance()`, passing over the tracks `skip` picks; they go to history. Repeat One
    /// stays on the current track whatever `skip` says. When only skipped tracks are left
    /// (Repeat All included, so it can't loop), it returns `None` and nothing moves.
    pub fn advance_past(&mut self, skip: impl Fn(&Track) -> bool) -> Option<&Track> {
        self.index = self.next_index(skip)?;
        self.current()
    }

    /// The next `n` tracks `advance_past(skip)` would move to, one after another, without
    /// moving: at most one lap round the queue. Repeat One gives the current track.
    pub fn peek_ahead_past(&self, n: usize, skip: impl Fn(&Track) -> bool) -> Vec<&Track> {
        let mut out = Vec::with_capacity(n);
        let mut seen = HashSet::new();
        let mut at = self.index;
        while out.len() < n
            && let Some(next) = self.next_index_from(at, &skip)
            && seen.insert(next)
        {
            out.push(&self.tracks[next]);
            at = next;
        }
        out
    }

    /// The index of the next track `skip` doesn't pick, at most one lap round the queue.
    fn next_index(&self, skip: impl Fn(&Track) -> bool) -> Option<usize> {
        self.next_index_from(self.index, skip)
    }

    /// `next_index` as if the track at `from` were current.
    fn next_index_from(&self, from: usize, skip: impl Fn(&Track) -> bool) -> Option<usize> {
        if self.repeat == RepeatMode::One {
            return (from < self.tracks.len()).then_some(from);
        }
        let mut at = from;
        for _ in 0..self.tracks.len() {
            at = if at + 1 < self.tracks.len() {
                at + 1
            } else if self.wraps() {
                0
            } else {
                return None;
            };
            if !skip(&self.tracks[at]) {
                return Some(at);
            }
        }
        None
    }

    /// Whether the end of the queue goes back to its first track.
    fn wraps(&self) -> bool {
        match self.repeat {
            RepeatMode::All => true,
            RepeatMode::Off => !self.radio && self.list_end == ListEnd::Restart,
            RepeatMode::One => false,
        }
    }

    /// "Previous": restarts the track if more than 3 s in, or if there is no previous track.
    pub fn back(&mut self, position: Duration) -> BackAction {
        if position > RESTART_THRESHOLD || self.index == 0 {
            return BackAction::Restart;
        }
        self.index -= 1;
        BackAction::Previous
    }

    /// Inserts `track` right after the current one. On an empty queue it becomes current.
    pub fn play_next(&mut self, track: Track) {
        let at = if self.tracks.is_empty() {
            0
        } else {
            self.index + 1
        };
        let key = self.insert(at, track);
        if self.shuffle {
            let current = self.keys.get(self.index).copied();
            match self.original.iter().position(|&k| Some(k) == current) {
                Some(pos) if at > 0 => self.original.insert(pos + 1, key),
                _ => self.original.push(key),
            }
        }
    }

    /// Inserts `tracks` right after the current one, in their order (a whole playlist's Play
    /// next). On an empty queue the first one becomes current.
    pub fn play_next_all(&mut self, tracks: Vec<Track>) {
        if self.tracks.is_empty() {
            self.enqueue(tracks);
            return;
        }
        // Each goes right after the current track, so the last one goes in first.
        for track in tracks.into_iter().rev() {
            self.play_next(track);
        }
    }

    /// Appends `tracks` to the end. On an empty queue the first one becomes current.
    pub fn enqueue(&mut self, tracks: Vec<Track>) {
        for track in tracks {
            let key = self.push(track);
            self.added.insert(key);
        }
    }

    /// More of the list that's playing, arrived after it started (a long playlist's later
    /// pages): after the list's upcoming tracks, before any Add to queue put after them
    /// (where those would be had the whole list been read) and any a continuation appended.
    /// With `mix_seed` (the list was shuffled), or with shuffle on, each goes to a random
    /// place among the list's upcoming tracks instead. On an empty queue the first one
    /// becomes current. Nothing once Up next was cleared since the list was played.
    pub fn extend_list(&mut self, tracks: Vec<Track>, mix_seed: Option<u64>) {
        if self.list_cleared {
            return;
        }
        let after_list = |k: &u64| self.continued.contains(k) || self.added.contains(k);
        let base = (self.index + 1).min(self.tracks.len());
        let list_end = base
            + self.keys[base..]
                .iter()
                .position(after_list)
                .unwrap_or(self.tracks.len() - base);
        if let Some(seed) = mix_seed {
            self.rng = seed ^ GOLDEN;
        }
        if self.rng == 0 {
            self.rng = GOLDEN;
        }
        let mix = mix_seed.is_some() || self.shuffle;
        // In queued order they go where the list's own tracks end, too.
        let mut queued_at = self
            .original
            .iter()
            .position(after_list)
            .unwrap_or(self.original.len());
        // Where the list's own tracks end, as each one goes in.
        for (end, track) in (list_end..).zip(tracks) {
            let at = if mix {
                base + (self.next_random() % (end - base + 1) as u64) as usize
            } else {
                end
            };
            let key = self.insert(at, track);
            if self.shuffle {
                self.original.insert(queued_at, key);
                queued_at += 1;
            }
        }
    }

    /// Moves an upcoming track. Indices are into `upcoming()`; out-of-range is ignored.
    pub fn move_item(&mut self, from: usize, to: usize) {
        let len = self.upcoming().len();
        if from >= len || to >= len {
            return;
        }
        let base = self.index + 1;
        let track = self.tracks.remove(base + from);
        let key = self.keys.remove(base + from);
        self.tracks.insert(base + to, track);
        self.keys.insert(base + to, key);
    }

    /// Removes an upcoming track. The index is into `upcoming()`; out-of-range is ignored.
    pub fn remove(&mut self, index: usize) {
        if index >= self.upcoming().len() {
            return;
        }
        let at = self.index + 1 + index;
        self.tracks.remove(at);
        self.keys.remove(at);
    }

    /// Drops every upcoming track. History and the current track stay.
    pub fn clear_upcoming(&mut self) {
        self.list_cleared = true;
        let keep = (self.index + 1).min(self.tracks.len());
        self.tracks.truncate(keep);
        self.keys.truncate(keep);
    }

    /// Turning shuffle on (or on again) shuffles the upcoming tracks with `seed`.
    /// Turning it off restores their queued order; `seed` is then ignored.
    pub fn set_shuffle(&mut self, on: bool, seed: u64) {
        if on {
            if !self.shuffle {
                self.original = self.keys.clone();
                self.shuffle = true;
            }
            self.rng = match seed ^ GOLDEN {
                0 => GOLDEN,
                state => state,
            };
            self.shuffle_upcoming();
        } else if self.shuffle {
            self.shuffle = false;
            self.restore_upcoming_order();
            self.original.clear();
        }
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
    }

    pub fn set_list_end(&mut self, list_end: ListEnd) {
        self.list_end = list_end;
    }

    /// The tracks after the current one, in play order.
    pub fn upcoming(&self) -> &[Track] {
        self.tracks.get(self.index + 1..).unwrap_or(&[])
    }

    /// Sets `duration` on every entry `matches` picks. Returns whether any entry changed.
    pub fn set_duration(&mut self, matches: impl Fn(&Track) -> bool, duration: Duration) -> bool {
        let mut changed = false;
        for t in self.tracks.iter_mut().filter(|t| matches(t)) {
            changed |= t.duration.replace(duration) != Some(duration);
        }
        changed
    }

    /// Sets `like` on every entry of the track `id`. Returns whether any entry changed.
    pub fn set_like(&mut self, id: &TrackId, like: LikeStatus) -> bool {
        let mut changed = false;
        for t in self.tracks.iter_mut().filter(|t| t.id == *id) {
            changed |= std::mem::replace(&mut t.like, like) != like;
        }
        changed
    }

    /// The upcoming entries of the music video `video` that know no song version become
    /// `song` (`Track::into_song_version`). Returns whether any did.
    pub fn set_song_version(&mut self, video: &TrackId, song: &Track) -> bool {
        let mut changed = false;
        let base = (self.index + 1).min(self.tracks.len());
        for t in &mut self.tracks[base..] {
            if t.id == *video && t.video_type == VideoType::MusicVideo && t.counterpart.is_none() {
                let mut v = t.clone();
                v.counterpart = Some(Box::new(song.clone()));
                *t = v.into_song_version();
                changed = true;
            }
        }
        changed
    }

    /// The upcoming entries with a known counterpart play as their song versions (`songs`)
    /// or as the videos. Returns whether any changed.
    pub fn set_versions(&mut self, songs: bool) -> bool {
        let base = (self.index + 1).min(self.tracks.len());
        let mut changed = false;
        for t in &mut self.tracks[base..] {
            let swap = if songs {
                t.song_version().is_some()
            } else {
                t.video_version().is_some()
            };
            if swap {
                let entry = t.clone();
                *t = if songs {
                    entry.into_song_version()
                } else {
                    entry.into_video_version()
                };
                changed = true;
            }
        }
        changed
    }

    /// Whether this is a radio queue (it continues by itself).
    pub fn is_radio(&self) -> bool {
        self.radio
    }

    pub fn is_shuffled(&self) -> bool {
        self.shuffle
    }

    /// The index into `upcoming()` of the first track a radio continuation appended, where
    /// the Up next panel shows its "Autoplay" divider.
    pub fn autoplay_start(&self) -> Option<usize> {
        let base = (self.index + 1).min(self.keys.len());
        self.keys[base..]
            .iter()
            .position(|k| self.continued.contains(k))
    }

    /// Whether the current track is one a continuation appended.
    pub fn current_is_autoplay(&self) -> bool {
        self.keys
            .get(self.index)
            .is_some_and(|k| self.continued.contains(k))
    }

    /// Whether this queue continues with radio tracks: a radio always does, a list only
    /// with `ListEnd::Autoplay` and without Repeat All (which restarts it instead).
    pub fn continues(&self) -> bool {
        self.radio || (self.list_end == ListEnd::Autoplay && self.repeat != RepeatMode::All)
    }

    /// Radio queues, and lists that autoplay, ask for more tracks when two or fewer are
    /// left.
    pub fn needs_continuation(&self) -> bool {
        self.continues() && self.upcoming().len() <= 2
    }

    /// Drops the upcoming tracks a continuation appended. Returns whether any were.
    pub fn drop_upcoming_continuation(&mut self) -> bool {
        let base = (self.index + 1).min(self.tracks.len());
        let before = self.tracks.len();
        let mut at = base;
        while at < self.tracks.len() {
            if self.continued.contains(&self.keys[at]) {
                self.tracks.remove(at);
                self.keys.remove(at);
            } else {
                at += 1;
            }
        }
        self.tracks.len() != before
    }

    /// Appends radio continuation tracks, skipping ids already in the queue (history
    /// included) and repeats within `tracks`. A music video and its song version count as
    /// one recording.
    pub fn append_continuation(&mut self, tracks: Vec<Track>) {
        let mut seen: HashSet<TrackId> = self.tracks.iter().flat_map(Track::ids).cloned().collect();
        for track in tracks {
            if track.ids().all(|id| !seen.contains(id)) {
                seen.extend(track.ids().cloned());
                let key = self.push(track);
                self.continued.insert(key);
            }
        }
    }

    fn new_key(&mut self) -> u64 {
        let key = self.next_key;
        self.next_key += 1;
        key
    }

    fn insert(&mut self, at: usize, track: Track) -> u64 {
        let key = self.new_key();
        self.tracks.insert(at, track);
        self.keys.insert(at, key);
        key
    }

    fn push(&mut self, track: Track) -> u64 {
        let key = self.insert(self.tracks.len(), track);
        if self.shuffle {
            self.original.push(key);
        }
        key
    }

    /// Seeded Fisher–Yates over the upcoming tracks.
    fn shuffle_upcoming(&mut self) {
        let base = self.index + 1;
        let len = self.tracks.len().saturating_sub(base);
        for i in (1..len).rev() {
            let j = (self.next_random() % (i as u64 + 1)) as usize;
            self.tracks.swap(base + i, base + j);
            self.keys.swap(base + i, base + j);
        }
    }

    /// xorshift64.
    fn next_random(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn restore_upcoming_order(&mut self) {
        let rank: HashMap<u64, usize> = self
            .original
            .iter()
            .enumerate()
            .map(|(pos, &key)| (key, pos))
            .collect();
        let base = (self.index + 1).min(self.tracks.len());
        let mut rest: Vec<(u64, Track)> = self
            .keys
            .drain(base..)
            .zip(self.tracks.drain(base..))
            .collect();
        rest.sort_by_key(|(key, _)| rank.get(key).copied().unwrap_or(usize::MAX));
        for (key, track) in rest {
            self.keys.push(key);
            self.tracks.push(track);
        }
    }
}
