//! One-time migration of the XDG directories from the old `asteroid` name to
//! `cincel` (`docs/specs/06-etapa4-conexiones-y-cincel.md` §7).
//!
//! Must run before anything else in the process reads configuration, state,
//! data or cache: [`migrate_xdg_dirs`] is the first thing `main` calls.

use std::path::{Path, PathBuf};

/// The name every directory used to have.
const OLD_APP_DIR: &str = "asteroid";
/// The name every directory has now (`crate::paths::APP_DIR`).
const NEW_APP_DIR: &str = "cincel";

/// One of the four XDG base directories Cincel keeps something under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XdgRoot {
    /// `~/.config`.
    Config,
    /// `~/.local/state`.
    State,
    /// `~/.local/share`.
    Data,
    /// `~/.cache`.
    Cache,
}

impl XdgRoot {
    /// Every root, in the order `migrate_xdg_dirs` checks them.
    const ALL: [XdgRoot; 4] = [
        XdgRoot::Config,
        XdgRoot::State,
        XdgRoot::Data,
        XdgRoot::Cache,
    ];

    /// The real base directory (honours `$XDG_*_HOME`, like the rest of the
    /// application), or `None` on a machine with no home directory.
    fn base(self) -> Option<PathBuf> {
        match self {
            XdgRoot::Config => dirs::config_dir(),
            XdgRoot::State => dirs::state_dir(),
            XdgRoot::Data => dirs::data_dir(),
            XdgRoot::Cache => dirs::cache_dir(),
        }
    }

    /// Name used in log fields.
    fn label(self) -> &'static str {
        match self {
            XdgRoot::Config => "config",
            XdgRoot::State => "state",
            XdgRoot::Data => "data",
            XdgRoot::Cache => "cache",
        }
    }
}

/// How one root's directory ended up migrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// A plain `rename` (2), same filesystem: the common case.
    Renamed,
    /// `rename` failed (typically `EXDEV`, a different filesystem); the
    /// directory was copied instead and the original left in place.
    Copied,
}

/// One root Cincel migrated on this run.
#[derive(Debug, Clone)]
pub struct MigratedRoot {
    /// Which of the four roots.
    pub root: XdgRoot,
    /// Old path (`<root>/asteroid`).
    pub old: PathBuf,
    /// New path (`<root>/cincel`).
    pub new: PathBuf,
    /// Rename or copy.
    pub outcome: MigrationOutcome,
}

/// Migrates every XDG root's `asteroid` directory to `cincel`, once.
///
/// A root is skipped when `cincel` already exists there (idempotent: a
/// second run, or a run after a partial migration, does nothing more) or
/// when `asteroid` does not exist (a fresh install). The `config` root is
/// also skipped when `$CINCEL_CONFIG_DIR` is set: that variable points
/// `Paths::resolve` at an arbitrary directory instead of XDG, so there is no
/// `asteroid` sibling to move.
///
/// Never fails: a root that cannot be migrated (no home directory, a
/// permission error, an unreadable entry) is logged and skipped, and the
/// editor starts regardless (`docs/specs/06-etapa4-conexiones-y-cincel.md` §7
/// reuses the "nothing here is fatal" rule of `modulos/settings.md`).
pub fn migrate_xdg_dirs() -> Vec<MigratedRoot> {
    XdgRoot::ALL
        .iter()
        .filter_map(|&root| migrate_root(root))
        .collect()
}

fn migrate_root(root: XdgRoot) -> Option<MigratedRoot> {
    if root == XdgRoot::Config && std::env::var_os("CINCEL_CONFIG_DIR").is_some() {
        return None;
    }
    let base = root.base()?;
    let old = base.join(OLD_APP_DIR);
    let new = base.join(NEW_APP_DIR);
    match migrate_dir(&old, &new) {
        Ok(Some(outcome)) => {
            tracing::info!(
                root = root.label(),
                old = %old.display(),
                new = %new.display(),
                ?outcome,
                "Migrada la configuración de Asteroid a Cincel"
            );
            Some(MigratedRoot {
                root,
                old,
                new,
                outcome,
            })
        }
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(
                root = root.label(),
                old = %old.display(),
                new = %new.display(),
                %error,
                "no se pudo migrar la configuración de Asteroid a Cincel"
            );
            None
        }
    }
}

/// Migrates one `old` → `new` directory pair: `None` when there is nothing to
/// do (idempotent), `Some` with how it went otherwise.
///
/// Split out from [`migrate_root`] so tests can exercise both the rename and
/// the copy-fallback path with plain temporary directories, through
/// [`migrate_dir_using`].
pub fn migrate_dir(old: &Path, new: &Path) -> std::io::Result<Option<MigrationOutcome>> {
    migrate_dir_using(old, new, |old, new| std::fs::rename(old, new))
}

/// [`migrate_dir`], with the rename step injected so tests can force the
/// fallback without needing a second real filesystem.
fn migrate_dir_using(
    old: &Path,
    new: &Path,
    rename: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<Option<MigrationOutcome>> {
    if new.exists() || !old.exists() {
        return Ok(None);
    }
    match rename(old, new) {
        Ok(()) => Ok(Some(MigrationOutcome::Renamed)),
        Err(_) => {
            // Not on the same filesystem (or some other reason `rename`
            // refuses): copy instead, and leave `old` untouched so a failed
            // or partial copy never loses anything.
            copy_dir_recursive(old, new)?;
            Ok(Some(MigrationOutcome::Copied))
        }
    }
}

/// Recursively copies `src` into `dst`, creating directories as needed.
/// Symlinks are skipped rather than followed or failed on: a broken or
/// self-referential one must not abort the whole migration.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_symlink() {
            continue;
        } else if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn renames_when_possible() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("asteroid");
        let new = dir.path().join("cincel");
        write(&old.join("settings.json"), "{}");
        write(&old.join("themes/mine.json"), "{}");

        let outcome = migrate_dir(&old, &new).unwrap();

        assert_eq!(outcome, Some(MigrationOutcome::Renamed));
        assert!(!old.exists(), "el directorio viejo debe desaparecer");
        assert_eq!(
            std::fs::read_to_string(new.join("settings.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            std::fs::read_to_string(new.join("themes/mine.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn falls_back_to_copy_when_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("asteroid");
        let new = dir.path().join("cincel");
        write(&old.join("settings.json"), "contenido");
        write(&old.join("themes/mine.json"), "tema");

        let outcome = migrate_dir_using(&old, &new, |_, _| {
            Err(std::io::Error::other("simulated EXDEV"))
        })
        .unwrap();

        assert_eq!(outcome, Some(MigrationOutcome::Copied));
        // The original is left in place: a failed copy must never lose data.
        assert!(old.join("settings.json").exists());
        assert_eq!(
            std::fs::read_to_string(new.join("settings.json")).unwrap(),
            "contenido"
        );
        assert_eq!(
            std::fs::read_to_string(new.join("themes/mine.json")).unwrap(),
            "tema"
        );
    }

    #[test]
    fn is_idempotent_when_cincel_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("asteroid");
        let new = dir.path().join("cincel");
        write(&old.join("settings.json"), "vieja");
        write(&new.join("settings.json"), "nueva");

        let outcome = migrate_dir(&old, &new).unwrap();

        assert_eq!(outcome, None);
        // Neither side is touched.
        assert_eq!(
            std::fs::read_to_string(old.join("settings.json")).unwrap(),
            "vieja"
        );
        assert_eq!(
            std::fs::read_to_string(new.join("settings.json")).unwrap(),
            "nueva"
        );
    }

    #[test]
    fn does_nothing_without_an_old_directory() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("asteroid");
        let new = dir.path().join("cincel");

        assert_eq!(migrate_dir(&old, &new).unwrap(), None);
        assert!(!new.exists());
    }
}
