//! Git status, from the `git` binary.
//!
//! `docs/specs/03-arquitectura.md` §1 settles the question: we shell out to
//! the system `git` instead of linking a library, so credentials, `includeIf`
//! configuration, hooks and worktrees behave exactly as they do in the user's
//! terminal.
//!
//! The command is `git --no-optional-locks status --porcelain=v2 -z`, whose
//! output is stable, unambiguous and NUL separated, so paths with spaces,
//! quotes or newlines need no unquoting.
//!
//! The editor's git gutter lives here too: [`diff_against_head`] turns the
//! `-U0` diff of one file into [`GitHunk`]s, and [`GitDirWatcher`] reports
//! commits, `git add`, checkouts and the like done outside Cincel by watching
//! the index, `HEAD` and the branch refs.
//!
//! Nothing here fails loudly: no `git` on the `PATH`, a directory that is not
//! a repository, or a command that errors all give an empty status, because
//! the file tree must open either way.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_channel::Receiver;
use notify::RecursiveMode;
use notify::event::EventKind;
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
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
        // `--no-optional-locks`: without it `git status` refreshes the stat
        // information of `.git/index` and rewrites it, the `.git` watcher
        // sees the write, asks for a new status, and the loop never ends
        // (`docs/specs/07-etapa5-productividad.md` D10).
        let output = Command::new("git")
            .arg("--no-optional-locks")
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

    /// Whether `path` (absolute) is tracked by git: it lies inside the
    /// repository and is neither untracked nor ignored.
    ///
    /// A clean tracked file has no entry at all, so "no status" inside a
    /// repository means tracked; outside one (or with no repository) the
    /// answer is always `false`.
    pub fn is_tracked(&self, path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        let Some(repo_root) = self.repo_root.as_deref() else {
            return false;
        };
        if !path.starts_with(repo_root) {
            return false;
        }
        !matches!(
            self.get(path),
            Some(GitFileStatus::Untracked | GitFileStatus::Ignored)
        )
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
            .name("cincel-git-status".to_owned())
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

// -- the editor's git gutter ------------------------------------------------

/// What a [`GitHunk`] did to the lines of `HEAD`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GitHunkKind {
    /// Lines that do not exist in `HEAD`.
    Added,
    /// Lines of `HEAD` replaced by other lines.
    Modified,
    /// Lines of `HEAD` that are gone.
    Deleted,
}

/// One hunk of a file's diff against `HEAD`, in rows of the working copy.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GitHunk {
    /// What the hunk did.
    pub kind: GitHunkKind,
    /// Zero-based rows of the working copy the hunk covers. Empty for
    /// [`GitHunkKind::Deleted`], where `start` is the position of the gap:
    /// the row that now follows the deleted lines (0 when they were at the
    /// top of the file).
    pub new_rows: Range<u32>,
    /// How many lines of `HEAD` the hunk replaced (0 for an addition).
    pub old_len: u32,
}

/// A file's diff against `HEAD`, as the gutter needs it: hunks only, sorted
/// by row, no text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LineDiff {
    /// The hunks, in file order.
    pub hunks: Vec<GitHunk>,
}

impl LineDiff {
    /// Whether the file matches `HEAD`.
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }
}

/// The directory git keeps its data in for the repository containing `root`
/// (`git rev-parse --absolute-git-dir`), or `None` outside a repository or
/// without `git`.
///
/// In a linked worktree `.git` is a file pointing elsewhere; this returns the
/// directory it points to, which is where that worktree's `index` and `HEAD`
/// live.
pub fn git_dir(root: impl AsRef<Path>) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(root.as_ref())
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let dir = text.trim_end_matches(['\n', '\r']);
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

/// The diff of `path` against `HEAD`, for the editor's gutter.
///
/// Runs `git --no-optional-locks -C <repo_root> diff --no-color --no-ext-diff
/// --no-renames -U0 HEAD -- <path>` and parses only the `@@` headers
/// (`docs/specs/07-etapa5-productividad.md` §6.2). `repo_root` may be the
/// repository root or any directory inside it (the project root): `path` is
/// made relative to it when it can be, and git resolves the pathspec from
/// there. Pathspecs are literal, so a file called `*.rs` is just that file.
///
/// Returns `None` — no bars, no error — when there is no `git`, `path` is not
/// in a repository, git does not track it (a new file never `git add`ed), the
/// diff is binary, or git fails. A file in the index but not in `HEAD` (and
/// any tracked file of a repository with no commit yet) is all
/// [`GitHunkKind::Added`].
///
/// Blocking: call it from a background thread.
pub fn diff_against_head(repo_root: &Path, path: &Path) -> Option<LineDiff> {
    let pathspec = path.strip_prefix(repo_root).unwrap_or(path);
    if pathspec.as_os_str().is_empty() {
        return None;
    }
    let git = || {
        let mut command = Command::new("git");
        command
            .args(["--no-optional-locks", "--literal-pathspecs"])
            .arg("-C")
            .arg(repo_root)
            .stdin(Stdio::null());
        command
    };

    // Untracked (or outside the repository): no bars.
    let tracked = git()
        .args(["ls-files", "-z", "--"])
        .arg(pathspec)
        .output()
        .ok()?;
    if !tracked.status.success() || tracked.stdout.is_empty() {
        return None;
    }

    let diff = |base: &str| {
        git()
            .args([
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-renames",
                "-U0",
                base,
                "--",
            ])
            .arg(pathspec)
            .output()
            .ok()
    };
    let mut output = diff("HEAD")?;
    if !output.status.success() {
        // A repository with no commit has no `HEAD`: every tracked line is
        // new, which is the diff against the empty tree.
        let has_head = git()
            .args(["rev-parse", "--verify", "--quiet", "HEAD"])
            .output()
            .ok()
            .is_some_and(|output| output.status.success());
        if has_head {
            tracing::debug!(
                code = ?output.status.code(),
                "git diff falló; el archivo queda sin marcas de git"
            );
            return None;
        }
        // The hash of the empty tree depends on the repository's object
        // format (SHA-1 or SHA-256), so ask git instead of hard-coding it.
        let empty_tree = git()
            .args(["hash-object", "-t", "tree", "--stdin"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|hash| !hash.is_empty())?;
        output = diff(&empty_tree)?;
        if !output.status.success() {
            return None;
        }
    }
    parse_unified_zero(&String::from_utf8_lossy(&output.stdout))
}

/// Parses the output of `git diff -U0`: only the `@@ -a[,b] +c[,d] @@`
/// headers matter.
///
/// `b = 0` is an addition of `d` rows starting at row `c`; `d = 0` a deletion
/// of `b` lines right after row `c` (so the gap sits before zero-based row
/// `c`); anything else a modification of rows `c..c + d`. An omitted count is
/// 1. A binary diff gives `None`.
pub(crate) fn parse_unified_zero(text: &str) -> Option<LineDiff> {
    let mut hunks = Vec::new();
    for line in text.lines() {
        if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            return None;
        }
        let Some(header) = line.strip_prefix("@@ ") else {
            continue;
        };
        let Some((ranges, _)) = header.split_once(" @@") else {
            continue;
        };
        let mut parts = ranges.split(' ');
        let (Some(old), Some(new)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (Some(old), Some(new)) = (old.strip_prefix('-'), new.strip_prefix('+')) else {
            continue;
        };
        let (Some((_, old_len)), Some((new_start, new_len))) = (parse_range(old), parse_range(new))
        else {
            continue;
        };
        let hunk = if old_len == 0 {
            // `+c,d`: rows `c..c + d`, one-based.
            let start = new_start.saturating_sub(1);
            GitHunk {
                kind: GitHunkKind::Added,
                new_rows: start..start + new_len,
                old_len,
            }
        } else if new_len == 0 {
            // `+c,0`: the lines went away right after (one-based) row `c`,
            // that is right before zero-based row `c`.
            GitHunk {
                kind: GitHunkKind::Deleted,
                new_rows: new_start..new_start,
                old_len,
            }
        } else {
            let start = new_start.saturating_sub(1);
            GitHunk {
                kind: GitHunkKind::Modified,
                new_rows: start..start + new_len,
                old_len,
            }
        };
        hunks.push(hunk);
    }
    hunks.sort_by_key(|hunk| (hunk.new_rows.start, hunk.new_rows.end));
    Some(LineDiff { hunks })
}

/// `start[,count]` of a hunk header; the count defaults to 1.
fn parse_range(range: &str) -> Option<(u32, u32)> {
    match range.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}

/// Debounce of [`GitDirWatcher`] (`docs/specs/07-etapa5-productividad.md`
/// §6.2).
pub const GIT_DIR_DEBOUNCE: Duration = Duration::from_millis(300);

/// Something git did to its own directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GitDirEvent {
    /// The index, `HEAD` or a branch moved: a commit, `git add`, a checkout,
    /// a stash, a merge…
    Changed,
}

/// Watches a repository's git directory for the changes that move the
/// editor's gutter: the index, `HEAD`, `ORIG_HEAD`, `MERGE_HEAD`,
/// `packed-refs` and the branch refs under `refs/heads/`.
///
/// The git directory is watched non-recursively (objects and logs are not
/// interesting and are busy) and `refs/heads` recursively, through the
/// `notify` debouncer with [`GIT_DIR_DEBOUNCE`]. Only creations,
/// modifications, removals and renames count — never a plain read, so
/// running `git --no-optional-locks status` produces nothing — and `*.lock`
/// files are ignored: git writes `index.lock` and renames it over `index`,
/// and only the rename's destination is news.
///
/// The watch lives as long as this value.
pub struct GitDirWatcher {
    git_dir: PathBuf,
    _debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
}

impl GitDirWatcher {
    /// Starts watching `git_dir` (see [`git_dir`]).
    pub fn new(
        git_dir: impl Into<PathBuf>,
    ) -> Result<(GitDirWatcher, Receiver<GitDirEvent>), notify::Error> {
        GitDirWatcher::with_debounce(git_dir, GIT_DIR_DEBOUNCE)
    }

    /// Same, with an explicit debounce.
    pub fn with_debounce(
        git_dir: impl Into<PathBuf>,
        debounce: Duration,
    ) -> Result<(GitDirWatcher, Receiver<GitDirEvent>), notify::Error> {
        let git_dir = git_dir.into();
        let (sender, receiver) = async_channel::unbounded();
        let filter_dir = git_dir.clone();
        let mut debouncer =
            new_debouncer(
                debounce,
                None,
                move |result: DebounceEventResult| match result {
                    Ok(events) => {
                        let changed = events
                            .iter()
                            .any(|event| is_git_dir_change(&filter_dir, &event.event));
                        if changed && sender.try_send(GitDirEvent::Changed).is_err() {
                            tracing::debug!("nadie escucha los cambios de .git");
                        }
                    }
                    Err(errors) => {
                        for error in errors {
                            tracing::debug!(%error, "error observando .git");
                        }
                    }
                },
            )?;
        debouncer.watch(&git_dir, RecursiveMode::NonRecursive)?;
        let heads = git_dir.join("refs").join("heads");
        if heads.is_dir()
            && let Err(error) = debouncer.watch(&heads, RecursiveMode::Recursive)
        {
            tracing::debug!(%error, "no se pudo observar refs/heads");
        }
        Ok((
            GitDirWatcher {
                git_dir,
                _debouncer: debouncer,
            },
            receiver,
        ))
    }

    /// The watched git directory.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }
}

/// Whether a `notify` event is one the gutter must react to.
fn is_git_dir_change(git_dir: &Path, event: &notify::Event) -> bool {
    let relevant_kind = matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    );
    relevant_kind
        && event
            .paths
            .iter()
            .any(|path| is_git_dir_path(git_dir, path))
}

/// Whether `path` is one of the files of the git directory that move `HEAD`
/// or the index.
fn is_git_dir_path(git_dir: &Path, path: &Path) -> bool {
    if path
        .extension()
        .is_some_and(|extension| extension == "lock")
    {
        return false;
    }
    let Ok(relative) = path.strip_prefix(git_dir) else {
        return false;
    };
    if relative.starts_with(Path::new("refs").join("heads")) {
        return true;
    }
    let mut components = relative.components();
    match (components.next(), components.next()) {
        (Some(name), None) => matches!(
            name.as_os_str().to_str(),
            Some("HEAD" | "index" | "packed-refs" | "ORIG_HEAD" | "MERGE_HEAD")
        ),
        _ => false,
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
        git(&["config", "user.email", "test@cincel"])?;
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

    /// Runs git in `dir`, `None` when it fails.
    fn run(dir: &Path, args: &[&str]) -> Option<std::process::Output> {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
    }

    /// A repository whose `HEAD` holds `name` with `contents`.
    fn repo_with(name: &str, contents: &[u8]) -> Option<tempfile::TempDir> {
        let dir = repo()?;
        std::fs::write(dir.path().join(name), contents).unwrap();
        run(dir.path(), &["add", "."])?;
        run(dir.path(), &["commit", "-qm", "archivo"])?;
        Some(dir)
    }

    fn numbered(lines: std::ops::RangeInclusive<u32>) -> Vec<String> {
        lines.map(|n| format!("línea {n}")).collect()
    }

    fn hunk(kind: GitHunkKind, rows: Range<u32>, old_len: u32) -> GitHunk {
        GitHunk {
            kind,
            new_rows: rows,
            old_len,
        }
    }

    #[test]
    fn parses_the_zero_context_headers() {
        let text = "diff --git a/x b/x\n\
                    index 111..222 100644\n\
                    --- a/x\n\
                    +++ b/x\n\
                    @@ -3 +3 @@ fn uno()\n\
                    -viejo\n\
                    +nuevo\n\
                    @@ -10,0 +11,2 @@\n\
                    +a\n\
                    +b\n\
                    @@ -20 +21,0 @@\n\
                    -borrada\n\
                    @@ -30,3 +30 @@\n\
                    -x\n\
                    -y\n\
                    -z\n\
                    +w\n";
        let diff = parse_unified_zero(text).unwrap();
        assert_eq!(
            diff.hunks,
            vec![
                hunk(GitHunkKind::Modified, 2..3, 1),
                hunk(GitHunkKind::Added, 10..12, 0),
                hunk(GitHunkKind::Deleted, 21..21, 1),
                hunk(GitHunkKind::Modified, 29..30, 3),
            ]
        );
    }

    #[test]
    fn parses_the_edge_headers() {
        // A new file (in the index, not in HEAD): `-0,0`.
        let diff = parse_unified_zero("@@ -0,0 +1,3 @@\n+a\n+b\n+c\n").unwrap();
        assert_eq!(diff.hunks, vec![hunk(GitHunkKind::Added, 0..3, 0)]);
        // Lines deleted at the very top: the gap sits before row 0.
        let diff = parse_unified_zero("@@ -1,2 +0,0 @@\n-a\n-b\n").unwrap();
        assert_eq!(diff.hunks, vec![hunk(GitHunkKind::Deleted, 0..0, 2)]);
        // A missing final newline is a line of its own in the body, not a
        // header.
        let diff =
            parse_unified_zero("@@ -2 +2 @@\n-b\n\\ No newline at end of file\n+b\n").unwrap();
        assert_eq!(diff.hunks, vec![hunk(GitHunkKind::Modified, 1..2, 1)]);
        // No change at all.
        assert_eq!(parse_unified_zero(""), Some(LineDiff::default()));
        // Binary.
        assert_eq!(
            parse_unified_zero("diff --git a/x b/x\nBinary files a/x and b/x differ\n"),
            None
        );
        // Garbage headers are skipped, not fatal.
        let diff = parse_unified_zero("@@ -a +b @@\n@@ -1 +1 @@\n").unwrap();
        assert_eq!(diff.hunks, vec![hunk(GitHunkKind::Modified, 0..1, 1)]);
    }

    /// `docs/specs/07-etapa5-productividad.md` §6.4: modify line 3, add two
    /// lines after line 10 and delete (the original) line 20.
    #[test]
    fn diffs_a_working_copy_against_head() {
        let original = numbered(1..=25).join("\n") + "\n";
        let Some(dir) = repo_with("f.txt", original.as_bytes()) else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let mut lines = numbered(1..=25);
        lines[2] = "línea 3 cambiada".to_owned();
        lines.remove(19);
        lines.insert(10, "nueva a".to_owned());
        lines.insert(11, "nueva b".to_owned());
        let path = dir.path().join("f.txt");
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();

        let diff = diff_against_head(dir.path(), &path).expect("hay diff");
        assert_eq!(
            diff.hunks,
            vec![
                // Blue on line 3, green on 11–12.
                hunk(GitHunkKind::Modified, 2..3, 1),
                hunk(GitHunkKind::Added, 10..12, 0),
                // The old line 20 sat between the old 19 and 21, now (with
                // the two added lines) 21 and 22: the gap is before
                // zero-based row 21.
                hunk(GitHunkKind::Deleted, 21..21, 1),
            ]
        );

        // A clean file is an empty diff, not `None`.
        std::fs::write(&path, &original).unwrap();
        assert_eq!(
            diff_against_head(dir.path(), &path),
            Some(LineDiff::default())
        );
    }

    #[test]
    fn untracked_files_and_folders_without_git_have_no_diff() {
        let Some(dir) = repo() else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let untracked = dir.path().join("nuevo.txt");
        std::fs::write(&untracked, "hola\n").unwrap();
        assert_eq!(diff_against_head(dir.path(), &untracked), None);

        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("a.txt");
        std::fs::write(&file, "hola\n").unwrap();
        // A temp dir is normally outside any repository.
        if git_dir(outside.path()).is_none() {
            assert_eq!(diff_against_head(outside.path(), &file), None);
        }
    }

    #[test]
    fn a_file_added_to_the_index_but_not_to_head_is_all_added() {
        let Some(dir) = repo() else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let path = dir.path().join("nuevo.txt");
        std::fs::write(&path, "a\nb\nc\n").unwrap();
        run(dir.path(), &["add", "nuevo.txt"]).unwrap();
        let diff = diff_against_head(dir.path(), &path).unwrap();
        assert_eq!(diff.hunks, vec![hunk(GitHunkKind::Added, 0..3, 0)]);
    }

    #[test]
    fn a_repository_without_commits_diffs_against_the_empty_tree() {
        let dir = tempfile::tempdir().unwrap();
        if run(dir.path(), &["init", "-q"]).is_none() {
            eprintln!("sin git utilizable; se omite");
            return;
        }
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "uno\ndos\n").unwrap();
        // Not added yet: untracked.
        assert_eq!(diff_against_head(dir.path(), &path), None);
        run(dir.path(), &["add", "a.txt"]).unwrap();
        let diff = diff_against_head(dir.path(), &path).unwrap();
        assert_eq!(diff.hunks, vec![hunk(GitHunkKind::Added, 0..2, 0)]);
    }

    #[test]
    fn a_deleted_file_and_pure_deletions() {
        let Some(dir) = repo_with("f.txt", b"a\nb\nc\nd\n") else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let path = dir.path().join("f.txt");
        // Only deletions: the first two lines, then the last one.
        std::fs::write(&path, "c\n").unwrap();
        let diff = diff_against_head(dir.path(), &path).unwrap();
        assert_eq!(
            diff.hunks,
            vec![
                hunk(GitHunkKind::Deleted, 0..0, 2),
                hunk(GitHunkKind::Deleted, 1..1, 1),
            ]
        );
        // The file is gone from disk but still tracked: everything deleted,
        // at the top.
        std::fs::remove_file(&path).unwrap();
        let diff = diff_against_head(dir.path(), &path).unwrap();
        assert_eq!(diff.hunks, vec![hunk(GitHunkKind::Deleted, 0..0, 4)]);
    }

    #[test]
    fn a_missing_final_newline_is_a_modified_last_line() {
        let Some(dir) = repo_with("f.txt", b"a\nb") else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let path = dir.path().join("f.txt");
        // `b` gains the newline it lacked, and `c` follows.
        std::fs::write(&path, "a\nb\nc").unwrap();
        let diff = diff_against_head(dir.path(), &path).unwrap();
        assert_eq!(diff.hunks, vec![hunk(GitHunkKind::Modified, 1..3, 1)]);
    }

    #[test]
    fn binary_files_have_no_diff() {
        let Some(dir) = repo_with("b.bin", b"\0\x01\x02binario\0") else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let path = dir.path().join("b.bin");
        std::fs::write(&path, b"\0\x03otro\0").unwrap();
        assert_eq!(diff_against_head(dir.path(), &path), None);
    }

    #[test]
    fn paths_are_literal_and_may_be_absolute_outside_the_root() {
        let Some(dir) = repo_with("[x]*.txt", b"uno\n") else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let path = dir.path().join("[x]*.txt");
        std::fs::write(&path, "uno\ndos\n").unwrap();
        let diff = diff_against_head(dir.path(), &path).unwrap();
        assert_eq!(diff.hunks, vec![hunk(GitHunkKind::Added, 1..2, 0)]);
        // From a subdirectory used as the root: the pathspec is resolved
        // from there.
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let nested = dir.path().join("sub/n.txt");
        std::fs::write(&nested, "a\n").unwrap();
        run(dir.path(), &["add", "."]).unwrap();
        run(dir.path(), &["commit", "-qm", "sub"]).unwrap();
        std::fs::write(&nested, "b\n").unwrap();
        let diff = diff_against_head(&dir.path().join("sub"), &nested).unwrap();
        assert_eq!(diff.hunks, vec![hunk(GitHunkKind::Modified, 0..1, 1)]);
    }

    #[test]
    fn is_tracked_follows_the_status() {
        let Some(dir) = repo() else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        std::fs::write(dir.path().join("nuevo.txt"), "x").unwrap();
        let status = GitStatus::collect(dir.path());
        let root = status.repo_root().unwrap().to_path_buf();
        assert!(status.is_tracked(root.join("a.txt")));
        assert!(!status.is_tracked(root.join("nuevo.txt")));
        assert!(!status.is_tracked(Path::new("/fuera/del/repo.txt")));
        assert!(!GitStatus::default().is_tracked(root.join("a.txt")));
    }

    /// D10: `git status` must not rewrite `.git/index`, or the `.git`
    /// watcher would see its own refresh and loop forever.
    #[test]
    fn status_does_not_rewrite_the_index() {
        let Some(dir) = repo() else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let index = git_dir(dir.path()).unwrap().join("index");
        // Rewrite a tracked file with the same bytes: its stat information no
        // longer matches the index, which is exactly when a plain
        // `git status` refreshes (and rewrites) the index.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.path().join("a.txt"), "uno\n").unwrap();
        let before = std::fs::read(&index).unwrap();
        let modified_before = std::fs::metadata(&index).unwrap().modified().unwrap();

        let status = GitStatus::collect(dir.path());
        assert!(status.repo_root().is_some());
        let _ = diff_against_head(dir.path(), &dir.path().join("a.txt"));

        assert_eq!(
            std::fs::read(&index).unwrap(),
            before,
            "se reescribió el índice"
        );
        assert_eq!(
            std::fs::metadata(&index).unwrap().modified().unwrap(),
            modified_before,
            "se tocó el índice"
        );
    }

    fn wait_for_git_dir_event(events: &Receiver<GitDirEvent>, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if events.try_recv().is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn the_git_dir_watcher_sees_commits_but_not_status() {
        let Some(dir) = repo() else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let git_dir = git_dir(dir.path()).unwrap();
        let (watcher, events) =
            GitDirWatcher::with_debounce(&git_dir, Duration::from_millis(50)).unwrap();
        assert_eq!(watcher.git_dir(), git_dir);
        // Give the backend a moment to arm the watches.
        std::thread::sleep(Duration::from_millis(100));

        // Reading the status (even with a stale index) is silent.
        std::fs::write(dir.path().join("a.txt"), "uno\n").unwrap();
        for _ in 0..3 {
            let _ = GitStatus::collect(dir.path());
        }
        assert!(
            !wait_for_git_dir_event(&events, Duration::from_millis(500)),
            "git status disparó el vigilante de .git"
        );

        // A commit made "in the terminal" is reported.
        std::fs::write(dir.path().join("a.txt"), "uno cambiado\n").unwrap();
        run(dir.path(), &["commit", "-qam", "cambio"]).unwrap();
        assert!(
            wait_for_git_dir_event(&events, Duration::from_secs(5)),
            "no llegó el aviso del commit"
        );
        // So is a `git add`.
        while events.try_recv().is_ok() {}
        std::fs::write(dir.path().join("b.txt"), "dos cambiado\n").unwrap();
        run(dir.path(), &["add", "b.txt"]).unwrap();
        assert!(
            wait_for_git_dir_event(&events, Duration::from_secs(5)),
            "no llegó el aviso del git add"
        );
    }

    /// §6.4: a `git commit -am` made outside Cincel clears the bars in less
    /// than a second — the watcher's debounce plus a new diff.
    #[test]
    fn a_commit_clears_the_diff_in_under_a_second() {
        let Some(dir) = repo_with("f.txt", b"uno\ndos\n") else {
            eprintln!("sin git utilizable; se omite");
            return;
        };
        let path = dir.path().join("f.txt");
        std::fs::write(&path, "uno\ndos cambiado\n").unwrap();
        assert_eq!(
            diff_against_head(dir.path(), &path).unwrap().hunks,
            vec![hunk(GitHunkKind::Modified, 1..2, 1)]
        );
        let (_watcher, events) = GitDirWatcher::new(git_dir(dir.path()).unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(100));

        run(dir.path(), &["commit", "-qam", "desde la terminal"]).unwrap();
        let started = Instant::now();
        assert!(wait_for_git_dir_event(&events, Duration::from_secs(5)));
        let diff = diff_against_head(dir.path(), &path).unwrap();
        let elapsed = started.elapsed();
        assert!(diff.is_empty(), "{diff:?}");
        assert!(elapsed < Duration::from_secs(1), "tardó {elapsed:?}");
    }

    #[test]
    fn filters_the_git_dir_paths() {
        let git_dir = Path::new("/r/.git");
        for interesting in [
            "/r/.git/HEAD",
            "/r/.git/index",
            "/r/.git/packed-refs",
            "/r/.git/ORIG_HEAD",
            "/r/.git/MERGE_HEAD",
            "/r/.git/refs/heads/main",
            "/r/.git/refs/heads/feature/x",
        ] {
            assert!(
                is_git_dir_path(git_dir, Path::new(interesting)),
                "{interesting}"
            );
        }
        for boring in [
            "/r/.git/index.lock",
            "/r/.git/HEAD.lock",
            "/r/.git/refs/heads/main.lock",
            "/r/.git/objects/ab/cdef",
            "/r/.git/logs/HEAD",
            "/r/.git/FETCH_HEAD",
            "/r/.git/COMMIT_EDITMSG",
            "/r/src/index",
        ] {
            assert!(!is_git_dir_path(git_dir, Path::new(boring)), "{boring}");
        }
        use notify::event::{AccessKind, CreateKind, Event, ModifyKind, RenameMode};
        let access =
            Event::new(EventKind::Access(AccessKind::Read)).add_path("/r/.git/index".into());
        assert!(!is_git_dir_change(git_dir, &access));
        let rename = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path("/r/.git/index.lock".into())
            .add_path("/r/.git/index".into());
        assert!(is_git_dir_change(git_dir, &rename));
        let lock =
            Event::new(EventKind::Create(CreateKind::File)).add_path("/r/.git/index.lock".into());
        assert!(!is_git_dir_change(git_dir, &lock));
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
