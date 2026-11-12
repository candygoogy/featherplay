//! Looking lyrics up: LRCLIB and the song's own lyrics (`settle` picks), the fixtures in
//! `--fixtures`, and when a track is looked up and asked again; answers land in the cache.

use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

use featherplay_controller::catalog::Source;
use featherplay_controller::fixtures::Fixtures;
use featherplay_core::model::{Track, TrackId};
use featherplay_core::tr;
use featherplay_lyrics::lrclib::{self, Lookup, LrcLib, Query};
use featherplay_lyrics::{Line, Lyrics};
use featherplay_ytm::SongLyrics;
use slint::{ComponentHandle, Timer, TimerMode};

use super::{Entry, LOOKUPS, RESYNC, STATE, WAIT, WAITING, fill, show_failure, sync};
use crate::app::Ctx;
use crate::pages::settings;
use crate::ui::LyricsData;

/// The credit under lyrics from LRCLIB.
fn lrclib_credit() -> &'static str {
    tr!("Lyrics from LRCLIB")
}
/// Tracks whose lyrics are kept; past this the map starts over.
const MAX_CACHED: usize = 64;
/// LRCLIB failed and the song's plain lyrics show instead: it's asked once more this long
/// after, while the view stays open on them. It answered 503 on and off on 2026-10-31.
const RESYNC_AFTER: Duration = Duration::from_secs(60);
/// A track change while a lyrics view shows waits this long before looking the new track up,
/// so skipping through songs costs no requests.
const SKIP_WAIT: Duration = Duration::from_secs(1);

/// What a lookup found for one track.
#[derive(Debug)]
pub struct Found {
    pub(super) lyrics: Option<Lyrics>,
    /// Where they come from, shown under them ("" when unknown).
    pub(super) credit: String,
    /// Kept for the session. Not when LRCLIB failed and these are the fallback: the tab
    /// asks again the next time it opens on the track.
    pub(super) keep: bool,
}

impl Found {
    /// The song's plain lyrics, shown because LRCLIB failed rather than missed.
    pub(super) fn synced_unavailable(&self) -> bool {
        !self.keep && matches!(self.lyrics, Some(Lyrics::Plain(_)))
    }
}

/// What LRCLIB is asked once more, a minute after it failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Again {
    /// Synced lyrics, to take the place of the song's plain ones: anything else leaves those.
    Synced,
    /// Any lyrics, for a song with none of its own; LRCLIB having none is an answer too.
    Any,
}

/// Asks LRCLIB once more. `None` when it fails again, or has nothing better than what shows.
async fn ask_lrclib_again(lrclib: &LrcLib, track: &Track, again: Again) -> Option<Found> {
    let title = &track.title;
    let lookup = match lrclib.find(&query(track)).await {
        Ok(lookup) => lookup,
        Err(e) => {
            log!("lyrics {title:?}: {e}, asked again");
            return None;
        }
    };
    log!("lyrics {title:?}: LRCLIB {lookup}, asked again");
    match (lookup, again) {
        (
            Lookup::Found {
                lyrics: lyrics @ Lyrics::Synced(_),
                ..
            },
            _,
        )
        | (Lookup::Found { lyrics, .. }, Again::Any) => Some(Found {
            lyrics: Some(lyrics),
            credit: lrclib_credit().into(),
            keep: true,
        }),
        (_, Again::Synced) => None,
        (_, Again::Any) => Some(Found {
            lyrics: None,
            credit: String::new(),
            keep: true,
        }),
    }
}

/// The re-ask on the runtime: the fixtures' synced record in `--fixtures`, never the network.
async fn ask_again_from(source: Source, track: Track, again: Again) -> Option<Found> {
    if let Source::Fixtures(f) = source {
        return fixture_for(&f, &track.id, true).ok();
    }
    ask_lrclib_again(&client()?, &track, again).await
}

/// Looks `track` up `SKIP_WAIT` from now if it's still the one shown: another track change
/// meanwhile replaces the wait, so a run of skips looks up only the track that stays.
pub(super) fn fetch_soon(ctx: &Rc<Ctx>, track: Track) {
    WAITING.with(|w| *w.borrow_mut() = Some(crate::busy::Busy::start()));
    let weak = Rc::downgrade(ctx);
    WAIT.with(|t| {
        t.start(TimerMode::SingleShot, SKIP_WAIT, move || {
            WAITING.with(|w| w.borrow_mut().take());
            let Some(ctx) = weak.upgrade() else { return };
            let due = STATE.with(|s| {
                let s = s.borrow();
                s.live && s.shown.as_ref() == Some(&track.id) && !s.cache.contains_key(&track.id)
            });
            if due {
                fetch(&ctx, track.clone());
            }
        });
    });
}

/// Stops a wait for a lookup; whether one was waiting.
pub(super) fn stop_wait() -> bool {
    WAIT.with(Timer::stop);
    WAITING.with(|w| w.borrow_mut().take()).is_some()
}

/// LRCLIB failed for `id`, on screen (the song's plain lyrics show, or none at all): it's
/// asked once more `RESYNC_AFTER` from now, as long as the view stays open on the track
/// (closing it or another track stops the timer). Once: what the re-ask finds doesn't arm it
/// again.
pub(super) fn resync_later(ctx: &Rc<Ctx>, id: &TrackId, again: Again) {
    if !STATE.with(|s| s.borrow().live) {
        return;
    }
    let weak = Rc::downgrade(ctx);
    let id = id.clone();
    RESYNC.with(|t| {
        t.start(TimerMode::SingleShot, RESYNC_AFTER, move || {
            if let Some(ctx) = weak.upgrade() {
                resync(&ctx, id.clone(), again);
            }
        });
    });
}

/// Asks LRCLIB again for `id`, still on screen, and shows what it found if that's better.
fn resync(ctx: &Rc<Ctx>, id: TrackId, again: Again) {
    let on_screen = settings::lyrics_shown()
        && STATE.with(|s| {
            let s = s.borrow();
            s.live && s.shown.as_ref() == Some(&id)
        });
    let Some(track) = ctx
        .player()
        .and_then(|s| s.current)
        .filter(|t| on_screen && t.id == id)
    else {
        return;
    };
    LOOKUPS.with(|n| n.set(n.get() + 1));
    let generation = STATE.with(|s| s.borrow().generation);
    ctx.load(
        move |source| ask_again_from(source, track, again),
        move |ctx, ui, found| {
            let Some(found) = found else { return };
            let found = Rc::new(found);
            let shown = STATE.with(|s| {
                let mut s = s.borrow_mut();
                if s.generation != generation {
                    return false;
                }
                s.retry.succeeded(&id);
                s.cache.insert(id.clone(), Entry::Ready(found.clone()));
                s.shown.as_ref() == Some(&id)
            });
            if shown {
                fill(&ui.global::<LyricsData>(), &found);
                sync(ctx);
            }
        },
    );
}

/// Looks up `track`'s lyrics on the runtime; the answer lands in the cache, and on the tab
/// if it still shows that track.
pub(super) fn fetch(ctx: &Rc<Ctx>, track: Track) {
    if !settings::lyrics_shown() {
        return;
    }
    let use_lrclib = settings::lyrics_lrclib();
    LOOKUPS.with(|n| n.set(n.get() + 1));
    let id = track.id.clone();
    let generation = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.cache.len() >= MAX_CACHED {
            let shown = s.shown.clone();
            s.cache.retain(|k, _| Some(k) == shown.as_ref());
        }
        s.cache.insert(id.clone(), Entry::Loading);
        s.generation
    });
    ctx.load(
        move |source| look_up(source, use_lrclib, track),
        move |ctx, ui, result| {
            let current = STATE.with(|s| s.borrow().generation == generation);
            if !current {
                return;
            }
            let found = result.map(Rc::new);
            if let Err(e) = &found {
                log!("lyrics for {}: {e}", id.0);
            }
            let shown = STATE.with(|s| {
                let mut s = s.borrow_mut();
                match &found {
                    Ok(f) if f.keep => {
                        s.retry.succeeded(&id);
                        s.cache.insert(id.clone(), Entry::Ready(f.clone()))
                    }
                    // Asked again the next time the view opens on this track, or on Try again.
                    _ => {
                        s.retry.failed(id.clone());
                        s.cache.remove(&id)
                    }
                };
                s.shown.as_ref() == Some(&id)
            });
            if shown {
                let data = ui.global::<LyricsData>();
                match found {
                    Ok(f) => {
                        fill(&data, &f);
                        if f.synced_unavailable() {
                            resync_later(ctx, &id, Again::Synced);
                        }
                    }
                    Err(failure) => show_failure(ctx, &data, &id, &failure),
                }
                sync(ctx);
            }
        },
    );
}

/// Why a lookup found nothing.
#[derive(Debug)]
pub(super) enum Failure {
    /// LRCLIB failed and the song has no lyrics of its own: there may be some once LRCLIB
    /// answers again.
    Unavailable(String),
    /// The song's own lyrics couldn't be asked for.
    Failed(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Unavailable(e) | Failure::Failed(e) => f.write_str(e),
        }
    }
}

/// LRCLIB when allowed, then the song's own lyrics (`settle` picks). An error only when
/// nothing was found and a lookup failed. Logs what each found, by the track's title.
async fn look_up(source: Source, use_lrclib: bool, track: Track) -> Result<Found, Failure> {
    if let Source::Fixtures(f) = source {
        return fixture_for(&f, &track.id, use_lrclib).map_err(Failure::Failed);
    }
    let title = &track.title;
    let mut lrclib = FromLrclib::Missed;
    if use_lrclib {
        match client() {
            Some(client) => match client.find(&query(&track)).await {
                Ok(lookup) => {
                    log!("lyrics {title:?}: LRCLIB {lookup}");
                    match lookup {
                        Lookup::Found {
                            lyrics: lyrics @ Lyrics::Synced(_),
                            ..
                        } => {
                            return Ok(Found {
                                lyrics: Some(lyrics),
                                credit: lrclib_credit().into(),
                                keep: true,
                            });
                        }
                        Lookup::Found {
                            lyrics: Lyrics::Plain(text),
                            ..
                        } => lrclib = FromLrclib::Plain(text),
                        _ => {}
                    }
                }
                Err(e) => {
                    log!("lyrics {title:?}: {e}");
                    lrclib = FromLrclib::Failed(e.to_string());
                }
            },
            None => lrclib = FromLrclib::Failed("no LRCLIB client".into()),
        }
    }
    let own = source.lyrics(&track.id).await.map_err(|e| e.to_string());
    match &own {
        Ok(Some(SongLyrics::Timed(t))) => {
            log!(
                "lyrics {title:?}: YouTube Music, {} timed lines",
                t.lines.len()
            );
        }
        Ok(Some(SongLyrics::Plain(_))) => log!("lyrics {title:?}: YouTube Music, plain"),
        Ok(None) => log!("lyrics {title:?}: YouTube Music, none"),
        Err(_) => {}
    }
    settle(lrclib, own)
}

/// What LRCLIB said, short of synced lyrics.
#[derive(Debug)]
enum FromLrclib {
    /// Nothing, or not asked (Settings).
    Missed,
    /// Plain lyrics only.
    Plain(String),
    /// It couldn't tell: an error or an outage.
    Failed(String),
}

/// What the tab gets once LRCLIB had no synced lyrics: the song's own timed lyrics, then
/// LRCLIB's plain ones, then the song's own plain ones, or none. The song's plain lyrics
/// after an LRCLIB failure aren't kept, so the tab asks LRCLIB again the next time it opens
/// on the track; timed ones are.
fn settle(lrclib: FromLrclib, own: Result<Option<SongLyrics>, String>) -> Result<Found, Failure> {
    let plain = match own {
        Ok(Some(SongLyrics::Timed(timed))) => {
            let lines = timed
                .lines
                .into_iter()
                .map(|l| Line {
                    start: l.start,
                    text: l.text,
                })
                .collect();
            return Ok(Found {
                lyrics: Some(Lyrics::Synced(lines)),
                credit: timed.source.unwrap_or_default(),
                keep: true,
            });
        }
        Ok(Some(SongLyrics::Plain(plain))) => Ok(Some(plain)),
        Ok(None) => Ok(None),
        Err(e) => Err(e),
    };
    let lrclib_failed = match lrclib {
        FromLrclib::Plain(text) => {
            return Ok(Found {
                lyrics: Some(Lyrics::Plain(text)),
                credit: lrclib_credit().into(),
                keep: true,
            });
        }
        FromLrclib::Missed => None,
        FromLrclib::Failed(e) => Some(e),
    };
    let keep = lrclib_failed.is_none();
    match plain {
        Ok(Some(plain)) => Ok(Found {
            lyrics: Some(Lyrics::Plain(plain.text)),
            credit: plain.source.unwrap_or_default(),
            keep,
        }),
        Ok(None) => match lrclib_failed {
            Some(e) => Err(Failure::Unavailable(e)),
            None => Ok(Found {
                lyrics: None,
                credit: String::new(),
                keep,
            }),
        },
        Err(e) => Err(Failure::Failed(match lrclib_failed {
            Some(f) => format!("{f}; {e}"),
            None => e,
        })),
    }
}

/// One LRCLIB client for the session.
fn client() -> Option<LrcLib> {
    static CLIENT: OnceLock<Option<LrcLib>> = OnceLock::new();
    CLIENT
        .get_or_init(|| match LrcLib::new() {
            Ok(c) => Some(c),
            Err(e) => {
                log!("lyrics: {e}");
                None
            }
        })
        .clone()
}

/// What LRCLIB is asked: the title, the first artist, the album and the length.
fn query(track: &Track) -> Query {
    Query {
        title: track.title.clone(),
        artist: track
            .artists
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_default(),
        album: track.album.as_ref().map(|a| a.title.clone()),
        duration: track.duration,
    }
}

/// `--fixtures`: the recorded LRCLIB record (placeholder lines), never the network.
/// `FEATHERPLAY_FIXTURE_LYRICS` names another record in LRCLIB's format instead, for
/// measuring with lyrics as long as a real song's (`tests/lyrics_frames.rs`).
pub(super) fn fixture(synced: bool) -> Result<Found, String> {
    const RECORDED: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/lyrics/tests/fixtures/lrclib_get.json"
    );
    let path = std::env::var_os("FEATHERPLAY_FIXTURE_LYRICS")
        .map_or_else(|| RECORDED.into(), std::path::PathBuf::from);
    let text = std::fs::read_to_string(path).map_err(|e| format!("lyrics fixture: {e}"))?;
    let record = lrclib::parse_record(&text).map_err(|e| e.to_string())?;
    if synced {
        return Ok(Found {
            lyrics: record.lyrics(),
            credit: lrclib_credit().into(),
            keep: true,
        });
    }
    Ok(Found {
        lyrics: record.plain_lyrics.map(Lyrics::Plain),
        credit: tr!("Source: LyricFind").into(),
        keep: true,
    })
}

/// `--fixtures`: in the demo set, the song's own lines (or none), as LRCLIB would have them
/// (`synced`) or as the song's plain lyrics; otherwise the recorded record (`fixture`).
pub(super) fn fixture_for(f: &Fixtures, id: &TrackId, synced: bool) -> Result<Found, String> {
    if !f.is_demo() {
        return fixture(synced);
    }
    let Some((lines, credit)) = f.demo_lyrics(id).map_err(|e| e.to_string())? else {
        return Ok(Found {
            lyrics: None,
            credit: String::new(),
            keep: true,
        });
    };
    let lyrics = if synced {
        Lyrics::Synced(lines)
    } else {
        let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        Lyrics::Plain(text.join(
            "
",
        ))
    };
    Ok(Found {
        lyrics: Some(lyrics),
        credit,
        keep: true,
    })
}

#[cfg(test)]
mod tests {
    use featherplay_ytm::{PlainLyrics, TimedLine, TimedLyrics};

    use super::*;

    fn plain() -> Result<Option<SongLyrics>, String> {
        Ok(Some(SongLyrics::Plain(PlainLyrics {
            text: "Placeholder line".into(),
            source: Some("Source: LyricFind".into()),
        })))
    }

    fn timed() -> Result<Option<SongLyrics>, String> {
        let line = |ms, text: &str| TimedLine {
            start: Duration::from_millis(ms),
            text: text.into(),
        };
        Ok(Some(SongLyrics::Timed(TimedLyrics {
            lines: vec![
                line(250, "Placeholder line 1"),
                line(3980, "Placeholder line 2"),
            ],
            source: Some("Source: Musixmatch".into()),
        })))
    }

    #[test]
    fn the_demo_set_should_show_a_songs_own_timed_lines_with_their_credit() {
        let f = Fixtures::load_set(featherplay_controller::fixtures::FixtureSet::Demo)
            .expect("demo fixtures");
        let found = fixture_for(&f, &TrackId("demo-invisible-light".into()), true).expect("found");
        let Some(Lyrics::Synced(lines)) = &found.lyrics else {
            panic!("not synced: {found:?}");
        };
        assert!(lines.len() > 5);
        assert!(found.credit.contains("Josh Woodward"));
    }

    fn failed() -> FromLrclib {
        FromLrclib::Failed("status 503".into())
    }

    #[test]
    fn a_plain_fallback_after_an_lrclib_failure_should_be_shown_but_not_kept() {
        let found = settle(failed(), plain()).expect("plain lyrics");
        assert!(matches!(found.lyrics, Some(Lyrics::Plain(_))));
        assert!(!found.keep, "asked again the next time the tab opens");
    }

    #[test]
    fn a_plain_fallback_after_an_lrclib_miss_should_be_kept() {
        let found = settle(FromLrclib::Missed, plain()).expect("plain lyrics");
        assert!(found.keep);
    }

    #[test]
    fn only_a_plain_fallback_after_an_lrclib_failure_says_synced_lyrics_are_unavailable() {
        let failed = settle(failed(), plain()).expect("plain lyrics");
        assert!(failed.synced_unavailable());
        let missed = settle(FromLrclib::Missed, plain()).expect("plain lyrics");
        assert!(!missed.synced_unavailable(), "LRCLIB answered: not synced");
        let synced = fixture(true).expect("fixture");
        assert!(!synced.synced_unavailable());
    }

    #[test]
    fn an_lrclib_failure_should_fall_back_to_youtube_musics_timed_lyrics_without_the_note() {
        let found = settle(failed(), timed()).expect("timed lyrics");
        let Some(Lyrics::Synced(lines)) = &found.lyrics else {
            panic!("not synced: {found:?}");
        };
        assert_eq!(lines[1].start, Duration::from_millis(3980));
        assert_eq!(lines[1].text, "Placeholder line 2");
        assert_eq!(found.credit, "Source: Musixmatch");
        assert!(found.keep, "kept: LRCLIB coming back has nothing better");
        assert!(
            !found.synced_unavailable(),
            "no outage note over synced lines"
        );
    }

    #[test]
    fn youtube_musics_timed_lyrics_should_come_before_lrclibs_plain_ones() {
        let lrclib_plain = || FromLrclib::Plain("LRCLIB's words".into());
        let found = settle(lrclib_plain(), timed()).expect("timed lyrics");
        assert!(matches!(found.lyrics, Some(Lyrics::Synced(_))));
        // No timed lines: LRCLIB's plain lyrics, as before, even if the song's own failed.
        for own in [plain(), Ok(None), Err("offline".into())] {
            let found = settle(lrclib_plain(), own).expect("LRCLIB's plain lyrics");
            assert_eq!(found.lyrics, Some(Lyrics::Plain("LRCLIB's words".into())));
            assert_eq!(found.credit, lrclib_credit());
            assert!(found.keep);
        }
    }

    /// The track of the recorded LRCLIB record.
    fn track() -> Track {
        use featherplay_core::model::{AlbumId, AlbumRef, ArtistRef, LikeStatus, VideoType};
        Track {
            id: TrackId("t".into()),
            title: "Get Lucky".into(),
            artists: vec![ArtistRef {
                id: None,
                name: "Daft Punk".into(),
            }],
            album: Some(AlbumRef {
                id: AlbumId("a".into()),
                title: "Random Access Memories".into(),
            }),
            duration: Some(Duration::from_secs(367)),
            video_type: VideoType::Song,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            counterpart: None,
        }
    }

    /// A stand-in for lrclib.net on a local port: `get` answers `status` with `body`, `search`
    /// with an empty list. Serves `requests` requests, then stops.
    fn stand_in(status: u16, body: &'static str, requests: usize) -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
        let base = format!("http://{}/api/", listener.local_addr().expect("address"));
        std::thread::spawn(move || {
            for stream in listener.incoming().take(requests) {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                reader.read_line(&mut line).expect("request line");
                let mut header = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    header.clear();
                }
                let (status, body) = if line.starts_with("GET /api/get") {
                    (status, body)
                } else {
                    (200, "[]")
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        base
    }

    const RECORD: &str = include_str!("../../../../crates/lyrics/tests/fixtures/lrclib_get.json");

    #[tokio::test]
    async fn asking_lrclib_again_should_swap_in_synced_lyrics_once_it_answers() {
        let lrclib = LrcLib::with_base(&stand_in(200, RECORD, 1)).expect("client");
        let found = ask_lrclib_again(&lrclib, &track(), Again::Synced)
            .await
            .expect("synced");
        assert!(matches!(found.lyrics, Some(Lyrics::Synced(_))));
        assert!(found.keep && !found.synced_unavailable());
    }

    #[tokio::test]
    async fn asking_lrclib_again_should_change_nothing_while_it_still_fails() {
        let lrclib = LrcLib::with_base(&stand_in(503, "{}", 2)).expect("client");
        assert!(
            ask_lrclib_again(&lrclib, &track(), Again::Synced)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn asking_lrclib_again_should_keep_the_fallback_when_it_has_only_plain_lyrics() {
        let plain_only = Box::leak(
            RECORD
                .replace("\"syncedLyrics\"", "\"unusedSyncedLyrics\"")
                .into_boxed_str(),
        );
        let lrclib = LrcLib::with_base(&stand_in(200, plain_only, 2)).expect("client");
        assert!(
            ask_lrclib_again(&lrclib, &track(), Again::Synced)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn asking_lrclib_again_for_a_song_without_lyrics_should_take_plain_ones_too() {
        let plain_only = Box::leak(
            RECORD
                .replace("\"syncedLyrics\"", "\"unusedSyncedLyrics\"")
                .into_boxed_str(),
        );
        let lrclib = LrcLib::with_base(&stand_in(200, plain_only, 2)).expect("client");
        let found = ask_lrclib_again(&lrclib, &track(), Again::Any)
            .await
            .expect("an answer");
        assert!(matches!(found.lyrics, Some(Lyrics::Plain(_))));
    }

    #[tokio::test]
    async fn asking_lrclib_again_for_a_song_without_lyrics_should_take_none_as_an_answer() {
        let lrclib = LrcLib::with_base(&stand_in(404, "{}", 2)).expect("client");
        let found = ask_lrclib_again(&lrclib, &track(), Again::Any)
            .await
            .expect("an answer");
        assert!(found.lyrics.is_none() && found.keep);
    }

    #[tokio::test]
    async fn asking_lrclib_again_for_a_song_without_lyrics_should_change_nothing_while_it_fails() {
        let lrclib = LrcLib::with_base(&stand_in(503, "{}", 2)).expect("client");
        assert!(
            ask_lrclib_again(&lrclib, &track(), Again::Any)
                .await
                .is_none()
        );
    }

    #[test]
    fn nothing_found_after_an_lrclib_failure_should_be_an_error() {
        assert!(settle(FromLrclib::Failed("timed out".into()), Ok(None)).is_err());
        assert!(settle(FromLrclib::Missed, Ok(None)).is_ok_and(|f| f.lyrics.is_none() && f.keep));
    }

    #[test]
    fn lrclib_down_with_no_lyrics_of_the_songs_own_should_be_unavailable() {
        let failure = settle(failed(), Ok(None)).expect_err("a failure");
        assert!(matches!(failure, Failure::Unavailable(_)), "{failure:?}");
    }

    #[test]
    fn the_songs_own_lyrics_failing_should_be_a_failed_lookup() {
        for lrclib in [FromLrclib::Missed, failed()] {
            let failure = settle(lrclib, Err("offline".into())).expect_err("a failure");
            assert!(matches!(failure, Failure::Failed(_)), "{failure:?}");
        }
    }
}
