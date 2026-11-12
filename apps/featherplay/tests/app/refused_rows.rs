//! A song YouTube refuses (`UiEvent::Refused`) shows greyed in the lists, its row saying why
//! to screen readers (and on hover), from the moment it's marked to the moment the mark goes,
//! with the list already on screen. Offscreen, with the fixture playlist.

use std::ops::ControlFlow;

use featherplay::app;
use featherplay::controller::UiEvent;
use featherplay::snapshot::Scene;
use featherplay::ui::{PlaylistData, TrackRow};
use featherplay_controller::place::Place;
use featherplay_core::model::{PlaylistId, TrackId};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::window::WindowInner;
use slint::{ComponentHandle, Model};

const WHY: &str = "YouTube won't play this song in featherplay right now";

/// What screen readers hear of the shown row labelled `label`, beyond its label.
fn description(scene: &Scene, label: &str) -> String {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    root.visit_descendants(|item| {
        let says = item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|l| l == label);
        if says && item.is_visible() {
            ControlFlow::Break(
                item.accessible_string_property(AccessibleStringProperty::Description)
                    .unwrap_or_default()
                    .to_string(),
            )
        } else {
            ControlFlow::Continue(())
        }
    })
    .unwrap_or_else(|| panic!("no row {label:?} shown"))
}

#[test]
fn a_refused_songs_row_says_why_while_it_is_marked() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    let mix = Place::Playlist(PlaylistId(
        "RDTMAK5uy_FIXTURE_MIX_100000000000000000000".into(),
    ));
    scene.ctx.navigate(mix.into());
    scene.settle();
    let rows = scene.ui.global::<PlaylistData>().get_tracks();
    let (refused, other) = (
        rows.row_data(1).expect("row"),
        rows.row_data(2).expect("row"),
    );
    let id = refused.id.clone();
    // As the playlist's rows are labelled.
    let label = |r: &TrackRow| format!("{}, {}", r.title, r.artist);
    let (refused, other) = (label(&refused), label(&other));
    assert_eq!(description(&scene, &refused), "");

    app::on_event(UiEvent::Refused(vec![TrackId(id.to_string())]));
    scene.settle();
    assert_eq!(description(&scene, &refused), WHY);
    assert_eq!(description(&scene, &other), "", "only the refused one");

    // It played after all.
    app::on_event(UiEvent::Refused(Vec::new()));
    scene.settle();
    assert_eq!(description(&scene, &refused), "");
}
