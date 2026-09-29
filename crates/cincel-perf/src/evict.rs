//! Cold start without root (D4): drops an app's own files from the page
//! cache with `posix_fadvise(POSIX_FADV_DONTNEED)`. The system libraries are
//! left warm for every app alike.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rustix::fs::{Advice, fadvise};

/// What an eviction touched.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Evicted {
    pub files: usize,
    pub bytes: u64,
    /// Files that could not be opened or advised (reported, not fatal).
    pub errors: usize,
}

/// Evicts every regular file under `paths` (files or folders, recursively;
/// symbolic links are not followed, so nothing outside the app is touched).
pub fn evict(paths: &[PathBuf]) -> Result<Evicted> {
    let mut evicted = Evicted::default();
    // The given paths themselves may be links (`~/.local/bin/zed`): resolve
    // them once; links found while walking are skipped.
    let mut pending: Vec<PathBuf> = paths
        .iter()
        .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
        .collect();
    while let Some(path) = pending.pop() {
        let metadata =
            fs::symlink_metadata(&path).with_context(|| format!("leyendo {}", path.display()))?;
        if metadata.is_dir() {
            for entry in
                fs::read_dir(&path).with_context(|| format!("leyendo {}", path.display()))?
            {
                pending.push(entry?.path());
            }
        } else if metadata.is_file() {
            match evict_file(&path) {
                Ok(()) => {
                    evicted.files += 1;
                    evicted.bytes += metadata.len();
                }
                Err(_) => evicted.errors += 1,
            }
        }
    }
    Ok(evicted)
}

fn evict_file(path: &Path) -> std::io::Result<()> {
    let file = fs::File::open(path)?;
    fadvise(&file, 0, None, Advice::DontNeed)?;
    Ok(())
}
