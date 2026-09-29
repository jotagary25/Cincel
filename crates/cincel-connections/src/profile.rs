//! Per-agent isolated profiles (`docs/specs/06-etapa4-conexiones-y-cincel.md`
//! §2): which variable points the agent at its profile directory, which
//! inherited variables are stripped, what is seeded, and where the agent
//! writes its credentials.

use std::path::{Path, PathBuf};

use cincel_acp::ProcessEnv;

use crate::error::{ConnectionsError, Result, io_err};
use crate::paths::write_atomic;

/// Agents Cincel can connect by subscription in this stage.
///
/// Gemini CLI was here until 2026-09-27: Google stopped accepting its
/// personal login ("This client is no longer supported for Gemini Code
/// Assist for individuals... migrate to Antigravity", `IneligibleTierError`
/// from gemini-cli 0.61.0), so Google's official Antigravity ACP server took
/// its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    /// `claude-acp` (`@agentclientprotocol/claude-agent-acp`).
    Claude,
    /// `codex-acp` (`@agentclientprotocol/codex-acp`).
    Codex,
    /// `antigravity-acp` (Google's prebuilt `agy_acp_server`, a `binary`
    /// distribution of the registry).
    Antigravity,
}

/// Where an agent's adapter comes from in the ACP registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterSource {
    /// An npm package run with the private Node runtime.
    Npm {
        /// Package name (the registry pins the version).
        package: &'static str,
        /// The `bin` entry of the package that speaks ACP.
        command: &'static str,
    },
    /// A prebuilt archive (`distribution.binary.<platform>`), no Node needed.
    Binary,
}

/// How the provider login of an agent runs (spec 06 §4 F2 step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginMethod {
    /// The provider's own login command in a hidden pty (Claude, Codex).
    Pty,
    /// ACP `authenticate` with this method id against the adapter itself
    /// (Antigravity: `oauth-personal`); the link arrives on stderr or as a
    /// URL elicitation and the login completes by the agent's local
    /// redirect server.
    AcpAuthenticate(&'static str),
}

/// Registry ids of agents Cincel no longer supports, with the reason shown
/// on their saved connections ("No disponible"). They can still be deleted.
pub const RETIRED_AGENTS: &[(&str, &str)] = &[(
    "gemini",
    "Google dejó de aceptar el inicio de sesión personal de Gemini CLI. \
     Eliminá esta conexión y conectá Google Antigravity.",
)];

impl AgentKind {
    /// All supported agents, in display order.
    pub const ALL: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codex, AgentKind::Antigravity];

    /// Map a registry id onto a supported agent.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::UnsupportedAgent`] for any other id.
    pub fn from_agent_id(agent_id: &str) -> Result<Self> {
        match agent_id {
            "claude-acp" => Ok(Self::Claude),
            "codex-acp" => Ok(Self::Codex),
            "antigravity-acp" => Ok(Self::Antigravity),
            other => Err(ConnectionsError::UnsupportedAgent(other.to_string())),
        }
    }

    /// Registry id.
    #[must_use]
    pub fn agent_id(self) -> &'static str {
        match self {
            Self::Claude => "claude-acp",
            Self::Codex => "codex-acp",
            Self::Antigravity => "antigravity-acp",
        }
    }

    /// Where the adapter comes from.
    #[must_use]
    pub fn source(self) -> AdapterSource {
        match self {
            Self::Claude => AdapterSource::Npm {
                package: "@agentclientprotocol/claude-agent-acp",
                command: "claude-agent-acp",
            },
            Self::Codex => AdapterSource::Npm {
                package: "@agentclientprotocol/codex-acp",
                command: "codex-acp",
            },
            Self::Antigravity => AdapterSource::Binary,
        }
    }

    /// npm package of the adapter (the registry pins the version), `None`
    /// for binary distributions.
    #[must_use]
    pub fn npm_package(self) -> Option<&'static str> {
        match self.source() {
            AdapterSource::Npm { package, .. } => Some(package),
            AdapterSource::Binary => None,
        }
    }

    /// Whether running the adapter needs Cincel's private Node runtime.
    #[must_use]
    pub fn needs_node(self) -> bool {
        matches!(self.source(), AdapterSource::Npm { .. })
    }

    /// Short display name ("Antigravity · personal" labels, status lines).
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Antigravity => "Antigravity",
        }
    }

    /// Full product name for the agent grid.
    #[must_use]
    pub fn full_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Antigravity => "Google Antigravity",
        }
    }

    /// One line under the name in the agent grid.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Claude => "Tu suscripción de Claude (Pro o Max)",
            Self::Codex => "Tu suscripción de ChatGPT, vía Codex",
            Self::Antigravity => "Tu suscripción de Google, vía el agente oficial de Antigravity",
        }
    }

    /// Variable that points the agent at its profile directory. Antigravity's
    /// `GEMINI_HOME` names the `.gemini` directory itself: every file it
    /// keeps (token, settings, conversations) lives under
    /// `<GEMINI_HOME>/antigravity-acp/`.
    #[must_use]
    pub fn profile_var(self) -> &'static str {
        match self {
            Self::Claude => "CLAUDE_CONFIG_DIR",
            Self::Codex => "CODEX_HOME",
            Self::Antigravity => "GEMINI_HOME",
        }
    }

    /// How the login runs.
    #[must_use]
    pub fn login_method(self) -> LoginMethod {
        match self {
            Self::Claude | Self::Codex => LoginMethod::Pty,
            Self::Antigravity => LoginMethod::AcpAuthenticate("oauth-personal"),
        }
    }

    /// Whether the login flow asks the user to paste a code back (Claude
    /// only). Codex completes by its local callback, Antigravity by its local
    /// redirect server.
    #[must_use]
    pub fn login_needs_code(self) -> bool {
        matches!(self, Self::Claude)
    }
}

/// The reason a retired agent id is no longer available, if it is one.
#[must_use]
pub fn retired_reason(agent_id: &str) -> Option<&'static str> {
    RETIRED_AGENTS
        .iter()
        .find(|(id, _)| *id == agent_id)
        .map(|(_, reason)| *reason)
}

/// Provider credential variables stripped from every agent process (spec
/// 06 §2), plus `NO_BROWSER` (never injected nor inherited). A trailing `*`
/// is a prefix match. The list is the union for all providers: a connection
/// must depend on nothing from the user's shell, whatever the agent.
///
/// Google's entries are whole prefixes: Antigravity reads `GOOGLE_*`
/// (`GOOGLE_API_KEY`, `GOOGLE_CLOUD_PROJECT`, ADC), `GEMINI_*`
/// (`GEMINI_API_KEY`; our own `GEMINI_HOME` is set again after the strip),
/// `CLOUDSDK_*`/`GCLOUD_*` (gcloud's project and account), and its own
/// `ANTIGRAVITY_*`/`AGY_*` knobs (`ANTIGRAVITY_HARNESS_PATH`,
/// `AGY_ACP_CCPA_PROJECT`...). Any of them left in the user's shell would
/// otherwise change which account or project the isolated profile uses.
pub const STRIPPED_VARIABLES: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_PROFILE",
    "CLAUDE_CODE_USE_*",
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "GOOGLE_*",
    "GEMINI_*",
    "CLOUDSDK_*",
    "GCLOUD_*",
    "ANTIGRAVITY_*",
    "AGY_*",
    "NO_BROWSER",
];

/// `BROWSER` value that makes a login print its link instead of opening a
/// browser: Claude's CLI runs `$BROWSER <url>` before falling back to
/// `xdg-open`, `codex login` and Antigravity's Python `webbrowser.open`
/// honor it too. Cincel shows the link itself ("Abrir en el navegador").
pub(crate) fn neutral_browser() -> String {
    if Path::new("/bin/true").is_file() {
        "/bin/true".to_string()
    } else {
        "true".to_string()
    }
}

/// One connection's isolated profile directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    kind: AgentKind,
    dir: PathBuf,
}

impl Profile {
    /// Profile of `kind` rooted at `dir` (usually
    /// `<data>/connections/<uuid>`).
    #[must_use]
    pub fn new(kind: AgentKind, dir: impl Into<PathBuf>) -> Self {
        Self {
            kind,
            dir: dir.into(),
        }
    }

    /// Agent this profile belongs to.
    #[must_use]
    pub fn kind(&self) -> AgentKind {
        self.kind
    }

    /// Profile directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Environment for any process run against this profile (agent, login,
    /// status probes): the profile variable, every
    /// [`STRIPPED_VARIABLES`] entry removed, `BROWSER` neutralized for
    /// Antigravity, and `node_bin` (the private runtime's `bin/`) first in
    /// `PATH` when given.
    #[must_use]
    pub fn process_env(&self, node_bin: Option<&Path>) -> ProcessEnv {
        let mut env = ProcessEnv::new().with_set(
            self.kind.profile_var(),
            self.dir.to_string_lossy().into_owned(),
        );
        for name in STRIPPED_VARIABLES {
            env = env.with_unset(*name);
        }
        if self.kind == AgentKind::Antigravity {
            // The agent calls Python's `webbrowser.open` on every sign-in:
            // Cincel shows the link itself ("Abrir en el navegador").
            env = env.with_set("BROWSER", neutral_browser());
            // The server writes its own logs under the temp dir (they carry
            // the working directory and the profile path): keep them inside
            // the profile instead of the shared `/tmp`.
            let tmp = self.dir.join("tmp");
            let _ = std::fs::create_dir_all(&tmp);
            env = env.with_set("TMPDIR", tmp.to_string_lossy().into_owned());
        }
        if let Some(bin) = node_bin {
            env = env.with_path_prepend(bin);
        }
        env
    }

    /// Environment for a login session: [`Profile::process_env`] plus
    /// `BROWSER` neutralized for every agent, so no login opens a browser
    /// tab by itself (Claude's `auth login --claudeai` and `codex login`
    /// would otherwise launch the default browser). Scoped to logins: the
    /// chat process of Claude and Codex keeps the plain
    /// [`Profile::process_env`].
    #[must_use]
    pub fn login_env(&self, node_bin: Option<&Path>) -> ProcessEnv {
        let env = self.process_env(node_bin);
        if env.set.iter().any(|(name, _)| name == "BROWSER") {
            env
        } else {
            env.with_set("BROWSER", neutral_browser())
        }
    }

    /// Seed the minimum the agent needs to run without interaction (spec
    /// 06 §2): Codex gets `config.toml` with
    /// `cli_auth_credentials_store = "file"` (never the keyring). Claude and
    /// Antigravity start empty (Antigravity writes
    /// `antigravity-acp/settings.json` with `auth.type` itself after a
    /// successful `authenticate`; seeding it earlier would make `session/new`
    /// start a browser login inside the chat instead of answering
    /// `auth_required`). Nothing is copied from the user's own config.
    /// Existing files are left alone.
    ///
    /// # Errors
    ///
    /// I/O errors writing the seed files.
    pub fn seed(&self) -> Result<()> {
        match self.kind {
            AgentKind::Claude => Ok(()),
            AgentKind::Antigravity => {
                // Private temp dir for the server's logs (see `process_env`).
                std::fs::create_dir_all(self.dir.join("tmp")).map_err(|source| {
                    ConnectionsError::Io {
                        path: self.dir.join("tmp"),
                        source,
                    }
                })
            }
            AgentKind::Codex => {
                let config = self.dir.join("config.toml");
                if config.exists() {
                    return Ok(());
                }
                write_atomic(&config, b"cli_auth_credentials_store = \"file\"\n")
            }
        }
    }

    /// File the agent writes its credentials to inside the profile: Claude
    /// `.credentials.json`, Codex `auth.json`, Antigravity
    /// `antigravity-acp/acp_token.json` (the consumer token; the business
    /// one, `acp_business_token.json`, is never used by Cincel).
    #[must_use]
    pub fn credentials_file(&self) -> PathBuf {
        match self.kind {
            AgentKind::Claude => self.dir.join(".credentials.json"),
            AgentKind::Codex => self.dir.join("auth.json"),
            AgentKind::Antigravity => self.dir.join("antigravity-acp").join("acp_token.json"),
        }
    }

    /// Whether the credentials file exists and is not empty.
    #[must_use]
    pub fn has_credentials(&self) -> bool {
        std::fs::metadata(self.credentials_file()).is_ok_and(|meta| meta.len() > 0)
    }

    /// Identity readable from the profile without spawning anything
    /// (Antigravity: an `email` field of the token file, when present). The
    /// token file of `antigravity-acp` 1.2.1 holds `client_id`,
    /// `client_secret`, `refresh_token`, `token_uri`, `scopes` and
    /// `project_id` but no email, so today this is `None`; the agent does not
    /// announce `_auth/status_update` either.
    #[must_use]
    pub fn offline_identity(&self) -> Option<crate::Identity> {
        if self.kind != AgentKind::Antigravity {
            return None;
        }
        let body = std::fs::read_to_string(self.credentials_file()).ok()?;
        let value: serde_json::Value = serde_json::from_str(&body).ok()?;
        let email = value.get("email")?.as_str()?.trim();
        (!email.is_empty()).then(|| crate::Identity {
            email: Some(email.to_string()),
            plan: None,
            organization: None,
        })
    }

    /// Remove the credentials file (used to simulate an expired session in
    /// tests and by "Volver a conectar" before re-running the login).
    ///
    /// # Errors
    ///
    /// I/O errors other than "not found".
    pub fn clear_credentials(&self) -> Result<()> {
        let path = self.credentials_file();
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_err(path)(error)),
        }
    }
}

/// Status of a connection, computed when listing (never persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// Credentials present, runtime and adapter installed.
    Connected,
    /// The profile has no credentials any more (revoked, expired, deleted):
    /// "Volver a conectar".
    SessionExpired,
    /// Something needed to run is missing: "Reparar".
    Unavailable {
        /// What is missing, user-facing (Spanish).
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_ids_round_trip_and_unknown_is_rejected() {
        for kind in AgentKind::ALL {
            assert_eq!(AgentKind::from_agent_id(kind.agent_id()).expect("id"), kind);
        }
        for retired in ["opencode", "gemini"] {
            assert!(matches!(
                AgentKind::from_agent_id(retired),
                Err(ConnectionsError::UnsupportedAgent(_))
            ));
        }
        assert!(retired_reason("gemini").is_some_and(|why| why.contains("Antigravity")));
        assert!(retired_reason("claude-acp").is_none());
    }

    #[test]
    fn antigravity_is_a_binary_agent_logging_in_over_acp() {
        let kind = AgentKind::Antigravity;
        assert_eq!(kind.agent_id(), "antigravity-acp");
        assert_eq!(kind.source(), AdapterSource::Binary);
        assert!(!kind.needs_node());
        assert_eq!(kind.npm_package(), None);
        assert_eq!(
            kind.login_method(),
            LoginMethod::AcpAuthenticate("oauth-personal")
        );
        assert!(!kind.login_needs_code());
        assert_eq!(kind.display_name(), "Antigravity");
        assert_eq!(kind.full_name(), "Google Antigravity");
        assert_eq!(
            kind.description(),
            "Tu suscripción de Google, vía el agente oficial de Antigravity"
        );
        assert!(AgentKind::Claude.needs_node() && AgentKind::Codex.needs_node());
        assert_eq!(AgentKind::Claude.login_method(), LoginMethod::Pty);
    }

    #[test]
    fn process_env_sets_profile_var_strips_credentials_and_prepends_node() {
        let profile = Profile::new(AgentKind::Codex, "/data/connections/abc");
        let env = profile.process_env(Some(Path::new("/data/runtime/node-v24/bin")));
        assert_eq!(
            env.set,
            vec![(
                "CODEX_HOME".to_string(),
                "/data/connections/abc".to_string()
            )]
        );
        for name in [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "CLAUDE_CODE_USE_BEDROCK",
            "GEMINI_API_KEY",
            "GEMINI_CLI_HOME",
            "GOOGLE_API_KEY",
            "GOOGLE_CLOUD_PROJECT",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "GOOGLE_GENAI_USE_VERTEXAI",
            "GOOGLE_CLOUD_ACCESS_TOKEN",
            "GCLOUD_PROJECT",
            "CLOUDSDK_CORE_PROJECT",
            "ANTIGRAVITY_HARNESS_PATH",
            "AGY_ACP_CCPA_PROJECT",
            "NO_BROWSER",
        ] {
            assert!(env.unsets(name), "{name} debe quitarse");
        }
        assert!(!env.unsets("HOME"));
        assert_eq!(
            env.path_prepend,
            vec![PathBuf::from("/data/runtime/node-v24/bin")]
        );
        let claude = Profile::new(AgentKind::Claude, "/p").process_env(None);
        assert_eq!(claude.set[0].0, "CLAUDE_CONFIG_DIR");
        assert!(claude.path_prepend.is_empty());
        assert!(!claude.set.iter().any(|(name, _)| name == "BROWSER"));
    }

    #[test]
    fn every_login_env_mutes_the_browser_but_chat_env_does_not() {
        let neutral = neutral_browser();
        for kind in AgentKind::ALL {
            let profile = Profile::new(kind, "/data/connections/x");
            let login = profile.login_env(Some(Path::new("/node/bin")));
            let browsers: Vec<&str> = login
                .set
                .iter()
                .filter(|(name, _)| name == "BROWSER")
                .map(|(_, value)| value.as_str())
                .collect();
            assert_eq!(browsers, vec![neutral.as_str()], "{kind:?}");
            assert_eq!(login.set[0].0, kind.profile_var(), "{kind:?}");
        }
        for kind in [AgentKind::Claude, AgentKind::Codex] {
            let chat = Profile::new(kind, "/p").process_env(None);
            assert!(
                !chat.set.iter().any(|(name, _)| name == "BROWSER"),
                "{kind:?}: el proceso del chat no toca BROWSER"
            );
        }
    }

    #[test]
    fn antigravity_env_points_gemini_home_at_the_profile_and_mutes_the_browser() {
        let env = Profile::new(AgentKind::Antigravity, "/data/connections/agy").process_env(None);
        assert_eq!(
            env.set[0],
            (
                "GEMINI_HOME".to_string(),
                "/data/connections/agy".to_string()
            )
        );
        let browser = env
            .set
            .iter()
            .find(|(name, _)| name == "BROWSER")
            .map(|(_, value)| value.as_str());
        assert!(
            browser.is_some_and(|value| value.ends_with("true")),
            "{browser:?}"
        );
        assert!(env.path_prepend.is_empty(), "Antigravity no usa Node");
        // `GEMINI_*` is stripped from the inherited environment and our own
        // `GEMINI_HOME` is set again afterwards (unset runs before set).
        assert!(env.unsets("GEMINI_HOME"));
        let resolved = crate::envutil::resolve(&env, &std::collections::BTreeMap::new());
        let last = resolved
            .iter()
            .rev()
            .find(|(name, _)| name == "GEMINI_HOME")
            .and_then(|(_, value)| value.clone());
        assert_eq!(last, Some("/data/connections/agy".into()));
    }

    #[test]
    fn seed_writes_the_codex_file_store_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let codex = Profile::new(AgentKind::Codex, dir.path().join("codex"));
        codex.seed().expect("seed");
        let config = std::fs::read_to_string(codex.dir().join("config.toml")).expect("config");
        assert_eq!(config, "cli_auth_credentials_store = \"file\"\n");

        let claude = Profile::new(AgentKind::Claude, dir.path().join("claude"));
        std::fs::create_dir_all(claude.dir()).expect("mkdir");
        claude.seed().expect("seed");
        assert_eq!(
            std::fs::read_dir(claude.dir()).expect("ls").count(),
            0,
            "Claude arranca con un perfil vacío"
        );

        // Antigravity only gets a private `tmp/` for the server's own logs,
        // so nothing of it lands in the shared `/tmp`.
        let agy = Profile::new(AgentKind::Antigravity, dir.path().join("agy"));
        std::fs::create_dir_all(agy.dir()).expect("mkdir");
        agy.seed().expect("seed");
        let entries: Vec<_> = std::fs::read_dir(agy.dir())
            .expect("ls")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("tmp")]);
        let env = agy.process_env(None);
        let resolved = crate::envutil::resolve(&env, &std::collections::BTreeMap::new());
        let tmpdir = resolved
            .iter()
            .rev()
            .find(|(name, _)| name == "TMPDIR")
            .and_then(|(_, value)| value.clone());
        assert_eq!(tmpdir, Some(agy.dir().join("tmp").into_os_string()));
    }

    #[test]
    fn seed_keeps_existing_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let codex = Profile::new(AgentKind::Codex, dir.path());
        std::fs::write(dir.path().join("config.toml"), "model = \"x\"\n").expect("write");
        codex.seed().expect("seed");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("config.toml")).expect("read"),
            "model = \"x\"\n"
        );
    }

    #[test]
    fn credentials_file_per_agent_and_detection() {
        let dir = tempfile::tempdir().expect("tempdir");
        let claude = Profile::new(AgentKind::Claude, dir.path());
        assert!(claude.credentials_file().ends_with(".credentials.json"));
        assert!(
            Profile::new(AgentKind::Codex, dir.path())
                .credentials_file()
                .ends_with("auth.json")
        );
        assert!(
            Profile::new(AgentKind::Antigravity, dir.path())
                .credentials_file()
                .ends_with("antigravity-acp/acp_token.json")
        );
        assert!(!claude.has_credentials());
        std::fs::write(claude.credentials_file(), "").expect("write");
        assert!(!claude.has_credentials(), "un archivo vacío no cuenta");
        std::fs::write(claude.credentials_file(), "{}").expect("write");
        assert!(claude.has_credentials());
        claude.clear_credentials().expect("clear");
        assert!(!claude.has_credentials());
        claude.clear_credentials().expect("idempotente");
    }

    #[test]
    fn antigravity_offline_identity_reads_an_email_only_if_the_token_has_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let profile = Profile::new(AgentKind::Antigravity, dir.path());
        assert!(profile.offline_identity().is_none());
        std::fs::create_dir_all(dir.path().join("antigravity-acp")).expect("mkdir");
        // The real 1.2.1 token: no email.
        std::fs::write(
            profile.credentials_file(),
            r#"{"client_id":"x","client_secret":"y","refresh_token":"z","token_uri":"u","scopes":[],"project_id":"p"}"#,
        )
        .expect("write");
        assert!(profile.has_credentials());
        assert!(profile.offline_identity().is_none());
        std::fs::write(
            profile.credentials_file(),
            r#"{"refresh_token":"z","email":"ana@example.com"}"#,
        )
        .expect("write");
        assert_eq!(
            profile
                .offline_identity()
                .and_then(|identity| identity.email),
            Some("ana@example.com".to_string())
        );
        assert!(
            Profile::new(AgentKind::Claude, dir.path())
                .offline_identity()
                .is_none()
        );
    }
}
