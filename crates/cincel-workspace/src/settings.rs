//! The configuration the running application holds, and its hot reload.
//!
//! `cincel-settings` reads the files; this module keeps the result in a GPUI
//! global, applies it (theme, typography, keymap) and re-applies it when the
//! watcher says a file changed, without restarting
//! (`docs/specs/modulos/workspace.md`, "Binario `cincel`").

use std::path::PathBuf;
use std::time::SystemTime;

use cincel_settings::{Config, Paths, Settings, SettingsEvent, SettingsIssue, Theme};
use gpui::{App, Global};

/// Smallest and largest interface zoom (`workspace::zoom_in` / `zoom_out`).
pub const MIN_UI_SCALE: f32 = 0.7;
/// See [`MIN_UI_SCALE`].
pub const MAX_UI_SCALE: f32 = 2.0;
/// One step of `workspace::zoom_in` / `zoom_out`.
pub const UI_SCALE_STEP: f32 = 0.1;

/// The configuration in force, as a GPUI global.
pub struct AppSettings {
    /// Settings, keymap and themes as they were last read from disk.
    pub config: Config,
    /// What the desktop reports, for `theme.mode = "system"`. Kept in sync
    /// with the window's appearance by [`set_system_dark`].
    pub system_dark: bool,
    /// Interface zoom, 1.0 being 100 %.
    pub ui_scale: f32,
    /// What the configuration files looked like the last time they were read.
    fingerprint: Fingerprint,
}

impl Global for AppSettings {}

impl AppSettings {
    /// The theme in force, given [`system_dark`](Self::system_dark).
    pub fn theme(&self) -> &Theme {
        self.config.theme(self.system_dark)
    }
}

/// Installs `config` as the global configuration and applies it.
///
/// Returns nothing: problems found while *reading* the files are reported by
/// the caller, which is the only one that knows whether a toast can be shown
/// yet.
pub fn install(config: Config, cx: &mut App) {
    let fingerprint = Fingerprint::capture(config.paths.as_ref());
    let settings = AppSettings {
        config,
        system_dark: true,
        ui_scale: 1.0,
        fingerprint,
    };
    crate::theme::apply(
        settings.theme(),
        &settings.config.settings,
        settings.ui_scale,
        cx,
    );
    crate::keymap::install(&settings.config.keymap, cx);
    cx.set_global(settings);
}

/// The settings in force. Falls back to the defaults when nothing has been
/// installed (a test that only needs a widget).
pub fn settings(cx: &App) -> Settings {
    cx.try_global::<AppSettings>()
        .map(|state| state.config.settings.clone())
        .unwrap_or_default()
}

/// The keymap in force: the built-in JSONC layered with the user's
/// `keymap.json` (`docs/specs/07-etapa5-productividad.md` §5.2, the
/// shortcuts modal). Falls back to the built-in one alone when nothing has
/// been installed yet.
pub fn keymap(cx: &App) -> cincel_settings::Keymap {
    cx.try_global::<AppSettings>()
        .map(|state| state.config.keymap.clone())
        .unwrap_or_default()
}

/// Records what the desktop's appearance is and re-applies the theme when it
/// changed, which is how `theme.mode = "system"` follows the desktop.
///
/// Returns whether anything changed.
pub fn set_system_dark(dark: bool, cx: &mut App) -> bool {
    if !cx.has_global::<AppSettings>() {
        return false;
    }
    let state = cx.global_mut::<AppSettings>();
    if state.system_dark == dark {
        return false;
    }
    state.system_dark = dark;
    let (theme, settings, scale) = (
        state.theme().clone(),
        state.config.settings.clone(),
        state.ui_scale,
    );
    crate::theme::apply(&theme, &settings, scale, cx);
    tracing::info!(dark, "apariencia del escritorio");
    true
}

/// The interface zoom in force.
pub fn ui_scale(cx: &App) -> f32 {
    cx.try_global::<AppSettings>()
        .map(|state| state.ui_scale)
        .unwrap_or(1.0)
}

/// Sets the interface zoom, clamped to [`MIN_UI_SCALE`]..=[`MAX_UI_SCALE`],
/// and re-applies the theme so every size follows. Returns the new value.
pub fn set_ui_scale(scale: f32, cx: &mut App) -> f32 {
    if !cx.has_global::<AppSettings>() {
        return 1.0;
    }
    let scale = scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
    let (theme, settings) = {
        let state = cx.global_mut::<AppSettings>();
        state.ui_scale = scale;
        (state.theme().clone(), state.config.settings.clone())
    };
    crate::theme::apply(&theme, &settings, scale, cx);
    scale
}

/// The outcome of a [`reload`].
#[derive(Debug, Default)]
pub struct Reloaded {
    /// Whether anything on disk had actually changed.
    pub changed: bool,
    /// The problems found, ready for a toast.
    pub issues: Vec<SettingsIssue>,
}

/// Re-reads the document `event` names and applies whatever it changed.
///
/// Never fails: a broken file leaves the previous values in place for the keys
/// it could not read.
///
/// An event whose file is byte-for-byte what we already have is ignored, and
/// that is not an optimization but a requirement: `notify`'s watch mask
/// includes `IN_OPEN`, so *reading* a watched file or directory produces
/// another event. Without this guard, one reload feeds the next and the editor
/// reloads its configuration ten times a second forever. See the report of
/// stage 1-D for the fix `cincel-settings` should carry.
pub fn reload(event: SettingsEvent, cx: &mut App) -> Reloaded {
    if !cx.has_global::<AppSettings>() {
        return Reloaded::default();
    }

    let fingerprint = {
        let state = cx.global::<AppSettings>();
        let fingerprint = Fingerprint::capture(state.config.paths.as_ref());
        if fingerprint == state.fingerprint {
            tracing::trace!(
                ?event,
                "cambio de configuración sin contenido nuevo, se ignora"
            );
            return Reloaded::default();
        }
        fingerprint
    };

    let (issues, theme, settings, keymap, scale) = {
        let state = cx.global_mut::<AppSettings>();
        let issues = state.config.reload(event);
        state.fingerprint = fingerprint;
        (
            issues,
            state.theme().clone(),
            state.config.settings.clone(),
            state.config.keymap.clone(),
            state.ui_scale,
        )
    };

    match event {
        SettingsEvent::SettingsChanged | SettingsEvent::ThemeChanged => {
            crate::theme::apply(&theme, &settings, scale, cx);
        }
        SettingsEvent::KeymapChanged => {}
    }
    if matches!(
        event,
        SettingsEvent::KeymapChanged | SettingsEvent::SettingsChanged
    ) {
        crate::keymap::install(&keymap, cx);
    }
    cx.refresh_windows();
    tracing::info!(?event, "configuración recargada");
    Reloaded {
        changed: true,
        issues,
    }
}

/// Modification time and size of every configuration file.
///
/// Built with `stat` only: unlike opening a file, statting it produces no file
/// system event, so taking a fingerprint cannot itself trigger a reload. The
/// list of theme files is remembered so a theme's *content* change is noticed
/// without re-listing the directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Fingerprint {
    settings: Option<Stamp>,
    keymap: Option<Stamp>,
    themes_dir: Option<Stamp>,
    themes: Vec<(PathBuf, Stamp)>,
}

/// What `stat` says about one file.
type Stamp = (Option<SystemTime>, u64);

impl Fingerprint {
    fn capture(paths: Option<&Paths>) -> Self {
        let Some(paths) = paths else {
            return Self::default();
        };
        let mut themes: Vec<(PathBuf, Stamp)> = std::fs::read_dir(&paths.themes)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                stamp(&path).map(|stamp| (path, stamp))
            })
            .collect();
        themes.sort();
        Self {
            settings: stamp(&paths.settings),
            keymap: stamp(&paths.keymap),
            themes_dir: stamp(&paths.themes),
            themes,
        }
    }
}

/// The modification time and size of `path`, or `None` when it is not there.
fn stamp(path: &std::path::Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.modified().ok(), metadata.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fingerprint_notices_every_document() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(&paths.themes).unwrap();
        let empty = Fingerprint::capture(Some(&paths));

        std::fs::write(&paths.settings, "{}").unwrap();
        let with_settings = Fingerprint::capture(Some(&paths));
        assert_ne!(empty, with_settings);

        std::fs::write(&paths.keymap, "[]").unwrap();
        let with_keymap = Fingerprint::capture(Some(&paths));
        assert_ne!(with_settings, with_keymap);

        let theme = paths.themes.join("mio.json");
        std::fs::write(&theme, "{}").unwrap();
        let with_theme = Fingerprint::capture(Some(&paths));
        assert_ne!(with_keymap, with_theme);

        // A theme whose content grows is a change even if the directory is
        // untouched.
        std::fs::write(&theme, "{ \"name\": \"Mío\" }").unwrap();
        assert_ne!(with_theme, Fingerprint::capture(Some(&paths)));
    }

    #[test]
    fn reading_the_files_does_not_change_the_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(&paths.themes).unwrap();
        std::fs::write(&paths.settings, "{ \"ui_font_size\": 15 }").unwrap();

        let before = Fingerprint::capture(Some(&paths));
        let _ = Config::load_from(&paths);
        assert_eq!(before, Fingerprint::capture(Some(&paths)));
    }
}
