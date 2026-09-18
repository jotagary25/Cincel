//! The open project, as a GPUI entity.
//!
//! `asteroid-project` has no idea what an executor is: it starts things and
//! hands back channels (`docs/specs/modulos/project.md`). This module is where
//! those channels are drained on GPUI's threads
//! (`docs/specs/03-arquitectura.md` §3):
//!
//! | Source | Producer | Consumer |
//! |---|---|---|
//! | [`WorktreeScan`] | the `ignore` walker's own thread pool | a foreground task, in batches, feeding the tree incrementally |
//! | [`Watcher`] | `notify`'s debouncer thread | a foreground task: worktree, then [`BufferStore::handle_fs_events`], then a git refresh |
//! | [`GitStatusWatcher`] | its own thread running `git status` | a foreground task, recoloring the tree |
//!
//! The tree, the buffers and the git status all live on the main thread
//! because the UI reads them every frame; everything expensive happens before
//! the value reaches the channel.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use asteroid_project::{
    BufferChange, BufferHandle, BufferStore, FsEvent, GitFileStatus, GitStatus, GitStatusWatcher,
    OpenError, Recents, WatchOptions, Watcher, Worktree, WorktreeConfig,
};
use asteroid_settings::Settings;
use asteroid_syntax::{Language, LanguageRegistry};
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Task};

/// How long the scan drain waits between batches, so a large project paints
/// progressively instead of rebuilding the tree hundreds of times a second.
const SCAN_BATCH_INTERVAL: Duration = Duration::from_millis(30);

/// Which background pieces a project starts.
///
/// Everything is on in the real application. The tests turn it all off: GPUI's
/// test scheduler panics when a foreign thread wakes a foreground task ("your
/// test is not deterministic"), and the walker, the file watcher and the git
/// status all run on threads of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectOptions {
    /// Whether the directory walk continues in the background (otherwise the
    /// whole tree is listed before `open` returns).
    pub background_scan: bool,
    /// Whether the file system watcher runs.
    pub watch_files: bool,
    /// Whether `git status` is polled.
    pub watch_git: bool,
}

impl Default for ProjectOptions {
    fn default() -> Self {
        Self {
            background_scan: true,
            watch_files: true,
            watch_git: true,
        }
    }
}

impl ProjectOptions {
    /// A project with no threads at all, for tests and tooling.
    pub fn inert() -> Self {
        Self {
            background_scan: false,
            watch_files: false,
            watch_git: false,
        }
    }
}

/// What changed in the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectEvent {
    /// Entries were added or removed: the tree has to be rebuilt.
    TreeChanged,
    /// The git status changed: the tree only has to repaint.
    GitChanged,
    /// A buffer was opened, saved, reloaded or closed.
    BuffersChanged(Vec<BufferChange>),
}

/// The open project: its file tree, its open buffers and its git status.
pub struct Project {
    root: PathBuf,
    worktree: Worktree,
    buffers: BufferStore,
    git: GitStatus,
    languages: Arc<LanguageRegistry>,
    git_watcher: Option<GitStatusWatcher>,
    /// Kept alive: dropping it stops the watch.
    _watcher: Option<Watcher>,
    /// Kept alive: dropping them stops the drains.
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<ProjectEvent> for Project {}

impl Project {
    /// Starts the background scan, the file watcher and the git status watcher
    /// around a worktree whose root has already been listed.
    ///
    /// [`open`] is the way in: listing the root is the only step that can
    /// fail, and it happens before the entity exists so the tasks below can
    /// hold a weak handle to it.
    fn new(
        mut worktree: Worktree,
        scan: asteroid_project::WorktreeScan,
        settings: &Settings,
        options: ProjectOptions,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut tasks = Vec::new();

        if options.background_scan {
            // The walk is drained on the background executor, which coalesces
            // it into batches (`docs/specs/03-arquitectura.md` §3: directory
            // walks belong there), and the main thread only applies the
            // batches: a 50 000 file project must not rebuild the tree once
            // per 256 entries.
            let scan = scan.into_receiver();
            let (batches, ready) = async_channel::unbounded::<Vec<asteroid_project::ScanEvent>>();
            let executor = cx.background_executor().clone();
            let batching = executor.clone();
            executor
                .spawn(async move {
                    loop {
                        let Ok(event) = scan.recv().await else {
                            break;
                        };
                        let mut batch = vec![event];
                        // Let the walker fill the channel for a moment, then
                        // take everything it produced in one go.
                        batching.timer(SCAN_BATCH_INTERVAL).await;
                        while let Ok(event) = scan.try_recv() {
                            batch.push(event);
                        }
                        if batches.send(batch).await.is_err() {
                            break;
                        }
                    }
                })
                .detach();

            tasks.push(cx.spawn(async move |this, cx| {
                while let Ok(batch) = ready.recv().await {
                    let applied = this.update(cx, |project, cx| {
                        for event in batch {
                            project.worktree.apply(event);
                        }
                        cx.emit(ProjectEvent::TreeChanged);
                        cx.notify();
                    });
                    if applied.is_err() {
                        break;
                    }
                }
            }));
        } else {
            // Deterministic path: finish the walk here, before anyone looks.
            for event in scan {
                worktree.apply(event);
            }
        }

        let root = worktree.root().to_path_buf();
        tracing::info!(path = %root.display(), entries = worktree.len(), "proyecto abierto");

        // The file watcher.
        let watch_options = WatchOptions {
            excludes: settings.files.exclude.clone(),
            ..WatchOptions::default()
        };
        let watcher = match (options.watch_files, Watcher::new(&root, watch_options)) {
            (true, Ok((watcher, events))) => {
                tasks.push(cx.spawn(async move |this, cx| {
                    while let Ok(batch) = events.recv().await {
                        let applied =
                            this.update(cx, |project, cx| project.apply_fs_events(&batch, cx));
                        if applied.is_err() {
                            break;
                        }
                    }
                }));
                Some(watcher)
            }
            (true, Err(error)) => {
                tracing::warn!(%error, "el proyecto se abre sin vigilancia de archivos");
                None
            }
            (false, _) => None,
        };

        // The git status.
        let git_watcher = if options.watch_git {
            let (git_watcher, git_updates) = GitStatusWatcher::spawn(&root);
            tasks.push(cx.spawn(async move |this, cx| {
                while let Ok(status) = git_updates.recv().await {
                    let applied = this.update(cx, |project, cx| {
                        tracing::debug!(files = status.len(), "estado de git actualizado");
                        project.git = status;
                        cx.emit(ProjectEvent::GitChanged);
                        cx.notify();
                    });
                    if applied.is_err() {
                        break;
                    }
                }
            }));
            Some(git_watcher)
        } else {
            None
        };

        let mut recents = Recents::load();
        recents.push(&root);
        if let Err(error) = recents.save() {
            tracing::warn!(%error, "no se pudo guardar la lista de proyectos recientes");
        }

        Self {
            root,
            worktree,
            buffers: BufferStore::new(),
            git: GitStatus::default(),
            languages: Arc::new(LanguageRegistry::new()),
            git_watcher,
            _watcher: watcher,
            _tasks: tasks,
        }
    }

    /// The project root, absolute.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file tree.
    pub fn worktree(&self) -> &Worktree {
        &self.worktree
    }

    /// The open buffers.
    pub fn buffers(&self) -> &BufferStore {
        &self.buffers
    }

    /// The open buffers, mutably.
    pub fn buffers_mut(&mut self) -> &mut BufferStore {
        &mut self.buffers
    }

    /// The git status of the project.
    pub fn git(&self) -> &GitStatus {
        &self.git
    }

    /// The language registry, shared with whoever highlights text.
    pub fn languages(&self) -> &Arc<LanguageRegistry> {
        &self.languages
    }

    /// The language of `path`, by file name and extension.
    pub fn language_for(&self, path: &Path) -> Option<Arc<Language>> {
        self.languages.language_for_path(path)
    }

    /// The git status of an absolute path.
    pub fn git_status(&self, path: &Path) -> Option<GitFileStatus> {
        self.git.get(path)
    }

    /// Whether any file below `path` (absolute) has changes, which is what a
    /// collapsed folder shows as a dot (`docs/specs/02-visual.md` §6.4).
    pub fn has_changes_under(&self, path: &Path) -> bool {
        self.git.contains_changes_under(path)
    }

    /// The absolute path of a tree entry.
    pub fn absolute(&self, relative: &Path) -> PathBuf {
        if relative.is_absolute() {
            relative.to_path_buf()
        } else {
            self.root.join(relative)
        }
    }

    /// The path relative to the project root, for the breadcrumb and the tabs.
    pub fn relative(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf())
    }

    /// Opens (or returns the already open) buffer for an absolute path.
    pub fn open_buffer(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Result<BufferHandle, OpenError> {
        let handle = self.buffers.open(path)?;
        cx.emit(ProjectEvent::BuffersChanged(Vec::new()));
        Ok(handle)
    }

    /// Whether the buffer at `path` has unsaved changes.
    pub fn is_dirty(&self, path: &Path) -> bool {
        self.buffers.is_dirty(path)
    }

    /// Saves the buffer at `path`.
    pub fn save(&mut self, path: &Path, cx: &mut Context<Self>) -> Result<(), String> {
        self.buffers
            .save(path)
            .map_err(|error| error.to_string())
            .inspect(|()| {
                cx.emit(ProjectEvent::BuffersChanged(Vec::new()));
            })
    }

    /// Drops the buffer at `path`, discarding unsaved changes.
    pub fn close_buffer(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.buffers.close(path) {
            cx.emit(ProjectEvent::BuffersChanged(Vec::new()));
        }
    }

    /// Asks for a git status refresh (after a save, for instance).
    pub fn refresh_git(&self) {
        if let Some(watcher) = &self.git_watcher {
            watcher.trigger();
        }
    }

    /// Applies one batch of file system events to the tree and the buffers.
    fn apply_fs_events(&mut self, events: &[FsEvent], cx: &mut Context<Self>) {
        let mut tree_changed = false;
        for event in events {
            tree_changed |= self.worktree.apply_fs_event(event);
        }
        let buffer_changes = self.buffers.handle_fs_events(events);
        if !buffer_changes.is_empty() {
            tracing::debug!(changes = buffer_changes.len(), "buffers reconciliados");
            cx.emit(ProjectEvent::BuffersChanged(buffer_changes));
        }
        if tree_changed {
            cx.emit(ProjectEvent::TreeChanged);
        }
        self.refresh_git();
        cx.notify();
    }
}

/// Opens `root` as a project entity.
///
/// Listing the root directory is synchronous — the window cannot paint a tree
/// without it — and is the only step that can fail; everything else is started
/// in the background by [`Project::new`].
pub fn open(
    root: impl AsRef<Path>,
    settings: &Settings,
    cx: &mut App,
) -> std::io::Result<Entity<Project>> {
    open_with(root, settings, ProjectOptions::default(), cx)
}

/// Opens `root` with an explicit set of background pieces.
pub fn open_with(
    root: impl AsRef<Path>,
    settings: &Settings,
    options: ProjectOptions,
    cx: &mut App,
) -> std::io::Result<Entity<Project>> {
    let root = std::path::absolute(root.as_ref())?;
    let config = WorktreeConfig::with_excludes(settings.files.exclude.clone());
    let (worktree, scan) = Worktree::scan(&root, config)?;
    for issue in worktree.issues() {
        tracing::warn!(%issue, "problema al preparar el recorrido del proyecto");
    }
    Ok(cx.new(|cx| Project::new(worktree, scan, settings, options, cx)))
}
