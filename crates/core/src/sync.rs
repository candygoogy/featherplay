//! Locks that keep working after a thread panicked while holding them.
//!
//! A std lock is poisoned when its holder panics, and `lock().unwrap()` then panics in
//! every later caller, so one bug takes down the audio or controller thread for good. Use
//! these where the data stays valid at every point a holder could panic: each critical
//! section either writes whole values or finishes its update before anything that can
//! panic. The poison flag is ignored and the data used as it is.

use std::sync::{
    Condvar, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard,
};

/// [`Mutex::lock`] that takes over a poisoned lock instead of panicking.
pub trait LockExt<T: ?Sized> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> LockExt<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// [`RwLock::read`] and [`RwLock::write`] that take over a poisoned lock.
pub trait RwLockExt<T: ?Sized> {
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T>;
    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> RwLockExt<T> for RwLock<T> {
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(PoisonError::into_inner)
    }
}

/// [`Condvar::wait`] that takes over the mutex when it was poisoned meanwhile.
pub trait CondvarExt {
    fn wait_or_recover<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T>;
}

impl CondvarExt for Condvar {
    fn wait_or_recover<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        self.wait(guard).unwrap_or_else(PoisonError::into_inner)
    }
}
