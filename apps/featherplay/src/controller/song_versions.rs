//! Song versions of the music videos that come without one. Only the radio's watch playlist
//! names a video's song (its counterpart); Home, search, artist pages and playlists give
//! the video alone, so "Prefer song versions" had nothing to play for them.
//!
//! A video's own watch playlist (the radio seeded on it: the request a click on it makes
//! anyway, for what plays after it) names its song as its first entry's counterpart. So
//! that a start never waits on it, it's asked ahead: when the video's card or row is
//! hovered or pressed, and for the queue's tracks ahead. A click that comes before the
//! answer, or before the song's stream is as ready as the video's, plays the video, which
//! then stays (no switch mid-song); the answer serves the next time. The radio a click
//! starts after a lookup reuses its watch playlist, so the click asks nothing more.
//!
//! Each video is asked at most once a session (a failure counts as "none"), `LOOKUPS` at a
//! time; nothing retries.

use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime};

use featherplay_core::model::{Track, TrackId, VideoType};
use featherplay_core::player::Input;
use featherplay_ytm::ApiError;

use super::streams::Readiness;
use super::{Controller, Msg};

/// Videos whose answer is kept, oldest dropped first.
const KNOWN: usize = 256;
/// Lookups on their way at once; more wait for a later hover or song.
const LOOKUPS: usize = 2;
/// The lookups' watch playlists kept for the radio of a click.
const RADIOS: usize = 4;
/// How long a lookup's watch playlist stands in for the radio a click asks for.
const RADIO_FRESH: Duration = Duration::from_secs(120);

#[derive(Default)]
pub(super) struct SongVersions {
    /// Each video asked about, with its song version or `None` (it has none, or the lookup
    /// failed). Oldest first.
    known: VecDeque<(TrackId, Option<Track>)>,
    /// The videos being looked up.
    running: Vec<TrackId>,
    /// The lookups' watch playlists: (video, when, tracks). Oldest first.
    radios: VecDeque<(TrackId, Instant, Vec<Track>)>,
}

impl SongVersions {
    fn known(&self, video: &TrackId) -> Option<&Option<Track>> {
        self.known.iter().find(|(v, _)| v == video).map(|(_, s)| s)
    }

    fn remember(&mut self, video: TrackId, song: Option<Track>) {
        self.known.retain(|(v, _)| *v != video);
        self.known.push_back((video, song));
        if self.known.len() > KNOWN {
            self.known.pop_front();
        }
    }

    /// Gives a music video that came without its song version the one found for it.
    pub fn fill(&self, t: &mut Track) {
        if t.video_type == VideoType::MusicVideo
            && t.counterpart.is_none()
            && let Some(Some(song)) = self.known(&t.id)
        {
            t.counterpart = Some(Box::new(song.clone()));
        }
    }

    /// Whether looking `t` up could find something: a music video without a song version
    /// that's neither known nor on its way, with a lookup slot free.
    pub fn wants(&self, t: &Track) -> bool {
        t.video_type == VideoType::MusicVideo
            && t.counterpart.is_none()
            && self.known(&t.id).is_none()
            && !self.running.contains(&t.id)
            && self.running.len() < LOOKUPS
    }

    pub fn started(&mut self, video: &TrackId) {
        self.running.push(video.clone());
    }

    /// What a watch playlist says: each music video in it has the song version it names,
    /// or none (unless one was found before).
    pub fn learn(&mut self, tracks: &[Track]) {
        for t in tracks
            .iter()
            .filter(|t| t.video_type == VideoType::MusicVideo)
        {
            let song = t.song_version().cloned();
            if song.is_some() || self.known(&t.id).is_none() {
                self.remember(t.id.clone(), song);
            }
        }
    }

    /// The lookup of `video` ended with its watch playlist, or `None` on failure. Returns
    /// the song version it found.
    pub fn done(
        &mut self,
        video: &TrackId,
        answer: Option<Vec<Track>>,
        now: Instant,
    ) -> Option<Track> {
        self.running.retain(|v| v != video);
        match answer {
            Some(tracks) => {
                self.learn(&tracks);
                if self.known(video).is_none() {
                    // Not in its own watch playlist (or not a music video there).
                    self.remember(video.clone(), None);
                }
                self.radios.push_back((video.clone(), now, tracks));
                if self.radios.len() > RADIOS {
                    self.radios.pop_front();
                }
            }
            None => self.remember(video.clone(), None),
        }
        self.known(video).cloned().flatten()
    }

    /// A lookup's watch playlist that is the radio of `seed`, a video looked up lately or
    /// the song playing in its place.
    pub fn radio(&self, seed: &TrackId, now: Instant) -> Option<Vec<Track>> {
        self.radios
            .iter()
            .rev()
            .filter(|(_, at, _)| now.duration_since(*at) < RADIO_FRESH)
            .find(|(video, _, _)| {
                video == seed || matches!(self.known(video), Some(Some(song)) if song.id == *seed)
            })
            .map(|(_, _, tracks)| tracks.clone())
    }
}

/// `input` with the song versions found for the music videos it brings (`fill`). A track it
/// starts plays its song version only if the song's stream is as ready as the video's
/// (`ready`): a start never waits longer for the song. Otherwise it plays the video, and
/// stays on it.
pub(super) fn with_song_versions(
    mut input: Input,
    known: &SongVersions,
    queue_empty: bool,
    ready: impl Fn(&TrackId) -> Readiness,
) -> Input {
    let (tracks, start): (&mut [Track], Option<usize>) = match &mut input {
        Input::PlayTracks { tracks, start, .. } => (tracks, Some(*start)),
        Input::PlayNextAll(tracks) | Input::Enqueue(tracks) => (tracks, queue_empty.then_some(0)),
        Input::PlayNext(t) => (std::slice::from_mut(t), queue_empty.then_some(0)),
        Input::ContinuationLoaded(_, tracks) => (tracks, None),
        _ => return input,
    };
    for t in tracks.iter_mut() {
        known.fill(t);
    }
    if let Some(t) = start.and_then(|i| tracks.get_mut(i))
        && let Some(song) = t.song_version()
        && ready(&song.id) < ready(&t.id)
    {
        log!(
            "play {}: its song version {} isn't as ready, the video plays",
            t.id.0,
            song.id.0
        );
        t.counterpart = None;
    }
    input
}

impl Controller {
    /// Looks up the song version of a hovered or upcoming music video, when nobody knows
    /// it yet (see the module docs).
    pub(super) fn look_up_song_version(&mut self, t: &Track) {
        if !self.settings.prefer_song_versions || !self.song_versions.wants(t) {
            return;
        }
        log!("song version of {}: looking it up", t.id.0);
        self.song_versions.started(&t.id);
        let source = self.ui.catalog.source();
        let (id, tx) = (t.id.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = source.radio(&id).await;
            let _ = tx.send(Msg::SongVersion(id, result));
        });
    }

    /// The queue's tracks ahead get their song versions looked up, as their streams are
    /// resolved ahead.
    pub(super) fn look_up_ahead(&mut self, n: usize) {
        let ahead: Vec<Track> = self
            .player
            .queue()
            .upcoming()
            .iter()
            .take(n)
            .filter(|t| self.song_versions.wants(t))
            .cloned()
            .collect();
        for t in &ahead {
            self.look_up_song_version(t);
        }
    }

    pub(super) fn song_version_found(
        &mut self,
        video: TrackId,
        result: Result<Vec<Track>, ApiError>,
    ) {
        let answer = match result {
            Ok(tracks) => Some(tracks),
            Err(e) => {
                log!("song version of {}: the lookup failed: {e}", video.0);
                None
            }
        };
        let Some(mut song) = self.song_versions.done(&video, answer, Instant::now()) else {
            log!("song version of {}: none", video.0);
            return;
        };
        log!("song version of {}: {}", video.0, song.id.0);
        if !self.settings.prefer_song_versions {
            return;
        }
        let id = song.id.clone();
        // A song rated this session keeps its rating (a dislike is passed over).
        if let Some(like) = self.likes.get(&id) {
            song.like = *like;
        }
        // Its entries to come in the queue play the song now.
        self.input(Input::SongVersion { video, song });
        // And a click on the card or row that asked plays it: its stream ahead too.
        self.streams.hint(&id, SystemTime::now());
        self.start_hints();
    }
}

#[cfg(test)]
mod tests {
    use featherplay_core::model::LikeStatus;

    use super::*;

    fn id(s: &str) -> TrackId {
        TrackId(s.into())
    }

    fn typed(s: &str, video_type: VideoType) -> Track {
        Track {
            id: id(s),
            title: format!("Title {s}"),
            artists: Vec::new(),
            album: None,
            duration: None,
            video_type,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            counterpart: None,
        }
    }

    /// A watch playlist seeded on video `v`, whose song is `s`.
    fn watch_playlist() -> Vec<Track> {
        let mut v = typed("v", VideoType::MusicVideo);
        v.counterpart = Some(Box::new(typed("s", VideoType::Song)));
        vec![v, typed("a", VideoType::Song)]
    }

    fn found() -> SongVersions {
        let mut s = SongVersions::default();
        s.started(&id("v"));
        s.done(&id("v"), Some(watch_playlist()), Instant::now());
        s
    }

    #[test]
    fn a_lookup_finds_the_song_in_the_videos_watch_playlist() {
        let mut s = SongVersions::default();
        let video = typed("v", VideoType::MusicVideo);
        assert!(s.wants(&video));
        s.started(&id("v"));
        assert!(!s.wants(&video), "on its way");
        let song = s.done(&id("v"), Some(watch_playlist()), Instant::now());
        assert_eq!(song.map(|t| t.id), Some(id("s")));
        let mut later = video.clone();
        s.fill(&mut later);
        assert_eq!(later.song_version().map(|t| &t.id), Some(&id("s")));
        assert!(!s.wants(&video), "asked once");
    }

    /// A failed lookup isn't asked again: no request loop.
    #[test]
    fn a_failed_lookup_is_not_asked_again() {
        let mut s = SongVersions::default();
        s.started(&id("v"));
        assert_eq!(s.done(&id("v"), None, Instant::now()), None);
        assert!(!s.wants(&typed("v", VideoType::MusicVideo)));
    }

    #[test]
    fn lookups_are_bounded() {
        let mut s = SongVersions::default();
        for v in ["a", "b"] {
            s.started(&id(v));
        }
        assert!(!s.wants(&typed("c", VideoType::MusicVideo)));
        assert!(
            !s.wants(&typed("d", VideoType::Song)),
            "songs aren't looked up"
        );
    }

    /// A click on the video after its lookup: its radio is the lookup's watch playlist,
    /// whether the video plays or the song in its place.
    #[test]
    fn the_radio_after_a_lookup_is_its_watch_playlist() {
        let s = found();
        let now = Instant::now();
        assert_eq!(s.radio(&id("v"), now), Some(watch_playlist()));
        assert_eq!(s.radio(&id("s"), now), Some(watch_playlist()));
        assert_eq!(s.radio(&id("a"), now), None);
        assert_eq!(s.radio(&id("v"), now + RADIO_FRESH), None, "too old");
    }

    fn play(t: Track) -> Input {
        Input::PlayTracks {
            tracks: vec![t],
            start: 0,
            radio: true,
        }
    }

    fn started(input: Input) -> Track {
        match input {
            Input::PlayTracks { tracks, .. } => tracks.into_iter().next().unwrap(),
            other => panic!("{other:?}"),
        }
    }

    /// A click plays the song version found ahead, once its stream is as ready as the
    /// video's.
    #[test]
    fn a_click_plays_the_song_version_when_it_is_as_ready() {
        let s = found();
        let ready = |t: &TrackId| {
            if t.0 == "s" {
                Readiness::Here
            } else {
                Readiness::Resolving
            }
        };
        let input = with_song_versions(play(typed("v", VideoType::MusicVideo)), &s, true, ready);
        assert_eq!(started(input).song_version().map(|t| &t.id), Some(&id("s")));
    }

    /// The video's stream is ready and the song's isn't: the video plays, as before, so
    /// the start isn't slower.
    #[test]
    fn a_click_plays_the_video_when_the_song_would_start_later() {
        let s = found();
        let ready = |t: &TrackId| {
            if t.0 == "v" {
                Readiness::Here
            } else {
                Readiness::Resolving
            }
        };
        let input = with_song_versions(play(typed("v", VideoType::MusicVideo)), &s, true, ready);
        assert_eq!(started(input).counterpart, None);
    }

    /// Tracks added after the current one take the song versions found; none of them
    /// starts, so readiness doesn't matter.
    #[test]
    fn tracks_added_to_the_queue_take_their_song_versions() {
        let s = found();
        let input = with_song_versions(
            Input::Enqueue(vec![typed("v", VideoType::MusicVideo)]),
            &s,
            false,
            |_| Readiness::Unresolved,
        );
        let Input::Enqueue(tracks) = input else {
            panic!()
        };
        assert!(tracks[0].song_version().is_some());
    }
}
