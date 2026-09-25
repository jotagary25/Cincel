//! Autonomy policy.
//!
//! Mirrors `docs/specs/modulos/acp.md` §"Modo de autonomía":
//!
//! * `ReviewAfter` auto-answers `allow_once` for tool calls of kind
//!   `edit | read | search | think`, except on sensitive paths; `execute` is
//!   always forwarded to the UI.
//! * `AskBefore` forwards everything to the UI.
//! * `AlwaysApply` auto-answers `allow_once` for everything except sensitive
//!   paths.
//!
//! `reject_*` options are never auto-selected.
//!
//! Sensitive paths are matched with a [`globset::GlobSet`] (Etapa 2): the
//! built-in patterns cover the credential stores called out in the spec, and
//! [`Autonomy::with_sensitive_globs`] lets settings replace them entirely.

use std::path::Path;
use std::sync::Arc;

use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind, ToolKind};
use globset::{Glob, GlobSet, GlobSetBuilder};

/// How much the agent may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutonomyMode {
    /// Apply edits and review them afterwards (default).
    #[default]
    ReviewAfter,
    /// Ask before every tool call.
    AskBefore,
    /// Apply everything, including shell commands.
    AlwaysApply,
}

impl AutonomyMode {
    /// Human readable label for the CLI.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            AutonomyMode::ReviewAfter => "revisar después",
            AutonomyMode::AskBefore => "pedir antes",
            AutonomyMode::AlwaysApply => "aplicar siempre",
        }
    }
}

/// Outcome of the policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoAnswer {
    /// The client may answer `allow_once` by itself.
    Allow,
    /// The request must reach the user.
    Ask,
}

/// Glob patterns that always require a human decision, mirroring the fixed
/// fragment list from Etapa 0 as glob equivalents.
const DEFAULT_SENSITIVE_GLOBS: &[&str] = &[
    "**/.ssh/**",
    "**/.gnupg/**",
    "**/.aws/**",
    "**/.git/config",
    "**/.netrc",
    "**/.npmrc",
    "**/.pypirc",
    "**/id_rsa*",
    "**/id_ed25519*",
    "**/*credentials*",
    "**/.env",
    "**/.env.*",
];

fn build_globset(patterns: &[String]) -> std::result::Result<GlobSet, globset::Error> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    builder.build()
}

fn default_globset() -> GlobSet {
    let patterns: Vec<String> = DEFAULT_SENSITIVE_GLOBS
        .iter()
        .map(|pattern| (*pattern).to_string())
        .collect();
    build_globset(&patterns).expect("los patrones por defecto son válidos")
}

/// The autonomy policy: a mode plus the glob matcher for sensitive paths.
#[derive(Debug, Clone)]
pub struct Autonomy {
    mode: AutonomyMode,
    sensitive: Arc<GlobSet>,
}

impl Default for Autonomy {
    fn default() -> Self {
        Autonomy::new(AutonomyMode::default())
    }
}

impl Autonomy {
    /// Build a policy in `mode` with the built-in sensitive-path globs.
    #[must_use]
    pub fn new(mode: AutonomyMode) -> Self {
        Self {
            mode,
            sensitive: Arc::new(default_globset()),
        }
    }

    /// Replace the sensitive-path matcher with `globs` (from settings).
    ///
    /// # Errors
    ///
    /// Returns the `globset` parse error for an invalid pattern; `self` is
    /// consumed either way so callers should keep the previous value on error.
    pub fn with_sensitive_globs(
        mut self,
        globs: Vec<String>,
    ) -> std::result::Result<Self, globset::Error> {
        self.sensitive = Arc::new(build_globset(&globs)?);
        Ok(self)
    }

    /// The active mode.
    #[must_use]
    pub fn mode(&self) -> AutonomyMode {
        self.mode
    }

    /// Human readable label for the CLI.
    #[must_use]
    pub fn label(&self) -> &'static str {
        self.mode.label()
    }

    /// Whether `path` matches one of the sensitive globs.
    #[must_use]
    pub fn is_sensitive(&self, path: &Path) -> bool {
        self.sensitive.is_match(path)
    }

    /// Decide whether a permission request can be answered automatically.
    #[must_use]
    pub fn decide(&self, kind: ToolKind, paths: &[impl AsRef<Path>]) -> AutoAnswer {
        if paths.iter().any(|path| self.is_sensitive(path.as_ref())) {
            return AutoAnswer::Ask;
        }
        match self.mode {
            AutonomyMode::AskBefore => AutoAnswer::Ask,
            AutonomyMode::AlwaysApply => AutoAnswer::Allow,
            AutonomyMode::ReviewAfter => match kind {
                ToolKind::Edit | ToolKind::Read | ToolKind::Search | ToolKind::Think => {
                    AutoAnswer::Allow
                }
                _ => AutoAnswer::Ask,
            },
        }
    }
}

/// Pick the option to answer with, preferring `allow_once` over `allow_always`.
///
/// Returns `None` when the agent offered no allow-style option; in that case the
/// request must be shown to the user.
#[must_use]
pub fn pick_allow_option(options: &[PermissionOption]) -> Option<&PermissionOption> {
    options
        .iter()
        .find(|option| option.kind == PermissionOptionKind::AllowOnce)
        .or_else(|| {
            options
                .iter()
                .find(|option| option.kind == PermissionOptionKind::AllowAlways)
        })
}

/// Pick a rejection option, preferring `reject_once`. Never used automatically.
#[must_use]
pub fn pick_reject_option(options: &[PermissionOption]) -> Option<&PermissionOption> {
    options
        .iter()
        .find(|option| option.kind == PermissionOptionKind::RejectOnce)
        .or_else(|| {
            options
                .iter()
                .find(|option| option.kind == PermissionOptionKind::RejectAlways)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn review_after_allows_edits_but_asks_for_execute() {
        let policy = Autonomy::new(AutonomyMode::ReviewAfter);
        let files = paths(&["/proj/src/main.rs"]);
        assert_eq!(policy.decide(ToolKind::Edit, &files), AutoAnswer::Allow);
        assert_eq!(policy.decide(ToolKind::Read, &files), AutoAnswer::Allow);
        assert_eq!(policy.decide(ToolKind::Search, &files), AutoAnswer::Allow);
        assert_eq!(policy.decide(ToolKind::Think, &files), AutoAnswer::Allow);
        assert_eq!(policy.decide(ToolKind::Execute, &files), AutoAnswer::Ask);
        assert_eq!(policy.decide(ToolKind::Other, &files), AutoAnswer::Ask);
    }

    #[test]
    fn ask_before_always_asks() {
        let files = paths(&["/proj/src/main.rs"]);
        assert_eq!(
            Autonomy::new(AutonomyMode::AskBefore).decide(ToolKind::Edit, &files),
            AutoAnswer::Ask
        );
    }

    #[test]
    fn always_apply_allows_execute() {
        let files = paths(&["/proj/src/main.rs"]);
        assert_eq!(
            Autonomy::new(AutonomyMode::AlwaysApply).decide(ToolKind::Execute, &files),
            AutoAnswer::Allow
        );
    }

    #[test]
    fn sensitive_paths_always_ask() {
        let policy = Autonomy::new(AutonomyMode::AlwaysApply);
        let files = paths(&["/home/u/.ssh/id_rsa"]);
        assert_eq!(policy.decide(ToolKind::Read, &files), AutoAnswer::Ask);
        assert!(policy.is_sensitive(Path::new("/proj/.env")));
        assert!(!policy.is_sensitive(Path::new("/proj/src/env.rs")));
    }

    #[test]
    fn never_auto_selects_reject() {
        let options = vec![
            PermissionOption::new("r", "Rechazar", PermissionOptionKind::RejectOnce),
            PermissionOption::new("a", "Permitir", PermissionOptionKind::AllowOnce),
        ];
        let picked = pick_allow_option(&options).expect("allow");
        assert_eq!(picked.kind, PermissionOptionKind::AllowOnce);
    }

    #[test]
    fn custom_sensitive_globs_replace_the_defaults() {
        let policy = Autonomy::new(AutonomyMode::AlwaysApply)
            .with_sensitive_globs(vec!["**/*.secret".to_string()])
            .expect("globs válidos");
        // The custom list no longer flags `.ssh`, but does flag `*.secret`.
        assert!(!policy.is_sensitive(Path::new("/home/u/.ssh/id_rsa")));
        assert!(policy.is_sensitive(Path::new("/proj/api.secret")));
    }

    #[test]
    fn invalid_glob_pattern_is_rejected() {
        let error = Autonomy::default().with_sensitive_globs(vec!["[".to_string()]);
        assert!(error.is_err());
    }

    #[test]
    fn default_autonomy_mode_is_review_after() {
        assert_eq!(Autonomy::default().mode(), AutonomyMode::ReviewAfter);
    }
}
