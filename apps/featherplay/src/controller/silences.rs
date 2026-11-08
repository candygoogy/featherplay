//! Skip silence's marks for the seek bar: where the current track is silent, from the audio
//! engine's scan of the whole stream (`AudioEvent::Silences`) or from the songs scanned
//! before (`SilenceCache`, by video id, so a song played again isn't scanned again). The
//! window gets what the current mode drops, in fractions of the duration
//! (`UiEvent::Silences`), and nothing while the mode is off or another track is current.
//!
//! A track's scan starts once its stream has all downloaded, while Skip silence is on.
//! Turned on mid-song, it scans the song then, at the decoder's next refill (~0.75 s of
//! playback); while paused, once it plays again; in the song's last 1.5 s (the decoder is
//! done with it), from the next song.

use std::ops::Range;
use std::time::Duration;

use featherplay_audio::{Silences, SkipSilence};
use featherplay_core::model::TrackId;
use featherplay_store::{SilenceCache, SongSilences};

use super::{Controller, UiEvent, skip_silences};

/// A stretch of the bar, as fractions of the duration: (start, end).
pub type Mark = (f32, f32);

/// The current track's silences, with what each mode drops worked out once.
#[derive(Clone, Debug)]
pub(super) struct TrackSilences {
    /// The id the stream was resolved for (`Player::play_id`).
    id: TrackId,
    /// The stream's length, for a track whose duration isn't known.
    length: Duration,
    start_and_end: Vec<Range<Duration>>,
    everywhere: Vec<Range<Duration>>,
}

impl TrackSilences {
    pub(super) fn new(id: TrackId, s: &Silences) -> Self {
        let length = if s.is_plausible() {
            Duration::from_secs_f64(s.frames as f64 / f64::from(s.rate))
        } else {
            Duration::ZERO
        };
        TrackSilences {
            id,
            length,
            start_and_end: s.spans(SkipSilence::StartAndEnd),
            everywhere: s.spans(SkipSilence::Everywhere),
        }
    }
}

/// What the bar marks: what `mode` drops from the track `current` (its play id), as
/// fractions of `duration` (else of the stream's length). Nothing for another track.
pub(super) fn marks(
    silences: Option<&TrackSilences>,
    current: Option<&TrackId>,
    mode: SkipSilence,
    duration: Option<Duration>,
) -> Vec<Mark> {
    let Some(s) = silences.filter(|s| Some(&s.id) == current) else {
        return Vec::new();
    };
    let spans = match mode {
        SkipSilence::Off => return Vec::new(),
        SkipSilence::StartAndEnd => &s.start_and_end,
        SkipSilence::Everywhere => &s.everywhere,
    };
    let total = duration
        .filter(|d| !d.is_zero())
        .unwrap_or(s.length)
        .as_secs_f64();
    if total <= 0.0 {
        return Vec::new();
    }
    let at = |d: Duration| (d.as_secs_f64() / total).clamp(0.0, 1.0) as f32;
    spans
        .iter()
        .map(|r| (at(r.start), at(r.end)))
        .filter(|(a, b)| b > a)
        .collect()
}

fn to_song(s: &Silences) -> SongSilences {
    SongSilences {
        rate: s.rate,
        frames: s.frames,
        runs: s.runs.clone(),
    }
}

/// A cached entry, unless it's damaged.
fn from_song(s: &SongSilences) -> Option<Silences> {
    Some(Silences {
        rate: s.rate,
        frames: s.frames,
        runs: s.runs.clone(),
    })
    .filter(Silences::is_plausible)
}

impl Controller {
    fn silence_cache(&mut self) -> &mut SilenceCache {
        let paths = &self.paths;
        self.silence_cache
            .get_or_insert_with(|| SilenceCache::load(paths))
    }

    /// `id` is about to load, preload or fade in from `url`: if it was scanned before, the
    /// engine needn't scan it, and its marks are known. Returns them.
    pub(super) fn known_silences(&mut self, id: &TrackId, url: &str) -> Option<TrackSilences> {
        let found = self.silence_cache().get(&id.0).and_then(from_song)?;
        self.audio.silences_known(url);
        Some(TrackSilences::new(id.clone(), &found))
    }

    /// `id` becomes the current track: its marks, if it was scanned before.
    pub(super) fn track_loaded(&mut self, id: &TrackId, url: &str) {
        self.silences = self.known_silences(id, url);
        self.push_marks();
    }

    /// The engine scanned the stream at `url`: kept for the song, and shown if it's still
    /// the current track's.
    pub(super) fn silences_found(&mut self, url: &str, found: &Silences) {
        let Some(id) = self
            .player
            .queue()
            .current()
            .map(|t| self.player.play_id(t))
        else {
            return;
        };
        if self.streams.get(&id).is_none_or(|s| s.url != url) {
            return;
        }
        log!(
            "silences of {}: {} run(s) in {:.1}s",
            id.0,
            found.runs.len(),
            found.frames as f64 / f64::from(found.rate.max(1))
        );
        let cache = self.silence_cache();
        cache.insert(&id.0, to_song(found));
        let cache = cache.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = cache.save() {
                log!("saving the silences failed: {e}");
            }
        });
        self.silences = Some(TrackSilences::new(id, found));
        self.push_marks();
    }

    /// Sends the bar's marks when they changed: a new track (none until it's scanned), its
    /// scan, or another mode.
    pub(super) fn push_marks(&mut self) {
        let current = self.player.queue().current();
        let id = current.map(|t| self.player.play_id(t));
        let marks = marks(
            self.silences.as_ref(),
            id.as_ref(),
            skip_silences(self.settings.skip_silence),
            current.and_then(|t| t.duration),
        );
        if self.shown_marks.as_ref() != Some(&marks) {
            self.shown_marks = Some(marks.clone());
            (self.ui.events)(UiEvent::Silences(marks));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10 s at 8 kHz: 1 s of silence at the start, 3 s from 4 s, 2 s at the end.
    fn found() -> Silences {
        Silences {
            rate: 8_000,
            frames: 80_000,
            runs: vec![(0, 8_000), (32_000, 24_000), (64_000, 16_000)],
        }
    }

    #[test]
    fn a_damaged_cache_entry_marks_nothing() {
        let song = |rate, frames, runs| SongSilences { rate, frames, runs };
        for bad in [
            song(1, u64::MAX, vec![(0, u64::MAX)]),
            song(48_000, 48_000, vec![(0, 96_000)]),
            song(48_000, 96_000, vec![(48_000, 100), (0, 100)]),
        ] {
            assert_eq!(from_song(&bad), None, "{bad:?}");
            let s = TrackSilences::new(
                id("a"),
                &Silences {
                    rate: bad.rate,
                    frames: bad.frames,
                    runs: bad.runs,
                },
            );
            assert!(marks(Some(&s), Some(&id("a")), SkipSilence::Everywhere, None).is_empty());
        }
    }

    fn id(s: &str) -> TrackId {
        TrackId(s.into())
    }

    #[test]
    fn the_marks_are_the_current_tracks_only() {
        let s = TrackSilences::new(id("a"), &found());
        let ten = Some(Duration::from_secs(10));
        let mode = SkipSilence::StartAndEnd;
        assert_eq!(
            marks(Some(&s), Some(&id("a")), mode, ten),
            [(0.0, 0.1), (0.85, 1.0)]
        );
        // The next track: nothing until it's scanned.
        assert!(marks(Some(&s), Some(&id("b")), mode, ten).is_empty());
        assert!(marks(Some(&s), None, mode, ten).is_empty());
    }

    #[test]
    fn the_marks_follow_the_mode() {
        let s = TrackSilences::new(id("a"), &found());
        let a = Some(&id("a"));
        // The stream's length when the track's isn't known.
        assert_eq!(
            marks(Some(&s), a, SkipSilence::Everywhere, None),
            [(0.0, 0.1), (0.45, 0.7), (0.85, 1.0)]
        );
        assert!(marks(Some(&s), a, SkipSilence::Off, None).is_empty());
    }
}
