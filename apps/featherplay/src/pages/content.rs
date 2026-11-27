//! What the content pages share: a loaded track list with its rows, the playing-row
//! mark and lazy covers; meta lines; error texts; playing a whole album, playlist or artist.
//! The formatting helpers are pure and tested below.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use featherplay_controller::place::Place;
use featherplay_controller::playlist_edit::{self, Moved};
use featherplay_core::model::{AlbumId, ArtistRef, PlaylistId, Track, TrackId};
use featherplay_core::{tr, trn};
use featherplay_ytm::{ApiError, EntryId, PlaylistPart};
use slint::{Image, Model, ModelRc, SharedString, VecModel};

use crate::app::Ctx;
use crate::controller::{Action, PlayerState, PlayingFrom, QueueAction};
use crate::covers::Covers;
use crate::pages::{ahead, playlist};
use crate::song_menu;
use crate::ui::{CardItem, ItemKind, TrackRow};
use crate::ui_bridge::{model, to_rows};

/// Row covers, in logical pixels.
pub const ROW_COVER: u32 = 40;
/// Card covers: a card is 150 to 200 px wide.
pub const CARD_COVER: u32 = 200;
/// The header cover of an album, playlist or artist.
pub const HEADER_COVER: u32 = 232;

/// A page's tracks and the rows showing them, in the same order.
pub struct TrackList {
    pub tracks: Vec<Track>,
    pub rows: Rc<VecModel<TrackRow>>,
    /// The Up next panel's "Playing from" for a play from this list ("Liked songs", a
    /// playlist's title) and its page; an empty label lets the controller name it (an
    /// album, a radio).
    pub from: PlayingFrom,
    asked: Vec<bool>,
    playing: Option<Playing>,
    /// While the list's songs are still arriving (a long playlist's later pages): where a
    /// play from it is noted, so the songs to come join its queue.
    pub arriving: Option<Arriving>,
    /// A playlist's: each song's entry in it, in the order of `tracks`, what a move names
    /// (`playlist_edit::move_rows`); empty elsewhere, or where they weren't read.
    pub entries: Vec<Option<EntryId>>,
    /// A playlist the account may edit (`Playlist::editable`): its songs can be moved.
    pub editable: bool,
}

thread_local! {
    /// Plays started from a list or a card, counted: the songs still arriving for a list
    /// join only the queue of the last one (`Arriving`).
    static PLAYS: Cell<u64> = const { Cell::new(0) };
}

/// Notes a play starting from a list or a card; returns its number.
fn started_play() -> u64 {
    PLAYS.with(|p| {
        p.set(p.get() + 1);
        p.get()
    })
}

/// A list whose songs are still arriving (a long playlist's later pages), shared by what
/// shows it and what fetches it: the play from it, once one starts, that the songs to come
/// join. A play from it while they arrive plays the songs there; the rest follow in the
/// queue as they come, in order after a Play, mixed in after a Shuffle.
#[derive(Clone, Debug, Default)]
pub struct Arriving(Rc<RefCell<Option<Joined>>>);

#[derive(Debug)]
struct Joined {
    /// Its number (`started_play`).
    play: u64,
    /// Shuffled: the songs to come are mixed in.
    mix: bool,
    from: PlayingFrom,
}

impl Arriving {
    /// Play `play` started from the list.
    pub fn played(&self, play: u64, mix: bool, from: PlayingFrom) {
        *self.0.borrow_mut() = Some(Joined { play, mix, from });
    }

    /// Hands `tracks` to the queue the play from this list started, while that's the last
    /// play started (the controller drops them too if the queue is from somewhere else
    /// now). Returns whether there's such a play.
    pub fn join(&self, ctx: &Ctx, tracks: &[Track]) -> bool {
        let joined = self.0.borrow();
        let Some(j) = joined.as_ref().filter(|j| j.play == PLAYS.with(Cell::get)) else {
            return false;
        };
        if !tracks.is_empty() {
            song_menu::remember(tracks);
            ctx.send(Action::Queue(QueueAction::Extend {
                tracks: tracks.to_vec(),
                mix: j.mix,
                from: j.from.clone(),
            }));
        }
        true
    }
}

/// What a list marks as playing: the current track, and the music video it plays in the
/// place of when it's that video's song version (`Track::into_song_version`), since the
/// list shows the video.
#[derive(Clone, Debug, PartialEq)]
pub struct Playing {
    id: TrackId,
    video: Option<TrackId>,
}

impl Playing {
    fn is(&self, id: &TrackId) -> bool {
        self.id == *id || self.video.as_ref() == Some(id)
    }

    /// Whether this is the mark `playing` makes for `track`, without making one.
    fn of(&self, track: &Track) -> bool {
        self.id == track.id && self.video.as_ref() == track.video_version()
    }
}

impl TrackList {
    /// Builds the rows (numbered from 1), marks the playing one and lets the song menu
    /// find the tracks. Returns the model to hand to Slint.
    pub fn new(tracks: Vec<Track>, ctx: &Ctx) -> (TrackList, ModelRc<TrackRow>) {
        song_menu::remember(&tracks);
        let (rows, model) = model(to_rows(&tracks));
        let mut list = TrackList {
            asked: vec![false; tracks.len()],
            tracks,
            rows,
            from: PlayingFrom::default(),
            playing: None,
            arriving: None,
            entries: Vec::new(),
            editable: false,
        };
        list.follow(ctx.player().as_ref());
        (list, model)
    }

    /// Shows `tracks` in place of the current ones, in the same model: only the rows that
    /// changed are set, so a list on screen keeps its scroll position and doesn't flash. A
    /// song still in the list keeps the cover already loaded for it, wherever it moved.
    ///
    /// Returns the rows to load a cover for now (`need_cover`): a row Slint has already made
    /// asks only once (`init`), so a new song in its place (the newest at History's top)
    /// would otherwise stay without one.
    pub fn replace(&mut self, tracks: Vec<Track>, state: Option<&PlayerState>) -> Vec<usize> {
        song_menu::remember(&tracks);
        // Only the rows asked for can have one.
        let mut covers: HashMap<SharedString, Image> = HashMap::new();
        for (i, _) in self.asked.iter().enumerate().filter(|(_, asked)| **asked) {
            if let Some(row) = self.rows.row_data(i)
                && row.cover.size().width > 0
            {
                covers.insert(row.id, row.cover);
            }
        }
        let mut asked = Vec::with_capacity(tracks.len());
        for (i, mut row) in to_rows(&tracks).into_iter().enumerate() {
            let cover = covers.get(row.id.as_str());
            asked.push(cover.is_some());
            if let Some(img) = cover {
                row.cover = img.clone();
            }
            if i < self.rows.row_count() {
                if self.rows.row_data(i).as_ref() != Some(&row) {
                    self.rows.set_row_data(i, row);
                }
            } else {
                self.rows.push(row);
            }
        }
        while self.rows.row_count() > tracks.len() {
            self.rows.remove(self.rows.row_count() - 1);
        }
        let made = std::mem::replace(&mut self.asked, asked);
        self.tracks = tracks;
        self.entries.clear();
        self.playing = None;
        self.follow(state);
        (0..self.asked.len())
            .filter(|&i| !self.asked[i] && made.get(i) == Some(&true))
            .collect()
    }

    /// Adds `tracks` after the rows shown (a playlist's next page), numbered on from them.
    /// The rows shown stay as they are, so the list keeps its scroll position.
    pub fn append(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        song_menu::remember(&tracks);
        let base = self.tracks.len();
        let playing = self.playing.as_ref();
        let rows: Vec<TrackRow> = to_rows(&tracks)
            .into_iter()
            .zip(&tracks)
            .enumerate()
            .map(|(k, (mut row, t))| {
                row.index = (base + k + 1).to_string().into();
                row.playing = playing.is_some_and(|p| p.is(&t.id));
                row
            })
            .collect();
        self.rows.extend(rows);
        self.asked.resize(base + tracks.len(), false);
        self.tracks.extend(tracks);
    }

    /// Moves the rows at `picked` together to right before the row at `before`, or to the
    /// end (`playlist_edit::move_rows`): the songs, their entries and the rows shown, which
    /// keep their covers and marks. The moves to send, or `None` when nothing moved.
    pub fn move_rows(&mut self, picked: &[usize], before: Option<usize>) -> Option<Moved> {
        let mut order: Vec<usize> = (0..self.tracks.len()).collect();
        let moved = playlist_edit::move_rows(&mut order, &mut self.entries, picked, before)?;
        self.reorder(&order);
        Some(moved)
    }

    /// Puts the list back as it was before `moved` (its write failed); `false` if it can't.
    pub fn put_back(&mut self, moved: &Moved) -> bool {
        let mut order: Vec<usize> = (0..self.tracks.len()).collect();
        if !moved.put_back(&mut order, &mut self.entries) {
            return false;
        }
        self.reorder(&order);
        true
    }

    /// Row `i` becomes the one that was at `order[i]`: the songs, their covers asked and the
    /// rows shown (numbered by their new place). Only the rows that changed are set, and
    /// nothing is copied but the rows' data.
    fn reorder(&mut self, order: &[usize]) {
        // The rows that move lie between the first and the last that changed place.
        let changed = |(i, &was): (usize, &usize)| i != was;
        let Some(lo) = order.iter().enumerate().position(changed) else {
            return;
        };
        let hi = order.len()
            - order
                .iter()
                .enumerate()
                .rev()
                .position(changed)
                .unwrap_or(0);
        let window: Vec<usize> = order[lo..hi].iter().map(|&was| was - lo).collect();
        permute(&mut self.tracks[lo..hi], &window);
        permute(&mut self.asked[lo..hi], &window);
        let mut rows: Vec<Option<TrackRow>> = (lo..hi).map(|i| self.rows.row_data(i)).collect();
        for (k, &was) in window.iter().enumerate() {
            if k != was
                && let Some(mut row) = rows[was].take()
            {
                row.index = (lo + k + 1).to_string().into();
                self.rows.set_row_data(lo + k, row);
            }
        }
    }

    /// Resolves the first row's stream ahead: when a list opens, it's the likeliest click
    /// and what Play starts with. Only that row (the controller bounds resolves ahead).
    pub fn hint_first(&self, ctx: &Ctx) {
        if let Some(t) = self.tracks.first() {
            ctx.send(Action::Prefetch {
                id: t.id.clone(),
                track: Some(t.clone()),
            });
        }
    }

    /// Loads every row's cover now (short lists).
    pub fn fill_covers(&mut self, covers: &Covers) {
        for i in 0..self.tracks.len() {
            self.need_cover(covers, i);
        }
    }

    /// Loads row `i`'s cover once (long lists ask as rows appear).
    pub fn need_cover(&mut self, covers: &Covers, i: usize) {
        let Some(asked) = self.asked.get_mut(i) else {
            return;
        };
        if *asked {
            return;
        }
        *asked = true;
        let Some(url) = self.tracks[i].thumbnail.as_deref() else {
            return;
        };
        let weak = Rc::downgrade(&self.rows);
        let id = self.tracks[i].id.0.clone();
        covers.load(url, ROW_COVER, move |img| {
            let Some(rows) = weak.upgrade() else { return };
            // The row is still the one the cover was asked for.
            if let Some(mut row) = rows.row_data(i)
                && row.id.as_str() == id
            {
                row.cover = img;
                rows.set_row_data(i, row);
            }
        });
    }

    /// Marks the row of `state`'s current track and clears the old mark. The player's state
    /// comes with every position: the same track again makes no mark, so it costs nothing.
    pub fn follow(&mut self, state: Option<&PlayerState>) {
        let current = state.and_then(|s| s.current.as_ref());
        let same = match (&self.playing, current) {
            (Some(p), Some(t)) => p.of(t),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.mark_playing(playing(state).as_ref());
        }
    }

    /// Marks the rows `playing` names and clears the old mark; cheap when unchanged.
    fn mark_playing(&mut self, playing: Option<&Playing>) {
        if self.playing.as_ref() == playing {
            return;
        }
        for (i, t) in self.tracks.iter().enumerate() {
            let now = playing.is_some_and(|p| p.is(&t.id));
            let was = self.playing.as_ref().is_some_and(|p| p.is(&t.id));
            if now != was
                && let Some(mut row) = self.rows.row_data(i)
            {
                row.playing = now;
                self.rows.set_row_data(i, row);
            }
        }
        self.playing = playing.cloned();
    }

    /// Plays the list from `start`.
    pub fn play(&self, ctx: &Ctx, start: usize, radio: bool) {
        if start < self.tracks.len() {
            self.send_play(ctx, self.tracks.clone(), start, radio, false);
        }
    }

    /// Plays track `i` alone, with its radio after it (search results, Home cards).
    pub fn play_one(&self, ctx: &Ctx, i: usize) {
        if let Some(t) = self.tracks.get(i) {
            started_play();
            ctx.send(Action::Play {
                tracks: vec![t.clone()],
                start: 0,
                radio: true,
            });
        }
    }

    /// Plays the list in a random order.
    pub fn shuffle(&self, ctx: &Ctx) {
        if !self.tracks.is_empty() {
            self.send_play(ctx, shuffled(&self.tracks, seed()), 0, false, true);
        }
    }

    /// Plays `tracks`, labelled with `from` when the list has one. While the list's songs
    /// are still arriving, the ones to come join this queue (`mix`: mixed in).
    fn send_play(&self, ctx: &Ctx, tracks: Vec<Track>, start: usize, radio: bool, mix: bool) {
        let play = started_play();
        if let Some(arriving) = &self.arriving {
            arriving.played(play, mix, self.from.clone());
        }
        ctx.send(if self.from.label.is_empty() {
            Action::Play {
                tracks,
                start,
                radio,
            }
        } else {
            Action::Queue(QueueAction::PlayFrom {
                tracks,
                start,
                radio,
                from: self.from.clone(),
            })
        });
    }

    /// Flips the like of row `i` (the heart in the row).
    pub fn toggle_like(&self, ctx: &Ctx, i: usize) {
        let Some(row) = self.rows.row_data(i) else {
            return;
        };
        song_menu::set_like(ctx, &TrackId(row.id.to_string()), !row.liked);
    }
}

/// Shows `tracks` in the list `slot` holds, in place (`TrackList::replace`), asking for the
/// covers that leaves out; with no list there yet, hands `tracks` back to make one.
pub fn refill(slot: &mut Option<TrackList>, tracks: Vec<Track>, ctx: &Ctx) -> Option<Vec<Track>> {
    let Some(list) = slot.as_mut() else {
        return Some(tracks);
    };
    for i in list.replace(tracks, ctx.player().as_ref()) {
        list.need_cover(&ctx.covers, i);
    }
    None
}

/// What the lists mark as playing: the current track.
fn playing(state: Option<&PlayerState>) -> Option<Playing> {
    let t = state.and_then(|s| s.current.as_ref())?;
    Some(Playing {
        id: t.id.clone(),
        video: t.video_version().cloned(),
    })
}

/// Sets `liked` on every row of `id` in `rows`.
pub fn set_liked(rows: &ModelRc<TrackRow>, id: &str, liked: bool) {
    for i in 0..rows.row_count() {
        if let Some(mut row) = rows.row_data(i)
            && row.id.as_str() == id
            && row.liked != liked
        {
            row.liked = liked;
            rows.set_row_data(i, row);
        }
    }
}

/// `"1 song"`, `"16 songs"`.
pub fn songs(n: usize) -> String {
    trn!("{n} song" | "{n} songs" % n)
}

/// The length of a list: `"1 hour 14 minutes"`, `"48 minutes"`, `"1 minute"`. `None` when
/// no track has a duration. No comma inside, so it can follow a song count after one.
pub fn total_length(tracks: &[Track]) -> Option<String> {
    let total: Duration = tracks.iter().filter_map(|t| t.duration).sum();
    if total.is_zero() {
        return None;
    }
    let minutes = (total.as_secs() + 30) / 60;
    let (h, m) = (minutes / 60, minutes % 60);
    let hours = |h: u64| trn!("{n} hour" | "{n} hours" % h);
    let minutes = |m: u64| trn!("{n} minute" | "{n} minutes" % m);
    Some(match (h, m) {
        (0, m) => minutes(m.max(1)),
        (h, 0) => hours(h),
        (h, m) => format!("{} {}", hours(h), minutes(m)),
    })
}

/// Joins the non-empty parts with commas, as the cards' subtitles do: `"Album, 1996"`.
pub fn meta<S: AsRef<str>>(parts: &[S]) -> String {
    parts
        .iter()
        .map(AsRef::as_ref)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// An album whose tracks aren't all by the album's artists (a compilation, or "Various
/// Artists"): its track list shows an artist column.
pub fn is_compilation(album_artists: &[ArtistRef], tracks: &[Track]) -> bool {
    let names = |a: &[ArtistRef]| a.iter().map(|r| r.name.to_lowercase()).collect::<Vec<_>>();
    let album = names(album_artists);
    if album.iter().any(|n| n == "various artists") {
        return true;
    }
    tracks
        .iter()
        .any(|t| !t.artists.is_empty() && names(&t.artists) != album)
}

/// A copy of `tracks` in a random order (xorshift; good enough for a shuffle button).
pub fn shuffled(tracks: &[Track], seed: u64) -> Vec<Track> {
    let mut out = tracks.to_vec();
    let mut x = seed | 1;
    for i in (1..out.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.swap(i, (x % (i as u64 + 1)) as usize);
    }
    out
}

fn seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}

/// What a page says when its data didn't load, under its "This didn't load" title (the page
/// adds Try again). The details go to the log. Pages that belong to the account don't load
/// signed out: they ask to sign in instead (`AccountGate`).
pub fn error_text(what: &str, e: &ApiError) -> String {
    log!("{what}: {e}");
    match e {
        ApiError::AuthExpired => tr!("Your session has ended. Sign in to see this.").into(),
        ApiError::SignInRequired => tr!("Sign in to see this.").into(),
        ApiError::Network(_) => {
            tr!("Can't reach the music service. Check the connection, then try again.").into()
        }
        ApiError::NotFound => tr!("This isn't available.").into(),
        ApiError::Parse(_) => tr!("The music service sent something unexpected.").into(),
    }
}

/// A card's play button: plays the album, playlist or artist without opening it.
/// Songs play from their page, which knows the track.
pub fn play_card(ctx: &Rc<Ctx>, item: &CardItem) {
    let id = item.id.to_string();
    match item.kind {
        ItemKind::Album => play_album(ctx, AlbumId(id)),
        ItemKind::Playlist => play_playlist(ctx, PlaylistId(id)),
        ItemKind::Artist => play_artist(ctx, featherplay_core::model::ArtistId(id)),
        ItemKind::Song => {}
    }
}

/// Plays a whole album, from the start (one call, or its answer read ahead).
pub fn play_album(ctx: &Rc<Ctx>, id: AlbumId) {
    let ahead = ahead::album(ctx, &id);
    ctx.load(
        move |source| async move {
            ahead::or_fetch(ahead, ahead::Answer::album, source.album(&id)).await
        },
        |ctx, _, result| match result {
            Ok(album) => {
                crate::sidebar::opened(ctx, crate::sidebar::album_entry(&album.summary));
                play_tracks(ctx, album.tracks)
            }
            Err(e) => log!("play album: {e}"),
        },
    );
}

/// Plays a whole playlist from the start: its first page as soon as it's read, the rest
/// joining the queue as it comes (`Arriving`).
pub fn play_playlist(ctx: &Rc<Ctx>, id: PlaylistId) {
    stream_playlist(ctx, id, false);
}

/// Plays a whole playlist in a random order, as its page's Shuffle does: the first page
/// shuffled as soon as it's read, the rest mixed in as it comes (Home's "Shuffle Liked
/// songs").
pub fn shuffle_playlist(ctx: &Rc<Ctx>, id: PlaylistId) {
    stream_playlist(ctx, id, true);
}

fn stream_playlist(ctx: &Rc<Ctx>, id: PlaylistId, mix: bool) {
    let arriving = Arriving::default();
    let mut from = None;
    let wanted = id.clone();
    let made = crate::playlist_menu::made_here(&id.0);
    let ahead = ahead::playlist(ctx, &id);
    ctx.stream(
        move |source, parts| playlist::fetch(source, wanted, made, ahead, parts),
        move |ctx, _, part| match part {
            playlist::Part::Page(PlaylistPart::First(p), _) => {
                crate::sidebar::opened(ctx, crate::sidebar::playlist_entry(&p.summary));
                let label = if playlist::is_liked_songs(&id) {
                    playlist::liked_songs().into()
                } else {
                    p.summary.title.clone()
                };
                let page = Place::Playlist(id.clone());
                from = Some(PlayingFrom::on_page(label, page));
                start_or_join(ctx, &arriving, from.as_ref(), p.tracks, mix)
            }
            playlist::Part::Page(PlaylistPart::More { tracks, .. }, _) => {
                start_or_join(ctx, &arriving, from.as_ref(), tracks, mix)
            }
            playlist::Part::End(result, _) => {
                if let Err(e) = result {
                    log!("play playlist: {e}");
                }
                ControlFlow::Break(())
            }
        },
    );
}

/// A card's playlist page: the first songs that arrive start the queue, the later ones
/// join it (`mix`: shuffled, the later ones mixed in). Stops once the queue is from elsewhere.
fn start_or_join(
    ctx: &Ctx,
    arriving: &Arriving,
    from: Option<&PlayingFrom>,
    tracks: Vec<Track>,
    mix: bool,
) -> ControlFlow<()> {
    let started = arriving.0.borrow().is_some();
    if started {
        return if arriving.join(ctx, &tracks) {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        };
    }
    let Some(from) = from else {
        return ControlFlow::Break(());
    };
    if !tracks.is_empty() {
        arriving.played(started_play(), mix, from.clone());
        song_menu::remember(&tracks);
        let tracks = if mix {
            shuffled(&tracks, seed())
        } else {
            tracks
        };
        ctx.send(Action::Queue(QueueAction::PlayFrom {
            tracks,
            start: 0,
            radio: false,
            from: from.clone(),
        }));
    }
    ControlFlow::Continue(())
}

/// Plays an artist's top songs (one call, or its answer read ahead).
pub fn play_artist(ctx: &Rc<Ctx>, id: featherplay_core::model::ArtistId) {
    let ahead = ahead::artist(ctx, &id);
    ctx.load(
        move |source| async move {
            ahead::or_fetch(ahead, ahead::Answer::artist, source.artist(&id)).await
        },
        |ctx, _, result| match result {
            Ok(artist) => {
                if artist.top_songs.is_empty() {
                    return;
                }
                started_play();
                song_menu::remember(&artist.top_songs);
                let page = Place::Artist(artist.summary.id.clone());
                ctx.send(Action::Queue(QueueAction::PlayFrom {
                    tracks: artist.top_songs,
                    start: 0,
                    radio: false,
                    from: PlayingFrom::on_page(artist.summary.name, page),
                }));
            }
            Err(e) => log!("play artist: {e}"),
        },
    );
}

fn play_tracks(ctx: &Ctx, tracks: Vec<Track>) {
    if !tracks.is_empty() {
        started_play();
        song_menu::remember(&tracks);
        ctx.send(Action::Play {
            tracks,
            start: 0,
            radio: false,
        });
    }
}

/// Makes `list[i]` the item that was at `order[i]` (`order` a permutation of `list`'s
/// places), in place: by swaps along the permutation's cycles.
fn permute<T>(list: &mut [T], order: &[usize]) {
    let mut placed = vec![false; order.len()];
    for start in 0..order.len() {
        let mut i = start;
        while !placed[i] {
            placed[i] = true;
            let was = order[i];
            if was == start {
                break;
            }
            list.swap(i, was);
            i = was;
        }
    }
}

#[cfg(test)]
mod tests {
    use featherplay_core::model::ArtistId;

    use super::*;

    fn track(id: &str, artists: &[&str], secs: u64) -> Track {
        Track {
            artists: artists.iter().map(|a| artist(a)).collect(),
            duration: (secs > 0).then(|| Duration::from_secs(secs)),
            ..crate::testing::track(id)
        }
    }

    fn artist(name: &str) -> ArtistRef {
        ArtistRef {
            id: Some(ArtistId(format!("UC{name}"))),
            name: name.into(),
        }
    }

    #[test]
    fn total_length_reads_like_a_sentence() {
        assert_eq!(total_length(&[track("a", &[], 0)]), None);
        assert_eq!(total_length(&[track("a", &[], 20)]).unwrap(), "1 minute");
        assert_eq!(
            total_length(&[track("a", &[], 1500), track("b", &[], 1380)]).unwrap(),
            "48 minutes"
        );
        assert_eq!(total_length(&[track("a", &[], 3600)]).unwrap(), "1 hour");
        assert_eq!(
            total_length(&[track("a", &[], 3600 + 14 * 60 + 10)]).unwrap(),
            "1 hour 14 minutes"
        );
        assert_eq!(
            total_length(&[track("a", &[], 2 * 3600 + 60)]).unwrap(),
            "2 hours 1 minute"
        );
    }

    #[test]
    fn meta_skips_empty_parts() {
        assert_eq!(meta(&["Album", "", "1996"]), "Album, 1996");
        assert_eq!(meta::<&str>(&[]), "");
        assert_eq!(songs(1), "1 song");
        assert_eq!(songs(16), "16 songs");
    }

    #[test]
    fn compilation_when_tracks_have_other_artists() {
        let album = [artist("Daft Punk")];
        let own = [track("a", &["Daft Punk"], 1), track("b", &["daft punk"], 1)];
        assert!(!is_compilation(&album, &own));
        // Tracks without artists inherit the album's.
        assert!(!is_compilation(&album, &[track("a", &[], 1)]));
        let mixed = [track("a", &["Daft Punk"], 1), track("b", &["Kraftwerk"], 1)];
        assert!(is_compilation(&album, &mixed));
        let feat = [track("a", &["Daft Punk", "Pharrell"], 1)];
        assert!(is_compilation(&album, &feat));
        assert!(is_compilation(&[artist("Various Artists")], &own));
    }

    #[test]
    fn replacing_the_tracks_updates_rows_in_place_and_keeps_loaded_covers() {
        let old = vec![track("a", &[], 1), track("b", &[], 1)];
        let (rows, model) = model(to_rows(&old));
        let mut list = TrackList {
            asked: vec![true, true],
            tracks: old,
            rows,
            from: PlayingFrom::default(),
            playing: None,
            arriving: None,
            entries: Vec::new(),
            editable: false,
        };
        let cover = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(2, 2));
        for i in 0..2 {
            let mut row = list.rows.row_data(i).unwrap();
            row.cover = cover.clone();
            list.rows.set_row_data(i, row);
        }

        // A new song at the top: the others move down a row. The top row, made already,
        // asks for the new song's cover; the row added under them asks when it's made.
        let ask = list.replace(
            vec![track("c", &[], 1), track("a", &[], 1), track("b", &[], 1)],
            None,
        );
        assert_eq!(ask, [0]);
        // The model Slint holds is the same one, updated.
        let ids: Vec<String> = (0..model.row_count())
            .map(|i| model.row_data(i).unwrap().id.to_string())
            .collect();
        assert_eq!(ids, ["c", "a", "b"]);
        let size = |i: usize| model.row_data(i).unwrap().cover.size();
        assert_eq!(size(0).width, 0, "the new song has no cover yet");
        assert_eq!(size(1).width, 2, "a's cover moved with it");
        assert_eq!(size(2).width, 2, "b's cover moved with it");
        assert_eq!(list.asked, [false, true, true]);

        // Fewer songs: the extra rows go.
        assert!(list.replace(vec![track("a", &[], 1)], None).is_empty());
        assert_eq!(model.row_count(), 1);
        assert_eq!(size(0).width, 2);
    }

    /// A playlist's next page: its songs go after the rows shown, numbered on from them,
    /// and the rows shown stay as they were (their covers too), so nothing on screen moves.
    #[test]
    fn appended_tracks_follow_the_rows_shown_numbered_on() {
        let old = vec![track("a", &[], 1), track("b", &[], 1)];
        let (rows, model) = model(to_rows(&old));
        let mut list = TrackList {
            asked: vec![true, false],
            tracks: old,
            rows,
            from: PlayingFrom::default(),
            playing: None,
            arriving: None,
            entries: Vec::new(),
            editable: false,
        };
        let cover = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(2, 2));
        let mut row = list.rows.row_data(0).unwrap();
        row.cover = cover;
        list.rows.set_row_data(0, row);
        list.mark_playing(Some(&Playing {
            id: TrackId("c".into()),
            video: None,
        }));

        list.append(vec![track("c", &[], 1), track("d", &[], 1)]);

        let row = |i: usize| model.row_data(i).unwrap();
        let shown: Vec<(String, String)> = (0..model.row_count())
            .map(|i| (row(i).id.to_string(), row(i).index.to_string()))
            .collect();
        assert_eq!(
            shown,
            [("a", "1"), ("b", "2"), ("c", "3"), ("d", "4")].map(|(a, b)| (a.into(), b.into()))
        );
        assert_eq!(row(0).cover.size().width, 2, "a row shown was made again");
        assert!(row(2).playing, "the playing song arrived unmarked");
        assert!(!row(3).playing);
        assert_eq!(list.asked, [true, false, false, false]);
        assert_eq!(list.tracks.len(), 4);
    }

    /// The list marks the current track's row, or the music video's that it plays in the
    /// place of; the same track again (every position) sets no row; nothing playing clears it.
    #[test]
    fn the_list_follows_the_current_track() {
        use featherplay_core::model::{RepeatMode, VideoType};
        let tracks = vec![track("a", &[], 1), track("v", &[], 1)];
        let (rows, model) = model(to_rows(&tracks));
        let mut list = TrackList {
            asked: vec![false; 2],
            tracks,
            rows,
            from: PlayingFrom::default(),
            playing: None,
            arriving: None,
            entries: Vec::new(),
            editable: false,
        };
        let state = |current: Option<Track>| PlayerState {
            current,
            upcoming: Vec::new().into(),
            playing: true,
            audible: true,
            loading: false,
            position: Duration::ZERO,
            shuffle: false,
            repeat: RepeatMode::Off,
            radio: false,
            volume: 1.0,
            queue: Default::default(),
            speed: 1.0,
            crossfade: None,
        };
        let marked = || -> Vec<bool> { model.iter().map(|r| r.playing).collect() };
        list.follow(Some(&state(Some(track("a", &[], 1)))));
        assert_eq!(marked(), [true, false]);

        // The song version of video `v`, as the player plays it.
        let mut video = track("v", &[], 1);
        video.video_type = VideoType::MusicVideo;
        let song = Track {
            counterpart: Some(Box::new(video)),
            ..track("s", &[], 1)
        };
        list.follow(Some(&state(Some(song.clone()))));
        assert_eq!(marked(), [false, true]);
        // Again, as the position moves on: no row is set.
        let mut row = model.row_data(1).unwrap();
        row.title = "kept".into();
        list.rows.set_row_data(1, row);
        let mut later = state(Some(song));
        later.position = Duration::from_secs(1);
        list.follow(Some(&later));
        assert_eq!(model.row_data(1).unwrap().title, "kept");

        list.follow(Some(&state(None)));
        assert_eq!(marked(), [false, false]);
        list.follow(None);
        assert_eq!(marked(), [false, false]);
    }

    #[test]
    fn shuffle_keeps_every_track() {
        let tracks: Vec<Track> = (0..50).map(|i| track(&i.to_string(), &[], 1)).collect();
        let out = shuffled(&tracks, 42);
        assert_eq!(out.len(), 50);
        let mut ids: Vec<u32> = out.iter().map(|t| t.id.0.parse().unwrap()).collect();
        assert_ne!(ids, (0..50).collect::<Vec<_>>(), "the order changed");
        ids.sort_unstable();
        assert_eq!(ids, (0..50).collect::<Vec<_>>(), "each track once");
        assert!(shuffled(&[], 1).is_empty());
    }

    #[test]
    fn permute_puts_each_item_where_the_order_says() {
        // One cycle, two cycles with a fixed point, the identity, and a rows' move to the end.
        for order in [
            vec![1, 2, 3, 0],
            vec![2, 1, 0, 4, 3],
            vec![0, 1, 2],
            vec![1, 2, 4, 5, 0, 3],
        ] {
            let mut list: Vec<String> = (0..order.len()).map(|i| i.to_string()).collect();
            permute(&mut list, &order);
            let want: Vec<String> = order.iter().map(|i| i.to_string()).collect();
            assert_eq!(list, want, "{order:?}");
        }
    }
}
