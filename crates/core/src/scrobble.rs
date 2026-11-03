//! Scrobble rules: when a track counts as played, which tracks count,
//! and the title cleanup sent to Last.fm. No I/O.

use std::time::{Duration, SystemTime};

use crate::model::{Track, VideoType};

/// Tracks this short or shorter are never scrobbled.
const MIN_DURATION: Duration = Duration::from_secs(30);
/// A track is scrobbled after half its duration or this much play time, whichever is less.
const MAX_THRESHOLD: Duration = Duration::from_secs(240);

/// Trailing video-title suffixes stripped for music and user videos, matched case-insensitively.
const TITLE_SUFFIXES: [&str; 6] = [
    " (Official Video)",
    " (Official Music Video)",
    " (Official Audio)",
    " (Lyrics)",
    " [Official Video]",
    " (Visualizer)",
];

/// The play time after which a track of `duration` is scrobbled:
/// `None` if it is 30 s or shorter, else `min(duration / 2, 240 s)`.
pub fn threshold(duration: Duration) -> Option<Duration> {
    (duration > MIN_DURATION).then(|| (duration / 2).min(MAX_THRESHOLD))
}

/// The title sent to Last.fm. For music and user videos, trailing suffixes
/// such as " (Official Video)" are removed; other titles are returned unchanged.
pub fn clean_title(title: &str, t: VideoType) -> String {
    if !matches!(t, VideoType::MusicVideo | VideoType::UserVideo) {
        return title.to_string();
    }
    let mut rest = title.trim_end();
    while let Some(stripped) = strip_one_suffix(rest) {
        rest = stripped;
    }
    rest.to_string()
}

fn strip_one_suffix(title: &str) -> Option<&str> {
    TITLE_SUFFIXES.iter().find_map(|suffix| {
        let cut = title.len().checked_sub(suffix.len())?;
        // The suffixes are ASCII, so a byte match also means `cut` is a char boundary.
        if !title.as_bytes()[cut..].eq_ignore_ascii_case(suffix.as_bytes()) {
            return None;
        }
        // Never reduce a title to nothing.
        let stripped = title[..cut].trim_end();
        (!stripped.is_empty()).then_some(stripped)
    })
}

/// The video's own metadata, as YouTube has it beside YouTube Music's: Pear's
/// "alternative" title and artist. Read from the stream's player data (yt-dlp's `title`
/// and first `tags` entry); either is `None` when the player data didn't have it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Alternates {
    /// The video title, unlocalised (YouTube Music may show a translated or romanised one).
    pub title: Option<String>,
    /// The first video tag. For songs (ATV) it is the artist as the label spelled it.
    pub artist: Option<String>,
}

impl Alternates {
    /// From the raw player fields; blank values count as missing.
    pub fn new(title: Option<&str>, first_tag: Option<&str>) -> Alternates {
        let keep = |s: Option<&str>| {
            s.map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        Alternates {
            title: keep(title),
            artist: keep(first_tag),
        }
    }
}

/// Which alternates replace YouTube Music's title and artist (Settings, Scrobbling).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetaOptions {
    pub alternative_titles: bool,
    pub alternative_artists: bool,
}

/// What a scrobble or a listen says about a track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScrobbleFields {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub duration_s: Option<u32>,
}

/// The fields sent for `track`. As in Pear: with alternative titles on, the video title
/// replaces the song title when there is one, and with alternative artists on, the first
/// tag replaces the artist. Otherwise the YouTube Music title and first artist are sent.
/// `clean_title` applies to whichever title is used. `None` when there's no artist at all.
pub fn scrobble_fields(track: &Track, alt: &Alternates, o: MetaOptions) -> Option<ScrobbleFields> {
    let artist = alt
        .artist
        .as_ref()
        .filter(|_| o.alternative_artists)
        .or_else(|| track.artists.first().map(|a| &a.name))?
        .clone();
    let title = alt
        .title
        .as_deref()
        .filter(|_| o.alternative_titles)
        .unwrap_or(&track.title);
    Some(ScrobbleFields {
        artist,
        track: clean_title(title, track.video_type),
        album: track.album.as_ref().map(|a| a.title.clone()),
        duration_s: track.duration.and_then(|d| u32::try_from(d.as_secs()).ok()),
    })
}

/// Whether a track of this type may be scrobbled. With "scrobble other media" off,
/// only songs (ATV) and official music videos (OMV) are.
pub fn eligible(t: VideoType, scrobble_other_media: bool) -> bool {
    scrobble_other_media || matches!(t, VideoType::Song | VideoType::MusicVideo)
}

/// Counts real play time for the current track and fires a scrobble once it
/// reaches the threshold. Seeks don't count: callers pass wall-clock play deltas.
#[derive(Debug, Default)]
pub struct ScrobbleTracker {
    current: Option<Current>,
}

#[derive(Debug)]
struct Current {
    track: Track,
    started: SystemTime,
    threshold: Option<Duration>,
    played: Duration,
    fired: bool,
}

impl ScrobbleTracker {
    /// Starts counting for `track`, which began playing at `started` (the scrobble timestamp).
    /// Call it again for each repeat-one loop so the track scrobbles again.
    pub fn start(&mut self, track: &Track, started: SystemTime) {
        self.current = Some(Current {
            track: track.clone(),
            started,
            threshold: track.duration.and_then(threshold),
            played: Duration::ZERO,
            fired: false,
        });
    }

    /// Adds `delta` of play time. Returns the track and its start time the first time
    /// the threshold is reached since the last `start`, and `None` otherwise.
    pub fn played(&mut self, delta: Duration) -> Option<(Track, SystemTime)> {
        let current = self.current.as_mut()?;
        let threshold = current.threshold?;
        if current.fired {
            return None;
        }
        current.played = current.played.saturating_add(delta);
        if current.played < threshold {
            return None;
        }
        current.fired = true;
        Some((current.track.clone(), current.started))
    }
}
