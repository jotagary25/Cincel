//! The three documents, loaded together.
//!
//! [`Config`] is what the binary and `cincel-workspace` actually hold: the
//! settings, the keymap (default plus the user's) and every theme, loaded in
//! one call and reloaded one document at a time from a
//! [`SettingsEvent`](crate::SettingsEvent).

use crate::issue::{Loaded, SettingsIssue};
use crate::keymap::Keymap;
use crate::paths::Paths;
use crate::settings::Settings;
use crate::theme::{Theme, ThemeRegistry};
use crate::watcher::SettingsEvent;

/// Everything Cincel reads from `~/.config/cincel/`.
///
/// The default is what Cincel runs on with no configuration at all: the
/// default settings, the built-in keymap and the two built-in themes.
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// `settings.json`.
    pub settings: Settings,
    /// The default keymap with `keymap.json` layered over it.
    pub keymap: Keymap,
    /// The built-in themes plus `themes/*.json`.
    pub themes: ThemeRegistry,
    /// Where it all came from.
    pub paths: Option<Paths>,
}

impl Config {
    /// Loads the three documents from the real XDG locations.
    ///
    /// Never fails: every problem comes back as a [`SettingsIssue`] and the
    /// offending value is the default.
    pub fn load() -> Loaded<Config> {
        match Paths::resolve() {
            Some(paths) => Config::load_from(&paths),
            None => Loaded::clean(Config::default()),
        }
    }

    /// Loads the three documents from an explicit configuration directory.
    pub fn load_from(paths: &Paths) -> Loaded<Config> {
        let mut issues = Vec::new();
        let settings = Settings::load_from(&paths.settings);
        issues.extend(prefix(settings.issues, "settings.json"));
        let keymap = Keymap::load_from(&paths.keymap);
        issues.extend(prefix(keymap.issues, "keymap.json"));
        let themes = ThemeRegistry::load(&paths.themes);
        issues.extend(themes.issues);
        Loaded {
            value: Config {
                settings: settings.value,
                keymap: keymap.value,
                themes: themes.value,
                paths: Some(paths.clone()),
            },
            issues,
        }
    }

    /// Re-reads the document `event` is about, returning what was wrong with
    /// it. Everything else is left alone.
    pub fn reload(&mut self, event: SettingsEvent) -> Vec<SettingsIssue> {
        let Some(paths) = self.paths.clone() else {
            return Vec::new();
        };
        match event {
            SettingsEvent::SettingsChanged => {
                let loaded = Settings::load_from(&paths.settings);
                self.settings = loaded.value;
                prefix(loaded.issues, "settings.json")
            }
            SettingsEvent::KeymapChanged => {
                let loaded = Keymap::load_from(&paths.keymap);
                self.keymap = loaded.value;
                prefix(loaded.issues, "keymap.json")
            }
            SettingsEvent::ThemeChanged => {
                let loaded = ThemeRegistry::load(&paths.themes);
                self.themes = loaded.value;
                loaded.issues
            }
        }
    }

    /// The theme in force, given what the desktop portal reports.
    pub fn theme(&self, system_dark: bool) -> &Theme {
        self.themes.resolve(&self.settings.theme, system_dark)
    }
}

/// Prefixes every issue with the file it came from.
fn prefix(issues: Vec<SettingsIssue>, file: &str) -> Vec<SettingsIssue> {
    issues
        .into_iter()
        .map(|issue| issue.in_file(file))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{DARK_THEME_NAME, LIGHT_THEME_NAME};

    #[test]
    fn an_empty_config_directory_is_all_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = Config::load_from(&Paths::under(dir.path()));
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(loaded.value.settings, Settings::default());
        assert_eq!(loaded.value.keymap, Keymap::default());
        assert_eq!(loaded.value.theme(true).name, DARK_THEME_NAME);
        assert_eq!(loaded.value.theme(false).name, LIGHT_THEME_NAME);
    }

    #[test]
    fn issues_say_which_file_they_came_from() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(&paths.themes).unwrap();
        std::fs::write(&paths.settings, r#"{ "ui_font_size": "grande" }"#).unwrap();
        std::fs::write(&paths.keymap, r#"[{ "context": "&&" }]"#).unwrap();
        std::fs::write(paths.themes.join("roto.json"), "{").unwrap();

        let loaded = Config::load_from(&paths);
        let paths_of: Vec<&str> = loaded.issues.iter().map(|i| i.path.as_str()).collect();
        assert!(
            paths_of
                .iter()
                .any(|path| path.starts_with("settings.json: ui_font_size")),
            "{paths_of:?}"
        );
        assert!(
            paths_of
                .iter()
                .any(|path| path.starts_with("keymap.json: ")),
            "{paths_of:?}"
        );
        assert!(
            paths_of.iter().any(|path| path.starts_with("roto.json: ")),
            "{paths_of:?}"
        );
    }

    #[test]
    fn reload_touches_only_the_document_that_changed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(&paths.themes).unwrap();
        let mut config = Config::load_from(&paths).value;
        assert_eq!(config.settings.ui_font_size, 13.);

        std::fs::write(&paths.settings, r#"{ "ui_font_size": 17 }"#).unwrap();
        assert!(config.reload(SettingsEvent::SettingsChanged).is_empty());
        assert_eq!(config.settings.ui_font_size, 17.);
        assert_eq!(config.keymap, Keymap::default());

        std::fs::write(
            paths.themes.join("mio.json"),
            r##"{ "name": "Mío", "bg.app": "#010203" }"##,
        )
        .unwrap();
        assert!(config.reload(SettingsEvent::ThemeChanged).is_empty());
        assert!(config.themes.get("Mío").is_some());
        assert_eq!(config.settings.ui_font_size, 17.);
    }
}
