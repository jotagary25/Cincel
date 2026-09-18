//! `settings.json`: the v1 keys of `docs/specs/modulos/settings.md`.
//!
//! Every type has a complete [`Default`] and `#[serde(default)]`, so a file
//! with a single key is valid and everything else keeps its default value.
//! [`Settings::parse`] goes one step further than serde: it reads key by key
//! and replaces only the values it cannot use, reporting each one as a
//! [`SettingsIssue`].

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::issue::{Loaded, SettingsIssue};
use crate::jsonc::{self, Reader};

/// How the theme follows the desktop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Follow the desktop portal's color scheme.
    #[default]
    System,
    /// Always dark.
    Dark,
    /// Always light.
    Light,
}

/// `theme`: which theme to use in each appearance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeSettings {
    /// Whether to follow the system or force an appearance.
    pub mode: ThemeMode,
    /// Theme used in dark appearance.
    pub dark: String,
    /// Theme used in light appearance.
    pub light: String,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            mode: ThemeMode::System,
            dark: crate::theme::DARK_THEME_NAME.to_owned(),
            light: crate::theme::LIGHT_THEME_NAME.to_owned(),
        }
    }
}

/// `text_rendering`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextRendering {
    /// Subpixel antialiasing (the default, `02-visual.md` §3).
    #[default]
    Subpixel,
    /// Grayscale antialiasing.
    Grayscale,
}

/// `editor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorSettings {
    /// Wrap long lines instead of scrolling horizontally.
    pub soft_wrap: bool,
    /// Width of a tab stop, in columns.
    pub tab_size: u32,
    /// Insert spaces when pressing Tab.
    pub insert_spaces: bool,
    /// Draw whitespace characters.
    pub show_whitespace: bool,
    /// Column of the vertical guide; `0` hides it.
    pub ruler: u32,
    /// Blink the caret.
    pub cursor_blink: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            soft_wrap: false,
            tab_size: 4,
            insert_spaces: true,
            show_whitespace: false,
            ruler: 100,
            cursor_blink: true,
        }
    }
}

/// `files.autosave`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Autosave {
    /// Never save automatically.
    #[default]
    Off,
    /// Save when the editor loses focus.
    OnFocusChange,
}

/// `files`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FilesSettings {
    /// Globs never shown in the tree nor walked by the worktree.
    pub exclude: Vec<String>,
    /// When to save automatically.
    pub autosave: Autosave,
}

impl Default for FilesSettings {
    fn default() -> Self {
        Self {
            exclude: vec![
                "**/.git".to_owned(),
                "**/target".to_owned(),
                "**/node_modules".to_owned(),
            ],
            autosave: Autosave::Off,
        }
    }
}

/// `review`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReviewSettings {
    /// Jump to the next pending hunk after accepting or rejecting one.
    pub jump_to_next_on_decide: bool,
    /// Files larger than this are reviewed per file, not per hunk.
    pub max_file_size_kb: u64,
    /// Same, by line count.
    pub max_lines: u32,
    /// Globs that always require confirmation before the agent touches them.
    pub sensitive_paths: Vec<String>,
}

impl Default for ReviewSettings {
    fn default() -> Self {
        Self {
            jump_to_next_on_decide: true,
            max_file_size_kb: 2048,
            max_lines: 50_000,
            sensitive_paths: vec![
                "**/.env*".to_owned(),
                "**/.git/**".to_owned(),
                "**/Cargo.lock".to_owned(),
                "**/package-lock.json".to_owned(),
            ],
        }
    }
}

/// `agents.autonomy`: how much the agent may do without asking.
///
/// Mirrors `asteroid_acp::Autonomy`; that crate owns the behaviour, this one
/// only owns the setting, so the two never depend on each other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    /// Apply edits and review them afterwards (the default).
    #[default]
    ReviewAfter,
    /// Ask before every tool call.
    AskBefore,
    /// Never ask.
    AlwaysApply,
}

/// One entry of `agents.custom`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSettings {
    /// Stable identifier used in `agents.default`.
    pub id: String,
    /// Name shown in the chat header.
    pub name: String,
    /// Executable to run.
    pub command: String,
    /// Arguments passed to it.
    pub args: Vec<String>,
    /// Extra environment variables.
    pub env: BTreeMap<String, String>,
}

/// One entry of `agents.mcp_servers`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpServerSettings {
    /// Name announced to the agent.
    pub name: String,
    /// Executable to run.
    pub command: String,
    /// Arguments passed to it.
    pub args: Vec<String>,
    /// Extra environment variables.
    pub env: BTreeMap<String, String>,
}

/// Default agent registry, from `modulos/settings.md`.
pub const DEFAULT_REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// `agents`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentsSettings {
    /// Id of the agent selected on startup.
    pub default: String,
    /// Autonomy policy.
    pub autonomy: Autonomy,
    /// Where the agent registry is downloaded from.
    pub registry_url: String,
    /// Agents defined by the user, added to the registry ones.
    pub custom: Vec<AgentSettings>,
    /// MCP servers offered to every session.
    pub mcp_servers: Vec<McpServerSettings>,
}

impl Default for AgentsSettings {
    fn default() -> Self {
        Self {
            default: "claude-acp".to_owned(),
            autonomy: Autonomy::ReviewAfter,
            registry_url: DEFAULT_REGISTRY_URL.to_owned(),
            custom: Vec::new(),
            mcp_servers: Vec::new(),
        }
    }
}

/// `window.decorations`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decorations {
    /// Client-side decorations (the integrated title bar).
    #[default]
    Client,
    /// Let the compositor draw the title bar.
    Server,
}

/// `window`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowSettings {
    /// Who draws the title bar.
    pub decorations: Decorations,
}

/// Everything `settings.json` can hold.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Theme selection.
    pub theme: ThemeSettings,
    /// UI font family (`02-visual.md` §3).
    pub ui_font_family: String,
    /// UI font size in logical pixels.
    pub ui_font_size: f32,
    /// Editor font family.
    pub buffer_font_family: String,
    /// Editor font size in logical pixels.
    pub buffer_font_size: f32,
    /// Editor line height as a multiple of the font size.
    pub buffer_line_height: f32,
    /// Text antialiasing.
    pub text_rendering: TextRendering,
    /// Editor behaviour.
    pub editor: EditorSettings,
    /// File handling.
    pub files: FilesSettings,
    /// Agent change review.
    pub review: ReviewSettings,
    /// Agents.
    pub agents: AgentsSettings,
    /// Window chrome.
    pub window: WindowSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeSettings::default(),
            ui_font_family: "Inter".to_owned(),
            ui_font_size: 13.,
            buffer_font_family: "JetBrains Mono".to_owned(),
            buffer_font_size: 14.,
            buffer_line_height: 1.5,
            text_rendering: TextRendering::Subpixel,
            editor: EditorSettings::default(),
            files: FilesSettings::default(),
            review: ReviewSettings::default(),
            agents: AgentsSettings::default(),
            window: WindowSettings::default(),
        }
    }
}

/// Smallest and largest font size we accept, in logical pixels.
const FONT_SIZE_RANGE: (f32, f32) = (6., 72.);
/// Smallest and largest line height multiplier.
const LINE_HEIGHT_RANGE: (f32, f32) = (1., 4.);

impl Settings {
    /// Reads `~/.config/asteroid/settings.json`.
    ///
    /// A missing file, an empty file and a file full of syntax errors all
    /// produce [`Settings::default`]; the last one also produces an issue.
    pub fn load() -> Loaded<Settings> {
        match crate::paths::Paths::resolve() {
            Some(paths) => Settings::load_from(&paths.settings),
            None => Loaded::clean(Settings::default()),
        }
    }

    /// Reads an explicit settings file.
    pub fn load_from(path: &Path) -> Loaded<Settings> {
        match std::fs::read_to_string(path) {
            Ok(text) => Settings::parse(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Loaded::clean(Settings::default())
            }
            Err(error) => Loaded {
                value: Settings::default(),
                issues: vec![SettingsIssue::document(format!(
                    "no se pudo leer «{}»: {error}",
                    path.display()
                ))],
            },
        }
    }

    /// Reads settings from JSONC text.
    pub fn parse(text: &str) -> Loaded<Settings> {
        let mut issues = Vec::new();
        let value = match jsonc::parse(text) {
            Ok(value) => value,
            Err(error) => {
                issues.push(SettingsIssue::document(format!(
                    "no se pudo leer settings.json ({error}); se usan los valores predeterminados"
                )));
                return Loaded {
                    value: Settings::default(),
                    issues,
                };
            }
        };
        let settings = Reader::document(&value, &mut issues, Settings::read);
        Loaded {
            value: settings,
            issues,
        }
    }

    fn read(reader: &mut Reader<'_>, object: &Map<String, Value>) -> Settings {
        let d = Settings::default();
        let settings = Settings {
            theme: reader.object(object, "theme", |reader, theme| {
                let d = ThemeSettings::default();
                let value = ThemeSettings {
                    mode: reader.field(theme, "mode", d.mode),
                    dark: reader.field(theme, "dark", d.dark),
                    light: reader.field(theme, "light", d.light),
                };
                reader.unknown_keys(theme, &["mode", "dark", "light"]);
                value
            }),
            ui_font_family: reader.field(object, "ui_font_family", d.ui_font_family),
            ui_font_size: font_size(reader, object, "ui_font_size", d.ui_font_size),
            buffer_font_family: reader.field(object, "buffer_font_family", d.buffer_font_family),
            buffer_font_size: font_size(reader, object, "buffer_font_size", d.buffer_font_size),
            buffer_line_height: reader.checked_field(
                object,
                "buffer_line_height",
                d.buffer_line_height,
                |height: &f32| in_range(*height, LINE_HEIGHT_RANGE, "el interlineado"),
            ),
            text_rendering: reader.field(object, "text_rendering", d.text_rendering),
            editor: reader.object(object, "editor", |reader, editor| {
                let d = EditorSettings::default();
                let value = EditorSettings {
                    soft_wrap: reader.field(editor, "soft_wrap", d.soft_wrap),
                    tab_size: reader.checked_field(editor, "tab_size", d.tab_size, |size: &u32| {
                        if (1..=16).contains(size) {
                            Ok(())
                        } else {
                            Err("el tamaño de tabulación debe estar entre 1 y 16".to_owned())
                        }
                    }),
                    insert_spaces: reader.field(editor, "insert_spaces", d.insert_spaces),
                    show_whitespace: reader.field(editor, "show_whitespace", d.show_whitespace),
                    ruler: reader.checked_field(editor, "ruler", d.ruler, |ruler: &u32| {
                        if *ruler <= 1000 {
                            Ok(())
                        } else {
                            Err("la guía vertical debe estar entre 0 y 1000".to_owned())
                        }
                    }),
                    cursor_blink: reader.field(editor, "cursor_blink", d.cursor_blink),
                };
                reader.unknown_keys(
                    editor,
                    &[
                        "soft_wrap",
                        "tab_size",
                        "insert_spaces",
                        "show_whitespace",
                        "ruler",
                        "cursor_blink",
                    ],
                );
                value
            }),
            files: reader.object(object, "files", |reader, files| {
                let d = FilesSettings::default();
                let value = FilesSettings {
                    exclude: reader.array(files, "exclude", d.exclude, |reader, item| {
                        match item.as_str() {
                            Some(glob) => Some(glob.to_owned()),
                            None => {
                                reader.issue("", "cada exclusión debe ser un texto; se ignora");
                                None
                            }
                        }
                    }),
                    autosave: reader.field(files, "autosave", d.autosave),
                };
                reader.unknown_keys(files, &["exclude", "autosave"]);
                value
            }),
            review: reader.object(object, "review", |reader, review| {
                let d = ReviewSettings::default();
                let value = ReviewSettings {
                    jump_to_next_on_decide: reader.field(
                        review,
                        "jump_to_next_on_decide",
                        d.jump_to_next_on_decide,
                    ),
                    max_file_size_kb: reader.field(review, "max_file_size_kb", d.max_file_size_kb),
                    max_lines: reader.field(review, "max_lines", d.max_lines),
                    sensitive_paths: reader.array(
                        review,
                        "sensitive_paths",
                        d.sensitive_paths,
                        |reader, item| match item.as_str() {
                            Some(glob) => Some(glob.to_owned()),
                            None => {
                                reader.issue("", "cada ruta sensible debe ser un texto; se ignora");
                                None
                            }
                        },
                    ),
                };
                reader.unknown_keys(
                    review,
                    &[
                        "jump_to_next_on_decide",
                        "max_file_size_kb",
                        "max_lines",
                        "sensitive_paths",
                    ],
                );
                value
            }),
            agents: reader.object(object, "agents", |reader, agents| {
                let d = AgentsSettings::default();
                let value = AgentsSettings {
                    default: reader.field(agents, "default", d.default),
                    autonomy: reader.field(agents, "autonomy", d.autonomy),
                    registry_url: reader.field(agents, "registry_url", d.registry_url),
                    custom: reader.array(agents, "custom", d.custom, read_custom_agent),
                    mcp_servers: reader.array(
                        agents,
                        "mcp_servers",
                        d.mcp_servers,
                        read_mcp_server,
                    ),
                };
                reader.unknown_keys(
                    agents,
                    &[
                        "default",
                        "autonomy",
                        "registry_url",
                        "custom",
                        "mcp_servers",
                    ],
                );
                value
            }),
            window: reader.object(object, "window", |reader, window| {
                let value = WindowSettings {
                    decorations: reader.field(
                        window,
                        "decorations",
                        WindowSettings::default().decorations,
                    ),
                };
                reader.unknown_keys(window, &["decorations"]);
                value
            }),
        };
        reader.unknown_keys(
            object,
            &[
                "theme",
                "ui_font_family",
                "ui_font_size",
                "buffer_font_family",
                "buffer_font_size",
                "buffer_line_height",
                "text_rendering",
                "editor",
                "files",
                "review",
                "agents",
                "window",
            ],
        );
        settings
    }
}

/// Reads one `agents.custom` entry, dropping it when it cannot be launched.
fn read_custom_agent(reader: &mut Reader<'_>, item: &Value) -> Option<AgentSettings> {
    let Value::Object(object) = item else {
        reader.issue(
            "",
            format!(
                "cada agente debe ser un objeto, no {}; se ignora",
                jsonc::type_name(item)
            ),
        );
        return None;
    };
    let agent = AgentSettings {
        id: reader.field(object, "id", String::new()),
        name: reader.field(object, "name", String::new()),
        command: reader.field(object, "command", String::new()),
        args: reader.field(object, "args", Vec::new()),
        env: reader.field(object, "env", BTreeMap::new()),
    };
    reader.unknown_keys(object, &["id", "name", "command", "args", "env"]);
    if agent.id.trim().is_empty() || agent.command.trim().is_empty() {
        reader.issue("", "un agente necesita «id» y «command»; se ignora");
        return None;
    }
    Some(AgentSettings {
        name: if agent.name.is_empty() {
            agent.id.clone()
        } else {
            agent.name
        },
        ..agent
    })
}

/// Reads one `agents.mcp_servers` entry.
fn read_mcp_server(reader: &mut Reader<'_>, item: &Value) -> Option<McpServerSettings> {
    let Value::Object(object) = item else {
        reader.issue(
            "",
            format!(
                "cada servidor MCP debe ser un objeto, no {}; se ignora",
                jsonc::type_name(item)
            ),
        );
        return None;
    };
    let server = McpServerSettings {
        name: reader.field(object, "name", String::new()),
        command: reader.field(object, "command", String::new()),
        args: reader.field(object, "args", Vec::new()),
        env: reader.field(object, "env", BTreeMap::new()),
    };
    reader.unknown_keys(object, &["name", "command", "args", "env"]);
    if server.name.trim().is_empty() || server.command.trim().is_empty() {
        reader.issue("", "un servidor MCP necesita «name» y «command»; se ignora");
        return None;
    }
    Some(server)
}

/// Reads a font size and checks it is inside [`FONT_SIZE_RANGE`].
fn font_size(reader: &mut Reader<'_>, object: &Map<String, Value>, key: &str, default: f32) -> f32 {
    reader.checked_field(object, key, default, |size: &f32| {
        in_range(*size, FONT_SIZE_RANGE, "el tamaño de fuente")
    })
}

/// `Ok` when `value` is finite and inside `range`.
fn in_range(value: f32, range: (f32, f32), what: &str) -> Result<(), String> {
    if value.is_finite() && (range.0..=range.1).contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{what} debe estar entre {} y {}",
            range.0 as i32, range.1 as i32
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_the_default() {
        let loaded = Settings::load_from(Path::new("/no/existe/settings.json"));
        assert!(loaded.is_clean());
        assert_eq!(loaded.value, Settings::default());
    }

    #[test]
    fn empty_file_is_the_default() {
        for text in ["", "   \n\t", "// nada que ver\n", "{}"] {
            let loaded = Settings::parse(text);
            assert!(loaded.is_clean(), "{text:?} -> {:?}", loaded.issues);
            assert_eq!(loaded.value, Settings::default());
        }
    }

    #[test]
    fn syntax_error_is_reported_and_defaulted() {
        let loaded = Settings::parse("{ \"ui_font_size\": }");
        assert_eq!(loaded.value, Settings::default());
        assert_eq!(loaded.issues.len(), 1);
        assert_eq!(loaded.issues[0].path, "$");
        assert!(loaded.issues[0].message.contains("settings.json"));
    }

    #[test]
    fn a_non_object_document_is_reported() {
        let loaded = Settings::parse("[1, 2, 3]");
        assert_eq!(loaded.value, Settings::default());
        assert_eq!(loaded.issues.len(), 1);
        assert_eq!(loaded.issues[0].path, "$");
    }

    #[test]
    fn one_bad_value_does_not_lose_the_good_ones() {
        let loaded = Settings::parse(
            r#"{
              "ui_font_size": "grande",      // inválido
              "buffer_font_size": 16,        // válido
              "editor": { "tab_size": 0, "soft_wrap": true },
              "files": { "autosave": "cuando_sea" },
            }"#,
        );
        let settings = &loaded.value;
        assert_eq!(settings.ui_font_size, 13.);
        assert_eq!(settings.buffer_font_size, 16.);
        assert_eq!(settings.editor.tab_size, 4);
        assert!(settings.editor.soft_wrap);
        assert_eq!(settings.files.autosave, Autosave::Off);
        let paths: Vec<&str> = loaded.issues.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, ["ui_font_size", "editor.tab_size", "files.autosave"]);
    }

    #[test]
    fn unknown_keys_are_reported() {
        let loaded = Settings::parse(r#"{ "ui_font_familly": "Inter", "editor": { "tabs": 2 } }"#);
        let paths: Vec<&str> = loaded.issues.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"ui_font_familly"), "{paths:?}");
        assert!(paths.contains(&"editor.tabs"), "{paths:?}");
        assert_eq!(loaded.value, Settings::default());
    }

    #[test]
    fn out_of_range_numbers_are_rejected() {
        let loaded = Settings::parse(
            r#"{ "ui_font_size": 0, "buffer_font_size": 900, "buffer_line_height": 0.1,
                 "editor": { "ruler": 99999 } }"#,
        );
        assert_eq!(loaded.value, Settings::default());
        assert_eq!(loaded.issues.len(), 4);
    }

    #[test]
    fn wrong_shape_falls_back_to_the_whole_section() {
        let loaded = Settings::parse(r#"{ "editor": 4, "files": { "exclude": "**/target" } }"#);
        assert_eq!(loaded.value.editor, EditorSettings::default());
        assert_eq!(loaded.value.files.exclude, FilesSettings::default().exclude);
        let paths: Vec<&str> = loaded.issues.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, ["editor", "files.exclude"]);
    }

    #[test]
    fn custom_agents_need_id_and_command() {
        let loaded = Settings::parse(
            r#"{ "agents": { "custom": [
                 { "id": "mi-agente", "name": "Mi agente", "command": "mi-agente",
                   "args": ["acp"], "env": { "K": "V" } },
                 { "name": "sin id" },
                 42
               ] } }"#,
        );
        let custom = &loaded.value.agents.custom;
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0].id, "mi-agente");
        assert_eq!(custom[0].args, ["acp"]);
        assert_eq!(custom[0].env["K"], "V");
        assert_eq!(loaded.issues.len(), 2);
        assert!(loaded.issues[0].path.starts_with("agents.custom[1]"));
        assert!(loaded.issues[1].path.starts_with("agents.custom[2]"));
    }

    #[test]
    fn full_document_round_trips_through_serde() {
        let settings = Settings {
            ui_font_size: 15.,
            agents: AgentsSettings {
                custom: vec![AgentSettings {
                    id: "x".to_owned(),
                    name: "X".to_owned(),
                    command: "x".to_owned(),
                    args: vec!["acp".to_owned()],
                    env: BTreeMap::from([("A".to_owned(), "B".to_owned())]),
                }],
                mcp_servers: vec![McpServerSettings {
                    name: "fs".to_owned(),
                    command: "mcp-fs".to_owned(),
                    ..McpServerSettings::default()
                }],
                ..AgentsSettings::default()
            },
            ..Settings::default()
        };
        let json = serde_json::to_string_pretty(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
        // The tolerant reader must agree with serde on a valid document.
        let loaded = Settings::parse(&json);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(loaded.value, settings);
    }

    #[test]
    fn enums_use_the_names_from_the_spec() {
        let loaded = Settings::parse(
            r#"{ "theme": { "mode": "dark" }, "text_rendering": "grayscale",
                 "files": { "autosave": "on_focus_change" },
                 "agents": { "autonomy": "always_apply" },
                 "window": { "decorations": "server" } }"#,
        );
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(loaded.value.theme.mode, ThemeMode::Dark);
        assert_eq!(loaded.value.text_rendering, TextRendering::Grayscale);
        assert_eq!(loaded.value.files.autosave, Autosave::OnFocusChange);
        assert_eq!(loaded.value.agents.autonomy, Autonomy::AlwaysApply);
        assert_eq!(loaded.value.window.decorations, Decorations::Server);
    }

    #[test]
    fn reads_a_file_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ \"ui_font_size\": 20 }").unwrap();
        let loaded = Settings::load_from(&path);
        assert!(loaded.is_clean());
        assert_eq!(loaded.value.ui_font_size, 20.);
    }
}
