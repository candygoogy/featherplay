//! An artist's discography page (`artist_releases.json`: Daft Punk's albums as a visitor
//! sees them, recorded 2026-11-26, scrubbed to its `contents` and `header`, without
//! tracking tokens), and the reload tokens of its chips and sort orders.

use std::path::PathBuf;

use featherplay_ytm::{BrowseRef, convert};
use proptest::prelude::*;
use serde_json::Value;

const DISCOGRAPHY: &str = "MPADUCRr1xG_2WIDs18a6cIiCxeA";

fn fixture(name: &str) -> String {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn page(params: &str) -> BrowseRef {
    BrowseRef {
        id: DISCOGRAPHY.into(),
        params: Some(params.into()),
    }
}

/// Every album of the grid, with its kind and year; the kind chips, where the one that's
/// on turns off (every release); the sort orders, the one in use marked.
#[test]
fn a_discography_lists_its_releases_chips_and_sort_orders() {
    let r = convert::artist_releases(&fixture("artist_releases")).expect("parse");
    assert_eq!(r.items.len(), 11);
    let first = &r.items[0];
    assert_eq!(
        first.album.title,
        "Random Access Memories (Drumless Edition)"
    );
    assert_eq!(first.album.id.0, "MPREb_OgeEnoHTsCm");
    assert_eq!(first.album.year.as_deref(), Some("2023"));
    assert_eq!(first.kind.as_deref(), Some("Album"));
    assert!(first.album.thumbnail.is_some() && !first.album.single);
    assert!(r.items.iter().all(|i| i.kind.as_deref() == Some("Album")));

    let chips: Vec<(&str, bool)> = r
        .chips
        .iter()
        .map(|c| (c.title.as_str(), c.selected))
        .collect();
    assert_eq!(chips, [("Albums", true), ("Singles & EPs", false)]);
    assert_eq!(r.chips[0].target, page("ggMGegQaAgECoAYB"));
    assert_eq!(r.chips[1].target, page("ggMIegYIAhoCAQKgBgE%3D"));

    let sorts: Vec<(&str, bool)> = r
        .sorts
        .iter()
        .map(|c| (c.title.as_str(), c.selected))
        .collect();
    assert_eq!(
        sorts,
        [
            ("Default", true),
            ("Recency", false),
            ("Popularity", false),
            ("Alphabetical order", false)
        ]
    );
    assert_eq!(r.sorts[1].target, page("ggMKeggIARABGgIBAqAGAQ%3D%3D"));
}

/// The page follows the account's language: chips and sort orders are told by where they
/// sit, and the kind is the card's own word.
#[test]
fn a_discography_in_another_language_reads_the_same() {
    let english = convert::artist_releases(&fixture("artist_releases")).expect("english");
    let raw = fixture("artist_releases")
        .replace("\"Albums\"", "\"Alben\"")
        .replace("\"Default\"", "\"Standard\"")
        .replace("\"text\":\"Album\"", "\"text\":\"Albumi\"");
    let other = convert::artist_releases(&raw).expect("other language");
    assert_eq!(other.items.len(), english.items.len());
    assert_eq!(other.items[0].kind.as_deref(), Some("Albumi"));
    assert_eq!(other.chips[0].title, "Alben");
    assert!(other.chips[0].selected && other.sorts[0].selected);
    assert_eq!(other.chips[0].target, english.chips[0].target);
}

/// An artist with a few albums has no "More": the albums are there, with nowhere to go on.
#[test]
fn an_albums_shelf_without_more_goes_nowhere() {
    let mut v: Value = serde_json::from_str(&fixture("artist")).expect("fixture json");
    let header = v
        .pointer_mut("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/1/musicCarouselShelfRenderer/header/musicCarouselShelfBasicHeaderRenderer")
        .and_then(Value::as_object_mut)
        .expect("albums header");
    header.remove("moreContentButton");
    header
        .get_mut("title")
        .and_then(|t| t.pointer_mut("/runs/0"))
        .and_then(Value::as_object_mut)
        .expect("title run")
        .remove("navigationEndpoint");
    let artist = convert::artist(&v.to_string()).expect("parse");
    assert_eq!(artist.albums.len(), 10);
    assert_eq!(artist.albums_all, None);
    assert!(artist.singles_all.is_some());
}

/// A grid continuation page has releases only.
#[test]
fn a_discography_continuation_has_its_releases() {
    let first: Value = serde_json::from_str(&fixture("artist_releases")).expect("json");
    let items = first
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/gridRenderer/items")
        .expect("items")
        .clone();
    let more =
        serde_json::json!({ "continuationContents": { "gridContinuation": { "items": items } } });
    let r = convert::artist_releases(&more.to_string()).expect("parse");
    assert_eq!(r.items.len(), 11);
    assert!(r.chips.is_empty() && r.sorts.is_empty());
}

fn varint(mut n: u64, out: &mut Vec<u8>) {
    while n >= 0x80 {
        out.push((n as u8) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn bytes_field(field: u64, value: &[u8], out: &mut Vec<u8>) {
    varint(field << 3 | 2, out);
    varint(value.len() as u64, out);
    out.extend_from_slice(value);
}

/// URL-safe base64 without padding, as the tokens are.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// A reload token for `id` and `params`, with `noise` (a number field) before each.
fn token(id: &str, params: &str, noise: Option<u64>) -> String {
    let mut inner = Vec::new();
    if let Some(n) = noise {
        varint(1 << 3, &mut inner);
        varint(n, &mut inner);
    }
    bytes_field(2, id.as_bytes(), &mut inner);
    bytes_field(3, params.as_bytes(), &mut inner);
    let mut outer = Vec::new();
    if let Some(n) = noise {
        varint(7 << 3, &mut outer);
        varint(n, &mut outer);
    }
    bytes_field(80_226_972, &inner, &mut outer);
    base64(&outer)
}

#[test]
fn a_recorded_reload_token_names_its_page() {
    let t = "4qmFsgI2EhxNUEFEVUNScjF4R18yV0lEczE4YTZjSWlDeGVBGhZnZ01JZWdZSUFob0NBUUtnQmdFJTNE";
    assert_eq!(
        convert::reload_target(t),
        Some(page("ggMIegYIAhoCAQKgBgE%3D"))
    );
}

proptest! {
    /// Any text is read without a panic, as a page or as nothing.
    #[test]
    fn any_token_is_read_safely(t in ".{0,120}", raw in proptest::collection::vec(any::<u8>(), 0..80)) {
        let _ = convert::reload_target(&t);
        let _ = convert::reload_target(&base64(&raw));
    }

    /// A token made of a page's id and params gives them back, past fields it doesn't know.
    #[test]
    fn a_made_token_gives_its_page_back(
        id in "[A-Za-z0-9_-]{1,40}",
        params in "[A-Za-z0-9%_=-]{0,60}",
        noise in proptest::option::of(any::<u64>()),
    ) {
        let got = convert::reload_target(&token(&id, &params, noise));
        prop_assert_eq!(got, Some(BrowseRef { id, params: Some(params) }));
    }
}
