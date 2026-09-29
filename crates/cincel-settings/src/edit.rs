//! Editing `settings.json` in place, keeping comments and everything else
//! byte for byte outside the touched value.
//!
//! `docs/specs/07-etapa5-productividad.md` §4.3 (D3): the settings tab writes
//! through [`apply_edit`]/[`write_edit`] while a user can still edit the file
//! by hand; both paths converge on the same document, so nothing here may
//! reformat what it does not touch. That rules out a round trip through
//! [`serde_json::Value`] (which forgets comments and key order) in favour of
//! `jsonc-parser`'s `cst` feature: it parses into a mutable concrete syntax
//! tree that keeps every comment, blank line and trailing comma as a node,
//! and only the nodes an edit touches are replaced.
//!
//! The two operations the settings tab needs are [`SettingsEdit::Set`] (write
//! a value at a dotted path, creating intermediate objects as needed) and
//! [`SettingsEdit::Remove`] (delete a key, leaving its parent object in place
//! even if it becomes empty — that parent might be `agents`, a section the
//! user still has other keys under).

use std::io::Write as _;
use std::path::Path;

use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use serde_json::Value;

use crate::paths::Paths;

/// The text a brand new `settings.json` starts from: a missing or empty file
/// is treated as this document, not as `{}`, so the first edit a user makes
/// still explains itself (`docs/specs/07-etapa5-productividad.md` §4.3).
const EMPTY_DOCUMENT: &str = "{\n  // Ajustes de Cincel. `cincel --print-default-settings` muestra todos con su explicación.\n}\n";

/// One change to apply to `settings.json`.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsEdit {
    /// Writes `value` at `path`, creating any missing intermediate object.
    ///
    /// `path` is the sequence of object keys to descend through, e.g.
    /// `&["editor", "soft_wrap"]` for `editor.soft_wrap`. It must not be
    /// empty.
    Set {
        /// Dotted path of the key to write, most significant segment first.
        path: &'static [&'static str],
        /// The new value, in its `serde_json` shape (numbers keep whatever
        /// representation `serde_json` gives them, so `1.6` stays `1.6`).
        value: Value,
    },
    /// Removes the key at `path`, if present. A missing key (or a missing
    /// intermediate object) is not an error: there is nothing to do.
    ///
    /// The parent object is kept even if this empties it out — it may still
    /// carry keys this crate does not know about (`docs/specs/modulos/settings.md`).
    Remove {
        /// Dotted path of the key to remove, most significant segment first.
        path: &'static [&'static str],
    },
}

/// Something that kept [`apply_edit`] or [`write_edit`] from succeeding.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    /// The document has a syntax error, at the given 1-indexed position.
    #[error("error de sintaxis en la línea {line}, columna {column}: {message}")]
    Syntax {
        /// 1-indexed line of the error.
        line: usize,
        /// 1-indexed column of the error.
        column: usize,
        /// Human-readable explanation, in whatever language `jsonc-parser`
        /// gives it (English); the caller decides how to present it.
        message: String,
    },
    /// Reading or writing the file failed.
    #[error("no se pudo leer o escribir settings.json: {0}")]
    Io(#[from] std::io::Error),
    /// The document's top level is not a JSON object, so there is nowhere to
    /// set or remove a key without destroying whatever *is* there.
    #[error("settings.json no es un objeto JSON")]
    NotAnObject,
}

/// Applies `edit` to `text`, returning the whole document with the change
/// made and everything else preserved byte for byte.
///
/// A blank `text` (matching a missing or empty file, `docs/specs/07-etapa5-productividad.md`
/// §4.3) starts from [`EMPTY_DOCUMENT`] instead of a bare `{}`, so the file
/// keeps its explanatory comment even on the very first edit.
pub fn apply_edit(text: &str, edit: &SettingsEdit) -> Result<String, EditError> {
    let text = if text.trim().is_empty() {
        EMPTY_DOCUMENT
    } else {
        text
    };
    let root = CstRootNode::parse(text, &crate::jsonc::parse_options()).map_err(|error| {
        EditError::Syntax {
            line: error.line_display(),
            column: error.column_display(),
            message: error.to_string(),
        }
    })?;
    // `object_value_or_set` would happily overwrite a non-object root (an
    // array, a bare string); that would silently destroy the file, so a
    // document whose root already holds something else is refused instead.
    if let Some(value) = root.value()
        && value.as_object().is_none()
    {
        return Err(EditError::NotAnObject);
    }
    let object = root.object_value_or_set();
    match edit {
        SettingsEdit::Set { path, value } => set_path(&object, path, value),
        SettingsEdit::Remove { path } => remove_path(&object, path),
    }
    Ok(root.to_string())
}

/// Reads `paths.settings`, applies `edit`, and writes the result back
/// atomically (temp file in the same directory, then rename).
///
/// The file is re-read right before writing (this function's own read, with
/// no caching), so a change made by hand between two calls is never
/// clobbered: each call sees the latest content on disk.
pub fn write_edit(paths: &Paths, edit: &SettingsEdit) -> Result<(), EditError> {
    let text = read_or_default(&paths.settings)?;
    let updated = apply_edit(&text, edit)?;
    atomic_write(&paths.settings, &updated)
}

/// The current content of `path`, or [`EMPTY_DOCUMENT`]'s trigger (an empty
/// string, which [`apply_edit`] expands) when the file does not exist.
fn read_or_default(path: &Path) -> Result<String, EditError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(EditError::Io(error)),
    }
}

/// Writes `contents` to `path` atomically: a `<name>.tmp` file in the same
/// directory, `fsync`ed, then renamed over `path`. The original's
/// permissions are kept when the file already existed.
fn atomic_write(path: &Path, contents: &str) -> Result<(), EditError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file_name = path.file_name().map(|name| {
        let mut name = name.to_os_string();
        name.push(".tmp");
        name
    });
    let tmp_path = match file_name {
        Some(name) => path.with_file_name(name),
        None => path.with_extension("tmp"),
    };
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }
    if let Ok(metadata) = std::fs::metadata(path) {
        // Best effort: a permission we cannot set (e.g. a read-only mount)
        // should not keep the write itself from succeeding.
        let _ = std::fs::set_permissions(&tmp_path, metadata.permissions());
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Sets `path`'s value on `object`, creating intermediate objects and
/// appending or overwriting the final key as needed.
fn set_path(object: &CstObject, path: &[&str], value: &Value) {
    let Some((last, init)) = path.split_last() else {
        return;
    };
    let mut current = object.clone();
    for segment in init {
        current = current.object_value_or_set(segment);
    }
    let input = to_cst_value(value);
    match current.get(last) {
        Some(prop) => prop.set_value(input),
        None => {
            current.append(last, input);
        }
    }
}

/// Removes `path`'s key from `object`, if the key (and every intermediate
/// object along the way) exists. A missing key is a no-op, not an error.
fn remove_path(object: &CstObject, path: &[&str]) {
    let Some((last, init)) = path.split_last() else {
        return;
    };
    let mut current = object.clone();
    for segment in init {
        match current.object_value(segment) {
            Some(next) => current = next,
            // The intermediate object does not exist, so neither does the key.
            None => return,
        }
    }
    if let Some(prop) = current.get(last) {
        prop.remove();
    }
}

/// Converts a `serde_json::Value` into the shape `jsonc-parser`'s CST wants
/// to insert. Numbers keep `serde_json`'s own textual representation
/// (`Number::to_string`), so `1.6` is written as `1.6`, not `1.6000000000000001`
/// or `1`.
fn to_cst_value(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(flag) => CstInputValue::Bool(*flag),
        Value::Number(number) => CstInputValue::Number(number.to_string()),
        Value::String(text) => CstInputValue::String(text.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(to_cst_value).collect()),
        Value::Object(map) => CstInputValue::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), to_cst_value(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn soft_wrap_edit() -> SettingsEdit {
        SettingsEdit::Set {
            path: &["editor", "soft_wrap"],
            value: Value::Bool(false),
        }
    }

    #[test]
    fn sets_a_new_nested_key_keeping_the_rest_of_the_file() {
        let original = "{\n  // comentario del usuario\n  \"ui_font_size\": 13,\n\n  \"editor\": {\n    // otro comentario\n    \"tab_size\": 2\n  }\n}\n";
        let updated = apply_edit(original, &soft_wrap_edit()).unwrap();
        assert!(updated.contains("// comentario del usuario"));
        assert!(updated.contains("// otro comentario"));
        assert!(updated.contains("\"ui_font_size\": 13"));
        assert!(updated.contains("\"tab_size\": 2"));
        assert!(updated.contains("\"soft_wrap\": false"));
        let loaded = Settings::parse(&updated);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert!(!loaded.value.editor.soft_wrap);
    }

    #[test]
    fn replaces_an_existing_value() {
        let original =
            "{\n  // se conserva\n  \"editor\": { \"soft_wrap\": true, \"tab_size\": 4 }\n}\n";
        let updated = apply_edit(original, &soft_wrap_edit()).unwrap();
        assert!(updated.contains("// se conserva"));
        assert!(updated.contains("\"soft_wrap\": false"));
        assert!(updated.contains("\"tab_size\": 4"));
        let loaded = Settings::parse(&updated);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
    }

    #[test]
    fn removes_a_key() {
        let original = "{\n  \"ui_font_size\": 13,\n  \"editor\": { \"soft_wrap\": false, \"tab_size\": 4 }\n}\n";
        let updated = apply_edit(
            original,
            &SettingsEdit::Remove {
                path: &["editor", "soft_wrap"],
            },
        )
        .unwrap();
        assert!(!updated.contains("soft_wrap"));
        assert!(updated.contains("\"tab_size\": 4"));
        assert!(updated.contains("\"ui_font_size\": 13"));
        let loaded = Settings::parse(&updated);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert!(loaded.value.editor.soft_wrap, "vuelve al predeterminado");
    }

    #[test]
    fn removing_the_last_key_of_a_section_keeps_the_empty_object() {
        let original = "{ \"editor\": { \"soft_wrap\": false } }";
        let updated = apply_edit(
            original,
            &SettingsEdit::Remove {
                path: &["editor", "soft_wrap"],
            },
        )
        .unwrap();
        let loaded = Settings::parse(&updated);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        // `editor` itself is still there, just empty; a future `agents`-style
        // section with keys this crate does not know about must survive the
        // same way.
        let value = crate::jsonc::parse(&updated).unwrap();
        assert!(value.get("editor").is_some(), "{updated:?}");
    }

    #[test]
    fn missing_key_removal_is_a_harmless_no_op() {
        let original = "{ \"ui_font_size\": 13 }";
        let updated = apply_edit(
            original,
            &SettingsEdit::Remove {
                path: &["no", "existe"],
            },
        )
        .unwrap();
        assert!(Settings::parse(&updated).is_clean());
        assert!(updated.contains("\"ui_font_size\": 13"));
    }

    #[test]
    fn empty_or_missing_file_starts_from_the_explained_document() {
        for original in ["", "   \n\t"] {
            let updated = apply_edit(original, &soft_wrap_edit()).unwrap();
            assert!(updated.contains("Ajustes de Cincel"));
            assert!(updated.contains("\"soft_wrap\": false"));
            let loaded = Settings::parse(&updated);
            assert!(loaded.is_clean(), "{:?}", loaded.issues);
        }
    }

    #[test]
    fn syntax_error_reports_line_and_column() {
        let error = apply_edit("{ \"ui_font_size\": }", &soft_wrap_edit()).unwrap_err();
        match error {
            EditError::Syntax { line, .. } => assert_eq!(line, 1),
            other => panic!("se esperaba EditError::Syntax, no {other:?}"),
        }
    }

    #[test]
    fn a_non_object_root_is_rejected() {
        let error = apply_edit("[1, 2, 3]", &soft_wrap_edit()).unwrap_err();
        assert!(matches!(error, EditError::NotAnObject));
    }

    #[test]
    fn the_retired_agents_section_and_unknown_keys_survive() {
        let original = "{\n  \"agents\": { \"default\": \"codex-acp\" },\n  \"un_ajuste_desconocido\": 1,\n  \"editor\": { \"soft_wrap\": true }\n}\n";
        let updated = apply_edit(original, &soft_wrap_edit()).unwrap();
        assert!(updated.contains("\"agents\": { \"default\": \"codex-acp\" }"));
        assert!(updated.contains("\"un_ajuste_desconocido\": 1"));
        assert!(updated.contains("\"soft_wrap\": false"));
    }

    #[test]
    fn trailing_commas_are_preserved() {
        let original = "{\n  \"ui_font_size\": 13,\n  \"editor\": { \"soft_wrap\": true, },\n}\n";
        let updated = apply_edit(original, &soft_wrap_edit()).unwrap();
        assert!(Settings::parse(&updated).is_clean());
        assert!(updated.contains("\"soft_wrap\": false"));
    }

    #[test]
    fn arrays_round_trip() {
        let original = "{ \"files\": { \"exclude\": [\"**/.git\"] } }";
        let updated = apply_edit(
            original,
            &SettingsEdit::Set {
                path: &["files", "exclude"],
                value: serde_json::json!(["**/.git", "**/target", "**/node_modules"]),
            },
        )
        .unwrap();
        let loaded = Settings::parse(&updated);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(
            loaded.value.files.exclude,
            vec!["**/.git", "**/target", "**/node_modules"]
        );
    }

    #[test]
    fn a_decimal_number_keeps_its_representation() {
        let updated = apply_edit(
            "{}",
            &SettingsEdit::Set {
                path: &["buffer_line_height"],
                value: serde_json::json!(1.6),
            },
        )
        .unwrap();
        assert!(updated.contains("1.6"), "{updated:?}");
        assert!(!updated.contains("1.6000"), "{updated:?}");
        let loaded = Settings::parse(&updated);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
        assert_eq!(loaded.value.buffer_line_height, 1.6);
    }

    #[test]
    fn round_trip_through_settings_parse_never_reports_an_issue() {
        let original = crate::defaults::default_settings_jsonc();
        for edit in [
            soft_wrap_edit(),
            SettingsEdit::Set {
                path: &["ui_font_size"],
                value: serde_json::json!(16),
            },
            SettingsEdit::Remove {
                path: &["editor", "ruler"],
            },
        ] {
            let updated = apply_edit(&original, &edit).unwrap();
            let loaded = Settings::parse(&updated);
            assert!(loaded.is_clean(), "{edit:?} -> {:?}", loaded.issues);
        }
    }

    #[test]
    fn write_edit_reads_and_writes_the_real_file_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(
            &paths.settings,
            "{\n  // comentario\n  \"ui_font_size\": 13\n}\n",
        )
        .unwrap();

        write_edit(&paths, &soft_wrap_edit()).unwrap();

        let text = std::fs::read_to_string(&paths.settings).unwrap();
        assert!(text.contains("// comentario"));
        assert!(text.contains("\"soft_wrap\": false"));
        assert!(!paths.settings.with_file_name("settings.json.tmp").exists());
        let loaded = Settings::parse(&text);
        assert!(loaded.is_clean(), "{:?}", loaded.issues);
    }

    #[test]
    fn write_edit_creates_the_file_when_it_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        assert!(!paths.settings.exists());

        write_edit(&paths, &soft_wrap_edit()).unwrap();

        let text = std::fs::read_to_string(&paths.settings).unwrap();
        assert!(text.contains("Ajustes de Cincel"));
        assert!(text.contains("\"soft_wrap\": false"));
    }
}
