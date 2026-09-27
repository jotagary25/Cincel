//! [`ProcessEnv`]: environment adjustments applied to every agent process a
//! connection spawns (`docs/specs/06-etapa4-conexiones-y-cincel.md` §2, §5.7).
//!
//! `cincel-connections` uses it to inject the profile variable
//! (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_HOME`), strip provider
//! credential variables inherited from the user's shell, and put the private
//! Node runtime first in `PATH`.

use std::ffi::OsString;
use std::path::PathBuf;

/// Environment adjustments for a spawned agent process.
///
/// Applied in this order on top of the inherited environment:
/// 1. every name in [`ProcessEnv::unset`] is removed (a trailing `*` makes it
///    a prefix match, e.g. `CLAUDE_CODE_USE_*`);
/// 2. the [`crate::LaunchSpec`]'s own `env` is layered on;
/// 3. every pair in [`ProcessEnv::set`] is layered on (wins over the launch
///    spec);
/// 4. [`ProcessEnv::path_prepend`] entries are put in front of `PATH` (the
///    `PATH` from step 3 if one was set, otherwise the inherited one).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessEnv {
    /// Variables to set.
    pub set: Vec<(String, String)>,
    /// Variables to remove. A trailing `*` matches every variable with that
    /// prefix.
    pub unset: Vec<String>,
    /// Directories to put first in `PATH`, in order.
    pub path_prepend: Vec<PathBuf>,
}

impl ProcessEnv {
    /// An empty set of adjustments (inherit everything as is).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a variable to set.
    #[must_use]
    pub fn with_set(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.set.push((name.into(), value.into()));
        self
    }

    /// Add a variable (or `PREFIX*` pattern) to remove.
    #[must_use]
    pub fn with_unset(mut self, name: impl Into<String>) -> Self {
        self.unset.push(name.into());
        self
    }

    /// Add a directory to put first in `PATH`.
    #[must_use]
    pub fn with_path_prepend(mut self, dir: impl Into<PathBuf>) -> Self {
        self.path_prepend.push(dir.into());
        self
    }

    /// Whether `name` is matched by one of the [`ProcessEnv::unset`] entries.
    #[must_use]
    pub fn unsets(&self, name: &str) -> bool {
        self.unset
            .iter()
            .any(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => name.starts_with(prefix),
                None => name == pattern,
            })
    }

    /// Names from `inherited` that [`ProcessEnv::unset`] removes. Exact names
    /// are always returned (removing an absent variable is harmless);
    /// patterns are expanded against `inherited`.
    #[must_use]
    pub fn names_to_remove<I>(&self, inherited: I) -> Vec<OsString>
    where
        I: IntoIterator<Item = OsString>,
    {
        let mut names: Vec<OsString> = self
            .unset
            .iter()
            .filter(|pattern| !pattern.ends_with('*'))
            .map(OsString::from)
            .collect();
        for name in inherited {
            if let Some(text) = name.to_str()
                && self.unsets(text)
                && !names.contains(&name)
            {
                names.push(name);
            }
        }
        names
    }

    /// The `PATH` value the child should see, or `None` when there is
    /// nothing to prepend. `base` is the `PATH` the child would otherwise
    /// get.
    #[must_use]
    pub fn joined_path(&self, base: Option<OsString>) -> Option<OsString> {
        if self.path_prepend.is_empty() {
            return None;
        }
        let mut parts: Vec<PathBuf> = self.path_prepend.clone();
        if let Some(base) = base {
            parts.extend(std::env::split_paths(&base));
        }
        std::env::join_paths(parts).ok()
    }

    /// Apply these adjustments (plus `launch_env`, see the type docs for the
    /// order) to a command about to be spawned.
    pub fn apply(
        &self,
        command: &mut tokio::process::Command,
        launch_env: &std::collections::BTreeMap<String, String>,
    ) {
        for name in self.names_to_remove(std::env::vars_os().map(|(name, _)| name)) {
            command.env_remove(name);
        }
        command.envs(launch_env);
        for (name, value) in &self.set {
            command.env(name, value);
        }
        let base = self
            .set
            .iter()
            .rev()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| OsString::from(value))
            .or_else(|| launch_env.get("PATH").map(OsString::from))
            .or_else(|| std::env::var_os("PATH"));
        if let Some(path) = self.joined_path(base) {
            command.env("PATH", path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_supports_exact_names_and_prefixes() {
        let env = ProcessEnv::new()
            .with_unset("ANTHROPIC_API_KEY")
            .with_unset("CLAUDE_CODE_USE_*");
        assert!(env.unsets("ANTHROPIC_API_KEY"));
        assert!(env.unsets("CLAUDE_CODE_USE_BEDROCK"));
        assert!(!env.unsets("ANTHROPIC_API_KEY_2"));
        assert!(!env.unsets("CLAUDE_CONFIG_DIR"));
    }

    #[test]
    fn names_to_remove_expands_patterns_against_inherited() {
        let env = ProcessEnv::new()
            .with_unset("OPENAI_API_KEY")
            .with_unset("CLAUDE_CODE_USE_*");
        let names = env.names_to_remove(
            ["CLAUDE_CODE_USE_VERTEX", "HOME", "CLAUDE_CODE_USE_BEDROCK"]
                .into_iter()
                .map(OsString::from),
        );
        assert_eq!(
            names,
            vec![
                OsString::from("OPENAI_API_KEY"),
                OsString::from("CLAUDE_CODE_USE_VERTEX"),
                OsString::from("CLAUDE_CODE_USE_BEDROCK"),
            ]
        );
    }

    #[test]
    fn joined_path_puts_prepended_dirs_first() {
        let env = ProcessEnv::new().with_path_prepend("/opt/cincel/node/bin");
        let joined = env
            .joined_path(Some(OsString::from("/usr/bin:/bin")))
            .expect("path");
        assert_eq!(joined, OsString::from("/opt/cincel/node/bin:/usr/bin:/bin"));
        assert!(ProcessEnv::new().joined_path(None).is_none());
        // An empty system PATH still yields the private runtime.
        assert_eq!(
            env.joined_path(None).expect("path"),
            OsString::from("/opt/cincel/node/bin")
        );
    }
}
