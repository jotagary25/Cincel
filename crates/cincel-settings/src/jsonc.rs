//! JSONC parsing and the tolerant reader every settings type is built with.
//!
//! # Why `jsonc-parser`
//!
//! The spec (`docs/specs/03-arquitectura.md` §1) allows `json5` or
//! `jsonc-parser`. We use [`jsonc_parser`] because:
//!
//! - it parses *exactly* the dialect the spec documents — JSON plus comments
//!   plus trailing commas — instead of JSON5, which also changes number
//!   literals, string quoting and identifier keys, so a `settings.json`
//!   written for VS Code or Zed behaves identically here;
//! - it reports errors with a line and column, which is what a
//!   [`SettingsIssue`] needs to be actionable;
//! - it is the parser `dprint`/`deno` maintain, released regularly, with no
//!   transitive dependencies beyond `serde_json` under the feature we use.
//!
//! The permissive extras it offers (single quotes, unquoted keys, missing
//! commas, hex numbers) are turned **off** so that a file accepted here is
//! accepted by every other JSONC reader too.
//!
//! # The tolerant reader
//!
//! `serde` alone is all-or-nothing: one bad value and the whole document is
//! rejected. The spec asks for the opposite (report the key, default the
//! value, keep going), so every type is read key by key through [`Reader`],
//! which deserializes one value at a time and records a [`SettingsIssue`]
//! whenever that fails.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::issue::SettingsIssue;

/// Parses JSONC text into a [`Value`].
///
/// Empty (or whitespace-only) input is `Value::Null`, exactly like a missing
/// file, so callers treat both the same way.
pub(crate) fn parse(text: &str) -> Result<Value, String> {
    let options = jsonc_parser::ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    };
    jsonc_parser::parse_to_serde_value::<Value>(text, &options).map_err(|error| error.to_string())
}

/// Reads a document object key by key, collecting issues instead of failing.
pub(crate) struct Reader<'a> {
    issues: &'a mut Vec<SettingsIssue>,
    /// Dotted path of the object being read, `""` at the document root.
    prefix: String,
}

impl<'a> Reader<'a> {
    /// A reader for the root of a document.
    pub(crate) fn new(issues: &'a mut Vec<SettingsIssue>) -> Self {
        Self {
            issues,
            prefix: String::new(),
        }
    }

    /// Records an issue at `key` of the object being read.
    pub(crate) fn issue(&mut self, key: &str, message: impl Into<String>) {
        let path = self.path(key);
        self.issues.push(SettingsIssue::new(path, message));
    }

    /// The dotted path of `key` inside the object being read.
    fn path(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_owned()
        } else if key.is_empty() {
            self.prefix.clone()
        } else {
            format!("{}.{key}", self.prefix)
        }
    }

    /// Reads `key` as `T`, falling back to `default` and recording an issue
    /// when it is missing-but-wrong-shaped or fails to deserialize.
    pub(crate) fn field<T: DeserializeOwned>(
        &mut self,
        object: &Map<String, Value>,
        key: &str,
        default: T,
    ) -> T {
        let Some(raw) = object.get(key) else {
            return default;
        };
        match serde_json::from_value::<T>(raw.clone()) {
            Ok(value) => value,
            Err(error) => {
                self.issue(
                    key,
                    format!("valor inválido ({error}); se usa el predeterminado"),
                );
                default
            }
        }
    }

    /// Reads `key` as `T` and then checks it with `validate`, which returns
    /// the reason the value is unacceptable.
    pub(crate) fn checked_field<T: DeserializeOwned + Clone>(
        &mut self,
        object: &Map<String, Value>,
        key: &str,
        default: T,
        validate: impl FnOnce(&T) -> Result<(), String>,
    ) -> T {
        let value = self.field(object, key, default.clone());
        match validate(&value) {
            Ok(()) => value,
            Err(reason) => {
                self.issue(key, format!("{reason}; se usa el predeterminado"));
                default
            }
        }
    }

    /// Reads the nested object at `key` with `read`, or returns
    /// `T::default()` when it is absent or not an object.
    pub(crate) fn object<T: Default>(
        &mut self,
        object: &Map<String, Value>,
        key: &str,
        read: impl FnOnce(&mut Reader<'_>, &Map<String, Value>) -> T,
    ) -> T {
        let Some(raw) = object.get(key) else {
            return T::default();
        };
        match raw {
            Value::Object(map) => {
                let prefix = self.path(key);
                let mut nested = Reader {
                    issues: self.issues,
                    prefix,
                };
                read(&mut nested, map)
            }
            other => {
                self.issue(
                    key,
                    format!(
                        "debe ser un objeto, no {}; se usa el predeterminado",
                        type_name(other)
                    ),
                );
                T::default()
            }
        }
    }

    /// Reads the array at `key`, mapping each element with `read`. Elements
    /// that `read` rejects are dropped with an issue naming their index.
    pub(crate) fn array<T>(
        &mut self,
        object: &Map<String, Value>,
        key: &str,
        default: Vec<T>,
        mut read: impl FnMut(&mut Reader<'_>, &Value) -> Option<T>,
    ) -> Vec<T> {
        let Some(raw) = object.get(key) else {
            return default;
        };
        let Value::Array(items) = raw else {
            self.issue(
                key,
                format!(
                    "debe ser una lista, no {}; se usa el predeterminado",
                    type_name(raw)
                ),
            );
            return default;
        };
        let mut out = Vec::with_capacity(items.len());
        let base = self.path(key);
        for (index, item) in items.iter().enumerate() {
            let mut element = Reader {
                issues: self.issues,
                prefix: format!("{base}[{index}]"),
            };
            if let Some(value) = read(&mut element, item) {
                out.push(value);
            }
        }
        out
    }

    /// Records an issue for every key of `object` that is not in `known`.
    ///
    /// A typo in a settings file is silent otherwise, which is the most
    /// common way a user "loses" a setting.
    pub(crate) fn unknown_keys(&mut self, object: &Map<String, Value>, known: &[&str]) {
        for key in object.keys() {
            if !known.contains(&key.as_str()) {
                self.issue(key, "clave desconocida; se ignora");
            }
        }
    }

    /// Runs `read` over the object at the root of `value`.
    ///
    /// A `null` document (a missing or empty file) yields the default with no
    /// issue; any other non-object document yields the default with one.
    pub(crate) fn document<T: Default>(
        value: &Value,
        issues: &mut Vec<SettingsIssue>,
        read: impl FnOnce(&mut Reader<'_>, &Map<String, Value>) -> T,
    ) -> T {
        match value {
            Value::Null => T::default(),
            Value::Object(map) => {
                let mut reader = Reader::new(issues);
                read(&mut reader, map)
            }
            other => {
                issues.push(SettingsIssue::document(format!(
                    "el archivo debe contener un objeto JSON, no {}",
                    type_name(other)
                )));
                T::default()
            }
        }
    }
}

/// The JSON type of `value`, in Spanish, for issue messages.
pub(crate) fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "nulo",
        Value::Bool(_) => "un booleano",
        Value::Number(_) => "un número",
        Value::String(_) => "un texto",
        Value::Array(_) => "una lista",
        Value::Object(_) => "un objeto",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_and_trailing_commas() {
        let value = parse(
            r#"{
              // a line comment
              "a": 1, /* a block comment */
              "b": [1, 2,],
            }"#,
        )
        .expect("debe parsear");
        assert_eq!(value["a"], 1);
        assert_eq!(value["b"], serde_json::json!([1, 2]));
    }

    #[test]
    fn empty_input_is_null() {
        assert_eq!(parse("").unwrap(), Value::Null);
        assert_eq!(parse("  \n // solo un comentario\n").unwrap(), Value::Null);
    }

    #[test]
    fn syntax_errors_carry_a_position() {
        let error = parse("{ \"a\": }").unwrap_err();
        assert!(error.contains("line") || error.contains("Line"), "{error}");
    }

    #[test]
    fn json5_only_syntax_is_rejected() {
        assert!(parse("{ a: 1 }").is_err());
        assert!(parse("{ 'a': 1 }").is_err());
    }

    #[test]
    fn field_defaults_and_reports() {
        let value = parse(r#"{ "n": "no soy un número", "m": 7 }"#).unwrap();
        let object = value.as_object().unwrap();
        let mut issues = Vec::new();
        let mut reader = Reader::new(&mut issues);
        assert_eq!(reader.field(object, "n", 3u32), 3);
        assert_eq!(reader.field(object, "m", 3u32), 7);
        assert_eq!(reader.field(object, "ausente", 3u32), 3);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, "n");
    }

    #[test]
    fn nested_paths_are_dotted() {
        let value = parse(r#"{ "editor": { "tab_size": "cuatro" } }"#).unwrap();
        let object = value.as_object().unwrap();
        let mut issues = Vec::new();
        let mut reader = Reader::new(&mut issues);
        let size = reader.object(object, "editor", |reader, editor| {
            reader.field(editor, "tab_size", 4u32)
        });
        assert_eq!(size, 4);
        assert_eq!(issues[0].path, "editor.tab_size");
    }
}
