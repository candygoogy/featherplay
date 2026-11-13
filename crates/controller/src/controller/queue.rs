//! The Up next panel's edits (play next, add to queue, reorder, remove, clear, skip to a
//! track) and where the queue came from ("Playing from …", and the page that shows it).

use std::collections::HashMap;
use std::time::Duration;

use featherplay_core::model::{LikeStatus, Track, TrackId};
use featherplay_core::player::Input;
use featherplay_core::queue::Queue;
use featherplay_core::tr;

use super::{Action, Controller};
use crate::place::Place;

#[derive(Debug)]
pub enum QueueAction {
    /// Indices are into the upcoming tracks, as the panel lists them.
    Move {
        from: usize,
        to: usize,
    },
    Remove(usize),
    Clear,
    PlayNext(Track),
    /// A whole list after the current track, in order (the sidebar menu's Play next).
    PlayNextAll(Vec<Track>),
    Enqueue(Vec<Track>),
    /// The quick search's Play next: `tracks` right after the current track, or right after
    /// `after` (what the same box set to play next before) while that's still to come, so
    /// several picks play in the order they were made (`play_after_inputs`).
    PlayAfter {
        tracks: Vec<Track>,
        after: Option<TrackId>,
    },
    /// Play the upcoming track at this index now.
    SkipTo(usize),
    /// `Action::Play` with where the tracks come from: "Liked songs", a playlist's title,
    /// and its page. Plain `Action::Play` names albums and radios by itself.
    PlayFrom {
        tracks: Vec<Track>,
        start: usize,
        radio: bool,
        from: PlayingFrom,
    },
    /// More of a list that was still arriving when a play from it started (a long
    /// playlist's later pages): into that queue, after its own tracks, or with `mix` (it
    /// was shuffled) mixed in among them. Dropped once the queue is from somewhere else.
    Extend {
        tracks: Vec<Track>,
        mix: bool,
        from: PlayingFrom,
    },
    /// The panel was opened or closed; remembered for the next run.
    PanelOpen(bool),
    /// Now playing's side panel shown or hidden (the bar's Up next button there): kept in the
    /// settings for the next run.
    SidePanelOpen(bool),
}

/// Where a queue came from: what the Up next panel and Now playing's header call it, and
/// the page that shows it, when one does (an album, a playlist, the history; not a radio).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayingFrom {
    /// "Homework", "Around the World radio", "Liked songs"; empty when unknown.
    pub label: String,
    pub page: Option<Place>,
}

impl PlayingFrom {
    /// A source with no page of its own: a radio, Autoplay.
    pub fn named(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            page: None,
        }
    }

    /// A source `page` shows.
    pub fn on_page(label: impl Into<String>, page: Place) -> Self {
        Self {
            label: label.into(),
            page: Some(page),
        }
    }
}

/// What the Up next panel shows besides the tracks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueueInfo {
    pub playing_from: PlayingFrom,
    /// Where the tracks a radio continuation added start, as an index into `upcoming`.
    pub autoplay_at: Option<usize>,
}

/// The song menu's Play next and Add to queue, as the Up next panel's edits; `None` for
/// other actions.
pub fn queue_edit(action: Action) -> Option<QueueAction> {
    match action {
        Action::PlayNext(t) => Some(QueueAction::PlayNext(t)),
        Action::AddToQueue(t) => Some(QueueAction::Enqueue(vec![t])),
        _ => None,
    }
}

/// What Play next and Add to queue ask of the player; `None` for the other edits.
pub fn add_input(edit: QueueAction) -> Option<Input> {
    match edit {
        QueueAction::PlayNext(t) => Some(Input::PlayNext(t)),
        QueueAction::PlayNextAll(tracks) => Some(Input::PlayNextAll(tracks)),
        QueueAction::Enqueue(tracks) => Some(Input::Enqueue(tracks)),
        _ => None,
    }
}

/// What puts `tracks` next in `queue`: right after the current track, then, when `after`
/// is still to come (by its id or its song version's), each moved to just after it, in
/// their order. Nothing for no tracks.
pub fn play_after_inputs(queue: &Queue, tracks: Vec<Track>, after: Option<&TrackId>) -> Vec<Input> {
    let count = tracks.len();
    if count == 0 {
        return Vec::new();
    }
    let at = after.and_then(|id| {
        queue.upcoming().iter().position(|t| {
            t.id == *id || t.counterpart.as_deref().is_some_and(|song| song.id == *id)
        })
    });
    let mut inputs = vec![Input::PlayNextAll(tracks)];
    // Once in, the tracks are upcoming 0..count and `after` is at `at + count`: each first
    // one moved to `at + count` lands right after it, behind the ones moved before.
    if let Some(at) = at {
        inputs.extend((0..count).map(|_| Input::MoveUpcoming {
            from: 0,
            to: at + count,
        }));
    }
    inputs
}

/// `input` with the ratings the user made this session on the tracks it brings. A page keeps
/// its tracks as they were fetched, so without this a song disliked since would come back
/// into the queue unrated, and play when the one before it ends.
pub fn with_likes(mut input: Input, likes: &HashMap<TrackId, LikeStatus>) -> Input {
    if likes.is_empty() {
        return input;
    }
    let tracks: &mut [Track] = match &mut input {
        Input::PlayTracks { tracks, .. }
        | Input::PlayNextAll(tracks)
        | Input::Enqueue(tracks)
        | Input::ExtendList { tracks, .. }
        | Input::ContinuationLoaded(_, tracks) => tracks,
        Input::PlayNext(track) => std::slice::from_mut(track),
        _ => &mut [],
    };
    for t in tracks {
        if let Some(like) = likes.get(&t.id) {
            t.like = *like;
        }
        // A song version takes the queue entry's place (`Track::into_song_version`).
        if let Some(song) = t.counterpart.as_deref_mut()
            && let Some(like) = likes.get(&song.id)
        {
            song.like = *like;
        }
    }
    input
}

/// What takes the player back to `track` at `at` (Undo after "Not for me"): when it's the
/// track before the current one, back to it, so the queue after it is as it was (the
/// position first goes to 0, so "previous" doesn't restart the current track instead);
/// when the user has moved on since, it plays again right away. Nothing when it's still
/// the current track.
pub fn back_to_inputs(queue: &Queue, track: &Track, at: Duration) -> Vec<Input> {
    let is = |t: Option<&Track>| t.is_some_and(|t| t.id == track.id);
    if is(queue.current()) {
        Vec::new()
    } else if is(queue.previous()) {
        vec![
            Input::SeekTo(Duration::ZERO),
            Input::Previous,
            Input::SeekTo(at),
        ]
    } else {
        vec![
            Input::PlayNext(track.clone()),
            Input::Next,
            Input::SeekTo(at),
        ]
    }
}

/// Where a queue started from `tracks` came from: a radio is named after the track it
/// started from, a queue from one album after the album, whose page shows it. Anything
/// else is unknown.
pub fn playing_from(tracks: &[Track], start: usize, radio: bool) -> PlayingFrom {
    if radio {
        return tracks
            .get(start)
            .or(tracks.first())
            .map(|t| PlayingFrom::named(format!("{} radio", t.display_title())))
            .unwrap_or_default();
    }
    let mut albums = tracks.iter().map(|t| t.album.as_ref());
    match albums.next() {
        Some(Some(first)) if albums.all(|a| a.is_some_and(|a| a.id == first.id)) => {
            let page = Place::Album(first.id.clone());
            PlayingFrom::on_page(first.title.clone(), page)
        }
        _ => PlayingFrom::default(),
    }
}

/// Where the queue the controller starts with came from (a restored session or `--play`).
pub fn initial_playing_from(initial: &[Input]) -> PlayingFrom {
    initial
        .iter()
        .find_map(|i| match i {
            Input::PlayTracks {
                tracks,
                start,
                radio,
            } => Some(playing_from(tracks, *start, *radio)),
            // A radio session starts with the track it was started from.
            Input::Restore(s) => Some(playing_from(&s.queue, 0, s.radio)),
            _ => None,
        })
        .unwrap_or_default()
}

impl Controller {
    pub(super) fn queue_action(&mut self, action: QueueAction) {
        match action {
            QueueAction::Move { from, to } => self.input(Input::MoveUpcoming { from, to }),
            QueueAction::Remove(i) => self.input(Input::RemoveUpcoming(i)),
            QueueAction::Clear => self.input(Input::ClearUpcoming),
            edit @ (QueueAction::PlayNext(_)
            | QueueAction::PlayNextAll(_)
            | QueueAction::Enqueue(_)) => {
                if self.player.queue().current().is_none() {
                    self.playing_from = PlayingFrom::default();
                }
                if let Some(input) = add_input(edit) {
                    self.input(input);
                }
            }
            QueueAction::PlayAfter { tracks, after } => {
                if self.player.queue().current().is_none() {
                    self.playing_from = PlayingFrom::default();
                }
                let inputs = play_after_inputs(self.player.queue(), tracks, after.as_ref());
                self.inputs(inputs);
            }
            QueueAction::SkipTo(i) => self.input(Input::SkipTo(i)),
            QueueAction::PlayFrom {
                tracks,
                start,
                radio,
                from,
            } => {
                log!("play {} tracks from {start} ({})", tracks.len(), from.label);
                self.playing_from = from;
                self.input(Input::PlayTracks {
                    tracks,
                    start,
                    radio,
                });
            }
            QueueAction::Extend { tracks, mix, from } => {
                if from != self.playing_from || self.player.queue().current().is_none() {
                    log!(
                        "{} more tracks of {}: the queue is from elsewhere now",
                        tracks.len(),
                        from.label
                    );
                    return;
                }
                self.input(Input::ExtendList { tracks, mix });
            }
            QueueAction::PanelOpen(open) => {
                if self.settings.queue_open != open {
                    self.settings.queue_open = open;
                    if let Err(e) = self.settings.save(&self.paths) {
                        log!("saving the settings failed: {}", e.kind());
                    }
                }
            }
            QueueAction::SidePanelOpen(open) => {
                if self.settings.nowplaying_side_open != open {
                    self.settings.nowplaying_side_open = open;
                    if let Err(e) = self.settings.save(&self.paths) {
                        log!("saving the settings failed: {}", e.kind());
                    }
                }
            }
        }
    }

    pub(super) fn queue_info(&self) -> QueueInfo {
        queue_info(self.player.queue(), &self.playing_from)
    }
}

/// The panel's label and divider for `queue`, started from `from`. A list's name stays
/// while its own tracks play; once its autoplay does, the label says so and every track
/// left is autoplay, so there's no divider (nor a page to open). A radio keeps its name in
/// its continuation.
pub fn queue_info(queue: &Queue, from: &PlayingFrom) -> QueueInfo {
    if !queue.is_radio() && queue.current_is_autoplay() {
        return QueueInfo {
            playing_from: PlayingFrom::named(tr!("Autoplay").to_string()),
            autoplay_at: None,
        };
    }
    QueueInfo {
        playing_from: from.clone(),
        autoplay_at: queue.autoplay_start(),
    }
}
