//! Tests of the batching debouncer: batches, rename pairing and, above all,
//! that nothing wakes up at rest (`docs/rendimiento.md`, E6-G, M8).

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use notify::event::{CreateKind, DataChange, EventKind, ModifyKind, RenameMode};
use notify::{Event, RecursiveMode};

use super::{Debouncer, coalesce};

const WINDOW: Duration = Duration::from_millis(50);

fn start(root: &std::path::Path) -> (Debouncer, mpsc::Receiver<Vec<Event>>) {
    let (sender, receiver) = mpsc::channel();
    let mut debouncer = Debouncer::new("cincel-watch-test", WINDOW, move |batch| {
        let _ = sender.send(batch.events);
    })
    .unwrap();
    debouncer.watch(root, RecursiveMode::Recursive).unwrap();
    (debouncer, receiver)
}

fn event(kind: EventKind, paths: &[&str]) -> Event {
    let mut event = Event::new(kind);
    for path in paths {
        event = event.add_path(PathBuf::from(path));
    }
    event
}

/// The regression test of M8: with nothing happening on disk, the batching
/// thread stays blocked (the debouncer it replaced woke up every 25 ms).
#[test]
fn nothing_wakes_up_at_rest() {
    let dir = tempfile::tempdir().unwrap();
    let (debouncer, events) = start(dir.path());
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(debouncer.wakeups(), 0, "the thread woke up with no events");

    std::fs::write(dir.path().join("a.txt"), "hola").unwrap();
    let batch = events.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!batch.is_empty());
    let after_burst = debouncer.wakeups();
    assert!(after_burst >= 1);

    // At rest again: the count stays where the burst left it.
    std::thread::sleep(Duration::from_millis(400));
    while events.try_recv().is_ok() {}
    let settled = debouncer.wakeups();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(debouncer.wakeups(), settled, "the thread woke up at rest");
}

#[test]
fn a_burst_arrives_as_one_batch() {
    let dir = tempfile::tempdir().unwrap();
    let (_debouncer, events) = start(dir.path());
    std::thread::sleep(Duration::from_millis(100));
    for index in 0..20 {
        std::fs::write(dir.path().join(format!("f{index}.txt")), "x").unwrap();
    }
    let first = events.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut paths: Vec<PathBuf> = first.iter().flat_map(|e| e.paths.clone()).collect();
    // A slow machine may split the burst in two windows, never in twenty.
    if let Ok(second) = events.recv_timeout(WINDOW * 4) {
        paths.extend(second.iter().flat_map(|e| e.paths.clone()));
    }
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), 20, "{paths:?}");
}

#[test]
fn a_paired_rename_drops_its_halves() {
    let both = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
        &["/p/a.rs", "/p/b.rs"],
    )
    .set_tracker(7);
    let from = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::From)),
        &["/p/a.rs"],
    )
    .set_tracker(7);
    let to = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::To)),
        &["/p/b.rs"],
    )
    .set_tracker(7);
    let other = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::From)),
        &["/p/c.rs"],
    )
    .set_tracker(8);
    let batch = coalesce(vec![Ok(from), Ok(to), Ok(both.clone()), Ok(other.clone())]);
    assert_eq!(batch.events, vec![both, other]);
}

#[test]
fn repeated_events_collapse() {
    let modify = event(
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        &["/p/a.rs"],
    );
    let create = event(EventKind::Create(CreateKind::File), &["/p/a.rs"]);
    let batch = coalesce(vec![
        Ok(create.clone()),
        Ok(modify.clone()),
        Ok(modify.clone()),
        Ok(modify.clone()),
    ]);
    assert_eq!(batch.events, vec![create, modify]);
}

#[test]
fn a_real_rename_is_reported_as_a_pair() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("viejo.rs");
    std::fs::write(&from, "hola").unwrap();
    let (_debouncer, events) = start(dir.path());
    std::thread::sleep(Duration::from_millis(100));
    let to = dir.path().join("nuevo.rs");
    std::fs::rename(&from, &to).unwrap();
    let batch = events.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        batch.iter().any(|event| {
            event.kind == EventKind::Modify(ModifyKind::Name(RenameMode::Both))
                && event.paths == vec![from.clone(), to.clone()]
        }),
        "{batch:?}"
    );
    assert!(
        !batch
            .iter()
            .any(|event| event.kind == EventKind::Modify(ModifyKind::Name(RenameMode::From))),
        "the lone half stayed: {batch:?}"
    );
}
