//! Looking lyrics up: LRCLIB and the song's own lyrics (`settle` picks), the fixtures in
//! `--fixtures`, and when a track is looked up and asked again; answers land in the cache.

use std::cell::RefCell;
use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::OnceLock;
use std::task::Poll;
use std::time::Duration;

use featherplay_controller::catalog::Source;
use featherplay_controller::fixtures::Fixtures;
use featherplay_core::model::{Track, TrackId, VideoType};
use featherplay_core::tr;
use featherplay_lyrics::lrclib::{self, Lookup, LrcLib, Query};
use featherplay_lyrics::lyrictified::{self, Lyrictified};
use featherplay_lyrics::unison::{self, Unison};
use featherplay_lyrics::{Line, Lyrics, Part, WordLine};
use featherplay_ytm::SongLyrics;
use slint::{ComponentHandle, Timer, TimerMode};
use tokio::sync::oneshot;

use super::{Entry, LOOKUPS, RESYNC, STATE, WAIT, WAITING, fill, show_failure, sync};
use crate::app::Ctx;
use crate::pages::settings;
use crate::ui::LyricsData;

/// The credit under lyrics from LRCLIB.
pub(super) fn lrclib_credit() -> &'static str {
    tr!("Lyrics from LRCLIB")
}

/// A piece of a credit: its text, and the page it links to ("" for none).
pub(super) type Piece = (String, String);

/// Lyrictified's site, where its lyrics can be searched (it has no page per song).
const LYRICTIFIED_SITE: &str = "https://api.lyrictified.xyz/";
/// Unison's pages: a song's lyrics and who timed them.
const UNISON_SITE: &str = "https://unison.betterlyrics.org/";

/// The credit under lyrics from Lyrictified, linking to its site.
fn lyrictified_credit() -> Vec<Piece> {
    vec![(
        tr!("Lyrics from Lyrictified").into(),
        LYRICTIFIED_SITE.into(),
    )]
}

/// The credit under lyrics from Unison, as its terms ask: "Lyrics from Unison" linking to the
/// song's page there, then who timed them, when it says, linking to their page.
pub(super) fn unison_credit(
    video: Option<&str>,
    submitter: Option<&str>,
    curator: Option<&str>,
) -> Vec<Piece> {
    let song = video.and_then(unison::song_page).unwrap_or_default();
    let mut pieces = vec![(tr!("Lyrics from Unison").to_string(), song)];
    if let Some(name) = submitter {
        let by = tr!(", timed by {}");
        let (before, after) = by.split_once("{}").unwrap_or((by, ""));
        let page = curator.and_then(unison::curator_page).unwrap_or_default();
        pieces.push((before.into(), String::new()));
        pieces.push((name.into(), page));
        if !after.is_empty() {
            pieces.push((after.into(), String::new()));
        }
    }
    pieces
}

/// Opens a page a credit links to in the browser, off the UI thread: Unison's or
/// Lyrictified's, nothing else.
pub(super) fn open_page(ctx: &Ctx, url: &str) {
    if !(url.starts_with(UNISON_SITE) || url.starts_with(LYRICTIFIED_SITE)) {
        return;
    }
    let url = url.to_string();
    featherplay_controller::task::blocking_detached(ctx.catalog.runtime(), move || {
        if let Err(e) = featherplay_platform::open::url(&url) {
            log!("lyrics: opening a credit's page: {e}");
        }
    });
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
#[derive(Debug, Default)]
pub struct Found {
    pub(super) lyrics: Option<Lyrics>,
    /// The timing of each synced line's words, one entry per line (empty for a line with
    /// none); empty altogether when the lyrics have no word timing. Made with the lines
    /// (`timed`), so the two always line up.
    pub(super) words: Vec<Vec<Part>>,
    /// Where they come from, shown under them ("" when unknown).
    pub(super) credit: String,
    /// The same credit in pieces when some link to a page (`linked`); empty when none does.
    pub(super) links: Vec<Piece>,
    /// Kept for the session. Not when LRCLIB failed and these are the fallback: the tab
    /// asks again the next time it opens on the track.
    pub(super) keep: bool,
    /// The LRCLIB record they are, when they're LRCLIB's: "Choose other lyrics" marks it.
    pub(super) lrclib_id: Option<u64>,
    /// They came with the song from YouTube Music.
    pub(super) from_song: bool,
    /// The user picked them (or none) in "Choose other lyrics".
    pub(super) picked: bool,
}

impl Found {
    /// Lyrics with their words' timing where they have it, kept for the session. The lines'
    /// text and pieces are moved in, not copied.
    pub(super) fn timed(lines: Vec<WordLine>, credit: String) -> Found {
        let has_words = featherplay_lyrics::words::has_words(&lines);
        let (lines, words): (Vec<Line>, Vec<Vec<Part>>) = lines
            .into_iter()
            .map(|l| {
                (
                    Line {
                        start: l.start,
                        text: l.text,
                    },
                    l.parts,
                )
            })
            .unzip();
        Found {
            words: if has_words { words } else { Vec::new() },
            lyrics: Some(Lyrics::Synced(lines)),
            credit,
            keep: true,
            ..Found::default()
        }
    }

    /// The lyrics with `pieces` for their credit, some linking to a page.
    pub(super) fn linked(self, pieces: Vec<Piece>) -> Found {
        Found {
            credit: pieces.iter().map(|(text, _)| text.as_str()).collect(),
            links: pieces,
            ..self
        }
    }

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
                id,
                ..
            },
            _,
        )
        | (Lookup::Found { lyrics, id, .. }, Again::Any) => Some(Found {
            lyrics: Some(lyrics),
            credit: lrclib_credit().into(),
            keep: true,
            lrclib_id: id,
            ..Found::default()
        }),
        (_, Again::Synced) => None,
        (_, Again::Any) => Some(Found {
            keep: true,
            ..Found::default()
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
    WAITING.with(|w| *w.borrow_mut() = Some(crate::busy::Busy::on_timers()));
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
                fill(ctx, &ui.global::<LyricsData>(), &id, &found, true);
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
    stop_late();
    // Picked in "Choose other lyrics" and kept on this PC: shown without asking anyone.
    if let Some(found) = super::choose::picked(&track.id) {
        let found = Rc::new(found);
        let shown = STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.cache
                .insert(track.id.clone(), Entry::Ready(found.clone()));
            s.shown.as_ref() == Some(&track.id)
        });
        if shown && let Some(ui) = ctx.window() {
            fill(ctx, &ui.global::<LyricsData>(), &track.id, &found, false);
            sync(ctx);
        }
        return;
    }
    let use_lrclib = settings::lyrics_lrclib();
    let word_timed = settings::lyrics_word_timed();
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
    let (stop, stopped) = oneshot::channel::<()>();
    LATE.with(|l| *l.borrow_mut() = Some((id.clone(), stop)));
    let mut first = true;
    ctx.stream(
        move |source, answers| {
            let stopped = async move {
                let _ = stopped.await;
            };
            look_up(source, use_lrclib, word_timed, track, stopped, move |a| {
                answers.send(a).is_ok()
            })
        },
        move |ctx, ui, answer| {
            if STATE.with(|s| s.borrow().generation != generation) {
                return ControlFlow::Break(());
            }
            if std::mem::take(&mut first) {
                first_answer(ctx, ui, &id, answer);
                ControlFlow::Continue(())
            } else {
                better_answer(ctx, ui, &id, answer)
            }
        },
    );
}

thread_local! {
    /// The track a lookup is for, and what stops it: once its first answer shows, it may
    /// still be waiting on a slower, preferred source (`as_they_come`).
    static LATE: RefCell<Option<(TrackId, oneshot::Sender<()>)>> = const { RefCell::new(None) };
}

/// `current` plays now: a lookup still waiting on a better answer for another track stops.
pub(super) fn playing(current: Option<&TrackId>) {
    LATE.with(|l| {
        let mut l = l.borrow_mut();
        if l.as_ref().is_some_and(|(id, _)| Some(id) != current) {
            *l = None;
        }
    });
}

/// A lookup still waiting on a better answer stops (the settings changed, or the track is
/// looked up anew).
pub(super) fn stop_late() {
    LATE.with(|l| l.borrow_mut().take());
}

/// A lookup's first answer for `id`: kept (or, failed, asked again later) and shown if the
/// view shows the track.
fn first_answer(ctx: &Rc<Ctx>, ui: &crate::ui::AppWindow, id: &TrackId, answer: Answer) {
    let found = answer.map(Rc::new);
    if let Err(e) = &found {
        log!("lyrics for {}: {e}", id.0);
    }
    let shown = STATE.with(|s| {
        let mut s = s.borrow_mut();
        match &found {
            Ok(f) if f.keep => {
                s.retry.succeeded(id);
                s.cache.insert(id.clone(), Entry::Ready(f.clone()))
            }
            // Asked again the next time the view opens on this track, or on Try again.
            _ => {
                s.retry.failed(id.clone());
                s.cache.remove(id)
            }
        };
        s.shown.as_ref() == Some(id)
    });
    if shown {
        let data = ui.global::<LyricsData>();
        match found {
            Ok(f) => {
                fill(ctx, &data, id, &f, false);
                if f.synced_unavailable() {
                    resync_later(ctx, id, Again::Synced);
                }
            }
            Err(failure) => show_failure(ctx, &data, id, &failure),
        }
        sync(ctx);
    }
}

/// A better answer for `id` than the one shown came later (`as_they_come`): while the track
/// plays, it's kept in the first one's place, and swapped in where the view shows the track,
/// on the line playing. Lyrics picked in "Choose other lyrics" meanwhile stay.
fn better_answer(
    ctx: &Rc<Ctx>,
    ui: &crate::ui::AppWindow,
    id: &TrackId,
    answer: Answer,
) -> ControlFlow<()> {
    let playing = ctx
        .player()
        .and_then(|s| s.current)
        .is_some_and(|t| t.id == *id);
    let Ok(found) = answer.map(Rc::new) else {
        return ControlFlow::Continue(());
    };
    let shown = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let picked = matches!(s.cache.get(id), Some(Entry::Ready(f)) if f.picked);
        if !playing || picked {
            return None;
        }
        s.retry.succeeded(id);
        s.cache.insert(id.clone(), Entry::Ready(found.clone()));
        Some(s.shown.as_ref() == Some(id))
    });
    match shown {
        None => return ControlFlow::Break(()),
        Some(true) => {
            // LRCLIB asked again after an outage would have nothing better now.
            RESYNC.with(Timer::stop);
            fill(ctx, &ui.global::<LyricsData>(), id, &found, true);
            sync(ctx);
        }
        Some(false) => {}
    }
    ControlFlow::Continue(())
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

/// How long Lyrictified has for both its requests (the search and the file). Long: a search
/// it has to resolve from other sources first takes it 7–10 s, and the lyrics found sooner
/// show meanwhile (`as_they_come`).
const LYRICTIFIED_DEADLINE: Duration = Duration::from_secs(15);

/// Lyrictified's lyrics for the track, timed by the word or the line, as its file is. It gets
/// the title and nothing else (the artist is matched here): a video's without its video tags,
/// as shown, and the lookup leaves out a guest credit. A title it hasn't seen answers
/// nothing while it looks the song up, so nothing found is the usual answer; a failed or slow
/// request is a miss too: Lyrictified is an extra, never a reason for no lyrics.
async fn from_lyrictified(track: &Track) -> Option<Found> {
    let client = lyrictified_client()?;
    let title = &track.title;
    let query = lyrictified::Query {
        song: track.display_title().into_owned(),
        artists: track.artists.iter().map(|a| a.name.clone()).collect(),
    };
    match tokio::time::timeout(LYRICTIFIED_DEADLINE, client.find(&query)).await {
        Ok(Ok(lyrictified::Lookup::Found { lines, format, .. })) => {
            log!(
                "lyrics {title:?}: Lyrictified, {format}, {} lines",
                lines.len()
            );
            Some(Found::timed(lines, String::new()).linked(lyrictified_credit()))
        }
        Ok(Ok(lyrictified::Lookup::Missed)) => {
            log!("lyrics {title:?}: Lyrictified, no match");
            None
        }
        Ok(Err(e)) => {
            log!("lyrics {title:?}: {e}");
            None
        }
        Err(_) => {
            log!("lyrics {title:?}: Lyrictified, no answer in time");
            None
        }
    }
}

/// How long Unison has for both its requests (by the video's ID, then by the song): each may
/// take its 6 s. The lyrics found sooner show meanwhile (`as_they_come`).
const UNISON_DEADLINE: Duration = Duration::from_secs(12);

/// Unison's lyrics for the track when they're timed by the word, asked by the video's ID; for
/// a music video without word-timed lyrics there (nothing filed under it, or lines timed only
/// by line), once more by the song (`unison_song`). Nothing found, a failed request, or no
/// answer within `UNISON_DEADLINE` is a miss: Unison is an extra, never a reason for no
/// lyrics.
async fn from_unison(track: &Track) -> Option<Found> {
    ask_unison(&unison_client()?, track, UNISON_DEADLINE).await
}

/// `from_unison` with `client`, given `deadline` for its requests.
async fn ask_unison(client: &Unison, track: &Track, deadline: Duration) -> Option<Found> {
    let title = &track.title;
    let song = unison_song(track);
    let asked = client.find_track(&track.id.0, song.as_ref());
    let Ok(answer) = tokio::time::timeout(deadline, asked).await else {
        log!("lyrics {title:?}: Unison, no answer in time");
        return None;
    };
    match answer {
        Ok(unison::Lookup::Words {
            lines,
            submitter,
            curator,
            video,
            ..
        }) => {
            log!("lyrics {title:?}: Unison, {} lines", lines.len());
            let video = video.as_deref().or(Some(track.id.0.as_str()));
            let credit = unison_credit(video, submitter.as_deref(), curator.as_deref());
            Some(Found::timed(lines, String::new()).linked(credit))
        }
        Ok(unison::Lookup::Missed) => {
            log!("lyrics {title:?}: Unison, no word-timed lyrics");
            None
        }
        Err(e) => {
            log!("lyrics {title:?}: {e}");
            None
        }
    }
}

/// What Unison is asked for a music video without word-timed lyrics under its ID: the song by
/// its title (without the video's tags), main artist, album and length. `None` for anything else: a
/// song's lyrics are filed under its own ID.
fn unison_song(track: &Track) -> Option<unison::SongQuery> {
    if track.video_type != VideoType::MusicVideo {
        return None;
    }
    Some(unison::SongQuery {
        song: featherplay_core::scrobble::clean_title(&track.title, track.video_type),
        artist: track.artists.first()?.name.clone(),
        album: track.album.as_ref().map(|a| a.title.clone()),
        duration: track.duration,
    })
}

/// One client for the session, made the first time it's asked for; `None` (logged) if it
/// can't be made.
fn once<C: Clone, E: std::fmt::Display>(
    cell: &OnceLock<Option<C>>,
    make: impl FnOnce() -> Result<C, E>,
) -> Option<C> {
    cell.get_or_init(|| {
        make().map_or_else(
            |e| {
                log!("lyrics: {e}");
                None
            },
            Some,
        )
    })
    .clone()
}

fn lyrictified_client() -> Option<Lyrictified> {
    static CLIENT: OnceLock<Option<Lyrictified>> = OnceLock::new();
    once(&CLIENT, Lyrictified::new)
}

fn unison_client() -> Option<Unison> {
    static CLIENT: OnceLock<Option<Unison>> = OnceLock::new();
    once(&CLIENT, Unison::new)
}

/// A source's answer on its way.
type Asking<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Asks every source at once and hands `show` the first answer that comes, whichever source
/// it's from (`None` is a miss), then each later one from a source before the one shown in
/// `sources`' order (their order is the preference), unless it would lose the timing by the
/// word (`timed_by_word`) that shows: a late answer timed by the line never replaces one
/// timed by the word. So a slow source holds nothing up. A source that can no longer be shown
/// is dropped, and its request with it; all of them are once `show` says nobody takes its
/// answers, or once something shows and `stopped` comes (another track plays).
async fn as_they_come<T>(
    sources: Vec<Asking<'_, Option<T>>>,
    timed_by_word: impl Fn(&T) -> bool,
    stopped: impl Future<Output = ()>,
    mut show: impl FnMut(T) -> bool,
) {
    let mut asking: Vec<Option<Asking<'_, Option<T>>>> = sources.into_iter().map(Some).collect();
    // Where the answer shown stands in the order, and whether it's timed by the word.
    let mut shown: Option<(usize, bool)> = None;
    let mut stopped = std::pin::pin!(stopped);
    while asking.iter().any(Option::is_some) {
        let answer = std::future::poll_fn(|cx| {
            if shown.is_some() && stopped.as_mut().poll(cx).is_ready() {
                return Poll::Ready(None);
            }
            for (rank, slot) in asking.iter_mut().enumerate() {
                if let Some(f) = slot
                    && let Poll::Ready(a) = f.as_mut().poll(cx)
                {
                    *slot = None;
                    return Poll::Ready(Some((rank, a)));
                }
            }
            Poll::Pending
        })
        .await;
        let Some((rank, answer)) = answer else { return };
        let Some(answer) = answer else { continue };
        let words = timed_by_word(&answer);
        if shown.is_none_or(|(at, had_words)| rank < at && (words || !had_words)) {
            if !show(answer) {
                return;
            }
            shown = Some((rank, words));
            // Those after it can't take its place any more.
            asking[rank..].fill_with(|| None);
        }
    }
}

/// LRCLIB's synced lyrics for the track, or what it said short of them.
async fn from_lrclib(use_lrclib: bool, track: &Track) -> Result<Found, FromLrclib> {
    if !use_lrclib {
        return Err(FromLrclib::Missed);
    }
    let title = &track.title;
    let Some(client) = client() else {
        return Err(FromLrclib::Failed("no LRCLIB client".into()));
    };
    match client.find(&query(track)).await {
        Ok(lookup) => {
            log!("lyrics {title:?}: LRCLIB {lookup}");
            match lookup {
                Lookup::Found {
                    lyrics: lyrics @ Lyrics::Synced(_),
                    id,
                    ..
                } => Ok(Found {
                    lyrics: Some(lyrics),
                    credit: lrclib_credit().into(),
                    keep: true,
                    lrclib_id: id,
                    ..Found::default()
                }),
                Lookup::Found {
                    lyrics: Lyrics::Plain(text),
                    id,
                    ..
                } => Err(FromLrclib::Plain(text, id)),
                _ => Err(FromLrclib::Missed),
            }
        }
        Err(e) => {
            log!("lyrics {title:?}: {e}");
            Err(FromLrclib::Failed(e.to_string()))
        }
    }
}

/// What a lookup hands the view, as it comes (`look_up`): an error only when nothing was
/// found and a lookup failed.
type Answer = Result<Found, Failure>;

/// With lyrics timed by the word on (`word_timed`), Lyrictified, then Unison; then LRCLIB's
/// synced lyrics when allowed, or short of them the song's own lyrics (`settle` picks). All
/// are asked at once and preferred in that order: `show` gets the first answer, then a better
/// one if a source before it comes later (`as_they_come`), until `stopped`. With it off,
/// neither Lyrictified nor Unison is contacted. Logs what each found, by the track's title.
async fn look_up(
    source: Source,
    use_lrclib: bool,
    word_timed: bool,
    track: Track,
    stopped: impl Future<Output = ()>,
    mut show: impl FnMut(Answer) -> bool,
) {
    if let Source::Fixtures(f) = &source {
        show(fixture_for(f, &track.id, use_lrclib).map_err(Failure::Failed));
        return;
    }
    let mut sources: Vec<Asking<'_, Option<Answer>>> = Vec::new();
    if word_timed {
        sources.push(Box::pin(async { from_lyrictified(&track).await.map(Ok) }));
        sources.push(Box::pin(async { from_unison(&track).await.map(Ok) }));
    }
    sources.push(Box::pin(async {
        Some(lrclib_or_song(&source, use_lrclib, &track).await)
    }));
    let timed_by_word = |a: &Answer| a.as_ref().is_ok_and(|f| !f.words.is_empty());
    as_they_come(sources, timed_by_word, stopped, show).await;
}

/// LRCLIB's synced lyrics when allowed; short of them, the song's own lyrics (`settle` picks).
async fn lrclib_or_song(source: &Source, use_lrclib: bool, track: &Track) -> Answer {
    let lrclib = match from_lrclib(use_lrclib, track).await {
        Ok(found) => return Ok(found),
        Err(lrclib) => lrclib,
    };
    let title = &track.title;
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
    /// Plain lyrics only, and the record's id.
    Plain(String, Option<u64>),
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
                from_song: true,
                ..Found::default()
            });
        }
        Ok(Some(SongLyrics::Plain(plain))) => Ok(Some(plain)),
        Ok(None) => Ok(None),
        Err(e) => Err(e),
    };
    let lrclib_failed = match lrclib {
        FromLrclib::Plain(text, id) => {
            return Ok(Found {
                lyrics: Some(Lyrics::Plain(text)),
                credit: lrclib_credit().into(),
                keep: true,
                lrclib_id: id,
                ..Found::default()
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
            from_song: true,
            ..Found::default()
        }),
        Ok(None) => match lrclib_failed {
            Some(e) => Err(Failure::Unavailable(e)),
            None => Ok(Found {
                keep,
                ..Found::default()
            }),
        },
        Err(e) => Err(Failure::Failed(match lrclib_failed {
            Some(f) => format!("{f}; {e}"),
            None => e,
        })),
    }
}

/// One LRCLIB client for the session.
pub(super) fn client() -> Option<LrcLib> {
    static CLIENT: OnceLock<Option<LrcLib>> = OnceLock::new();
    once(&CLIENT, LrcLib::new)
}

/// What LRCLIB is asked: the title, the first artist, the album and the length.
pub(super) fn query(track: &Track) -> Query {
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
    let record = read_fixture_record()?;
    if synced {
        return Ok(Found {
            lyrics: record.lyrics(),
            credit: lrclib_credit().into(),
            keep: true,
            lrclib_id: record.id,
            ..Found::default()
        });
    }
    Ok(Found {
        lyrics: record.plain_lyrics.map(Lyrics::Plain),
        credit: tr!("Source: LyricFind").into(),
        keep: true,
        from_song: true,
        ..Found::default()
    })
}

/// The recorded LRCLIB record, or `FEATHERPLAY_FIXTURE_LYRICS`'s.
fn read_fixture_record() -> Result<lrclib::Record, String> {
    const RECORDED: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/lyrics/tests/fixtures/lrclib_get.json"
    );
    let path = std::env::var_os("FEATHERPLAY_FIXTURE_LYRICS")
        .map_or_else(|| RECORDED.into(), std::path::PathBuf::from);
    let text = std::fs::read_to_string(path).map_err(|e| format!("lyrics fixture: {e}"))?;
    lrclib::parse_record(&text).map_err(|e| e.to_string())
}

/// `--fixtures` outside the demo set: the recorded record, as "Choose other lyrics" lists it.
pub(super) fn fixture_record() -> Option<lrclib::Record> {
    read_fixture_record().ok()
}

/// `--fixtures`: in the demo set, the song's own lines (or none), as LRCLIB would have them
/// (`synced`) or as the song's plain lyrics; otherwise the recorded record (`fixture`).
pub(super) fn fixture_for(f: &Fixtures, id: &TrackId, synced: bool) -> Result<Found, String> {
    if !f.is_demo() {
        return fixture(synced);
    }
    let Some((lines, credit)) = f.demo_lyrics(id).map_err(|e| e.to_string())? else {
        return Ok(Found {
            keep: true,
            ..Found::default()
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
        // As LRCLIB's record would be, or as the song's own plain lyrics.
        lrclib_id: synced.then_some(DEMO_RECORD),
        from_song: !synced,
        ..Found::default()
    })
}

/// `--fixtures`: the demo song's own lines stand for this LRCLIB record ("Choose other
/// lyrics" marks it in use).
pub(super) const DEMO_RECORD: u64 = 1;

#[cfg(test)]
mod tests {
    use featherplay_ytm::{PlainLyrics, TimedLine, TimedLyrics};

    use super::*;

    #[test]
    fn unisons_credit_should_link_the_song_and_who_timed_it_as_its_terms_ask() {
        let found = Found::default().linked(unison_credit(
            Some("T0Ry1m0Blug"),
            Some("A Tester"),
            Some("0a1b"),
        ));
        assert_eq!(found.credit, "Lyrics from Unison, timed by A Tester");
        let links: Vec<&str> = found.links.iter().map(|(_, url)| url.as_str()).collect();
        assert_eq!(
            links,
            [
                "https://unison.betterlyrics.org/song/T0Ry1m0Blug",
                "",
                "https://unison.betterlyrics.org/curator/0a1b"
            ]
        );
        // Nobody named: the service alone; nothing that could leave the site is linked.
        let alone = unison_credit(Some("../x"), None, None);
        assert_eq!(alone, [("Lyrics from Unison".to_string(), String::new())]);
    }

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

    /// A source that answers `answer` after `ms` milliseconds.
    fn after<T: Send + 'static>(ms: u64, answer: T) -> Asking<'static, T> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            answer
        })
    }

    /// A source that never answers (its timeout is the caller's).
    fn hangs<T: Send + 'static>() -> Asking<'static, T> {
        Box::pin(std::future::pending())
    }

    /// An answer in these tests: whose it is, and whether it's timed by the word.
    type Said = (&'static str, bool);

    /// What `as_they_come` shows of `sources`, in order, and when, on tokio's paused clock;
    /// and when it ended.
    async fn shown_until(
        sources: Vec<Asking<'static, Option<Said>>>,
        stopped: impl Future<Output = ()>,
        mut on_show: impl FnMut(),
    ) -> (Vec<(&'static str, Duration)>, Duration) {
        let start = tokio::time::Instant::now();
        let mut shown = Vec::new();
        let asked = as_they_come(
            sources,
            |s: &Said| s.1,
            stopped,
            |s| {
                shown.push((s.0, start.elapsed()));
                on_show();
                true
            },
        );
        tokio::time::timeout(Duration::from_secs(60), asked)
            .await
            .expect("ended");
        (shown, start.elapsed())
    }

    async fn shown(sources: Vec<Asking<'static, Option<Said>>>) -> Vec<(&'static str, Duration)> {
        shown_until(sources, std::future::pending(), || {}).await.0
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_answer_should_show_and_one_preferred_take_its_place_later() {
        // The second answers first and shows; the first, preferred, takes its place.
        let got = shown(vec![
            after(30, Some(("first", false))),
            after(0, Some(("second", false))),
            hangs(),
        ])
        .await;
        assert_eq!(got, [("second", ms(0)), ("first", ms(30))]);
        // The first misses: the second's stays, and the last one isn't waited for.
        let got = shown(vec![
            after(10, None),
            after(0, Some(("second", false))),
            hangs(),
        ])
        .await;
        assert_eq!(got, [("second", ms(0))]);
        // Every one before it misses: the last source's answer.
        let got = shown(vec![
            after(0, None),
            after(5, None),
            after(10, Some(("last", false))),
        ])
        .await;
        assert_eq!(got, [("last", ms(10))]);
        // With nothing before it (lyrics timed by the word off), just the last source.
        assert_eq!(
            shown(vec![after(0, Some(("last", false)))]).await,
            [("last", ms(0))]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_source_that_hangs_should_hold_up_nothing_and_be_dropped_at_its_timeout() {
        let timed_out: Asking<'static, Option<Said>> = Box::pin(async {
            tokio::time::timeout(ms(40), std::future::pending::<Option<Said>>())
                .await
                .ok()
                .flatten()
        });
        let sources = vec![timed_out, after(0, Some(("second", false))), hangs()];
        let (got, ended) = shown_until(sources, std::future::pending(), || {}).await;
        assert_eq!(got, [("second", ms(0))]);
        assert_eq!(
            ended,
            ms(40),
            "waits for the preferred one until its timeout"
        );
        // The first in the order shows: the one after it, hanging, is dropped at once.
        let sources = vec![after(0, Some(("first", false))), hangs()];
        let (_, ended) = shown_until(sources, std::future::pending(), || {}).await;
        assert_eq!(ended, ms(0));
    }

    #[tokio::test(start_paused = true)]
    async fn lrclib_should_show_first_and_lyrictified_timed_by_the_word_swap_in_later() {
        // (Lyrictified, Unison, LRCLIB): LRCLIB at 100 ms, Unison misses, Lyrictified at 8 s.
        let got = shown(vec![
            after(8000, Some(("lyrictified", true))),
            after(300, None),
            after(100, Some(("lrclib", false))),
        ])
        .await;
        assert_eq!(got, [("lrclib", ms(100)), ("lyrictified", ms(8000))]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_answer_timed_by_the_line_should_never_replace_one_timed_by_the_word() {
        let got = shown(vec![
            after(8000, Some(("lyrictified", false))),
            after(300, Some(("unison", true))),
            after(100, Some(("lrclib", false))),
        ])
        .await;
        assert_eq!(got, [("lrclib", ms(100)), ("unison", ms(300))]);
        // Timed by the word too, the preferred one does.
        let got = shown(vec![
            after(8000, Some(("lyrictified", true))),
            after(300, Some(("unison", true))),
            after(100, Some(("lrclib", false))),
        ])
        .await;
        let who: Vec<&str> = got.iter().map(|(w, _)| *w).collect();
        assert_eq!(who, ["lrclib", "unison", "lyrictified"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_track_change_should_stop_the_late_ask_but_not_the_first_answer() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let answered = Arc::new(AtomicBool::new(false));
        let lyrictified = || -> Asking<'static, Option<Said>> {
            let answered = answered.clone();
            Box::pin(async move {
                tokio::time::sleep(ms(8000)).await;
                answered.store(true, Ordering::SeqCst);
                Some(("lyrictified", true))
            })
        };
        // The track changes as LRCLIB's answer shows: Lyrictified is dropped, unanswered.
        let (stop, stopped) = oneshot::channel::<()>();
        let mut stop = Some(stop);
        let stopped = async move {
            let _ = stopped.await;
        };
        let sources = vec![lyrictified(), after(100, Some(("lrclib", false)))];
        let (got, ended) = shown_until(sources, stopped, || drop(stop.take())).await;
        assert_eq!(got, [("lrclib", ms(100))]);
        assert_eq!(ended, ms(100));
        assert!(!answered.load(Ordering::SeqCst));
        // Stopped before any answer: the first still shows (it's kept for the track).
        let (stop, stopped) = oneshot::channel::<()>();
        drop(stop);
        let stopped = async move {
            let _ = stopped.await;
        };
        let sources = vec![lyrictified(), after(100, Some(("lrclib", false)))];
        let (got, _) = shown_until(sources, stopped, || {}).await;
        assert_eq!(got, [("lrclib", ms(100))]);
        // What stops it: another track playing, not the same one.
        let (stop, mut stopped) = oneshot::channel::<()>();
        LATE.with(|l| *l.borrow_mut() = Some((TrackId("a".into()), stop)));
        playing(Some(&TrackId("a".into())));
        assert!(
            stopped
                .try_recv()
                .is_err_and(|e| e == oneshot::error::TryRecvError::Empty)
        );
        playing(Some(&TrackId("b".into())));
        assert!(
            stopped
                .try_recv()
                .is_err_and(|e| e == oneshot::error::TryRecvError::Closed)
        );
    }

    #[test]
    fn only_a_plain_fallback_after_an_lrclib_failure_says_synced_lyrics_are_unavailable() {
        // Shown, not kept: asked again the next time the tab opens.
        let failed = settle(failed(), plain()).expect("plain lyrics");
        assert!(failed.synced_unavailable());
        let missed = settle(FromLrclib::Missed, plain()).expect("plain lyrics");
        assert!(!missed.synced_unavailable(), "LRCLIB answered: not synced");
        assert!(missed.keep, "LRCLIB answered: kept");
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
        let lrclib_plain = || FromLrclib::Plain("LRCLIB's words".into(), None);
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
        use featherplay_core::model::{AlbumId, AlbumRef, ArtistRef};
        Track {
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
            ..crate::testing::track("t")
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

    /// Unison hanging: its lookup ends at its deadline as a miss, not at its requests' own
    /// timeouts (6 s each, two for a music video).
    #[tokio::test]
    async fn unison_hanging_should_be_a_miss_at_its_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
        let base = format!("http://{}/", listener.local_addr().expect("address"));
        // Takes the connections and never answers.
        std::thread::spawn(move || {
            let held: Vec<_> = listener.incoming().take(2).collect();
            std::thread::sleep(Duration::from_secs(15));
            drop(held);
        });
        let client = Unison::with_base(&base).expect("client");
        let video = Track {
            video_type: VideoType::MusicVideo,
            ..track()
        };
        let started = std::time::Instant::now();
        let found = ask_unison(&client, &video, Duration::from_millis(300)).await;
        assert!(found.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
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
        for again in [Again::Synced, Again::Any] {
            let lrclib = LrcLib::with_base(&stand_in(503, "{}", 2)).expect("client");
            let found = ask_lrclib_again(&lrclib, &track(), again).await;
            assert!(found.is_none(), "{again:?}: {found:?}");
        }
    }

    #[tokio::test]
    async fn plain_lyrics_asked_again_keep_the_fallback_or_fill_a_song_without_lyrics() {
        let plain_only = Box::leak(
            RECORD
                .replace("\"syncedLyrics\"", "\"unusedSyncedLyrics\"")
                .into_boxed_str(),
        );
        let lrclib = LrcLib::with_base(&stand_in(200, plain_only, 4)).expect("client");
        let synced = ask_lrclib_again(&lrclib, &track(), Again::Synced).await;
        assert!(synced.is_none(), "the fallback stays: {synced:?}");
        let found = ask_lrclib_again(&lrclib, &track(), Again::Any)
            .await
            .expect("an answer");
        assert!(
            matches!(found.lyrics, Some(Lyrics::Plain(_))),
            "a song without lyrics takes plain ones: {found:?}"
        );
    }

    #[tokio::test]
    async fn asking_lrclib_again_for_a_song_without_lyrics_should_take_none_as_an_answer() {
        let lrclib = LrcLib::with_base(&stand_in(404, "{}", 2)).expect("client");
        let found = ask_lrclib_again(&lrclib, &track(), Again::Any)
            .await
            .expect("an answer");
        assert!(found.lyrics.is_none() && found.keep);
    }

    #[test]
    fn no_lyrics_settle_as_none_unavailable_or_a_failed_lookup() {
        use FromLrclib::Missed;
        let timed_out = || FromLrclib::Failed("timed out".into());
        let offline = || Err("offline".into());
        // (case: LRCLIB, then the song's own lyrics; from LRCLIB, the song's own, want)
        let cases = [
            ("missed, none own", Missed, Ok(None), "none, kept"),
            ("failed, none own", failed(), Ok(None), "unavailable"),
            ("timed out, none own", timed_out(), Ok(None), "unavailable"),
            ("missed, own failed", Missed, offline(), "failed"),
            ("failed, own failed", failed(), offline(), "failed"),
        ];
        for (case, lrclib, own, want) in cases {
            let got = match settle(lrclib, own) {
                Ok(f) if f.lyrics.is_none() && f.keep => "none, kept",
                Ok(_) => "lyrics, or not kept",
                Err(Failure::Unavailable(_)) => "unavailable",
                Err(Failure::Failed(_)) => "failed",
            };
            assert_eq!(got, want, "{case}");
        }
    }
}
