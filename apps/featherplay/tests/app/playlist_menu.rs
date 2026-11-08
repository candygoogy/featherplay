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

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

/// The sidebar's first list row (the list starts 280 px down, under the chips; rows are
/// 40 px), and Liked songs (188 px down).
const FIRST_ROW: f32 = 296.0;
const SECOND_ROW: f32 = 336.0;
const LIKED: f32 = 208.0;

fn click(w: &slint::Window, p: LogicalPosition, button: PointerEventButton) {
    w.dispatch_event(WindowEvent::PointerMoved { position: p });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: p,
        button,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: p,
        button,
    });
}

fn right_click(scene: &Scene, y: f32) {
    click(
        scene.ui.window(),
        LogicalPosition::new(120.0, y),
        PointerEventButton::Right,
    );
}

fn key(w: &slint::Window, k: Key) {
    w.dispatch_event(WindowEvent::KeyPressed { text: k.into() });
    w.dispatch_event(WindowEvent::KeyReleased { text: k.into() });
}

/// The rows the open menu shows, top to bottom, as screen readers find them.
fn row_y(scene: &Scene, label: &str) -> Option<f32> {
    // 8 px padding, the 56 px head, a 9 px divider, then 36 px rows with 9 px dividers
    // between the groups. It opens down from the anchor, or up from it when it wouldn't fit
    // above the bar.
    let menu = scene.ui.global::<PlaylistMenu>();
    let pin = if menu.get_pinned() {
        "Unpin"
    } else {
        "Pin to top"
    };
    let groups: Vec<Vec<&str>> = match menu.get_kind() {
        PlaylistKind::Own => vec![
            vec!["Play", "Shuffle", "Start radio"],
            vec!["Play next", "Add to queue"],
            vec!["Copy link", pin],
            vec!["Rename…", "Delete…"],
        ],
        PlaylistKind::Saved => vec![
            vec!["Play", "Shuffle", "Start radio"],
            vec!["Play next", "Add to queue"],
            vec!["Copy link", pin],
            vec!["Remove from library"],
        ],
        PlaylistKind::Liked => vec![
            vec!["Play", "Shuffle"],
            vec!["Play next", "Add to queue"],
            vec!["Copy link"],
        ],
        PlaylistKind::Album => vec![vec!["Play", "Shuffle", "Go to album"], vec![pin]],
        PlaylistKind::Artist => vec![vec!["Play", "Shuffle", "Go to artist"], vec![pin]],
    };
    let rows: usize = groups.iter().map(Vec::len).sum();
    let h = 16.0 + 56.0 + 9.0 + rows as f32 * 36.0 + (groups.len() - 1) as f32 * 9.0;
    let bottom = HEIGHT as f32 - scene.ui.global::<Theme>().get_bar_clear();
    let top = if menu.get_anchor_y() + h + 8.0 > bottom {
        (menu.get_anchor_top() - h).max(8.0)
    } else {
        menu.get_anchor_y()
    };
    let mut y = top + 8.0 + 56.0 + 9.0;
    for group in &groups {
        for row in group {
            if *row == label {
                return Some(y + 18.0);
            }
            y += 36.0;
        }
        y += 9.0;
    }
    None
}

fn click_row(scene: &Scene, label: &str) {
    let menu = scene.ui.global::<PlaylistMenu>();
    let y = row_y(scene, label).unwrap_or_else(|| panic!("no {label} row"));
    let x = menu.get_anchor_x() + 120.0;
    click(
        scene.ui.window(),
        LogicalPosition::new(x, y),
        PointerEventButton::Left,
    );
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
    right_click(&scene, FIRST_ROW);
    assert!(menu.get_open(), "a right-click didn't open the menu");
    assert_eq!(menu.get_item().id, own.id);
    assert_eq!(menu.get_kind(), PlaylistKind::Own);
    // Opens at the pointer, down and to the right.
    assert_eq!(menu.get_anchor_x(), 120.0);
    assert_eq!(menu.get_anchor_y(), FIRST_ROW);
    key(w, Key::Escape);
    assert!(!menu.get_open(), "Escape didn't close the menu");

    // A saved one.
    right_click(&scene, SECOND_ROW);
    assert_eq!(menu.get_item().id, saved.id);
    assert_eq!(menu.get_kind(), PlaylistKind::Saved);
    // A click outside closes it.
    click(
        w,
        LogicalPosition::new(900.0, 500.0),
        PointerEventButton::Left,
    );
    assert!(!menu.get_open(), "a click outside didn't close the menu");

    // Liked songs.
    right_click(&scene, LIKED);
    assert!(menu.get_open(), "Liked songs has no menu");
    assert_eq!(menu.get_item().id, "LM");
    assert_eq!(menu.get_kind(), PlaylistKind::Liked);

    // Copy link: the playlist's link on the clipboard, and a toast says so.
    click_row(&scene, "Copy link");
    assert!(!menu.get_open(), "Copy link left the menu open");
    assert_eq!(
        offscreen::clipboard().as_deref(),
        Some("https://music.youtube.com/playlist?list=LM")
    );
    assert_eq!(ui.global::<Toast>().get_text(), "Link copied");

    // Delete asks first; Cancel leaves the playlist alone.
    right_click(&scene, FIRST_ROW);
    click_row(&scene, "Delete…");
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
    right_click(&scene, FIRST_ROW);
    click_row(&scene, "Delete…");
    menu.invoke_delete(own.id.clone());
    scene.settle();
    assert_eq!(menu.get_dialog(), PlaylistDialog::None);
    assert_eq!(
        ui.global::<Toast>().get_text(),
        format!("Deleted \u{201C}{}\u{201D}", own.title)
    );

    // Rename starts from the current name and refuses an empty one.
    right_click(&scene, FIRST_ROW);
    click_row(&scene, "Rename…");
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
    right_click(&scene, FIRST_ROW);
    click_row(&scene, "Rename…");
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
    right_click(&scene, SECOND_ROW);
    click_row(&scene, "Remove from library");
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
    right_click(&scene, SECOND_ROW);
    assert!(!menu.get_pinned());
    click_row(&scene, "Pin to top");
    assert_eq!(
        sidebar.get_items().row_data(0).expect("first").id,
        second.id
    );
    assert_eq!(sidebar.get_pinned().row_data(0), Some(true));
    assert_eq!(
        ui.global::<Toast>().get_text(),
        format!("Pinned \u{201C}{}\u{201D} to the top", second.title)
    );
    right_click(&scene, FIRST_ROW);
    assert!(menu.get_pinned());
    click_row(&scene, "Unpin");
    assert_eq!(sidebar.get_pinned().row_data(0), Some(false));

    // The Albums chip: its entries get the album menu.
    sidebar.invoke_chip_picked(1);
    scene.settle();
    assert_eq!(sidebar.get_chip(), 1);
    let album = sidebar.get_items().row_data(0).expect("an album");
    right_click(&scene, FIRST_ROW);
    assert_eq!(menu.get_kind(), PlaylistKind::Album);
    assert_eq!(menu.get_item().id, album.id);
    key(w, Key::Escape);
}
