//! The photo of the project behind the review
//! (`docs/specs/03-arquitectura.md` §4, v2: "la base viene de la foto del
//! proyecto; las herramientas son solo una pista").
//!
//! A child module of `crate::review` (it works on [`Review`]'s own fields):
//!
//! 1. **Photo.** [`Review::start_photo`], from `begin_prompt`, takes a
//!    [`ProjectSnapshot`] on the background executor (on the calling thread
//!    for an inert project, the tests); the prompt waits for
//!    [`Review::photo_waiter`]. Hash-only entries whose buffer is open take
//!    the buffer's text as their copy.
//! 2. **Watch.** [`Review::on_files_changed`] gets every watcher batch of the
//!    project ([`crate::project::FilesChanged`]) and adopts each path:
//!    [`Review::adopt_change`] captures the photo's copy as the base of a
//!    path not in review (`file_created` if it was not there) and re-reads
//!    the disk (`file_written` / `file_deleted`). A rename is a deletion
//!    plus a creation. A buffer with unsaved changes keeps them, as the
//!    user's (they join the base).
//! 3. **Sweep.** [`Review::sweep_photo`], from `end_turn`, compares the whole
//!    photo with the disk and adopts whatever the watcher did not report.
//! 4. **Attribution.** [`Review::on_host_wrote`]: whatever Cincel writes
//!    during the turn refreshes the photo, so it is never the agent's.
//!    Changes by other programs are the agent's.
//! 5. **Binaries.** Files that are not UTF-8 cannot enter the store (no
//!    buffer): they are kept here as [`BinaryChange`]s, whole-file only,
//!    "archivo binario cambiado por el agente"; a reject writes the photo's
//!    bytes back. A file the photo kept no copy of (over the limits) can only
//!    be accepted.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cincel_project::{FsEvent, ProjectSnapshot, SnapshotContent, SnapshotLimits};
use cincel_review::TurnId;
use cincel_text::EditSource;
use gpui::{Context, Task};

use super::{FileState, Review, file_name};

/// Where the photo of the running turn is.
pub(super) enum PhotoState {
    /// Being taken on the background executor; `waiters` are told when it
    /// lands.
    Capturing {
        turn: TurnId,
        waiters: Vec<tokio::sync::oneshot::Sender<()>>,
        /// What Cincel wrote while the photo was being taken: the photo may
        /// have read it before or after, so it is re-read when it lands.
        host_writes: Vec<PathBuf>,
        _task: Task<()>,
    },
    /// Ready: the project as it was before the prompt.
    Ready {
        turn: TurnId,
        snapshot: Box<ProjectSnapshot>,
    },
}

/// A file outside the store the agent changed: not UTF-8, or without a copy
/// in the photo.
#[derive(Clone, Debug)]
pub(super) struct BinaryChange {
    /// The turn that changed it.
    pub(super) turn: TurnId,
    /// Modified, created or deleted.
    pub(super) state: FileState,
    /// The bytes a reject writes back (`None`: created, or no copy).
    pub(super) original: Option<Arc<[u8]>>,
    /// Not UTF-8 (on either side); `false` is a text file the photo kept no
    /// copy of.
    pub(super) binary: bool,
}

impl BinaryChange {
    /// Whether a reject can undo it.
    pub(super) fn restorable(&self) -> bool {
        self.state == FileState::Created || self.original.is_some()
    }
}

/// Text from disk the way a buffer holds it: no BOM, `\n` line endings.
pub(super) fn normalize(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_owned()
    }
}

impl Review {
    // ---------------------------------------------------------------- photo

    /// Starts the photo of `turn`: on the background executor, or right here
    /// for a project without background pieces (the tests).
    pub(super) fn start_photo(&mut self, turn: TurnId, cx: &mut Context<Self>) {
        self.photo = None;
        let Some(project) = self.project.clone() else {
            return;
        };
        let (source, inert) = {
            let project = project.read(cx);
            (
                project.worktree().snapshot_source(),
                !project.options().watch_files,
            )
        };
        let settings = crate::settings::settings(cx);
        let limits = SnapshotLimits::from_settings(
            settings.review.max_file_size_kb,
            settings.review.snapshot_max_total_mb,
        );
        if inert && !self.photo_in_background {
            let snapshot = source.capture(limits);
            self.photo = Some(PhotoState::Capturing {
                turn,
                waiters: Vec::new(),
                host_writes: Vec::new(),
                _task: Task::ready(()),
            });
            self.photo_arrived(turn, snapshot, cx);
            return;
        }
        let capture = cx
            .background_executor()
            .spawn(async move { source.capture(limits) });
        let task = cx.spawn(async move |this, cx| {
            let snapshot = capture.await;
            let _ = this.update(cx, |review, cx| review.photo_arrived(turn, snapshot, cx));
        });
        self.photo = Some(PhotoState::Capturing {
            turn,
            waiters: Vec::new(),
            host_writes: Vec::new(),
            _task: task,
        });
    }

    /// The photo of `turn` is ready: keep it (if the turn is still the one
    /// running) and release the prompt.
    fn photo_arrived(
        &mut self,
        turn: TurnId,
        mut snapshot: ProjectSnapshot,
        cx: &mut Context<Self>,
    ) {
        let waiting = matches!(
            &self.photo,
            Some(PhotoState::Capturing { turn: capturing, .. }) if *capturing == turn
        );
        if !waiting || self.active_turn != Some(turn) {
            return;
        }
        if let Some(PhotoState::Capturing { host_writes, .. }) = &mut self.photo {
            for path in host_writes.drain(..) {
                snapshot.refresh(&path);
            }
        }
        // Files the photo kept no copy of take the open buffer's text: the
        // spec's "la base se toma del buffer si está abierto".
        if let Some(project) = &self.project {
            let project = project.read(cx);
            let buffers = project.buffers();
            let open: Vec<PathBuf> = buffers.paths().map(Path::to_path_buf).collect();
            for path in open {
                let hash_only = snapshot
                    .entry(&path)
                    .is_some_and(|entry| entry.content == SnapshotContent::HashOnly);
                if hash_only
                    && !buffers.is_dirty(&path)
                    && let Some(handle) = buffers.get(&path)
                {
                    let text = handle.lock().snapshot().text();
                    snapshot.set_text(&path, Arc::from(text));
                }
            }
        }
        tracing::info!(
            files = snapshot.len(),
            copied = snapshot.copied_bytes(),
            over_total = snapshot.is_over_total(),
            elapsed = ?snapshot.elapsed(),
            "foto del proyecto lista para el turno"
        );
        if snapshot.is_over_total() {
            tracing::warn!(
                "el proyecto supera review.snapshot_max_total_mb: algunos archivos solo guardan su huella"
            );
        }
        let previous = self.photo.replace(PhotoState::Ready {
            turn,
            snapshot: Box::new(snapshot),
        });
        if let Some(PhotoState::Capturing { waiters, .. }) = previous {
            for waiter in waiters {
                let _ = waiter.send(());
            }
        }
    }

    /// While the photo of the running turn is being taken, a receiver that
    /// resolves when it is ready (or abandoned); `None` when the prompt can
    /// go out now.
    pub fn photo_waiter(&mut self) -> Option<tokio::sync::oneshot::Receiver<()>> {
        match &mut self.photo {
            Some(PhotoState::Capturing { waiters, .. }) => {
                let (sender, receiver) = tokio::sync::oneshot::channel();
                waiters.push(sender);
                Some(receiver)
            }
            _ => None,
        }
    }

    /// Takes the photo on the background executor even for an inert
    /// project, so a test can see a prompt wait for it.
    #[cfg(test)]
    pub(crate) fn set_photo_in_background(&mut self, on: bool) {
        self.photo_in_background = on;
    }

    /// Whether the running turn has its photo.
    pub fn is_photo_ready(&self) -> bool {
        self.ready_snapshot().is_some()
    }

    /// The photo of the running turn, when it is ready.
    pub fn photo(&self) -> Option<&ProjectSnapshot> {
        self.ready_snapshot()
    }

    pub(super) fn ready_snapshot(&self) -> Option<&ProjectSnapshot> {
        match &self.photo {
            Some(PhotoState::Ready { turn, snapshot }) if self.active_turn == Some(*turn) => {
                Some(snapshot)
            }
            _ => None,
        }
    }

    /// The photo's text copy of `path`.
    pub(super) fn photo_text(&self, path: &Path) -> Option<Arc<str>> {
        self.ready_snapshot()?.entry(path)?.text().cloned()
    }

    /// Cincel wrote or deleted `path` by itself: during a turn, the photo
    /// takes it, so it is never the agent's.
    pub(super) fn on_host_wrote(&mut self, path: &Path) {
        match &mut self.photo {
            Some(PhotoState::Ready { snapshot, .. }) => snapshot.refresh(path),
            Some(PhotoState::Capturing { host_writes, .. }) => {
                host_writes.push(path.to_path_buf());
            }
            None => {}
        }
    }

    // ---------------------------------------------------------------- watch

    /// A batch of the watcher: during a turn, every path it names is the
    /// agent's to review (directories expand to their files).
    pub(super) fn on_files_changed(&mut self, events: &[FsEvent], cx: &mut Context<Self>) {
        if self.active_turn.is_none() {
            return;
        }
        let Some(snapshot) = self.ready_snapshot() else {
            return;
        };
        let mut paths: BTreeSet<PathBuf> = BTreeSet::new();
        for event in events {
            let touched: Vec<&Path> = match event {
                FsEvent::Renamed { from, to } => vec![from, to],
                other => vec![other.path()],
            };
            for path in touched {
                if path.is_dir() {
                    paths.extend(snapshot.files_on_disk_under(path));
                    paths.extend(snapshot.paths_under(path));
                } else if !path.exists() {
                    // A file, or a whole directory, went away.
                    paths.insert(path.to_path_buf());
                    paths.extend(snapshot.paths_under(path));
                    paths.extend(
                        self.store
                            .files()
                            .map(|file| file.path.clone())
                            .chain(self.binaries.keys().cloned())
                            .filter(|tracked| tracked.starts_with(path) && tracked != path),
                    );
                } else {
                    paths.insert(path.to_path_buf());
                }
            }
        }
        let mut any = false;
        for path in paths {
            any |= self.adopt_change(&path, cx);
        }
        if any {
            if let Some(project) = &self.project {
                project.read(cx).refresh_git();
            }
            self.prune(cx);
            self.refresh(None, cx);
        }
    }

    /// The end-of-turn sweep: the whole photo against the disk, adopting
    /// whatever the watcher did not report. Returns how many files changed.
    pub(super) fn sweep_photo(&mut self, cx: &mut Context<Self>) -> usize {
        let Some(snapshot) = self.ready_snapshot() else {
            return 0;
        };
        let changes = snapshot.changes();
        if changes.is_empty() {
            return 0;
        }
        tracing::debug!(
            changes = changes.len(),
            "repaso de la foto al terminar el turno"
        );
        let mut adopted = 0;
        for change in &changes {
            if self.adopt_change(change.path(), cx) {
                adopted += 1;
            }
        }
        if adopted > 0
            && let Some(project) = &self.project
        {
            project.read(cx).refresh_git();
        }
        adopted
    }

    /// `path` changed on disk during the turn: puts it in review against the
    /// photo. Returns whether anything was done.
    pub(super) fn adopt_change(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        let Some(turn) = self.active_turn else {
            return false;
        };
        if !self.in_project(path, cx) {
            return false;
        }
        let Some(snapshot) = self.ready_snapshot() else {
            return false;
        };
        let ignored = snapshot.is_ignored(path, false);
        let entry = snapshot.entry(path).cloned();
        if self.store.file(path).is_some() {
            // Already in review: its base stays, the disk is the truth.
            return self.reread(path, cx);
        }
        let disk = if path.is_file() {
            std::fs::read(path).ok()
        } else {
            None
        };
        if self.binaries.contains_key(path) {
            return self.binary_changed_again(path, entry.as_ref(), disk.as_deref());
        }
        match (entry, disk) {
            (None, None) => false,
            (None, Some(bytes)) => {
                if ignored {
                    return false;
                }
                if std::str::from_utf8(&bytes).is_ok() {
                    self.track_created(path, cx);
                } else {
                    self.binaries.insert(
                        path.to_path_buf(),
                        BinaryChange {
                            turn,
                            state: FileState::Created,
                            original: None,
                            binary: true,
                        },
                    );
                }
                tracing::debug!(path = %path.display(), "archivo creado durante el turno");
                true
            }
            (Some(entry), disk) => {
                if disk.as_deref().is_some_and(|bytes| entry.same_bytes(bytes)) {
                    return false;
                }
                let disk_is_text = disk
                    .as_deref()
                    .is_none_or(|bytes| std::str::from_utf8(bytes).is_ok());
                match (&entry.content, disk_is_text) {
                    (SnapshotContent::Text(text), true) => {
                        let before = normalize(text);
                        self.store.capture_base_text(path, &before);
                        let dirty = disk.is_some()
                            && self
                                .project
                                .as_ref()
                                .is_some_and(|project| project.read(cx).buffers().is_dirty(path));
                        if dirty {
                            // The buffer holds the user's unsaved changes (the
                            // disk is a conflict for it): they are the user's
                            // and join the base.
                            let Some(handle) = self.ensure_buffer(path, cx) else {
                                return true;
                            };
                            self.flush(None, cx);
                            let current = handle.lock().snapshot();
                            let tracked = self.store.file_written(path, &current, EditSource::User);
                            self.note(path, tracked);
                            true
                        } else {
                            tracing::debug!(path = %path.display(), "cambio del agente sin pista: base de la foto");
                            self.reread(path, cx)
                        }
                    }
                    _ => {
                        let binary = entry.is_binary() || !disk_is_text;
                        self.binaries.insert(
                            path.to_path_buf(),
                            BinaryChange {
                                turn,
                                state: if disk.is_some() {
                                    FileState::Modified
                                } else {
                                    FileState::Deleted
                                },
                                original: entry.bytes().map(Arc::from),
                                binary,
                            },
                        );
                        tracing::debug!(path = %path.display(), binary, "archivo cambiado por el agente, solo entero");
                        true
                    }
                }
            }
        }
    }

    /// A file already kept as a [`BinaryChange`] changed again.
    fn binary_changed_again(
        &mut self,
        path: &Path,
        entry: Option<&cincel_project::SnapshotEntry>,
        disk: Option<&[u8]>,
    ) -> bool {
        let back = match (entry, disk) {
            (Some(entry), Some(bytes)) => entry.same_bytes(bytes),
            (None, None) => true,
            _ => false,
        };
        if back {
            // Back to what it was: nothing to review.
            self.binaries.remove(path);
            return true;
        }
        if let Some(change) = self.binaries.get_mut(path) {
            change.turn = self.active_turn.unwrap_or(change.turn);
            change.state = match (entry.is_some(), disk.is_some()) {
                (true, false) => FileState::Deleted,
                (true, true) => FileState::Modified,
                _ => FileState::Created,
            };
        }
        true
    }

    // ------------------------------------------------------------ binaries

    /// Whether `path` belongs to the running turn (no decisions yet).
    fn binary_locked(&self, path: &Path) -> bool {
        self.binaries
            .get(path)
            .is_some_and(|change| Some(change.turn) == self.active_turn)
    }

    /// Accepts a binary change: the file stays as the agent left it.
    pub(super) fn accept_binary(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.binary_locked(path) {
            return;
        }
        self.binaries.remove(path);
        self.decided(path, cx);
    }

    /// Rejects a binary change: the photo's bytes go back (or the created
    /// file goes away).
    pub(super) fn reject_binary(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.binary_locked(path) {
            return;
        }
        let Some(change) = self.binaries.get(path).cloned() else {
            return;
        };
        if !change.restorable() {
            crate::toast::warn(
                format!(
                    "No hay copia previa de «{}»: solo se puede aceptar",
                    file_name(path)
                ),
                cx,
            );
            return;
        }
        match self.restore_binary(path, &change, cx) {
            Ok(()) => {
                self.binaries.remove(path);
                crate::toast::info_keyed(
                    "review-reject",
                    format!("Se restauró «{}»", file_name(path)),
                    cx,
                );
            }
            Err(error) => crate::toast::error(error, cx),
        }
        self.decided(path, cx);
    }

    /// Accepts (or rejects) every binary change of `turn` (every turn with
    /// `None`), for the turn and panel commands.
    pub(super) fn decide_binaries(
        &mut self,
        turn: Option<TurnId>,
        accept: bool,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<PathBuf> = self
            .binaries
            .iter()
            .filter(|(_, change)| turn.is_none_or(|turn| change.turn == turn))
            .map(|(path, _)| path.clone())
            .collect();
        for path in paths {
            let Some(change) = self.binaries.get(&path).cloned() else {
                continue;
            };
            if accept {
                self.binaries.remove(&path);
                continue;
            }
            if !change.restorable() {
                crate::toast::warn(
                    format!(
                        "No hay copia previa de «{}»: solo se puede aceptar",
                        file_name(&path)
                    ),
                    cx,
                );
                continue;
            }
            match self.restore_binary(&path, &change, cx) {
                Ok(()) => {
                    self.binaries.remove(&path);
                }
                Err(error) => crate::toast::error(error, cx),
            }
        }
    }

    /// Puts the photo's bytes back, or deletes a created file.
    fn restore_binary(
        &mut self,
        path: &Path,
        change: &BinaryChange,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let result = match (&change.original, change.state) {
            (_, FileState::Created) => match std::fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            },
            (Some(bytes), _) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(path, bytes)
            }
            (None, _) => return Err(format!("No hay copia previa de «{}»", file_name(path))),
        };
        result.map_err(|error| format!("No se pudo restaurar «{}»: {error}", file_name(path)))?;
        self.on_host_wrote(path);
        if let Some(project) = &self.project {
            project.update(cx, |project, cx| {
                project.buffers_mut().reload_from_disk(path);
                project.note_host_write(path, cx);
            });
            project.read(cx).refresh_git();
        }
        Ok(())
    }
}
