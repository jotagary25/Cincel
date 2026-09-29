//! `keymap.json`: keystrokes to command names.
//!
//! The file is a list of sections, the same shape Zed uses so a user can
//! carry one over (`docs/specs/modulos/workspace.md`):
//!
//! ```jsonc
//! [
//!   { "context": "Editor && review_hunk_under_cursor",
//!     "bindings": { "ctrl-enter": "review::accept_hunk" } }
//! ]
//! ```
//!
//! The user's file is **layered over** the default one rather than replacing
//! it: later sections win, so a user only writes what they want to change.
//! A binding whose command is `null` removes the inherited binding.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::Value;

use crate::context::ContextExpr;
use crate::issue::{Loaded, SettingsIssue};
use crate::jsonc;

/// A single keystroke, as written in `keymap.json` (`ctrl-shift-enter`).
///
/// Modifiers may be written in any order and are normalized to
/// `ctrl-alt-shift-cmd-key` when printed, which is also how GPUI spells them.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Keystroke {
    /// `ctrl`.
    pub ctrl: bool,
    /// `alt`.
    pub alt: bool,
    /// `shift`.
    pub shift: bool,
    /// `cmd` (also spelled `super` or `win`; the spec avoids it on Linux).
    pub cmd: bool,
    /// The key itself, lowercased and de-aliased (`enter`, `escape`, `f7`,
    /// `=`, `a`).
    pub key: String,
}

impl Keystroke {
    /// Parses `ctrl-shift-enter` and friends.
    pub fn parse(source: &str) -> Result<Keystroke, String> {
        let mut keystroke = Keystroke::default();
        let mut rest = source.trim();
        if rest.is_empty() {
            return Err("un atajo no puede estar vacío".to_owned());
        }
        // A modifier is only a modifier when something follows it, so the
        // final `-` of `ctrl--` is the key, not a separator.
        while let Some((head, tail)) = rest.split_once('-') {
            if tail.is_empty() {
                break;
            }
            let slot = match head.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut keystroke.ctrl,
                "alt" | "option" => &mut keystroke.alt,
                "shift" => &mut keystroke.shift,
                "cmd" | "super" | "win" | "meta" => &mut keystroke.cmd,
                _ => break,
            };
            if *slot {
                return Err(format!("«{source}» repite un modificador"));
            }
            *slot = true;
            rest = tail;
        }
        if rest.is_empty() {
            return Err(format!("«{source}» no nombra ninguna tecla"));
        }
        // What is left is the key. `ctrl--` is Ctrl plus the `-` key, but
        // `ctrl-` is a modifier with nothing after it.
        if rest.len() > 1 && rest.ends_with('-') {
            return Err(format!("«{source}» no nombra ninguna tecla"));
        }
        if rest.contains(char::is_whitespace) {
            return Err(format!("«{source}» tiene espacios"));
        }
        keystroke.key = normalize_key(rest);
        Ok(keystroke)
    }

    /// Whether any modifier is held.
    pub fn has_modifiers(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.cmd
    }
}

/// Canonical spelling of a key name.
fn normalize_key(key: &str) -> String {
    let lower = key.to_ascii_lowercase();
    match lower.as_str() {
        "return" => "enter".to_owned(),
        "esc" => "escape".to_owned(),
        "del" => "delete".to_owned(),
        "bs" => "backspace".to_owned(),
        "ins" => "insert".to_owned(),
        "pgup" => "pageup".to_owned(),
        "pgdown" | "pgdn" => "pagedown".to_owned(),
        _ => lower,
    }
}

impl fmt::Display for Keystroke {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("ctrl-")?;
        }
        if self.alt {
            f.write_str("alt-")?;
        }
        if self.shift {
            f.write_str("shift-")?;
        }
        if self.cmd {
            f.write_str("cmd-")?;
        }
        f.write_str(&self.key)
    }
}

impl Serialize for Keystroke {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Keystroke {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Keystroke::parse(&text).map_err(D::Error::custom)
    }
}

/// One keystroke bound to one command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    /// The keystroke.
    pub keystroke: Keystroke,
    /// The command it runs, or `None` to unbind whatever a lower layer bound.
    pub command: Option<String>,
}

/// Where a [`KeymapSection`] came from, for the shortcuts modal
/// (`docs/specs/07-etapa5-productividad.md` §5.2). Not serialized: it is
/// derived while loading, never written back to a file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeymapOrigin {
    /// Built into Cincel (`DEFAULT_KEYMAP_JSONC`).
    #[default]
    Default,
    /// Loaded from the user's `keymap.json`.
    User,
}

/// A group of bindings that share a context.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeymapSection {
    /// When the bindings apply.
    pub context: ContextExpr,
    /// The bindings, sorted by keystroke so the file round-trips.
    pub bindings: Vec<KeyBinding>,
    /// Whether this section is built in or came from the user's file.
    pub origin: KeymapOrigin,
}

impl KeymapSection {
    /// A section for `context` with `bindings`, sorted. Its [`KeymapOrigin`]
    /// starts at [`KeymapOrigin::Default`]; [`Keymap::mark_origin`] relabels
    /// a whole document's sections once it is known to be the user's.
    pub fn new(context: ContextExpr, bindings: Vec<KeyBinding>) -> Self {
        let mut section = Self {
            context,
            bindings,
            origin: KeymapOrigin::Default,
        };
        section
            .bindings
            .sort_by(|a, b| a.keystroke.cmp(&b.keystroke));
        section
    }

    /// The command bound to `keystroke` here, if any.
    ///
    /// The outer `Option` says whether this section has an opinion; the inner
    /// one is `None` for an explicit unbind.
    pub fn get(&self, keystroke: &Keystroke) -> Option<Option<&str>> {
        self.bindings
            .iter()
            .find(|binding| &binding.keystroke == keystroke)
            .map(|binding| binding.command.as_deref())
    }
}

/// One row of the shortcuts modal (`docs/specs/07-etapa5-productividad.md`
/// §5.2): a single (keystroke, context) slot with whichever layer's binding
/// wins there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveBinding {
    /// The keystroke this row is about.
    pub keystroke: Keystroke,
    /// The context the winning binding was declared under.
    pub context: ContextExpr,
    /// The command that runs, or `None` when the user unbound it (`null`).
    pub command: Option<String>,
    /// Whether the winning binding is built in or the user's.
    pub origin: KeymapOrigin,
    /// The command a lower layer bound at the same (keystroke, context),
    /// if any — what this row's binding replaces (or disables, when
    /// `command` is `None`).
    pub replaces: Option<String>,
}

/// The keymap: the default sections plus the user's, in that order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keymap {
    sections: Vec<KeymapSection>,
}

impl Default for Keymap {
    /// The built-in keymap of `docs/specs/02-visual.md` §8.
    fn default() -> Self {
        let loaded = Keymap::parse(crate::defaults::DEFAULT_KEYMAP_JSONC);
        debug_assert!(
            loaded.is_clean(),
            "el keymap predeterminado no debería tener errores: {:?}",
            loaded.issues
        );
        loaded.value
    }
}

impl Keymap {
    /// An empty keymap (no bindings at all).
    pub fn empty() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    /// A keymap made of `sections`.
    pub fn from_sections(sections: Vec<KeymapSection>) -> Self {
        Self { sections }
    }

    /// The sections, lowest priority first.
    pub fn sections(&self) -> &[KeymapSection] {
        &self.sections
    }

    /// Appends `other`'s sections on top of this keymap's, so they win.
    pub fn layer(&mut self, other: Keymap) {
        self.sections.extend(other.sections);
    }

    /// This keymap with `other` layered over it.
    pub fn layered(mut self, other: Keymap) -> Self {
        self.layer(other);
        self
    }

    /// The command `keystroke` runs with these active contexts.
    ///
    /// Sections are consulted from the top layer down, so the user's keymap
    /// wins over the default one; an explicit `null` binding stops the search
    /// and leaves the keystroke unbound.
    pub fn resolve(&self, keystroke: &Keystroke, contexts: &[&str]) -> Option<&str> {
        for section in self.sections.iter().rev() {
            if !section.context.matches(contexts) {
                continue;
            }
            if let Some(command) = section.get(keystroke) {
                return command;
            }
        }
        None
    }

    /// Every keystroke that currently runs `command`, for tooltips and menus.
    pub fn keystrokes_for(&self, command: &str, contexts: &[&str]) -> Vec<Keystroke> {
        let mut seen = Vec::new();
        for section in self.sections.iter().rev() {
            if !section.context.matches(contexts) {
                continue;
            }
            for binding in &section.bindings {
                if seen.contains(&binding.keystroke) {
                    continue;
                }
                if self.resolve(&binding.keystroke, contexts) == Some(command) {
                    seen.push(binding.keystroke.clone());
                }
            }
        }
        seen
    }

    /// Relabels every section's [`KeymapOrigin`] to `origin`, in place.
    ///
    /// Used by [`Keymap::load_from`]: a file is parsed with [`Keymap::parse`]
    /// (which always produces [`KeymapOrigin::Default`] sections, since it
    /// does not know which file it came from), then marked as the user's.
    fn mark_origin(mut self, origin: KeymapOrigin) -> Self {
        for section in &mut self.sections {
            section.origin = origin;
        }
        self
    }

    /// Every binding the keymap declares, one row per distinct (keystroke,
    /// context) pair, for the shortcuts modal
    /// (`docs/specs/07-etapa5-productividad.md` §5.2).
    ///
    /// Sections are walked from the default layer up: a later section with
    /// the exact same keystroke *and* context expression as an earlier one
    /// overrides that row (its `origin` and `command` win, and `replaces`
    /// remembers what it overrode) rather than adding a second row. This is
    /// a display grouping, not [`Keymap::resolve`]'s runtime precedence: two
    /// bindings only collapse into one row when their contexts are written
    /// identically, since that is what the modal shows side by side.
    pub fn effective_bindings(&self) -> Vec<EffectiveBinding> {
        let mut rows: Vec<EffectiveBinding> = Vec::new();
        for section in &self.sections {
            for binding in &section.bindings {
                let existing = rows.iter().position(|row| {
                    row.keystroke == binding.keystroke && row.context == section.context
                });
                match existing {
                    Some(index) => {
                        let replaces = rows[index].command.clone();
                        rows[index] = EffectiveBinding {
                            keystroke: binding.keystroke.clone(),
                            context: section.context.clone(),
                            command: binding.command.clone(),
                            origin: section.origin,
                            replaces,
                        };
                    }
                    None => rows.push(EffectiveBinding {
                        keystroke: binding.keystroke.clone(),
                        context: section.context.clone(),
                        command: binding.command.clone(),
                        origin: section.origin,
                        replaces: None,
                    }),
                }
            }
        }
        rows
    }

    /// Every command the keymap mentions, sorted and deduplicated.
    pub fn commands(&self) -> Vec<&str> {
        let mut commands: Vec<&str> = self
            .sections
            .iter()
            .flat_map(|section| section.bindings.iter())
            .filter_map(|binding| binding.command.as_deref())
            .collect();
        commands.sort_unstable();
        commands.dedup();
        commands
    }

    /// The default keymap with `~/.config/cincel/keymap.json` layered over
    /// it. A missing file is not an issue.
    pub fn load() -> Loaded<Keymap> {
        match crate::paths::Paths::resolve() {
            Some(paths) => Keymap::load_from(&paths.keymap),
            None => Loaded::clean(Keymap::default()),
        }
    }

    /// The default keymap with the file at `path` layered over it.
    pub fn load_from(path: &Path) -> Loaded<Keymap> {
        let user = match std::fs::read_to_string(path) {
            Ok(text) => Keymap::parse(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Loaded::clean(Keymap::default());
            }
            Err(error) => Loaded {
                value: Keymap::empty(),
                issues: vec![SettingsIssue::document(format!(
                    "no se pudo leer «{}»: {error}",
                    path.display()
                ))],
            },
        };
        Loaded {
            value: Keymap::default().layered(user.value.mark_origin(KeymapOrigin::User)),
            issues: user.issues,
        }
    }

    /// Reads one keymap document (without layering it over the default one).
    pub fn parse(text: &str) -> Loaded<Keymap> {
        let mut issues = Vec::new();
        let value = match jsonc::parse(text) {
            Ok(value) => value,
            Err(error) => {
                issues.push(SettingsIssue::document(format!(
                    "no se pudo leer keymap.json ({error}); se ignora"
                )));
                return Loaded {
                    value: Keymap::empty(),
                    issues,
                };
            }
        };
        let sections = match &value {
            Value::Null => Vec::new(),
            Value::Array(items) => {
                let mut sections = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    if let Some(section) = read_section(index, item, &mut issues) {
                        sections.push(section);
                    }
                }
                sections
            }
            other => {
                issues.push(SettingsIssue::document(format!(
                    "keymap.json debe contener una lista de secciones, no {}",
                    jsonc::type_name(other)
                )));
                Vec::new()
            }
        };
        Loaded {
            value: Keymap::from_sections(sections),
            issues,
        }
    }

    /// The keymap as pretty JSON, the format of `keymap.json`.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("un keymap siempre serializa")
    }
}

/// Reads one `{ context, bindings }` object.
fn read_section(
    index: usize,
    item: &Value,
    issues: &mut Vec<SettingsIssue>,
) -> Option<KeymapSection> {
    let at = |key: &str| {
        if key.is_empty() {
            format!("[{index}]")
        } else {
            format!("[{index}].{key}")
        }
    };
    let Value::Object(object) = item else {
        issues.push(SettingsIssue::new(
            at(""),
            format!(
                "cada sección debe ser un objeto, no {}; se ignora",
                jsonc::type_name(item)
            ),
        ));
        return None;
    };
    let context = match object.get("context") {
        None | Some(Value::Null) => ContextExpr::Always,
        Some(Value::String(source)) => match ContextExpr::parse(source) {
            Ok(expr) => expr,
            Err(error) => {
                issues.push(SettingsIssue::new(
                    at("context"),
                    format!("{error}; se ignora la sección"),
                ));
                return None;
            }
        },
        Some(other) => {
            issues.push(SettingsIssue::new(
                at("context"),
                format!(
                    "el contexto debe ser un texto, no {}; se ignora la sección",
                    jsonc::type_name(other)
                ),
            ));
            return None;
        }
    };
    let bindings = match object.get("bindings") {
        None => BTreeMap::new(),
        Some(Value::Object(map)) => map.clone().into_iter().collect::<BTreeMap<_, _>>(),
        Some(other) => {
            issues.push(SettingsIssue::new(
                at("bindings"),
                format!(
                    "«bindings» debe ser un objeto, no {}; se ignora la sección",
                    jsonc::type_name(other)
                ),
            ));
            return None;
        }
    };
    for key in object.keys() {
        if key != "context" && key != "bindings" {
            issues.push(SettingsIssue::new(at(key), "clave desconocida; se ignora"));
        }
    }
    let mut parsed = Vec::new();
    for (source, command) in bindings {
        let keystroke = match Keystroke::parse(&source) {
            Ok(keystroke) => keystroke,
            Err(error) => {
                issues.push(SettingsIssue::new(
                    at(&format!("bindings.{source}")),
                    format!("{error}; se ignora el atajo"),
                ));
                continue;
            }
        };
        let command = match command {
            Value::Null => None,
            Value::String(command) if !command.trim().is_empty() => Some(command),
            other => {
                issues.push(SettingsIssue::new(
                    at(&format!("bindings.{source}")),
                    format!(
                        "el comando debe ser un texto (o null para desligar), no {}; se ignora el atajo",
                        jsonc::type_name(&other)
                    ),
                ));
                continue;
            }
        };
        parsed.push(KeyBinding { keystroke, command });
    }
    Some(KeymapSection::new(context, parsed))
}

impl Serialize for Keymap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.sections.len()))?;
        for section in &self.sections {
            let mut object = serde_json::Map::new();
            if section.context != ContextExpr::Always {
                object.insert(
                    "context".to_owned(),
                    Value::String(section.context.to_string()),
                );
            }
            let bindings: serde_json::Map<String, Value> = section
                .bindings
                .iter()
                .map(|binding| {
                    (
                        binding.keystroke.to_string(),
                        match &binding.command {
                            Some(command) => Value::String(command.clone()),
                            None => Value::Null,
                        },
                    )
                })
                .collect();
            object.insert("bindings".to_owned(), Value::Object(bindings));
            seq.serialize_element(&Value::Object(object))?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Keymap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let mut issues = Vec::new();
        let text = value.to_string();
        let loaded = Keymap::parse(&text);
        issues.extend(loaded.issues);
        if let Some(issue) = issues.first() {
            return Err(D::Error::custom(issue.to_string()));
        }
        Ok(loaded.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(source: &str) -> Keystroke {
        Keystroke::parse(source).unwrap()
    }

    #[test]
    fn keystrokes_normalize() {
        assert_eq!(key("ctrl-enter").to_string(), "ctrl-enter");
        assert_eq!(key("shift-ctrl-Z").to_string(), "ctrl-shift-z");
        assert_eq!(key("CTRL-Return").to_string(), "ctrl-enter");
        assert_eq!(
            key("alt-j"),
            Keystroke {
                alt: true,
                key: "j".to_owned(),
                ..Keystroke::default()
            }
        );
        assert_eq!(key("f7").key, "f7");
        assert_eq!(key("ctrl--").to_string(), "ctrl--");
        assert_eq!(key("ctrl-=").to_string(), "ctrl-=");
        assert_eq!(key("escape").key, "escape");
        assert_eq!(key("esc").key, "escape");
        assert!(!key("enter").has_modifiers());
    }

    #[test]
    fn broken_keystrokes_are_rejected() {
        for source in ["", "ctrl-", "ctrl-ctrl-a", "ctrl a"] {
            assert!(Keystroke::parse(source).is_err(), "{source:?}");
        }
    }

    #[test]
    fn parses_the_documented_shape() {
        let loaded = Keymap::parse(
            r#"[
              { "context": "Editor && review_hunk_under_cursor",
                "bindings": { "ctrl-enter": "review::accept_hunk" } },
              { "bindings": { "ctrl-shift-a": "workspace::toggle_chat" } },
            ]"#,
        );
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        let keymap = loaded.value;
        assert_eq!(
            keymap.resolve(&key("ctrl-enter"), &["Editor", "review_hunk_under_cursor"]),
            Some("review::accept_hunk")
        );
        assert_eq!(keymap.resolve(&key("ctrl-enter"), &["Editor"]), None);
        assert_eq!(
            keymap.resolve(&key("ctrl-shift-a"), &[]),
            Some("workspace::toggle_chat")
        );
    }

    #[test]
    fn user_layer_wins_and_can_unbind() {
        let base = Keymap::parse(
            r#"[{ "bindings": { "ctrl-s": "editor::save", "ctrl-w": "workspace::close_tab" } }]"#,
        )
        .value;
        let user =
            Keymap::parse(r#"[{ "bindings": { "ctrl-s": "editor::save_all", "ctrl-w": null } }]"#)
                .value;
        let keymap = base.layered(user);
        assert_eq!(
            keymap.resolve(&key("ctrl-s"), &[]),
            Some("editor::save_all")
        );
        assert_eq!(keymap.resolve(&key("ctrl-w"), &[]), None);
    }

    #[test]
    fn broken_entries_are_reported_one_by_one() {
        let loaded = Keymap::parse(
            r#"[
              { "context": "Editor &&", "bindings": { "ctrl-a": "x" } },
              { "context": 7, "bindings": {} },
              { "bindings": [1] },
              { "bindings": { "ctrl-": "x", "ctrl-b": 4, "ctrl-c": "editor::copy" }, "otra": 1 },
              "no soy una sección"
            ]"#,
        );
        let paths: Vec<&str> = loaded.issues.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "[0].context",
                "[1].context",
                "[2].bindings",
                "[3].otra",
                "[3].bindings.ctrl-",
                "[3].bindings.ctrl-b",
                "[4]",
            ]
        );
        // The one good binding of section 3 survived.
        assert_eq!(
            loaded.value.resolve(&key("ctrl-c"), &[]),
            Some("editor::copy")
        );
    }

    #[test]
    fn a_non_list_document_is_reported() {
        let loaded = Keymap::parse(r#"{ "ctrl-a": "x" }"#);
        assert_eq!(loaded.issues.len(), 1);
        assert_eq!(loaded.value, Keymap::empty());
        assert!(Keymap::parse("").is_clean());
        assert!(Keymap::parse("[]").is_clean());
    }

    #[test]
    fn round_trips_through_serde() {
        let keymap = Keymap::default();
        let json = keymap.to_json();
        let back: Keymap = serde_json::from_str(&json).unwrap();
        assert_eq!(back, keymap);
        let reparsed = Keymap::parse(&json);
        assert!(reparsed.is_clean(), "{:?}", reparsed.issues);
        assert_eq!(reparsed.value, keymap);
    }

    #[test]
    fn keystrokes_for_a_command() {
        let keymap = Keymap::default();
        let strokes = keymap.keystrokes_for("workspace::next_change", &[]);
        let printed: Vec<String> = strokes.iter().map(|k| k.to_string()).collect();
        assert!(printed.contains(&"alt-j".to_owned()), "{printed:?}");
        assert!(printed.contains(&"f7".to_owned()), "{printed:?}");
    }

    #[test]
    fn missing_user_file_keeps_the_default() {
        let loaded = Keymap::load_from(Path::new("/no/existe/keymap.json"));
        assert!(loaded.is_clean());
        assert_eq!(loaded.value, Keymap::default());
    }

    #[test]
    fn user_file_is_layered_over_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keymap.json");
        std::fs::write(
            &path,
            r#"[{ "context": "Editor", "bindings": { "ctrl-s": "editor::save_all" } }]"#,
        )
        .unwrap();
        let keymap = Keymap::load_from(&path).value;
        assert_eq!(
            keymap.resolve(&key("ctrl-s"), &["Editor"]),
            Some("editor::save_all")
        );
        // Everything else still comes from the default keymap.
        assert_eq!(
            keymap.resolve(&key("ctrl-shift-e"), &[]),
            Some("workspace::toggle_tree")
        );
    }

    #[test]
    fn default_keymap_sections_are_all_default_origin() {
        for section in Keymap::default().sections() {
            assert_eq!(section.origin, KeymapOrigin::Default);
        }
    }

    #[test]
    fn loading_a_user_file_marks_only_its_own_sections_as_user_origin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keymap.json");
        std::fs::write(
            &path,
            r#"[{ "context": "Editor", "bindings": { "ctrl-s": "editor::save_all" } }]"#,
        )
        .unwrap();
        let keymap = Keymap::load_from(&path).value;
        let sections = keymap.sections();
        // Every built-in section still comes first, unchanged.
        let default_count = Keymap::default().sections().len();
        for section in &sections[..default_count] {
            assert_eq!(section.origin, KeymapOrigin::Default);
        }
        // The user's own section, layered on top, is marked as such.
        let user_section = sections.last().unwrap();
        assert_eq!(user_section.origin, KeymapOrigin::User);
        assert_eq!(user_section.context, ContextExpr::parse("Editor").unwrap());
    }

    #[test]
    fn effective_bindings_keeps_one_row_per_keystroke_and_context() {
        let base = Keymap::parse(
            r#"[{ "bindings": { "ctrl-s": "editor::save", "ctrl-w": "workspace::close_tab" } }]"#,
        )
        .value;
        let rows = base.effective_bindings();
        assert_eq!(rows.len(), 2);
        let save = rows
            .iter()
            .find(|row| row.keystroke == key("ctrl-s"))
            .unwrap();
        assert_eq!(save.command.as_deref(), Some("editor::save"));
        assert_eq!(save.origin, KeymapOrigin::Default);
        assert_eq!(save.replaces, None);
    }

    #[test]
    fn effective_bindings_reports_a_user_override_and_what_it_replaces() {
        let base =
            Keymap::parse(r#"[{ "bindings": { "ctrl-shift-a": "workspace::toggle_chat" } }]"#)
                .value;
        let user =
            Keymap::parse(r#"[{ "bindings": { "ctrl-shift-a": "workspace::toggle_tree" } }]"#)
                .value
                .mark_origin(KeymapOrigin::User);
        let rows = base.layered(user).effective_bindings();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.command.as_deref(), Some("workspace::toggle_tree"));
        assert_eq!(row.origin, KeymapOrigin::User);
        assert_eq!(row.replaces.as_deref(), Some("workspace::toggle_chat"));
    }

    #[test]
    fn effective_bindings_reports_a_null_override_as_disabled() {
        let base = Keymap::parse(
            r#"[{ "context": "Editor", "bindings": { "ctrl-g": "editor::go_to_line" } }]"#,
        )
        .value;
        let user = Keymap::parse(r#"[{ "context": "Editor", "bindings": { "ctrl-g": null } }]"#)
            .value
            .mark_origin(KeymapOrigin::User);
        let rows = base.layered(user).effective_bindings();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.command, None, "un `null` desliga el comando");
        assert_eq!(row.origin, KeymapOrigin::User);
        assert_eq!(row.replaces.as_deref(), Some("editor::go_to_line"));
    }

    #[test]
    fn effective_bindings_keeps_separate_rows_for_different_contexts() {
        let keymap = Keymap::parse(
            r#"[
              { "bindings": { "ctrl-enter": "editor::insert_newline" } },
              { "context": "Editor && review_hunk_under_cursor",
                "bindings": { "ctrl-enter": "review::accept_hunk" } },
            ]"#,
        )
        .value;
        let rows = keymap.effective_bindings();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(
            rows.iter()
                .all(|row| row.keystroke == key("ctrl-enter") && row.replaces.is_none()),
            "distinct contexts do not override one another: {rows:?}"
        );
    }
}
