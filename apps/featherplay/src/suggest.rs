//! The suggestions under the top bar's search field (ui/top_bar.slint): while the user
//! types, YouTube Music's completions of the query and a few direct results (songs, albums,
//! artists, playlists). Picking a completion searches for it; picking a result plays it or
//! opens its page, as a row of the Search page does.
//!
//! One request per pause in the typing (`DEBOUNCE`), signed in or not, none for an empty
//! field or a link (Enter opens what it points at). Only the latest edit's answer shows; the
//! last few answers are kept (`CACHED`), so typing back to an earlier query shows its
//! suggestions at once. The keys and the motion are Slint's; nothing runs while closed.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use featherplay_ytm::{HomeItem, SearchSuggestions};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::app::Ctx;
use crate::controller::Action;
use crate::link;
use crate::router::Page;
use crate::ui::{AppWindow, Suggestion, SuggestionKind, TopBarData};

/// How long the typing must pause before the suggestions are asked for.
pub const DEBOUNCE: Duration = Duration::from_millis(180);
/// How many queries' answers are kept.
const CACHED: usize = 8;
/// At most this many completions, then this many results, as YouTube Music shows them.
const MAX_QUERIES: usize = 7;
const MAX_RESULTS: usize = 5;
/// A result's cover, square, in logical pixels.
const COVER_PX: u32 = 40;

/// What an edit of the field calls for.
#[derive(Debug, PartialEq)]
pub enum Next {
    /// Nothing to suggest for (an empty field, a link): no rows, no request.
    Hide,
    /// Asked for before: these, at once.
    Show(SearchSuggestions),
    /// Ask for `query` once the typing pauses; the answer is edit `ask`'s.
    Fetch { ask: u64, query: String },
}

/// Which edit is the latest, and the last answers. No Slint in here: the tests drive it.
#[derive(Default)]
pub struct Suggester {
    latest: u64,
    cache: VecDeque<(String, SearchSuggestions)>,
}

impl Suggester {
    /// The field now reads `text`.
    pub fn edited(&mut self, text: &str) -> Next {
        self.latest += 1;
        let query = text.trim();
        if query.is_empty() || link::parse(query).is_some() {
            return Next::Hide;
        }
        if let Some((_, cached)) = self.cache.iter().find(|(q, _)| q == query) {
            return Next::Show(cached.clone());
        }
        Next::Fetch {
            ask: self.latest,
            query: query.to_string(),
        }
    }

    /// Edit `ask` is still the latest: its request may go, its answer may show.
    pub fn is_latest(&self, ask: u64) -> bool {
        ask == self.latest
    }

    /// The answer to edit `ask`, for `query`: kept for later, and returned to show if no
    /// edit came after it.
    pub fn answered(
        &mut self,
        ask: u64,
        query: &str,
        mut answer: SearchSuggestions,
    ) -> Option<SearchSuggestions> {
        answer.queries.truncate(MAX_QUERIES);
        answer.results.truncate(MAX_RESULTS);
        self.cache.retain(|(q, _)| q != query);
        if self.cache.len() >= CACHED {
            self.cache.pop_front();
        }
        self.cache.push_back((query.to_string(), answer.clone()));
        self.is_latest(ask).then_some(answer)
    }

    /// The rows closed (Escape, a pick, a page change): answers on their way are dropped.
    pub fn close(&mut self) {
        self.latest += 1;
    }
}

/// What a picked row does.
#[derive(Debug, PartialEq)]
pub enum Pick {
    /// Search for the completion.
    Search(String),
    /// Play the song or video, with its radio after it.
    Play(Box<featherplay_core::model::Track>),
    Open(Page),
}

/// What row `i` of `shown`'s rows (completions, then results) does.
pub fn pick_of(shown: &SearchSuggestions, i: usize) -> Option<Pick> {
    if let Some(q) = shown.queries.get(i) {
        return Some(Pick::Search(q.text.clone()));
    }
    let r = shown.results.get(i - shown.queries.len())?;
    Some(match &r.item {
        HomeItem::Track(t) => Pick::Play(Box::new(t.clone())),
        HomeItem::Album(a) => Pick::Open(Page::Album { id: a.id.clone() }),
        HomeItem::Artist(a) => Pick::Open(Page::Artist { id: a.id.clone() }),
        HomeItem::Playlist(p) => Pick::Open(Page::Playlist { id: p.id.clone() }),
    })
}

/// The rows for `s`, and each one's cover URL.
pub fn rows(s: &SearchSuggestions) -> (Vec<Suggestion>, Vec<Option<String>>) {
    let completions = s.queries.iter().map(|q| {
        let (typed, rest) = q.text.split_at(q.typed.min(q.text.len()));
        let row = Suggestion {
            kind: if q.history {
                SuggestionKind::History
            } else {
                SuggestionKind::Query
            },
            title: typed.into(),
            rest: rest.into(),
            ..Suggestion::default()
        };
        (row, None)
    });
    let results = s.results.iter().map(|r| {
        let (kind, title, cover) = match &r.item {
            HomeItem::Track(t) => (SuggestionKind::Song, &t.title, &t.thumbnail),
            HomeItem::Album(a) => (SuggestionKind::Album, &a.title, &a.thumbnail),
            HomeItem::Artist(a) => (SuggestionKind::Artist, &a.name, &a.thumbnail),
            HomeItem::Playlist(p) => (SuggestionKind::Playlist, &p.title, &p.thumbnail),
        };
        let row = Suggestion {
            kind,
            title: title.into(),
            subtitle: r.subtitle.as_str().into(),
            ..Suggestion::default()
        };
        (row, cover.clone())
    });
    completions.chain(results).unzip()
}

#[derive(Default)]
struct State {
    suggester: Suggester,
    /// What the rows show, for a pick.
    shown: Option<SearchSuggestions>,
    /// The pause before a request: a single shot, so nothing runs once it has fired.
    debounce: Option<slint::Timer>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Connects the field's suggestion callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let field = ui.global::<TopBarData>();
    let c = ctx.clone();
    field.on_edited(move |text| edited(&c, &text));
    let c = ctx.clone();
    field.on_pick(move |i| pick(&c, i.max(0) as usize));
    field.on_dismissed(|| STATE.with(|s| s.borrow_mut().suggester.close()));
}

fn edited(ctx: &Rc<Ctx>, text: &str) {
    let next = STATE.with(|s| s.borrow_mut().suggester.edited(text));
    let Some(ui) = ctx.window() else { return };
    match next {
        Next::Hide => {
            stop_debounce();
            hide(&ui);
        }
        Next::Show(s) => {
            stop_debounce();
            show(ctx, &ui, s);
        }
        // The rows shown stay until the new answer replaces them.
        Next::Fetch { ask, query } => {
            let c = ctx.clone();
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                let timer = s.debounce.get_or_insert_with(slint::Timer::default);
                timer.start(slint::TimerMode::SingleShot, DEBOUNCE, move || {
                    fetch(&c, ask, query.clone());
                });
            });
        }
    }
}

fn stop_debounce() {
    STATE.with(|s| {
        if let Some(t) = &s.borrow().debounce {
            t.stop();
        }
    });
}

/// Edit `ask`'s request, unless another edit came while the typing paused.
fn fetch(ctx: &Rc<Ctx>, ask: u64, query: String) {
    if !STATE.with(|s| s.borrow().suggester.is_latest(ask)) {
        return;
    }
    let asked = query.clone();
    ctx.load(
        move |source| async move { source.search_suggestions(&asked).await },
        move |ctx, ui, result| match result {
            Ok(answer) => {
                let latest = STATE.with(|s| s.borrow_mut().suggester.answered(ask, &query, answer));
                // The field may have been cleared or filled by a page since.
                let current = ui.global::<TopBarData>().get_query().trim() == query;
                if let Some(answer) = latest.filter(|_| current) {
                    show(ctx, ui, answer);
                }
            }
            Err(e) => log!("search suggestions: {e}"),
        },
    );
}

fn show(ctx: &Ctx, ui: &AppWindow, s: SearchSuggestions) {
    let field = ui.global::<TopBarData>();
    let (items, covers) = rows(&s);
    let model = Rc::new(VecModel::from(items));
    field.set_completions(s.queries.len() as i32);
    field.set_suggestions(ModelRc::from(model.clone()));
    field.set_highlighted(-1);
    field.set_suggesting(model.row_count() > 0);
    ctx.covers
        .fill(&model, covers, COVER_PX, |row, img| row.cover = img);
    STATE.with(|st| st.borrow_mut().shown = Some(s));
}

fn hide(ui: &AppWindow) {
    let field = ui.global::<TopBarData>();
    field.set_suggesting(false);
    field.set_highlighted(-1);
    field.set_completions(0);
    field.set_suggestions(ModelRc::default());
    STATE.with(|s| s.borrow_mut().shown = None);
}

/// The page changed: the rows close, and an answer on its way is dropped.
pub fn close(ui: &AppWindow) {
    stop_debounce();
    STATE.with(|s| s.borrow_mut().suggester.close());
    hide(ui);
}

fn pick(ctx: &Rc<Ctx>, i: usize) {
    let what = STATE.with(|s| {
        s.borrow()
            .shown
            .as_ref()
            .and_then(|shown| pick_of(shown, i))
    });
    STATE.with(|s| s.borrow_mut().suggester.close());
    if let Some(ui) = ctx.window() {
        ui.global::<TopBarData>().set_suggesting(false);
    }
    match what {
        Some(Pick::Search(query)) => {
            if let Some(ui) = ctx.window() {
                ui.global::<TopBarData>().set_query(query.as_str().into());
            }
            ctx.navigate(link::search_page(&query));
        }
        Some(Pick::Play(track)) => ctx.send(Action::Play {
            tracks: vec![*track],
            start: 0,
            radio: true,
        }),
        Some(Pick::Open(page)) => ctx.navigate(page),
        None => {}
    }
}

/// `--snapshot search-suggestions`: Ctrl+K into the field, "daft pu" in it, and the recorded
/// suggestions asked for at once (no pause): they show once the loads have settled.
pub fn show_for_snapshot(ctx: &Rc<Ctx>) {
    use slint::platform::{Key, WindowEvent};
    let Some(ui) = ctx.window() else { return };
    let w = ui.window();
    w.dispatch_event(WindowEvent::WindowActiveChanged(true));
    let key = |text: slint::SharedString, down: bool| {
        w.dispatch_event(if down {
            WindowEvent::KeyPressed { text }
        } else {
            WindowEvent::KeyReleased { text }
        });
    };
    key(Key::Control.into(), true);
    key("k".into(), true);
    key("k".into(), false);
    key(Key::Control.into(), false);
    ui.global::<TopBarData>().set_query("daft pu".into());
    let next = STATE.with(|s| s.borrow_mut().suggester.edited("daft pu"));
    if let Next::Fetch { ask, query } = next {
        fetch(ctx, ask, query);
    }
}

#[cfg(test)]
mod tests {
    use featherplay_core::model::{AlbumId, ArtistId, Track, TrackId};
    use featherplay_ytm::{AlbumSummary, ArtistSummary, QuerySuggestion, SuggestedResult};

    use super::*;

    fn answer(query: &str) -> SearchSuggestions {
        SearchSuggestions {
            queries: vec![QuerySuggestion {
                text: format!("{query}nk"),
                typed: query.len(),
                history: false,
            }],
            results: Vec::new(),
        }
    }

    fn ask_of(next: Next) -> u64 {
        match next {
            Next::Fetch { ask, .. } => ask,
            other => panic!("expected a fetch, got {other:?}"),
        }
    }

    #[test]
    fn an_edit_asks_for_its_trimmed_text() {
        let mut s = Suggester::default();
        assert_eq!(
            s.edited("  daft "),
            Next::Fetch {
                ask: 1,
                query: "daft".into()
            }
        );
    }

    #[test]
    fn an_empty_field_asks_nothing() {
        assert_eq!(Suggester::default().edited("   "), Next::Hide);
    }

    #[test]
    fn a_link_asks_nothing() {
        let mut s = Suggester::default();
        assert_eq!(s.edited("https://youtu.be/Jb6gcoR266U"), Next::Hide);
    }

    #[test]
    fn a_stale_answer_is_kept_but_not_shown() {
        let mut s = Suggester::default();
        let first = ask_of(s.edited("da"));
        let second = ask_of(s.edited("daf"));
        assert_eq!(s.answered(first, "da", answer("da")), None);
        assert_eq!(
            s.answered(second, "daf", answer("daf")),
            Some(answer("daf"))
        );
    }

    #[test]
    fn an_answer_after_the_rows_closed_is_dropped() {
        let mut s = Suggester::default();
        let ask = ask_of(s.edited("daft"));
        s.close();
        assert_eq!(s.answered(ask, "daft", answer("daft")), None);
    }

    #[test]
    fn a_query_answered_before_shows_at_once() {
        let mut s = Suggester::default();
        let ask = ask_of(s.edited("da"));
        s.answered(ask, "da", answer("da"));
        s.edited("daf");
        assert_eq!(s.edited("da "), Next::Show(answer("da")));
    }

    #[test]
    fn only_the_last_few_answers_are_kept() {
        let mut s = Suggester::default();
        for i in 0..=CACHED {
            let q = format!("q{i}");
            let ask = ask_of(s.edited(&q));
            s.answered(ask, &q, answer(&q));
        }
        assert!(
            matches!(s.edited("q0"), Next::Fetch { .. }),
            "the oldest went"
        );
        assert!(matches!(s.edited("q1"), Next::Show(_)));
    }

    #[test]
    fn a_debounce_skipped_by_a_newer_edit_is_no_longer_latest() {
        let mut s = Suggester::default();
        let first = ask_of(s.edited("a"));
        s.edited("ai");
        assert!(!s.is_latest(first));
    }

    fn mixed() -> SearchSuggestions {
        let track = Track {
            id: TrackId("qe8Q7mjxjig".into()),
            title: "Veridis Quo".into(),
            artists: Vec::new(),
            album: None,
            duration: None,
            video_type: featherplay_core::model::VideoType::Song,
            thumbnail: Some("https://example.test/song".into()),
            like: featherplay_core::model::LikeStatus::Indifferent,
            counterpart: None,
        };
        SearchSuggestions {
            queries: vec![
                QuerySuggestion {
                    text: "daft punk".into(),
                    typed: 7,
                    history: false,
                },
                QuerySuggestion {
                    text: "daft punk live".into(),
                    typed: 7,
                    history: true,
                },
            ],
            results: vec![
                SuggestedResult {
                    item: HomeItem::Artist(ArtistSummary {
                        id: ArtistId("UCRr1xG_2WIDs18a6cIiCxeA".into()),
                        name: "Daft Punk".into(),
                        thumbnail: None,
                    }),
                    subtitle: "68.2M monthly audience".into(),
                },
                SuggestedResult {
                    item: HomeItem::Track(track),
                    subtitle: "Song • Daft Punk".into(),
                },
                SuggestedResult {
                    item: HomeItem::Album(AlbumSummary {
                        id: AlbumId("MPREb_dMHztPsrDgN".into()),
                        title: "Homework".into(),
                        artists: Vec::new(),
                        year: None,
                        thumbnail: None,
                    }),
                    subtitle: "Album • Daft Punk".into(),
                },
            ],
        }
    }

    #[test]
    fn a_completion_row_splits_at_the_typed_part() {
        let (rows, _) = rows(&mixed());
        assert_eq!(
            (rows[0].title.as_str(), rows[0].rest.as_str()),
            ("daft pu", "nk")
        );
        assert_eq!(rows[1].kind, SuggestionKind::History);
    }

    #[test]
    fn result_rows_follow_the_completions_with_their_covers() {
        let (rows, covers) = rows(&mixed());
        let kinds: Vec<_> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            [
                SuggestionKind::Query,
                SuggestionKind::History,
                SuggestionKind::Artist,
                SuggestionKind::Song,
                SuggestionKind::Album
            ]
        );
        assert_eq!(covers[3].as_deref(), Some("https://example.test/song"));
        assert_eq!(rows[3].subtitle, "Song • Daft Punk");
    }

    #[test]
    fn picking_a_completion_searches_for_it() {
        assert_eq!(
            pick_of(&mixed(), 1),
            Some(Pick::Search("daft punk live".into()))
        );
    }

    #[test]
    fn picking_a_song_plays_it() {
        assert!(matches!(pick_of(&mixed(), 3), Some(Pick::Play(t)) if t.id.0 == "qe8Q7mjxjig"));
    }

    #[test]
    fn picking_an_artist_or_album_opens_its_page() {
        assert_eq!(
            pick_of(&mixed(), 2),
            Some(Pick::Open(Page::Artist {
                id: ArtistId("UCRr1xG_2WIDs18a6cIiCxeA".into())
            }))
        );
        assert_eq!(
            pick_of(&mixed(), 4),
            Some(Pick::Open(Page::Album {
                id: AlbumId("MPREb_dMHztPsrDgN".into())
            }))
        );
    }

    #[test]
    fn a_row_past_the_end_picks_nothing() {
        assert_eq!(pick_of(&mixed(), 5), None);
    }
}
