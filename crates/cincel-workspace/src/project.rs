//! The open project, as a GPUI entity.
//!
//! `cincel-project` has no idea what an executor is: it starts things and
//! hands back channels (`docs/specs/modulos/project.md`). This module is where
//! those channels are drained on GPUI's threads
//! (`docs/specs/03-arquitectura.md` §3):
//!
//! | Source | Producer | Consumer |
//! |---|---|---|
//! | [`WorktreeScan`] | the `ignore` walker's own thread pool | a foreground task, in batches, feeding the tree incrementally |
//! | [`Watcher`] | `notify`'s debouncer thread | a foreground task: worktree, then [`BufferStore::handle_fs_events`], then [`FilesChanged`] for the review, then a git refresh |
//! | [`GitStatusWatcher`] | its own thread running `git status` | a foreground task, recoloring the tree |
//! | [`GitDirWatcher`] | `notify`'s debouncer thread, on `.git` | a foreground task: a status refresh and a new gutter diff of every open file |
//! | [`diff_against_head`] | the background executor, one task per file | a foreground task handing the hunks to the file's editors |
//!
//! The tree, the buffers and the git status all live on the main thread
//! because the UI reads them every frame; everything expensive happens before
//! the value reaches the channel.
//!
//! # The git gutter (`docs/specs/07-etapa5-productividad.md` §6)
//!
//! Every editor created over a buffer of this project's store is adopted
//! (`App::observe_new`), and its file gets a diff against `HEAD` on the
//! background executor: when the tab opens, when the file is saved (from
//! Cincel or on disk) and when the index or `HEAD` move. Each path carries a
//! version counter, so a diff that lands after a newer one was asked for is
//! dropped. A result is stored and announced with [`GitDiffChanged`], and the
//! project itself consumes that event by handing the hunks — and the theme's
//! `git.*` colours — to every editor of the file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cincel_editor::{EditorView, GitGutterColors, GitGutterHunk, GitGutterKind};
use cincel_project::{
    BufferChange, BufferHandle, BufferStore, FsEvent, GitDirWatcher, GitFileStatus, GitHunkKind,
    GitStatus, GitStatusWatcher, LineDiff, OpenError, Recents, WatchOptions, Watcher, Worktree,
    WorktreeConfig, diff_against_head,
};
use cincel_settings::Settings;
use cincel_syntax::{Language, LanguageRegistry};
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Subscription, Task, WeakEntity};

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
    /// Whether `git status` is polled and `.git` is watched.
    pub watch_git: bool,
    /// Whether the editors' git gutter is computed: one
    /// `git diff -U0 HEAD` per open file, on the background executor.
    pub git_gutter: bool,
}

impl Default for ProjectOptions {
    fn default() -> Self {
        Self {
            background_scan: true,
            watch_files: true,
            watch_git: true,
            git_gutter: true,
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
            git_gutter: false,
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

/// The diff against `HEAD` of `0` (absolute) was recomputed; read it with
/// [`Project::git_diff`].
///
/// A type of its own rather than a [`ProjectEvent`] variant, so the existing
/// exhaustive `match`es over `ProjectEvent` keep compiling; the project
/// consumes it itself to update the editors of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDiffChanged(pub PathBuf);

/// One batch of the file watcher, as it came: every created, modified,
/// removed or renamed path of the project, open or not. The review reads it
/// to follow what the agent changes by any means
/// (`docs/specs/03-arquitectura.md` §4, v2). A type of its own for the same
/// reason as [`GitDiffChanged`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesChanged(pub Vec<FsEvent>);

/// Cincel itself wrote (or deleted) this file: a save, a new file, a
/// reject. During an agent turn that change is not the agent's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostWrote(pub PathBuf);

/// The gutter diff of one path: the version of the last request and the last
/// result that landed.
#[derive(Debug, Default)]
struct GitDiffSlot {
    requested: u64,
    diff: Option<LineDiff>,
}

/// The open project: its file tree, its open buffers and its git status.
pub struct Project {
    root: PathBuf,
    worktree: Worktree,
    buffers: BufferStore,
    git: GitStatus,
    languages: Arc<LanguageRegistry>,
    git_watcher: Option<GitStatusWatcher>,
    /// Whether the gutter diffs are computed ([`ProjectOptions::git_gutter`]).
    git_gutter: bool,
    /// The background pieces this project was opened with.
    options: ProjectOptions,
    /// Gutter diff per open file (absolute path).
    git_diffs: HashMap<PathBuf, GitDiffSlot>,
    /// The editors over buffers of this project, by file.
    editors: Vec<(PathBuf, WeakEntity<EditorView>)>,
    /// Kept alive: dropping it stops the watch of `.git`.
    _git_dir_watcher: Option<GitDirWatcher>,
    /// Kept alive: dropping it stops the watch.
    _watcher: Option<Watcher>,
    /// Kept alive: dropping them stops the drains.
    _tasks: Vec<Task<()>>,
    /// Kept alive: the editor adoption, the theme observer and the
    /// [`GitDiffChanged`] consumer.
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ProjectEvent> for Project {}
impl EventEmitter<GitDiffChanged> for Project {}
impl EventEmitter<FilesChanged> for Project {}
impl EventEmitter<HostWrote> for Project {}

impl Project {
    /// Starts the background scan, the file watcher and the git status watcher
    /// around a worktree whose root has already been listed.
    ///
    /// [`open`] is the way in: listing the root is the only step that can
    /// fail, and it happens before the entity exists so the tasks below can
    /// hold a weak handle to it.
    fn new(
        mut worktree: Worktree,
        scan: cincel_project::WorktreeScan,
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
            let (batches, ready) = async_channel::unbounded::<Vec<cincel_project::ScanEvent>>();
            let executor = cx.background_executor().clone();
            let batching = executor.clone();
            // `CINCEL_TRACE_TIMINGS=1`: lives until the walk ends
            // (`crate::bench::TIMING_TARGET`).
            let first_scan = tracing::info_span!(target: "cincel::timing", "first_scan");
            executor
                .spawn(async move {
                    let _first_scan = first_scan;
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
            let _first_scan = tracing::info_span!(target: "cincel::timing", "first_scan").entered();
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

        // The `.git` directory: commits, `git add`, checkouts… done
        // elsewhere. Finding it runs git, so it happens off the main thread.
        if options.watch_git {
            let git_root = root.clone();
            let find = cx
                .background_executor()
                .spawn(async move { cincel_project::git_dir(&git_root) });
            tasks.push(cx.spawn(async move |this, cx| {
                let Some(git_dir) = find.await else {
                    return;
                };
                let events = match GitDirWatcher::new(&git_dir) {
                    Ok((watcher, events)) => {
                        let stored = this.update(cx, |project, _| {
                            project._git_dir_watcher = Some(watcher);
                        });
                        if stored.is_err() {
                            return;
                        }
                        events
                    }
                    Err(error) => {
                        tracing::warn!(%error, "no se pudo observar .git; el margen de git se actualiza solo al guardar");
                        return;
                    }
                };
                while events.recv().await.is_ok() {
                    let applied = this.update(cx, |project, cx| {
                        tracing::debug!("cambió .git: estado y margen de git");
                        project.refresh_git();
                        project.refresh_git_diffs(cx);
                    });
                    if applied.is_err() {
                        break;
                    }
                }
            }));
        }

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

        let mut subscriptions = Vec::new();
        if options.git_gutter {
            // Every editor built over one of our buffers gets the git column.
            // Adoption is deferred: the editor is being created inside
            // someone else's update, possibly of this very project.
            let project = cx.weak_entity();
            subscriptions.push(cx.observe_new::<EditorView>(move |editor, _window, cx| {
                let buffer = editor.buffer().clone();
                let editor = cx.weak_entity();
                let project = project.clone();
                cx.defer(move |cx| {
                    project
                        .update(cx, |project, cx| project.adopt_editor(&buffer, editor, cx))
                        .ok();
                });
            }));
            // The project consumes its own announcement.
            subscriptions.push(cx.subscribe_self(|project, event: &GitDiffChanged, cx| {
                project.push_git_diff(&event.0, cx);
            }));
            // A theme change repaints the columns with the new `git.*`.
            subscriptions.push(
                cx.observe_global::<crate::settings::AppSettings>(|project, cx| {
                    project.push_git_colors(cx)
                }),
            );
        }

        Self {
            root,
            worktree,
            buffers: BufferStore::new(),
            git: GitStatus::default(),
            languages: Arc::new(LanguageRegistry::new()),
            git_watcher,
            git_gutter: options.git_gutter,
            options,
            git_diffs: HashMap::new(),
            editors: Vec::new(),
            _git_dir_watcher: None,
            _watcher: watcher,
            _tasks: tasks,
            _subscriptions: subscriptions,
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

    /// The background pieces this project runs ([`ProjectOptions::inert`]
    /// in the tests).
    pub fn options(&self) -> ProjectOptions {
        self.options
    }

    /// Cincel wrote or deleted `path` by itself (outside [`Project::save`],
    /// which already says so): announces [`HostWrote`].
    pub fn note_host_write(&mut self, path: &Path, cx: &mut Context<Self>) {
        cx.emit(HostWrote(path.to_path_buf()));
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

    /// Saves the buffer at `path`, and recomputes its git gutter.
    pub fn save(&mut self, path: &Path, cx: &mut Context<Self>) -> Result<(), String> {
        self.buffers
            .save(path)
            .map_err(|error| error.to_string())
            .inspect(|()| {
                cx.emit(HostWrote(path.to_path_buf()));
                cx.emit(ProjectEvent::BuffersChanged(Vec::new()));
                self.refresh_git_diff(path, cx);
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

    /// The last diff against `HEAD` computed for `path` (absolute): `None`
    /// before the first one lands, for a file git does not track, or outside
    /// a repository.
    pub fn git_diff(&self, path: &Path) -> Option<&LineDiff> {
        self.git_diffs.get(path).and_then(|slot| slot.diff.as_ref())
    }

    /// Recomputes the gutter diff of every open file (the index or `HEAD`
    /// moved).
    pub fn refresh_git_diffs(&mut self, cx: &mut Context<Self>) {
        let mut paths: Vec<PathBuf> = self.editors.iter().map(|(path, _)| path.clone()).collect();
        paths.sort();
        paths.dedup();
        for path in paths {
            self.refresh_git_diff(&path, cx);
        }
    }

    /// Recomputes the gutter diff of `path` (absolute) on the background
    /// executor. A newer request for the same path supersedes this one.
    pub fn refresh_git_diff(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !self.git_gutter {
            return;
        }
        let slot = self.git_diffs.entry(path.to_path_buf()).or_default();
        slot.requested += 1;
        let version = slot.requested;
        let root = self.root.clone();
        let file = path.to_path_buf();
        let diff = cx
            .background_executor()
            .spawn(async move { diff_against_head(&root, &file) });
        let path = path.to_path_buf();
        cx.spawn(async move |this, cx| {
            let diff = diff.await;
            this.update(cx, |project, cx| {
                project.finish_git_diff(path, version, diff, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Stores a diff that landed, unless a newer one was asked for since.
    fn finish_git_diff(
        &mut self,
        path: PathBuf,
        version: u64,
        diff: Option<LineDiff>,
        cx: &mut Context<Self>,
    ) {
        let Some(slot) = self.git_diffs.get_mut(&path) else {
            return;
        };
        if slot.requested != version {
            return;
        }
        slot.diff = diff;
        cx.emit(GitDiffChanged(path));
    }

    /// Registers an editor created over `buffer` if the buffer is one of
    /// ours, and asks for its file's diff.
    fn adopt_editor(
        &mut self,
        buffer: &BufferHandle,
        editor: WeakEntity<EditorView>,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self
            .buffers
            .paths()
            .find(|path| {
                self.buffers
                    .get(path)
                    .is_some_and(|handle| Arc::ptr_eq(&handle, buffer))
            })
            .map(Path::to_path_buf)
        else {
            return;
        };
        self.editors
            .retain(|(_, editor)| editor.upgrade().is_some());
        self.editors.push((path.clone(), editor.clone()));
        let colors = git_colors(cx);
        editor
            .update(cx, |editor, cx| editor.set_git_colors(colors, cx))
            .ok();
        self.refresh_git_diff(&path, cx);
    }

    /// Hands the stored diff of `path` to every editor of the file.
    fn push_git_diff(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.editors
            .retain(|(_, editor)| editor.upgrade().is_some());
        let hunks = self.git_diff(path).map(gutter_hunks);
        let colors = git_colors(cx);
        for (_, editor) in self.editors.iter().filter(|(file, _)| file == path) {
            let hunks = hunks.clone();
            editor
                .update(cx, |editor, cx| {
                    editor.set_git_colors(colors, cx);
                    editor.set_git_diff(hunks, cx);
                })
                .ok();
        }
    }

    /// Repaints every git column with the theme in force.
    fn push_git_colors(&mut self, cx: &mut Context<Self>) {
        let colors = git_colors(cx);
        for (_, editor) in &self.editors {
            editor
                .update(cx, |editor, cx| editor.set_git_colors(colors, cx))
                .ok();
        }
    }

    /// Applies one batch of file system events to the tree and the buffers,
    /// then hands it on as [`FilesChanged`]. The watcher's drain calls it;
    /// the tests call it by hand (their projects run without a watcher).
    pub fn apply_fs_events(&mut self, events: &[FsEvent], cx: &mut Context<Self>) {
        // `CINCEL_TRACE_TIMINGS=1` (`crate::bench::TIMING_TARGET`): the
        // main-thread cost of one watcher batch, the review's share included.
        let _watch_batch =
            tracing::info_span!(target: "cincel::timing", "watch_batch", events = events.len())
                .entered();
        let mut tree_changed = false;
        for event in events {
            tree_changed |= self.worktree.apply_fs_event(event);
        }
        let buffer_changes = self.buffers.handle_fs_events(events);
        // An open file changed on disk: its diff against `HEAD` did too.
        let mut changed_open_files: Vec<PathBuf> = events
            .iter()
            .flat_map(|event| match event {
                FsEvent::Renamed { from, to } => vec![from.clone(), to.clone()],
                other => vec![other.path().to_path_buf()],
            })
            .filter(|path| self.buffers.get(path).is_some())
            .collect();
        changed_open_files.sort();
        changed_open_files.dedup();
        for path in changed_open_files {
            self.refresh_git_diff(&path, cx);
        }
        if !buffer_changes.is_empty() {
            tracing::debug!(changes = buffer_changes.len(), "buffers reconciliados");
            cx.emit(ProjectEvent::BuffersChanged(buffer_changes));
        }
        if tree_changed {
            cx.emit(ProjectEvent::TreeChanged);
        }
        if !events.is_empty() {
            cx.emit(FilesChanged(events.to_vec()));
        }
        self.refresh_git();
        cx.notify();
    }
}

/// The editor's hunks for a diff against `HEAD`: the same rows, in the
/// editor's own vocabulary (the editor knows nothing about git).
fn gutter_hunks(diff: &LineDiff) -> Vec<GitGutterHunk> {
    diff.hunks
        .iter()
        .map(|hunk| GitGutterHunk {
            kind: match hunk.kind {
                GitHunkKind::Added => GitGutterKind::Added,
                GitHunkKind::Modified => GitGutterKind::Modified,
                GitHunkKind::Deleted => GitGutterKind::Deleted,
            },
            rows: hunk.new_rows.clone(),
        })
        .collect()
}

/// The `git.*` colours of the theme in force (the bundled dark theme's
/// before the settings are installed).
fn git_colors(cx: &App) -> GitGutterColors {
    let Some(settings) = cx.try_global::<crate::settings::AppSettings>() else {
        return GitGutterColors::default();
    };
    let theme = settings.theme();
    let color = |color: cincel_settings::Rgba| gpui::rgba(color.rgba_u32());
    GitGutterColors {
        added: color(theme.git_added),
        modified: color(theme.git_modified),
        deleted: color(theme.git_deleted),
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
