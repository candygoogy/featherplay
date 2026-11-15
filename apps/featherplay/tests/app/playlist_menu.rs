//! A right-click on a sidebar playlist opens the playlist menu, on the offscreen platform
//! with fixture data (no window is shown, nothing goes out): the account's own playlist
//! offers Rename and Delete, a saved one Remove from library, Liked songs neither. Copy link
//! puts the playlist's link on the clipboard; Delete asks first; Rename refuses an empty
//! name. Escape and a click outside close the menu. Pin to top moves an entry to the top of
//! the sidebar's list; the Albums chip's entries get Play, Shuffle, Go to album and Pin.

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{PlaylistDialog, PlaylistKind, PlaylistMenu, SidebarData, Theme, Toast};
use featherplay_core::model::PlaylistId;
use featherplay_ytm::PlaylistSummary;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};

use crate::support::{click_with, label};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

/// What the sidebar's `i`th list entry says.
fn entry(scene: &Scene, i: usize) -> String {
    let sidebar = scene.ui.global::<SidebarData>();
    let title = sidebar.get_items().row_data(i).expect("an entry").title;
    if sidebar.get_pinned().row_data(i).unwrap_or(false) {
        format!("{title}, pinned")
    } else {
        title.to_string()
    }
}

/// Right-clicks the middle of the sidebar's entry labelled `name`; where it clicked.
fn right_click(scene: &Scene, name: &str) -> LogicalPosition {
    let side = scene.ui.global::<Theme>().get_sidebar_w();
    let found = label::shown_where(scene, |item, l| {
        l == name && label::rect(item).max_x() <= side
    });
    let c = label::rect(
        found
            .first()
            .unwrap_or_else(|| panic!("no {name:?} in the sidebar")),
    )
    .center();
    let at = LogicalPosition::new(c.x, c.y);
    click_with(scene, at, PointerEventButton::Right);
    at
}

fn key(w: &slint::Window, k: Key) {
    w.dispatch_event(WindowEvent::KeyPressed { text: k.into() });
    w.dispatch_event(WindowEvent::KeyReleased { text: k.into() });
}

/// One test: the platform can be installed once per process.
#[test]
fn the_sidebar_playlist_menu() {
    let scene = Scene::new(WIDTH, HEIGHT, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    let ui = &scene.ui;
    let w = ui.window();
    let menu = ui.global::<PlaylistMenu>();
    let playlists = ui.global::<SidebarData>().get_playlists();
    assert!(
        playlists.row_count() > 1,
        "the sidebar has no fixture playlists"
    );
    let own = playlists.row_data(0).expect("first");
    let saved = playlists.row_data(1).expect("second");

    // The account's own playlist.
    let at = right_click(&scene, &entry(&scene, 0));
    assert!(menu.get_open(), "a right-click didn't open the menu");
    assert_eq!(menu.get_item().id, own.id);
    assert_eq!(menu.get_kind(), PlaylistKind::Own);
    // Opens at the pointer, down and to the right.
    assert_eq!(menu.get_anchor_x(), at.x);
    assert_eq!(menu.get_anchor_y(), at.y);
    key(w, Key::Escape);
    assert!(!menu.get_open(), "Escape didn't close the menu");

    // A saved one.
    right_click(&scene, &entry(&scene, 1));
    assert_eq!(menu.get_item().id, saved.id);
    assert_eq!(menu.get_kind(), PlaylistKind::Saved);
    // A click outside closes it.
    click_with(
        w,
        LogicalPosition::new(900.0, 500.0),
        PointerEventButton::Left,
    );
    assert!(!menu.get_open(), "a click outside didn't close the menu");

    // Liked songs.
    right_click(&scene, "Liked songs");
    assert!(menu.get_open(), "Liked songs has no menu");
    assert_eq!(menu.get_item().id, "LM");
    assert_eq!(menu.get_kind(), PlaylistKind::Liked);

    // Copy link: the playlist's link on the clipboard, and a toast says so.
    label::click(&scene, "Copy link");
    assert!(!menu.get_open(), "Copy link left the menu open");
    assert_eq!(
        offscreen::clipboard().as_deref(),
        Some("https://music.youtube.com/playlist?list=LM")
    );
    assert_eq!(ui.global::<Toast>().get_text(), "Link copied");

    // Delete asks first; Cancel leaves the playlist alone.
    right_click(&scene, &entry(&scene, 0));
    label::click(&scene, "Delete…");
    assert!(!menu.get_open());
    assert_eq!(menu.get_dialog(), PlaylistDialog::Delete);
    assert_eq!(menu.get_target().id, own.id);
    key(w, Key::Escape);
    assert_eq!(
        menu.get_dialog(),
        PlaylistDialog::None,
        "Escape didn't cancel"
    );

    // Confirmed, it's deleted (fixtures accept the write) and the toast names it.
    right_click(&scene, &entry(&scene, 0));
    label::click(&scene, "Delete…");
    menu.invoke_delete(own.id.clone());
    scene.settle();
    assert_eq!(menu.get_dialog(), PlaylistDialog::None);
    assert_eq!(
        ui.global::<Toast>().get_text(),
        format!("Deleted \u{201C}{}\u{201D}", own.title)
    );

    // Rename starts from the current name and refuses an empty one.
    right_click(&scene, &entry(&scene, 0));
    label::click(&scene, "Rename…");
    assert_eq!(menu.get_dialog(), PlaylistDialog::Rename);
    assert_eq!(menu.get_draft(), own.title);
    menu.invoke_rename(own.id.clone(), "   ".into());
    assert_eq!(
        menu.get_dialog(),
        PlaylistDialog::Rename,
        "an empty name closed the dialog"
    );
    assert!(
        !menu.get_dialog_error().is_empty(),
        "an empty name gave no reason"
    );
    menu.invoke_rename(own.id.clone(), "  Road trip ".into());
    scene.settle();
    assert_eq!(menu.get_dialog(), PlaylistDialog::None);
    assert_eq!(
        ui.global::<Toast>().get_text(),
        "Renamed to \u{201C}Road trip\u{201D}"
    );

    // A name in styled letters, shown plain: Rename starts from it as written, and saving
    // it unchanged writes nothing.
    let styled = "𝙍𝙤𝙖𝙙 𝙩𝙧𝙞𝙥";
    featherplay::playlist_menu::remember(&[PlaylistSummary {
        id: PlaylistId(own.id.to_string()),
        title: styled.into(),
        author: None,
        count: None,
        views: None,
        avatar: None,
        thumbnail: None,
        owned: true,
    }]);
    right_click(&scene, &entry(&scene, 0));
    label::click(&scene, "Rename…");
    assert_eq!(menu.get_draft(), styled);
    ui.global::<Toast>().set_text("".into());
    menu.invoke_rename(own.id.clone(), menu.get_draft());
    scene.settle();
    assert_eq!(menu.get_dialog(), PlaylistDialog::None);
    assert_eq!(
        ui.global::<Toast>().get_text(),
        "",
        "an unchanged name was sent"
    );

    // Remove from library, on the saved one.
    right_click(&scene, &entry(&scene, 1));
    label::click(&scene, "Remove from library");
    scene.settle();
    assert!(!menu.get_open());
    assert_eq!(
        ui.global::<Toast>().get_text(),
        format!("Removed \u{201C}{}\u{201D} from your library", saved.title)
    );

    // Pin to top: the second entry goes to the top of the list, marked; the menu then
    // offers Unpin.
    let sidebar = ui.global::<SidebarData>();
    let second = sidebar.get_items().row_data(1).expect("a second entry");
    right_click(&scene, &entry(&scene, 1));
    assert!(!menu.get_pinned());
    label::click(&scene, "Pin to top");
    assert_eq!(
        sidebar.get_items().row_data(0).expect("first").id,
        second.id
    );
    assert_eq!(sidebar.get_pinned().row_data(0), Some(true));
    assert_eq!(
        ui.global::<Toast>().get_text(),
        format!("Pinned \u{201C}{}\u{201D} to the top", second.title)
    );
    right_click(&scene, &entry(&scene, 0));
    assert!(menu.get_pinned());
    label::click(&scene, "Unpin");
    assert_eq!(sidebar.get_pinned().row_data(0), Some(false));

    // The Albums chip: its entries get the album menu.
    sidebar.invoke_chip_picked(1);
    scene.settle();
    assert_eq!(sidebar.get_chip(), 1);
    let album = sidebar.get_items().row_data(0).expect("an album");
    right_click(&scene, &entry(&scene, 0));
    assert_eq!(menu.get_kind(), PlaylistKind::Album);
    assert_eq!(menu.get_item().id, album.id);
    key(w, Key::Escape);
}
