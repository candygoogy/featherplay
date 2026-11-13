//! Tasks nobody waits for, on tokio, and the UI's saves, in order.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::sync::mpsc::{self, SendError, Sender};

/// `tokio::spawn` for a task nobody waits for, on the current runtime. Boxed, so tokio's
/// task code is compiled once for all of them rather than once for each (~2 KB apiece).
pub fn spawn_detached(task: impl Future<Output = ()> + Send + 'static) {
    tokio::spawn(Box::pin(task) as Pin<Box<dyn Future<Output = ()> + Send>>);
}

/// `spawn_blocking` on `rt` for work nobody waits for, boxed as `spawn_detached` is.
pub fn blocking_detached(rt: &tokio::runtime::Handle, work: impl FnOnce() + Send + 'static) {
    rt.spawn_blocking(Box::new(work) as Box<dyn FnOnce() + Send>);
}

type Write = Box<dyn FnOnce() + Send>;

/// Runs `write`, a save nobody waits for, off the calling thread and after every save given
/// before it: on one thread kept for them, started by the first. Two quick saves of one file
/// (a copy of the state each) land in the order they were made, so an older copy never
/// overwrites a newer one, as it could from two threads. When that thread can't start, or
/// has stopped (a save panicked), the save runs here.
pub fn write_in_order(write: impl FnOnce() + Send + 'static) {
    static WRITER: OnceLock<Option<Sender<Write>>> = OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Write>();
        std::thread::Builder::new()
            .name("featherplay-writes".into())
            .spawn(move || rx.into_iter().for_each(|write| write()))
            .ok()
            .map(|_| tx)
    });
    let write: Write = Box::new(write);
    match writer {
        Some(tx) => {
            if let Err(SendError(write)) = tx.send(write) {
                write();
            }
        }
        None => write(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, mpsc};

    use super::write_in_order;

    /// Saves given one after another run in that order, off this thread: the last one given
    /// is the last to run.
    #[test]
    fn saves_run_in_the_order_given() {
        let done = Arc::new(Mutex::new(Vec::new()));
        for i in 0..200 {
            let done = done.clone();
            write_in_order(move || done.lock().unwrap().push(i));
        }
        let (tx, rx) = mpsc::channel();
        write_in_order(move || tx.send(()).unwrap());
        rx.recv().unwrap();
        assert_eq!(*done.lock().unwrap(), (0..200).collect::<Vec<_>>());
    }
}
