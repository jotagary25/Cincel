//! The file system watcher.
//!
//! `notify` 8 batched by `cincel-watch` (no wakeups at rest; it replaced
//! `notify-debouncer-full`, whose thread woke up every 25 ms forever), 100 ms
//! windows, one recursive
//! watch on the project root — **directories, never files**, which is both
//! what the spec asks for and the only way to survive an atomic save: an
//! editor that writes `main.rs.tmp` and renames it over `main.rs` destroys
//! the inode a per-file watch was holding.
//!
//! inotify pairs the two halves of a rename (`RenameMode::Both`, which
//! `cincel-watch` keeps instead of the halves), and that pair becomes a
//! single [`FsEvent::Renamed`]; when a backend only reports one half, the event
//! degrades to a [`FsEvent::Removed`] plus a [`FsEvent::Created`], which the
//! `Worktree` and the `BufferStore` both handle.
//!
//! Events arrive in **batches**, one per debounce window, because that is the
//! unit the git status refresh and the tree repaint want.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use cincel_watch::Debouncer;
use notify::RecursiveMode;
use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode};

use crate::ignore_rules::{ExcludeSet, IgnoreRules};

/// Debounce window of the project watcher (`modulos/project.md`).
pub const WATCH_DEBOUNCE: Duration = Duration::from_millis(100);

/// Something that happened to a file or directory of the project.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FsEvent {
    /// A path appeared.
    Created(PathBuf),
    /// A file's contents or metadata changed.
    Modified(PathBuf),
    /// A path is gone.
    Removed(PathBuf),
    /// A path moved. Either side may be outside the project, in which case
    /// the event still carries both paths and the consumer ignores the half
    /// it does not know.
    Renamed {
        /// Where it was.
        from: PathBuf,
        /// Where it is now.
        to: PathBuf,
    },
}

impl FsEvent {
    /// The path the event is about (the destination, for a rename).
    pub fn path(&self) -> &Path {
        match self {
            FsEvent::Created(path) | FsEvent::Modified(path) | FsEvent::Removed(path) => path,
            FsEvent::Renamed { to, .. } => to,
        }
    }
}

/// How to watch a project.
#[derive(Clone, Debug)]
pub struct WatchOptions {
    /// Debounce window; events inside it are coalesced.
    pub debounce: Duration,
    /// `files.exclude` globs: events under them are dropped.
    pub excludes: Vec<String>,
    /// Whether `.gitignore`d paths are dropped as well.
    pub respect_gitignore: bool,
    /// Whether dot files produce events.
    pub show_hidden: bool,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            debounce: WATCH_DEBOUNCE,
            excludes: vec![
                "**/.git".to_owned(),
                "**/target".to_owned(),
                "**/node_modules".to_owned(),
            ],
            respect_gitignore: true,
            show_hidden: true,
        }
    }
}

/// Watching the project failed.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    /// The root could not be resolved.
    #[error("no se pudo resolver «{path}»: {source}")]
    Root {
        /// The root as given.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// `notify` refused to watch.
    #[error("no se pudo observar el proyecto: {0}")]
    Notify(#[from] notify::Error),
}

/// A recursive watch over the project root.
///
/// The watch lives as long as this value: drop it to stop.
pub struct Watcher {
    root: PathBuf,
    rules: Arc<IgnoreRules>,
    _debouncer: Debouncer,
}

impl Watcher {
    /// Starts watching `root`.
    ///
    /// The receiver yields one `Vec<FsEvent>` per debounce window, in the
    /// order the events happened.
    pub fn new(
        root: impl Into<PathBuf>,
        options: WatchOptions,
    ) -> Result<(Watcher, Receiver<Vec<FsEvent>>), WatchError> {
        let root = root.into();
        let root = std::path::absolute(&root).map_err(|source| WatchError::Root {
            path: root.clone(),
            source,
        })?;
        let (excludes, rejected) = ExcludeSet::new(&options.excludes);
        for issue in rejected {
            tracing::warn!(%issue, "patrón de «files.exclude» ignorado");
        }
        let rules = Arc::new(IgnoreRules::new(
            root.clone(),
            excludes,
            options.respect_gitignore,
            options.show_hidden,
        ));

        let (sender, receiver) = async_channel::unbounded();
        let filter = rules.clone();
        let mut debouncer = Debouncer::new(
            "cincel-watch-project",
            options.debounce,
            move |batch: cincel_watch::Batch| {
                for error in batch.errors {
                    tracing::warn!(%error, "error observando el proyecto");
                }
                let batch: Vec<FsEvent> = batch
                    .events
                    .iter()
                    .filter_map(translate)
                    .filter(|event| keep(&filter, event))
                    .collect();
                if !batch.is_empty() {
                    send(&sender, batch);
                }
            },
        )?;
        debouncer.watch(&root, RecursiveMode::Recursive)?;

        Ok((
            Watcher {
                root,
                rules,
                _debouncer: debouncer,
            },
            receiver,
        ))
    }

    /// The watched root, absolute.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The rules used to drop uninteresting events.
    pub fn rules(&self) -> &Arc<IgnoreRules> {
        &self.rules
    }
}

fn send(sender: &Sender<Vec<FsEvent>>, batch: Vec<FsEvent>) {
    if sender.try_send(batch).is_err() {
        tracing::debug!("nadie escucha los cambios del proyecto");
    }
}

/// Whether an event is about something the project cares about.
///
/// A removal cannot be `stat`ed any more, so its "is it a directory?" answer
/// is a guess; both answers are tried and the event survives if either says
/// the path is visible. Renames survive when *either* side is visible, so a
/// file moved out of `target/` still shows up.
fn keep(rules: &IgnoreRules, event: &FsEvent) -> bool {
    let visible = |path: &Path| {
        let is_dir = path.is_dir();
        !rules.is_ignored(path, is_dir) || (!is_dir && !rules.is_ignored(path, true))
    };
    match event {
        FsEvent::Renamed { from, to } => visible(from) || visible(to),
        other => visible(other.path()),
    }
}

/// Maps a `notify` event onto the four project events.
fn translate(event: &notify::Event) -> Option<FsEvent> {
    let first = event.paths.first()?.clone();
    match event.kind {
        EventKind::Create(
            CreateKind::Any | CreateKind::File | CreateKind::Folder | CreateKind::Other,
        ) => Some(FsEvent::Created(first)),
        EventKind::Remove(
            RemoveKind::Any | RemoveKind::File | RemoveKind::Folder | RemoveKind::Other,
        ) => Some(FsEvent::Removed(first)),
        EventKind::Modify(ModifyKind::Name(mode)) => match mode {
            // inotify pairs both halves by their cookie.
            RenameMode::Both => {
                let to = event.paths.get(1).cloned()?;
                Some(FsEvent::Renamed { from: first, to })
            }
            RenameMode::From => Some(FsEvent::Removed(first)),
            RenameMode::To => Some(FsEvent::Created(first)),
            RenameMode::Any | RenameMode::Other => {
                // An unpaired rename: whichever side exists is the truth.
                if first.exists() {
                    Some(FsEvent::Created(first))
                } else {
                    Some(FsEvent::Removed(first))
                }
            }
        },
        EventKind::Modify(_) => Some(FsEvent::Modified(first)),
        // `Access` (an editor merely reading the file) and `Any`/`Other` are
        // not worth a repaint.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// Collects events until `predicate` is happy or `timeout` elapses.
    /// Returns how long it took.
    fn wait_for(
        receiver: &Receiver<Vec<FsEvent>>,
        timeout: Duration,
        mut predicate: impl FnMut(&FsEvent) -> bool,
    ) -> Option<Duration> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            match receiver.try_recv() {
                Ok(batch) => {
                    if batch.iter().any(&mut predicate) {
                        return Some(started.elapsed());
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        None
    }

    fn options() -> WatchOptions {
        WatchOptions {
            debounce: Duration::from_millis(50),
            ..WatchOptions::default()
        }
    }

    /// `modulos/project.md`: an external create, delete or rename shows up in
    /// under 300 ms.
    #[test]
    fn external_changes_arrive_quickly() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::write(root.join("viejo.rs"), "hola").unwrap();
        let (_watcher, events) = Watcher::new(&root, options()).unwrap();
        // Give the backend a moment to arm the inotify watch.
        std::thread::sleep(Duration::from_millis(100));

        let created = root.join("nuevo.rs");
        std::fs::write(&created, "hola").unwrap();
        let elapsed = wait_for(&events, Duration::from_secs(5), |event| {
            matches!(event, FsEvent::Created(path) | FsEvent::Modified(path) if path == &created)
        })
        .expect("no llegó el alta");
        assert!(elapsed < Duration::from_millis(300), "tardó {elapsed:?}");

        let renamed = root.join("renombrado.rs");
        std::fs::rename(&created, &renamed).unwrap();
        let elapsed = wait_for(&events, Duration::from_secs(5), |event| match event {
            FsEvent::Renamed { to, .. } => to == &renamed,
            FsEvent::Created(path) => path == &renamed,
            _ => false,
        })
        .expect("no llegó el renombrado");
        assert!(elapsed < Duration::from_millis(300), "tardó {elapsed:?}");

        std::fs::remove_file(&renamed).unwrap();
        let elapsed = wait_for(
            &events,
            Duration::from_secs(5),
            |event| matches!(event, FsEvent::Removed(path) if path == &renamed),
        )
        .expect("no llegó la baja");
        assert!(elapsed < Duration::from_millis(300), "tardó {elapsed:?}");
    }

    #[test]
    fn an_atomic_save_lands_on_the_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let file = root.join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let (_watcher, events) = Watcher::new(&root, options()).unwrap();
        std::thread::sleep(Duration::from_millis(100));

        let temp = root.join("main.rs.tmp");
        std::fs::write(&temp, "fn main() { let x = 1; }\n").unwrap();
        std::fs::rename(&temp, &file).unwrap();

        assert!(
            wait_for(&events, Duration::from_secs(5), |event| match event {
                FsEvent::Renamed { to, .. } => to == &file,
                FsEvent::Created(path) | FsEvent::Modified(path) => path == &file,
                _ => false,
            })
            .is_some(),
            "el guardado atómico no se vio en el archivo real"
        );
    }

    #[test]
    fn excluded_paths_produce_no_events() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        let (_watcher, events) = Watcher::new(&root, options()).unwrap();
        std::thread::sleep(Duration::from_millis(100));

        std::fs::write(root.join("target/debug/x.o"), "basura").unwrap();
        let visible = root.join("visible.rs");
        std::fs::write(&visible, "hola").unwrap();

        // The visible file arrives; nothing under `target` ever does.
        assert!(
            wait_for(&events, Duration::from_secs(5), |event| event.path()
                == visible)
            .is_some()
        );
        while let Ok(batch) = events.try_recv() {
            for event in batch {
                assert!(
                    !event.path().starts_with(root.join("target")),
                    "llegó un evento de target: {event:?}"
                );
            }
        }
    }

    #[test]
    fn translates_the_notify_kinds() {
        use notify::event::Event;
        let path = PathBuf::from("/p/a.rs");
        let other = PathBuf::from("/p/b.rs");

        let event = Event::new(EventKind::Create(CreateKind::File)).add_path(path.clone());
        assert_eq!(translate(&event), Some(FsEvent::Created(path.clone())));

        let event = Event::new(EventKind::Remove(RemoveKind::File)).add_path(path.clone());
        assert_eq!(translate(&event), Some(FsEvent::Removed(path.clone())));

        let event = Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content,
        )))
        .add_path(path.clone());
        assert_eq!(translate(&event), Some(FsEvent::Modified(path.clone())));

        let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(path.clone())
            .add_path(other.clone());
        assert_eq!(
            translate(&event),
            Some(FsEvent::Renamed {
                from: path.clone(),
                to: other,
            })
        );

        let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From)))
            .add_path(path.clone());
        assert_eq!(translate(&event), Some(FsEvent::Removed(path.clone())));

        let event =
            Event::new(EventKind::Access(notify::event::AccessKind::Read)).add_path(path.clone());
        assert_eq!(translate(&event), None);
    }
}
