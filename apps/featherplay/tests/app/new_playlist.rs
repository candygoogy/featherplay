//! New playlist from the sidebar's first row, on the offscreen platform with fixture data
//! (nothing goes out; the fixtures accept the write): Rename's dialog, empty; Create with no
//! name keeps it open with the reason; a name closes it and the new playlist opens.

use featherplay::snapshot::Scene;
use featherplay::ui::{Nav, PageKind, PlaylistDialog, PlaylistMenu};
use slint::ComponentHandle;

use crate::support::label;

/// One test: the platform can be installed once per process.
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
