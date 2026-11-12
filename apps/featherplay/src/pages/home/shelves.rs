//! Home's shelves: the shape each one shows in, their order (`arrange`), the Shortcuts row
//! (`with_liked`), and the rows `HomeData` shows them as.

use featherplay_core::model::{PlaylistId, Track, VideoType};
use featherplay_core::tr;
use featherplay_ytm::{HomeItem, HomeShelf, PlaylistSummary};
use slint::ModelRc;

use crate::app::Ctx;
use crate::pages::content::{CARD_COVER, ROW_COVER};
use crate::song_menu;
use crate::ui::{HomeShelfRow, PageKind, ShelfShape};
use crate::ui_bridge::{home_badge, home_card, home_thumbnail, model, to_rows};

/// A community playlist's avatar badge, in logical pixels.
const BADGE: u32 = 28;

/// A shelf for `HomeData`, its covers on their way at the size its shape shows them. Its
/// cards' covers belong to `page` (`Covers::cards`).
pub(crate) fn shelf_row(
    ctx: &Ctx,
    s: &HomeShelf,
    shape: ShelfShape,
    page: PageKind,
) -> HomeShelfRow {
    let (cards, items) = model(s.items.iter().map(home_card).collect());
    // Community playlists' creators, over their collages: an empty image where there's none.
    let urls: Vec<Option<String>> = s.items.iter().map(home_badge).collect();
    let badges = if urls.iter().any(Option::is_some) {
        let (images, badges) = model(vec![slint::Image::default(); urls.len()]);
        ctx.covers
            .fill(&images, urls, BADGE, |slot, img| *slot = img);
        badges
    } else {
        ModelRc::default()
    };
    let mut songs = ModelRc::default();
    // For the song menu, and so a song card resolved ahead (`RowHover`) is the song Play
    // starts (`Player::play_id`).
    let tracks: Vec<Track> = s.items.iter().filter_map(as_track).cloned().collect();
    song_menu::remember(&tracks);
    if shape == ShelfShape::Songs {
        // Quick picks: rows, not cards.
        let urls = tracks.iter().map(|t| t.thumbnail.clone()).collect();
        let (rows, model_rows) = model(to_rows(&tracks));
        ctx.covers
            .fill(&rows, urls, ROW_COVER, |row, img| row.cover = img);
        songs = model_rows;
    } else if shape == ShelfShape::Videos {
        // 16:9 art, as the tile shows it (no square's middle band).
        let urls = s.items.iter().map(home_thumbnail).collect();
        ctx.covers.cards_wide(page, &cards, urls, cover_px(shape));
    } else {
        let urls = s.items.iter().map(home_thumbnail).collect();
        ctx.covers.cards(page, &cards, urls, cover_px(shape));
    }
    HomeShelfRow {
        title: s.title.as_str().into(),
        lead: lead(s.strapline.as_deref()).into(),
        items,
        songs,
        badges,
        shape,
    }
}

/// The size, in logical pixels, a shape's covers are fetched at: about a tile's width
/// (`Shapes.tile-max` in home.slint, which tiles don't grow past). Covers are square, except a
/// video tile's: 16:9, this wide.
fn cover_px(shape: ShelfShape) -> u32 {
    match shape {
        ShelfShape::Songs => ROW_COVER,
        ShelfShape::Mixes => 64,
        ShelfShape::Videos => 320,
        ShelfShape::Playlists => 160,
        ShelfShape::Artists => 140,
        _ => CARD_COVER,
    }
}

/// Rows per column of a song grid (`Shapes.grid-rows`): fewer songs than that stay cards.
const SONG_GRID_ROWS: usize = 4;

fn as_track(item: &HomeItem) -> Option<&Track> {
    match item {
        HomeItem::Track(t) => Some(t),
        _ => None,
    }
}

/// The shelves to show, in order, as (index in `shelves`, shape): empty shelves are dropped,
/// and the first shelf of your mixes moves up under the hero (they're what gets replayed most).
pub fn arrange(shelves: &[HomeShelf]) -> Vec<(usize, ShelfShape)> {
    let mut out: Vec<(usize, ShelfShape)> = shelves
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.items.is_empty())
        .map(|(i, s)| {
            let shape = if is_shortcuts(s) {
                ShelfShape::Mixes
            } else {
                shape(&s.items)
            };
            (i, shape)
        })
        .collect();
    if let Some(at) = out.iter().position(|(_, s)| *s == ShelfShape::Mixes) {
        let mixes = out.remove(at);
        out.insert(0, mixes);
    }
    out
}

/// How a shelf shows, from what it holds. Anything else (a mix of kinds, a kind added later)
/// is standard cards.
pub(super) fn shape(items: &[HomeItem]) -> ShelfShape {
    let all = |f: fn(&HomeItem) -> bool| !items.is_empty() && items.iter().all(f);
    if all(|i| matches!(i, HomeItem::Track(_))) {
        let videos = items
            .iter()
            .filter_map(as_track)
            .filter(|t| t.video_type != VideoType::Song)
            .count();
        if videos * 2 > items.len() {
            ShelfShape::Videos
        } else if items.len() >= SONG_GRID_ROWS {
            ShelfShape::Songs
        } else {
            ShelfShape::Cards
        }
    } else if all(|i| matches!(i, HomeItem::Playlist(p) if is_mix(&p.id.0))) {
        ShelfShape::Mixes
    } else if all(|i| matches!(i, HomeItem::Playlist(_))) {
        ShelfShape::Playlists
    } else if all(|i| matches!(i, HomeItem::Artist(_))) {
        ShelfShape::Artists
    } else {
        ShelfShape::Cards
    }
}

/// Liked songs ("LM"), as Home pins it.
fn liked_songs() -> HomeItem {
    HomeItem::Playlist(PlaylistSummary {
        id: PlaylistId(LIKED.into()),
        title: tr!("Liked songs").into(),
        author: None,
        count: None,
        views: None,
        avatar: None,
        thumbnail: None,
        owned: false,
    })
}

/// The id of Liked songs, and of the feed's own "Liked Music" tile.
const LIKED: &str = "LM";

/// Tiles in the Shortcuts row, Liked songs included.
const SHORTCUTS: usize = 4;
/// The Shortcuts row's title (`with_liked`).
fn shortcuts_title() -> &'static str {
    tr!("Shortcuts")
}

/// The row `with_liked` makes without mixes: Liked songs, then playlists and albums, shown
/// as your mixes are, as wide shortcut tiles.
pub(super) fn is_shortcuts(s: &HomeShelf) -> bool {
    s.title == shortcuts_title()
        && matches!(s.items.first(), Some(HomeItem::Playlist(p)) if p.id.0 == LIKED)
}

/// The feed as Home shows it: Liked songs pinned as the first of your mixes' shortcuts, so
/// it's always one click away. The feed's own Liked Music tile goes, so it shows once.
/// Without a shelf of mixes, Liked songs leads a "Shortcuts" row of its own, followed by
/// `picks` (the playlists and albums opened or played lately, `sidebar::shortcuts`), four
/// tiles at most; with no picks there's no row, never a lone tile. Nothing before the feed
/// arrives.
pub fn with_liked(shelves: &[HomeShelf], picks: &[HomeItem]) -> Vec<HomeShelf> {
    if shelves.is_empty() {
        return Vec::new();
    }
    let mut out = shelves.to_vec();
    for s in &mut out {
        s.items
            .retain(|i| !matches!(i, HomeItem::Playlist(p) if p.id.0 == LIKED));
    }
    match out
        .iter()
        .position(|s| !s.items.is_empty() && shape(&s.items) == ShelfShape::Mixes)
    {
        Some(at) => out[at].items.insert(0, liked_songs()),
        None if !picks.is_empty() => {
            let mut items = vec![liked_songs()];
            items.extend(picks.iter().take(SHORTCUTS - 1).cloned());
            out.insert(
                0,
                HomeShelf {
                    title: shortcuts_title().into(),
                    strapline: None,
                    items,
                },
            );
        }
        None => {}
    }
    out
}

/// Your own mixes: the ones made for the account (My Supermix, Replay, Discover, Archive:
/// "RDTMAK…") and Liked Music. Editorial playlists ("RDCLAK…") are playlists.
fn is_mix(id: &str) -> bool {
    id == "LM" || id.starts_with("RDTMAK")
}

/// The words before a shelf's title, from its strapline, in sentence case: "Similar to"
/// Ari Abdul. Personal (scrubbed) and label-only straplines show nothing.
fn lead(strapline: Option<&str>) -> &'static str {
    match strapline {
        Some("SIMILAR TO") => tr!("Similar to"),
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_items::{album, artist, playlist, shelf, song, video};
    use super::*;

    fn shapes(shelves: &[HomeShelf]) -> Vec<ShelfShape> {
        arrange(shelves).into_iter().map(|(_, s)| s).collect()
    }

    #[test]
    fn each_shelf_takes_the_shape_of_what_it_holds() {
        let songs = || (0..4).map(|i| song(&i.to_string())).collect::<Vec<_>>();
        let shelves = [
            shelf("Quick picks", songs()),
            shelf("Listen together", vec![video("v"), song("s"), video("w")]),
            shelf("Ari Abdul", vec![playlist("PLa"), playlist("PLb")]),
            shelf("Isabel LaRosa", vec![artist(), artist()]),
            shelf("Albums for you", vec![album(), album()]),
        ];
        assert_eq!(
            shapes(&shelves),
            [
                ShelfShape::Songs,
                ShelfShape::Videos,
                ShelfShape::Playlists,
                ShelfShape::Artists,
                ShelfShape::Cards,
            ]
        );
    }

    #[test]
    fn mixed_or_short_shelves_are_standard_cards() {
        let shelves = [
            // An artist with playlists ("Similar to"): standard cards, round only the artist.
            shelf("Similar", vec![artist(), playlist("PLa"), playlist("PLb")]),
            // Too few songs to fill a column of the grid.
            shelf("Two songs", vec![song("a"), song("b")]),
            shelf("Mixed", vec![song("a"), album()]),
            // Your mixes with an editorial playlist among them.
            shelf(
                "Not all mixes",
                vec![playlist("LM"), playlist("RDCLAK5uy_x")],
            ),
        ];
        assert_eq!(
            shapes(&shelves),
            [
                ShelfShape::Cards,
                ShelfShape::Cards,
                ShelfShape::Cards,
                ShelfShape::Playlists,
            ]
        );
    }

    #[test]
    fn your_mixes_come_first_and_empty_shelves_are_dropped() {
        let shelves = [
            shelf(
                "Quick picks",
                (0..4).map(|i| song(&i.to_string())).collect(),
            ),
            shelf("Empty", Vec::new()),
            shelf(
                "Fresh finds, old favorites",
                vec![
                    playlist("RDTMAK5uy_a"),
                    playlist("LM"),
                    playlist("RDTMAK5uy_b"),
                ],
            ),
            shelf("Albums for you", vec![album()]),
            shelf("More mixes", vec![playlist("RDTMAK5uy_c")]),
        ];
        assert_eq!(
            arrange(&shelves),
            [
                (2, ShelfShape::Mixes),
                (0, ShelfShape::Songs),
                (3, ShelfShape::Cards),
                (4, ShelfShape::Mixes),
            ]
        );
    }

    #[test]
    fn similar_to_leads_the_title_in_sentence_case() {
        assert_eq!(lead(Some("SIMILAR TO")), "Similar to");
        // Personal (scrubbed) and label straplines don't show.
        assert_eq!(lead(Some("STATION")), "");
        assert_eq!(lead(Some("REDACTED")), "");
        assert_eq!(lead(None), "");
    }

    fn ids(shelf: &HomeShelf) -> Vec<&str> {
        shelf
            .items
            .iter()
            .map(|i| match i {
                HomeItem::Playlist(p) => p.id.0.as_str(),
                _ => "?",
            })
            .collect()
    }

    #[test]
    fn liked_songs_is_the_first_shortcut_and_shows_once() {
        let shelves = [
            shelf(
                "Quick picks",
                (0..4).map(|i| song(&i.to_string())).collect(),
            ),
            shelf(
                "Fresh finds, old favorites",
                vec![
                    playlist("RDTMAK5uy_a"),
                    playlist("LM"),
                    playlist("RDTMAK5uy_b"),
                ],
            ),
        ];
        // Recent picks don't join a feed that has your mixes.
        let pinned = with_liked(&shelves, &[playlist("PLrecent")]);
        let (first, shape) = arrange(&pinned)[0];
        assert_eq!(shape, ShelfShape::Mixes);
        assert_eq!(ids(&pinned[first]), ["LM", "RDTMAK5uy_a", "RDTMAK5uy_b"]);
        let HomeItem::Playlist(liked) = &pinned[first].items[0] else {
            unreachable!()
        };
        assert_eq!(liked.title, "Liked songs");
    }

    #[test]
    fn without_mixes_the_shortcuts_are_liked_songs_and_recent_picks() {
        let shelves = [shelf("Albums for you", vec![album(), album()])];
        let recent = [playlist("PLa"), playlist("PLb"), playlist("PLc")];
        let pinned = with_liked(&shelves, &recent);
        let (first, shape) = arrange(&pinned)[0];
        assert_eq!(shape, ShelfShape::Mixes);
        assert_eq!(pinned[first].title, "Shortcuts");
        assert_eq!(ids(&pinned[first]), ["LM", "PLa", "PLb", "PLc"]);
        // Nothing to pin to before the feed arrives.
        assert!(with_liked(&[], &recent).is_empty());
    }

    #[test]
    fn the_shortcuts_row_shows_four_at_most() {
        let shelves = [shelf("Albums for you", vec![album(), album()])];
        let recent: Vec<HomeItem> = (0..6).map(|i| playlist(&format!("PL{i}"))).collect();
        let pinned = with_liked(&shelves, &recent);
        assert_eq!(pinned[0].items.len(), 4);
    }

    #[test]
    fn the_shortcuts_row_is_never_a_single_tile() {
        let shelves = [shelf("Albums for you", vec![album(), album()])];
        let pinned = with_liked(&shelves, &[]);
        assert_eq!(pinned.len(), 1, "a row with Liked songs alone was added");
        assert_ne!(arrange(&pinned)[0].1, ShelfShape::Mixes);
    }

    #[test]
    fn an_album_among_the_shortcuts_keeps_the_row_a_shortcut_row() {
        let shelves = [shelf(
            "Quick picks",
            (0..4).map(|i| song(&i.to_string())).collect(),
        )];
        let pinned = with_liked(&shelves, &[album()]);
        let (first, shape) = arrange(&pinned)[0];
        assert_eq!(shape, ShelfShape::Mixes);
        assert_eq!(pinned[first].items.len(), 2);
    }
}
