//! Problems found while reading a configuration file.

use std::fmt;

/// One thing that was wrong in a configuration file.
///
/// `path` locates the offending value: a dotted JSON path relative to the
/// document (`editor.tab_size`, `agents.custom[1].command`), `"$"` when the
/// whole document is at fault (a syntax error, a top level that is not an
/// object), or `"<archivo>: <ruta>"` when several files feed the same load
/// (themes). `message` is user facing and therefore Spanish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsIssue {
    /// Dotted path of the value that was ignored.
    pub path: String,
    /// What was wrong with it, in Spanish, ready to show in a toast.
    pub message: String,
}

impl SettingsIssue {
    /// An issue about the value at `path`.
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }

    /// An issue about the document as a whole (syntax error, wrong shape).
    pub fn document(message: impl Into<String>) -> Self {
        Self::new("$", message)
    }

    /// The same issue with `prefix` prepended to its path, used when a load
    /// merges several files.
    pub fn in_file(mut self, file: &str) -> Self {
        self.path = format!("{file}: {}", self.path);
        self
    }
}

impl fmt::Display for SettingsIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// A value that was loaded together with everything that went wrong while
/// loading it.
///
/// Loading never fails: `value` is always usable, with defaults substituted
/// for whatever `issues` describes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Loaded<T> {
    /// The loaded value, with defaults for every rejected key.
    pub value: T,
    /// Everything that was ignored, in document order.
    pub issues: Vec<SettingsIssue>,
}

impl<T> Loaded<T> {
    /// A clean load with no issues.
    pub fn clean(value: T) -> Self {
        Self {
            value,
            issues: Vec::new(),
        }
    }

    /// Whether anything was rejected.
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    /// Logs every issue through `tracing` and returns the value.
    pub fn log(self) -> T {
        for issue in &self.issues {
            tracing::warn!(path = %issue.path, message = %issue.message, "ajuste ignorado");
        }
        self.value
    }

    /// Maps the value, keeping the issues.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Loaded<U> {
        Loaded {
            value: f(self.value),
            issues: self.issues,
        }
    }
}
