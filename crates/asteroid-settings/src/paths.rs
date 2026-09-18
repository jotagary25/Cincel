//! XDG locations, per `docs/specs/03-arquitectura.md` §6.
//!
//! Config lives in `~/.config/asteroid/`, volatile state in
//! `~/.local/state/asteroid/`. Every function returns `None` on a machine
//! with no home directory (a container, a daemon): callers then run on
//! defaults instead of failing.

use std::path::PathBuf;

/// Directory name used under every XDG base directory.
pub const APP_DIR: &str = "asteroid";

/// `~/.config/asteroid`.
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(APP_DIR))
}

/// `~/.local/state/asteroid`.
pub fn state_dir() -> Option<PathBuf> {
    dirs::state_dir().map(|dir| dir.join(APP_DIR))
}

/// `~/.config/asteroid/settings.json`.
pub fn settings_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("settings.json"))
}

/// `~/.config/asteroid/keymap.json`.
pub fn keymap_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("keymap.json"))
}

/// `~/.config/asteroid/themes`.
pub fn themes_dir() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("themes"))
}

/// `~/.local/state/asteroid/recents.json` (written by `asteroid-project`).
pub fn recents_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("recents.json"))
}

/// The three configuration locations, resolved once.
///
/// [`Paths::under`] builds the same layout under an arbitrary root, which is
/// what the tests and `$ASTEROID_CONFIG_DIR` use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// The configuration directory itself.
    pub config_dir: PathBuf,
    /// `settings.json` inside it.
    pub settings: PathBuf,
    /// `keymap.json` inside it.
    pub keymap: PathBuf,
    /// The `themes/` directory inside it.
    pub themes: PathBuf,
}

impl Paths {
    /// The real XDG locations, or `None` without a home directory.
    ///
    /// `$ASTEROID_CONFIG_DIR` overrides them, which is how a second instance
    /// or a test run keeps its own configuration.
    pub fn resolve() -> Option<Self> {
        if let Some(dir) = std::env::var_os("ASTEROID_CONFIG_DIR") {
            return Some(Self::under(PathBuf::from(dir)));
        }
        config_dir().map(Self::under)
    }

    /// The layout under an explicit configuration directory.
    pub fn under(config_dir: impl Into<PathBuf>) -> Self {
        let config_dir = config_dir.into();
        Self {
            settings: config_dir.join("settings.json"),
            keymap: config_dir.join("keymap.json"),
            themes: config_dir.join("themes"),
            config_dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_under_a_root() {
        let paths = Paths::under("/tmp/cfg");
        assert_eq!(paths.settings, PathBuf::from("/tmp/cfg/settings.json"));
        assert_eq!(paths.keymap, PathBuf::from("/tmp/cfg/keymap.json"));
        assert_eq!(paths.themes, PathBuf::from("/tmp/cfg/themes"));
    }

    #[test]
    fn xdg_paths_end_in_the_app_directory() {
        if let Some(dir) = config_dir() {
            assert!(dir.ends_with(APP_DIR));
        }
        if let Some(path) = settings_path() {
            assert!(path.ends_with("asteroid/settings.json"));
        }
    }
}
