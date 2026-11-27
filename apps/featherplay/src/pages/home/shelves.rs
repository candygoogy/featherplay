//! Home's shelves: the shape each one shows in, their order (`arrange`), the feed without its
//! Liked Music tile (`without_liked`), and the rows `HomeData` shows them as.

use std::borrow::{Borrow, Cow};

use featherplay_core::model::{Track, VideoType};
use featherplay_core::tr;
use featherplay_ytm::{HomeItem, HomeShelf};
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
    song_menu::remember(s.items.iter().filter_map(as_track));
    if shape == ShelfShape::Songs {
        // Quick picks: rows, not cards.
        let tracks: Vec<Track> = s.items.iter().filter_map(as_track).cloned().collect();
        let urls = tracks.iter().map(|t| t.thumbnail.clone()).collect();
        let (rows, model_rows) = model(to_rows(&tracks));
        ctx.covers
            .fill(&rows, urls, ROW_COVER, |row, img| row.cover = img);
        songs = model_rows;
    } else if shape == ShelfShape::Videos {
        // 16:9 art, as the tile shows it (no square's middle band).
        let urls = s.items.iter().map(home_thumbnail).collect();
        ctx.covers
            .cards_wide(page, &cards, urls, cover_px(shape, page));
    } else {
        let urls = s.items.iter().map(home_thumbnail).collect();
        ctx.covers.cards(page, &cards, urls, cover_px(shape, page));
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

/// The size, in logical pixels, a shape's covers are fetched at on `page`: a tile's width,
/// which tiles don't grow past (home.slint: Home's `Rails.tile-w`, Explore's
/// `Shapes.tile-max`). Covers are square, except a video tile's: 16:9, this wide.
fn cover_px(shape: ShelfShape, page: PageKind) -> u32 {
    match shape {
        ShelfShape::Songs => ROW_COVER,
        ShelfShape::Videos if page == PageKind::Home => 300,
        ShelfShape::Videos => 320,
        _ if page == PageKind::Home => CARD_COVER,
        ShelfShape::Mixes => 64,
        ShelfShape::Playlists => 160,
        ShelfShape::Artists => 140,
        _ => CARD_COVER,
    }
}

/// The fewest songs that make a song grid: fewer stay cards.
const SONG_GRID_ROWS: usize = 4;

fn as_track(item: &HomeItem) -> Option<&Track> {
    match item {
        HomeItem::Track(t) => Some(t),
        _ => None,
    }
}

/// The shelves to show, in order, as (index in `shelves`, shape): empty shelves are dropped,
/// and the first shelf of your mixes moves up under the band (they're what gets replayed most).
pub fn arrange<S: Borrow<HomeShelf>>(shelves: &[S]) -> Vec<(usize, ShelfShape)> {
    let mut out: Vec<(usize, ShelfShape)> = shelves
        .iter()
        .map(Borrow::borrow)
        .enumerate()
        .filter(|(_, s)| !s.items.is_empty())
        .map(|(i, s)| (i, shape(&s.items)))
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

/// The id of Liked songs, and of the feed's own "Liked Music" tile.
const LIKED: &str = "LM";

/// The feed as Home shows it: without the feed's own Liked Music tile, as Liked songs leads
/// the Shortcuts row above the shelves. Only a shelf that holds the tile is copied.
pub fn without_liked(shelves: &[HomeShelf]) -> Vec<Cow<'_, HomeShelf>> {
    let liked = |i: &HomeItem| matches!(i, HomeItem::Playlist(p) if p.id.0 == LIKED);
    shelves
        .iter()
        .map(|s| {
            if !s.items.iter().any(liked) {
                return Cow::Borrowed(s);
            }
            let mut s = s.clone();
            s.items.retain(|i| !liked(i));
            Cow::Owned(s)
        })
        .collect()
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

    #[test]
    fn the_feeds_liked_music_tile_goes_and_your_mixes_stay_a_shelf() {
        let shelves = [shelf(
            "Fresh finds, old favorites",
            vec![
                playlist("RDTMAK5uy_a"),
                playlist("LM"),
                playlist("RDTMAK5uy_b"),
            ],
        )];
        let shown = without_liked(&shelves);
        let ids: Vec<&str> = shown[0]
            .items
            .iter()
            .map(|i| match i {
                HomeItem::Playlist(p) => p.id.0.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(ids, ["RDTMAK5uy_a", "RDTMAK5uy_b"]);
        assert_eq!(arrange(&shown), [(0, ShelfShape::Mixes)]);
    }
}
