//! The pacing and the cache of the suggestions under the top bar's search field: while the
//! user types, which requests go when, which answer may show, and the last answers.
//!
//! Asked for while the typing goes on, signed in or not: from the third letter at once (one
//! or two letters once the typing pauses, `PAUSE`), then at most one request per `INTERVAL`
//! for the text by then, the last one for the final text. None for an empty field or a link
//! (Enter opens what it points at). An answer shows unless a newer one already does or the
//! text has moved off its query; the rows shown stay until the next answer replaces them.
//! The last few answers are kept (`CACHED`), so typing back to an earlier query shows its
//! suggestions at once. The rows, the keys, the motion and the one timer are the app's.

use std::collections::VecDeque;
use std::time::Duration;

use featherplay_ytm::{HomeItem, SearchSuggestions};

use crate::link;
use crate::place::Place;

/// How long the typing must pause before one or two letters are asked for.
pub const PAUSE: Duration = Duration::from_millis(180);
/// While the typing goes on, at most one request per this long: five a second.
pub const INTERVAL: Duration = Duration::from_millis(200);
/// From this many letters, the first request goes at once.
const MIN_LETTERS: usize = 3;
/// How many queries' answers are kept.
const CACHED: usize = 8;
/// At most this many completions, then this many results, as YouTube Music shows them.
const MAX_QUERIES: usize = 7;
const MAX_RESULTS: usize = 5;

/// What an edit of the field, or the field's timer, calls for.
#[derive(Debug, PartialEq)]
pub enum Next {
    /// Nothing to suggest for (an empty field, a link): no rows, no request, no timer.
    Hide,
    /// Asked for before: these, at once.
    Show(SearchSuggestions),
    /// Ask for `query` now, and start the timer for `INTERVAL`; the answer is edit `ask`'s.
    Fetch { ask: u64, query: String },
    /// Start the timer (again) for `PAUSE`: one or two letters wait for the typing to pause.
    Pause,
    /// Nothing now: the timer running asks for the text when it fires, or the text's
    /// request has gone already.
    Wait,
}

/// The typing's requests and their answers: which to ask for when, which answer may show,
/// and the last answers. No Slint in here: the tests drive it, the field runs its one timer.
#[derive(Default)]
pub struct Suggester {
    /// Edits so far.
    latest: u64,
    /// The edit whose rows (or none) show: an answer asked for before it never replaces them.
    shown: u64,
    /// The field's text, trimmed; empty while nothing is suggested.
    text: String,
    /// `text` is still to be asked for.
    pending: bool,
    /// The timer runs as the interval after a request, not as the pause.
    gated: bool,
    /// A request went since the field was emptied or the rows closed: no more pauses.
    asked: bool,
    cache: VecDeque<(String, SearchSuggestions)>,
}

impl Suggester {
    /// The field now reads `text`.
    pub fn edited(&mut self, text: &str) -> Next {
        self.latest += 1;
        let query = text.trim();
        if query.is_empty() || link::parse(query).is_some() {
            self.reset();
            return Next::Hide;
        }
        // Only a space at the end: asked for or shown already, or about to be.
        if query == self.text {
            return Next::Wait;
        }
        query.clone_into(&mut self.text);
        if let Some((_, cached)) = self.cache.iter().find(|(q, _)| q == query) {
            self.pending = false;
            self.shown = self.latest;
            return Next::Show(cached.clone());
        }
        self.pending = true;
        if self.gated {
            Next::Wait
        } else if self.asked || query.chars().count() >= MIN_LETTERS {
            self.ask()
        } else {
            Next::Pause
        }
    }

    /// The timer fired: the text typed since the last request, if any, is asked for now.
    pub fn due(&mut self) -> Next {
        if self.pending {
            self.ask()
        } else {
            self.gated = false;
            Next::Wait
        }
    }

    fn ask(&mut self) -> Next {
        self.pending = false;
        self.gated = true;
        self.asked = true;
        Next::Fetch {
            ask: self.latest,
            query: self.text.clone(),
        }
    }

    /// The answer to edit `ask`, for `query`: kept for later, and returned to show unless a
    /// newer answer shows already or the text no longer starts with `query`.
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
        let fresh = ask > self.shown && self.text.starts_with(query);
        if fresh {
            self.shown = ask;
        }
        fresh.then_some(answer)
    }

    /// The rows closed (Escape, a pick, a page change): answers on their way are dropped.
    pub fn close(&mut self) {
        self.latest += 1;
        self.reset();
    }

    fn reset(&mut self) {
        self.shown = self.latest;
        self.text.clear();
        self.pending = false;
        self.gated = false;
        self.asked = false;
    }
}

/// What a picked row does.
#[derive(Debug, PartialEq)]
pub enum Pick {
    /// Search for the completion.
    Search(String),
    /// Play the song or video, with its radio after it.
    Play(Box<featherplay_core::model::Track>),
    Open(Place),
}

/// What row `i` of `shown`'s rows (completions, then results) does.
pub fn pick_of(shown: &SearchSuggestions, i: usize) -> Option<Pick> {
    if let Some(q) = shown.queries.get(i) {
        return Some(Pick::Search(q.text.clone()));
    }
    let r = shown.results.get(i - shown.queries.len())?;
    Some(match &r.item {
        HomeItem::Track(t) => Pick::Play(Box::new(t.clone())),
        HomeItem::Album(a) => Pick::Open(Place::Album(a.id.clone())),
        HomeItem::Artist(a) => Pick::Open(Place::Artist(a.id.clone())),
        HomeItem::Playlist(p) => Pick::Open(Place::Playlist(p.id.clone())),
    })
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
    fn a_stale_answer_is_kept_for_later() {
        let mut s = Suggester::default();
        let first = ask_of(s.edited("daf"));
        s.edited("daft");
        let second = ask_of(s.due());
        s.answered(second, "daft", answer("daft"));
        s.answered(first, "daf", answer("daf"));
        assert_eq!(s.edited("daf"), Next::Show(answer("daf")));
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
        let ask = ask_of(s.edited("daf"));
        s.answered(ask, "daf", answer("daf"));
        s.edited("daft");
        assert_eq!(s.edited("daf "), Next::Show(answer("daf")));
    }

    #[test]
    fn only_the_last_few_answers_are_kept() {
        let mut s = Suggester::default();
        for i in 0..=CACHED {
            let q = format!("query {i}");
            let ask = ask_of(s.edited(&q));
            s.answered(ask, &q, answer(&q));
            // The interval passes before the next one.
            s.due();
        }
        assert!(
            matches!(s.edited("query 0"), Next::Fetch { .. }),
            "the oldest went"
        );
        assert!(matches!(s.edited("query 1"), Next::Show(_)));
    }

    /// `text` typed a letter every `every` ms on a fake clock, the one timer run as the field
    /// runs it: each request's time from the first key, and its query.
    fn requests(text: &str, every: u64) -> Vec<(u64, String)> {
        let ms = |d: Duration| d.as_millis() as u64;
        let keys: Vec<(u64, &str)> = text
            .char_indices()
            .enumerate()
            .map(|(n, (i, c))| (n as u64 * every, &text[..i + c.len_utf8()]))
            .collect();
        let mut s = Suggester::default();
        let (mut timer, mut asked) = (None, Vec::new());
        let mut keys = keys.into_iter().peekable();
        loop {
            // A timer due at a key's time fires first.
            let fires = match (timer, keys.peek()) {
                (None, None) => return asked,
                (Some(t), Some((k, _))) => t <= *k,
                (timer, _) => timer.is_some(),
            };
            let (now, next) = match timer.take_if(|_| fires) {
                Some(t) => (t, s.due()),
                None => {
                    let (k, typed) = keys.next().expect("a key is next");
                    (k, s.edited(typed))
                }
            };
            match next {
                Next::Fetch { query, .. } => {
                    asked.push((now, query));
                    timer = Some(now + ms(INTERVAL));
                }
                Next::Pause => timer = Some(now + ms(PAUSE)),
                Next::Hide => timer = None,
                Next::Show(_) | Next::Wait => {}
            }
        }
    }

    fn at(list: &[(u64, &str)]) -> Vec<(u64, String)> {
        list.iter().map(|(t, q)| (*t, q.to_string())).collect()
    }

    #[test]
    fn typing_asks_from_the_third_letter_then_at_most_every_interval() {
        // A key every 120 ms: "daf" at 240 ms, then one request per 200 ms for the text by
        // then ("daft " is "daft" again), the last 80 ms after the last key.
        assert_eq!(
            requests("daft punk", 120),
            at(&[
                (240, "daf"),
                (440, "daft"),
                (640, "daft p"),
                (840, "daft pu"),
                (1040, "daft punk"),
            ])
        );
    }

    #[test]
    fn quick_typing_keeps_to_the_interval() {
        assert_eq!(
            requests("the weeknd", 60),
            at(&[
                (120, "the"),
                (320, "the we"),
                (520, "the weekn"),
                (720, "the weeknd"),
            ])
        );
    }

    #[test]
    fn two_letters_are_asked_for_once_the_typing_pauses() {
        assert_eq!(requests("da", 120), at(&[(300, "da")]));
    }

    #[test]
    fn after_a_pause_the_next_letter_is_asked_for_at_once() {
        let mut s = Suggester::default();
        assert_eq!(s.edited("d"), Next::Pause);
        assert!(matches!(s.due(), Next::Fetch { .. }));
        assert_eq!(s.due(), Next::Wait, "the interval passed with nothing new");
        assert!(matches!(s.edited("da"), Next::Fetch { .. }));
    }

    #[test]
    fn a_cleared_field_starts_over() {
        let mut s = Suggester::default();
        ask_of(s.edited("daft"));
        s.due();
        assert_eq!(s.edited(""), Next::Hide);
        assert_eq!(s.edited("d"), Next::Pause);
    }

    #[test]
    fn an_older_prefix_answer_shows_while_the_typing_goes_on() {
        let mut s = Suggester::default();
        let ask = ask_of(s.edited("daf"));
        assert_eq!(s.edited("daft"), Next::Wait);
        assert_eq!(s.answered(ask, "daf", answer("daf")), Some(answer("daf")));
    }

    #[test]
    fn an_older_answer_never_replaces_a_newer_one() {
        let mut s = Suggester::default();
        let first = ask_of(s.edited("daf"));
        s.edited("daft");
        let second = ask_of(s.due());
        assert!(s.answered(second, "daft", answer("daft")).is_some());
        assert_eq!(s.answered(first, "daf", answer("daf")), None);
    }

    #[test]
    fn an_answer_for_text_no_longer_typed_is_dropped() {
        let mut s = Suggester::default();
        let ask = ask_of(s.edited("daft"));
        s.edited("daf");
        assert_eq!(s.answered(ask, "daft", answer("daft")), None);
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
            Some(Pick::Open(Place::Artist(ArtistId(
                "UCRr1xG_2WIDs18a6cIiCxeA".into()
            ))))
        );
        assert_eq!(
            pick_of(&mixed(), 4),
            Some(Pick::Open(Place::Album(AlbumId(
                "MPREb_dMHztPsrDgN".into()
            ))))
        );
    }

    #[test]
    fn a_row_past_the_end_picks_nothing() {
        assert_eq!(pick_of(&mixed(), 5), None);
    }
}
