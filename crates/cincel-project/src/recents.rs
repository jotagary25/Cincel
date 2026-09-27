//! The list of recently opened projects.
//!
//! `docs/specs/02-visual.md` §9 shows it on the empty screen and
//! `docs/specs/modulos/workspace.md` opens the last project when `cincel`
//! is started without a path. It lives in the XDG state directory, next to
//! `window.json`, because it is volatile: losing it costs nothing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many projects are remembered (`modulos/project.md`).
pub const MAX_RECENTS: usize = 10;

/// The recently opened projects, most recent first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Recents {
    /// Absolute project paths, most recent first.
    projects: Vec<PathBuf>,
}

impl Recents {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// `~/.local/state/cincel/recents.json`.
    pub fn path() -> Option<PathBuf> {
        dirs::state_dir().map(|dir| dir.join("cincel").join("recents.json"))
    }

    /// Reads the list, or an empty one when it is missing or unreadable.
    ///
    /// A corrupt file is not an error: the list is a convenience.
    pub fn load() -> Recents {
        match Recents::path() {
            Some(path) => Recents::load_from(&path),
            None => Recents::new(),
        }
    }

    /// Reads the list from an explicit file.
    pub fn load_from(path: &Path) -> Recents {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Recents::new();
        };
        match serde_json::from_str::<Recents>(&text) {
            Ok(mut recents) => {
                recents.projects.truncate(MAX_RECENTS);
                recents
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "no se pudo leer la lista de proyectos recientes");
                Recents::new()
            }
        }
    }

    /// Writes the list, creating the state directory if needed.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Recents::path()
            .ok_or_else(|| std::io::Error::other("no hay un directorio de estado XDG"))?;
        self.save_to(&path)
    }

    /// Writes the list to an explicit file.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Records `path` as the most recently opened project.
    ///
    /// Moves it to the front when it was already there, so the list is an
    /// access order, and drops the oldest entry beyond [`MAX_RECENTS`].
    pub fn push(&mut self, path: impl AsRef<Path>) {
        let path =
            std::path::absolute(path.as_ref()).unwrap_or_else(|_| path.as_ref().to_path_buf());
        self.projects.retain(|existing| existing != &path);
        self.projects.insert(0, path);
        self.projects.truncate(MAX_RECENTS);
    }

    /// Forgets `path`, for the "remove from list" menu item and for projects
    /// that no longer exist.
    pub fn remove(&mut self, path: impl AsRef<Path>) -> bool {
        let before = self.projects.len();
        let path = path.as_ref();
        self.projects.retain(|existing| existing != path);
        self.projects.len() != before
    }

    /// Drops every project that is no longer a directory.
    pub fn prune_missing(&mut self) {
        self.projects.retain(|path| path.is_dir());
    }

    /// The projects, most recent first.
    pub fn projects(&self) -> &[PathBuf] {
        &self.projects
    }

    /// The most recently opened project, which is what `cincel` with no
    /// argument opens.
    pub fn most_recent(&self) -> Option<&Path> {
        self.projects.first().map(PathBuf::as_path)
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_moves_to_the_front_and_caps_the_list() {
        let mut recents = Recents::new();
        for index in 0..MAX_RECENTS + 5 {
            recents.push(format!("/p/{index}"));
        }
        assert_eq!(recents.projects().len(), MAX_RECENTS);
        assert_eq!(recents.most_recent().unwrap(), Path::new("/p/14"));

        recents.push("/p/10");
        assert_eq!(recents.most_recent().unwrap(), Path::new("/p/10"));
        assert_eq!(recents.projects().len(), MAX_RECENTS);
        // No duplicates.
        assert_eq!(
            recents
                .projects()
                .iter()
                .filter(|p| p.as_path() == Path::new("/p/10"))
                .count(),
            1
        );
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recents.json");
        let mut recents = Recents::new();
        recents.push("/uno");
        recents.push("/dos");
        recents.save_to(&path).unwrap();

        let loaded = Recents::load_from(&path);
        assert_eq!(loaded, recents);
        assert_eq!(loaded.most_recent().unwrap(), Path::new("/dos"));
    }

    #[test]
    fn a_missing_or_broken_file_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Recents::load_from(&dir.path().join("no-existe.json")).is_empty());
        let broken = dir.path().join("roto.json");
        std::fs::write(&broken, "{ esto no es json").unwrap();
        assert!(Recents::load_from(&broken).is_empty());
    }

    #[test]
    fn relative_paths_are_stored_absolute() {
        let mut recents = Recents::new();
        recents.push(".");
        assert!(recents.most_recent().unwrap().is_absolute());
    }

    #[test]
    fn missing_projects_can_be_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let mut recents = Recents::new();
        recents.push(dir.path());
        recents.push("/no/existe/seguro");
        recents.prune_missing();
        assert_eq!(recents.projects().len(), 1);
        assert!(recents.remove(recents.projects()[0].clone()));
        assert!(recents.is_empty());
    }

    #[test]
    fn a_long_saved_list_is_truncated_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recents.json");
        let many: Vec<String> = (0..50).map(|index| format!("/p/{index}")).collect();
        std::fs::write(
            &path,
            serde_json::to_string(&serde_json::json!({ "projects": many })).unwrap(),
        )
        .unwrap();
        assert_eq!(Recents::load_from(&path).projects().len(), MAX_RECENTS);
    }
}
