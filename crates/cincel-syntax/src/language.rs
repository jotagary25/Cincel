//! The language registry: grammars, queries and file detection.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use tree_sitter::{Language as TsLanguage, Query};

use crate::highlight::HighlightId;

/// Static description of a language. One per cargo feature.
struct LanguageConfig {
    /// Canonical name, also the key of `LanguageRegistry::language`.
    name: &'static str,
    /// Extra names accepted by `language`, used by Markdown injections
    /// (```` ```rs ````, ```` ```sh ````, …).
    aliases: &'static [&'static str],
    /// File extensions, without the dot, lowercase.
    extensions: &'static [&'static str],
    /// Whole file names, matched case-sensitively first, then insensitively.
    filenames: &'static [&'static str],
    /// File name prefixes (`Dockerfile.dev`).
    filename_prefixes: &'static [&'static str],
    /// Interpreter names accepted in a `#!` line.
    shebangs: &'static [&'static str],
    /// The grammar.
    grammar: fn() -> TsLanguage,
    /// Highlight query source. Some languages concatenate several files.
    highlights: fn() -> String,
    /// Injection query source, empty when the language has none.
    injections: fn() -> String,
}

/// A language: a tree-sitter grammar plus its compiled queries.
///
/// Queries are compiled on first use and cached, so building a
/// [`LanguageRegistry`] costs nothing.
pub struct Language {
    config: &'static LanguageConfig,
    compiled: OnceLock<Result<Grammar, String>>,
}

/// The compiled side of a [`Language`].
pub(crate) struct Grammar {
    pub(crate) ts_language: TsLanguage,
    pub(crate) highlights: Query,
    /// Capture index -> highlight, `None` for captures the theme ignores.
    pub(crate) capture_map: Vec<Option<HighlightId>>,
    pub(crate) injections: Option<Query>,
    pub(crate) injection_content: Option<u32>,
    pub(crate) injection_language: Option<u32>,
    /// Whether the injections query can ever name a language other than this
    /// one. Rust only injects Rust into its own macros, so running the query
    /// on every re-parse would be pure cost.
    pub(crate) injects_other_languages: bool,
}

impl std::fmt::Debug for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Language")
            .field("name", &self.name())
            .finish()
    }
}

impl Language {
    /// Canonical name (`rust`, `typescript`, `tsx`, …).
    pub fn name(&self) -> &'static str {
        self.config.name
    }

    /// Extensions this language claims.
    pub fn extensions(&self) -> &'static [&'static str] {
        self.config.extensions
    }

    /// Whole file names this language claims.
    pub fn filenames(&self) -> &'static [&'static str] {
        self.config.filenames
    }

    /// The tree-sitter grammar, compiling the queries on first use.
    pub fn ts_language(&self) -> Result<TsLanguage, &str> {
        self.grammar().map(|grammar| grammar.ts_language.clone())
    }

    /// Whether the grammar and its queries compile. Useful in tests.
    pub fn is_usable(&self) -> bool {
        self.grammar().is_ok()
    }

    pub(crate) fn grammar(&self) -> Result<&Grammar, &str> {
        self.compiled
            .get_or_init(|| Self::compile(self.config))
            .as_ref()
            .map_err(String::as_str)
    }

    fn compile(config: &'static LanguageConfig) -> Result<Grammar, String> {
        let ts_language = (config.grammar)();
        let highlights = Query::new(&ts_language, &(config.highlights)())
            .map_err(|error| format!("{}: highlights query: {error}", config.name))?;
        let capture_map = highlights
            .capture_names()
            .iter()
            .map(|name| HighlightId::from_capture_name(name))
            .collect();

        let injection_source = (config.injections)();
        let injections = if injection_source.trim().is_empty() {
            None
        } else {
            Some(
                Query::new(&ts_language, &injection_source)
                    .map_err(|error| format!("{}: injections query: {error}", config.name))?,
            )
        };
        let injection_content = injections
            .as_ref()
            .and_then(|query| query.capture_index_for_name("injection.content"));
        let injection_language = injections
            .as_ref()
            .and_then(|query| query.capture_index_for_name("injection.language"));
        let injects_other_languages = injections.as_ref().is_some_and(|query| {
            injection_language.is_some()
                || (0..query.pattern_count()).any(|pattern| {
                    query.property_settings(pattern).iter().any(|property| {
                        &*property.key == "injection.language"
                            && property
                                .value
                                .as_deref()
                                .is_some_and(|value| value != config.name)
                    })
                })
        });

        Ok(Grammar {
            ts_language,
            highlights,
            capture_map,
            injections,
            injection_content,
            injection_language,
            injects_other_languages,
        })
    }

    fn matches_filename(&self, file_name: &str) -> bool {
        let lower = file_name.to_ascii_lowercase();
        self.config
            .filenames
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(file_name))
            || self
                .config
                .filename_prefixes
                .iter()
                .any(|prefix| lower.starts_with(&prefix.to_ascii_lowercase()))
    }

    fn matches_extension(&self, extension: &str) -> bool {
        let lower = extension.to_ascii_lowercase();
        self.config.extensions.contains(&lower.as_str())
    }

    fn matches_name(&self, name: &str) -> bool {
        self.config.name.eq_ignore_ascii_case(name)
            || self
                .config
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
    }

    fn matches_shebang(&self, interpreter: &str) -> bool {
        self.config.shebangs.contains(&interpreter)
    }
}

/// Every language compiled into this build.
///
/// Detection order, as `docs/specs/modulos/syntax.md` asks: whole file name
/// first (`Dockerfile`), then extension, then the `#!` line. A file with no
/// grammar is plain text, which is why every lookup returns an `Option`.
/// `Makefile` is recognized as a *file name* rule but has no v1 grammar, so it
/// resolves to `None`.
pub struct LanguageRegistry {
    languages: Vec<Arc<Language>>,
}

impl Default for LanguageRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageRegistry {
    /// Builds the registry with every language enabled by cargo features.
    pub fn new() -> Self {
        let languages = CONFIGS
            .iter()
            .map(|config| {
                Arc::new(Language {
                    config,
                    compiled: OnceLock::new(),
                })
            })
            .collect();
        Self { languages }
    }

    /// Every language, in table order.
    pub fn languages(&self) -> &[Arc<Language>] {
        &self.languages
    }

    /// Looks a language up by canonical name or alias (`rs`, `sh`, `c++`).
    pub fn language(&self, name: &str) -> Option<Arc<Language>> {
        self.languages
            .iter()
            .find(|language| language.matches_name(name))
            .cloned()
    }

    /// Detects from the path alone (file name, then extension).
    pub fn language_for_path(&self, path: impl AsRef<Path>) -> Option<Arc<Language>> {
        let path = path.as_ref();
        if let Some(file_name) = path.file_name().and_then(|name| name.to_str())
            && let Some(language) = self
                .languages
                .iter()
                .find(|language| language.matches_filename(file_name))
        {
            return Some(language.clone());
        }
        let extension = path.extension().and_then(|ext| ext.to_str())?;
        self.languages
            .iter()
            .find(|language| language.matches_extension(extension))
            .cloned()
    }

    /// Detects from a `#!` first line (`#!/usr/bin/env python3`).
    pub fn language_for_shebang(&self, first_line: &str) -> Option<Arc<Language>> {
        let rest = first_line.strip_prefix("#!")?.trim();
        // `#!/usr/bin/env -S python3 -u` and `#!/bin/bash -e` both end up with
        // the interpreter as the first token that is not a flag or `env`.
        let interpreter = rest
            .split_whitespace()
            .map(|token| token.rsplit('/').next().unwrap_or(token))
            .find(|token| !token.starts_with('-') && *token != "env")?;
        // Strip a trailing version suffix: python3.12 -> python3 -> python.
        let mut candidate = interpreter;
        loop {
            if let Some(language) = self
                .languages
                .iter()
                .find(|language| language.matches_shebang(candidate))
            {
                return Some(language.clone());
            }
            match candidate.rfind(|c: char| c.is_ascii_digit() || c == '.') {
                Some(ix) if ix + 1 == candidate.len() => candidate = &candidate[..ix],
                _ => return None,
            }
        }
    }

    /// The full detection: file name, extension, then the first line.
    pub fn language_for(
        &self,
        path: impl AsRef<Path>,
        first_line: Option<&str>,
    ) -> Option<Arc<Language>> {
        self.language_for_path(path)
            .or_else(|| first_line.and_then(|line| self.language_for_shebang(line)))
    }
}

/// Concatenates query sources, skipping empty ones.
fn join(parts: &[&str]) -> String {
    parts.join("\n")
}

/// The language table. Each entry is behind its cargo feature.
static CONFIGS: &[LanguageConfig] = &[
    #[cfg(feature = "rust")]
    LanguageConfig {
        name: "rust",
        aliases: &["rs"],
        extensions: &["rs"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_rust::LANGUAGE.into(),
        highlights: || tree_sitter_rust::HIGHLIGHTS_QUERY.to_owned(),
        injections: || tree_sitter_rust::INJECTIONS_QUERY.to_owned(),
    },
    #[cfg(feature = "typescript")]
    LanguageConfig {
        name: "typescript",
        aliases: &["ts"],
        extensions: &["ts", "mts", "cts"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &["ts-node", "tsx", "deno"],
        grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        // The TypeScript queries only cover the TS-specific nodes; the shared
        // JavaScript ones come first so that later (more specific) patterns
        // win, which is tree-sitter's precedence rule.
        highlights: || {
            join(&[
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
            ])
        },
        injections: || tree_sitter_javascript::INJECTIONS_QUERY.to_owned(),
    },
    #[cfg(feature = "tsx")]
    LanguageConfig {
        name: "tsx",
        aliases: &[],
        extensions: &["tsx"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        highlights: || {
            join(&[
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
            ])
        },
        injections: || tree_sitter_javascript::INJECTIONS_QUERY.to_owned(),
    },
    #[cfg(feature = "javascript")]
    LanguageConfig {
        name: "javascript",
        aliases: &["js", "jsx", "node"],
        extensions: &["js", "mjs", "cjs", "jsx"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &["node", "bun"],
        grammar: || tree_sitter_javascript::LANGUAGE.into(),
        highlights: || {
            join(&[
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            ])
        },
        injections: || tree_sitter_javascript::INJECTIONS_QUERY.to_owned(),
    },
    #[cfg(feature = "json")]
    LanguageConfig {
        name: "json",
        aliases: &["jsonc"],
        extensions: &["json", "jsonc"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_json::LANGUAGE.into(),
        highlights: || tree_sitter_json::HIGHLIGHTS_QUERY.to_owned(),
        injections: String::new,
    },
    #[cfg(feature = "toml")]
    LanguageConfig {
        name: "toml",
        aliases: &[],
        extensions: &["toml"],
        filenames: &["Cargo.lock"],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_toml_ng::LANGUAGE.into(),
        highlights: || tree_sitter_toml_ng::HIGHLIGHTS_QUERY.to_owned(),
        injections: String::new,
    },
    #[cfg(feature = "yaml")]
    LanguageConfig {
        name: "yaml",
        aliases: &["yml"],
        extensions: &["yaml", "yml"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_yaml::LANGUAGE.into(),
        highlights: || tree_sitter_yaml::HIGHLIGHTS_QUERY.to_owned(),
        injections: String::new,
    },
    #[cfg(feature = "markdown")]
    LanguageConfig {
        name: "markdown",
        aliases: &["md"],
        extensions: &["md", "markdown", "mdx"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || arborium_markdown::language().into(),
        highlights: || arborium_markdown::HIGHLIGHTS_QUERY.to_owned(),
        injections: || arborium_markdown::INJECTIONS_QUERY.to_owned(),
    },
    #[cfg(feature = "html")]
    LanguageConfig {
        name: "html",
        aliases: &["xhtml"],
        extensions: &["html", "htm", "xhtml"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_html::LANGUAGE.into(),
        highlights: || tree_sitter_html::HIGHLIGHTS_QUERY.to_owned(),
        injections: || tree_sitter_html::INJECTIONS_QUERY.to_owned(),
    },
    #[cfg(feature = "css")]
    LanguageConfig {
        name: "css",
        aliases: &[],
        extensions: &["css"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_css::LANGUAGE.into(),
        highlights: || tree_sitter_css::HIGHLIGHTS_QUERY.to_owned(),
        injections: String::new,
    },
    #[cfg(feature = "python")]
    LanguageConfig {
        name: "python",
        aliases: &["py"],
        extensions: &["py", "pyi", "pyw"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &["python", "python2", "python3", "uv", "pypy"],
        grammar: || tree_sitter_python::LANGUAGE.into(),
        highlights: || tree_sitter_python::HIGHLIGHTS_QUERY.to_owned(),
        injections: String::new,
    },
    #[cfg(feature = "go")]
    LanguageConfig {
        name: "go",
        aliases: &["golang"],
        extensions: &["go"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_go::LANGUAGE.into(),
        highlights: || tree_sitter_go::HIGHLIGHTS_QUERY.to_owned(),
        injections: String::new,
    },
    #[cfg(feature = "bash")]
    LanguageConfig {
        name: "bash",
        aliases: &["sh", "shell", "zsh", "console"],
        extensions: &["sh", "bash", "zsh", "bashrc", "zshrc", "profile"],
        filenames: &[".bashrc", ".bash_profile", ".zshrc", ".profile"],
        filename_prefixes: &[],
        shebangs: &["sh", "bash", "zsh", "dash", "ksh", "ash"],
        grammar: || tree_sitter_bash::LANGUAGE.into(),
        highlights: || tree_sitter_bash::HIGHLIGHT_QUERY.to_owned(),
        injections: String::new,
    },
    #[cfg(feature = "c")]
    LanguageConfig {
        name: "c",
        aliases: &[],
        extensions: &["c", "h"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_c::LANGUAGE.into(),
        highlights: || tree_sitter_c::HIGHLIGHT_QUERY.to_owned(),
        injections: String::new,
    },
    #[cfg(feature = "cpp")]
    LanguageConfig {
        name: "cpp",
        aliases: &["c++", "cxx"],
        extensions: &["cc", "cpp", "cxx", "hpp", "hh", "hxx", "c++", "ipp"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_cpp::LANGUAGE.into(),
        // The C++ grammar is a superset of C: both query files apply.
        highlights: || {
            join(&[
                #[cfg(feature = "c")]
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_cpp::HIGHLIGHT_QUERY,
            ])
        },
        injections: String::new,
    },
    #[cfg(feature = "java")]
    LanguageConfig {
        name: "java",
        aliases: &[],
        extensions: &["java"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_java::LANGUAGE.into(),
        highlights: || tree_sitter_java::HIGHLIGHTS_QUERY.to_owned(),
        injections: String::new,
    },
    #[cfg(feature = "sql")]
    LanguageConfig {
        name: "sql",
        aliases: &["sequel", "postgres", "mysql"],
        extensions: &["sql"],
        filenames: &[],
        filename_prefixes: &[],
        shebangs: &[],
        grammar: || tree_sitter_sequel::LANGUAGE.into(),
        highlights: || tree_sitter_sequel::HIGHLIGHTS_QUERY.to_owned(),
        injections: String::new,
    },
    #[cfg(feature = "dockerfile")]
    LanguageConfig {
        name: "dockerfile",
        aliases: &["docker", "containerfile"],
        extensions: &["dockerfile", "containerfile"],
        filenames: &["Dockerfile", "Containerfile"],
        filename_prefixes: &["dockerfile.", "containerfile."],
        shebangs: &[],
        grammar: || arborium_dockerfile::language().into(),
        highlights: || arborium_dockerfile::HIGHLIGHTS_QUERY.to_owned(),
        injections: String::new,
    },
];
