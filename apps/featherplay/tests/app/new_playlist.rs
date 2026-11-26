//! New playlist from the sidebar's first row, on the offscreen platform with fixture data
//! (nothing goes out; the fixtures accept the write): Rename's dialog, empty; Create with no
//! name keeps it open with the reason; a name closes it and the new playlist opens.

use featherplay::snapshot::Scene;
use featherplay::ui::{CardItem, Nav, PageKind, PlaylistDialog, PlaylistMenu, SidebarData};
use featherplay_core::model::PlaylistId;
use featherplay_ytm::PlaylistSummary;
use slint::{ComponentHandle, Model};

use crate::support::label;

#[test]
fn a_new_playlist_from_the_sidebar_opens_once_it_has_a_name() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    let menu = scene.ui.global::<PlaylistMenu>();

    label::click(&scene, "New playlist");
    assert_eq!(menu.get_dialog(), PlaylistDialog::Create);
    assert_eq!(menu.get_draft(), "", "the field starts empty");
    assert_eq!(menu.get_new_song(), "", "no song goes in from the sidebar");

    // Only spaces: the dialog stays, saying why.
    menu.set_draft("   ".into());
    label::click(&scene, "Create");
    scene.settle();
    assert_eq!(
        menu.get_dialog(),
        PlaylistDialog::Create,
        "an empty name closed the dialog"
    );
    assert_eq!(menu.get_dialog_error(), "Enter a name for the playlist.");

    menu.set_draft("Road trip".into());
    label::click(&scene, "Create");
    scene.settle();
    assert_eq!(menu.get_dialog(), PlaylistDialog::None);
    let nav = scene.ui.global::<Nav>();
    assert_eq!(
        nav.get_page(),
        PageKind::Playlist,
        "the new playlist didn't open"
    );
    assert_eq!(nav.get_page_id(), "PLfixturecreated");
}

/// The write's answer puts the new playlist at the top of the sidebar's list and the song
/// menu's Add to playlist at once, before the list is fetched again.
#[test]
fn a_new_playlist_shows_in_the_sidebar_before_the_list_comes_back() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    let data = scene.ui.global::<SidebarData>();
    let listed = data.get_items().row_count();
    assert!(listed > 0, "no playlists in the sidebar");
    featherplay::sidebar::created(
        &scene.ctx,
        &PlaylistSummary {
            id: PlaylistId("PLmadehere".into()),
            title: "Road trip".into(),
            author: None,
            count: Some(0),
            views: None,
            avatar: None,
            thumbnail: None,
            owned: true,
        },
    );
    let first = |m: slint::ModelRc<CardItem>| m.row_data(0).map(|c| (c.id, c.title));
    let made = Some(("PLmadehere".into(), "Road trip".into()));
    assert_eq!(first(data.get_items()), made, "not first in the sidebar");
    assert_eq!(data.get_items().row_count(), listed + 1);
    assert_eq!(first(data.get_playlists()), made, "not in Add to playlist");
}
