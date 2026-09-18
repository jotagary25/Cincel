//! Git status, from the `git` binary.
//!
//! `docs/specs/03-arquitectura.md` §1 settles the question: we shell out to
//! the system `git` instead of linking a library, so credentials, `includeIf`
//! configuration, hooks and worktrees behave exactly as they do in the user's
//! terminal.
//!
//! The command is `git status --porcelain=v2 -z`, whose output is stable,
//! unambiguous and NUL separated, so paths with spaces, quotes or newlines
//! need no unquoting.
//!
//! Nothing here fails loudly: no `git` on the `PATH`, a directory that is not
//! a repository, or a command that errors all give an empty status, because
//! the file tree must open either way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_channel::Receiver;
use parking_lot::{Condvar, Mutex};

/// Debounce applied to status refreshes (`modulos/project.md`).
pub const GIT_DEBOUNCE: Duration = Duration::from_millis(500);

/// The state of one file according to git.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GitFileStatus {
    /// Not tracked by git.
    Untracked,
    /// Tracked and changed.
    Modified,
    /// Newly added to the index.
    Added,
    /// Deleted.
    Deleted,
    /// Unmerged: a conflict.
    Conflicted,
    /// Matched by a `.gitignore` (only reported when asked for).
    Ignored,
}

/// The status of every interesting file in the repository.
#[derive(Clone, Debug, Default)]
pub struct GitStatus {
    /// Repository root, when there is one.
    repo_root: Option<PathBuf>,
    /// Absolute path to status.
    entries: HashMap<PathBuf, GitFileStatus>,
}

impl GitStatus {
    /// Runs `git status` for the repository containing `root`.
    ///
    /// Blocking: call it from a background thread (or through
    /// [`GitStatusWatcher`], which owns one).
    pub fn collect(root: impl AsRef<Path>) -> GitStatus {
        let root = root.as_ref();
        let Some(repo_root) = repository_root(root) else {
            return GitStatus::default();
        };
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["status", "--porcelain=v2", "-z", "--untracked-files=all"])
            .output();
        let output = match output {
            Ok(output) if output.status.success() => output,
            Ok(output) => {
                tracing::debug!(
                    code = ?output.status.code(),
                    "git status falló; se asume un proyecto sin git"
                );
                return GitStatus::default();
            }
            Err(error) => {
                tracing::debug!(%error, "no se pudo ejecutar git");
                return GitStatus::default();
            }
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let entries = parse_porcelain_v2(&text, &repo_root);
        GitStatus {
            repo_root: Some(repo_root),
            entries,
        }
    }

    /// The repository root, when `collect` found one.
    pub fn repo_root(&self) -> Option<&Path> {
        self.repo_root.as_deref()
    }

    /// The status of `path` (absolute).
    pub fn get(&self, path: impl AsRef<Path>) -> Option<GitFileStatus> {
        self.entries.get(path.as_ref()).copied()
    }

    /// The status of `relative` inside the project rooted at `root`.
    pub fn get_relative(&self, root: &Path, relative: &Path) -> Option<GitFileStatus> {
        self.get(root.join(relative))
    }

    /// Whether any directory below `path` has a changed file, which is what
    /// the tree needs to mark a collapsed folder.
    pub fn contains_changes_under(&self, path: &Path) -> bool {
        self.entries.keys().any(|changed| changed.starts_with(path))
    }

    /// Every known path and its status.
    pub fn entries(&self) -> impl Iterator<Item = (&Path, GitFileStatus)> {
        self.entries
            .iter()
            .map(|(path, status)| (path.as_path(), *status))
    }

    /// How many files have a status.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is reported (also the case with no repository).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The root of the repository containing `path`, or `None`.
fn repository_root(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let root = text.trim_end_matches(['\n', '\r']);
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}

/// Parses `git status --porcelain=v2 -z`.
///
/// Record kinds: `1` ordinary change, `2` rename or copy (its record carries
/// a second NUL-separated path, the original one), `u` unmerged, `?`
/// untracked, `!` ignored, `#` headers.
fn parse_porcelain_v2(text: &str, repo_root: &Path) -> HashMap<PathBuf, GitFileStatus> {
    let mut entries = HashMap::new();
    let mut records = text.split('\0').peekable();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let (kind, rest) = record.split_at(1);
        let rest = rest.trim_start();
        match kind {
            "#" => {}
            "?" => {
                entries.insert(repo_root.join(rest), GitFileStatus::Untracked);
            }
            "!" => {
                entries.insert(repo_root.join(rest), GitFileStatus::Ignored);
            }
            "1" | "2" | "u" => {
                // Fields before the path: 8 for an ordinary change, 9 for a
                // rename or copy (it carries the similarity score), 10 for an
                // unmerged entry (three stages of mode and hash).
                let field_count = match kind {
                    "1" => 8,
                    "2" => 9,
                    _ => 10,
                };
                let mut fields = rest.splitn(field_count, ' ');
                let Some(xy) = fields.next() else { continue };
                let Some(path) = fields.last() else { continue };
                if kind == "2" {
                    // The original path follows as its own NUL-separated
                    // field; skip it so it is not read as a record.
                    records.next();
                }
                let status = if kind == "u" {
                    GitFileStatus::Conflicted
                } else {
                    classify(xy)
                };
                entries.insert(repo_root.join(path), status);
            }
            _ => {}
        }
    }
    entries
}

/// Maps the two `XY` letters of a porcelain record onto a status.
///
/// `X` is the index, `Y` the worktree; `.` means unchanged. A file added to
/// the index and then modified is reported as added, which is what the tree
/// shows.
fn classify(xy: &str) -> GitFileStatus {
    let mut chars = xy.chars();
    let index = chars.next().unwrap_or('.');
    let worktree = chars.next().unwrap_or('.');
    match (index, worktree) {
        ('A', _) => GitFileStatus::Added,
        ('D', _) | (_, 'D') => GitFileStatus::Deleted,
        _ => GitFileStatus::Modified,
    }
}

/// Runs `git status` on a background thread, at most once every
/// [`GIT_DEBOUNCE`].
///
/// [`GitStatusWatcher::trigger`] is cheap and coalescing: call it once per
/// batch of watcher events and once when the project opens. Results arrive on
/// the returned channel.
pub struct GitStatusWatcher {
    inner: Arc<Inner>,
}

struct Inner {
    requested: Mutex<bool>,
    signal: Condvar,
    stopped: AtomicBool,
}

impl GitStatusWatcher {
    /// Starts the thread for the project at `root` and asks for a first
    /// status right away.
    pub fn spawn(root: impl Into<PathBuf>) -> (GitStatusWatcher, Receiver<GitStatus>) {
        GitStatusWatcher::spawn_with_debounce(root, GIT_DEBOUNCE)
    }

    /// Same, with an explicit debounce (tests use a short one).
    pub fn spawn_with_debounce(
        root: impl Into<PathBuf>,
        debounce: Duration,
    ) -> (GitStatusWatcher, Receiver<GitStatus>) {
        let root = root.into();
        let (sender, receiver) = async_channel::unbounded();
        let inner = Arc::new(Inner {
            requested: Mutex::new(true),
            signal: Condvar::new(),
            stopped: AtomicBool::new(false),
        });
        let worker = inner.clone();
        std::thread::Builder::new()
            .name("asteroid-git-status".to_owned())
            .spawn(move || {
                loop {
                    // Wait for a request.
                    {
                        let mut requested = worker.requested.lock();
                        while !*requested && !worker.stopped.load(Ordering::Acquire) {
                            worker.signal.wait(&mut requested);
                        }
                        if worker.stopped.load(Ordering::Acquire) {
                            return;
                        }
                        *requested = false;
                    }
                    // Let the rest of the burst arrive, then coalesce it.
                    let deadline = Instant::now() + debounce;
                    loop {
                        let now = Instant::now();
                        if now >= deadline {
                            break;
                        }
                        std::thread::sleep((deadline - now).min(Duration::from_millis(25)));
                        if worker.stopped.load(Ordering::Acquire) {
                            return;
                        }
                    }
                    *worker.requested.lock() = false;
                    if sender.send_blocking(GitStatus::collect(&root)).is_err() {
                        return;
                    }
                }
            })
            .expect("no se pudo lanzar el hilo de git");
        (GitStatusWatcher { inner }, receiver)
    }

    /// Asks for a refresh. Calls inside the debounce window collapse into one.
    pub fn trigger(&self) {
        *self.inner.requested.lock() = true;
        self.inner.signal.notify_all();
    }
}

impl Drop for GitStatusWatcher {
    fn drop(&mut self) {
        self.inner.stopped.store(true, Ordering::Release);
        self.inner.signal.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository, or `None` when there is no usable `git` on this machine.
    fn repo() -> Option<tempfile::TempDir> {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .ok()
                .filter(|output| output.status.success())
        };
        git(&["init", "-q"])?;
        git(&["config", "user.email", "test@asteroid"])?;
        git(&["config", "user.name", "Test"])?;
        std::fs::write(dir.path().join("a.txt"), "uno\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "dos\n").unwrap();
        git(&["add", "."])?;
        git(&["commit", "-qm", "inicial"])?;
        Some(dir)
    }

    #[test]
    fn no_repository_is_an_empty_status_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hola").unwrap();
        // A temp dir is normally outside any repository; if this machine has
        // one at `/tmp` the status is still allowed to be empty.
        let status = GitStatus::collect(dir.path());
        assert!(status.get(dir.path().join("a.txt")).is_none());
    }

    #[test]
    fn maps_the_porcelain_records() {
        let Some(dir) = repo() else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "uno cambiado\n").unwrap();
        std::fs::remove_file(root.join("b.txt")).unwrap();
        std::fs::write(root.join("c.txt"), "tres\n").unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/d.txt"), "cuatro\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["add", "sub/d.txt"])
            .output()
            .unwrap();

        let status = GitStatus::collect(root);
        let real_root = status.repo_root().unwrap().to_path_buf();
        assert_eq!(
            status.get(real_root.join("a.txt")),
            Some(GitFileStatus::Modified)
        );
        assert_eq!(
            status.get(real_root.join("b.txt")),
            Some(GitFileStatus::Deleted)
        );
        assert_eq!(
            status.get(real_root.join("c.txt")),
            Some(GitFileStatus::Untracked)
        );
        assert_eq!(
            status.get(real_root.join("sub/d.txt")),
            Some(GitFileStatus::Added)
        );
        assert!(status.contains_changes_under(&real_root.join("sub")));
        assert!(!status.contains_changes_under(&real_root.join("no-existe")));
    }

    #[test]
    fn parses_renames_without_eating_the_next_record() {
        // A `2` record is followed by the original path in its own field.
        let text = "1 .M N... 100644 100644 100644 aaa bbb a.txt\0\
                    2 R. N... 100644 100644 100644 ccc ddd R100 nuevo.txt\0viejo.txt\0\
                    ? otro.txt\0";
        let entries = parse_porcelain_v2(text, Path::new("/repo"));
        assert_eq!(
            entries.get(Path::new("/repo/a.txt")),
            Some(&GitFileStatus::Modified)
        );
        assert_eq!(
            entries.get(Path::new("/repo/nuevo.txt")),
            Some(&GitFileStatus::Modified)
        );
        assert_eq!(
            entries.get(Path::new("/repo/otro.txt")),
            Some(&GitFileStatus::Untracked)
        );
        assert!(!entries.contains_key(Path::new("/repo/viejo.txt")));
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn unmerged_records_are_conflicts() {
        let text = "u UU N... 100644 100644 100644 100644 aaa bbb ccc conflicto.txt\0";
        let entries = parse_porcelain_v2(text, Path::new("/repo"));
        assert_eq!(
            entries.get(Path::new("/repo/conflicto.txt")),
            Some(&GitFileStatus::Conflicted)
        );
    }

    #[test]
    fn paths_with_spaces_survive() {
        let text = "1 .M N... 100644 100644 100644 aaa bbb un archivo con espacios.txt\0";
        let entries = parse_porcelain_v2(text, Path::new("/repo"));
        assert_eq!(
            entries.get(Path::new("/repo/un archivo con espacios.txt")),
            Some(&GitFileStatus::Modified)
        );
    }

    #[test]
    fn the_watcher_coalesces_triggers() {
        let Some(dir) = repo() else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let (watcher, statuses) =
            GitStatusWatcher::spawn_with_debounce(dir.path(), Duration::from_millis(50));
        // The first status arrives on its own.
        let first = recv(&statuses).expect("no llegó el primer estado");
        assert!(first.repo_root().is_some());

        std::fs::write(dir.path().join("a.txt"), "cambiado\n").unwrap();
        for _ in 0..10 {
            watcher.trigger();
        }
        let status = recv(&statuses).expect("no llegó el estado tras los triggers");
        assert_eq!(
            status.get(status.repo_root().unwrap().join("a.txt")),
            Some(GitFileStatus::Modified)
        );
        // Ten triggers must not produce ten runs.
        std::thread::sleep(Duration::from_millis(200));
        assert!(statuses.try_recv().is_err(), "se ejecutó git de más");
    }

    fn recv(receiver: &Receiver<GitStatus>) -> Option<GitStatus> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(status) = receiver.try_recv() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }
}
