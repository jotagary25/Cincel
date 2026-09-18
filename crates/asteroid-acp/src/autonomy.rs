//! Autonomy policy (stub for E0).
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

use std::path::Path;

use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind, ToolKind};

/// How much the agent may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Autonomy {
    /// Apply edits and review them afterwards (default).
    #[default]
    ReviewAfter,
    /// Ask before every tool call.
    AskBefore,
    /// Apply everything, including shell commands.
    AlwaysApply,
}

/// Outcome of the policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoAnswer {
    /// The client may answer `allow_once` by itself.
    Allow,
    /// The request must reach the user.
    Ask,
}

/// Path fragments that always require a human decision.
const SENSITIVE_FRAGMENTS: &[&str] = &[
    ".ssh/",
    ".gnupg/",
    ".aws/",
    ".git/config",
    ".netrc",
    ".npmrc",
    ".pypirc",
    "id_rsa",
    "id_ed25519",
    "credentials",
];

/// File names that always require a human decision.
const SENSITIVE_NAMES: &[&str] = &[".env", ".env.local", ".netrc", ".npmrc", ".pypirc"];

impl Autonomy {
    /// Decide whether a permission request can be answered automatically.
    #[must_use]
    pub fn decide(self, kind: ToolKind, paths: &[impl AsRef<Path>]) -> AutoAnswer {
        if paths.iter().any(|path| is_sensitive(path.as_ref())) {
            return AutoAnswer::Ask;
        }
        match self {
            Autonomy::AskBefore => AutoAnswer::Ask,
            Autonomy::AlwaysApply => AutoAnswer::Allow,
            Autonomy::ReviewAfter => match kind {
                ToolKind::Edit | ToolKind::Read | ToolKind::Search | ToolKind::Think => {
                    AutoAnswer::Allow
                }
                _ => AutoAnswer::Ask,
            },
        }
    }

    /// Human readable label for the CLI.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Autonomy::ReviewAfter => "revisar después",
            Autonomy::AskBefore => "pedir antes",
            Autonomy::AlwaysApply => "aplicar siempre",
        }
    }
}

/// Whether a path looks like a credential store.
#[must_use]
pub fn is_sensitive(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    if SENSITIVE_FRAGMENTS
        .iter()
        .any(|fragment| text.contains(fragment))
    {
        return true;
    }
    path.file_name()
        .map(|name| {
            let name = name.to_string_lossy();
            SENSITIVE_NAMES.iter().any(|sensitive| name == *sensitive)
        })
        .unwrap_or(false)
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
        let policy = Autonomy::ReviewAfter;
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
            Autonomy::AskBefore.decide(ToolKind::Edit, &files),
            AutoAnswer::Ask
        );
    }

    #[test]
    fn always_apply_allows_execute() {
        let files = paths(&["/proj/src/main.rs"]);
        assert_eq!(
            Autonomy::AlwaysApply.decide(ToolKind::Execute, &files),
            AutoAnswer::Allow
        );
    }

    #[test]
    fn sensitive_paths_always_ask() {
        let files = paths(&["/home/u/.ssh/id_rsa"]);
        assert_eq!(
            Autonomy::AlwaysApply.decide(ToolKind::Read, &files),
            AutoAnswer::Ask
        );
        assert!(is_sensitive(Path::new("/proj/.env")));
        assert!(!is_sensitive(Path::new("/proj/src/env.rs")));
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
}
