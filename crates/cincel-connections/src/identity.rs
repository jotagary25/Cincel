//! Identity confirmation after a login (spec 06 §4 F2 step 5): spawn the
//! adapter once with the profile and read `_auth/status_update`, or fall
//! back to the provider CLI's status command through the adapter's bundled
//! binary, or (Antigravity, which announces no `_meta.authStatus`) to the
//! profile files.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use cincel_acp::{
    AgentCommand, AgentConnection, AgentEvent, AuthStatusKind, LaunchSpec, ProcessEnv,
};

use crate::envutil;
use crate::profile::{AgentKind, Profile};
use crate::store::Identity;

/// What a probe learned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbeOutcome {
    /// `Some(true)` logged in, `Some(false)` known logged out, `None`
    /// unknown (the probe could not tell).
    pub logged_in: Option<bool>,
    /// Identity, when reported.
    pub identity: Option<Identity>,
}

/// Something that can tell whether a profile is logged in, and as whom.
pub trait IdentityProbe: Send {
    /// Run the probe (blocking; called from the login thread).
    fn probe(&self) -> ProbeOutcome;
}

/// Spawn the adapter over ACP with the profile environment, wait for
/// `initialize`, then for an `_auth/status_update` (when the agent announces
/// `agentCapabilities._meta.authStatus`). Falls back to `fallback` when no
/// status arrives.
pub struct AcpIdentityProbe {
    /// Adapter launch.
    pub launch: LaunchSpec,
    /// Profile environment (see [`Profile::process_env`]).
    pub env: ProcessEnv,
    /// Working directory and sandbox root (the profile directory).
    pub cwd: PathBuf,
    /// How long to wait for `initialize` plus the status push.
    pub wait: Duration,
    /// Used when the agent does not push a status in time.
    pub fallback: Option<Box<dyn IdentityProbe>>,
}

impl IdentityProbe for AcpIdentityProbe {
    fn probe(&self) -> ProbeOutcome {
        let outcome = acp_status(&self.launch, &self.env, &self.cwd, self.wait);
        match (outcome, &self.fallback) {
            (Some(outcome), _) => outcome,
            (None, Some(fallback)) => fallback.probe(),
            (None, None) => ProbeOutcome::default(),
        }
    }
}

fn identity_from_status(
    kind: &AuthStatusKind,
    label: &str,
    account: Option<cincel_acp::AuthAccount>,
) -> ProbeOutcome {
    if *kind == AuthStatusKind::None {
        return ProbeOutcome {
            logged_in: Some(false),
            identity: None,
        };
    }
    let account = account.unwrap_or_default();
    let identity = Identity {
        email: account.email,
        plan: account
            .plan
            .or_else(|| (!label.is_empty()).then(|| label.to_string())),
        organization: account.organization,
    };
    ProbeOutcome {
        logged_in: Some(true),
        identity: (!identity.is_empty()).then_some(identity),
    }
}

/// Spawn the agent and read one status push. `None` when nothing
/// conclusive arrived within `wait`.
fn acp_status(
    launch: &LaunchSpec,
    env: &ProcessEnv,
    cwd: &std::path::Path,
    wait: Duration,
) -> Option<ProbeOutcome> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .ok()?;
    let mut connection = AgentConnection::start_with_env(cwd.to_path_buf(), env.clone());
    let result = runtime.block_on(async {
        connection
            .send(AgentCommand::Spawn {
                launch: launch.clone(),
                cwd: cwd.to_path_buf(),
            })
            .await
            .ok()?;
        tokio::time::timeout(wait, async {
            let mut announces_status = true;
            loop {
                match connection.recv().await.ok()? {
                    AgentEvent::Connected { capabilities, .. } => {
                        announces_status = cincel_acp::agent_supports_auth_status(&capabilities);
                        if !announces_status {
                            return None;
                        }
                    }
                    AgentEvent::AuthStatus {
                        kind,
                        label,
                        account,
                        ..
                    } => return Some(identity_from_status(&kind, &label, account)),
                    AgentEvent::Exited { .. } => return None,
                    _ if !announces_status => return None,
                    _ => {}
                }
            }
        })
        .await
        .ok()
        .flatten()
    });
    connection.shutdown();
    result
}

/// `claude auth status --json` (Claude) or `codex login status` (Codex),
/// run through the adapter's bundled CLI with the profile environment.
pub struct CliStatusProbe {
    /// Which CLI output format to expect.
    pub kind: AgentKind,
    /// The status command (`node <adapter> --cli auth status --json`...).
    pub command: LaunchSpec,
    /// Profile environment.
    pub env: ProcessEnv,
    /// Working directory.
    pub cwd: PathBuf,
}

impl IdentityProbe for CliStatusProbe {
    fn probe(&self) -> ProbeOutcome {
        let mut command = std::process::Command::new(&self.command.program);
        command
            .args(&self.command.args)
            .current_dir(&self.cwd)
            .stdin(Stdio::null());
        envutil::apply_std(&mut command, &self.env, &BTreeMap::new());
        let Ok(output) = command.output() else {
            return ProbeOutcome::default();
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        match self.kind {
            AgentKind::Claude => parse_claude_status(&stdout),
            AgentKind::Codex => parse_codex_status(output.status.success(), &stdout),
            AgentKind::Antigravity => ProbeOutcome::default(),
        }
    }
}

/// Parse `claude auth status --json` (`loggedIn`, `email`, `orgName`,
/// `subscriptionType`; logged-out exits 1 but still prints JSON).
#[must_use]
pub fn parse_claude_status(stdout: &str) -> ProbeOutcome {
    let Some(start) = stdout.find('{') else {
        return ProbeOutcome::default();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&stdout[start..]) else {
        return ProbeOutcome::default();
    };
    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let logged_in = value.get("loggedIn").and_then(serde_json::Value::as_bool);
    let identity = Identity {
        email: text("email"),
        plan: text("subscriptionType"),
        organization: text("orgName"),
    };
    ProbeOutcome {
        logged_in,
        identity: (logged_in == Some(true) && !identity.is_empty()).then_some(identity),
    }
}

/// Parse `codex login status`: exit 0 and "Logged in" means logged in (it
/// prints no email).
#[must_use]
pub fn parse_codex_status(success: bool, stdout: &str) -> ProbeOutcome {
    let lower = stdout.to_lowercase();
    let logged_in = if lower.contains("not logged in") {
        Some(false)
    } else if success && lower.contains("logged in") {
        Some(true)
    } else if success {
        None
    } else {
        Some(false)
    };
    ProbeOutcome {
        logged_in,
        identity: None,
    }
}

/// Offline probe from the profile files: the credentials file decides
/// whether the profile is logged in, [`Profile::offline_identity`] adds an
/// email when the file has one (Antigravity).
pub struct ProfileProbe {
    /// The profile to inspect.
    pub profile: Profile,
}

impl IdentityProbe for ProfileProbe {
    fn probe(&self) -> ProbeOutcome {
        ProbeOutcome {
            logged_in: Some(self.profile.has_credentials()),
            identity: self.profile.offline_identity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_status_json_is_parsed() {
        let out = parse_claude_status(
            r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","email":"gary@example.com","orgName":"Org","subscriptionType":"max"}"#,
        );
        assert_eq!(out.logged_in, Some(true));
        let identity = out.identity.expect("identity");
        assert_eq!(identity.email.as_deref(), Some("gary@example.com"));
        assert_eq!(identity.plan.as_deref(), Some("max"));
        assert_eq!(identity.organization.as_deref(), Some("Org"));
        let logged_out = parse_claude_status(r#"{"loggedIn":false}"#);
        assert_eq!(logged_out.logged_in, Some(false));
        assert_eq!(parse_claude_status("basura"), ProbeOutcome::default());
    }

    #[test]
    fn codex_status_text_is_parsed() {
        assert_eq!(
            parse_codex_status(true, "Logged in using ChatGPT\n").logged_in,
            Some(true)
        );
        assert_eq!(
            parse_codex_status(false, "Not logged in\n").logged_in,
            Some(false)
        );
        assert_eq!(parse_codex_status(true, "").logged_in, None);
    }

    #[test]
    fn auth_status_none_means_logged_out_and_label_is_plan_fallback() {
        let out = identity_from_status(&AuthStatusKind::None, "Not logged in", None);
        assert_eq!(out.logged_in, Some(false));
        let out = identity_from_status(&AuthStatusKind::Account, "Claude Max", None);
        assert_eq!(out.logged_in, Some(true));
        assert_eq!(
            out.identity.and_then(|identity| identity.plan),
            Some("Claude Max".to_string())
        );
    }

    #[test]
    fn profile_probe_reads_credentials_presence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let profile = Profile::new(AgentKind::Antigravity, dir.path());
        let probe = ProfileProbe {
            profile: profile.clone(),
        };
        assert_eq!(probe.probe().logged_in, Some(false));
        std::fs::create_dir_all(dir.path().join("antigravity-acp")).expect("mkdir");
        std::fs::write(profile.credentials_file(), "{\"x\":1}").expect("write");
        assert_eq!(probe.probe().logged_in, Some(true));
    }
}
