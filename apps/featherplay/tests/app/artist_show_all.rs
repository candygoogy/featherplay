//! The artist page's "Show all" (fixtures, offscreen): Albums' opens the discography with the
//! recorded releases, its chips switch the kind in place of the page in the history, Top songs'
//! opens the playlist page as "All songs" under the artist, and a shelf that links nowhere has
//! none.

use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{ArtistData, DiscographyData, PlaylistData, Theme};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::support::{label, resize};

/// The artist page, settled, in a window tall enough to show all its shelves.
fn artist() -> Scene {
    let (w, h) = (1280, 2600);
    let scene = Scene::new(w, h, true, false).expect("scene");
    resize(scene.ui.window(), w, h);
    scene.open("artist").expect("artist");
    scene.settle();
    scene
}

/// Presses the "Show all" on the title row of the page's shelf `title`.
fn show_all(scene: &Scene, title: &str) {
    let side = scene.ui.global::<Theme>().get_sidebar_w();
    let row = label::any_where(scene, |item, l| {
        l == title && label::rect(item).min_x() > side
    });
    let y = label::rect(row.first().expect("the shelf's title"))
        .center()
        .y;
    let link = label::any_where(scene, |item, l| {
        l == "Show all" && item.is_visible() && (label::rect(item).center().y - y).abs() < 16.0
    });
    link.first()
        .unwrap_or_else(|| panic!("{title} has no Show all"))
        .accessible_action(&i_slint_core::accessibility::AccessibilityAction::Default);
    scene.settle();
}

#[test]
fn albums_show_all_opens_the_discography_and_its_chip_switches_the_kind() {
    let scene = artist();
    show_all(&scene, "Albums");
    assert!(matches!(scene.ctx.current_page(), Page::Discography(_)));
    let data = scene.ui.global::<DiscographyData>();
    assert_eq!(data.get_cards().row_count(), 11, "the recorded albums");
    let first = data.get_cards().row_data(0).expect("a card");
    assert_eq!(first.title, "Random Access Memories (Drumless Edition)");
    assert_eq!(first.subtitle, "Album \u{b7} 2023");
    assert_eq!(data.get_title(), "Albums");
    assert_eq!(data.get_artist_name(), "Daft Punk");
    assert_eq!(data.get_sorts().row_count(), 4);
    assert_eq!(
        data.get_sorts()
            .row_data(data.get_sort() as usize)
            .as_deref(),
        Some("Default")
    );

    // The page's chip, not the sidebar's.
    let side = scene.ui.global::<Theme>().get_sidebar_w();
    let chip = label::shown_where(&scene, |item, l| {
        l == "Singles & EPs" && label::rect(item).min_x() > side
    });
    chip.first()
        .expect("the Singles & EPs chip")
        .accessible_action(&i_slint_core::accessibility::AccessibilityAction::Default);
    scene.settle();
    let Page::Discography(d) = scene.ctx.current_page() else {
        panic!("the chip left the discography");
    };
    assert_eq!(d.tab, 1);
    assert_eq!(data.get_tab(), 1);
    assert_eq!(data.get_title(), "Singles & EPs");
    assert!(data.get_cards().row_count() > 0);

    // The chip took the albums' place: Back is the artist page.
    scene.ctx.back();
    scene.settle();
    assert!(matches!(scene.ctx.current_page(), Page::Artist { .. }));
}

#[test]
fn top_songs_show_all_opens_all_songs_under_the_artist() {
    let scene = artist();
    show_all(&scene, "Top songs");
    assert!(matches!(
        scene.ctx.current_page(),
        Page::ArtistSongs { videos: false, .. }
    ));
    let data = scene.ui.global::<PlaylistData>();
    assert_eq!(data.get_title(), "All songs");
    assert_eq!(data.get_byline(), "Daft Punk");
    assert!(data.get_round() && !data.get_artist_id().is_empty());
    assert!(data.get_tracks().row_count() > 0);
    assert!(label::shows(&scene, "All songs"));
}

#[test]
fn a_shelf_that_links_nowhere_has_no_show_all() {
    let scene = artist();
    let data = scene.ui.global::<ArtistData>();
    assert_eq!(
        data.get_more().iter().collect::<Vec<_>>(),
        [true, true, true, true],
        "the recorded page links each of its four shelves"
    );
    let shown = || label::all(&scene, "Show all").len();
    assert_eq!(shown(), 4);
    // Only Albums goes on.
    data.set_more(ModelRc::new(VecModel::from(vec![
        false, true, false, false,
    ])));
    scene.settle();
    assert_eq!(shown(), 1);
}
