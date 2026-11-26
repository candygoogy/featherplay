//! The playlist page's ⋯ for one of the account's own playlists, on the offscreen platform
//! with fixture data (no window is shown, nothing goes out): it opens the sidebar's playlist
//! menu, with Rename and Delete, and a confirmed Delete leaves the page.

use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{PlaylistData, PlaylistDialog, PlaylistKind, PlaylistMenu};
use slint::ComponentHandle;

use crate::support::label;

/// One test: the platform can be installed once per process.
#[test]
fn an_own_playlists_page_menu_renames_and_deletes() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    // The fixtures' first library playlist is the account's own.
    scene.open("playlist").expect("playlist");
    scene.settle();
    let ui = &scene.ui;
    let id = ui.global::<PlaylistData>().get_id();

    label::click(&scene, "More");
    let menu = ui.global::<PlaylistMenu>();
    assert!(
        menu.get_open(),
        "the page's ⋯ didn't open the playlist menu"
    );
    assert_eq!(menu.get_item().id, id);
    assert_eq!(menu.get_kind(), PlaylistKind::Own);
    assert!(label::shows(&scene, "Rename…"));
    assert!(label::shows(&scene, "Delete…"));

    // Delete asks first; confirmed, the write goes (fixtures accept it) and the page is left.
    label::click(&scene, "Delete…");
    assert_eq!(menu.get_dialog(), PlaylistDialog::Delete);
    assert_eq!(menu.get_target().id, id);
    label::click(&scene, "Delete");
    scene.settle();
    assert_eq!(menu.get_dialog(), PlaylistDialog::None);
    assert_ne!(
        scene.ctx.current_page(),
        Page::Playlist {
            id: featherplay_core::model::PlaylistId(id.to_string())
        },
        "the deleted playlist's page stayed open"
    );
}
