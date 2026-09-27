//! End-to-end tests for the acceptance criteria of
//! `docs/specs/modulos/project.md`.
//!
//! The timing ones generate a synthetic tree, so the big one is `#[ignore]`d:
//! run it with
//!
//! ```text
//! cargo test -p cincel-project --test project -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cincel_project::{
    BufferStore, FsEvent, ReloadOutcome, ScanEvent, WatchOptions, Watcher, Worktree, WorktreeConfig,
};

/// Builds `dirs` directories with `files_per_dir` files each, plus a
/// `node_modules` of the same size that must never be walked.
fn synthetic_project(root: &Path, dirs: usize, files_per_dir: usize) -> usize {
    let mut created = 0;
    for dir in 0..dirs {
        let path = root.join(format!("crate{dir:03}/src"));
        std::fs::create_dir_all(&path).unwrap();
        for file in 0..files_per_dir {
            std::fs::write(path.join(format!("mod{file:04}.rs")), "// hola\n").unwrap();
            created += 1;
        }
    }
    // The noise the spec says must be ignored.
    for dir in 0..dirs {
        let path = root.join(format!("node_modules/paquete{dir:03}"));
        std::fs::create_dir_all(&path).unwrap();
        for file in 0..files_per_dir {
            std::fs::write(path.join(format!("index{file:04}.js")), "// no\n").unwrap();
        }
    }
    created
}

/// Runs a scan and returns (time to the root listing, time to the full tree,
/// number of entries).
fn measure(root: &Path) -> (Duration, Duration, usize) {
    let started = Instant::now();
    let (mut worktree, scan) = Worktree::scan(root, WorktreeConfig::default()).unwrap();
    let root_ready = started.elapsed();
    for event in scan {
        if let ScanEvent::Finished { .. } = &event {
            worktree.apply(event);
            break;
        }
        worktree.apply(event);
    }
    (started.elapsed(), root_ready, worktree.len())
}

#[test]
fn a_medium_project_scans_quickly() {
    let dir = tempfile::tempdir().unwrap();
    let files = synthetic_project(dir.path(), 25, 200); // 5 000 files + 5 000 ignored
    let (total, root_ready, entries) = measure(dir.path());
    println!("5k: raíz en {root_ready:?}, total {total:?}, {entries} entradas");
    assert!(
        root_ready < Duration::from_millis(500),
        "la raíz tardó {root_ready:?}"
    );
    assert!(entries >= files, "faltan entradas: {entries} < {files}");
    // `node_modules` was excluded, so the tree is not twice the size.
    assert!(entries < files * 2, "se recorrió node_modules: {entries}");
}

/// `modulos/project.md`: opening a project with 50 000 files (with
/// `node_modules` ignored) takes under 500 ms to have the root tree, and
/// finishes in the background.
#[test]
#[ignore = "crea 100 000 archivos; se corre a mano"]
fn a_50k_file_project_opens_in_under_500ms() {
    let dir = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let files = synthetic_project(dir.path(), 100, 500); // 50 000 + 50 000 ignored
    println!("generar {files} archivos tardó {:?}", started.elapsed());

    let (total, root_ready, entries) = measure(dir.path());
    println!("50k: raíz en {root_ready:?}, total {total:?}, {entries} entradas");
    assert!(
        root_ready < Duration::from_millis(500),
        "la raíz tardó {root_ready:?}"
    );
    assert!(entries >= files, "faltan entradas: {entries} < {files}");
}

/// Waits for `predicate` to hold over the incoming batches.
fn wait_for_batch(
    events: &async_channel::Receiver<Vec<FsEvent>>,
    timeout: Duration,
    mut predicate: impl FnMut(&FsEvent) -> bool,
) -> Option<(Duration, Vec<FsEvent>)> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        match events.try_recv() {
            Ok(batch) => {
                if batch.iter().any(&mut predicate) {
                    return Some((started.elapsed(), batch));
                }
            }
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    None
}

/// `modulos/project.md`: creating, deleting or renaming a file from another
/// application shows up in the tree in under 300 ms.
#[test]
fn external_changes_reach_the_tree_in_under_300ms() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

    let mut worktree = Worktree::scan_blocking(&root, WorktreeConfig::default()).unwrap();
    let (_watcher, events) = Watcher::new(
        &root,
        WatchOptions {
            debounce: Duration::from_millis(50),
            ..WatchOptions::default()
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));

    // Create.
    let created = root.join("src/nuevo.rs");
    std::fs::write(&created, "// hola\n").unwrap();
    let (elapsed, batch) = wait_for_batch(&events, Duration::from_secs(5), |event| {
        event.path() == created
    })
    .expect("no llegó el alta");
    for event in &batch {
        worktree.apply_fs_event(event);
    }
    println!("alta vista en {elapsed:?}");
    assert!(elapsed < Duration::from_millis(300), "tardó {elapsed:?}");
    assert!(worktree.find("src/nuevo.rs").is_some());

    // Rename.
    let renamed = root.join("src/renombrado.rs");
    std::fs::rename(&created, &renamed).unwrap();
    let (elapsed, batch) = wait_for_batch(&events, Duration::from_secs(5), |event| match event {
        FsEvent::Renamed { to, .. } => to == &renamed,
        FsEvent::Created(path) => path == &renamed,
        _ => false,
    })
    .expect("no llegó el renombrado");
    for event in &batch {
        worktree.apply_fs_event(event);
    }
    println!("renombrado visto en {elapsed:?}");
    assert!(elapsed < Duration::from_millis(300), "tardó {elapsed:?}");
    assert!(worktree.find("src/renombrado.rs").is_some());
    assert!(worktree.find("src/nuevo.rs").is_none());

    // Delete.
    std::fs::remove_file(&renamed).unwrap();
    let (elapsed, batch) = wait_for_batch(
        &events,
        Duration::from_secs(5),
        |event| matches!(event, FsEvent::Removed(path) if path == &renamed),
    )
    .expect("no llegó la baja");
    for event in &batch {
        worktree.apply_fs_event(event);
    }
    println!("baja vista en {elapsed:?}");
    assert!(elapsed < Duration::from_millis(300), "tardó {elapsed:?}");
    assert!(worktree.find("src/renombrado.rs").is_none());
}

/// `modulos/project.md`: saving from Cincel does not reload the buffer, and
/// an external write does (or raises a conflict when the buffer is dirty).
#[test]
fn saving_and_external_writes_through_the_real_watcher() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let file = root.join("main.rs");
    std::fs::write(&file, "fn main() {\n    uno();\n}\n").unwrap();

    let mut store = BufferStore::new();
    let buffer = store.open(&file).unwrap();
    let (_watcher, events) = Watcher::new(
        &root,
        WatchOptions {
            debounce: Duration::from_millis(50),
            ..WatchOptions::default()
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));

    // 1. Our own save must not reload the buffer.
    buffer.lock().replace(0..2, "FN");
    store.save(&file).unwrap();
    let (_, batch) = wait_for_batch(&events, Duration::from_secs(5), |event| {
        event.path() == file
    })
    .expect("no llegó el evento del guardado propio");
    let changes = store.handle_fs_events(&batch);
    assert!(
        changes.is_empty(),
        "el guardado propio recargó: {changes:?}"
    );
    assert!(buffer.lock().text().starts_with("FN main"));
    assert!(!store.is_dirty(&file));

    // 2. An external write on a clean buffer reloads it.
    std::fs::write(&file, "fn main() {\n    dos();\n}\n").unwrap();
    let (_, batch) = wait_for_batch(&events, Duration::from_secs(5), |event| {
        event.path() == file
    })
    .expect("no llegó el evento externo");
    let changes = store.handle_fs_events(&batch);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].outcome, ReloadOutcome::Reloaded);
    assert!(buffer.lock().text().contains("dos()"));

    // 3. An external write on a dirty buffer is a conflict.
    buffer.lock().replace(0..2, "FN");
    std::fs::write(&file, "otra cosa entera\n").unwrap();
    let (_, batch) = wait_for_batch(&events, Duration::from_secs(5), |event| {
        event.path() == file
    })
    .expect("no llegó el segundo evento externo");
    let changes = store.handle_fs_events(&batch);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].outcome, ReloadOutcome::Conflict);
    assert!(store.has_conflict(&file));
    assert!(buffer.lock().text().starts_with("FN main"));
}

/// The agent path: read, write, save, and no reload of our own bytes.
#[test]
fn an_agent_write_lands_as_minimal_edits_and_does_not_bounce_back() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let file = root.join("lib.rs");
    std::fs::write(&file, "fn suma(a: i32, b: i32) -> i32 {\n    a - b\n}\n").unwrap();

    let mut store = BufferStore::new();
    let buffer = store.open(&file).unwrap();
    let snapshot = store.read_for_agent(&file, None, None).unwrap();
    assert!(snapshot.contains("a - b"));
    let version = buffer.lock().version();

    let write = store
        .apply_agent_write(
            &file,
            "fn suma(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
            42,
        )
        .unwrap();
    assert_eq!(write.edits, 1);
    assert!(!write.rebased);
    assert_eq!(buffer.lock().version(), version + 1);
    assert!(std::fs::read_to_string(&file).unwrap().contains("a + b"));

    // The watcher event that write produced must not reload anything.
    let changes = store.handle_fs_event(&FsEvent::Modified(file.clone()));
    assert!(changes.is_empty(), "{changes:?}");
}

/// The tree and the store agree on what is open after a rename.
#[test]
fn a_renamed_open_file_is_reported_as_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let file = root.join("a.rs");
    std::fs::write(&file, "hola\n").unwrap();
    let mut worktree = Worktree::scan_blocking(&root, WorktreeConfig::default()).unwrap();
    let mut store = BufferStore::new();
    store.open(&file).unwrap();

    let moved = root.join("b.rs");
    std::fs::rename(&file, &moved).unwrap();
    let event = FsEvent::Renamed {
        from: file.clone(),
        to: moved.clone(),
    };
    worktree.apply_fs_event(&event);
    let changes = store.handle_fs_event(&event);

    assert!(worktree.find("b.rs").is_some());
    assert!(worktree.find("a.rs").is_none());
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, PathBuf::from(&file));
    assert_eq!(changes[0].outcome, ReloadOutcome::Deleted);
}
