//! asteroid-settings: ver `docs/specs/modulos/settings.md`.
//!
//! Types and loading for the three configuration files Asteroid reads:
//!
//! | File | Type | Loader |
//! |---|---|---|
//! | `~/.config/asteroid/settings.json` | [`Settings`] | [`Settings::load`] |
//! | `~/.config/asteroid/keymap.json` | [`Keymap`] | [`Keymap::load`] |
//! | `~/.config/asteroid/themes/*.json` | [`Theme`] | [`ThemeRegistry::load`] |
//!
//! Three rules shape the whole crate:
//!
//! 1. **Nothing is fatal.** A missing, empty, truncated or nonsensical file
//!    yields defaults plus a list of [`SettingsIssue`]s describing what was
//!    wrong, one per offending key. The editor always starts.
//! 2. **JSONC, not JSON.** The files accept `//` and `/* */` comments and
//!    trailing commas, like VS Code's and Zed's.
//! 3. **No GPUI.** Colors are plain [`Rgba`] values; the GPUI crates convert
//!    them. See `docs/specs/03-arquitectura.md` §2.
//!
//! Issue messages are user-facing and therefore written in Spanish; the code
//! and the docs are in English.

mod color;
mod config;
mod context;
mod defaults;
mod issue;
mod jsonc;
mod keymap;
mod paths;
mod settings;
mod theme;
mod watcher;

pub use color::Rgba;
pub use config::Config;
pub use context::ContextExpr;
pub use defaults::{default_keymap_jsonc, default_settings_jsonc};
pub use issue::{Loaded, SettingsIssue};
pub use keymap::{KeyBinding, Keymap, KeymapSection, Keystroke};
pub use paths::{
    Paths, config_dir, keymap_path, recents_path, settings_path, state_dir, themes_dir,
};
pub use settings::{
    AgentSettings, AgentsSettings, Autonomy, Autosave, DEFAULT_REGISTRY_URL, Decorations,
    EditorSettings, FilesSettings, McpServerSettings, ReviewSettings, Settings, TextRendering,
    ThemeMode, ThemeSettings, WindowSettings,
};
pub use theme::{
    COLOR_KEYS, DARK_THEME_NAME, LIGHT_THEME_NAME, SYNTAX_KEYS, SyntaxTheme, Theme, ThemeRegistry,
};
pub use watcher::{DEFAULT_DEBOUNCE, SettingsEvent, SettingsWatcher, WatchError};
