//! Color themes: the tokens of `docs/specs/02-visual.md` §2 plus the twelve
//! tree-sitter capture colors of the same section.
//!
//! A theme is one JSON file in `~/.config/cincel/themes/`. Its keys are the
//! dotted token names of the spec (`"bg.app"`, `"diff.deleted.word"`), so the
//! file reads like the table in the spec; the Rust fields replace the dots
//! with underscores and match `cincel-workspace`'s E0 `Theme` one for one,
//! so that type can be retired in favour of this one.
//!
//! Two themes are built in and always present: `Cincel Dark` (the default,
//! One Dark based) and `Cincel Light` (One Light based). A user theme with
//! the same name replaces the built-in one.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::color::Rgba;
use crate::issue::{Loaded, SettingsIssue};
use crate::jsonc::{self, Reader};
use crate::settings::{ThemeMode, ThemeSettings};

/// Name of the built-in dark theme.
pub const DARK_THEME_NAME: &str = "Cincel Dark";
/// Name of the built-in light theme.
pub const LIGHT_THEME_NAME: &str = "Cincel Light";

/// The twelve standard tree-sitter captures of `02-visual.md` §2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SyntaxTheme {
    /// `keyword`.
    pub keyword: Rgba,
    /// `function`.
    pub function: Rgba,
    /// `type`.
    #[serde(rename = "type")]
    pub type_: Rgba,
    /// `string`.
    pub string: Rgba,
    /// `number`.
    pub number: Rgba,
    /// `comment`.
    pub comment: Rgba,
    /// `variable`.
    pub variable: Rgba,
    /// `property`.
    pub property: Rgba,
    /// `operator`.
    pub operator: Rgba,
    /// `punctuation`.
    pub punctuation: Rgba,
    /// `constant`.
    pub constant: Rgba,
    /// `attribute`.
    pub attribute: Rgba,
}

/// The names of the twelve captures, in spec order.
pub const SYNTAX_KEYS: [&str; 12] = [
    "keyword",
    "function",
    "type",
    "string",
    "number",
    "comment",
    "variable",
    "property",
    "operator",
    "punctuation",
    "constant",
    "attribute",
];

impl SyntaxTheme {
    /// The color of `capture`, or `None` when it is not one of the twelve.
    pub fn get(&self, capture: &str) -> Option<Rgba> {
        // A tree-sitter capture is dotted (`function.method`); the spec only
        // defines the twelve roots, so everything falls back to its root.
        let root = capture.split('.').next().unwrap_or(capture);
        Some(match root {
            "keyword" => self.keyword,
            "function" => self.function,
            "type" => self.type_,
            "string" => self.string,
            "number" => self.number,
            "comment" => self.comment,
            "variable" => self.variable,
            "property" => self.property,
            "operator" => self.operator,
            "punctuation" => self.punctuation,
            "constant" => self.constant,
            "attribute" => self.attribute,
            _ => return None,
        })
    }

    fn slot(&mut self, capture: &str) -> Option<&mut Rgba> {
        Some(match capture {
            "keyword" => &mut self.keyword,
            "function" => &mut self.function,
            "type" => &mut self.type_,
            "string" => &mut self.string,
            "number" => &mut self.number,
            "comment" => &mut self.comment,
            "variable" => &mut self.variable,
            "property" => &mut self.property,
            "operator" => &mut self.operator,
            "punctuation" => &mut self.punctuation,
            "constant" => &mut self.constant,
            "attribute" => &mut self.attribute,
            _ => return None,
        })
    }
}

impl Default for SyntaxTheme {
    fn default() -> Self {
        Self::one_dark()
    }
}

impl SyntaxTheme {
    /// The One Dark palette used by `Cincel Dark`.
    pub const fn one_dark() -> Self {
        Self {
            keyword: Rgba::hex(0xc678dd),
            function: Rgba::hex(0x61afef),
            type_: Rgba::hex(0xe5c07b),
            string: Rgba::hex(0x98c379),
            number: Rgba::hex(0xd19a66),
            comment: Rgba::hex(0x5c6370),
            variable: Rgba::hex(0xabb2bf),
            property: Rgba::hex(0xe06c75),
            operator: Rgba::hex(0x56b6c2),
            punctuation: Rgba::hex(0xabb2bf),
            constant: Rgba::hex(0xd19a66),
            attribute: Rgba::hex(0xd19a66),
        }
    }

    /// The One Light palette used by `Cincel Light`.
    pub const fn one_light() -> Self {
        Self {
            keyword: Rgba::hex(0xa626a4),
            function: Rgba::hex(0x4078f2),
            type_: Rgba::hex(0xc18401),
            string: Rgba::hex(0x50a14f),
            number: Rgba::hex(0x986801),
            comment: Rgba::hex(0xa0a1a7),
            variable: Rgba::hex(0x383a42),
            property: Rgba::hex(0xe45649),
            operator: Rgba::hex(0x0184bc),
            punctuation: Rgba::hex(0x383a42),
            constant: Rgba::hex(0x986801),
            attribute: Rgba::hex(0x986801),
        }
    }
}

/// A color theme: every token of `02-visual.md` §2 plus [`SyntaxTheme`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    /// Name shown in the settings and used to select the theme.
    pub name: String,
    /// Whether this is a dark theme (drives the system-follows mode and the
    /// gpui-kit theme mode).
    pub dark: bool,
    /// `bg.app`: panel background.
    #[serde(rename = "bg.app")]
    pub bg_app: Rgba,
    /// `bg.editor`: editor background.
    #[serde(rename = "bg.editor")]
    pub bg_editor: Rgba,
    /// `bg.surface`: cards and inputs.
    #[serde(rename = "bg.surface")]
    pub bg_surface: Rgba,
    /// `bg.elevated`: menus, popovers, the floating bar.
    #[serde(rename = "bg.elevated")]
    pub bg_elevated: Rgba,
    /// `border`: panel borders.
    pub border: Rgba,
    /// `border.focus`: focus ring.
    #[serde(rename = "border.focus")]
    pub border_focus: Rgba,
    /// `text`: main text.
    pub text: Rgba,
    /// `text.muted`: secondary text and line numbers.
    #[serde(rename = "text.muted")]
    pub text_muted: Rgba,
    /// `text.accent`: links and active items.
    #[serde(rename = "text.accent")]
    pub text_accent: Rgba,
    /// `cursor`: the caret.
    pub cursor: Rgba,
    /// `selection`.
    pub selection: Rgba,
    /// `diff.deleted.bg`: background of a deleted line.
    #[serde(rename = "diff.deleted.bg")]
    pub diff_deleted_bg: Rgba,
    /// `diff.deleted.word`: a deleted word inside a deleted line.
    #[serde(rename = "diff.deleted.word")]
    pub diff_deleted_word: Rgba,
    /// `diff.added.bg`: background of an added line.
    #[serde(rename = "diff.added.bg")]
    pub diff_added_bg: Rgba,
    /// `diff.added.word`: an added word inside an added line.
    #[serde(rename = "diff.added.word")]
    pub diff_added_word: Rgba,
    /// `diff.gutter.deleted`.
    #[serde(rename = "diff.gutter.deleted")]
    pub diff_gutter_deleted: Rgba,
    /// `diff.gutter.added`.
    #[serde(rename = "diff.gutter.added")]
    pub diff_gutter_added: Rgba,
    /// `diff.gutter.modified`.
    #[serde(rename = "diff.gutter.modified")]
    pub diff_gutter_modified: Rgba,
    /// `status.error`.
    #[serde(rename = "status.error")]
    pub status_error: Rgba,
    /// `status.warning`.
    #[serde(rename = "status.warning")]
    pub status_warning: Rgba,
    /// `status.ok`.
    #[serde(rename = "status.ok")]
    pub status_ok: Rgba,
    /// The twelve syntax colors.
    pub syntax: SyntaxTheme,
}

/// The dotted names of the twenty-one color tokens, in spec order.
pub const COLOR_KEYS: [&str; 21] = [
    "bg.app",
    "bg.editor",
    "bg.surface",
    "bg.elevated",
    "border",
    "border.focus",
    "text",
    "text.muted",
    "text.accent",
    "cursor",
    "selection",
    "diff.deleted.bg",
    "diff.deleted.word",
    "diff.added.bg",
    "diff.added.word",
    "diff.gutter.deleted",
    "diff.gutter.added",
    "diff.gutter.modified",
    "status.error",
    "status.warning",
    "status.ok",
];

impl Default for Theme {
    fn default() -> Self {
        Self::cincel_dark()
    }
}

impl Theme {
    /// The built-in dark theme, exactly the table of `02-visual.md` §2.
    pub fn cincel_dark() -> Self {
        let red = Rgba::hex(0xe06c75);
        let green = Rgba::hex(0x98c379);
        let yellow = Rgba::hex(0xe5c07b);
        Self {
            name: DARK_THEME_NAME.to_owned(),
            dark: true,
            bg_app: Rgba::hex(0x1e2127),
            bg_editor: Rgba::hex(0x282c34),
            bg_surface: Rgba::hex(0x21252b),
            bg_elevated: Rgba::hex(0x2c313a),
            border: Rgba::hex(0x181a1f),
            border_focus: Rgba::hex(0x528bff),
            text: Rgba::hex(0xabb2bf),
            text_muted: Rgba::hex(0x5c6370),
            text_accent: Rgba::hex(0x61afef),
            cursor: Rgba::hex(0x528bff),
            selection: Rgba::hex(0x3e4451),
            diff_deleted_bg: red.alpha(0.18),
            diff_deleted_word: red.alpha(0.38),
            diff_added_bg: green.alpha(0.18),
            diff_added_word: green.alpha(0.38),
            diff_gutter_deleted: red,
            diff_gutter_added: green,
            diff_gutter_modified: yellow,
            status_error: red,
            status_warning: yellow,
            status_ok: green,
            syntax: SyntaxTheme::one_dark(),
        }
    }

    /// The built-in light theme.
    ///
    /// Derived from the dark one token by token: the One Dark greys become
    /// their One Light counterparts (background and text swap roles, the
    /// elevated surface becomes the lightest rather than the darkest), and
    /// the three accent hues become the One Light ones, keeping the same
    /// tint percentages for the diff backgrounds so the diff reads the same.
    pub fn cincel_light() -> Self {
        let red = Rgba::hex(0xe45649);
        let green = Rgba::hex(0x50a14f);
        let yellow = Rgba::hex(0xc18401);
        Self {
            name: LIGHT_THEME_NAME.to_owned(),
            dark: false,
            bg_app: Rgba::hex(0xf0f0f1),
            bg_editor: Rgba::hex(0xfafafa),
            bg_surface: Rgba::hex(0xeaeaeb),
            bg_elevated: Rgba::hex(0xffffff),
            border: Rgba::hex(0xd4d4d6),
            border_focus: Rgba::hex(0x4078f2),
            text: Rgba::hex(0x383a42),
            text_muted: Rgba::hex(0x9d9d9f),
            text_accent: Rgba::hex(0x4078f2),
            cursor: Rgba::hex(0x526fff),
            selection: Rgba::hex(0xcfd1d6),
            diff_deleted_bg: red.alpha(0.18),
            diff_deleted_word: red.alpha(0.38),
            diff_added_bg: green.alpha(0.18),
            diff_added_word: green.alpha(0.38),
            diff_gutter_deleted: red,
            diff_gutter_added: green,
            diff_gutter_modified: yellow,
            status_error: red,
            status_warning: yellow,
            status_ok: green,
            syntax: SyntaxTheme::one_light(),
        }
    }

    /// Both built-in themes, dark first.
    pub fn builtin() -> Vec<Theme> {
        vec![Theme::cincel_dark(), Theme::cincel_light()]
    }

    /// Reads a theme from JSONC text, defaulting every rejected token.
    ///
    /// The defaults come from the built-in theme of the same appearance, so a
    /// partial theme file (only `bg.app` and `text`, say) is a valid theme.
    pub fn parse(text: &str) -> Loaded<Theme> {
        Theme::parse_named(text, None)
    }

    /// Same as [`Theme::parse`], but a theme with no `name` takes
    /// `fallback_name` (the file stem) instead of the built-in theme's name,
    /// so an unnamed user theme does not shadow a built-in one.
    fn parse_named(text: &str, fallback_name: Option<&str>) -> Loaded<Theme> {
        let mut issues = Vec::new();
        let value = match jsonc::parse(text) {
            Ok(value) => value,
            Err(error) => {
                issues.push(SettingsIssue::document(format!(
                    "no se pudo leer el tema: {error}"
                )));
                return Loaded {
                    value: Theme::default(),
                    issues,
                };
            }
        };
        let theme = Reader::document(&value, &mut issues, |reader, object| {
            Theme::read(reader, object, fallback_name)
        });
        Loaded {
            value: theme,
            issues,
        }
    }

    /// Reads a theme file, defaulting when it cannot be read at all.
    pub fn load(path: &Path) -> Loaded<Theme> {
        let fallback = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned());
        match std::fs::read_to_string(path) {
            Ok(text) => Theme::parse_named(&text, fallback.as_deref()),
            Err(error) => Loaded {
                value: Theme::default(),
                issues: vec![SettingsIssue::document(format!(
                    "no se pudo leer «{}»: {error}",
                    path.display()
                ))],
            },
        }
    }

    /// The theme as pretty JSON, the format of a theme file.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("un tema siempre serializa")
    }

    fn read(
        reader: &mut Reader<'_>,
        object: &Map<String, Value>,
        fallback_name: Option<&str>,
    ) -> Theme {
        // `dark` first: it decides which built-in theme fills the gaps.
        let dark = reader.field(object, "dark", true);
        let mut theme = if dark {
            Theme::cincel_dark()
        } else {
            Theme::cincel_light()
        };
        let default_name = fallback_name
            .map(str::to_owned)
            .unwrap_or_else(|| theme.name.clone());
        theme.name = reader.checked_field(object, "name", default_name, |name: &String| {
            if name.trim().is_empty() {
                Err("el nombre del tema no puede estar vacío".to_owned())
            } else {
                Ok(())
            }
        });
        for key in COLOR_KEYS {
            if !object.contains_key(key) {
                continue;
            }
            let current = *theme.slot(key).expect("COLOR_KEYS names a real token");
            let color = reader.field(object, key, current);
            *theme.slot(key).expect("idem") = color;
        }
        if object.contains_key("syntax") {
            theme.syntax = reader.object(object, "syntax", |reader, syntax| {
                let mut colors = if dark {
                    SyntaxTheme::one_dark()
                } else {
                    SyntaxTheme::one_light()
                };
                for key in SYNTAX_KEYS {
                    if !syntax.contains_key(key) {
                        continue;
                    }
                    let current = *colors.slot(key).expect("SYNTAX_KEYS names a capture");
                    let color = reader.field(syntax, key, current);
                    *colors.slot(key).expect("idem") = color;
                }
                reader.unknown_keys(syntax, &SYNTAX_KEYS);
                colors
            });
        }
        let mut known: Vec<&str> = COLOR_KEYS.to_vec();
        known.extend_from_slice(&["name", "dark", "syntax"]);
        reader.unknown_keys(object, &known);
        theme
    }

    fn slot(&mut self, token: &str) -> Option<&mut Rgba> {
        Some(match token {
            "bg.app" => &mut self.bg_app,
            "bg.editor" => &mut self.bg_editor,
            "bg.surface" => &mut self.bg_surface,
            "bg.elevated" => &mut self.bg_elevated,
            "border" => &mut self.border,
            "border.focus" => &mut self.border_focus,
            "text" => &mut self.text,
            "text.muted" => &mut self.text_muted,
            "text.accent" => &mut self.text_accent,
            "cursor" => &mut self.cursor,
            "selection" => &mut self.selection,
            "diff.deleted.bg" => &mut self.diff_deleted_bg,
            "diff.deleted.word" => &mut self.diff_deleted_word,
            "diff.added.bg" => &mut self.diff_added_bg,
            "diff.added.word" => &mut self.diff_added_word,
            "diff.gutter.deleted" => &mut self.diff_gutter_deleted,
            "diff.gutter.added" => &mut self.diff_gutter_added,
            "diff.gutter.modified" => &mut self.diff_gutter_modified,
            "status.error" => &mut self.status_error,
            "status.warning" => &mut self.status_warning,
            "status.ok" => &mut self.status_ok,
            _ => return None,
        })
    }
}

/// Every theme Cincel can use: the two built-in ones plus whatever lives in
/// `~/.config/cincel/themes/`.
#[derive(Clone, Debug)]
pub struct ThemeRegistry {
    themes: Vec<Theme>,
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ThemeRegistry {
    /// Only the built-in themes.
    pub fn builtin() -> Self {
        Self {
            themes: Theme::builtin(),
        }
    }

    /// The built-in themes plus every `*.json` in `dir`.
    ///
    /// A user theme named like a built-in one replaces it. A missing
    /// directory is not an issue; an unreadable file or a broken theme is.
    pub fn load(dir: &Path) -> Loaded<ThemeRegistry> {
        let mut registry = ThemeRegistry::builtin();
        let mut issues = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Loaded::clean(registry);
            }
            Err(error) => {
                issues.push(SettingsIssue::document(format!(
                    "no se pudo listar «{}»: {error}",
                    dir.display()
                )));
                return Loaded {
                    value: registry,
                    issues,
                };
            }
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        files.sort();
        for file in files {
            let name = file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let loaded = Theme::load(&file);
            // A file we could not read at all yields the *default* theme,
            // which would shadow a built-in one; only its issues are kept.
            let unreadable = loaded.issues.iter().any(|issue| issue.path == "$");
            issues.extend(loaded.issues.into_iter().map(|issue| issue.in_file(&name)));
            if !unreadable {
                registry.insert(loaded.value);
            }
        }
        Loaded {
            value: registry,
            issues,
        }
    }

    /// The registry for the real XDG themes directory.
    pub fn load_default() -> Loaded<ThemeRegistry> {
        match crate::paths::Paths::resolve() {
            Some(paths) => ThemeRegistry::load(&paths.themes),
            None => Loaded::clean(ThemeRegistry::builtin()),
        }
    }

    /// Adds `theme`, replacing any theme with the same name.
    pub fn insert(&mut self, theme: Theme) {
        match self
            .themes
            .iter_mut()
            .find(|existing| existing.name == theme.name)
        {
            Some(existing) => *existing = theme,
            None => self.themes.push(theme),
        }
    }

    /// The theme called `name`.
    pub fn get(&self, name: &str) -> Option<&Theme> {
        self.themes.iter().find(|theme| theme.name == name)
    }

    /// Every theme, built-ins first.
    pub fn themes(&self) -> &[Theme] {
        &self.themes
    }

    /// Every theme name, for the theme picker.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.themes.iter().map(|theme| theme.name.as_str())
    }

    /// The theme `settings` selects.
    ///
    /// `system_dark` is what the desktop portal reports and only matters in
    /// [`ThemeMode::System`]. A name that does not exist falls back to the
    /// built-in theme of the right appearance, which is always present.
    pub fn resolve(&self, settings: &ThemeSettings, system_dark: bool) -> &Theme {
        let dark = match settings.mode {
            ThemeMode::System => system_dark,
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
        };
        let wanted = if dark {
            &settings.dark
        } else {
            &settings.light
        };
        self.get(wanted)
            .or_else(|| {
                self.get(if dark {
                    DARK_THEME_NAME
                } else {
                    LIGHT_THEME_NAME
                })
            })
            .unwrap_or(&self.themes[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_matches_the_spec_table() {
        let theme = Theme::cincel_dark();
        assert_eq!(theme.bg_app, Rgba::hex(0x1e2127));
        assert_eq!(theme.bg_editor, Rgba::hex(0x282c34));
        assert_eq!(theme.bg_surface, Rgba::hex(0x21252b));
        assert_eq!(theme.bg_elevated, Rgba::hex(0x2c313a));
        assert_eq!(theme.border, Rgba::hex(0x181a1f));
        assert_eq!(theme.border_focus, Rgba::hex(0x528bff));
        assert_eq!(theme.text, Rgba::hex(0xabb2bf));
        assert_eq!(theme.text_muted, Rgba::hex(0x5c6370));
        assert_eq!(theme.text_accent, Rgba::hex(0x61afef));
        assert_eq!(theme.cursor, Rgba::hex(0x528bff));
        assert_eq!(theme.selection, Rgba::hex(0x3e4451));
        assert_eq!(theme.diff_gutter_deleted, Rgba::hex(0xe06c75));
        assert_eq!(theme.diff_gutter_added, Rgba::hex(0x98c379));
        assert_eq!(theme.diff_gutter_modified, Rgba::hex(0xe5c07b));
        assert_eq!(theme.status_error, Rgba::hex(0xe06c75));
        assert_eq!(theme.status_warning, Rgba::hex(0xe5c07b));
        assert_eq!(theme.status_ok, Rgba::hex(0x98c379));
        assert!((theme.diff_deleted_bg.alpha_f32() - 0.18).abs() < 0.005);
        assert!((theme.diff_added_word.alpha_f32() - 0.38).abs() < 0.005);
    }

    #[test]
    fn json_carries_exactly_the_spec_tokens() {
        let value = serde_json::to_value(Theme::cincel_dark()).unwrap();
        let object = value.as_object().unwrap();
        for key in COLOR_KEYS {
            assert!(object.contains_key(key), "falta el token {key}");
        }
        assert_eq!(object.len(), COLOR_KEYS.len() + 3); // + name, dark, syntax
        let syntax = object["syntax"].as_object().unwrap();
        assert_eq!(syntax.len(), SYNTAX_KEYS.len());
        for key in SYNTAX_KEYS {
            assert!(syntax.contains_key(key), "falta la captura {key}");
        }
    }

    #[test]
    fn round_trips_through_serde() {
        for theme in Theme::builtin() {
            let json = theme.to_json();
            let back: Theme = serde_json::from_str(&json).unwrap();
            assert_eq!(back, theme);
            // And through the tolerant reader, which must agree with serde.
            let loaded = Theme::parse(&json);
            assert!(loaded.is_clean(), "{:?}", loaded.issues);
            assert_eq!(loaded.value, theme);
        }
    }

    #[test]
    fn partial_theme_inherits_the_builtin_of_its_appearance() {
        let loaded = Theme::parse(
            r##"{
              // solo cambio el fondo
              "name": "Mío",
              "bg.app": "#000000",
              "syntax": { "keyword": "#ff0000" },
            }"##,
        );
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        let theme = loaded.value;
        assert_eq!(theme.name, "Mío");
        assert_eq!(theme.bg_app, Rgba::hex(0x000000));
        assert_eq!(theme.bg_editor, Theme::cincel_dark().bg_editor);
        assert_eq!(theme.syntax.keyword, Rgba::hex(0xff0000));
        assert_eq!(theme.syntax.string, SyntaxTheme::one_dark().string);
    }

    #[test]
    fn light_theme_inherits_light_defaults() {
        let loaded = Theme::parse(r#"{ "name": "Claro", "dark": false }"#);
        assert!(loaded.is_clean());
        assert_eq!(loaded.value.bg_app, Theme::cincel_light().bg_app);
        assert_eq!(
            loaded.value.syntax.keyword,
            SyntaxTheme::one_light().keyword
        );
    }

    #[test]
    fn broken_values_are_reported_and_defaulted() {
        let loaded = Theme::parse(
            r#"{ "name": "", "bg.app": "azul", "dark": "sí", "nope": 1,
                 "syntax": { "keyword": 5 } }"#,
        );
        let paths: Vec<&str> = loaded.issues.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"bg.app"), "{paths:?}");
        assert!(paths.contains(&"dark"), "{paths:?}");
        assert!(paths.contains(&"name"), "{paths:?}");
        assert!(paths.contains(&"nope"), "{paths:?}");
        assert!(paths.contains(&"syntax.keyword"), "{paths:?}");
        assert_eq!(loaded.value, Theme::cincel_dark());
    }

    #[test]
    fn broken_syntax_yields_the_default_theme() {
        let loaded = Theme::parse("{ esto no es json");
        assert_eq!(loaded.value, Theme::default());
        assert_eq!(loaded.issues.len(), 1);
        assert_eq!(loaded.issues[0].path, "$");
    }

    #[test]
    fn registry_prefers_user_themes_and_resolves_modes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("mio.json"),
            r##"{ "name": "Cincel Dark", "bg.app": "#010203" }"##,
        )
        .unwrap();
        std::fs::write(dir.path().join("roto.json"), "{").unwrap();
        let loaded = ThemeRegistry::load(dir.path());
        assert_eq!(loaded.issues.len(), 1);
        assert!(loaded.issues[0].path.starts_with("roto.json"));
        let registry = loaded.value;
        assert_eq!(
            registry.get(DARK_THEME_NAME).unwrap().bg_app,
            Rgba::hex(0x010203)
        );

        let settings = ThemeSettings::default();
        assert_eq!(registry.resolve(&settings, true).name, DARK_THEME_NAME);
        assert_eq!(registry.resolve(&settings, false).name, LIGHT_THEME_NAME);
        let forced = ThemeSettings {
            mode: ThemeMode::Dark,
            ..ThemeSettings::default()
        };
        assert_eq!(registry.resolve(&forced, false).name, DARK_THEME_NAME);
        let missing = ThemeSettings {
            mode: ThemeMode::Light,
            light: "No existe".to_owned(),
            ..ThemeSettings::default()
        };
        assert_eq!(registry.resolve(&missing, false).name, LIGHT_THEME_NAME);
    }

    #[test]
    fn missing_themes_directory_is_not_an_issue() {
        let loaded = ThemeRegistry::load(Path::new("/no/existe/themes"));
        assert!(loaded.is_clean());
        assert_eq!(loaded.value.themes().len(), 2);
    }

    #[test]
    fn syntax_lookup_falls_back_to_the_capture_root() {
        let syntax = SyntaxTheme::one_dark();
        assert_eq!(syntax.get("function.method"), Some(syntax.function));
        assert_eq!(syntax.get("type"), Some(syntax.type_));
        assert_eq!(syntax.get("desconocido"), None);
    }
}
