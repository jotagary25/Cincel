//! What the project tree leaves out.
//!
//! Two sources decide it: the `files.exclude` globs from the settings
//! ([`ExcludeSet`]) and the repository's `.gitignore` files ([`IgnoreRules`]).
//!
//! During the initial scan the `ignore` crate applies both itself — that is
//! what [`ignore::WalkBuilder`] is for. [`IgnoreRules`] exists for the other
//! half of the job: deciding, one path at a time, whether a file system event
//! is about something the tree cares about. It caches one
//! [`Gitignore`](ignore::gitignore::Gitignore) per directory, built lazily,
//! and consults them from the root down so a nested `.gitignore` can
//! re-include what its parent excluded, the way git does.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use parking_lot::Mutex;

/// The `files.exclude` globs, compiled.
///
/// Patterns are matched against the path **relative to the project root**,
/// with `/` separators, the same way they are written in the settings
/// (`**/target`, `**/node_modules`).
#[derive(Clone, Debug, Default)]
pub struct ExcludeSet {
    set: Option<GlobSet>,
    patterns: Vec<String>,
}

impl ExcludeSet {
    /// Compiles `patterns`, returning the set and the patterns it had to
    /// drop, so the caller can report them without failing to open the
    /// project.
    pub fn new<S: AsRef<str>>(patterns: &[S]) -> (ExcludeSet, Vec<String>) {
        let mut builder = GlobSetBuilder::new();
        let mut kept = Vec::new();
        let mut rejected = Vec::new();
        for pattern in patterns {
            let pattern = pattern.as_ref();
            match Glob::new(pattern) {
                Ok(glob) => {
                    builder.add(glob);
                    // `**/target` must also match `target` at the root, which
                    // is how gitignore reads it but not how globset does.
                    if let Some(stripped) = pattern.strip_prefix("**/")
                        && let Ok(glob) = Glob::new(stripped)
                    {
                        builder.add(glob);
                    }
                    kept.push(pattern.to_owned());
                }
                Err(error) => rejected.push(format!("«{pattern}» no es un patrón válido: {error}")),
            }
        }
        if kept.is_empty() {
            return (ExcludeSet::default(), rejected);
        }
        match builder.build() {
            Ok(set) => (
                ExcludeSet {
                    set: Some(set),
                    patterns: kept,
                },
                rejected,
            ),
            Err(error) => {
                rejected.push(format!("no se pudo compilar «files.exclude»: {error}"));
                (ExcludeSet::default(), rejected)
            }
        }
    }

    /// Whether `relative` (a path relative to the project root) is excluded.
    ///
    /// A directory excludes everything below it, so every ancestor is checked
    /// too: `**/target` hides `target/debug/build.rs` without a glob for it.
    pub fn matches(&self, relative: &Path) -> bool {
        let Some(set) = &self.set else {
            return false;
        };
        let mut current = PathBuf::new();
        for component in relative.components() {
            if !matches!(component, Component::Normal(_)) {
                continue;
            }
            current.push(component.as_os_str());
            if set.is_match(&current) {
                return true;
            }
        }
        false
    }

    /// The patterns that compiled.
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// Whether there is nothing to exclude.
    pub fn is_empty(&self) -> bool {
        self.set.is_none()
    }
}

/// Everything needed to answer "does the tree show this path?".
#[derive(Debug)]
pub struct IgnoreRules {
    root: PathBuf,
    excludes: ExcludeSet,
    respect_gitignore: bool,
    show_hidden: bool,
    /// One `.gitignore` matcher per directory, built on first use.
    cache: Mutex<HashMap<PathBuf, Arc<Gitignore>>>,
}

impl IgnoreRules {
    /// Rules for `root`.
    pub fn new(
        root: impl Into<PathBuf>,
        excludes: ExcludeSet,
        respect_gitignore: bool,
        show_hidden: bool,
    ) -> Self {
        Self {
            root: root.into(),
            excludes,
            respect_gitignore,
            show_hidden,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The project root these rules are about.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The compiled `files.exclude` globs.
    pub fn excludes(&self) -> &ExcludeSet {
        &self.excludes
    }

    /// Whether `path` (absolute, or relative to the root) is hidden from the
    /// tree.
    ///
    /// `is_dir` matters: gitignore patterns ending in `/` only match
    /// directories.
    pub fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        let Some(relative) = self.relative(path) else {
            // Outside the project: not ours to show.
            return true;
        };
        if relative.as_os_str().is_empty() {
            return false;
        }
        if self.excludes.matches(&relative) {
            return true;
        }
        if !self.show_hidden && is_hidden(&relative) {
            return true;
        }
        if self.respect_gitignore && self.is_gitignored(&relative, is_dir) {
            return true;
        }
        false
    }

    /// `path` relative to the root, or `None` when it is outside it.
    pub fn relative(&self, path: &Path) -> Option<PathBuf> {
        if path.is_absolute() {
            path.strip_prefix(&self.root).ok().map(Path::to_path_buf)
        } else {
            Some(path.to_path_buf())
        }
    }

    /// Consults every `.gitignore` from the root down to the file's parent.
    ///
    /// The deepest matching rule wins, and a `!` rule in a nested file
    /// re-includes a path its parent excluded, which is git's own precedence.
    fn is_gitignored(&self, relative: &Path, is_dir: bool) -> bool {
        let absolute = self.root.join(relative);
        let mut dir = self.root.clone();
        let mut ignored = false;
        loop {
            let matcher = self.gitignore_for(&dir);
            match matcher.matched_path_or_any_parents(&absolute, is_dir) {
                Match::Ignore(_) => ignored = true,
                Match::Whitelist(_) => ignored = false,
                Match::None => {}
            }
            // Walk one directory closer to the file.
            let Ok(rest) = absolute.strip_prefix(&dir) else {
                break;
            };
            let mut components = rest.components();
            let Some(next) = components.next() else {
                break;
            };
            if components.next().is_none() {
                // `next` is the file itself; there is no deeper directory.
                break;
            }
            dir.push(next.as_os_str());
        }
        ignored
    }

    /// The (cached) matcher for `dir`'s own `.gitignore`.
    fn gitignore_for(&self, dir: &Path) -> Arc<Gitignore> {
        if let Some(matcher) = self.cache.lock().get(dir) {
            return matcher.clone();
        }
        let mut builder = GitignoreBuilder::new(dir);
        let file = dir.join(".gitignore");
        if file.exists()
            && let Some(error) = builder.add(&file)
        {
            tracing::debug!(path = %file.display(), %error, "no se pudo leer .gitignore");
        }
        if dir == self.root {
            // `.git` is not in any `.gitignore`, but it is never part of the
            // tree either.
            let _ = builder.add_line(None, ".git/");
        }
        let matcher = Arc::new(builder.build().unwrap_or_else(|_| Gitignore::empty()));
        self.cache.lock().insert(dir.to_path_buf(), matcher.clone());
        matcher
    }

    /// Forgets the cached `.gitignore` matchers, after one of them changed.
    pub fn invalidate(&self) {
        self.cache.lock().clear();
    }
}

/// Whether any component of `relative` starts with a dot.
fn is_hidden(relative: &Path) -> bool {
    relative.components().any(|component| {
        matches!(component, Component::Normal(name)
            if name.to_string_lossy().starts_with('.'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_match_at_any_depth() {
        let (excludes, rejected) = ExcludeSet::new(&["**/.git", "**/target", "**/node_modules"]);
        assert!(rejected.is_empty());
        assert!(excludes.matches(Path::new("target")));
        assert!(excludes.matches(Path::new("crates/x/target")));
        assert!(excludes.matches(Path::new(".git")));
        assert!(excludes.matches(Path::new("web/node_modules")));
        // Everything below an excluded directory is excluded too.
        assert!(excludes.matches(Path::new("target/debug/build.rs")));
        assert!(!excludes.matches(Path::new("src/main.rs")));
        assert!(!excludes.matches(Path::new("targets.txt")));
    }

    #[test]
    fn broken_globs_are_reported_not_fatal() {
        let (excludes, rejected) = ExcludeSet::new(&["**/target", "a[b"]);
        assert_eq!(rejected.len(), 1);
        assert!(rejected[0].contains("a[b"));
        assert!(excludes.matches(Path::new("target")));
    }

    #[test]
    fn empty_set_matches_nothing() {
        let (excludes, rejected) = ExcludeSet::new::<String>(&[]);
        assert!(rejected.is_empty());
        assert!(excludes.is_empty());
        assert!(!excludes.matches(Path::new("target")));
    }

    #[test]
    fn gitignore_is_respected_with_nesting() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "*.log\nbuild/\n").unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/.gitignore"), "!importante.log\n").unwrap();

        let (excludes, _) = ExcludeSet::new(&["**/.git"]);
        let rules = IgnoreRules::new(root, excludes, true, true);
        assert!(rules.is_ignored(&root.join("a.log"), false));
        assert!(rules.is_ignored(&root.join("build"), true));
        assert!(rules.is_ignored(&root.join("build/x.o"), false));
        assert!(!rules.is_ignored(&root.join("src/main.rs"), false));
        assert!(!rules.is_ignored(&root.join("sub/importante.log"), false));
        assert!(rules.is_ignored(&root.join("sub/otro.log"), false));
        // `.git` is never part of the tree.
        assert!(rules.is_ignored(&root.join(".git/HEAD"), false));
    }

    #[test]
    fn gitignore_can_be_turned_off_and_hidden_files_shown() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        let (excludes, _) = ExcludeSet::new(&["**/.git"]);

        let rules = IgnoreRules::new(root, excludes.clone(), false, true);
        assert!(!rules.is_ignored(&root.join("a.log"), false));
        assert!(!rules.is_ignored(&root.join(".env"), false));

        let strict = IgnoreRules::new(root, excludes, false, false);
        assert!(strict.is_ignored(&root.join(".env"), false));
        assert!(strict.is_ignored(&root.join(".config/x"), false));
    }

    #[test]
    fn paths_outside_the_project_are_ignored() {
        let (excludes, _) = ExcludeSet::new(&["**/.git"]);
        let rules = IgnoreRules::new("/proyecto", excludes, true, true);
        assert!(rules.is_ignored(Path::new("/otro/archivo.rs"), false));
        assert!(!rules.is_ignored(Path::new("/proyecto"), true));
    }
}
