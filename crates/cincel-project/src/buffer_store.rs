//! The open files.
//!
//! One [`Buffer`] per path, plus what the editor needs to reconcile it with
//! the disk: whether it is dirty, the modification time and content hash it
//! had when it was loaded or last saved, and whether an external change
//! turned into a conflict.
//!
//! # Buffers without GPUI
//!
//! `cincel-workspace` will wrap each buffer in a GPUI `Entity`, which this
//! crate cannot use, so buffers are handed out as
//! `Arc<parking_lot::Mutex<Buffer>>` ([`BufferHandle`]). Locking is always
//! short: read the text, apply edits, release. Nothing in this crate holds a
//! lock across I/O.
//!
//! # Reconciling with the disk (`modulos/project.md`)
//!
//! - saving records the hash of the bytes written, so the watcher event our
//!   own save produces is recognised ([`ReloadOutcome::SelfWrite`]) and does
//!   **not** reload the buffer;
//! - an external change to a clean buffer reloads it, as the minimal set of
//!   edits, so anchors and selections survive;
//! - an external change to a dirty buffer raises [`ReloadOutcome::Conflict`]
//!   and touches nothing: the workspace asks the user.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use cincel_text::{Buffer, LoadError};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::diff::minimal_edits;
use crate::watcher::FsEvent;

pub use cincel_text::EditSource;

/// A buffer shared between the store and whoever is editing it.
pub type BufferHandle = Arc<Mutex<Buffer>>;

/// SHA-256 of a file's bytes.
///
/// Used for one thing only: telling our own writes apart from somebody
/// else's. Comparing hashes is what makes "saving does not reload" work even
/// when the editor rewrites identical bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Hashes `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        ContentHash(out)
    }

    /// The raw digest.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in &self.0[..8] {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// What the file looked like on disk the last time we read or wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskState {
    /// Hash of the bytes.
    pub hash: ContentHash,
    /// Modification time, when the platform gave us one.
    pub mtime: Option<SystemTime>,
    /// Size in bytes.
    pub len: u64,
}

/// A summary of one open buffer, for the tab bar and the status bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferState {
    /// Whether it has unsaved changes.
    pub dirty: bool,
    /// Whether it changed on disk while it was dirty.
    pub conflict: bool,
    /// Whether the file is gone from the disk.
    pub deleted: bool,
    /// The disk state as of the last load or save.
    pub disk: DiskState,
    /// The buffer version the file was last saved at.
    pub saved_version: u64,
    /// The current buffer version.
    pub version: u64,
}

/// Opening a file failed.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// The bytes are not valid UTF-8; v1 does not transcode.
    #[error("«{0}» no es UTF-8 válido; se abre solo lectura")]
    NotUtf8(PathBuf),
    /// The path is a directory or something else that is not a file.
    #[error("«{0}» no es un archivo")]
    NotAFile(PathBuf),
    /// The file could not be read.
    #[error("no se pudo leer «{path}»: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// Saving failed.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// Nothing is open at that path.
    #[error("«{0}» no está abierto")]
    NotOpen(PathBuf),
    /// The file could not be written.
    #[error("no se pudo guardar «{path}»: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// What happened when a buffer was reconciled with the disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// Nothing is open at that path.
    NotOpen,
    /// The disk holds exactly what we last read or wrote.
    Unchanged,
    /// The change on disk is the one our own `save` just made.
    SelfWrite,
    /// The buffer was clean and now matches the disk again.
    Reloaded,
    /// The buffer is dirty and the disk changed: the user has to choose.
    Conflict,
    /// The file is gone.
    Deleted,
}

/// A buffer that changed because the disk did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferChange {
    /// Which file.
    pub path: PathBuf,
    /// What happened to it.
    pub outcome: ReloadOutcome,
}

/// The result of [`BufferStore::apply_agent_write`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentWrite {
    /// Which file.
    pub path: PathBuf,
    /// How many edits the write became.
    pub edits: usize,
    /// Whether the file did not exist before.
    pub created: bool,
    /// Whether the buffer had moved on from the snapshot the agent read, so
    /// the edits had to be computed against the live text instead.
    pub rebased: bool,
}

/// One open file.
#[derive(Debug)]
struct OpenBuffer {
    buffer: BufferHandle,
    disk: DiskState,
    saved_version: u64,
    conflict: bool,
    deleted: bool,
    /// Set by `save`, cleared by the first watcher event that matches the
    /// bytes we wrote.
    self_write: bool,
    /// The text the agent last read, and the buffer version it had then.
    agent_snapshot: Option<(String, u64)>,
}

/// Every file Cincel has open.
#[derive(Debug, Default)]
pub struct BufferStore {
    buffers: HashMap<PathBuf, OpenBuffer>,
}

impl BufferStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens `path`, or returns the buffer already open for it.
    ///
    /// One buffer per path: opening the same file twice gives the same
    /// handle, which is what keeps two tabs on one file in sync.
    pub fn open(&mut self, path: impl AsRef<Path>) -> Result<BufferHandle, OpenError> {
        let path = absolute(path.as_ref());
        if let Some(open) = self.buffers.get(&path) {
            return Ok(open.buffer.clone());
        }
        let bytes = std::fs::read(&path).map_err(|source| OpenError::Io {
            path: path.clone(),
            source,
        })?;
        let metadata = std::fs::metadata(&path).ok();
        if metadata.as_ref().is_some_and(|meta| meta.is_dir()) {
            return Err(OpenError::NotAFile(path));
        }
        let buffer = Buffer::from_bytes(&bytes).map_err(|error| match error {
            LoadError::NotUtf8 => OpenError::NotUtf8(path.clone()),
        })?;
        let open = OpenBuffer {
            buffer: Arc::new(Mutex::new(buffer)),
            disk: disk_state(&bytes, metadata.as_ref()),
            saved_version: 0,
            conflict: false,
            deleted: false,
            self_write: false,
            agent_snapshot: None,
        };
        let handle = open.buffer.clone();
        self.buffers.insert(path, open);
        Ok(handle)
    }

    /// Opens `path`, creating an empty buffer when the file does not exist
    /// yet. Used by the agent write path, where the agent may create files.
    pub fn open_or_create(&mut self, path: impl AsRef<Path>) -> Result<BufferHandle, OpenError> {
        let path = absolute(path.as_ref());
        match self.open(&path) {
            Err(OpenError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
                let open = OpenBuffer {
                    buffer: Arc::new(Mutex::new(Buffer::new(""))),
                    disk: disk_state(&[], None),
                    saved_version: 0,
                    conflict: false,
                    deleted: true,
                    self_write: false,
                    agent_snapshot: None,
                };
                let handle = open.buffer.clone();
                self.buffers.insert(path, open);
                Ok(handle)
            }
            other => other,
        }
    }

    /// The buffer open at `path`, if any.
    pub fn get(&self, path: impl AsRef<Path>) -> Option<BufferHandle> {
        self.buffers
            .get(&absolute(path.as_ref()))
            .map(|open| open.buffer.clone())
    }

    /// Whether `path` is open.
    pub fn is_open(&self, path: impl AsRef<Path>) -> bool {
        self.buffers.contains_key(&absolute(path.as_ref()))
    }

    /// Every open path.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.buffers.keys().map(PathBuf::as_path)
    }

    /// How many buffers are open.
    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    /// Whether nothing is open.
    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }

    /// Closes `path`, dropping the store's handle. Whoever else holds one
    /// keeps the buffer alive.
    pub fn close(&mut self, path: impl AsRef<Path>) -> bool {
        self.buffers.remove(&absolute(path.as_ref())).is_some()
    }

    /// Whether `path` has unsaved changes.
    pub fn is_dirty(&self, path: impl AsRef<Path>) -> bool {
        self.state(path).is_some_and(|state| state.dirty)
    }

    /// Whether `path` changed on disk while it was dirty.
    pub fn has_conflict(&self, path: impl AsRef<Path>) -> bool {
        self.state(path).is_some_and(|state| state.conflict)
    }

    /// Every dirty path.
    pub fn dirty_paths(&self) -> Vec<PathBuf> {
        self.buffers
            .iter()
            .filter(|(_, open)| open.is_dirty())
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// The state of `path`.
    pub fn state(&self, path: impl AsRef<Path>) -> Option<BufferState> {
        let open = self.buffers.get(&absolute(path.as_ref()))?;
        Some(BufferState {
            dirty: open.is_dirty(),
            conflict: open.conflict,
            deleted: open.deleted,
            disk: open.disk,
            saved_version: open.saved_version,
            version: open.buffer.lock().version(),
        })
    }

    /// Writes `path` to disk.
    ///
    /// The bytes come from the buffer, so the file's original line ending and
    /// BOM are re-applied (`cincel-text` keeps them). The hash of what was
    /// written is remembered, which is how the watcher event this produces is
    /// recognised as ours.
    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<(), SaveError> {
        let path = absolute(path.as_ref());
        let Some(open) = self.buffers.get_mut(&path) else {
            return Err(SaveError::NotOpen(path));
        };
        let bytes = open.buffer.lock().to_bytes();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|source| SaveError::Io {
                path: path.clone(),
                source,
            })?;
        }
        std::fs::write(&path, &bytes).map_err(|source| SaveError::Io {
            path: path.clone(),
            source,
        })?;
        let metadata = std::fs::metadata(&path).ok();
        open.disk = disk_state(&bytes, metadata.as_ref());
        open.mark_saved();
        open.conflict = false;
        open.deleted = false;
        open.self_write = true;
        Ok(())
    }

    /// Reconciles `path` with the disk, per `modulos/project.md`.
    ///
    /// Clean buffer and changed file: reload. Dirty buffer and changed file:
    /// [`ReloadOutcome::Conflict`], nothing touched. Our own save:
    /// [`ReloadOutcome::SelfWrite`].
    pub fn reload_from_disk(&mut self, path: impl AsRef<Path>) -> ReloadOutcome {
        let path = absolute(path.as_ref());
        let Some(open) = self.buffers.get_mut(&path) else {
            return ReloadOutcome::NotOpen;
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                open.deleted = true;
                return ReloadOutcome::Deleted;
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "no se pudo releer el archivo");
                return ReloadOutcome::Unchanged;
            }
        };
        open.deleted = false;
        let hash = ContentHash::of(&bytes);
        if hash == open.disk.hash {
            // Same bytes we last read or wrote.
            open.disk.mtime = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
            return if std::mem::take(&mut open.self_write) {
                ReloadOutcome::SelfWrite
            } else {
                ReloadOutcome::Unchanged
            };
        }
        open.self_write = false;
        if open.is_dirty() {
            open.conflict = true;
            return ReloadOutcome::Conflict;
        }
        match Buffer::from_bytes(&bytes) {
            Ok(fresh) => {
                open.replace_contents(&fresh.text(), EditSource::Load);
                let metadata = std::fs::metadata(&path).ok();
                open.disk = disk_state(&bytes, metadata.as_ref());
                open.mark_saved();
                ReloadOutcome::Reloaded
            }
            Err(LoadError::NotUtf8) => {
                tracing::warn!(path = %path.display(), "el archivo dejó de ser UTF-8; no se recarga");
                ReloadOutcome::Unchanged
            }
        }
    }

    /// Reloads `path` even though it is dirty, discarding the buffer's
    /// changes. What the workspace calls when the user picks "use the version
    /// on disk" after a conflict.
    pub fn discard_changes(&mut self, path: impl AsRef<Path>) -> ReloadOutcome {
        let path = absolute(path.as_ref());
        if let Some(open) = self.buffers.get_mut(&path) {
            open.mark_saved();
            open.conflict = false;
        }
        self.reload_from_disk(path)
    }

    /// Keeps the buffer as it is after a conflict; the next save overwrites
    /// the file.
    ///
    /// The bytes now on disk are recorded as seen, so the conflict is not
    /// raised again by the next event about the same change, and the buffer
    /// stays dirty, so saving writes the user's version over it.
    pub fn keep_my_version(&mut self, path: impl AsRef<Path>) -> bool {
        let path = absolute(path.as_ref());
        let on_disk = std::fs::read(&path).ok();
        match self.buffers.get_mut(&path) {
            Some(open) => {
                open.conflict = false;
                open.self_write = false;
                if let Some(bytes) = on_disk {
                    let metadata = std::fs::metadata(&path).ok();
                    open.disk = disk_state(&bytes, metadata.as_ref());
                }
                true
            }
            None => false,
        }
    }

    /// Reconciles the buffers a batch of file system events is about.
    ///
    /// Events for files that are not open are ignored; the tree deals with
    /// those.
    pub fn handle_fs_events(&mut self, events: &[FsEvent]) -> Vec<BufferChange> {
        let mut changes = Vec::new();
        for event in events {
            match event {
                FsEvent::Created(path) | FsEvent::Modified(path) => {
                    self.push_change(path, &mut changes);
                }
                FsEvent::Removed(path) => {
                    if self.is_open(path) {
                        self.push_change(path, &mut changes);
                    }
                }
                FsEvent::Renamed { from, to } => {
                    if self.is_open(from) {
                        self.push_change(from, &mut changes);
                    }
                    if self.is_open(to) {
                        self.push_change(to, &mut changes);
                    }
                }
            }
        }
        changes
    }

    /// Same as [`BufferStore::handle_fs_events`] for a single event.
    pub fn handle_fs_event(&mut self, event: &FsEvent) -> Vec<BufferChange> {
        self.handle_fs_events(std::slice::from_ref(event))
    }

    fn push_change(&mut self, path: &Path, changes: &mut Vec<BufferChange>) {
        if !self.is_open(path) {
            return;
        }
        let outcome = self.reload_from_disk(path);
        if matches!(
            outcome,
            ReloadOutcome::Reloaded | ReloadOutcome::Conflict | ReloadOutcome::Deleted
        ) {
            changes.push(BufferChange {
                path: absolute(path),
                outcome,
            });
        }
    }

    /// The text the agent should see: what is in memory, unsaved changes
    /// included.
    ///
    /// `line` is 1-based, as in ACP's `fs/read_text_file`; `limit` is a
    /// number of lines. Reading also records a snapshot, which is what
    /// [`BufferStore::apply_agent_write`] diffs against and what
    /// `cincel-review` uses as the base of the file for the turn.
    pub fn read_for_agent(
        &mut self,
        path: impl AsRef<Path>,
        line: Option<u32>,
        limit: Option<u32>,
    ) -> Result<String, OpenError> {
        let path = absolute(path.as_ref());
        self.open(&path)?;
        let open = self.buffers.get_mut(&path).expect("recién abierto");
        let (text, version) = {
            let buffer = open.buffer.lock();
            (buffer.text(), buffer.version())
        };
        open.agent_snapshot = Some((text.clone(), version));
        Ok(slice_lines(&text, line, limit))
    }

    /// The text the agent last read, if it has read this file.
    pub fn agent_snapshot(&self, path: impl AsRef<Path>) -> Option<String> {
        self.buffers
            .get(&absolute(path.as_ref()))
            .and_then(|open| open.agent_snapshot.as_ref())
            .map(|(text, _)| text.clone())
    }

    /// Applies an agent's whole-file write as the smallest set of edits, then
    /// saves.
    ///
    /// The edits are computed against the snapshot the agent read when that
    /// snapshot still matches the buffer. If the user has typed since, the
    /// live text is used instead and [`AgentWrite::rebased`] says so: the
    /// agent's content is authoritative for what it changed, but we will not
    /// silently resurrect text the user just deleted.
    pub fn apply_agent_write(
        &mut self,
        path: impl AsRef<Path>,
        content: &str,
        turn_id: u64,
    ) -> Result<AgentWrite, SaveError> {
        let path = absolute(path.as_ref());
        let created = !path.exists();
        if !self.is_open(&path) {
            // A file the agent creates has no buffer yet.
            self.open_or_create(&path).map_err(|error| SaveError::Io {
                path: path.clone(),
                source: std::io::Error::other(error.to_string()),
            })?;
        }
        let open = self.buffers.get_mut(&path).expect("abierto arriba");
        let live = open.buffer.lock().text();
        let rebased = match &open.agent_snapshot {
            Some((snapshot, _)) => snapshot != &live,
            None => false,
        };
        let edits = minimal_edits(&live, content);
        let count = edits.len();
        if count > 0 {
            let mut buffer = open.buffer.lock();
            buffer.start_transaction(EditSource::Agent { turn_id });
            for edit in edits.iter().rev() {
                buffer.replace(edit.range.clone(), &edit.new_text);
            }
            buffer.end_transaction();
        }
        open.agent_snapshot = Some((content.to_owned(), open.buffer.lock().version()));
        self.save(&path)?;
        Ok(AgentWrite {
            path,
            edits: count,
            created,
            rebased,
        })
    }
}

impl OpenBuffer {
    fn is_dirty(&self) -> bool {
        // Content-based: undoing back to the saved text is clean again.
        self.buffer.lock().is_dirty()
    }

    /// Marks the buffer's current text as the saved one and mirrors its version.
    fn mark_saved(&mut self) {
        let mut buffer = self.buffer.lock();
        buffer.mark_saved();
        self.saved_version = buffer.saved_version();
    }

    /// Replaces the buffer's text with `text` as minimal edits, so anchors,
    /// selections and review hunks survive a reload.
    fn replace_contents(&self, text: &str, source: EditSource) {
        let mut buffer = self.buffer.lock();
        let edits = minimal_edits(&buffer.text(), text);
        if edits.is_empty() {
            return;
        }
        buffer.start_transaction(source);
        for edit in edits.iter().rev() {
            buffer.replace(edit.range.clone(), &edit.new_text);
        }
        buffer.end_transaction();
    }
}

/// The disk state for `bytes` and (optional) metadata.
fn disk_state(bytes: &[u8], metadata: Option<&std::fs::Metadata>) -> DiskState {
    DiskState {
        hash: ContentHash::of(bytes),
        mtime: metadata.and_then(|meta| meta.modified().ok()),
        len: bytes.len() as u64,
    }
}

/// `path` made absolute without touching the file system.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `limit` lines of `text` starting at the 1-based `line`.
fn slice_lines(text: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return text.to_owned();
    }
    let start = line.unwrap_or(1).saturating_sub(1) as usize;
    let mut out = String::new();
    let lines = text.split_inclusive('\n').skip(start);
    match limit {
        Some(limit) => {
            for line in lines.take(limit as usize) {
                out.push_str(line);
            }
        }
        None => {
            for line in lines {
                out.push_str(line);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn undoing_back_to_the_saved_text_is_clean_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "hola\n").unwrap();
        let mut store = BufferStore::new();
        let handle = store.open(&path).unwrap();
        assert!(!store.is_dirty(&path));
        handle
            .lock()
            .transact(EditSource::User, |b| b.insert(0, "x"));
        assert!(store.is_dirty(&path));
        handle.lock().undo();
        assert!(
            !store.is_dirty(&path),
            "undo hasta el texto guardado debe quedar limpio"
        );
        handle.lock().redo();
        assert!(store.is_dirty(&path));
        handle.lock().undo();
        store.save(&path).unwrap();
        assert!(!store.is_dirty(&path));
    }

    use super::*;

    fn project() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("main.rs");
        std::fs::write(&file, "fn main() {\n    println!(\"hola\");\n}\n").unwrap();
        (dir, file)
    }

    #[test]
    fn one_buffer_per_path() {
        let (_dir, file) = project();
        let mut store = BufferStore::new();
        let a = store.open(&file).unwrap();
        let b = store.open(&file).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(store.len(), 1);
        assert!(store.close(&file));
        assert!(!store.is_open(&file));
    }

    #[test]
    fn dirty_tracking_and_save() {
        let (_dir, file) = project();
        let mut store = BufferStore::new();
        let buffer = store.open(&file).unwrap();
        assert!(!store.is_dirty(&file));

        buffer.lock().replace(0..2, "FN");
        assert!(store.is_dirty(&file));

        store.save(&file).unwrap();
        assert!(!store.is_dirty(&file));
        let on_disk = std::fs::read_to_string(&file).unwrap();
        assert!(on_disk.starts_with("FN main"));
    }

    #[test]
    fn saving_does_not_reload_our_own_write() {
        let (_dir, file) = project();
        let mut store = BufferStore::new();
        let buffer = store.open(&file).unwrap();
        buffer.lock().replace(0..2, "FN");
        store.save(&file).unwrap();

        // The watcher event our own save produced.
        let changes = store.handle_fs_event(&FsEvent::Modified(file.clone()));
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(store.reload_from_disk(&file), ReloadOutcome::Unchanged);
        assert!(buffer.lock().text().starts_with("FN main"));
        assert!(!store.is_dirty(&file));
    }

    #[test]
    fn an_external_change_reloads_a_clean_buffer() {
        let (_dir, file) = project();
        let mut store = BufferStore::new();
        let buffer = store.open(&file).unwrap();

        std::fs::write(&file, "fn main() {\n    println!(\"chau\");\n}\n").unwrap();
        let changes = store.handle_fs_event(&FsEvent::Modified(file.clone()));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].outcome, ReloadOutcome::Reloaded);
        assert!(buffer.lock().text().contains("chau"));
        assert!(!store.is_dirty(&file));
    }

    #[test]
    fn an_external_change_to_a_dirty_buffer_is_a_conflict() {
        let (_dir, file) = project();
        let mut store = BufferStore::new();
        let buffer = store.open(&file).unwrap();
        buffer.lock().replace(0..2, "FN");

        std::fs::write(&file, "otra cosa\n").unwrap();
        let changes = store.handle_fs_event(&FsEvent::Modified(file.clone()));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].outcome, ReloadOutcome::Conflict);
        assert!(store.has_conflict(&file));
        // Nothing was touched.
        assert!(buffer.lock().text().starts_with("FN main"));

        // The user can take the disk version...
        assert_eq!(store.discard_changes(&file), ReloadOutcome::Reloaded);
        assert_eq!(buffer.lock().text(), "otra cosa\n");
        assert!(!store.has_conflict(&file));
    }

    #[test]
    fn keeping_my_version_lets_the_next_save_win() {
        let (_dir, file) = project();
        let mut store = BufferStore::new();
        let buffer = store.open(&file).unwrap();
        buffer.lock().replace(0..2, "FN");
        std::fs::write(&file, "otra cosa\n").unwrap();
        assert_eq!(store.reload_from_disk(&file), ReloadOutcome::Conflict);

        assert!(store.keep_my_version(&file));
        assert!(!store.has_conflict(&file));
        store.save(&file).unwrap();
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .starts_with("FN main")
        );
    }

    #[test]
    fn a_deleted_file_is_reported() {
        let (_dir, file) = project();
        let mut store = BufferStore::new();
        store.open(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        let changes = store.handle_fs_event(&FsEvent::Removed(file.clone()));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].outcome, ReloadOutcome::Deleted);
        assert!(store.state(&file).unwrap().deleted);
    }

    #[test]
    fn events_for_closed_files_are_ignored() {
        let (_dir, file) = project();
        let mut store = BufferStore::new();
        assert!(store.handle_fs_event(&FsEvent::Modified(file)).is_empty());
        assert_eq!(
            store.reload_from_disk("/no/existe.rs"),
            ReloadOutcome::NotOpen
        );
    }

    #[test]
    fn reload_keeps_the_untouched_part_of_the_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "uno\ndos\ntres\n").unwrap();
        let mut store = BufferStore::new();
        let buffer = store.open(&file).unwrap();
        let version = buffer.lock().version();

        std::fs::write(&file, "uno\nDOS\ntres\n").unwrap();
        assert_eq!(store.reload_from_disk(&file), ReloadOutcome::Reloaded);
        // One minimal edit, not a whole-file replacement.
        assert_eq!(buffer.lock().version(), version + 1);
        assert_eq!(buffer.lock().text(), "uno\nDOS\ntres\n");
    }

    #[test]
    fn read_for_agent_serves_memory_and_slices_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "uno\ndos\ntres\ncuatro\n").unwrap();
        let mut store = BufferStore::new();
        let buffer = store.open(&file).unwrap();
        // An unsaved change must be visible to the agent.
        buffer.lock().replace(0..3, "UNO");

        assert_eq!(
            store.read_for_agent(&file, None, None).unwrap(),
            "UNO\ndos\ntres\ncuatro\n"
        );
        assert_eq!(
            store.read_for_agent(&file, Some(2), Some(2)).unwrap(),
            "dos\ntres\n"
        );
        assert_eq!(
            store.read_for_agent(&file, Some(4), None).unwrap(),
            "cuatro\n"
        );
        assert_eq!(store.read_for_agent(&file, Some(99), Some(2)).unwrap(), "");
        assert_eq!(
            store.agent_snapshot(&file).unwrap(),
            "UNO\ndos\ntres\ncuatro\n"
        );
    }

    #[test]
    fn agent_write_is_minimal_and_saves() {
        let (_dir, file) = project();
        let mut store = BufferStore::new();
        let buffer = store.open(&file).unwrap();
        store.read_for_agent(&file, None, None).unwrap();
        let version = buffer.lock().version();

        let content = "fn main() {\n    println!(\"chau\");\n}\n";
        let write = store.apply_agent_write(&file, content, 7).unwrap();
        assert_eq!(write.edits, 1);
        assert!(!write.created);
        assert!(!write.rebased);
        assert_eq!(buffer.lock().version(), version + 1);
        assert_eq!(buffer.lock().text(), content);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), content);
        assert!(!store.is_dirty(&file));

        // The edit is one undo unit, attributed to the agent.
        assert_eq!(buffer.lock().undo(), Some(EditSource::Agent { turn_id: 7 }));
    }

    #[test]
    fn agent_write_creates_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nuevo/sub/a.rs");
        let mut store = BufferStore::new();
        let write = store.apply_agent_write(&file, "hola\n", 1).unwrap();
        assert!(write.created);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hola\n");
    }

    #[test]
    fn agent_write_after_a_user_edit_is_rebased() {
        let (_dir, file) = project();
        let mut store = BufferStore::new();
        let buffer = store.open(&file).unwrap();
        store.read_for_agent(&file, None, None).unwrap();
        buffer.lock().replace(0..2, "FN");

        let content = "fn main() {\n    println!(\"chau\");\n}\n";
        let write = store.apply_agent_write(&file, content, 1).unwrap();
        assert!(write.rebased);
        assert_eq!(buffer.lock().text(), content);
    }

    #[test]
    fn a_self_write_event_after_an_agent_write_does_not_reload() {
        let (_dir, file) = project();
        let mut store = BufferStore::new();
        store.open(&file).unwrap();
        store.apply_agent_write(&file, "fn main() {}\n", 1).unwrap();
        assert!(store.handle_fs_event(&FsEvent::Modified(file)).is_empty());
    }

    #[test]
    fn non_utf8_files_are_rejected_not_mangled() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bin");
        std::fs::write(&file, [0xff, 0xfe, 0x00]).unwrap();
        let mut store = BufferStore::new();
        assert!(matches!(store.open(&file), Err(OpenError::NotUtf8(_))));
    }

    #[test]
    fn crlf_and_bom_survive_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("win.txt");
        let original: Vec<u8> = b"\xef\xbb\xbfuno\r\ndos\r\n".to_vec();
        std::fs::write(&file, &original).unwrap();
        let mut store = BufferStore::new();
        store.open(&file).unwrap();
        store.save(&file).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), original);
    }

    #[test]
    fn hashes_are_stable_and_short_to_print() {
        let hash = ContentHash::of(b"hola");
        assert_eq!(hash, ContentHash::of(b"hola"));
        assert_ne!(hash, ContentHash::of(b"chau"));
        assert_eq!(hash.to_string().len(), 16);
    }
}
