//! The album page's ⋯ menu rates a whole album, on the offscreen platform with fixture data
//! and a fake for the writes (nothing goes out): only the songs that need it get a write, in
//! the album's order; the first failed write stops the job and the toast says how far it
//! got; Cancel in the confirmation sends nothing.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay::album_actions::{self, Rater};
use featherplay::controller::Action;
use featherplay::snapshot::Scene;
use featherplay::ui::{AlbumAction, AlbumData, AlbumMenu, Theme, Toast};
use featherplay_core::model::{LikeStatus, TrackId};
use featherplay_ytm::ApiError;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};

/// Records every write; the `fail_at`th one (from 1) fails as a dropped connection would.
#[derive(Clone, Default)]
struct Fake {
    calls: Rc<RefCell<Vec<(String, LikeStatus)>>>,
    fail_at: Option<usize>,
}

impl Rater for Fake {
    async fn rate(&self, id: &TrackId, status: LikeStatus) -> Result<(), ApiError> {
        let mut calls = self.calls.borrow_mut();
        calls.push((id.0.clone(), status));
        if Some(calls.len()) == self.fail_at {
            return Err(ApiError::Network("connection reset".into()));
        }
        Ok(())
    }

    async fn pause(&self) {}
}

fn album() -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("album").expect("album");
    scene.settle();
    assert!(
        scene.ui.global::<AlbumData>().get_tracks().row_count() > 4,
        "the fixture album has too few tracks"
    );
    scene
}

fn ids(scene: &Scene) -> Vec<String> {
    let rows = scene.ui.global::<AlbumData>().get_tracks();
    rows.iter().map(|r| r.id.to_string()).collect()
}

fn liked(scene: &Scene) -> Vec<bool> {
    let rows = scene.ui.global::<AlbumData>().get_tracks();
    rows.iter().map(|r| r.liked).collect()
}

fn rated(actions: &[Action]) -> usize {
    actions
        .iter()
        .filter(|a| matches!(a, Action::Rate(..) | Action::Rated(..)))
        .count()
}

fn toast(scene: &Scene) -> String {
    scene.ui.global::<Toast>().get_text().to_string()
}

#[test]
fn like_all_rates_only_the_songs_not_liked_yet_in_order() {
    let scene = album();
    let data = scene.ui.global::<AlbumData>();
    let menu = scene.ui.global::<AlbumMenu>();
    let title = data.get_title().to_string();
    let all = ids(&scene);
    // Two songs liked the usual way, from their hearts.
    data.invoke_like(1);
    data.invoke_like(4);
    scene.take_actions();

    menu.invoke_pick(AlbumAction::LikeAll);
    let n = all.len() - 2;
    assert!(menu.get_dialog(), "Like all songs didn't ask first");
    assert_eq!(
        menu.get_question(),
        format!("Like {n} songs on \u{201C}{title}\u{201D}?")
    );
    assert_eq!(menu.get_ok_label(), "Like");

    let fake = Fake::default();
    album_actions::confirm_with(&scene.ctx, fake.clone());
    assert!(!menu.get_dialog());
    assert!(
        menu.get_busy(),
        "the menu's actions stay on while the job runs"
    );
    scene.settle();

    let wanted: Vec<(String, LikeStatus)> = all
        .iter()
        .enumerate()
        .filter(|(i, _)| ![1, 4].contains(i))
        .map(|(_, id)| (id.clone(), LikeStatus::Like))
        .collect();
    assert_eq!(*fake.calls.borrow(), wanted);
    assert_eq!(toast(&scene), format!("Liked {n} songs"));
    assert!(liked(&scene).iter().all(|l| *l), "a row's heart wasn't set");
    // The controller hears of each one (the bar, the queue), without a second write.
    let actions = scene.take_actions();
    assert_eq!(
        actions
            .iter()
            .filter(|a| matches!(a, Action::Rated(_, LikeStatus::Like)))
            .count(),
        n
    );
    assert!(!actions.iter().any(|a| matches!(a, Action::Rate(..))));
    assert!(!menu.get_busy());

    // Nothing left to like: a toast says so, and nothing is asked or sent.
    menu.invoke_pick(AlbumAction::LikeAll);
    assert!(!menu.get_dialog());
    assert_eq!(toast(&scene), "Every song on this album is already liked");
    scene.settle();
    assert_eq!(rated(&scene.take_actions()), 0);
    assert_eq!(fake.calls.borrow().len(), n);
}

#[test]
fn the_first_error_stops_the_job_and_the_toast_says_how_far_it_got() {
    let scene = album();
    let menu = scene.ui.global::<AlbumMenu>();
    let all = ids(&scene);

    menu.invoke_pick(AlbumAction::LikeAll);
    let fake = Fake {
        fail_at: Some(3),
        ..Fake::default()
    };
    album_actions::confirm_with(&scene.ctx, fake.clone());
    scene.settle();

    assert_eq!(
        fake.calls.borrow().len(),
        3,
        "the job went on after an error"
    );
    assert_eq!(
        toast(&scene),
        format!("Stopped after 2 of {}: no connection", all.len())
    );
    let hearts = liked(&scene);
    assert_eq!(&hearts[..3], [true, true, false]);
    assert!(hearts[3..].iter().all(|l| !l));
    assert_eq!(rated(&scene.take_actions()), 2);
    assert!(!menu.get_busy());
}

#[test]
fn cancel_in_the_confirmation_sends_nothing() {
    let scene = album();
    let ui = &scene.ui;
    let w = ui.window();
    let menu = ui.global::<AlbumMenu>();

    // The header's ⋯ opens the menu below itself.
    album_actions::show_for_snapshot(&scene.ctx, false);
    scene.settle();
    assert!(menu.get_open(), "the ⋯ didn't open the menu");

    // Its first row: 8 px of padding, then 36 px rows.
    let s2 = ui.global::<Theme>().get_s2();
    let row = LogicalPosition::new(menu.get_anchor_x() + 100.0, menu.get_anchor_y() + s2 + 18.0);
    for e in [
        WindowEvent::PointerMoved { position: row },
        WindowEvent::PointerPressed {
            position: row,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position: row,
            button: PointerEventButton::Left,
        },
    ] {
        w.dispatch_event(e);
    }
    assert!(!menu.get_open());
    assert!(menu.get_dialog(), "Like all songs didn't ask first");
    scene.settle();

    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    assert!(!menu.get_dialog(), "Escape didn't cancel");
    scene.settle();
    assert_eq!(rated(&scene.take_actions()), 0);
    assert!(liked(&scene).iter().all(|l| !l));
    assert!(!menu.get_busy());
    let said = toast(&scene);
    assert!(
        !said.starts_with("Lik") && !said.starts_with("Stopped"),
        "a toast spoke of a job: {said}"
    );
}
