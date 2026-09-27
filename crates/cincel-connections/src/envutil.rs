//! Apply a [`cincel_acp::ProcessEnv`] to the non-tokio command builders this
//! crate uses (`std::process::Command` for npm, `portable_pty::CommandBuilder`
//! for logins), with the same semantics as `ProcessEnv::apply`.

use std::collections::BTreeMap;
use std::ffi::OsString;

use cincel_acp::ProcessEnv;

/// The final environment changes as a list: `(name, Some(value))` to set,
/// `(name, None)` to remove. `extra` is layered between the unset step and
/// `env.set` (like a `LaunchSpec`'s env).
pub(crate) fn resolve(
    env: &ProcessEnv,
    extra: &BTreeMap<String, String>,
) -> Vec<(OsString, Option<OsString>)> {
    let mut out: Vec<(OsString, Option<OsString>)> = env
        .names_to_remove(std::env::vars_os().map(|(name, _)| name))
        .into_iter()
        .map(|name| (name, None))
        .collect();
    for (name, value) in extra {
        out.push((name.into(), Some(value.into())));
    }
    for (name, value) in &env.set {
        out.push((name.into(), Some(value.into())));
    }
    let base = env
        .set
        .iter()
        .rev()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| OsString::from(value))
        .or_else(|| extra.get("PATH").map(OsString::from))
        .or_else(|| std::env::var_os("PATH"));
    if let Some(path) = env.joined_path(base) {
        out.push(("PATH".into(), Some(path)));
    }
    out
}

pub(crate) fn apply_std(
    command: &mut std::process::Command,
    env: &ProcessEnv,
    extra: &BTreeMap<String, String>,
) {
    for (name, value) in resolve(env, extra) {
        match value {
            Some(value) => {
                command.env(name, value);
            }
            None => {
                command.env_remove(name);
            }
        }
    }
}

pub(crate) fn apply_pty(
    command: &mut portable_pty::CommandBuilder,
    env: &ProcessEnv,
    extra: &BTreeMap<String, String>,
) {
    for (name, value) in resolve(env, extra) {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_orders_unset_extra_set_and_path() {
        let env = ProcessEnv::new()
            .with_unset("OPENAI_API_KEY")
            .with_set("CODEX_HOME", "/p")
            .with_path_prepend("/node/bin");
        let mut extra = BTreeMap::new();
        extra.insert("BROWSER".to_string(), "/bin/true".to_string());
        extra.insert("CODEX_HOME".to_string(), "/pisado".to_string());
        let resolved = resolve(&env, &extra);
        assert_eq!(resolved[0], (OsString::from("OPENAI_API_KEY"), None));
        // The profile variable wins over `extra`.
        let last_codex_home = resolved
            .iter()
            .rev()
            .find(|(name, _)| name == "CODEX_HOME")
            .and_then(|(_, value)| value.clone());
        assert_eq!(last_codex_home, Some(OsString::from("/p")));
        let path = resolved
            .iter()
            .find(|(name, _)| name == "PATH")
            .and_then(|(_, value)| value.clone())
            .expect("PATH");
        assert!(path.to_string_lossy().starts_with("/node/bin"));
    }
}
