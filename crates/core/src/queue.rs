//! The play queue: history, the current track and Up Next.
//!
//! Shuffle only reorders the upcoming tracks; history and the current track stay put.
//! Turning shuffle off puts the remaining upcoming tracks back in the order they were
//! queued, including tracks added while shuffle was on.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::model::{ListEnd, RepeatMode, Track};

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
        match self.repeat {
            RepeatMode::One => self.current(),
            _ if self.wraps() => self.tracks.get(self.index + 1).or(self.tracks.first()),
            _ => self.tracks.get(self.index + 1),
        }
    }

    /// Moves to the next track and returns it. Repeat One stays on the current track,
    /// Repeat All (or a list set to `ListEnd::Restart`) wraps to the first track, and
    /// otherwise it returns `None` at the end (the last track stays current).
    pub fn advance(&mut self) -> Option<&Track> {
        if self.tracks.is_empty() {
            return None;
        }
        match self.repeat {
            RepeatMode::One => {}
            _ if self.index + 1 < self.tracks.len() => self.index += 1,
            _ if self.wraps() => self.index = 0,
            _ => return None,
        }
        self.current()
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
            self.push(track);
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
    /// included) and repeats within `tracks`.
    pub fn append_continuation(&mut self, tracks: Vec<Track>) {
        let mut seen: HashSet<_> = self.tracks.iter().map(|t| t.id.clone()).collect();
        for track in tracks {
            if seen.insert(track.id.clone()) {
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
