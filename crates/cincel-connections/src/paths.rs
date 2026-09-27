//! Where everything lives (`docs/specs/06-etapa4-conexiones-y-cincel.md`
//! §2-§3). Every directory hangs off two roots so tests can point both at a
//! temp dir.

use std::path::{Path, PathBuf};

use crate::error::{Result, io_err};

/// The two roots Cincel's connection data lives under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CincelPaths {
    /// `~/.local/share/cincel`: index, profiles, runtime, adapters.
    pub data_dir: PathBuf,
    /// `~/.cache/cincel`: npm cache and partial downloads.
    pub cache_dir: PathBuf,
}

impl CincelPaths {
    /// The XDG defaults: `$XDG_DATA_HOME/cincel` and `$XDG_CACHE_HOME/cincel`.
    #[must_use]
    pub fn from_xdg() -> Self {
        Self {
            data_dir: dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("cincel"),
            cache_dir: dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("cincel"),
        }
    }

    /// Both roots under one directory (tests): `<root>/data`, `<root>/cache`.
    #[must_use]
    pub fn under(root: &Path) -> Self {
        Self {
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
        }
    }

    /// `<data>/connections.json`.
    #[must_use]
    pub fn index_file(&self) -> PathBuf {
        self.data_dir.join("connections.json")
    }

    /// `<data>/connections/`: one profile directory per connection.
    #[must_use]
    pub fn connections_dir(&self) -> PathBuf {
        self.data_dir.join("connections")
    }

    /// `<data>/runtime/`: private Node installations.
    #[must_use]
    pub fn runtime_dir(&self) -> PathBuf {
        self.data_dir.join("runtime")
    }

    /// `<data>/agents/`: installed adapters.
    #[must_use]
    pub fn agents_dir(&self) -> PathBuf {
        self.data_dir.join("agents")
    }

    /// `<cache>/npm`: npm cache used for adapter installs (never the
    /// system one).
    #[must_use]
    pub fn npm_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("npm")
    }

    /// `<cache>/downloads`: partial runtime downloads (resumable).
    #[must_use]
    pub fn downloads_dir(&self) -> PathBuf {
        self.cache_dir.join("downloads")
    }
}

/// Create `dir` (and parents) and restrict it to the owner (0700).
pub(crate) fn create_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    set_mode(dir, 0o700)
}

#[cfg(unix)]
pub(crate) fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(io_err(path))
}

#[cfg(not(unix))]
pub(crate) fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// Write `bytes` to `path` atomically (temp file in the same directory, then
/// rename), with owner-only permissions.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(io_err(parent))?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes).map_err(io_err(&tmp))?;
    set_mode(&tmp, 0o600)?;
    std::fs::rename(&tmp, path).map_err(io_err(path))
}

/// Remove `target` recursively only if it canonicalizes to a path strictly
/// inside `root` (spec 06 §4 F3). A missing `target` is not an error.
///
/// # Errors
///
/// [`crate::ConnectionsError::UnsafePath`] when `target` escapes `root` (or
/// is `root` itself), I/O errors otherwise.
pub fn remove_dir_within(root: &Path, target: &Path) -> Result<bool> {
    if !target.exists() && target.symlink_metadata().is_err() {
        return Ok(false);
    }
    let root = root.canonicalize().map_err(io_err(root))?;
    // A symlink named like a profile must not redirect the delete: check the
    // link itself, not what it points to.
    if target
        .symlink_metadata()
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(crate::ConnectionsError::UnsafePath {
            path: target.to_path_buf(),
        });
    }
    let canonical = target.canonicalize().map_err(io_err(target))?;
    if canonical == root || !canonical.starts_with(&root) {
        return Err(crate::ConnectionsError::UnsafePath {
            path: target.to_path_buf(),
        });
    }
    std::fs::remove_dir_all(&canonical).map_err(io_err(&canonical))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_the_spec() {
        let paths = CincelPaths::under(Path::new("/r"));
        assert_eq!(paths.index_file(), Path::new("/r/data/connections.json"));
        assert_eq!(paths.connections_dir(), Path::new("/r/data/connections"));
        assert_eq!(paths.runtime_dir(), Path::new("/r/data/runtime"));
        assert_eq!(paths.agents_dir(), Path::new("/r/data/agents"));
        assert_eq!(paths.npm_cache_dir(), Path::new("/r/cache/npm"));
        let xdg = CincelPaths::from_xdg();
        assert!(xdg.data_dir.ends_with("cincel"));
        assert!(xdg.cache_dir.ends_with("cincel"));
    }

    #[test]
    fn remove_dir_within_refuses_escapes_and_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("connections");
        let inside = root.join("abc");
        let outside = dir.path().join("fuera");
        std::fs::create_dir_all(&inside).expect("mkdir");
        std::fs::create_dir_all(&outside).expect("mkdir");

        assert!(matches!(
            remove_dir_within(&root, &root.join("..").join("fuera")),
            Err(crate::ConnectionsError::UnsafePath { .. })
        ));
        assert!(outside.exists());
        assert!(matches!(
            remove_dir_within(&root, &root),
            Err(crate::ConnectionsError::UnsafePath { .. })
        ));
        assert!(remove_dir_within(&root, &inside).expect("remove"));
        assert!(!inside.exists());
        assert!(!remove_dir_within(&root, &inside).expect("ya no existe"));
    }

    #[cfg(unix)]
    #[test]
    fn remove_dir_within_refuses_symlinks_out_of_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("connections");
        let outside = dir.path().join("secreto");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::create_dir_all(&outside).expect("mkdir");
        std::fs::write(outside.join("dato"), "x").expect("write");
        let link = root.join("enlace");
        std::os::unix::fs::symlink(&outside, &link).expect("symlink");
        assert!(matches!(
            remove_dir_within(&root, &link),
            Err(crate::ConnectionsError::UnsafePath { .. })
        ));
        assert!(outside.join("dato").exists());
    }

    #[cfg(unix)]
    #[test]
    fn private_dirs_and_atomic_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let private = dir.path().join("p");
        create_private_dir(&private).expect("mkdir");
        let mode = std::fs::metadata(&private)
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        let file = private.join("f.json");
        write_atomic(&file, b"{}").expect("write");
        let mode = std::fs::metadata(&file).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read(&file).expect("read"), b"{}");
    }
}
