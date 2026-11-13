//! "Choose other lyrics" (`LyricsChoose`, ui/lyrics_menu.slint) and what is kept per song.
//!
//! The dialog lists LRCLIB's matches for the song: one `/api/search` when it opens, filled
//! with the song's title and artist, and one more per Enter, never as the user types. The
//! lyrics shown are marked "In use"; when they came with the song from YouTube Music they're
//! the first row. A click uses a match at once; "None of these" turns lyrics off for the
//! song; "Back to the automatic match" forgets the pick.
//!
//! What the user chose (a pick, and the timing nudge) is kept on this PC by video id
//! (`featherplay_store::LyricsChoices`, read the first time lyrics show), the picked
//! record's words with it, so a picked song shows its lyrics without asking anyone. Nothing
//! is sent anywhere but the search.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use featherplay_controller::catalog::Source;
use featherplay_controller::fixtures::Fixtures;
use featherplay_core::model::{Track, TrackId};
use featherplay_core::tr;
use featherplay_lyrics::lrclib::Record;
use featherplay_lyrics::{Line, Lyrics};
use featherplay_store::{LyricsChoice, LyricsChoices, LyricsPick, Paths};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::lookup::{DEMO_RECORD, client, lrclib_credit};
use super::{Entry, Found, SNAPSHOT, STATE, show};
use crate::app::Ctx;
use crate::pages::settings;
use crate::ui::{AppWindow, ChooseState, LyricsChoose, LyricsData, LyricsMatch, LyricsState};
use crate::ui_bridge::format_duration;

thread_local! {
    /// The songs' choices, read from disk the first time they're needed.
    static CHOICES: RefCell<Option<LyricsChoices>> = const { RefCell::new(None) };
    /// What the dialog's rows are: an LRCLIB record, or (`None`) the lyrics that came with
    /// the song.
    static ROWS: RefCell<Vec<Option<Record>>> = const { RefCell::new(Vec::new()) };
    /// Counts the searches, so the answer to an older one is dropped.
    static SEARCHES: Cell<u64> = const { Cell::new(0) };
}

/// Once at startup: connect the dialog.
pub(super) fn init(ctx: &Rc<Ctx>, ui: &AppWindow) {
    let c = ctx.clone();
    ui.global::<LyricsData>().on_choose(move || open(&c));
    let choose = ui.global::<LyricsChoose>();
    let c = ctx.clone();
    choose.on_search(move |text| search(&c, text.trim()));
    let c = ctx.clone();
    choose.on_pick(move |i| pick_row(&c, i));
    let c = ctx.clone();
    choose.on_pick_none(move || set_pick(&c, Some(LyricsPick::None)));
    let c = ctx.clone();
    choose.on_automatic(move || set_pick(&c, None));
}

fn with_choices<T>(f: impl FnOnce(&mut LyricsChoices) -> T) -> T {
    CHOICES.with(|c| {
        let mut c = c.borrow_mut();
        f(c.get_or_insert_with(|| LyricsChoices::load(&Paths::resolve())))
    })
}

/// How much later `id`'s lines show, in milliseconds (negative: sooner).
pub(super) fn nudge(id: &TrackId) -> i32 {
    with_choices(|c| c.get(&id.0).nudge_ms)
}

pub(super) fn set_nudge(id: &TrackId, nudge_ms: i32) {
    update(id, |c| c.nudge_ms = nudge_ms);
}

/// Whether any song has choices kept: those in memory once read, else whether the file is
/// there (it goes once it holds none).
pub(super) fn any() -> bool {
    CHOICES.with(|c| match &*c.borrow() {
        Some(choices) => !choices.is_empty(),
        None => LyricsChoices::saved(&Paths::resolve()),
    })
}

/// Forgets every song's choices, on disk too.
pub(super) fn forget() {
    let copy = with_choices(|c| {
        c.clear();
        c.clone()
    });
    save(copy);
}

/// Changes `id`'s choices and saves them.
fn update(id: &TrackId, change: impl FnOnce(&mut LyricsChoice)) {
    let copy = with_choices(|c| {
        let mut choice = c.get(&id.0);
        change(&mut choice);
        c.set(&id.0, choice);
        c.clone()
    });
    save(copy);
}

/// Saves `copy` off the UI thread, after the saves before it, so quick changes (Timing +
/// twice) land newest last (not for a snapshot); Settings' reset link follows.
fn save(copy: LyricsChoices) {
    if let Some(ctx) = Ctx::get() {
        settings::show_lyrics_resettable(&ctx);
    }
    if SNAPSHOT.with(Cell::get) {
        return;
    }
    featherplay_controller::task::write_in_order(move || {
        if let Err(e) = copy.save() {
            log!("lyrics choices: {e}");
        }
    });
}

/// What was picked for `id`, as the lyrics to show; `None` for the automatic match. A
/// record picked on LRCLIB counts only while LRCLIB is on in Settings, and only while its
/// kept words read; "none" always does.
pub(super) fn picked(id: &TrackId) -> Option<Found> {
    match with_choices(|c| c.get(&id.0).pick)? {
        LyricsPick::None => Some(Found {
            keep: true,
            picked: true,
            ..Found::default()
        }),
        LyricsPick::Lrclib { id, synced, plain } if settings::lyrics_lrclib() => {
            let record = Record {
                id: Some(id),
                synced_lyrics: synced,
                plain_lyrics: plain,
                ..Record::default()
            };
            // Kept words that no longer read (a hand-edited file): the automatic match.
            let lyrics = record.lyrics()?;
            Some(Found {
                lyrics: Some(lyrics),
                credit: lrclib_credit().into(),
                keep: true,
                lrclib_id: Some(id),
                picked: true,
                ..Found::default()
            })
        }
        LyricsPick::Lrclib { .. } => None,
    }
}

/// What the shown track's lyrics are, once known.
fn shown_found(id: &TrackId) -> Option<Rc<Found>> {
    STATE.with(|s| match s.borrow().cache.get(id) {
        Some(Entry::Ready(found)) => Some(found.clone()),
        _ => None,
    })
}

/// "Choose other lyrics…": the dialog opens on the playing song, searching LRCLIB for its
/// title and artist.
fn open(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let Some(track) = ctx.player().and_then(|s| s.current) else {
        return;
    };
    let choose = ui.global::<LyricsChoose>();
    let artist = track.artists.first().map_or("", |a| a.name.as_str());
    let query = format!("{} {artist}", track.title).trim().to_string();
    choose.set_query(query.as_str().into());
    choose.set_length(
        track
            .duration
            .map(format_duration)
            .unwrap_or_default()
            .into(),
    );
    choose.set_picked(shown_found(&track.id).is_some_and(|f| f.picked));
    choose.set_open(true);
    search(ctx, &query);
}

/// Asks LRCLIB for `text` (Enter in the dialog's field), and lists what it answers.
fn search(ctx: &Rc<Ctx>, text: &str) {
    let Some(ui) = ctx.window() else { return };
    let Some(track) = ctx.player().and_then(|s| s.current) else {
        return;
    };
    let choose = ui.global::<LyricsChoose>();
    choose.set_state(ChooseState::Loading);
    choose.set_matches(ModelRc::default());
    let n = SEARCHES.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    let text = text.to_string();
    let for_fixtures = track.clone();
    ctx.load(
        move |source| async move {
            if let Source::Fixtures(f) = source {
                return Ok(fixture_records(&f, &for_fixtures));
            }
            let client = client().ok_or_else(|| "no LRCLIB client".to_string())?;
            client.search_text(&text).await.map_err(|e| e.to_string())
        },
        move |_, ui, result: Result<Vec<Record>, String>| {
            if SEARCHES.with(Cell::get) != n {
                return;
            }
            let choose = ui.global::<LyricsChoose>();
            match result {
                Ok(records) => {
                    log!("lyrics search: {} records", records.len());
                    show_matches(&choose, &track, records);
                }
                Err(e) => {
                    log!("lyrics search: {e}");
                    choose.set_state(ChooseState::Error);
                }
            }
        },
    );
}

/// Fills the dialog's list: the song's own lyrics first when they're the ones shown, then
/// LRCLIB's records, the one shown marked.
fn show_matches(choose: &LyricsChoose, track: &Track, records: Vec<Record>) {
    let found = shown_found(&track.id);
    let in_use = found.as_ref().and_then(|f| f.lrclib_id);
    let mut rows: Vec<Option<Record>> = Vec::new();
    let mut shown: Vec<LyricsMatch> = Vec::new();
    if let Some(f) = found.as_ref().filter(|f| f.from_song && !f.picked)
        && let Some(lyrics) = &f.lyrics
    {
        rows.push(None);
        shown.push(LyricsMatch {
            title: crate::ui_bridge::shown(&track.title),
            byline: tr!("From YouTube Music").into(),
            first_line: first_line(lyrics).into(),
            synced: matches!(lyrics, Lyrics::Synced(_)),
            length: SharedString::default(),
            in_use: true,
        });
    }
    for record in records {
        let artist = record.artist_name.clone().unwrap_or_default();
        let byline = match record.album_name.as_deref().filter(|a| !a.is_empty()) {
            Some(album) => format!("{artist} · {album}"),
            None => artist,
        };
        shown.push(LyricsMatch {
            title: record.track_name.clone().unwrap_or_default().into(),
            byline: byline.into(),
            first_line: record.first_line().unwrap_or_default().into(),
            synced: record.synced(),
            length: record
                .duration
                .filter(|d| d.is_finite() && *d > 0.0)
                .map(|d| format_duration(Duration::from_secs_f64(d)))
                .unwrap_or_default()
                .into(),
            in_use: record.id.is_some() && record.id == in_use,
        });
        rows.push(Some(record));
    }
    choose.set_state(if shown.is_empty() {
        ChooseState::Empty
    } else {
        ChooseState::List
    });
    choose.set_matches(ModelRc::new(VecModel::from(shown)));
    ROWS.with(|r| *r.borrow_mut() = rows);
}

/// The first line of `lyrics` with words.
fn first_line(lyrics: &Lyrics) -> String {
    match lyrics {
        Lyrics::Synced(lines) => lines
            .iter()
            .map(|l| l.text.as_str())
            .find(|t| !t.is_empty())
            .unwrap_or_default()
            .to_string(),
        Lyrics::Plain(text) => text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or_default()
            .to_string(),
    }
}

/// A click on row `i`: its record is used for the song; the song's own lyrics row goes back
/// to the automatic match.
fn pick_row(ctx: &Rc<Ctx>, i: i32) {
    let row = ROWS.with(|r| {
        usize::try_from(i)
            .ok()
            .and_then(|i| r.borrow().get(i).cloned())
    });
    // The song's own lyrics (`None`) are already the automatic match's.
    let Some(Some(record)) = row else { return };
    let Some(id) = record.id else { return };
    set_pick(
        ctx,
        Some(LyricsPick::Lrclib {
            id,
            synced: record.synced_lyrics,
            plain: record.plain_lyrics,
        }),
    );
}

/// Keeps `pick` for the playing song (`None`: the automatic match) and shows what it gives.
fn set_pick(ctx: &Rc<Ctx>, pick: Option<LyricsPick>) {
    let Some(ui) = ctx.window() else { return };
    let Some(id) = ctx.player().and_then(|s| s.current).map(|t| t.id) else {
        return;
    };
    update(&id, |c| c.pick = pick);
    // What shows for the song is forgotten: the pick answers at once, or the automatic
    // lookup runs again.
    let live = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.cache.remove(&id);
        s.shown = None;
        s.live
    });
    if live {
        show(ctx, false);
    } else {
        ui.global::<LyricsData>().set_state(LyricsState::Idle);
    }
}

/// `--fixtures`: LRCLIB's matches for the playing song, made from the demo song's own
/// lines (as board B draws them): the record in use, a live recording, a plain record from
/// another album and a near-duplicate. Outside the demo set, the recorded record.
fn fixture_records(f: &Fixtures, track: &Track) -> Vec<Record> {
    let Ok(Some((lines, _))) = f.demo_lyrics(&track.id) else {
        return super::lookup::fixture_record().into_iter().collect();
    };
    if lines.is_empty() {
        return Vec::new();
    }
    let artist = track
        .artists
        .first()
        .map(|a| a.name.clone())
        .unwrap_or_default();
    let album = track.album.as_ref().map(|a| a.title.clone());
    // Whole seconds, as the song's length shows.
    let secs = track.duration.map_or(0.0, |d| d.as_secs() as f64);
    let short = match artist.split_once(' ') {
        Some((first, rest)) => format!("{}. {rest}", first.chars().next().unwrap_or(' ')),
        None => artist.clone(),
    };
    let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    let rotated: Vec<Line> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| Line {
            start: l.start,
            text: texts[(i + texts.len() / 3) % texts.len()].to_string(),
        })
        .collect();
    let record = |id: u64, title: String, artist: String, album: Option<String>, d: f64| Record {
        id: Some(id),
        track_name: Some(title),
        artist_name: Some(artist),
        album_name: album,
        duration: Some(d),
        instrumental: Some(false),
        ..Record::default()
    };
    vec![
        Record {
            synced_lyrics: Some(lrc_text(&lines)),
            ..record(
                DEMO_RECORD,
                track.title.clone(),
                artist.clone(),
                album,
                secs,
            )
        },
        Record {
            synced_lyrics: Some(lrc_text(&rotated)),
            ..record(
                2,
                format!("{} (Live)", track.title),
                artist.clone(),
                Some("Live in the Living Room".into()),
                secs + 138.0,
            )
        },
        Record {
            plain_lyrics: Some(texts[texts.len() / 2..].join("\n")),
            ..record(
                3,
                track.title.clone(),
                artist,
                Some("Breadcrumbs".into()),
                secs + 167.0,
            )
        },
        Record {
            synced_lyrics: Some(lrc_text(&lines)),
            ..record(4, track.title.clone(), short, None, secs + 1.0)
        },
    ]
}

/// Lines in LRC, as LRCLIB stores them.
fn lrc_text(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|l| {
            let cs = l.start.as_millis() / 10;
            format!(
                "[{:02}:{:02}.{:02}]{}",
                cs / 6000,
                cs / 100 % 60,
                cs % 100,
                l.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `--snapshot`: the dialog open on the playing song, its matches listed.
pub fn open_for_snapshot(ctx: &Rc<Ctx>) {
    open(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_matches_should_parse_back_and_differ_in_their_first_lines() {
        let f = Fixtures::load_set(featherplay_controller::fixtures::FixtureSet::Demo)
            .expect("demo fixtures");
        let track = f.radio().expect("radio").remove(0);
        let records = fixture_records(&f, &track);
        assert_eq!(records.len(), 4);
        assert!(records[0].synced() && records[1].synced() && !records[2].synced());
        assert_ne!(records[0].first_line(), records[1].first_line());
        assert_eq!(records[0].first_line(), records[3].first_line());
        // What a pick keeps parses back to the same lines.
        let lyrics = |r: &Record| r.lyrics().expect("lyrics");
        let kept = Record {
            id: records[0].id,
            synced_lyrics: records[0].synced_lyrics.clone(),
            ..Record::default()
        };
        assert_eq!(lyrics(&kept), lyrics(&records[0]));
    }
}
