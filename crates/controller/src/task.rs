//! Tasks nobody waits for, on tokio.

use std::future::Future;
use std::pin::Pin;

/// `tokio::spawn` for a task nobody waits for, on the current runtime. Boxed, so tokio's
/// task code is compiled once for all of them rather than once for each (~2 KB apiece).
pub fn spawn_detached(task: impl Future<Output = ()> + Send + 'static) {
    tokio::spawn(Box::pin(task) as Pin<Box<dyn Future<Output = ()> + Send>>);
}

/// `spawn_blocking` on `rt` for work nobody waits for, boxed as `spawn_detached` is.
pub fn blocking_detached(rt: &tokio::runtime::Handle, work: impl FnOnce() + Send + 'static) {
    rt.spawn_blocking(Box::new(work) as Box<dyn FnOnce() + Send>);
}
