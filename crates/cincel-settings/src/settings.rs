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
    /// Auto-close typed brackets and quotes (`(`, `[`, `{`, `"`, `'`, `` ` ``).
    pub auto_close_pairs: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            soft_wrap: true,
            tab_size: 4,
            insert_spaces: true,
            show_whitespace: false,
            ruler: 100,
            cursor_blink: true,
            auto_close_pairs: true,
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
    /// Save `autosave_delay_ms` after the last keystroke, if the buffer is
    /// still dirty by then (`docs/specs/07-etapa5-productividad.md` §10.5).
    AfterDelay,
}

/// Smallest and largest `files.autosave_delay_ms`, in milliseconds.
const AUTOSAVE_DELAY_RANGE: (u64, u64) = (100, 60_000);

/// `files`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FilesSettings {
    /// Globs never shown in the tree nor walked by the worktree.
    pub exclude: Vec<String>,
    /// When to save automatically.
    pub autosave: Autosave,
    /// With `autosave: "after_delay"`, how long to wait after the last
    /// keystroke before saving, in milliseconds.
    pub autosave_delay_ms: u64,
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
            autosave_delay_ms: 1000,
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
    /// Megabytes of file copies the photo taken before each agent turn may
    /// hold; past it, files keep only their hash (`03-arquitectura.md` §4).
    pub snapshot_max_total_mb: u64,
    /// Globs that always require confirmation before the agent touches them.
    pub sensitive_paths: Vec<String>,
}

impl Default for ReviewSettings {
    fn default() -> Self {
        Self {
            jump_to_next_on_decide: false,
            max_file_size_kb: 2048,
            max_lines: 50_000,
            snapshot_max_total_mb: 300,
            sensitive_paths: vec![
                "**/.env*".to_owned(),
                "**/.git/**".to_owned(),
                "**/Cargo.lock".to_owned(),
                "**/package-lock.json".to_owned(),
            ],
        }
    }
}

/// One entry of `connections.mcp_servers`.
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

/// The retired top-level `agents` section (Etapa 4, `docs/specs/06-etapa4-conexiones-y-cincel.md`
/// §9): renamed to `connections`, which drops `default`, `custom` and
/// `registry_url` (the connections system replacing them is a later
/// workstream).
const RETIRED_AGENTS_KEY: &str = "agents";

/// The warning shown when a settings file still carries the old `agents`
/// section (in any shape, including a lone `agents.autonomy`).
pub const RETIRED_AGENTS_MESSAGE: &str = "ajuste retirado: la sección «agents» ahora se llama «connections» \
     (con «default_label» y «runtime»; «default», «custom» y «registry_url» ya no existen); \
     se ignora, podés borrarlo";

/// `connections.runtime`: the private Node runtime used to run the npm-distributed
/// ACP adapters (`docs/specs/06-etapa4-conexiones-y-cincel.md` §3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeSettings {
    /// Node release line to download, e.g. `"lts"` or an exact major like `"22"`.
    pub node_version: String,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            node_version: "lts".to_owned(),
        }
    }
}

/// `connections` (formerly `agents`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectionsSettings {
    /// Label of the connection pre-selected in the "Conectar" popover;
    /// `null` (the default) means none.
    pub default_label: Option<String>,
    /// The private Node runtime.
    pub runtime: RuntimeSettings,
    /// MCP servers offered to every session.
    pub mcp_servers: Vec<McpServerSettings>,
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
    /// Agent connections (`docs/specs/06-etapa4-conexiones-y-cincel.md` §9).
    pub connections: ConnectionsSettings,
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
            connections: ConnectionsSettings::default(),
            window: WindowSettings::default(),
        }
    }
}

/// Smallest and largest font size we accept, in logical pixels.
const FONT_SIZE_RANGE: (f32, f32) = (6., 72.);
/// Smallest and largest line height multiplier.
const LINE_HEIGHT_RANGE: (f32, f32) = (1., 4.);

impl Settings {
    /// Reads `~/.config/cincel/settings.json`.
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
                    auto_close_pairs: reader.field(editor, "auto_close_pairs", d.auto_close_pairs),
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
                        "auto_close_pairs",
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
                    autosave_delay_ms: reader.checked_field(
                        files,
                        "autosave_delay_ms",
                        d.autosave_delay_ms,
                        |ms: &u64| {
                            if (AUTOSAVE_DELAY_RANGE.0..=AUTOSAVE_DELAY_RANGE.1).contains(ms) {
                                Ok(())
                            } else {
                                Err("files.autosave_delay_ms tiene que estar entre 100 y 60000"
                                    .to_owned())
                            }
                        },
                    ),
                };
                reader.unknown_keys(files, &["exclude", "autosave", "autosave_delay_ms"]);
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
                    snapshot_max_total_mb: reader.field(
                        review,
                        "snapshot_max_total_mb",
                        d.snapshot_max_total_mb,
                    ),
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
                        "snapshot_max_total_mb",
                        "sensitive_paths",
                    ],
                );
                value
            }),
            connections: reader.object(object, "connections", |reader, connections| {
                let d = ConnectionsSettings::default();
                let value = ConnectionsSettings {
                    default_label: reader.field(connections, "default_label", d.default_label),
                    runtime: reader.object(connections, "runtime", |reader, runtime| {
                        let d = RuntimeSettings::default();
                        let value = RuntimeSettings {
                            node_version: reader.field(runtime, "node_version", d.node_version),
                        };
                        reader.unknown_keys(runtime, &["node_version"]);
                        value
                    }),
                    mcp_servers: reader.array(
                        connections,
                        "mcp_servers",
                        d.mcp_servers,
                        read_mcp_server,
                    ),
                };
                reader.unknown_keys(connections, &["default_label", "runtime", "mcp_servers"]);
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
        // The old top-level `agents` section (Etapa 3 and earlier, including
        // a lone `agents.autonomy`) is recognized-but-retired rather than an
        // "unknown key": it gets its own explanatory message instead.
        if object.contains_key(RETIRED_AGENTS_KEY) {
            reader.issue(RETIRED_AGENTS_KEY, RETIRED_AGENTS_MESSAGE);
        }
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
                RETIRED_AGENTS_KEY,
                "connections",
                "window",
            ],
        );
        settings
    }
}

/// Reads one `connections.mcp_servers` entry.
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
              "editor": { "tab_size": 0, "soft_wrap": false },
              "files": { "autosave": "cuando_sea" },
            }"#,
        );
        let settings = &loaded.value;
        assert_eq!(settings.ui_font_size, 13.);
        assert_eq!(settings.buffer_font_size, 16.);
        assert_eq!(settings.editor.tab_size, 4);
        assert!(
            !settings.editor.soft_wrap,
            "el valor válido pisa el predeterminado"
        );
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
    fn mcp_servers_need_name_and_command() {
        let loaded = Settings::parse(
            r#"{ "connections": { "mcp_servers": [
                 { "name": "fs", "command": "mcp-fs", "args": ["--root", "."], "env": { "K": "V" } },
                 { "command": "sin nombre" },
                 42
               ] } }"#,
        );
        let servers = &loaded.value.connections.mcp_servers;
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "fs");
        assert_eq!(servers[0].args, ["--root", "."]);
        assert_eq!(servers[0].env["K"], "V");
        assert_eq!(loaded.issues.len(), 2);
        assert!(
            loaded.issues[0]
                .path
                .starts_with("connections.mcp_servers[1]")
        );
        assert!(
            loaded.issues[1]
                .path
                .starts_with("connections.mcp_servers[2]")
        );
    }

    #[test]
    fn connections_default_label_and_runtime_round_trip() {
        let loaded = Settings::parse(
            r#"{ "connections": { "default_label": "Claude · personal",
                 "runtime": { "node_version": "22" } } }"#,
        );
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(
            loaded.value.connections.default_label.as_deref(),
            Some("Claude · personal")
        );
        assert_eq!(loaded.value.connections.runtime.node_version, "22");
    }

    #[test]
    fn full_document_round_trips_through_serde() {
        let settings = Settings {
            ui_font_size: 15.,
            connections: ConnectionsSettings {
                default_label: Some("Claude · personal".to_owned()),
                runtime: RuntimeSettings {
                    node_version: "22".to_owned(),
                },
                mcp_servers: vec![McpServerSettings {
                    name: "fs".to_owned(),
                    command: "mcp-fs".to_owned(),
                    ..McpServerSettings::default()
                }],
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
    fn auto_close_pairs_defaults_to_on_and_can_be_turned_off() {
        assert!(EditorSettings::default().auto_close_pairs);
        let loaded = Settings::parse(r#"{ "editor": { "auto_close_pairs": false } }"#);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert!(!loaded.value.editor.auto_close_pairs);
    }

    #[test]
    fn enums_use_the_names_from_the_spec() {
        let loaded = Settings::parse(
            r#"{ "theme": { "mode": "dark" }, "text_rendering": "grayscale",
                 "files": { "autosave": "on_focus_change" },
                 "window": { "decorations": "server" } }"#,
        );
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(loaded.value.theme.mode, ThemeMode::Dark);
        assert_eq!(loaded.value.text_rendering, TextRendering::Grayscale);
        assert_eq!(loaded.value.files.autosave, Autosave::OnFocusChange);
        assert_eq!(loaded.value.window.decorations, Decorations::Server);
    }

    #[test]
    fn after_delay_autosave_round_trips_with_its_default_delay() {
        let loaded = Settings::parse(r#"{ "files": { "autosave": "after_delay" } }"#);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(loaded.value.files.autosave, Autosave::AfterDelay);
        assert_eq!(loaded.value.files.autosave_delay_ms, 1000);
    }

    #[test]
    fn after_delay_autosave_round_trips_through_serde() {
        let files = FilesSettings {
            autosave: Autosave::AfterDelay,
            autosave_delay_ms: 250,
            ..FilesSettings::default()
        };
        let json = serde_json::to_string(&files).unwrap();
        assert_eq!(serde_json::from_str::<FilesSettings>(&json).unwrap(), files);
        let loaded = Settings::parse(&format!(r#"{{ "files": {json} }}"#));
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(loaded.value.files, files);
    }

    #[test]
    fn autosave_delay_ms_out_of_range_is_rejected() {
        for delay in [0, 99, 60_001, 1_000_000] {
            let loaded = Settings::parse(&format!(
                r#"{{ "files": {{ "autosave_delay_ms": {delay} }} }}"#
            ));
            assert_eq!(loaded.value.files.autosave_delay_ms, 1000, "{delay}");
            assert_eq!(loaded.issues.len(), 1, "{delay}: {:?}", loaded.issues);
            assert_eq!(loaded.issues[0].path, "files.autosave_delay_ms");
        }
    }

    #[test]
    fn autosave_delay_ms_inside_range_is_accepted() {
        let loaded = Settings::parse(r#"{ "files": { "autosave_delay_ms": 250 } }"#);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(loaded.value.files.autosave_delay_ms, 250);
    }

    #[test]
    fn the_retired_agents_section_is_ignored_with_its_own_warning() {
        let loaded = Settings::parse(
            r#"{ "agents": { "default": "codex-acp", "autonomy": "always_apply" } }"#,
        );
        // The whole old section is retired: `connections` stays at its default.
        assert_eq!(loaded.value.connections, ConnectionsSettings::default());
        assert_eq!(loaded.issues.len(), 1, "{:?}", loaded.issues);
        assert_eq!(loaded.issues[0].path, "agents");
        assert!(
            loaded.issues[0].message.starts_with("ajuste retirado"),
            "{:?}",
            loaded.issues
        );
    }

    #[test]
    fn a_lone_agents_autonomy_key_is_also_the_retired_section() {
        let loaded = Settings::parse(r#"{ "agents": { "autonomy": "always_apply" } }"#);
        assert_eq!(loaded.issues.len(), 1, "{:?}", loaded.issues);
        assert_eq!(loaded.issues[0].path, "agents");
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
