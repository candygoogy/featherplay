//! What the content pages share: a loaded track list with its rows, the playing-row
//! mark and lazy covers; meta lines; error texts; playing a whole album, playlist or artist.
//! The formatting helpers are pure and tested below.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use featherplay_core::model::{AlbumId, ArtistRef, PlaylistId, Track, TrackId};
use featherplay_core::{tr, trn};
use featherplay_ytm::ApiError;
use slint::{Image, Model, ModelRc, VecModel};

use crate::app::Ctx;
use crate::controller::{Action, PlayerState, PlayingFrom, QueueAction};
use crate::covers::Covers;
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
    playing: Option<TrackId>,
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
        };
        list.mark_playing(current_id(ctx.player().as_ref()).as_ref());
        (list, model)
    }

    /// Shows `tracks` in place of the current ones, in the same model: only the rows that
    /// changed are set, so a list on screen keeps its scroll position and doesn't flash. A
    /// song still in the list keeps the cover already loaded for it, wherever it moved.
    pub fn replace(&mut self, tracks: Vec<Track>, playing: Option<&TrackId>) {
        song_menu::remember(&tracks);
        let mut covers: HashMap<String, Image> = HashMap::new();
        for (i, asked) in self.asked.iter().enumerate() {
            if let Some(row) = self.rows.row_data(i)
                && *asked
                && row.cover.size().width > 0
            {
                covers.insert(row.id.to_string(), row.cover);
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
        self.tracks = tracks;
        self.asked = asked;
        self.playing = None;
        self.mark_playing(playing);
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

    /// Marks the rows of `id` as playing and clears the old mark; cheap when unchanged.
    pub fn mark_playing(&mut self, id: Option<&TrackId>) {
        if self.playing.as_ref() == id {
            return;
        }
        for (i, t) in self.tracks.iter().enumerate() {
            let now = Some(&t.id) == id;
            let was = self.playing.as_ref() == Some(&t.id);
            if now != was
                && let Some(mut row) = self.rows.row_data(i)
            {
                row.playing = now;
                self.rows.set_row_data(i, row);
            }
        }
        self.playing = id.cloned();
    }

    /// Plays the list from `start`.
    pub fn play(&self, ctx: &Ctx, start: usize, radio: bool) {
        if start < self.tracks.len() {
            self.send_play(ctx, self.tracks.clone(), start, radio);
        }
    }

    /// Plays track `i` alone, with its radio after it (search results, Home cards).
    pub fn play_one(&self, ctx: &Ctx, i: usize) {
        if let Some(t) = self.tracks.get(i) {
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
            self.send_play(ctx, shuffled(&self.tracks, seed()), 0, false);
        }
    }

    /// Plays `tracks`, labelled with `from` when the list has one.
    fn send_play(&self, ctx: &Ctx, tracks: Vec<Track>, start: usize, radio: bool) {
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

/// The current track's id.
pub fn current_id(state: Option<&PlayerState>) -> Option<TrackId> {
    state.and_then(|s| s.current.as_ref()).map(|t| t.id.clone())
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

/// Plays a whole album, from the start (one call).
pub fn play_album(ctx: &Rc<Ctx>, id: AlbumId) {
    ctx.load(
        move |source| async move { source.album(&id).await },
        |ctx, _, result| match result {
            Ok(album) => {
                crate::sidebar::opened(ctx, crate::sidebar::album_entry(&album.summary));
                play_tracks(ctx, album.tracks)
            }
            Err(e) => log!("play album: {e}"),
        },
    );
}

/// Plays a whole playlist, from the start (one call).
pub fn play_playlist(ctx: &Rc<Ctx>, id: PlaylistId) {
    ctx.load(
        move |source| async move { source.playlist(&id).await },
        |ctx, _, result| match result {
            Ok(playlist) => {
                crate::sidebar::opened(ctx, crate::sidebar::playlist_entry(&playlist.summary));
                play_tracks(ctx, playlist.tracks)
            }
            Err(e) => log!("play playlist: {e}"),
        },
    );
}

/// Plays an artist's top songs (one call).
pub fn play_artist(ctx: &Rc<Ctx>, id: featherplay_core::model::ArtistId) {
    ctx.load(
        move |source| async move { source.artist(&id).await },
        |ctx, _, result| match result {
            Ok(artist) => play_tracks(ctx, artist.top_songs),
            Err(e) => log!("play artist: {e}"),
        },
    );
}

fn play_tracks(ctx: &Ctx, tracks: Vec<Track>) {
    if !tracks.is_empty() {
        song_menu::remember(&tracks);
        ctx.send(Action::Play {
            tracks,
            start: 0,
            radio: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use featherplay_core::model::{ArtistId, LikeStatus, VideoType};

    use super::*;

    fn track(id: &str, artists: &[&str], secs: u64) -> Track {
        Track {
            id: TrackId(id.into()),
            title: id.into(),
            artists: artists.iter().map(|a| artist(a)).collect(),
            album: None,
            duration: (secs > 0).then(|| Duration::from_secs(secs)),
            video_type: VideoType::Song,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            song_version: None,
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
        };
        let cover = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(2, 2));
        for i in 0..2 {
            let mut row = list.rows.row_data(i).unwrap();
            row.cover = cover.clone();
            list.rows.set_row_data(i, row);
        }

        // A new song at the top: the others move down a row.
        list.replace(
            vec![track("c", &[], 1), track("a", &[], 1), track("b", &[], 1)],
            None,
        );
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
        list.replace(vec![track("a", &[], 1)], None);
        assert_eq!(model.row_count(), 1);
        assert_eq!(size(0).width, 2);
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
}
