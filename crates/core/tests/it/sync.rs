use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;

use featherplay_core::sync::{CondvarExt, LockExt, RwLockExt};

/// Panics in a thread while it holds `lock`, which poisons it.
fn poison_mutex<T: Send + 'static>(lock: &Arc<Mutex<T>>) {
    let l = lock.clone();
    let joined = thread::spawn(move || {
        let _held = l.lock_or_recover();
        panic!("poisoning the lock on purpose");
    })
    .join();
    assert!(joined.is_err(), "the thread should have panicked");
    assert!(lock.is_poisoned());
}

#[test]
fn lock_or_recover_returns_the_data_of_a_poisoned_mutex() {
    let lock = Arc::new(Mutex::new(7));
    poison_mutex(&lock);

    assert_eq!(*lock.lock_or_recover(), 7);
}

#[test]
fn lock_or_recover_keeps_writes_after_poisoning() {
    let lock = Arc::new(Mutex::new(Vec::new()));
    poison_mutex(&lock);

    lock.lock_or_recover().push(1);

    assert_eq!(*lock.lock_or_recover(), [1]);
}

#[test]
fn read_and_write_or_recover_survive_a_poisoned_rwlock() {
    let lock = Arc::new(RwLock::new(String::from("before")));
    let l = lock.clone();
    let joined = thread::spawn(move || {
        let _held = l.write_or_recover();
        panic!("poisoning the lock on purpose");
    })
    .join();
    assert!(joined.is_err());
    assert!(lock.is_poisoned());

    *lock.write_or_recover() = String::from("after");

    assert_eq!(*lock.read_or_recover(), "after");
}

#[test]
fn wait_or_recover_wakes_on_a_poisoned_mutex() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let p = pair.clone();
    let joined = thread::spawn(move || {
        let _held = p.0.lock_or_recover();
        panic!("poisoning the lock on purpose");
    })
    .join();
    assert!(joined.is_err());

    let (lock, cv) = &*pair;
    let mut ready = lock.lock_or_recover();
    let p = pair.clone();
    let notifier = thread::spawn(move || {
        *p.0.lock_or_recover() = true;
        p.1.notify_all();
    });
    while !*ready {
        ready = cv.wait_or_recover(ready);
    }
    drop(ready);
    notifier.join().expect("the notifier thread doesn't panic");

    assert!(*lock.lock_or_recover());
}
