//! One featherplay per home (`featherplay::instance`): the guard's names, the named mutex
//! that tells a second launch the first is running, and the message that asks the first to
//! show itself. Nothing here opens a window or touches the user's running featherplay: each
//! test uses a home of its own, so its names are its own.

use std::ffi::OsStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use featherplay::endsession;
use featherplay::instance::{self, Claim};

/// A home no featherplay runs with, so the test's mutex and message are its own.
fn test_key(test: &str) -> String {
    instance::key(Some(OsStr::new(&format!(
        "D:/featherplay-test-{test}-{}",
        std::process::id()
    ))))
}

#[test]
fn an_installed_run_uses_the_fixed_name() {
    assert_eq!(
        instance::mutex_name(&instance::key(None)),
        r"Local\featherplay-featherplay.featherplay"
    );
}

#[test]
fn an_empty_home_counts_as_unset() {
    assert_eq!(instance::key(Some(OsStr::new(""))), instance::key(None));
}

#[test]
fn a_dev_home_has_a_guard_of_its_own() {
    assert_ne!(
        instance::key(Some(OsStr::new("D:/dev/featherplay-home"))),
        instance::key(None)
    );
}

#[test]
fn two_homes_have_two_guards() {
    assert_ne!(
        instance::key(Some(OsStr::new("D:/dev/featherplay-home"))),
        instance::key(Some(OsStr::new("D:/dev/featherplay-other")))
    );
}

#[test]
fn one_home_spelled_two_ways_has_one_guard() {
    assert_eq!(
        instance::key(Some(OsStr::new("D:/dev/featherplay-home"))),
        instance::key(Some(OsStr::new(r"d:\dev\Featherplay-Home\")))
    );
}

#[test]
fn the_names_hold_no_path_separator() {
    let key = instance::key(Some(OsStr::new(r"D:\Workspace\Cache\featherplay-dev")));
    // A kernel object name may have one backslash, after its namespace.
    assert!(!key.contains(['\\', '/', ':']), "{key}");
}

#[test]
fn a_second_claim_finds_the_first_running() {
    let name = instance::mutex_name(&test_key("second"));
    let first = instance::claim(&name).expect("first claim");
    assert!(matches!(first, Claim::First(_)), "the first claim runs");
    let second = instance::claim(&name).expect("second claim");
    assert!(matches!(second, Claim::Running(_)), "the second finds it");
}

#[test]
fn a_claim_after_the_first_has_gone_runs() {
    let name = instance::mutex_name(&test_key("gone"));
    drop(instance::claim(&name).expect("first claim"));
    assert!(matches!(
        instance::claim(&name).expect("claim again"),
        Claim::First(_)
    ));
}

#[test]
fn waiting_takes_over_once_the_first_lets_go() {
    let name = instance::mutex_name(&test_key("wait"));
    let (held_tx, held) = mpsc::channel();
    let (release_tx, release) = mpsc::channel::<()>();
    // The first instance on a thread of its own: a mutex is owned by a thread, so this one
    // can't take it over while the other holds it.
    let thread_name = name.clone();
    let first = std::thread::spawn(move || {
        let guard = instance::claim(&thread_name).expect("first claim");
        held_tx.send(()).expect("say held");
        let _ = release.recv();
        drop(guard);
    });
    held.recv().expect("the first holds it");
    let Claim::Running(running) = instance::claim(&name).expect("second claim") else {
        panic!("the second claim should find the first running");
    };
    let Err(running) = running.wait(Duration::from_millis(50)) else {
        panic!("taken while the first still holds it");
    };
    release_tx.send(()).expect("let go");
    first.join().expect("first thread");
    assert!(
        running.wait(Duration::from_secs(5)).is_ok(),
        "not taken over once the first let go"
    );
}

#[test]
fn asking_to_show_reaches_the_window_of_the_same_home() {
    let key = test_key("show");
    let shown = Arc::new(AtomicUsize::new(0));
    let count = shown.clone();
    endsession::listen(
        || {},
        Some(endsession::Show {
            message: instance::show_message(&key).expect("register the message"),
            on_show: Box::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                true
            }),
        }),
    )
    .expect("listen");

    assert!(instance::ask_to_show(&key), "no window answered");
    assert_eq!(shown.load(Ordering::SeqCst), 1);
}

#[test]
fn asking_to_show_passes_over_the_window_of_another_home() {
    let shown = Arc::new(AtomicUsize::new(0));
    let count = shown.clone();
    endsession::listen(
        || {},
        Some(endsession::Show {
            message: instance::show_message(&test_key("mine")).expect("register the message"),
            on_show: Box::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                true
            }),
        }),
    )
    .expect("listen");

    assert!(!instance::ask_to_show(&test_key("theirs")));
    assert_eq!(shown.load(Ordering::SeqCst), 0);
}

#[test]
fn a_closing_instance_does_not_count_as_shown() {
    let key = test_key("closing");
    endsession::listen(
        || {},
        Some(endsession::Show {
            message: instance::show_message(&key).expect("register the message"),
            on_show: Box::new(|| false),
        }),
    )
    .expect("listen");

    assert!(!instance::ask_to_show(&key));
}
