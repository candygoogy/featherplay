//! Discography page (ui/pages/discography.slint): an artist's albums, or singles and EPs, all
//! of them, from the artist page's "Show all". One call per visit (`Source::artist_releases`,
//! which follows the grid's pages itself). A chip or a sort order is a page of its own that
//! takes this one's place in the history (`router::Discography`), so Back returns to the
//! artist page. The cards' covers come as Library's do.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use featherplay_core::tr;
use featherplay_ytm::{BrowseRef, Release, ReleaseChoice, Releases};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::Ctx;
use crate::controller::PlayerState;
use crate::pages::content::{self, CARD_COVER};
use crate::router::{Discography, Page};
use crate::ui::{CardItem, DiscographyData, PageKind};
use crate::ui_bridge::{album_card, model, shown};

/// The round photo of the link back, in logical pixels.
const BACK_PHOTO: u32 = 24;

thread_local! {
    /// The sort orders the page on screen offers, for `pick_sort`.
    static SORTS: RefCell<Vec<ReleaseChoice>> = const { RefCell::new(Vec::new()) };
}

/// Once at startup: connect `DiscographyData`'s callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<DiscographyData>();
    let c = ctx.clone();
    data.on_pick_chip(move |i| pick_chip(&c, i.max(0) as usize));
    let c = ctx.clone();
    data.on_pick_sort(move |i| pick_sort(&c, i.max(0) as usize));
    let c = ctx.clone();
    data.on_play_card(move |it| content::play_card(&c, &it));
    let c = ctx.clone();
    data.on_retry(move || {
        // While the stored session can't connect, this load waits for a reconnect.
        c.reconnect();
        c.show_current();
    });
}

/// The discography on screen.
fn current(ctx: &Ctx) -> Option<Discography> {
    match ctx.current_page() {
        Page::Discography(d) => Some(*d),
        _ => None,
    }
}

/// Chip `i` of the ones shown: the kinds the artist page linked, in order.
fn pick_chip(ctx: &Rc<Ctx>, i: usize) {
    let Some(mut d) = current(ctx) else { return };
    let Some((tab, at)) = linked(&d).nth(i) else {
        return;
    };
    if tab == d.tab {
        return;
    }
    d.tab = tab;
    d.at = at;
    ctx.navigate(Page::Discography(Box::new(d)));
}

/// Sort order `i` of the ones the page offered.
fn pick_sort(ctx: &Rc<Ctx>, i: usize) {
    let Some(mut d) = current(ctx) else { return };
    let Some(target) = SORTS.with_borrow(|s| s.get(i).map(|c| c.target.clone())) else {
        return;
    };
    d.at = target;
    ctx.navigate(Page::Discography(Box::new(d)));
}

/// The kinds the artist page linked, as (tab, page).
fn linked(d: &Discography) -> impl Iterator<Item = (usize, BrowseRef)> + '_ {
    d.kinds
        .iter()
        .enumerate()
        .filter_map(|(tab, at)| Some((tab, at.clone()?)))
}

/// A kind's name, its chip and the page's title.
fn kind_title(tab: usize) -> &'static str {
    if tab == 0 {
        tr!("Albums")
    } else {
        tr!("Singles & EPs")
    }
}

/// The page is being shown.
pub fn open(ctx: &Rc<Ctx>, d: &Discography) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<DiscographyData>();
    data.set_loading(true);
    data.set_error("".into());
    data.set_artist_id(d.artist.id.0.as_str().into());
    data.set_artist_name(shown(&d.artist.name));
    data.set_artist_photo(Default::default());
    data.set_title(kind_title(d.tab).into());
    let chips: Vec<SharedString> = linked(d).map(|(tab, _)| kind_title(tab).into()).collect();
    data.set_chips(ModelRc::new(VecModel::from(chips)));
    data.set_tab(linked(d).position(|(tab, _)| tab == d.tab).unwrap_or(0) as i32);
    // The orders stay while the page loads in another: the pill doesn't come and go.
    data.set_cards(Default::default());
    if let Some(url) = &d.artist.photo {
        let weak = ui.as_weak();
        let wanted = data.get_artist_id();
        ctx.covers.single(
            PageKind::Discography,
            "artist photo",
            url,
            BACK_PHOTO,
            move |img| {
                if let Some(ui) = weak.upgrade() {
                    let data = ui.global::<DiscographyData>();
                    if data.get_artist_id() == wanted {
                        data.set_artist_photo(img);
                    }
                }
            },
        );
    }
    let at = d.at.clone();
    ctx.load_for_page(
        move |source| async move {
            let started = Instant::now();
            (source.artist_releases(&at).await, started.elapsed())
        },
        move |ctx, ui, (result, took)| {
            let data = ui.global::<DiscographyData>();
            data.set_loading(false);
            match result {
                Ok(r) => {
                    log!(
                        "discography: {} releases in {} ms",
                        r.items.len(),
                        took.as_millis()
                    );
                    show(ctx, &data, r);
                }
                Err(e) => {
                    data.set_sorts(Default::default());
                    data.set_sort(-1);
                    data.set_error(content::error_text("discography", &e).into());
                }
            }
        },
    );
}

/// The releases have arrived: the cards, their covers, and the sort orders.
fn show(ctx: &Rc<Ctx>, data: &DiscographyData, r: Releases) {
    let sort = r.sorts.iter().position(|s| s.selected);
    let names = sort_names(&r.sorts);
    data.set_sorts(ModelRc::new(VecModel::from(names)));
    data.set_sort(sort.map_or(-1, |i| i as i32));
    SORTS.with_borrow_mut(|s| *s = r.sorts);
    // For a pin from a card (`item_menu`): its title and cover in Shortcuts.
    crate::sidebar::seen(
        r.items
            .iter()
            .map(|i| crate::sidebar::album_entry(&i.album)),
    );
    let urls = r.items.iter().map(|i| i.album.thumbnail.clone()).collect();
    let (rows, cards) = model(r.items.iter().map(release_card).collect::<Vec<CardItem>>());
    data.set_cards(cards);
    ctx.covers
        .cards(PageKind::Discography, &rows, urls, CARD_COVER);
}

/// A release's card: its kind and year under the title ("Album · 2013").
fn release_card(r: &Release) -> CardItem {
    let mut card = album_card(&r.album);
    let kind = r.kind.as_deref().map(kind_word);
    card.subtitle = shown(
        &[kind, r.album.year.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" \u{b7} "),
    );
    card
}

/// The card's kind in the app's language when it's one of the three the service names in
/// English, else as the service says it.
fn kind_word(kind: &str) -> &str {
    match kind {
        "Album" => tr!("Album"),
        "Single" => tr!("Single"),
        "EP" => tr!("EP"),
        other => other,
    }
}

/// The sort orders' names in the app's language: the service offers four, in this order
/// (checked 2026-11-26), named in the account's language; any other set keeps its own names.
fn sort_names(sorts: &[ReleaseChoice]) -> Vec<SharedString> {
    if sorts.len() == 4 {
        return [
            tr!("Default"),
            tr!("Newest"),
            tr!("Most popular"),
            tr!("A to Z"),
        ]
        .into_iter()
        .map(SharedString::from)
        .collect();
    }
    sorts.iter().map(|s| shown(&s.title)).collect()
}

/// The controller's state changed (track, play/pause, queue).
pub fn player_changed(_ctx: &Rc<Ctx>, _state: &PlayerState) {}

#[cfg(test)]
mod tests {
    use featherplay_core::model::AlbumId;
    use featherplay_ytm::AlbumSummary;

    use super::*;

    fn choice(title: &str) -> ReleaseChoice {
        ReleaseChoice {
            title: title.into(),
            selected: false,
            target: BrowseRef {
                id: "MPAD".into(),
                params: None,
            },
        }
    }

    #[test]
    fn a_release_card_says_its_kind_then_its_year() {
        let album = AlbumSummary {
            id: AlbumId("MPREb_1".into()),
            title: "Discovery".into(),
            artists: Vec::new(),
            year: Some("2001".into()),
            thumbnail: None,
            single: false,
        };
        let card = |kind: Option<&str>, year: Option<&str>| {
            let mut album = album.clone();
            album.year = year.map(str::to_string);
            release_card(&Release {
                album,
                kind: kind.map(str::to_string),
            })
            .subtitle
        };
        assert_eq!(card(Some("Album"), Some("2001")), "Album \u{b7} 2001");
        assert_eq!(card(Some("Albumi"), Some("2001")), "Albumi \u{b7} 2001");
        assert_eq!(card(None, Some("2001")), "2001");
        assert_eq!(card(Some("EP"), None), "EP");
    }

    #[test]
    fn four_orders_take_the_apps_names_and_others_keep_theirs() {
        let four: Vec<ReleaseChoice> = ["Standard", "Neueste", "Beliebt", "A-Z"]
            .into_iter()
            .map(choice)
            .collect();
        assert_eq!(sort_names(&four)[1], "Newest");
        assert_eq!(sort_names(&[choice("Neueste")])[0], "Neueste");
    }
}
