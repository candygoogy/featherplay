//! Scrobble rules: when a track counts as played, which tracks count,
//! and the title cleanup sent to Last.fm. No I/O.

use std::borrow::Cow;
use std::ops::Range;
use std::time::{Duration, SystemTime};

use crate::model::{Track, VideoType};

/// Tracks this short or shorter are never scrobbled.
const MIN_DURATION: Duration = Duration::from_secs(30);
/// A track is scrobbled after half its duration or this much play time, whichever is less.
const MAX_THRESHOLD: Duration = Duration::from_secs(240);

/// What a scrobbled video title loses at its end on top of its `display_title` tags,
/// matched case-insensitively. On screen it stays.
const LYRICS_SUFFIX: &str = " (Lyrics)";

/// The play time after which a track of `duration` is scrobbled:
/// `None` if it is 30 s or shorter, else `min(duration / 2, 240 s)`.
pub fn threshold(duration: Duration) -> Option<Duration> {
    (duration > MIN_DURATION).then(|| (duration / 2).min(MAX_THRESHOLD))
}

/// Bracketed tags in video titles that name the video, not the music, matched whole and
/// case-insensitively. Version tags such as "(Live)" or "(Lyrics)" aren't among them.
const VIDEO_TAGS: [&str; 13] = [
    "official music video",
    "official video",
    "official audio",
    "official lyric video",
    "official lyrics video",
    "official visualizer",
    "official visualiser",
    "visualizer",
    "visualiser",
    "official hd video",
    "official 4k video",
    "music video",
    "lyric video",
];

/// How a title credits a guest artist, lowercase.
const FEAT: [&str; 2] = ["feat. ", "ft. "];

/// The title shown for a track: for music and user videos, YouTube's video title without
/// the bracketed video tags ("(Official Music Video)", "[Official Video]"…) and without a
/// "(feat. X)" whose X is already credited as "ft. X" or "feat. X". Other titles, titles
/// with nothing to remove and titles that would be left empty are returned as they are.
pub fn display_title(title: &str, t: VideoType) -> Cow<'_, str> {
    if !is_video(t) || !title.contains(['(', '[']) {
        return Cow::Borrowed(title);
    }
    // Lowercased in ASCII only, so its byte offsets are the title's.
    let lower = title.to_ascii_lowercase();
    let mut dropped: Vec<Range<usize>> = Vec::new();
    for g in bracket_groups(&lower) {
        let inner = lower[g.start + 1..g.end - 1].trim();
        if VIDEO_TAGS.contains(&inner)
            || guest(inner).is_some_and(|x| credited_elsewhere(&lower, x, &g, &dropped))
        {
            dropped.push(g);
        }
    }
    if dropped.is_empty() {
        return Cow::Borrowed(title);
    }
    // What's left between the cuts, joined by one space, or none inside a bracket.
    let starts = std::iter::once(0).chain(dropped.iter().map(|g| g.end));
    let ends = dropped.iter().map(|g| g.start).chain([title.len()]);
    let mut kept = String::with_capacity(title.len());
    for piece in starts.zip(ends).map(|(s, e)| title[s..e].trim()) {
        if piece.is_empty() {
            continue;
        }
        let tight = kept.ends_with(['(', '[']) || piece.starts_with([')', ']']);
        if !kept.is_empty() && !tight {
            kept.push(' ');
        }
        kept.push_str(piece);
    }
    if kept.is_empty() {
        Cow::Borrowed(title)
    } else {
        Cow::Owned(kept)
    }
}

/// The byte ranges of "(…)" and "[…]" groups, brackets included, in order. A group closes
/// with its own bracket; one interrupted by another bracket isn't a group.
fn bracket_groups(s: &str) -> Vec<Range<usize>> {
    let bytes = s.as_bytes();
    let mut groups = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let close = match bytes[i] {
            b'(' => b')',
            b'[' => b']',
            _ => {
                i += 1;
                continue;
            }
        };
        let Some(off) = bytes[i + 1..]
            .iter()
            .position(|b| matches!(b, b'(' | b')' | b'[' | b']'))
        else {
            break;
        };
        let j = i + 1 + off;
        if bytes[j] == close {
            groups.push(i..j + 1);
            i = j + 1;
        } else {
            i = j;
        }
    }
    groups
}

/// The guest a "(feat. x)" group credits: "x". YouTube Music adds such a group to
/// video titles that may already say "ft. x".
fn guest(inner: &str) -> Option<&str> {
    inner
        .strip_prefix("feat. ")
        .map(str::trim)
        .filter(|x| !x.is_empty())
}

/// `x` is credited as "ft. x" or "feat. x", as whole words, somewhere in `lower` outside
/// the group `own` and the groups already dropped.
fn credited_elsewhere(lower: &str, x: &str, own: &Range<usize>, dropped: &[Range<usize>]) -> bool {
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    FEAT.iter().any(|f| {
        let credit = format!("{f}{x}");
        lower.match_indices(&credit).any(|(at, _)| {
            // Both ends are char boundaries: the match starts with ASCII and ends with `x`.
            let end = at + credit.len();
            let outside = |r: &Range<usize>| !r.contains(&at);
            outside(own)
                && dropped.iter().all(outside)
                && !word(lower[..at].chars().next_back())
                && !word(lower[end..].chars().next())
        })
    })
}

/// The title sent to Last.fm. For music and user videos, the `display_title` without a
/// trailing " (Lyrics)"; other titles are returned unchanged.
pub fn clean_title(title: &str, t: VideoType) -> String {
    let shown = display_title(title, t);
    if !is_video(t) {
        return shown.into_owned();
    }
    let mut rest = shown.trim_end();
    while let Some(stripped) = strip_lyrics(rest) {
        rest = stripped;
    }
    rest.to_string()
}

fn strip_lyrics(title: &str) -> Option<&str> {
    let cut = title.len().checked_sub(LYRICS_SUFFIX.len())?;
    // The suffix is ASCII, so a byte match also means `cut` is a char boundary.
    if !title.as_bytes()[cut..].eq_ignore_ascii_case(LYRICS_SUFFIX.as_bytes()) {
        return None;
    }
    // Never reduce a title to nothing.
    let stripped = title[..cut].trim_end();
    (!stripped.is_empty()).then_some(stripped)
}

/// Music and user videos: the tracks whose titles are YouTube's video titles.
fn is_video(t: VideoType) -> bool {
    matches!(t, VideoType::MusicVideo | VideoType::UserVideo)
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
