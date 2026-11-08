//! The suggestions under the top bar's search field, with the recorded reply for "daft pu"
//! on the offscreen platform and a frozen clock (no window is shown, no request goes out):
//! one or two letters are asked for once the typing pauses, the third at once, and the
//! answers show while the typing goes on; Enter with no row highlighted searches for the
//! text as typed, Enter on a highlighted row picks it (a completion searches for itself, a
//! song plays with its radio); Escape closes the rows first, then clears the field; a link
//! asks for nothing.

use std::time::Duration;

use featherplay::controller::Action;
use featherplay::offscreen;
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::suggest::{INTERVAL, PAUSE};
use featherplay::ui::TopBarData;
use featherplay_core::model::TrackId;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};

/// The recording: six completions, then an artist and four songs.
const ROWS: usize = 11;
/// The first song, "Veridis Quo", after the completions and the artist.
const FIRST_SONG: usize = 7;

fn tap(w: &slint::Window, text: SharedString) {
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

fn ctrl_k(w: &slint::Window) {
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    tap(w, "k".into());
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

/// Ctrl+K, then `text` typed a key every 50 ms: quicker than the pause that asks.
fn type_in_field(scene: &Scene, text: &str) {
    ctrl_k(scene.ui.window());
    keys(scene, text);
}

/// `text` typed into the field that has the keys, a key every 50 ms.
fn keys(scene: &Scene, text: &str) {
    let w = scene.ui.window();
    for c in text.chars() {
        tap(w, c.to_string().into());
        offscreen::advance(Duration::from_millis(50));
        scene.run_queued();
    }
}

fn rows(scene: &Scene) -> usize {
    scene
        .ui
        .global::<TopBarData>()
        .get_suggestions()
        .row_count()
}

fn open(scene: &Scene) -> bool {
    scene.ui.global::<TopBarData>().get_suggesting() && rows(scene) > 0
}

fn search(query: &str) -> Page {
    Page::Search {
        query: query.into(),
    }
}

/// One test: the platform can be installed once per process.
#[test]
fn suggestions_follow_the_typing_and_the_keys() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    scene.take_actions();
    offscreen::freeze_clock();
    let w = scene.ui.window();
    let field = scene.ui.global::<TopBarData>();

    // Two letters quicker than the pause ask for nothing; the third asks at once, and its
    // answer shows while the typing goes on.
    type_in_field(&scene, "da");
    scene.settle();
    assert_eq!(rows(&scene), 0, "two letters asked for before a pause");
    keys(&scene, "f");
    scene.settle();
    assert_eq!(rows(&scene), ROWS, "the third letter's answer didn't show");
    keys(&scene, "t pu");
    offscreen::advance(INTERVAL);
    scene.settle();
    assert_eq!(rows(&scene), ROWS);
    assert!(open(&scene));
    assert_eq!(field.get_highlighted(), -1);

    // Enter with no row highlighted searches for the text as typed, and closes the rows.
    tap(w, Key::Return.into());
    scene.settle();
    assert_eq!(scene.ctx.current_page(), search("daft pu"));
    assert!(!open(&scene));

    // Typed again, the answer kept shows at once. Down, Down, Up: the first completion;
    // Enter searches for it and puts it in the field.
    type_in_field(&scene, "daft pu");
    assert!(open(&scene), "the kept answer didn't show at once");
    for key in [Key::DownArrow, Key::DownArrow, Key::UpArrow] {
        tap(w, key.into());
    }
    assert_eq!(field.get_highlighted(), 0);
    tap(w, Key::Return.into());
    scene.settle();
    assert_eq!(scene.ctx.current_page(), search("daft punk"));
    assert_eq!(field.get_query(), "daft punk");
    assert!(!open(&scene));

    // Escape closes the rows and leaves the text; the next one clears the field.
    type_in_field(&scene, "daft pu");
    assert!(open(&scene));
    tap(w, Key::Escape.into());
    assert!(!open(&scene));
    assert_eq!(field.get_query(), "daft pu");
    assert_eq!(scene.ctx.current_page(), search("daft punk"));
    tap(w, Key::Escape.into());
    assert_eq!(field.get_query(), "");

    // Enter on a song plays it with its radio, as a Search row does, and goes nowhere.
    type_in_field(&scene, "daft pu");
    for _ in 0..=FIRST_SONG {
        tap(w, Key::DownArrow.into());
    }
    assert_eq!(field.get_highlighted(), FIRST_SONG as i32);
    tap(w, Key::Return.into());
    scene.settle();
    let plays: Vec<Action> = scene
        .take_actions()
        .into_iter()
        .filter(|a| matches!(a, Action::Play { .. }))
        .collect();
    match plays.as_slice() {
        [
            Action::Play {
                tracks,
                start: 0,
                radio: true,
            },
        ] => assert_eq!(tracks[0].id, TrackId("qe8Q7mjxjig".into())),
        other => panic!("expected one Play with radio, got {other:?}"),
    }
    assert_eq!(scene.ctx.current_page(), search("daft punk"));
    assert!(!open(&scene));

    // A link asks for nothing: Enter opens what it points at.
    type_in_field(&scene, "https://youtu.be/Jb6gcoR266U");
    offscreen::advance(PAUSE.max(INTERVAL));
    scene.settle();
    assert!(!open(&scene));
    assert_eq!(rows(&scene), 0);
}
