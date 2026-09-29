//! File system watching in batches, with no wakeups at rest.
//!
//! `notify` reports every event the kernel produces; the editor wants them in
//! batches (one tree repaint, one `git status`, one review pass per burst).
//! `notify-debouncer-full`, which Cincel used before, batches them with a
//! thread that wakes up every quarter of the window **forever**: with three
//! watchers (the project, its `.git` and the configuration) that was about 90
//! wakeups a second and three quarters of Cincel's CPU at rest
//! (`docs/rendimiento.md`, E6-G).
//!
//! [`Debouncer`] batches with a thread that blocks on its channel while
//! nothing happens. The first event of a burst opens a window of
//! `window`; everything that arrives inside it goes out together when the
//! window closes (the window does not slide, so a steady stream of events
//! still goes out every `window` instead of never). Inside a batch:
//!
//! * a rename the backend paired (`RenameMode::Both`, which inotify reports
//!   next to its two halves with the same tracker) drops the lone halves;
//! * an event identical to the one before it (same kind, same paths) is
//!   dropped.
//!
//! Errors of the backend go out in the same batch, in a list of their own.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::event::{EventKind, ModifyKind, RenameMode};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};

pub use notify;

/// What one window collected.
#[derive(Debug, Default)]
pub struct Batch {
    /// The events, in the order they happened.
    pub events: Vec<Event>,
    /// Errors of the backend.
    pub errors: Vec<notify::Error>,
}

/// A `notify` watcher whose events arrive in batches, one per window.
///
/// Dropping it stops the watch and, once the last window is delivered, the
/// batching thread.
pub struct Debouncer {
    // Dropped first: its callback owns the sender, so the thread sees the
    // channel close and ends.
    watcher: RecommendedWatcher,
    wakeups: Arc<AtomicUsize>,
}

impl Debouncer {
    /// Starts a watcher that calls `handler` with every non-empty batch, on
    /// a thread of its own named `name`.
    pub fn new<F>(name: &str, window: Duration, mut handler: F) -> notify::Result<Self>
    where
        F: FnMut(Batch) + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel::<notify::Result<Event>>();
        let wakeups = Arc::new(AtomicUsize::new(0));
        let counted = wakeups.clone();
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                // Blocks without a timeout while nothing happens.
                while let Ok(first) = receiver.recv() {
                    counted.fetch_add(1, Ordering::Relaxed);
                    let mut raw = vec![first];
                    let deadline = Instant::now() + window;
                    let mut open = true;
                    loop {
                        let now = Instant::now();
                        if now >= deadline {
                            break;
                        }
                        match receiver.recv_timeout(deadline - now) {
                            Ok(next) => raw.push(next),
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => {
                                open = false;
                                break;
                            }
                        }
                    }
                    let batch = coalesce(raw);
                    if !batch.events.is_empty() || !batch.errors.is_empty() {
                        handler(batch);
                    }
                    if !open {
                        break;
                    }
                }
            })
            .map_err(notify::Error::io)?;
        let watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            // The thread is gone only when the debouncer is being dropped.
            let _ = sender.send(event);
        })?;
        Ok(Self { watcher, wakeups })
    }

    /// Watches `path`.
    pub fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        self.watcher.watch(path, mode)
    }

    /// Stops watching `path`.
    pub fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        self.watcher.unwatch(path)
    }

    /// How many windows the batching thread has opened: it only wakes up
    /// for events, so at rest this does not move.
    pub fn wakeups(&self) -> usize {
        self.wakeups.load(Ordering::Relaxed)
    }
}

/// One window's raw events as a [`Batch`] (see the module docs).
fn coalesce(raw: Vec<notify::Result<Event>>) -> Batch {
    let mut batch = Batch::default();
    let mut events = Vec::with_capacity(raw.len());
    for item in raw {
        match item {
            Ok(event) => events.push(event),
            Err(error) => batch.errors.push(error),
        }
    }
    let paired: Vec<usize> = events
        .iter()
        .filter(|event| is_rename(event, RenameMode::Both))
        .filter_map(Event::tracker)
        .collect();
    for event in events {
        let lone_half = is_rename(&event, RenameMode::From) || is_rename(&event, RenameMode::To);
        if lone_half
            && event
                .tracker()
                .is_some_and(|tracker| paired.contains(&tracker))
        {
            continue;
        }
        let repeated = batch
            .events
            .last()
            .is_some_and(|last: &Event| last.kind == event.kind && last.paths == event.paths);
        if !repeated {
            batch.events.push(event);
        }
    }
    batch
}

fn is_rename(event: &Event, mode: RenameMode) -> bool {
    event.kind == EventKind::Modify(ModifyKind::Name(mode))
}

#[cfg(test)]
mod tests;
