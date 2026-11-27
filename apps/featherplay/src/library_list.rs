//! The sidebar's library list (chips Playlists, Albums, Artists) and Home's Shortcuts row:
//! what the user pinned, what they opened or played lately, what they removed from
//! Shortcuts, and the order these make.
//!
//! Pure: the sidebar (`sidebar.rs`) and Home (`pages/home.rs`) hold the state and call these.

use std::collections::HashMap;

use featherplay_core::{tr, trn};
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

/// A saved, pinned or lately opened or played playlist, album or artist, with what a row or
/// tile shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub kind: Kind,
    pub id: String,
    pub title: String,
    /// Who: an album's artists, a playlist's author (a mix's line, as the feed gives it);
    /// nothing for an artist.
    #[serde(default)]
    pub subtitle: String,
    /// A playlist's songs, when known.
    #[serde(default)]
    pub count: Option<u32>,
    /// An album that's a single.
    #[serde(default)]
    pub single: bool,
    #[serde(default)]
    pub thumbnail: Option<String>,
}

impl Entry {
    /// `"album:MPRE…"`: how pins and the recent list name it.
    pub fn key(&self) -> String {
        key(self.kind, &self.id)
    }

    /// Whether `key` names it (`key() == key`, without making the key).
    pub fn is(&self, key: &str) -> bool {
        parse_key(key) == Some((self.kind, self.id.as_str()))
    }

    /// The same entry with what `other` knows and this one doesn't (a page's answer has no
    /// author for a mix; the feed's tile has).
    pub fn filled_from(mut self, other: &Entry) -> Entry {
        if self.subtitle.is_empty() {
            self.subtitle.clone_from(&other.subtitle);
        }
        if self.thumbnail.is_none() {
            self.thumbnail.clone_from(&other.thumbnail);
        }
        self.count = self.count.or(other.count);
        self.single |= other.single;
        self
    }
}

pub fn key(kind: Kind, id: &str) -> String {
    format!("{}:{id}", kind.tag())
}

/// The kind and id `key` names (`key`'s inverse): a tag has no colon, so the first one ends it.
fn parse_key(key: &str) -> Option<(Kind, &str)> {
    let (tag, id) = key.split_once(':')?;
    let kind = [Kind::Playlist, Kind::Album, Kind::Artist]
        .into_iter()
        .find(|k| k.tag() == tag)?;
    Some((kind, id))
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

/// Pins `key` (first among the pins: at the top of the sidebar's list, and first after
/// Liked songs in Shortcuts), or unpins it. True when it's pinned now.
pub fn toggle_pin(pins: &mut Vec<String>, key: &str) -> bool {
    if let Some(at) = pins.iter().position(|p| p == key) {
        pins.remove(at);
        false
    } else {
        pins.insert(0, key.to_string());
        true
    }
}

/// Moves pin `key` one place earlier (`later`: one place later) among the pins. False when
/// there's no such pin or no place to move it to.
pub fn move_pin(pins: &mut [String], key: &str, later: bool) -> bool {
    let Some(at) = pins.iter().position(|p| p == key) else {
        return false;
    };
    let to = if later {
        at + 1
    } else {
        match at.checked_sub(1) {
            Some(to) => to,
            None => return false,
        }
    };
    if to >= pins.len() {
        return false;
    }
    pins.swap(at, to);
    true
}

/// The most keys the hidden list keeps, the latest first.
pub const MAX_HIDDEN: usize = 200;

/// Notes a play from `e` (`played`, latest first) and brings it back to Shortcuts if it was
/// removed.
pub fn played(played: &mut Vec<Entry>, hidden: &mut Vec<String>, e: Entry) {
    let key = e.key();
    hidden.retain(|h| *h != key);
    note(played, e);
}

/// Removes `key` from Shortcuts' suggestions until it's played again.
pub fn hide(hidden: &mut Vec<String>, key: &str) {
    hidden.retain(|h| h != key);
    hidden.insert(0, key.to_string());
    hidden.truncate(MAX_HIDDEN);
}

/// The list as the sidebar shows it: the pinned entries first (the latest pin on top), then
/// the ones opened or played lately (latest first), then the rest in the order the library
/// gave them. Each entry once; pins and recents of entries no longer in the library don't
/// show. An entry the library lists twice shows at its first place.
pub fn order<'a>(items: &'a [Entry], pins: &[String], recent: &[Entry]) -> Vec<(&'a Entry, bool)> {
    // Each entry's first place, by kind and id: one look each, where a search of the list for
    // each made a library of 1,000 a million comparisons.
    let mut first: HashMap<(Kind, &str), usize> = HashMap::with_capacity(items.len());
    for (i, e) in items.iter().enumerate() {
        first.entry((e.kind, e.id.as_str())).or_insert(i);
    }
    let mut taken = vec![false; items.len()];
    let mut out = Vec::with_capacity(items.len());
    let mut take = |at: Option<&usize>, pinned: bool| {
        if let Some(&i) = at
            && !std::mem::replace(&mut taken[i], true)
        {
            out.push((&items[i], pinned));
        }
    };
    for pin in pins {
        take(parse_key(pin).and_then(|k| first.get(&k)), true);
    }
    for e in recent.iter().chain(items) {
        take(first.get(&(e.kind, e.id.as_str())), false);
    }
    out
}

/// The most tiles Home's Shortcuts row shows, Liked songs included.
pub const MAX_SHORTCUTS: usize = 24;

/// What Shortcuts is made of.
#[derive(Clone, Copy)]
pub struct Sources<'a> {
    /// The pins, in order, by key.
    pub pins: &'a [String],
    /// A pin's title and cover, by its key.
    pub known: &'a dyn Fn(&str) -> Option<&'a Entry>,
    /// Played from lately, latest first.
    pub played: &'a [Entry],
    /// The library's playlists, albums and artists, list after list.
    pub library: &'a [&'a [Entry]],
    /// Removed from Shortcuts, by key.
    pub hidden: &'a [String],
    /// "Suggest from what you play": off, only the pins follow Liked songs.
    pub suggest: bool,
}

/// One tile of Shortcuts after Liked songs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub entry: Entry,
    pub pinned: bool,
}

/// Home's Shortcuts after Liked songs: the pins in their order, then (suggesting) what was
/// played from lately, then the library, each once, never Liked songs (it has its own tile
/// first) nor a suggestion that was removed; `MAX_SHORTCUTS` with Liked songs at most. A
/// pin nothing knows the title of is left out.
pub fn shortcuts(s: &Sources) -> Vec<Shortcut> {
    let max = MAX_SHORTCUTS - 1;
    let mut out: Vec<Shortcut> = Vec::with_capacity(max);
    let fresh = |out: &[Shortcut], e: &Entry| {
        !(e.kind == Kind::Playlist && e.id == LIKED)
            && !out
                .iter()
                .any(|o| o.entry.kind == e.kind && o.entry.id == e.id)
    };
    for pin in s.pins {
        if let Some(e) = (s.known)(pin)
            && out.len() < max
            && fresh(&out, e)
        {
            out.push(Shortcut {
                entry: e.clone(),
                pinned: true,
            });
        }
    }
    if s.suggest {
        for e in s.played.iter().chain(s.library.iter().copied().flatten()) {
            if out.len() == max {
                break;
            }
            if fresh(&out, e) && !s.hidden.iter().any(|h| e.is(h)) {
                out.push(Shortcut {
                    entry: e.clone(),
                    pinned: false,
                });
            }
        }
    }
    out
}

/// Your own mixes (My Supermix, Replay, Discover: "RDTMAK…").
pub fn is_mix(id: &str) -> bool {
    id.starts_with("RDTMAK")
}

/// A tile's line under its title: "Album · Daft Punk", "Single · …", "Playlist · 12
/// songs" (or its author when the count isn't known), "Artist", "Mix · …".
pub fn subtitle(e: &Entry) -> String {
    let who = e.subtitle.trim();
    let with = |kind: &str, line: String| {
        if who.is_empty() {
            kind.to_string()
        } else {
            line
        }
    };
    match e.kind {
        Kind::Artist => tr!("Artist").to_string(),
        Kind::Album if e.single => with(tr!("Single"), tr!("Single · {}", who)),
        Kind::Album => with(tr!("Album"), tr!("Album · {}", who)),
        Kind::Playlist if is_mix(&e.id) => with(tr!("Mix"), tr!("Mix · {}", who)),
        Kind::Playlist => match e.count {
            Some(n) => trn!("Playlist · {n} song" | "Playlist · {n} songs" % n),
            None => with(tr!("Playlist"), tr!("Playlist · {}", who)),
        },
    }
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
            count: None,
            single: false,
            thumbnail: None,
        }
    }

    fn p(id: &str) -> Entry {
        e(Kind::Playlist, id)
    }

    #[test]
    fn pins_then_plays_then_the_library_each_once() {
        // Playlists by id, space-separated; a pinned entry shows as `*id`.
        let playlists = |ids: &str| ids.split_whitespace().map(p).collect::<Vec<_>>();
        // (case, library, pins, played lately, want)
        let cases: [(&str, &str, &[&str], &str, &str); 6] = [
            ("the library's order", "a b c", &[], "", "a b c"),
            ("listed twice", "a b a", &["playlist:a"], "b", "*a b"),
            ("played lately first", "a b c", &[], "c b", "c b a"),
            ("pins first", "a b c", &["playlist:b"], "c", "*b c a"),
            ("gone", "a", &["playlist:gone"], "also-gone", "a"),
            ("a pin of another kind", "a b", &["album:b"], "", "a b"),
        ];
        for (case, items, pins, recent, want) in cases {
            let pins: Vec<String> = pins.iter().map(|k| k.to_string()).collect();
            let library = playlists(items);
            let shown = order(&library, &pins, &playlists(recent));
            let shown: Vec<String> = shown
                .iter()
                .map(|(e, pinned)| format!("{}{}", if *pinned { "*" } else { "" }, e.id))
                .collect();
            assert_eq!(shown.join(" "), want, "{case}");
        }
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

    /// Shortcuts' keys, a pinned one as `*key`.
    fn shown(got: &[Shortcut]) -> Vec<String> {
        got.iter()
            .map(|s| format!("{}{}", if s.pinned { "*" } else { "" }, s.entry.key()))
            .collect()
    }

    #[test]
    fn shortcuts_are_pins_then_plays_then_the_library_each_once() {
        let known = [p("a"), e(Kind::Album, "x")];
        let played = [e(Kind::Artist, "y"), p("a"), p("LM"), p("hidden")];
        let library = [p("a"), p("b"), p("hidden"), e(Kind::Album, "x")];
        let pins = [
            "album:x".to_string(),
            "playlist:a".into(),
            "playlist:nowhere".into(),
        ];
        let hidden = ["playlist:hidden".to_string(), "album:x".into()];
        let sources = Sources {
            pins: &pins,
            known: &|k| known.iter().find(|e| e.is(k)),
            played: &played,
            library: &[&library],
            hidden: &hidden,
            suggest: true,
        };
        // A pin is never hidden; Liked songs never shows (it has its own tile first).
        assert_eq!(
            shown(&shortcuts(&sources)),
            ["*album:x", "*playlist:a", "artist:y", "playlist:b"]
        );
        let pins_only = Sources {
            suggest: false,
            ..sources
        };
        assert_eq!(shown(&shortcuts(&pins_only)), ["*album:x", "*playlist:a"]);
    }

    #[test]
    fn shortcuts_hold_24_tiles_with_liked_songs() {
        let library: Vec<Entry> = (0..40).map(|i| p(&i.to_string())).collect();
        let pins: Vec<String> = (0..30)
            .map(|i| key(Kind::Playlist, &i.to_string()))
            .collect();
        for pins in [&pins[..0], &pins[..]] {
            let got = shortcuts(&Sources {
                pins,
                known: &|k| library.iter().find(|e| e.is(k)),
                played: &[],
                library: &[&library],
                hidden: &[],
                suggest: true,
            });
            assert_eq!(got.len() + 1, MAX_SHORTCUTS);
        }
    }

    #[test]
    fn a_pin_moves_one_place_and_not_past_either_end() {
        let mut pins: Vec<String> = ["a", "b", "c"].iter().map(|k| k.to_string()).collect();
        assert!(move_pin(&mut pins, "b", false));
        assert_eq!(pins, ["b", "a", "c"]);
        assert!(!move_pin(&mut pins, "b", false));
        assert!(move_pin(&mut pins, "a", true));
        assert_eq!(pins, ["b", "c", "a"]);
        assert!(!move_pin(&mut pins, "a", true));
        assert!(!move_pin(&mut pins, "nowhere", true));
    }

    #[test]
    fn a_removed_suggestion_comes_back_when_played_again() {
        let (mut played_list, mut hidden) = (Vec::new(), Vec::new());
        hide(&mut hidden, "playlist:a");
        hide(&mut hidden, "playlist:b");
        hide(&mut hidden, "playlist:a");
        assert_eq!(hidden, ["playlist:a", "playlist:b"]);
        played(&mut played_list, &mut hidden, p("a"));
        assert_eq!(hidden, ["playlist:b"]);
        assert_eq!(played_list, [p("a")]);
    }

    #[test]
    fn subtitles_say_what_a_tile_is() {
        let with = |mut e: Entry, who: &str| {
            e.subtitle = who.into();
            e
        };
        let single = Entry {
            single: true,
            ..e(Kind::Album, "s")
        };
        let counted = Entry {
            count: Some(42),
            ..p("PLc")
        };
        let cases = [
            (with(e(Kind::Album, "a"), "Daft Punk"), "Album · Daft Punk"),
            (with(single, "Scott Buckley"), "Single · Scott Buckley"),
            (with(counted, "Someone"), "Playlist · 42 songs"),
            (with(p("PLa"), "Someone"), "Playlist · Someone"),
            (p("PLb"), "Playlist"),
            (with(e(Kind::Artist, "UC"), "ignored"), "Artist"),
            (
                with(p("RDTMAK5uy_x"), "Josh Woodward"),
                "Mix · Josh Woodward",
            ),
        ];
        for (entry, want) in cases {
            assert_eq!(subtitle(&entry), want, "{}", entry.id);
        }
    }
}
