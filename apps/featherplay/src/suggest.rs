//! The suggestions under the top bar's search field (ui/top_bar.slint): while the user
//! types, YouTube Music's completions of the query and a few direct results (songs, albums,
//! artists, playlists). Picking a completion searches for it; picking a result plays it or
//! opens its page, as a row of the Search page does. Last, after a divider, up to three
//! settings that match (`pages::settings::search`), with the music's answer, or alone when
//! it can't come (offline); picking one opens Settings at its row.
//!
//! When to ask and which answer shows is `featherplay_controller::suggest` (`Suggester`). The
//! keys and the motion are Slint's; this module runs the field's one timer, the rows and
//! their covers. Nothing runs while closed.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use featherplay_controller::suggest::{Next, Pick, Suggester, pick_of};
use featherplay_ytm::{HomeItem, SearchSuggestions};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::app::Ctx;
use crate::controller::Action;
use crate::link;
use crate::pages::settings::search::{self as settings_search, Suggested};
use crate::ui::{AppWindow, Suggestion, SuggestionKind, TopBarData};

pub use featherplay_controller::suggest::{INTERVAL, PAUSE};

/// A result's cover, square, in logical pixels.
const COVER_PX: u32 = 40;

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
            HomeItem::Track(t) => (SuggestionKind::Song, t.display_title(), &t.thumbnail),
            HomeItem::Album(a) => (SuggestionKind::Album, a.title.as_str().into(), &a.thumbnail),
            HomeItem::Artist(a) => (SuggestionKind::Artist, a.name.as_str().into(), &a.thumbnail),
            HomeItem::Playlist(p) => (
                SuggestionKind::Playlist,
                p.title.as_str().into(),
                &p.thumbnail,
            ),
        };
        let row = Suggestion {
            kind,
            title: crate::ui_bridge::shown(&title),
            subtitle: crate::ui_bridge::shown(&r.subtitle),
            ..Suggestion::default()
        };
        (row, cover.clone())
    });
    completions.chain(results).unzip()
}

/// The rows for the settings suggested.
pub fn setting_rows(settings: &[Suggested]) -> Vec<Suggestion> {
    settings
        .iter()
        .map(|s| Suggestion {
            kind: SuggestionKind::Setting,
            title: s.title.into(),
            subtitle: s.subtitle.as_str().into(),
            ..Suggestion::default()
        })
        .collect()
}

#[derive(Default)]
struct State {
    suggester: Suggester,
    /// What the rows show, for a pick: the music's, then the settings'.
    shown: Option<SearchSuggestions>,
    settings: Vec<Suggested>,
    /// The pause or the interval (`Next`): a single shot, so nothing runs once it has fired.
    timer: Option<slint::Timer>,
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
    field.on_dismissed(|| {
        stop_timer();
        STATE.with(|s| s.borrow_mut().suggester.close());
    });
}

fn edited(ctx: &Rc<Ctx>, text: &str) {
    let next = STATE.with(|s| s.borrow_mut().suggester.edited(text));
    act(ctx, next);
}

fn act(ctx: &Rc<Ctx>, next: Next) {
    let Some(ui) = ctx.window() else { return };
    match next {
        Next::Hide => {
            stop_timer();
            hide(&ui);
        }
        Next::Show(s) => {
            show(ctx, &ui, s);
            // A kept answer is the whole text's.
            #[cfg(feature = "perf-harness")]
            crate::perf::mark(crate::perf::Mark::SuggestTyped);
        }
        // The rows shown stay until the new answer replaces them.
        Next::Fetch { ask, query } => {
            start_timer(ctx, INTERVAL);
            fetch(ctx, ask, query);
        }
        Next::Pause => start_timer(ctx, PAUSE),
        Next::Wait => {}
    }
}

fn start_timer(ctx: &Rc<Ctx>, after: Duration) {
    let c = ctx.clone();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let timer = s.timer.get_or_insert_with(slint::Timer::default);
        timer.start(slint::TimerMode::SingleShot, after, move || {
            let next = STATE.with(|s| s.borrow_mut().suggester.due());
            act(&c, next);
        });
    });
}

fn stop_timer() {
    STATE.with(|s| {
        if let Some(t) = &s.borrow().timer {
            t.stop();
        }
    });
}

/// Edit `ask`'s request for `query`; its answer shows if it's still the freshest.
fn fetch(ctx: &Rc<Ctx>, ask: u64, query: String) {
    let asked = query.clone();
    ctx.load(
        move |source| async move { source.search_suggestions(&asked).await },
        move |ctx, ui, result| match result {
            Ok(answer) => {
                let fresh = STATE.with(|s| s.borrow_mut().suggester.answered(ask, &query, answer));
                // The field may have been cleared or filled by a page since.
                let field = ui.global::<TopBarData>().get_query();
                let current = field.trim().starts_with(query.as_str());
                if let Some(answer) = fresh.filter(|_| current) {
                    show(ctx, ui, answer);
                    #[cfg(feature = "perf-harness")]
                    if field.trim() == query {
                        crate::perf::mark(crate::perf::Mark::SuggestTyped);
                    }
                }
            }
            Err(e) => {
                log!("search suggestions: {e}");
                // Offline: the settings that match show alone, unless music rows still show.
                let field = ui.global::<TopBarData>().get_query();
                let current = field.trim().starts_with(query.as_str());
                let music = STATE.with(|s| {
                    s.borrow()
                        .shown
                        .as_ref()
                        .is_some_and(|m| !m.queries.is_empty() || !m.results.is_empty())
                });
                if current && !music {
                    show(ctx, ui, SearchSuggestions::default());
                }
            }
        },
    );
}

fn show(ctx: &Ctx, ui: &AppWindow, s: SearchSuggestions) {
    let field = ui.global::<TopBarData>();
    let (mut items, mut covers) = rows(&s);
    // The settings for the text as it is now: they need no request.
    let settings = settings_search::suggestions(ui, field.get_query().trim());
    let settings_from = if settings.is_empty() {
        -1
    } else {
        items.len() as i32
    };
    items.extend(setting_rows(&settings));
    covers.resize(items.len(), None);
    let model = Rc::new(VecModel::from(items));
    field.set_completions(s.queries.len() as i32);
    field.set_settings_from(settings_from);
    field.set_suggestions(ModelRc::from(model.clone()));
    field.set_highlighted(-1);
    field.set_suggesting(model.row_count() > 0);
    ctx.covers
        .fill(&model, covers, COVER_PX, |row, img| row.cover = img);
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        st.shown = Some(s);
        st.settings = settings;
    });
    #[cfg(feature = "perf-harness")]
    crate::perf::mark(crate::perf::Mark::Suggest);
}

fn hide(ui: &AppWindow) {
    let field = ui.global::<TopBarData>();
    field.set_suggesting(false);
    field.set_highlighted(-1);
    field.set_completions(0);
    field.set_settings_from(-1);
    field.set_suggestions(ModelRc::default());
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.shown = None;
        s.settings.clear();
    });
}

/// The page changed: the rows close, and an answer on its way is dropped.
pub fn close(ui: &AppWindow) {
    stop_timer();
    STATE.with(|s| s.borrow_mut().suggester.close());
    hide(ui);
}

fn pick(ctx: &Rc<Ctx>, i: usize) {
    let (what, setting) = STATE.with(|s| {
        let s = s.borrow();
        let music = s
            .shown
            .as_ref()
            .map_or(0, |m| m.queries.len() + m.results.len());
        match i.checked_sub(music) {
            Some(j) => (None, s.settings.get(j).map(|s| s.id)),
            None => (s.shown.as_ref().and_then(|shown| pick_of(shown, i)), None),
        }
    });
    stop_timer();
    STATE.with(|s| s.borrow_mut().suggester.close());
    let by_key = ctx.window().is_some_and(|ui| {
        let field = ui.global::<TopBarData>();
        field.set_suggesting(false);
        field.get_picked_by_key()
    });
    // A setting: Settings opens at its row, and the field empties.
    if let Some(id) = setting {
        if let Some(ui) = ctx.window() {
            close(&ui);
            ui.global::<TopBarData>().set_query("".into());
        }
        settings_search::open_at(ctx, id, by_key);
        return;
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
        Some(Pick::Open(place)) => ctx.navigate(place.into()),
        None => {}
    }
}

/// `--snapshot search-suggestions`: Ctrl+K into the field, `text` in it ("daft pu"), and the
/// recorded suggestions asked for at once: they show once the loads have settled.
pub fn show_for_snapshot(ctx: &Rc<Ctx>, text: &str) {
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
    ui.global::<TopBarData>().set_query(text.into());
    edited(ctx, text);
}

#[cfg(test)]
mod tests {
    use featherplay_core::model::{AlbumId, ArtistId, Track, TrackId};
    use featherplay_ytm::{AlbumSummary, ArtistSummary, QuerySuggestion, SuggestedResult};

    use super::*;

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
}
