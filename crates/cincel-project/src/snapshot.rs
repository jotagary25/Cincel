//! The photo of the project the review takes before every agent turn
//! (`docs/specs/03-arquitectura.md` §4, v2: "la base viene de la foto del
//! proyecto; las herramientas son solo una pista").
//!
//! [`ProjectSnapshot::capture`] walks the project exactly like the tree does
//! (the same [`WorktreeConfig`], `.gitignore` and `files.exclude`, through
//! the shared walker builder) and keeps, per regular file, its size, its
//! modification time and a copy of its content:
//!
//! - text ([`looks_binary`] says no) up to [`SnapshotLimits::max_file_bytes`]:
//!   [`SnapshotContent::Text`];
//! - binary files, whatever their size: [`SnapshotContent::Binary`], size
//!   and hash only. The review leaves them out (the agent's changes to them
//!   are applied without review), so the photo only needs to tell them apart
//!   from the text files;
//! - bigger text files, and every text file once
//!   [`SnapshotLimits::max_total_bytes`] of copies is reached:
//!   [`SnapshotContent::HashOnly`].
//!
//! The walk and the reads run on the walker's own thread pool; the call
//! blocks until everything is read, so run it on a background executor.
//! [`ProjectSnapshot::changes`] is the end-of-turn sweep: the whole photo
//! against the disk, cheap metadata first and the content only when the
//! metadata differs or is too recent to be trusted ([`RACY_WINDOW`], the
//! "racy git" problem: a write within the same timestamp tick as the photo
//! keeps size and mtime). It blocks too: the review runs it on the
//! background executor through an `Arc<ProjectSnapshot>` (the type is
//! `Send + Sync`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use ignore::WalkState;
use parking_lot::Mutex;

use crate::buffer_store::ContentHash;
use crate::ignore_rules::IgnoreRules;
use crate::worktree::{WorktreeConfig, project_walk_builder};

/// A file whose modification time is this close to (or after) the moment
/// the photo was taken is compared by content even when size and mtime did
/// not change.
pub const RACY_WINDOW: Duration = Duration::from_secs(2);

/// Files bigger than this are not even hashed: their changes are detected by
/// size and modification time only.
const MAX_HASHED_BYTES: u64 = 64 * 1024 * 1024;

/// How many leading bytes [`looks_binary`] searches for a NUL (the same
/// sniff as the review store's).
pub const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Whether `bytes` are a binary file for the review: not valid UTF-8, or a
/// NUL in the first [`BINARY_SNIFF_BYTES`]. `truncated` says `bytes` are only
/// the start of the file (a UTF-8 sequence cut at the end is then fine).
pub fn looks_binary_prefix(bytes: &[u8], truncated: bool) -> bool {
    if bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
        return true;
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => false,
        // An incomplete sequence at the very end of a truncated read.
        Err(error) => !(truncated && error.error_len().is_none()),
    }
}

/// Whether the whole content `bytes` is a binary file for the review
/// ([`looks_binary_prefix`] over all of it).
pub fn looks_binary(bytes: &[u8]) -> bool {
    looks_binary_prefix(bytes, false)
}

/// How much of the project the photo copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotLimits {
    /// Files up to this size are copied (`review.max_file_size_kb`).
    pub max_file_bytes: u64,
    /// Copies stop once they add up to this (`review.snapshot_max_total_mb`);
    /// later files keep only their hash.
    pub max_total_bytes: u64,
}

impl SnapshotLimits {
    /// From the settings' units: kilobytes per file, megabytes in total.
    pub fn from_settings(max_file_size_kb: u64, snapshot_max_total_mb: u64) -> Self {
        Self {
            max_file_bytes: max_file_size_kb.saturating_mul(1024),
            max_total_bytes: snapshot_max_total_mb.saturating_mul(1024 * 1024),
        }
    }
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        Self::from_settings(2048, 300)
    }
}

/// What a snapshot needs to walk a project like its tree does: the root, the
/// ignore rules and the walk configuration. `Send`, so it can travel to a
/// background thread. Built by
/// [`Worktree::snapshot_source`](crate::Worktree::snapshot_source).
#[derive(Clone, Debug)]
pub struct SnapshotSource {
    pub(crate) root: PathBuf,
    pub(crate) rules: Arc<IgnoreRules>,
    pub(crate) config: WorktreeConfig,
}

impl SnapshotSource {
    /// The project root, absolute.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Takes the photo. Blocks until every file is read.
    pub fn capture(&self, limits: SnapshotLimits) -> ProjectSnapshot {
        ProjectSnapshot::capture(self, limits)
    }
}

/// What the photo kept of one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotContent {
    /// Valid UTF-8, exactly as on disk (BOM and line endings included).
    Text(Arc<str>),
    /// A binary file ([`looks_binary`]): only size, time and (up to 64 MB)
    /// hash. The review leaves binary files out.
    Binary,
    /// A text file over a limit: only size, time and (up to 64 MB) hash.
    HashOnly,
}

/// One file of the photo.
#[derive(Clone, Debug)]
pub struct SnapshotEntry {
    /// Size in bytes.
    pub size: u64,
    /// Modification time, when the file system reports one.
    pub modified: Option<SystemTime>,
    /// SHA-256 of the content, for binary and hash-only entries (`None` over
    /// 64 MB).
    pub hash: Option<ContentHash>,
    /// The copy, if any.
    pub content: SnapshotContent,
}

impl SnapshotEntry {
    /// The text copy, for a UTF-8 file.
    pub fn text(&self) -> Option<&Arc<str>> {
        match &self.content {
            SnapshotContent::Text(text) => Some(text),
            _ => None,
        }
    }

    /// Whether the file was binary ([`looks_binary`]).
    pub fn is_binary(&self) -> bool {
        matches!(self.content, SnapshotContent::Binary)
    }

    /// Whether `bytes` are the content this entry recorded. An entry without
    /// a copy nor a hash (a huge file) cannot tell and answers by size.
    pub fn same_bytes(&self, bytes: &[u8]) -> bool {
        if bytes.len() as u64 != self.size {
            return false;
        }
        match &self.content {
            SnapshotContent::Text(text) => text.as_bytes() == bytes,
            SnapshotContent::Binary | SnapshotContent::HashOnly => {
                self.hash.is_none_or(|hash| hash == ContentHash::of(bytes))
            }
        }
    }

    fn copied_bytes(&self) -> u64 {
        match &self.content {
            SnapshotContent::Text(text) => text.len() as u64,
            SnapshotContent::Binary | SnapshotContent::HashOnly => 0,
        }
    }
}

/// A difference between the photo and the disk.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapshotChange {
    /// A file that was not in the photo.
    Created(PathBuf),
    /// A file whose content changed.
    Modified(PathBuf),
    /// A file of the photo that is gone.
    Removed(PathBuf),
}

impl SnapshotChange {
    /// The file the change is about.
    pub fn path(&self) -> &Path {
        match self {
            SnapshotChange::Created(path)
            | SnapshotChange::Modified(path)
            | SnapshotChange::Removed(path) => path,
        }
    }
}

/// Every file of the project as it was when the photo was taken, keyed by
/// absolute path.
#[derive(Clone, Debug)]
pub struct ProjectSnapshot {
    source: SnapshotSource,
    limits: SnapshotLimits,
    started: SystemTime,
    entries: HashMap<PathBuf, SnapshotEntry>,
    copied_bytes: u64,
    over_total: bool,
    elapsed: Duration,
}

impl ProjectSnapshot {
    /// Walks the project of `source` and reads every file (in parallel, on
    /// the walker's threads). Blocks until done.
    pub fn capture(source: &SnapshotSource, limits: SnapshotLimits) -> Self {
        let clock = Instant::now();
        let started = SystemTime::now();
        let total = AtomicU64::new(0);
        let over = AtomicBool::new(false);
        let found: Mutex<Vec<(PathBuf, SnapshotEntry)>> = Mutex::new(Vec::new());
        walk_files(source, &source.root, |path, metadata| {
            if let Some(entry) = read_entry(path, metadata, limits, &total, &over) {
                found.lock().push((path.to_path_buf(), entry));
            }
        });
        let entries: HashMap<PathBuf, SnapshotEntry> = found.into_inner().into_iter().collect();
        let snapshot = Self {
            source: source.clone(),
            limits,
            started,
            copied_bytes: total.load(Ordering::Relaxed),
            over_total: over.load(Ordering::Relaxed),
            entries,
            elapsed: clock.elapsed(),
        };
        tracing::debug!(
            files = snapshot.entries.len(),
            copied = snapshot.copied_bytes,
            over_total = snapshot.over_total,
            elapsed = ?snapshot.elapsed,
            "foto del proyecto"
        );
        snapshot
    }

    /// The project root, absolute.
    pub fn root(&self) -> &Path {
        &self.source.root
    }

    /// The limits the photo was taken with.
    pub fn limits(&self) -> SnapshotLimits {
        self.limits
    }

    /// When the photo was taken.
    pub fn started(&self) -> SystemTime {
        self.started
    }

    /// How long taking it took.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// How many files it holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the project had no files.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Bytes held as copies (text only).
    pub fn copied_bytes(&self) -> u64 {
        self.copied_bytes
    }

    /// Whether [`SnapshotLimits::max_total_bytes`] was reached (some files
    /// kept only their hash).
    pub fn is_over_total(&self) -> bool {
        self.over_total
    }

    /// The entry of `path` (absolute).
    pub fn entry(&self, path: &Path) -> Option<&SnapshotEntry> {
        self.entries.get(path)
    }

    /// Every file of the photo.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.entries.keys().map(PathBuf::as_path)
    }

    /// The files of the photo below `dir`, sorted.
    pub fn paths_under(&self, dir: &Path) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self
            .entries
            .keys()
            .filter(|path| path.starts_with(dir) && path.as_path() != dir)
            .cloned()
            .collect();
        paths.sort();
        paths
    }

    /// Whether the project walk would skip `path` (the tree hides it).
    pub fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        if !path.starts_with(&self.source.root) {
            return true;
        }
        self.source.rules.is_ignored(path, is_dir)
    }

    /// The regular files below `dir` on disk now, walked like the project,
    /// sorted (a directory created during the turn).
    pub fn files_on_disk_under(&self, dir: &Path) -> Vec<PathBuf> {
        let found = Mutex::new(Vec::new());
        walk_files(&self.source, dir, |path, _| {
            found.lock().push(path.to_path_buf());
        });
        let mut paths = found.into_inner();
        paths.sort();
        paths
    }

    /// Replaces the copy of `path` with `text`: the host knows better (a
    /// hash-only file whose open buffer holds the text, or a file Cincel
    /// itself just saved).
    pub fn set_text(&mut self, path: &Path, text: Arc<str>) {
        let modified = std::fs::metadata(path)
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        let entry = SnapshotEntry {
            size: text.len() as u64,
            modified,
            hash: None,
            content: SnapshotContent::Text(text),
        };
        self.copied_bytes = self.copied_bytes.saturating_add(entry.copied_bytes());
        if let Some(old) = self.entries.insert(path.to_path_buf(), entry) {
            self.copied_bytes = self.copied_bytes.saturating_sub(old.copied_bytes());
        }
    }

    /// Re-reads `path` from disk into the photo, or drops it when it is gone
    /// (Cincel wrote or deleted it: that is not the agent's change).
    pub fn refresh(&mut self, path: &Path) {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_file() && !self.is_ignored(path, false) => metadata,
            _ => {
                if let Some(old) = self.entries.remove(path) {
                    self.copied_bytes = self.copied_bytes.saturating_sub(old.copied_bytes());
                }
                return;
            }
        };
        let previous = self
            .entries
            .get(path)
            .map(SnapshotEntry::copied_bytes)
            .unwrap_or(0);
        let total = AtomicU64::new(self.copied_bytes.saturating_sub(previous));
        let over = AtomicBool::new(self.over_total);
        if let Some(entry) = read_entry(path, &metadata, self.limits, &total, &over) {
            self.entries.insert(path.to_path_buf(), entry);
        } else {
            self.entries.remove(path);
        }
        self.copied_bytes = total.load(Ordering::Relaxed);
        self.over_total = over.load(Ordering::Relaxed);
    }

    /// Whether the file at `path` differs from the photo now: created,
    /// changed or removed. Reads the content only when size and time do not
    /// settle it.
    pub fn differs_on_disk(&self, path: &Path) -> bool {
        let metadata = std::fs::metadata(path).ok().filter(|m| m.is_file());
        match (self.entries.get(path), metadata) {
            (None, None) => false,
            (None, Some(_)) | (Some(_), None) => true,
            (Some(entry), Some(metadata)) => {
                self.entry_differs(path, entry, metadata.len(), metadata.modified().ok())
            }
        }
    }

    /// The end-of-turn sweep: every file of the photo against the disk, and
    /// every file on disk that the photo lacks. Sorted by path.
    pub fn changes(&self) -> Vec<SnapshotChange> {
        let current: Mutex<Vec<(PathBuf, u64, Option<SystemTime>)>> = Mutex::new(Vec::new());
        walk_files(&self.source, &self.source.root, |path, metadata| {
            current
                .lock()
                .push((path.to_path_buf(), metadata.len(), metadata.modified().ok()));
        });
        let current = current.into_inner();
        let mut seen: HashSet<&Path> = HashSet::with_capacity(current.len());
        let mut changes = Vec::new();
        for (path, size, modified) in &current {
            seen.insert(path.as_path());
            match self.entries.get(path) {
                None => changes.push(SnapshotChange::Created(path.clone())),
                Some(entry) => {
                    if self.entry_differs(path, entry, *size, *modified) {
                        changes.push(SnapshotChange::Modified(path.clone()));
                    }
                }
            }
        }
        for path in self.entries.keys() {
            if !seen.contains(path.as_path()) {
                changes.push(SnapshotChange::Removed(path.clone()));
            }
        }
        changes.sort_by(|a, b| a.path().cmp(b.path()));
        changes
    }

    /// Whether a file of the photo, now `size` bytes modified at `modified`,
    /// changed.
    fn entry_differs(
        &self,
        path: &Path,
        entry: &SnapshotEntry,
        size: u64,
        modified: Option<SystemTime>,
    ) -> bool {
        let same_meta = size == entry.size && modified == entry.modified;
        let racy = entry
            .modified
            .is_none_or(|time| time + RACY_WINDOW >= self.started);
        if same_meta && !racy {
            return false;
        }
        if matches!(
            entry.content,
            SnapshotContent::HashOnly | SnapshotContent::Binary
        ) && entry.hash.is_none()
        {
            // Too big to hash: the metadata is all there is.
            return !same_meta;
        }
        if size != entry.size {
            return true;
        }
        match std::fs::read(path) {
            Ok(bytes) => !entry.same_bytes(&bytes),
            Err(_) => true,
        }
    }
}

/// Calls `visit` for every regular file below `dir`, walked like the
/// project tree, from the walker's threads.
fn walk_files(
    source: &SnapshotSource,
    dir: &Path,
    visit: impl Fn(&Path, &std::fs::Metadata) + Sync,
) {
    if !dir.starts_with(&source.root) {
        return;
    }
    let walker = project_walk_builder(dir, &source.rules, &source.config).build_parallel();
    let visit = &visit;
    walker.run(|| {
        Box::new(move |result| {
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                return WalkState::Continue;
            }
            if let Ok(metadata) = entry.metadata() {
                visit(entry.path(), &metadata);
            }
            WalkState::Continue
        })
    });
}

/// Reads one file into a photo entry, charging its copy to `total`.
fn read_entry(
    path: &Path,
    metadata: &std::fs::Metadata,
    limits: SnapshotLimits,
    total: &AtomicU64,
    over: &AtomicBool,
) -> Option<SnapshotEntry> {
    let modified = metadata.modified().ok();
    let size = metadata.len();
    if size > limits.max_file_bytes {
        let (hash, binary) = if size <= MAX_HASHED_BYTES {
            match std::fs::read(path) {
                Ok(bytes) => (Some(ContentHash::of(&bytes)), looks_binary(&bytes)),
                Err(_) => (None, sniff_binary(path)),
            }
        } else {
            (None, sniff_binary(path))
        };
        return Some(SnapshotEntry {
            size,
            modified,
            hash,
            content: if binary {
                SnapshotContent::Binary
            } else {
                SnapshotContent::HashOnly
            },
        });
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::debug!(path = %path.display(), %error, "la foto no pudo leer el archivo");
            return None;
        }
    };
    let size = bytes.len() as u64;
    if looks_binary(&bytes) {
        // Out of the review: no copy, nothing charged to the budget.
        return Some(SnapshotEntry {
            size,
            modified,
            hash: Some(ContentHash::of(&bytes)),
            content: SnapshotContent::Binary,
        });
    }
    let previous = total.fetch_add(size, Ordering::Relaxed);
    if previous.saturating_add(size) > limits.max_total_bytes {
        total.fetch_sub(size, Ordering::Relaxed);
        over.store(true, Ordering::Relaxed);
        return Some(SnapshotEntry {
            size,
            modified,
            hash: Some(ContentHash::of(&bytes)),
            content: SnapshotContent::HashOnly,
        });
    }
    // `looks_binary` said it is valid UTF-8.
    let text = String::from_utf8(bytes)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());
    Some(SnapshotEntry {
        size,
        modified,
        hash: None,
        content: SnapshotContent::Text(Arc::from(text)),
    })
}

/// [`looks_binary_prefix`] over the first [`BINARY_SNIFF_BYTES`] of a file
/// too big to read whole (unreadable: text, so it stays in the review).
fn sniff_binary(path: &Path) -> bool {
    use std::io::Read as _;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::with_capacity(BINARY_SNIFF_BYTES);
    if file
        .take(BINARY_SNIFF_BYTES as u64)
        .read_to_end(&mut head)
        .is_err()
    {
        return false;
    }
    looks_binary_prefix(&head, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Worktree, WorktreeConfig};

    fn source(root: &Path, config: WorktreeConfig) -> SnapshotSource {
        let (worktree, _scan) = Worktree::scan(root, config).unwrap();
        worktree.snapshot_source()
    }

    fn photo(root: &Path, limits: SnapshotLimits) -> ProjectSnapshot {
        source(root, WorktreeConfig::default()).capture(limits)
    }

    fn relative(snapshot: &ProjectSnapshot) -> Vec<String> {
        let mut paths: Vec<String> = snapshot
            .paths()
            .map(|path| {
                path.strip_prefix(snapshot.root())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn walks_every_file_of_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/deep/er")).unwrap();
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("src/deep/er/x.py"), "x = 1\n").unwrap();
        std::fs::write(root.join(".env"), "SECRET=1\n").unwrap();

        let snapshot = photo(root, SnapshotLimits::default());
        assert_eq!(
            relative(&snapshot),
            vec![".env", "a.txt", "src/deep/er/x.py", "src/main.rs"]
        );
        let main = snapshot.entry(&root.join("src/main.rs")).unwrap();
        assert_eq!(main.text().map(|text| &**text), Some("fn main() {}\n"));
        assert_eq!(main.size, 13);
        assert!(main.modified.is_some());
        assert_eq!(snapshot.copied_bytes(), 2 + 13 + 6 + 9);
        assert!(!snapshot.is_over_total());
    }

    #[test]
    fn skips_what_the_tree_skips() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for folder in [".git", "target/debug", "node_modules/pkg", "build", "src"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        std::fs::write(root.join(".git/HEAD"), "ref\n").unwrap();
        std::fs::write(root.join("target/debug/app"), "bin").unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "x").unwrap();
        std::fs::write(root.join("build/out.txt"), "ignorado por git").unwrap();
        std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        std::fs::write(root.join("src/secreto.key"), "k").unwrap();

        let mut config = WorktreeConfig::default();
        config.excludes.push("**/*.key".to_owned());
        let snapshot = source(root, config).capture(SnapshotLimits::default());
        assert_eq!(relative(&snapshot), vec![".gitignore", "src/lib.rs"]);
        assert!(snapshot.is_ignored(&root.join("target/debug/nuevo"), false));
        assert!(snapshot.is_ignored(&root.join("src/otro.key"), false));
        assert!(!snapshot.is_ignored(&root.join("src/otro.rs"), false));
        assert!(snapshot.is_ignored(Path::new("/fuera/del/proyecto.rs"), false));
    }

    #[test]
    fn size_limits_keep_only_the_hash() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("grande.txt"), "x".repeat(2000)).unwrap();
        std::fs::write(root.join("chico.txt"), "hola\n").unwrap();

        let limits = SnapshotLimits {
            max_file_bytes: 1000,
            max_total_bytes: 1_000_000,
        };
        let snapshot = photo(root, limits);
        let big = snapshot.entry(&root.join("grande.txt")).unwrap();
        assert_eq!(big.content, SnapshotContent::HashOnly);
        assert_eq!(big.size, 2000);
        assert!(big.hash.is_some());
        assert!(big.same_bytes("x".repeat(2000).as_bytes()));
        assert!(!big.same_bytes("y".repeat(2000).as_bytes()));
        assert!(
            snapshot
                .entry(&root.join("chico.txt"))
                .unwrap()
                .text()
                .is_some()
        );
        assert!(!snapshot.is_over_total());

        // A total budget of 12 bytes fits one of the two 10-byte files.
        std::fs::write(root.join("uno.txt"), "0123456789").unwrap();
        std::fs::write(root.join("dos.txt"), "abcdefghij").unwrap();
        std::fs::remove_file(root.join("grande.txt")).unwrap();
        std::fs::remove_file(root.join("chico.txt")).unwrap();
        let snapshot = photo(
            root,
            SnapshotLimits {
                max_file_bytes: 1000,
                max_total_bytes: 12,
            },
        );
        let copies = ["uno.txt", "dos.txt"]
            .iter()
            .filter(|name| snapshot.entry(&root.join(name)).unwrap().text().is_some())
            .count();
        assert_eq!(copies, 1, "solo entra una copia");
        assert!(snapshot.is_over_total());
        assert_eq!(snapshot.copied_bytes(), 10);
        let hashed = ["uno.txt", "dos.txt"]
            .iter()
            .map(|name| snapshot.entry(&root.join(name)).unwrap())
            .find(|entry| entry.content == SnapshotContent::HashOnly)
            .unwrap();
        assert!(hashed.hash.is_some());
    }

    #[test]
    fn binaries_keep_only_size_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bytes = [0xff_u8, 0xfe, 0x00, 0x41, 0x80];
        std::fs::write(root.join("imagen.bin"), bytes).unwrap();
        // Valid UTF-8 with a NUL is binary too (the review store's sniff).
        std::fs::write(root.join("nul.txt"), "a\0b").unwrap();
        std::fs::write(root.join("texto.txt"), "ñandú\n").unwrap();
        // A big binary is told apart by its first bytes.
        let mut big = vec![0x89_u8, b'P', b'N', b'G'];
        big.extend(std::iter::repeat_n(b'a', 4000));
        std::fs::write(root.join("grande.png"), &big).unwrap();

        let limits = SnapshotLimits {
            max_file_bytes: 1000,
            max_total_bytes: 1_000_000,
        };
        let snapshot = photo(root, limits);
        let binary = snapshot.entry(&root.join("imagen.bin")).unwrap();
        assert!(binary.is_binary());
        assert_eq!(binary.content, SnapshotContent::Binary);
        assert_eq!(binary.hash, Some(ContentHash::of(&bytes)));
        assert_eq!(binary.size, 5);
        assert!(binary.text().is_none());
        assert!(binary.same_bytes(&bytes));
        assert!(!binary.same_bytes(&[0xff, 0xfe, 0x00, 0x41, 0x81]));
        assert!(snapshot.entry(&root.join("nul.txt")).unwrap().is_binary());
        assert!(
            snapshot
                .entry(&root.join("grande.png"))
                .unwrap()
                .is_binary()
        );
        // Only the text file is copied.
        assert_eq!(snapshot.copied_bytes(), "ñandú\n".len() as u64);
    }

    #[test]
    fn binary_sniff() {
        assert!(looks_binary(&[0xff, 0xfe]));
        assert!(looks_binary(b"a\0b"));
        assert!(!looks_binary("ñandú".as_bytes()));
        // A multi-byte sequence cut by a truncated read is still text.
        let cut = &"ñ".as_bytes()[..1];
        assert!(looks_binary(cut));
        assert!(!looks_binary_prefix(cut, true));
        // A NUL past the sniffed prefix does not count.
        let mut late = vec![b'a'; BINARY_SNIFF_BYTES];
        late.push(0);
        assert!(!looks_binary(&late));
    }

    #[test]
    fn the_sweep_finds_created_modified_and_removed_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("a.txt"), "uno\n").unwrap();
        std::fs::write(root.join("b.txt"), "dos\n").unwrap();
        std::fs::write(root.join("c.txt"), "tres\n").unwrap();
        std::fs::write(root.join("igual.txt"), "igual\n").unwrap();
        let snapshot = photo(root, SnapshotLimits::default());
        assert!(snapshot.changes().is_empty(), "nada cambió todavía");

        std::fs::write(root.join("a.txt"), "UNO distinto\n").unwrap();
        // Same size and the very same modification time: only the content
        // tells (the photo is recent, so it is compared).
        let file = std::fs::File::options()
            .write(true)
            .open(root.join("b.txt"))
            .unwrap();
        let before = file.metadata().unwrap().modified().unwrap();
        drop(file);
        std::fs::write(root.join("b.txt"), "DOS\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(root.join("b.txt"))
            .unwrap()
            .set_modified(before)
            .unwrap();
        std::fs::remove_file(root.join("c.txt")).unwrap();
        std::fs::write(root.join("src/nuevo.rs"), "fn x() {}\n").unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/basura"), "x").unwrap();

        assert_eq!(
            snapshot.changes(),
            vec![
                SnapshotChange::Modified(root.join("a.txt")),
                SnapshotChange::Modified(root.join("b.txt")),
                SnapshotChange::Removed(root.join("c.txt")),
                SnapshotChange::Created(root.join("src/nuevo.rs")),
            ]
        );
        assert!(snapshot.differs_on_disk(&root.join("b.txt")));
        assert!(!snapshot.differs_on_disk(&root.join("igual.txt")));
        assert!(!snapshot.differs_on_disk(&root.join("nunca.txt")));
        assert_eq!(
            snapshot.files_on_disk_under(&root.join("src")),
            vec![root.join("src/nuevo.rs")]
        );
        assert!(snapshot.paths_under(&root.join("src")).is_empty());
    }

    #[test]
    fn refresh_and_set_text_follow_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "uno\n").unwrap();
        let mut snapshot = photo(root, SnapshotLimits::default());

        // Cincel saved it: the photo takes the new content.
        std::fs::write(root.join("a.txt"), "guardado por el usuario\n").unwrap();
        snapshot.refresh(&root.join("a.txt"));
        assert!(snapshot.changes().is_empty());
        assert_eq!(
            snapshot.copied_bytes(),
            "guardado por el usuario\n".len() as u64
        );

        // A new file Cincel created, then one it deleted.
        std::fs::write(root.join("nuevo.txt"), "").unwrap();
        snapshot.refresh(&root.join("nuevo.txt"));
        std::fs::remove_file(root.join("a.txt")).unwrap();
        snapshot.refresh(&root.join("a.txt"));
        assert!(snapshot.changes().is_empty());
        assert!(snapshot.entry(&root.join("a.txt")).is_none());
        assert_eq!(snapshot.copied_bytes(), 0);

        snapshot.set_text(&root.join("nuevo.txt"), Arc::from("del buffer"));
        assert_eq!(
            snapshot
                .entry(&root.join("nuevo.txt"))
                .and_then(SnapshotEntry::text)
                .map(|text| &**text),
            Some("del buffer")
        );
    }

    /// The end-of-turn sweep runs `changes()` on the background executor
    /// through an `Arc<ProjectSnapshot>` (`08-etapa6-cierre-1-0.md` D12).
    #[test]
    fn the_photo_can_be_shared_with_a_background_thread() {
        fn shareable<T: Send + Sync + 'static>() {}
        shareable::<ProjectSnapshot>();
        shareable::<SnapshotChange>();

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::write(root.join("a.txt"), "uno\n").unwrap();
        let snapshot = Arc::new(photo(&root, SnapshotLimits::default()));
        std::fs::write(root.join("a.txt"), "uno y dos\n").unwrap();
        let shared = snapshot.clone();
        let changes = std::thread::spawn(move || shared.changes()).join().unwrap();
        assert_eq!(changes, vec![SnapshotChange::Modified(root.join("a.txt"))]);
        assert_eq!(changes, snapshot.changes());
    }

    /// The budget of the spec: 5 000 files / 50 MB in well under a second
    /// (tolerant: fails only over 3 s, for slow CI machines).
    #[test]
    fn five_thousand_files_fit_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let line = "let valor = 42; // una línea de relleno para la foto\n";
        let body = line.repeat(10_000 / line.len() + 1);
        for index in 0..5_000 {
            let folder = root.join(format!("m{:02}", index % 50));
            if index < 50 {
                std::fs::create_dir_all(&folder).unwrap();
            }
            std::fs::write(folder.join(format!("f{index}.rs")), &body).unwrap();
        }
        let started = Instant::now();
        let snapshot = photo(root, SnapshotLimits::default());
        let elapsed = started.elapsed();
        eprintln!(
            "foto de {} archivos / {} bytes: {elapsed:?}",
            snapshot.len(),
            snapshot.copied_bytes()
        );
        assert_eq!(snapshot.len(), 5_000);
        assert!(snapshot.copied_bytes() >= 50_000_000);
        assert!(elapsed < Duration::from_secs(3), "tardó {elapsed:?}");
    }
}
