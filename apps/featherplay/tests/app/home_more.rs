//! Home loads more shelves as the user scrolls near its end, on the offscreen platform (no
//! window is shown) with the fixtures: nothing goes out without a scroll, the next page's
//! shelves (the community shelf) join the end, and once the feed ends scrolling asks no more.

use crate::support::{frames, home_at, wheel_run};
use featherplay::ui::{HomeData, MoreState};
use slint::{ComponentHandle, LogicalPosition, Model};

fn titles(data: &HomeData) -> Vec<String> {
    data.get_shelves()
        .iter()
        .map(|s| s.title.to_string())
        .collect()
}

#[test]
fn scrolling_near_the_end_loads_the_next_page_once() {
    let scene = home_at(1280, 800, 1.0);
    let w = scene.ui.window();
    let data = scene.ui.global::<HomeData>();
    let first = titles(&data);
    assert!(!first.is_empty(), "no shelves loaded");

    // Resting at the top, nothing more loads.
    let _ = frames(w, 30, |_| {});
    scene.settle();
    assert_eq!(titles(&data), first);
    assert_eq!(data.get_more(), MoreState::Idle);

    let at = LogicalPosition::new(700.0, 400.0);
    let _ = wheel_run(w, at, 20, -16.0);
    scene.settle();
    let _ = frames(w, 10, |_| {});
    let after = titles(&data);
    assert_eq!(after.len(), first.len() + 1, "{after:?}");
    assert_eq!(
        after.last().map(String::as_str),
        Some("Trending community playlists")
    );
    assert_eq!(data.get_more(), MoreState::Idle);
    let community = data.get_shelves().row_data(after.len() - 1).expect("shelf");
    assert_eq!(community.badges.row_count(), community.items.row_count());

    // The recording ends there: scrolling on asks for nothing more.
    let _ = wheel_run(w, at, 10, -16.0);
    scene.settle();
    assert_eq!(titles(&data), after);
    assert_eq!(data.get_more(), MoreState::Idle);
}
