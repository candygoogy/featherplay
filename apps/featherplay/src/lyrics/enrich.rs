//! The Translation and Romanization buttons: their session cache, network work and the
//! decoration model handed to Slint. Google is asked only while a button is on.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

use featherplay_core::model::TrackId;
use featherplay_lyrics::enrich::{self as service, Batch, Client};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

use crate::app::Ctx;
use crate::pages::settings;
use crate::ui::{LyricDecoration, LyricsData};

/// Translation follows the reference UI: English under the source line. A later language
/// picker can pass another BCP-47 code without changing the service client.
const TRANSLATION_TARGET: &str = "en";
/// Non-blank lines in a song's first translation request; enough to tell its language, so
/// an English song sends no more than these.
const FIRST_CHUNK: usize = 8;
const MAX_CACHED: usize = 64;
const FEED_ROWS: usize = 4;
const FEED_EVERY: Duration = Duration::from_millis(4);

#[derive(Clone, Debug, Default)]
struct Cached {
    source: Vec<String>,
    translation: Option<Vec<String>>,
    romanization: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Translation,
    Romanization,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Job {
    id: TrackId,
    generation: u64,
    kind: Kind,
}

#[derive(Clone, Debug)]
struct Current {
    id: TrackId,
    source: Vec<String>,
    generation: u64,
}

#[derive(Default)]
struct State {
    live: bool,
    current: Option<Current>,
    cache: HashMap<TrackId, Cached>,
    jobs: HashSet<Job>,
    generation: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// Hands a returned song's decorations to every lyrics view a few lines per frame, as
    /// the source lines themselves are fed in `lyrics.rs`.
    static FEED: Timer = Timer::default();
}

struct Answer {
    id: TrackId,
    source: Vec<String>,
    generation: u64,
    translation: Option<Result<Batch, String>>,
    romanization: Option<Result<Batch, String>>,
}

pub(super) fn init(ctx: &Rc<Ctx>, data: &LyricsData) {
    let c = ctx.clone();
    data.on_set_translation(move |on| set_translation(&c, on));
    let c = ctx.clone();
    data.on_set_romanization(move |on| set_romanization(&c, on));
    let c = ctx.clone();
    data.on_decorations_live_changed(move |live| {
        STATE.with(|state| state.borrow_mut().live = live);
        if live {
            if let Some(ui) = c.window() {
                show_cached(&ui.global::<LyricsData>());
            }
            request(&c);
        }
    });
}

fn set_translation(ctx: &Rc<Ctx>, on: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<LyricsData>();
    data.set_translation_on(on);
    settings::set_lyrics_translation(ctx, on);
    if on {
        show_cached(&data);
        request(ctx);
    } else {
        refresh_loading(&data);
    }
}

fn set_romanization(ctx: &Rc<Ctx>, on: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<LyricsData>();
    data.set_romanization_on(on);
    settings::set_lyrics_romanization(ctx, on);
    if on {
        show_cached(&data);
        request(ctx);
    } else {
        refresh_loading(&data);
    }
}

/// Another song or another set of words for this song came to the view.
pub(super) fn lines_changed(ctx: &Rc<Ctx>, data: &LyricsData, id: &TrackId, source: Vec<String>) {
    let has_non_latin = source.iter().any(|line| service::has_non_latin(line));
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let same = state
            .current
            .as_ref()
            .is_some_and(|current| current.id == *id && current.source == source);
        if !same {
            state.generation = state.generation.wrapping_add(1);
            let generation = state.generation;
            state.current = Some(Current {
                id: id.clone(),
                source: source.clone(),
                generation,
            });
        }
        let stale = state
            .cache
            .get(id)
            .is_some_and(|cached| cached.source != source);
        if stale {
            state.cache.remove(id);
        }
        if state.cache.len() >= MAX_CACHED && !state.cache.contains_key(id) {
            state.cache.retain(|cached_id, _| cached_id == id);
        }
        state.cache.entry(id.clone()).or_insert_with(|| Cached {
            source,
            ..Cached::default()
        });
    });
    data.set_has_non_latin(has_non_latin);
    show_cached(data);
    request(ctx);
}

/// A track with no lines is showing, or the view is between tracks.
pub(super) fn clear(data: &LyricsData) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.generation = state.generation.wrapping_add(1);
        state.current = None;
    });
    FEED.with(Timer::stop);
    data.set_decorations(ModelRc::new(VecModel::default()));
    data.set_has_non_latin(false);
    data.set_has_translation(false);
    data.set_has_romanization(false);
    data.set_translation_loading(false);
    data.set_romanization_loading(false);
}

fn request(ctx: &Rc<Ctx>) {
    // Fixture and snapshot runs are self-contained: never turn a saved switch into an
    // external request from a test or a design render.
    if ctx.catalog.fixtures().is_some() {
        return;
    }
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<LyricsData>();
    let translation_on = data.get_translation_on();
    let romanization_on = data.get_romanization_on() && data.get_has_non_latin();
    let Some((current, translation, romanization)) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if !state.live {
            return None;
        }
        let current = state.current.clone()?;
        if current.source.is_empty() {
            return None;
        }
        let cached = state.cache.get(&current.id)?;
        let translation_job = Job {
            id: current.id.clone(),
            generation: current.generation,
            kind: Kind::Translation,
        };
        let romanization_job = Job {
            id: current.id.clone(),
            generation: current.generation,
            kind: Kind::Romanization,
        };
        let translation = translation_on
            && cached.translation.is_none()
            && !state.jobs.contains(&translation_job);
        let romanization = romanization_on
            && cached.romanization.is_none()
            && !state.jobs.contains(&romanization_job);
        if translation {
            state.jobs.insert(translation_job);
        }
        if romanization {
            state.jobs.insert(romanization_job);
        }
        Some((current, translation, romanization))
    }) else {
        return;
    };

    refresh_loading(&data);
    if !translation && !romanization {
        return;
    }

    let id = current.id;
    let source = current.source;
    let generation = current.generation;
    let work_id = id.clone();
    let work_source = source.clone();
    ctx.load(
        move |_| async move {
            let client = client();
            let translation_result = if translation {
                Some(match &client {
                    Ok(client) => translate_song(client, &work_source)
                        .await
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(e.clone()),
                })
            } else {
                None
            };
            let romanization_result = if romanization {
                Some(match &client {
                    Ok(client) => client
                        .romanize(&work_source)
                        .await
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(e.clone()),
                })
            } else {
                None
            };
            Answer {
                id: work_id,
                source: work_source,
                generation,
                translation: translation_result,
                romanization: romanization_result,
            }
        },
        apply,
    );
}

fn apply(_: &Rc<Ctx>, ui: &crate::ui::AppWindow, answer: Answer) {
    let mut translation_error = None;
    let mut romanization_error = None;
    let answered_translation = answer.translation.is_some();
    let answered_romanization = answer.romanization.is_some();
    let current = STATE.with(|state| {
        let mut state = state.borrow_mut();
        for (kind, answered) in [
            (Kind::Translation, answered_translation),
            (Kind::Romanization, answered_romanization),
        ] {
            if answered {
                state.jobs.remove(&Job {
                    id: answer.id.clone(),
                    generation: answer.generation,
                    kind,
                });
            }
        }
        if let Some(cached) = state.cache.get_mut(&answer.id)
            && cached.source == answer.source
        {
            if let Some(result) = answer.translation {
                match result {
                    Ok(batch) => cached.translation = Some(strings(batch)),
                    Err(e) => translation_error = Some(e),
                }
            }
            if let Some(result) = answer.romanization {
                match result {
                    Ok(batch) => cached.romanization = Some(strings(batch)),
                    Err(e) => romanization_error = Some(e),
                }
            }
        }
        state.live
            && state.current.as_ref().is_some_and(|current| {
                current.id == answer.id
                    && current.source == answer.source
                    && current.generation == answer.generation
            })
    });
    let data = ui.global::<LyricsData>();
    refresh_loading(&data);
    if !current {
        return;
    }
    show_cached(&data);
    // A failure shows nothing: the lines stay as they are, and the next time these lyrics
    // come on screen they are asked for again.
    if let Some(error) = translation_error {
        log!("lyrics translation: {error}");
    }
    if let Some(error) = romanization_error {
        log!("lyrics romanization: {error}");
    }
}

/// Translates a song, its first few lines first: when those come back as English, the
/// rest isn't sent and the song keeps no translation.
async fn translate_song(client: &Client, lines: &[String]) -> Result<Batch, service::Error> {
    let split = first_chunk_len(lines);
    let mut batch = client
        .translate(&lines[..split], TRANSLATION_TARGET)
        .await?;
    if is_english(&batch) {
        batch.lines = vec![None; lines.len()];
        return Ok(batch);
    }
    if split < lines.len() {
        // A chorus already asked in the first request isn't sent again: it goes blank
        // here and takes the first answer for its text.
        let known: HashMap<&str, Option<String>> = lines[..split]
            .iter()
            .map(|line| line.trim())
            .zip(batch.lines.iter().cloned())
            .collect();
        let rest: Vec<String> = lines[split..]
            .iter()
            .map(|line| {
                if known.contains_key(line.trim()) {
                    String::new()
                } else {
                    line.clone()
                }
            })
            .collect();
        let answered = client.translate(&rest, TRANSLATION_TARGET).await?;
        batch.lines.extend(
            lines[split..]
                .iter()
                .zip(answered.lines)
                .map(|(line, answer)| known.get(line.trim()).cloned().unwrap_or(answer)),
        );
    }
    Ok(batch)
}

/// The lines up to and including the `FIRST_CHUNK`th non-blank one.
fn first_chunk_len(lines: &[String]) -> usize {
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .nth(FIRST_CHUNK - 1)
        .map_or(lines.len(), |(index, _)| index + 1)
}

fn is_english(batch: &Batch) -> bool {
    batch
        .detected_language
        .as_deref()
        .is_some_and(|language| language.starts_with("en"))
}

fn strings(batch: Batch) -> Vec<String> {
    batch
        .lines
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect()
}

fn client() -> Result<Client, String> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| Client::new().map_err(|e| e.to_string()))
        .clone()
}

fn refresh_loading(data: &LyricsData) {
    let (translation, romanization) = STATE.with(|state| {
        let state = state.borrow();
        let Some(current) = &state.current else {
            return (false, false);
        };
        let has = |kind| {
            state.jobs.contains(&Job {
                id: current.id.clone(),
                generation: current.generation,
                kind,
            })
        };
        (has(Kind::Translation), has(Kind::Romanization))
    });
    data.set_translation_loading(translation && data.get_translation_on());
    data.set_romanization_loading(romanization && data.get_romanization_on());
}

fn show_cached(data: &LyricsData) {
    let rows = STATE.with(|state| {
        let state = state.borrow();
        let current = state.current.as_ref()?;
        let cached = state.cache.get(&current.id)?;
        Some(
            cached
                .source
                .iter()
                .enumerate()
                .map(|(index, _)| LyricDecoration {
                    translation: cached
                        .translation
                        .as_ref()
                        .and_then(|lines| lines.get(index))
                        .map_or("", String::as_str)
                        .into(),
                    romanization: cached
                        .romanization
                        .as_ref()
                        .and_then(|lines| lines.get(index))
                        .map_or("", String::as_str)
                        .into(),
                })
                .collect::<Vec<_>>(),
        )
    });
    if let Some(rows) = rows {
        data.set_has_translation(rows.iter().any(|row| !row.translation.is_empty()));
        data.set_has_romanization(rows.iter().any(|row| !row.romanization.is_empty()));
        feed(data, rows);
    }
}

fn blank() -> LyricDecoration {
    LyricDecoration {
        translation: "".into(),
        romanization: "".into(),
    }
}

fn feed(data: &LyricsData, rows: Vec<LyricDecoration>) {
    FEED.with(Timer::stop);
    let first = rows.len().min(FEED_ROWS);
    let mut shown = vec![blank(); rows.len()];
    shown[..first].clone_from_slice(&rows[..first]);
    let model = Rc::new(VecModel::from(shown));
    data.set_decorations(ModelRc::from(model.clone()));
    if first == rows.len() {
        return;
    }
    let weak = Rc::downgrade(&model);
    let pending = RefCell::new(rows.into_iter().enumerate().skip(first));
    FEED.with(|timer| {
        timer.start(TimerMode::Repeated, FEED_EVERY, move || {
            let Some(model) = weak.upgrade() else {
                FEED.with(Timer::stop);
                return;
            };
            let mut pending = pending.borrow_mut();
            for (index, row) in pending.by_ref().take(FEED_ROWS) {
                model.set_row_data(index, row);
            }
            if pending.len() == 0 {
                FEED.with(Timer::stop);
            }
        });
    });
}

/// The snapshots' made-up song: each line, its romanization and its English.
pub(super) const SNAPSHOT_LINES: [(&str, &str, &str); 11] = [
    (
        "夜明け前の街を歩く",
        "yoake mae no machi o aruku",
        "Walking through the town before dawn",
    ),
    (
        "誰もいない交差点",
        "dare mo inai kōsaten",
        "A crossing with no one there",
    ),
    (
        "今ここで歌っている",
        "ima koko de utatte iru",
        "I'm singing here right now",
    ),
    (
        "何度でも思い出す",
        "nando demo omoidasu",
        "I remember it again and again",
    ),
    (
        "七色にきらめいて",
        "nana-iro ni kirameite",
        "Shining in seven colours",
    ),
    (
        "明日へ歩き出す",
        "ashita e arukidasu",
        "I start walking towards tomorrow",
    ),
    (
        "遠い空の向こうまで",
        "tōi sora no mukō made",
        "All the way past the distant sky",
    ),
    (
        "君の声が聞こえる",
        "kimi no koe ga kikoeru",
        "I can hear your voice",
    ),
    (
        "消えない光を抱いて",
        "kienai hikari o daite",
        "Holding a light that won't go out",
    ),
    ("風の中で", "kaze no naka de", "In the wind"),
    ("もう一度", "mō ichido", "One more time"),
];

/// `--snapshot now-playing-lyrics-full-enriched` (and `-translating`): both on without
/// asking the network, the answers in, or the English still on its way. `show_for_snapshot`
/// has already put `SNAPSHOT_LINES` on screen.
pub(super) fn show_for_snapshot(data: &LyricsData, translating: bool) {
    FEED.with(Timer::stop);
    data.set_translation_on(true);
    data.set_romanization_on(!translating);
    data.set_has_non_latin(true);
    data.set_romanization_loading(false);
    let kept = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(current) = state.current.clone() else {
            return false;
        };
        if translating {
            state.jobs.insert(Job {
                id: current.id.clone(),
                generation: current.generation,
                kind: Kind::Translation,
            });
        }
        let Some(cached) = state.cache.get_mut(&current.id) else {
            return false;
        };
        if !translating {
            cached.romanization = Some(SNAPSHOT_LINES.iter().map(|l| l.1.into()).collect());
            cached.translation = Some(SNAPSHOT_LINES.iter().map(|l| l.2.into()).collect());
        }
        true
    });
    if kept {
        show_cached(data);
        refresh_loading(data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_entries_become_empty_strings_without_losing_their_indices() {
        let batch = Batch {
            lines: vec![Some("ichi".into()), None, Some("san".into())],
            detected_language: Some("ja".into()),
        };
        assert_eq!(strings(batch), ["ichi", "", "san"]);
    }

    #[test]
    fn the_first_request_holds_the_first_eight_sung_lines() {
        let mut lines: Vec<String> = (0..12).map(|n| format!("line {n}")).collect();
        lines.insert(2, String::new());
        lines.insert(5, "  ".into());
        assert_eq!(first_chunk_len(&lines), 10);
        assert_eq!(first_chunk_len(&lines[..4]), 4);
    }

    /// A local stand-in for `translate_a/t` that answers every request with eight English
    /// lines, and counts the requests.
    fn english_stand_in() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::sync::atomic::Ordering;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
        let base = format!(
            "http://{}/translate_a",
            listener.local_addr().expect("address")
        );
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = count.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                seen.fetch_add(1, Ordering::SeqCst);
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut length = 0;
                let mut header = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    header.clear();
                }
                let _ = reader.read_exact(&mut vec![0; length]);
                let body = format!("[{}]", [r#"["Hello","en"]"#; FIRST_CHUNK].join(","));
                let reply = format!(
                    "HTTP/1.1 200 OK
content-type: application/json
content-length: {}
connection: close

{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (base, count)
    }

    #[tokio::test]
    async fn english_first_lines_stop_the_rest_of_the_song() {
        let (base, requests) = english_stand_in();
        let client = Client::with_base(&base).expect("client");
        let lines: Vec<String> = (0..20).map(|n| format!("sung line {n}")).collect();
        let batch = translate_song(&client, &lines).await.expect("an answer");
        assert_eq!(batch.lines, vec![None; lines.len()]);
        assert_eq!(batch.detected_language.as_deref(), Some("en"));
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
