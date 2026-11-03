//! The sidebar's library list (chips Playlists, Albums, Artists) and Home's Shortcuts row:
//! what the user pinned, what they opened or played lately, and the order these make.
//!
//! Pure: the sidebar (`sidebar.rs`) and Home (`pages/home.rs`) hold the state and call these.

use serde::{Deserialize, Serialize};

/// What a list entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    Playlist,
    Album,
    Artist,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Kind::Playlist => "playlist",
            Kind::Album => "album",
            Kind::Artist => "artist",
        }
    }
}

/// A saved or recently opened playlist, album or artist, with what a row or tile shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub kind: Kind,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub thumbnail: Option<String>,
}

impl Entry {
    /// `"album:MPRE…"`: how pins and the recent list name it.
    pub fn key(&self) -> String {
        key(self.kind, &self.id)
    }
}

pub fn key(kind: Kind, id: &str) -> String {
    format!("{}:{id}", kind.tag())
}

/// Liked songs' playlist id: it has its own row and tile, so it's never a recent entry.
const LIKED: &str = "LM";

/// The most entries the recent list keeps.
pub const MAX_RECENT: usize = 30;

/// Notes that `e` was opened or played: it goes to the front of `recent`, once.
pub fn note(recent: &mut Vec<Entry>, e: Entry) {
    if e.kind == Kind::Playlist && e.id == LIKED {
        return;
    }
    recent.retain(|r| r.kind != e.kind || r.id != e.id);
    recent.insert(0, e);
    recent.truncate(MAX_RECENT);
}

/// Pins `key` to the top (above the other pins), or unpins it. True when it's pinned now.
pub fn toggle_pin(pins: &mut Vec<String>, key: &str) -> bool {
    if let Some(at) = pins.iter().position(|p| p == key) {
        pins.remove(at);
        false
    } else {
        pins.insert(0, key.to_string());
        true
    }
}

/// The list as the sidebar shows it: the pinned entries first (the latest pin on top), then
/// the ones opened or played lately (latest first), then the rest in the order the library
/// gave them. Each entry once; pins and recents of entries no longer in the library don't
/// show.
pub fn order(items: &[Entry], pins: &[String], recent: &[Entry]) -> Vec<(Entry, bool)> {
    let keys: Vec<String> = items.iter().map(Entry::key).collect();
    let mut taken = vec![false; items.len()];
    let mut out = Vec::with_capacity(items.len());
    let mut take = |key: &str, pinned: bool, out: &mut Vec<(Entry, bool)>| {
        if let Some(i) = keys.iter().position(|k| k == key)
            && !taken[i]
        {
            taken[i] = true;
            out.push((items[i].clone(), pinned));
        }
    };
    for pin in pins {
        take(pin, true, &mut out);
    }
    for r in recent {
        take(&r.key(), false, &mut out);
    }
    for k in &keys {
        take(k, false, &mut out);
    }
    out
}

/// Home's shortcuts after Liked songs: playlists and albums opened or played lately, then
/// the library's playlists and albums, each once, at most `n`.
pub fn shortcuts(recent: &[Entry], library: &[Entry], n: usize) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::with_capacity(n);
    for e in recent.iter().chain(library) {
        if out.len() == n {
            break;
        }
        let fits = matches!(e.kind, Kind::Playlist | Kind::Album) && e.id != LIKED;
        if fits && !out.iter().any(|o| o.kind == e.kind && o.id == e.id) {
            out.push(e.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(kind: Kind, id: &str) -> Entry {
        Entry {
            kind,
            id: id.into(),
            title: id.into(),
            subtitle: String::new(),
            thumbnail: None,
        }
    }

    fn p(id: &str) -> Entry {
        e(Kind::Playlist, id)
    }

    fn ids(list: &[(Entry, bool)]) -> Vec<&str> {
        list.iter().map(|(e, _)| e.id.as_str()).collect()
    }

    #[test]
    fn without_pins_or_plays_the_list_keeps_the_librarys_order() {
        let shown = order(&[p("a"), p("b"), p("c")], &[], &[]);
        assert_eq!(ids(&shown), ["a", "b", "c"]);
        assert!(shown.iter().all(|(_, pinned)| !pinned));
    }

    #[test]
    fn played_lately_comes_first_latest_on_top() {
        let shown = order(&[p("a"), p("b"), p("c")], &[], &[p("c"), p("b")]);
        assert_eq!(ids(&shown), ["c", "b", "a"]);
    }

    #[test]
    fn pins_come_before_everything_and_are_marked() {
        let pins = vec![key(Kind::Playlist, "b")];
        let shown = order(&[p("a"), p("b"), p("c")], &pins, &[p("c")]);
        assert_eq!(ids(&shown), ["b", "c", "a"]);
        assert_eq!(
            shown.iter().map(|(_, pinned)| *pinned).collect::<Vec<_>>(),
            [true, false, false]
        );
    }

    #[test]
    fn pins_and_plays_of_entries_gone_from_the_library_dont_show() {
        let pins = vec![key(Kind::Playlist, "gone")];
        let shown = order(&[p("a")], &pins, &[p("also-gone")]);
        assert_eq!(ids(&shown), ["a"]);
    }

    #[test]
    fn a_pin_of_another_kind_doesnt_match() {
        let pins = vec![key(Kind::Album, "b")];
        let shown = order(&[p("a"), p("b")], &pins, &[]);
        assert_eq!(ids(&shown), ["a", "b"]);
    }

    #[test]
    fn the_latest_pin_goes_on_top_and_a_second_toggle_unpins() {
        let mut pins = Vec::new();
        assert!(toggle_pin(&mut pins, "playlist:a"));
        assert!(toggle_pin(&mut pins, "album:b"));
        assert_eq!(pins, ["album:b", "playlist:a"]);
        assert!(!toggle_pin(&mut pins, "playlist:a"));
        assert_eq!(pins, ["album:b"]);
    }

    #[test]
    fn noting_moves_an_entry_to_the_front_once() {
        let mut recent = vec![p("a"), p("b")];
        note(&mut recent, p("b"));
        assert_eq!(
            recent.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
    }

    #[test]
    fn the_recent_list_is_capped_and_skips_liked_songs() {
        let mut recent = Vec::new();
        for i in 0..MAX_RECENT + 5 {
            note(&mut recent, p(&i.to_string()));
        }
        note(&mut recent, p("LM"));
        assert_eq!(recent.len(), MAX_RECENT);
        assert_eq!(recent[0].id, (MAX_RECENT + 4).to_string());
    }

    #[test]
    fn shortcuts_are_recent_playlists_and_albums_then_the_library() {
        let recent = [e(Kind::Album, "x"), e(Kind::Artist, "y"), p("a")];
        let library = [p("a"), p("b"), p("c")];
        let got: Vec<String> = shortcuts(&recent, &library, 3)
            .iter()
            .map(Entry::key)
            .collect();
        assert_eq!(got, ["album:x", "playlist:a", "playlist:b"]);
    }

    #[test]
    fn shortcuts_never_hold_liked_songs() {
        let got = shortcuts(&[p("LM")], &[p("LM"), p("a")], 3);
        assert_eq!(got, [p("a")]);
    }
}
