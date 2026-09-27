//! The project tree.
//!
//! # How a scan is driven
//!
//! [`Worktree::scan`] does the smallest amount of work that makes the window
//! useful — one directory listing of the root — and hands back a
//! [`WorktreeScan`] for everything else:
//!
//! ```no_run
//! # use cincel_project::{Worktree, WorktreeConfig};
//! # fn main() -> std::io::Result<()> {
//! let (worktree, scan) = Worktree::scan("/proyecto", WorktreeConfig::default())?;
//! // `worktree` already answers `children("")`; the rest arrives here:
//! for event in scan {          // blocking iterator, run it on a background thread
//!     let _ = event;           // send it to whoever owns the `Worktree`
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [`WorktreeScan`] is a blocking [`Iterator`] *and* exposes its
//! [`async_channel::Receiver`], so it works on a thread, on GPUI's background
//! executor, or in an async task, without this crate depending on any of
//! them. Applying an event is cheap and happens wherever the `Worktree`
//! lives: [`Worktree::apply`].
//!
//! # Order
//!
//! Children are sorted the way a file manager sorts them: directories first,
//! then a case-insensitive natural order, so `file2` comes before `file10`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};
use ignore::{DirEntry, WalkBuilder, WalkState};

use crate::ignore_rules::{ExcludeSet, IgnoreRules};
use crate::watcher::FsEvent;

/// What an [`Entry`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryKind {
    /// A directory.
    Dir,
    /// A regular file.
    File,
    /// A symbolic link that was not followed.
    Symlink,
}

impl EntryKind {
    /// Whether this entry can have children.
    pub fn is_dir(self) -> bool {
        matches!(self, EntryKind::Dir)
    }
}

/// One node of the tree.
///
/// Deliberately lean: the tree needs a name and a kind, and a 50 000 file
/// scan should not pay for a `stat` per entry. Size and modification time are
/// read on demand by whoever needs them (the [`BufferStore`](crate::BufferStore)
/// keeps its own).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Path relative to the project root. The root itself is the empty path.
    pub path: PathBuf,
    /// Directory, file or symlink.
    pub kind: EntryKind,
}

impl Entry {
    /// The file name, or `""` for the root.
    pub fn name(&self) -> &str {
        self.path
            .file_name()
            .map(|name| name.to_str().unwrap_or_default())
            .unwrap_or_default()
    }

    /// Whether this entry is a directory.
    pub fn is_dir(&self) -> bool {
        self.kind.is_dir()
    }

    /// The parent's relative path (the root's parent is itself).
    pub fn parent(&self) -> PathBuf {
        self.path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }
}

/// How to walk a project.
#[derive(Clone, Debug)]
pub struct WorktreeConfig {
    /// `files.exclude` globs from the settings.
    pub excludes: Vec<String>,
    /// Whether `.gitignore`, `.ignore` and `.git/info/exclude` apply.
    pub respect_gitignore: bool,
    /// Whether dot files are part of the tree. Cincel shows them (a
    /// `.gitignore` or a `.env` is worth editing); `.git` itself is excluded
    /// by the default globs.
    pub show_hidden: bool,
    /// Whether to follow symbolic links while walking.
    pub follow_symlinks: bool,
    /// Threads used by the background pass.
    pub threads: usize,
    /// Upper bound on the number of entries, so a runaway directory cannot
    /// eat all the memory. The scan stops and reports it.
    pub max_entries: usize,
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            excludes: vec![
                "**/.git".to_owned(),
                "**/target".to_owned(),
                "**/node_modules".to_owned(),
            ],
            respect_gitignore: true,
            show_hidden: true,
            follow_symlinks: false,
            threads: 0, // `ignore` picks a sensible number
            max_entries: 500_000,
        }
    }
}

impl WorktreeConfig {
    /// The configuration with the `files.exclude` globs of the settings.
    pub fn with_excludes<S: Into<String>>(excludes: impl IntoIterator<Item = S>) -> Self {
        Self {
            excludes: excludes.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }
}

/// Something the background scan found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanEvent {
    /// A batch of entries to add to the tree.
    Entries(Vec<Entry>),
    /// A directory could not be read; the tree simply lacks it.
    Error {
        /// Path that failed.
        path: PathBuf,
        /// Reason, in Spanish, ready for a toast.
        message: String,
    },
    /// The scan is over.
    Finished {
        /// How many entries it produced.
        entries: usize,
        /// How long it took.
        elapsed: Duration,
    },
}

/// The events of a running scan.
///
/// Iterating blocks until the scan finishes. Dropping it stops the scan at
/// the next batch (the sending side notices the channel is closed).
pub struct WorktreeScan {
    receiver: Receiver<ScanEvent>,
}

impl WorktreeScan {
    /// The underlying channel, for callers that prefer `recv().await` or
    /// `try_recv()` over blocking iteration.
    pub fn receiver(&self) -> &Receiver<ScanEvent> {
        &self.receiver
    }

    /// The channel itself.
    pub fn into_receiver(self) -> Receiver<ScanEvent> {
        self.receiver
    }
}

impl Iterator for WorktreeScan {
    type Item = ScanEvent;

    fn next(&mut self) -> Option<ScanEvent> {
        self.receiver.recv_blocking().ok()
    }
}

/// The file tree of the open project.
#[derive(Debug)]
pub struct Worktree {
    root: PathBuf,
    rules: Arc<IgnoreRules>,
    config: WorktreeConfig,
    entries: HashMap<PathBuf, Entry>,
    children: HashMap<PathBuf, Vec<PathBuf>>,
    scan_complete: bool,
    /// Patterns from `files.exclude` that did not compile.
    issues: Vec<String>,
}

/// How many entries the background scan sends at once.
const BATCH: usize = 256;

impl Worktree {
    /// Lists the root directory and starts the rest in the background.
    ///
    /// The returned tree already answers [`Worktree::children`] for the root,
    /// which is what the file tree paints first. Feed it the events of the
    /// [`WorktreeScan`] with [`Worktree::apply`] as they arrive.
    pub fn scan(
        root: impl Into<PathBuf>,
        config: WorktreeConfig,
    ) -> std::io::Result<(Worktree, WorktreeScan)> {
        let root = std::path::absolute(root.into())?;
        let (excludes, issues) = ExcludeSet::new(&config.excludes);
        let rules = Arc::new(IgnoreRules::new(
            root.clone(),
            excludes,
            config.respect_gitignore,
            config.show_hidden,
        ));
        let mut worktree = Worktree {
            entries: HashMap::from([(
                PathBuf::new(),
                Entry {
                    path: PathBuf::new(),
                    kind: EntryKind::Dir,
                },
            )]),
            children: HashMap::new(),
            scan_complete: false,
            issues,
            rules,
            config,
            root,
        };

        // The root listing is synchronous: it is one `readdir`, and the
        // window cannot paint anything without it.
        let walker = worktree
            .walk_builder(&worktree.root)
            .max_depth(Some(1))
            .build();
        for result in walker {
            match result {
                Ok(entry) if entry.depth() > 0 => {
                    if let Some(entry) = worktree.to_entry(&entry) {
                        worktree.insert(entry);
                    }
                }
                Ok(_) => {}
                Err(error) => tracing::debug!(%error, "error listando la raíz"),
            }
        }

        let scan = worktree.spawn_background_scan();
        Ok((worktree, scan))
    }

    /// Scans everything on the calling thread. Handy for tests and for tools
    /// that have nothing else to do meanwhile.
    pub fn scan_blocking(
        root: impl Into<PathBuf>,
        config: WorktreeConfig,
    ) -> std::io::Result<Worktree> {
        let (mut worktree, scan) = Worktree::scan(root, config)?;
        for event in scan {
            worktree.apply(event);
        }
        Ok(worktree)
    }

    /// The project root, absolute.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The rules that decide what the tree shows.
    pub fn rules(&self) -> &Arc<IgnoreRules> {
        &self.rules
    }

    /// Problems found while compiling `files.exclude`, in Spanish.
    pub fn issues(&self) -> &[String] {
        &self.issues
    }

    /// Whether the background scan is over.
    pub fn is_scan_complete(&self) -> bool {
        self.scan_complete
    }

    /// How many entries the tree has, the root excluded.
    pub fn len(&self) -> usize {
        self.entries.len().saturating_sub(1)
    }

    /// Whether the tree has no entries beyond the root.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Applies one event of the background scan.
    pub fn apply(&mut self, event: ScanEvent) {
        match event {
            ScanEvent::Entries(entries) => {
                for entry in entries {
                    self.insert(entry);
                }
            }
            ScanEvent::Error { path, message } => {
                tracing::debug!(path = %path.display(), %message, "error recorriendo el proyecto");
                self.issues.push(message);
            }
            ScanEvent::Finished { entries, elapsed } => {
                self.scan_complete = true;
                tracing::info!(entries, ?elapsed, "recorrido del proyecto terminado");
            }
        }
    }

    /// The entry at `path` (absolute or relative to the root).
    pub fn find(&self, path: impl AsRef<Path>) -> Option<&Entry> {
        let relative = self.relative(path.as_ref())?;
        self.entries.get(&relative)
    }

    /// Whether the tree knows about `path`.
    pub fn contains(&self, path: impl AsRef<Path>) -> bool {
        self.find(path).is_some()
    }

    /// The children of `dir` (the root is the empty path), sorted.
    pub fn children(&self, dir: impl AsRef<Path>) -> impl Iterator<Item = &Entry> {
        let relative = self.relative(dir.as_ref()).unwrap_or_default();
        self.children
            .get(&relative)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|path| self.entries.get(path))
    }

    /// Every entry, depth first, in display order (the root itself excluded).
    pub fn entries(&self) -> Entries<'_> {
        Entries {
            worktree: self,
            stack: vec![self.child_paths(Path::new("")).iter()],
        }
    }

    /// Whether the tree would hide `path`.
    pub fn is_ignored(&self, path: impl AsRef<Path>, is_dir: bool) -> bool {
        self.rules.is_ignored(path.as_ref(), is_dir)
    }

    /// Updates the tree after a file system event.
    ///
    /// Returns whether anything changed, so the caller can skip a repaint.
    /// A new directory is scanned synchronously: it has just been created, so
    /// it is small in every realistic case. A rename of a large tree is the
    /// exception and costs one walk of the moved subtree.
    pub fn apply_fs_event(&mut self, event: &FsEvent) -> bool {
        match event {
            FsEvent::Created(path) => self.add_path(path),
            FsEvent::Modified(path) => {
                // Content changes do not alter the tree; a path that is not
                // there yet does (some backends report a create as a modify).
                if self.contains(path) {
                    false
                } else {
                    self.add_path(path)
                }
            }
            FsEvent::Removed(path) => self.remove_path(path),
            FsEvent::Renamed { from, to } => {
                let removed = self.remove_path(from);
                let added = self.add_path(to);
                removed || added
            }
        }
    }

    /// Adds `path` (and everything under it, when it is a directory).
    fn add_path(&mut self, path: &Path) -> bool {
        let absolute = self.absolute(path);
        let Ok(metadata) = std::fs::symlink_metadata(&absolute) else {
            return false;
        };
        let is_dir = metadata.is_dir();
        if self.rules.is_ignored(&absolute, is_dir) {
            return false;
        }
        let Some(relative) = self.relative(&absolute) else {
            return false;
        };
        if relative.as_os_str().is_empty() {
            return false;
        }
        let kind = kind_of(&metadata);
        let mut changed = self.insert(Entry {
            path: relative,
            kind,
        });
        if is_dir {
            // A `.gitignore` may have arrived with the directory.
            self.rules.invalidate();
            for result in self.walk_builder(&absolute).build() {
                let Ok(entry) = result else { continue };
                if entry.depth() == 0 {
                    continue;
                }
                if let Some(entry) = self.to_entry(&entry) {
                    changed |= self.insert(entry);
                }
            }
        }
        changed
    }

    /// Removes `path` and its whole subtree.
    fn remove_path(&mut self, path: &Path) -> bool {
        let Some(relative) = self.relative(path) else {
            return false;
        };
        if relative.as_os_str().is_empty() || !self.entries.contains_key(&relative) {
            return false;
        }
        let mut doomed = vec![relative.clone()];
        let mut index = 0;
        while index < doomed.len() {
            if let Some(children) = self.children.get(&doomed[index]) {
                doomed.extend(children.iter().cloned());
            }
            index += 1;
        }
        for path in &doomed {
            self.entries.remove(path);
            self.children.remove(path);
        }
        let parent = parent_of(&relative);
        if let Some(siblings) = self.children.get_mut(&parent) {
            siblings.retain(|sibling| sibling != &relative);
        }
        true
    }

    /// Inserts one entry, creating any missing ancestor. Returns whether the
    /// tree changed.
    fn insert(&mut self, entry: Entry) -> bool {
        if entry.path.as_os_str().is_empty() {
            return false;
        }
        if let Some(existing) = self.entries.get(&entry.path) {
            return if existing.kind == entry.kind {
                false
            } else {
                // A path changed kind (a file replaced by a directory).
                self.remove_path(&entry.path.clone());
                self.insert(entry)
            };
        }
        let parent = parent_of(&entry.path);
        if !parent.as_os_str().is_empty() && !self.entries.contains_key(&parent) {
            // The parallel walk does not promise parents before children.
            self.insert(Entry {
                path: parent.clone(),
                kind: EntryKind::Dir,
            });
        }
        let index = self.insertion_index(&parent, &entry);
        self.children
            .entry(parent)
            .or_default()
            .insert(index, entry.path.clone());
        self.entries.insert(entry.path.clone(), entry);
        true
    }

    /// Where `entry` goes among its siblings.
    fn insertion_index(&self, parent: &Path, entry: &Entry) -> usize {
        let siblings = self.child_paths(parent);
        siblings
            .binary_search_by(|sibling| {
                let sibling = self
                    .entries
                    .get(sibling)
                    .expect("los hijos siempre están en `entries`");
                compare_entries(sibling, entry)
            })
            .unwrap_or_else(|index| index)
    }

    fn child_paths(&self, dir: &Path) -> &[PathBuf] {
        self.children
            .get(dir)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// `path` relative to the root, or `None` when it is outside it.
    fn relative(&self, path: &Path) -> Option<PathBuf> {
        if path.is_absolute() {
            path.strip_prefix(&self.root).ok().map(Path::to_path_buf)
        } else {
            Some(normalize_relative(path))
        }
    }

    fn absolute(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    /// A walker configured like the project wants it.
    fn walk_builder(&self, root: &Path) -> WalkBuilder {
        let mut builder = WalkBuilder::new(root);
        let rules = self.rules.clone();
        builder
            .hidden(!self.config.show_hidden)
            .parents(self.config.respect_gitignore)
            .git_ignore(self.config.respect_gitignore)
            // `.gitignore` applies even before `git init`, which is what a
            // user expects from a file tree (ripgrep's `--no-require-git`).
            .require_git(false)
            .git_global(self.config.respect_gitignore)
            .git_exclude(self.config.respect_gitignore)
            .follow_links(self.config.follow_symlinks)
            .threads(self.config.threads)
            // `files.exclude` is applied here rather than through
            // `Override` so that one implementation answers both the walk and
            // the watcher's "is this path interesting?".
            .filter_entry(move |entry| {
                entry.depth() == 0
                    || !rules
                        .excludes()
                        .matches(&rules.relative(entry.path()).unwrap_or_default())
            });
        builder
    }

    /// Converts a walker entry into a tree entry, skipping the root.
    fn to_entry(&self, entry: &DirEntry) -> Option<Entry> {
        let relative = self.relative(entry.path())?;
        if relative.as_os_str().is_empty() {
            return None;
        }
        let file_type = entry.file_type()?;
        let kind = if file_type.is_dir() {
            EntryKind::Dir
        } else if file_type.is_symlink() {
            EntryKind::Symlink
        } else {
            EntryKind::File
        };
        Some(Entry {
            path: relative,
            kind,
        })
    }

    /// Starts the background pass over everything below the root.
    fn spawn_background_scan(&self) -> WorktreeScan {
        let (sender, receiver) = async_channel::unbounded();
        let root = self.root.clone();
        let walker = self.walk_builder(&root).build_parallel();
        let max_entries = self.config.max_entries;
        let rules = self.rules.clone();
        std::thread::Builder::new()
            .name("cincel-worktree-scan".to_owned())
            .spawn(move || {
                let started = Instant::now();
                let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                walker.run(|| {
                    let mut batch = Batch::new(sender.clone());
                    let counter = counter.clone();
                    let rules = rules.clone();
                    let root = root.clone();
                    Box::new(move |result| {
                        match result {
                            Ok(entry) => {
                                if entry.depth() == 0 {
                                    return WalkState::Continue;
                                }
                                let Some(relative) = rules.relative(entry.path()) else {
                                    return WalkState::Continue;
                                };
                                let Some(file_type) = entry.file_type() else {
                                    return WalkState::Continue;
                                };
                                let kind = if file_type.is_dir() {
                                    EntryKind::Dir
                                } else if file_type.is_symlink() {
                                    EntryKind::Symlink
                                } else {
                                    EntryKind::File
                                };
                                let count =
                                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                if count >= max_entries {
                                    batch.error(
                                        entry.path().to_path_buf(),
                                        format!(
                                            "el proyecto tiene más de {max_entries} archivos; \
                                             el árbol queda incompleto"
                                        ),
                                    );
                                    return WalkState::Quit;
                                }
                                if !batch.push(Entry {
                                    path: relative,
                                    kind,
                                }) {
                                    return WalkState::Quit;
                                }
                            }
                            Err(error) => {
                                // `ignore::Error` does not carry the path in
                                // every variant, so the root stands in for it.
                                batch.error(
                                    root.clone(),
                                    format!("no se pudo recorrer «{}»: {error}", root.display()),
                                );
                            }
                        }
                        WalkState::Continue
                    })
                });
                let entries = counter.load(std::sync::atomic::Ordering::Relaxed);
                let _ = sender.send_blocking(ScanEvent::Finished {
                    entries,
                    elapsed: started.elapsed(),
                });
            })
            .expect("no se pudo lanzar el hilo de recorrido");
        WorktreeScan { receiver }
    }
}

/// Buffers entries so the channel sees one message per [`BATCH`], and flushes
/// whatever is left when the walker thread is done with it.
struct Batch {
    sender: Sender<ScanEvent>,
    entries: Vec<Entry>,
}

impl Batch {
    fn new(sender: Sender<ScanEvent>) -> Self {
        Self {
            sender,
            entries: Vec::with_capacity(BATCH),
        }
    }

    /// Returns whether the receiver is still there.
    fn push(&mut self, entry: Entry) -> bool {
        self.entries.push(entry);
        if self.entries.len() >= BATCH {
            return self.flush();
        }
        true
    }

    fn error(&mut self, path: PathBuf, message: String) {
        self.flush();
        let _ = self
            .sender
            .send_blocking(ScanEvent::Error { path, message });
    }

    fn flush(&mut self) -> bool {
        if self.entries.is_empty() {
            return true;
        }
        let entries = std::mem::take(&mut self.entries);
        self.sender
            .send_blocking(ScanEvent::Entries(entries))
            .is_ok()
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Depth-first iterator over the tree, in display order.
pub struct Entries<'a> {
    worktree: &'a Worktree,
    stack: Vec<std::slice::Iter<'a, PathBuf>>,
}

impl<'a> Iterator for Entries<'a> {
    type Item = &'a Entry;

    fn next(&mut self) -> Option<&'a Entry> {
        loop {
            let level = self.stack.last_mut()?;
            match level.next() {
                Some(path) => {
                    let entry = self.worktree.entries.get(path)?;
                    if entry.is_dir() {
                        self.stack.push(self.worktree.child_paths(path).iter());
                    }
                    return Some(entry);
                }
                None => {
                    self.stack.pop();
                }
            }
        }
    }
}

/// Directories first, then natural order by name.
fn compare_entries(a: &Entry, b: &Entry) -> Ordering {
    b.is_dir()
        .cmp(&a.is_dir())
        .then_with(|| natural_cmp(a.name(), b.name()))
}

/// Case-insensitive natural order: `a2` before `a10`, `B` next to `b`.
///
/// Ties (same letters, different case) fall back to a byte comparison so the
/// order is total and therefore usable with `binary_search`.
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => break,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let x = take_number(&mut left);
                let y = take_number(&mut right);
                // Compare by value, then by length so `01` and `1` are stable.
                match x.0.cmp(&y.0).then_with(|| x.1.cmp(&y.1)) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            (Some(x), Some(y)) => {
                left.next();
                right.next();
                let folded = x
                    .to_lowercase()
                    .cmp(y.to_lowercase())
                    .then_with(|| x.cmp(&y));
                if folded != Ordering::Equal {
                    return folded;
                }
            }
        }
    }
    a.cmp(b)
}

/// Consumes a run of digits, returning its value (saturating) and its length.
fn take_number(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> (u128, usize) {
    let mut value: u128 = 0;
    let mut length = 0;
    while let Some(digit) = chars.peek().and_then(|c| c.to_digit(10)) {
        value = value.saturating_mul(10).saturating_add(digit as u128);
        length += 1;
        chars.next();
    }
    (value, length)
}

/// The parent of a relative path, `""` for a top-level entry.
fn parent_of(path: &Path) -> PathBuf {
    path.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Drops `.` components and leading separators from a relative path.
fn normalize_relative(path: &Path) -> PathBuf {
    path.components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .collect()
}

fn kind_of(metadata: &std::fs::Metadata) -> EntryKind {
    if metadata.is_dir() {
        EntryKind::Dir
    } else if metadata.is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::File
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a small project and returns its directory guard.
    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join(".gitignore"), "ignorado.txt\n").unwrap();
        std::fs::write(root.join("ignorado.txt"), "no").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        std::fs::write(root.join("tests/uno.rs"), "").unwrap();
        std::fs::write(root.join("target/debug/x.o"), "").unwrap();
        std::fs::write(root.join("node_modules/x/index.js"), "").unwrap();
        dir
    }

    fn names(worktree: &Worktree, dir: impl AsRef<Path>) -> Vec<String> {
        worktree
            .children(dir)
            .map(|entry| entry.name().to_owned())
            .collect()
    }

    #[test]
    fn scan_respects_gitignore_and_excludes() {
        let dir = project();
        let worktree = Worktree::scan_blocking(dir.path(), WorktreeConfig::default()).unwrap();
        assert!(worktree.is_scan_complete());
        let paths: Vec<String> = worktree
            .entries()
            .map(|entry| entry.path.to_string_lossy().into_owned())
            .collect();
        assert!(paths.contains(&"src/main.rs".to_owned()), "{paths:?}");
        assert!(paths.contains(&".gitignore".to_owned()), "{paths:?}");
        assert!(!paths.iter().any(|p| p.starts_with("target")), "{paths:?}");
        assert!(
            !paths.iter().any(|p| p.starts_with("node_modules")),
            "{paths:?}"
        );
        assert!(!paths.contains(&"ignorado.txt".to_owned()), "{paths:?}");
    }

    #[test]
    fn the_root_is_listed_before_the_background_pass() {
        let dir = project();
        let (worktree, scan) = Worktree::scan(dir.path(), WorktreeConfig::default()).unwrap();
        // No event has been applied yet and the root already has children.
        let top = names(&worktree, "");
        assert!(top.contains(&"src".to_owned()), "{top:?}");
        assert!(top.contains(&"Cargo.toml".to_owned()), "{top:?}");
        assert!(!worktree.is_scan_complete());
        drop(scan);
    }

    #[test]
    fn children_are_sorted_dirs_first_and_naturally() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["b.rs", "a10.rs", "a2.rs", "A1.rs"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        for name in ["zeta", "Alfa"] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        let worktree = Worktree::scan_blocking(dir.path(), WorktreeConfig::default()).unwrap();
        assert_eq!(
            names(&worktree, ""),
            ["Alfa", "zeta", "A1.rs", "a2.rs", "a10.rs", "b.rs"]
        );
    }

    #[test]
    fn natural_order_details() {
        assert_eq!(natural_cmp("a2", "a10"), Ordering::Less);
        assert_eq!(natural_cmp("a", "B"), Ordering::Less);
        assert_eq!(natural_cmp("x", "x"), Ordering::Equal);
        assert_eq!(natural_cmp("x1y", "x1y2"), Ordering::Less);
        assert_eq!(natural_cmp("2", "10"), Ordering::Less);
        assert_eq!(natural_cmp("árbol", "arbol"), Ordering::Greater);
    }

    #[test]
    fn find_and_children_take_relative_or_absolute_paths() {
        let dir = project();
        let worktree = Worktree::scan_blocking(dir.path(), WorktreeConfig::default()).unwrap();
        assert!(worktree.find("src/main.rs").is_some());
        assert!(worktree.find(dir.path().join("src/main.rs")).is_some());
        assert!(worktree.find("/otro/lado.rs").is_none());
        assert_eq!(names(&worktree, "src"), ["lib.rs", "main.rs"]);
        assert_eq!(
            names(&worktree, dir.path().join("src")),
            ["lib.rs", "main.rs"]
        );
    }

    #[test]
    fn entries_are_depth_first_in_display_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/c.rs"), "").unwrap();
        std::fs::write(dir.path().join("a/z.rs"), "").unwrap();
        std::fs::write(dir.path().join("w.rs"), "").unwrap();
        let worktree = Worktree::scan_blocking(dir.path(), WorktreeConfig::default()).unwrap();
        let paths: Vec<String> = worktree
            .entries()
            .map(|entry| entry.path.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(paths, ["a", "a/b", "a/b/c.rs", "a/z.rs", "w.rs"]);
    }

    #[test]
    fn incremental_updates_follow_the_watcher() {
        let dir = project();
        let root = dir.path();
        let mut worktree = Worktree::scan_blocking(root, WorktreeConfig::default()).unwrap();

        // Create.
        std::fs::write(root.join("src/nuevo.rs"), "").unwrap();
        assert!(worktree.apply_fs_event(&FsEvent::Created(root.join("src/nuevo.rs"))));
        assert_eq!(names(&worktree, "src"), ["lib.rs", "main.rs", "nuevo.rs"]);

        // Rename.
        std::fs::rename(root.join("src/nuevo.rs"), root.join("src/otro.rs")).unwrap();
        assert!(worktree.apply_fs_event(&FsEvent::Renamed {
            from: root.join("src/nuevo.rs"),
            to: root.join("src/otro.rs"),
        }));
        assert_eq!(names(&worktree, "src"), ["lib.rs", "main.rs", "otro.rs"]);

        // Remove.
        std::fs::remove_file(root.join("src/otro.rs")).unwrap();
        assert!(worktree.apply_fs_event(&FsEvent::Removed(root.join("src/otro.rs"))));
        assert_eq!(names(&worktree, "src"), ["lib.rs", "main.rs"]);

        // A whole new directory arrives with its contents.
        std::fs::create_dir_all(root.join("extra/hondo")).unwrap();
        std::fs::write(root.join("extra/hondo/x.rs"), "").unwrap();
        assert!(worktree.apply_fs_event(&FsEvent::Created(root.join("extra"))));
        assert!(worktree.find("extra/hondo/x.rs").is_some());

        // And leaves again, taking its subtree with it.
        std::fs::remove_dir_all(root.join("extra")).unwrap();
        assert!(worktree.apply_fs_event(&FsEvent::Removed(root.join("extra"))));
        assert!(worktree.find("extra/hondo/x.rs").is_none());
        assert!(worktree.find("extra").is_none());
    }

    #[test]
    fn excluded_and_ignored_paths_are_not_added_incrementally() {
        let dir = project();
        let root = dir.path();
        let mut worktree = Worktree::scan_blocking(root, WorktreeConfig::default()).unwrap();
        std::fs::write(root.join("target/debug/y.o"), "").unwrap();
        assert!(!worktree.apply_fs_event(&FsEvent::Created(root.join("target/debug/y.o"))));
        assert!(!worktree.apply_fs_event(&FsEvent::Created(root.join("ignorado.txt"))));
        assert!(!worktree.apply_fs_event(&FsEvent::Created(PathBuf::from("/afuera.rs"))));
    }

    #[test]
    fn hidden_files_can_be_hidden() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "").unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        let config = WorktreeConfig {
            show_hidden: false,
            ..WorktreeConfig::default()
        };
        let worktree = Worktree::scan_blocking(dir.path(), config).unwrap();
        assert_eq!(names(&worktree, ""), ["a.rs"]);
    }

    #[test]
    fn broken_exclude_globs_are_reported_not_fatal() {
        let dir = project();
        let config = WorktreeConfig {
            excludes: vec!["**/target".to_owned(), "a[b".to_owned()],
            ..WorktreeConfig::default()
        };
        let worktree = Worktree::scan_blocking(dir.path(), config).unwrap();
        assert_eq!(worktree.issues().len(), 1);
        assert!(worktree.find("src/main.rs").is_some());
    }
}
