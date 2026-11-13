//! The quick search box on the offscreen platform (no window is shown), over the recorded
//! fixtures, whose suggestions answer any text: results come first with the top one
//! highlighted, Enter plays it and closes the box and the pick comes back among the recent
//! ones, Shift+Enter puts songs next in the order picked and keeps the box open, Ctrl+Enter
//! opens the page in featherplay, Escape closes it and its window goes. With featherplay in
//! front, the key does what Ctrl+K does. Settings' switch registers the key (a stand-in
//! here), says when another app holds it or featherplay's own hotkeys use it, and the search
//! finds the row. `memory` (ignored) prints what the box costs open and after it closed.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay::controller::{AccountAction, Action, Change, QueueAction};
use featherplay::quick_search;
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{
    QuickData, QuickKeyData, QuickSearch, QuickState, SettingId, SettingKey, SettingsData,
    ShortcutKind, Shortcuts, SuggestionKind,
};
use featherplay_integrations::KeyError;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};

use crate::support::private_mb;

fn scene() -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("play");
    scene.settle();
    // A file of this test's own, so picks don't meet another test's or the user's.
    let file = std::env::temp_dir().join(format!(
        "featherplay-quick-search-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&file);
    quick_search::keep_recent_in(file);
    scene
}

fn open(scene: &Scene) -> QuickSearch {
    let quick = quick_search::create(&scene.ctx).expect("quick search");
    quick.show().expect("show");
    quick
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    scene.settle();
    quick
}

fn typed(scene: &Scene, quick: &QuickSearch, text: &str) {
    quick_search::type_for_snapshot(&scene.ctx, quick, text);
    scene.settle();
}

/// Presses `key` in the box, Shift or Ctrl held as asked.
fn press(scene: &Scene, quick: &QuickSearch, key: Key, shift: bool, ctrl: bool) {
    let w = quick.window();
    let send = |text: SharedString, down: bool| {
        w.dispatch_event(if down {
            WindowEvent::KeyPressed { text }
        } else {
            WindowEvent::KeyReleased { text }
        });
    };
    let held: Vec<SharedString> = [(shift, Key::Shift), (ctrl, Key::Control)]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, k)| k.into())
        .collect();
    for k in &held {
        send(k.clone(), true);
    }
    send(key.into(), true);
    send(key.into(), false);
    for k in held.iter().rev() {
        send(k.clone(), false);
    }
    scene.settle();
}

/// The index of the first row of `kind`.
fn first(quick: &QuickSearch, kind: SuggestionKind) -> usize {
    let rows = quick.global::<QuickData>().get_rows();
    (0..rows.row_count())
        .find(|&i| rows.row_data(i).is_some_and(|r| r.kind == kind))
        .unwrap_or_else(|| panic!("no {kind:?} row"))
}

fn down_to(scene: &Scene, quick: &QuickSearch, row: usize) {
    for _ in 0..row {
        press(scene, quick, Key::DownArrow, false, false);
    }
    assert_eq!(quick.global::<QuickData>().get_highlighted(), row as i32);
}

#[test]
fn results_come_first_and_enter_plays_the_pick_and_closes_the_box() {
    let scene = scene();
    let quick = open(&scene);
    let data = quick.global::<QuickData>();
    assert_eq!(data.get_state(), QuickState::Fresh, "no picks yet");

    typed(&scene, &quick, "daft");
    assert_eq!(data.get_state(), QuickState::Results);
    let rows = data.get_rows();
    let from = data.get_completions_from();
    assert!(from > 0, "no results before the completions");
    for i in 0..rows.row_count() {
        let completion = rows.row_data(i).unwrap().kind == SuggestionKind::Query;
        assert_eq!(completion, i as i32 >= from, "row {i}");
    }
    assert_eq!(data.get_highlighted(), 0, "the top result is highlighted");

    let song = first(&quick, SuggestionKind::Song);
    down_to(&scene, &quick, song);
    scene.take_actions();
    press(&scene, &quick, Key::Return, false, false);
    assert!(!quick_search::is_open(), "Enter left the box open");
    let played = scene
        .take_actions()
        .into_iter()
        .any(|a| matches!(a, Action::Play { ref tracks, radio: true, .. } if tracks.len() == 1));
    assert!(played, "the song didn't play with its radio");

    // Open again: the pick leads the recent ones, and nothing was asked for.
    let quick = open(&scene);
    let data = quick.global::<QuickData>();
    assert_eq!(data.get_state(), QuickState::Recent);
    let pick = data.get_rows().row_data(0).unwrap();
    assert_eq!(pick.kind, SuggestionKind::Song);
    quick_search::close(false);
}

#[test]
fn shift_enter_plays_songs_next_in_the_order_picked_and_keeps_the_box_open() {
    let scene = scene();
    let quick = open(&scene);
    typed(&scene, &quick, "daft");
    let song = first(&quick, SuggestionKind::Song);
    down_to(&scene, &quick, song);
    scene.take_actions();

    press(&scene, &quick, Key::Return, true, false);
    assert!(quick_search::is_open(), "Shift+Enter closed the box");
    let rows = quick.global::<QuickData>().get_rows();
    assert_eq!(rows.row_data(song).unwrap().tag, "Plays next");
    let first_id = match scene.take_actions().as_slice() {
        [
            Action::Queue(QueueAction::PlayAfter {
                tracks,
                after: None,
            }),
        ] => tracks[0].id.clone(),
        other => panic!("not played next: {other:?}"),
    };

    // The second goes after the first.
    press(&scene, &quick, Key::Return, true, false);
    assert_eq!(rows.row_data(song).unwrap().tag, "Plays after it");
    match scene.take_actions().as_slice() {
        [
            Action::Queue(QueueAction::PlayAfter {
                after: Some(after), ..
            }),
        ] => {
            assert_eq!(*after, first_id)
        }
        other => panic!("not after the first: {other:?}"),
    }
    quick_search::close(false);
}

#[test]
fn ctrl_enter_opens_the_page_in_featherplay_and_escape_closes_the_box() {
    let scene = scene();
    let quick = open(&scene);
    typed(&scene, &quick, "daft");
    let artist = first(&quick, SuggestionKind::Artist);
    down_to(&scene, &quick, artist);
    press(&scene, &quick, Key::Return, false, true);
    assert!(!quick_search::is_open());
    assert!(
        matches!(scene.ctx.current_page(), Page::Artist { .. }),
        "the artist's page didn't open: {:?}",
        scene.ctx.current_page()
    );

    // Escape: closed, and its window goes with it.
    let quick = open(&scene);
    let weak = quick.as_weak();
    drop(quick);
    let w = weak.upgrade().expect("open");
    let esc: SharedString = Key::Escape.into();
    w.window()
        .dispatch_event(WindowEvent::KeyPressed { text: esc.clone() });
    w.window()
        .dispatch_event(WindowEvent::KeyReleased { text: esc });
    drop(w);
    scene.settle();
    assert!(!quick_search::is_open(), "Escape left the box open");
    assert!(weak.upgrade().is_none(), "the closed box's window stayed");
}

#[test]
fn with_featherplay_in_front_the_key_does_what_ctrl_k_does() {
    let scene = scene();
    let used = Rc::new(RefCell::new(Vec::new()));
    let seen = used.clone();
    scene
        .ui
        .global::<Shortcuts>()
        .on_used(move |kind| seen.borrow_mut().push(kind));
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    quick_search::search_in_main(&scene.ctx);
    assert_eq!(used.borrow().as_slice(), [ShortcutKind::Search]);
    assert!(!quick_search::is_open(), "the box opened over featherplay");
}

thread_local! {
    static REGISTERED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

#[test]
fn the_switch_registers_the_key_and_says_when_it_cant() {
    let scene = scene();
    quick_search::register_keys_with(|key| {
        REGISTERED.with_borrow_mut(|r| r.push(key.to_string()));
        Ok(None)
    });
    scene.open("settings").expect("settings");
    scene.settle();
    let data = scene.ui.global::<QuickKeyData>();
    assert!(!data.get_on(), "on by default");

    data.invoke_toggled(true);
    assert!(quick_search::key_registered());
    assert_eq!(REGISTERED.with_borrow(Clone::clone), ["Ctrl+Alt+K"]);
    assert_eq!(data.get_error(), "");
    let saved = scene.take_actions().into_iter().any(|a| {
        matches!(
            a,
            Action::Account(AccountAction::Set(Change::QuickSearch(true)))
        )
    });
    assert!(saved, "the switch wasn't saved");

    // Another app holds the key: a calm line, and the switch stays on.
    quick_search::register_keys_with(|_| Err(KeyError::Taken("held".into())));
    data.invoke_reset();
    assert_eq!(
        data.get_error(),
        "Another app already uses Ctrl+Alt+K. Pick another key."
    );
    assert!(data.get_on());

    // Picks that can't be the quick search's are refused, not saved, and the key before
    // stays: featherplay's own Play or pause key, a key the system keeps, a bare letter.
    quick_search::register_keys_with(|_| Ok(None));
    let settings = scene.ui.global::<SettingsData>();
    settings.invoke_toggled(SettingKey::Hotkeys, true);
    scene.take_actions();
    let delete: SharedString = Key::Delete.into();
    for (text, ctrl, alt, line) in [
        (
            SharedString::from(" "),
            true,
            true,
            "featherplay already uses Ctrl+Alt+Space for Play or pause.",
        ),
        (
            delete,
            true,
            true,
            "Ctrl+Alt+Delete belongs to the system. Pick another key.",
        ),
        (
            SharedString::from("k"),
            false,
            false,
            if cfg!(windows) {
                "A shortcut needs Ctrl, Alt, Shift or Win."
            } else {
                "A shortcut needs Ctrl, Alt, Shift or Super."
            },
        ),
    ] {
        data.invoke_listen(true);
        data.invoke_captured(text, ctrl, alt, false, false);
        assert_eq!(data.get_error(), line);
        data.invoke_listen(false);
        assert_eq!(data.get_key_text(), "Ctrl+Alt+K", "{line}");
        assert!(quick_search::key_registered(), "{line}");
    }
    let saved_key = scene.take_actions().into_iter().any(|a| {
        matches!(
            a,
            Action::Account(AccountAction::Set(Change::QuickSearchKey(_)))
        )
    });
    assert!(!saved_key, "a refused key was saved");

    data.invoke_toggled(false);
    assert!(!quick_search::key_registered());
    assert_eq!(data.get_error(), "");
}

#[test]
fn a_corrupt_recent_file_or_a_hand_edited_key_never_breaks_it() {
    let scene = scene();
    let file = std::env::temp_dir().join(format!(
        "featherplay-quick-search-corrupt-{}.json",
        std::process::id()
    ));
    std::fs::write(&file, b"{ not the picks").expect("write");
    quick_search::keep_recent_in(file.clone());
    assert!(quick_search::load_recent().is_empty());
    let quick = open(&scene);
    assert_eq!(quick.global::<QuickData>().get_state(), QuickState::Fresh);
    drop(quick);
    quick_search::close(false);
    let _ = std::fs::remove_file(&file);

    // A saved key it can't use (empty, unknown, without a modifier) falls back to the default.
    for stored in ["", "Ctrl+Nope", "K", "Hyper+Ctrl+K+Q"] {
        assert_eq!(quick_search::usable_key(stored), "Ctrl+Alt+K", "{stored:?}");
    }
    assert_eq!(
        quick_search::usable_key("Ctrl+Shift+Space"),
        "Ctrl+Shift+Space"
    );
}

#[test]
fn settings_search_finds_the_row() {
    let scene = scene();
    for text in ["quick search", "launcher", "hotkey"] {
        let found = featherplay::pages::settings::search::suggestions(&scene.ui, text);
        assert!(
            found.iter().any(|s| s.id == SettingId::QuickSearch),
            "{text:?} didn't find it"
        );
    }
}

/// What the box costs, in private memory: before it ever opened (the main window drawn),
/// open with results drawn, closed, and closed again after ten more openings. Closed, it
/// should cost nothing: its window, timer and suggester go.
#[test]
#[ignore = "a measurement: cargo nextest run -p featherplay --run-ignored only -E 'test(quick_search::memory)'"]
#[allow(clippy::print_stdout, reason = "a measurement reports its numbers")]
fn memory() {
    let scene = scene();
    let cycle = |frame: bool| {
        let quick = open(&scene);
        typed(&scene, &quick, "daft");
        if frame {
            quick.window().take_snapshot().expect("frame");
        }
        let open = private_mb();
        drop(quick);
        quick_search::close(false);
        scene.settle();
        open
    };
    scene.ui.window().take_snapshot().expect("frame");
    scene.settle();
    let before = private_mb();
    let open = cycle(true);
    let closed = private_mb();
    for _ in 0..10 {
        cycle(true);
    }
    let after = private_mb();
    println!(
        "quick search: never opened {before:.2} MB; open {open:.2} MB ({:+.2}); closed {closed:.2} MB ({:+.2}); after 10 more {after:.2} MB ({:+.2})",
        open - before,
        closed - before,
        after - before
    );
}
