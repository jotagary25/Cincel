//! Permission policy.
//!
//! Mirrors `docs/specs/modulos/acp.md` §"Política de permisos". There is a
//! single, fixed behaviour (the old per-session autonomy modes are gone; the
//! agent's own modes, exposed as ACP config options, are the agent's
//! business):
//!
//! * permission requests whose tool call is of kind
//!   `edit | read | search | think` are answered `allow_once` automatically;
//! * `execute | fetch | delete | move | other` (and any future kind) reach the
//!   user;
//! * any request touching a path that matches the sensitive globs
//!   (`review.sensitive_paths`) always reaches the user, whatever its kind.
//!
//! `reject_*` options are never auto-selected.
//!
//! Sensitive paths are matched with a [`globset::GlobSet`]: the built-in
//! patterns cover the credential stores called out in the spec, and
//! [`PermissionPolicy::with_sensitive_globs`] lets settings replace them
//! entirely.

use std::path::Path;
use std::sync::Arc;

use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind, ToolKind};
use globset::{Glob, GlobSet, GlobSetBuilder};

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

/// The fixed permission policy plus the glob matcher for sensitive paths.
#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    sensitive: Arc<GlobSet>,
}

impl Default for PermissionPolicy {
    /// The policy with the built-in sensitive-path globs.
    fn default() -> Self {
        Self {
            sensitive: Arc::new(default_globset()),
        }
    }
}

impl PermissionPolicy {
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

    /// Whether `path` matches one of the sensitive globs.
    #[must_use]
    pub fn is_sensitive(&self, path: &Path) -> bool {
        self.sensitive.is_match(path)
    }

    /// Whether a tool call of `kind` is answered automatically when it touches
    /// no sensitive path.
    #[must_use]
    pub fn auto_allows_kind(kind: ToolKind) -> bool {
        matches!(
            kind,
            ToolKind::Edit | ToolKind::Read | ToolKind::Search | ToolKind::Think
        )
    }

    /// Decide whether a permission request can be answered automatically.
    #[must_use]
    pub fn decide(&self, kind: ToolKind, paths: &[impl AsRef<Path>]) -> AutoAnswer {
        if paths.iter().any(|path| self.is_sensitive(path.as_ref())) {
            return AutoAnswer::Ask;
        }
        if Self::auto_allows_kind(kind) {
            AutoAnswer::Allow
        } else {
            AutoAnswer::Ask
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
    fn edits_reads_searches_and_thinking_are_allowed() {
        let policy = PermissionPolicy::default();
        let files = paths(&["/proj/src/main.rs"]);
        for kind in [
            ToolKind::Edit,
            ToolKind::Read,
            ToolKind::Search,
            ToolKind::Think,
        ] {
            assert_eq!(policy.decide(kind, &files), AutoAnswer::Allow, "{kind:?}");
        }
    }

    #[test]
    fn execute_fetch_delete_move_and_other_reach_the_user() {
        let policy = PermissionPolicy::default();
        let files = paths(&["/proj/src/main.rs"]);
        for kind in [
            ToolKind::Execute,
            ToolKind::Fetch,
            ToolKind::Delete,
            ToolKind::Move,
            ToolKind::Other,
        ] {
            assert_eq!(policy.decide(kind, &files), AutoAnswer::Ask, "{kind:?}");
        }
        let none: [&str; 0] = [];
        assert_eq!(policy.decide(ToolKind::Execute, &none), AutoAnswer::Ask);
    }

    #[test]
    fn sensitive_paths_always_ask() {
        let policy = PermissionPolicy::default();
        let files = paths(&["/home/u/.ssh/id_rsa"]);
        for kind in [ToolKind::Read, ToolKind::Edit, ToolKind::Think] {
            assert_eq!(policy.decide(kind, &files), AutoAnswer::Ask, "{kind:?}");
        }
        let mixed = paths(&["/proj/src/main.rs", "/proj/.env"]);
        assert_eq!(policy.decide(ToolKind::Edit, &mixed), AutoAnswer::Ask);
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
        let policy = PermissionPolicy::default()
            .with_sensitive_globs(vec!["**/*.secret".to_string()])
            .expect("globs válidos");
        // The custom list no longer flags `.ssh`, but does flag `*.secret`.
        assert!(!policy.is_sensitive(Path::new("/home/u/.ssh/id_rsa")));
        assert!(policy.is_sensitive(Path::new("/proj/api.secret")));
        assert_eq!(
            policy.decide(ToolKind::Edit, &paths(&["/proj/api.secret"])),
            AutoAnswer::Ask
        );
    }

    #[test]
    fn invalid_glob_pattern_is_rejected() {
        let error = PermissionPolicy::default().with_sensitive_globs(vec!["[".to_string()]);
        assert!(error.is_err());
    }
}
