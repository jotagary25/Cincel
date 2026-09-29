//! Cliente ACP de Cincel (etapa E2).
//!
//! This crate discovers, launches and talks to ACP agents. It has no GUI
//! dependency: it runs on its own tokio runtime thread and communicates with
//! the UI through two `async-channel`s carrying [`AgentCommand`] and
//! [`AgentEvent`] (`docs/specs/03-arquitectura.md` §3).
//!
//! ```no_run
//! use cincel_acp::{AgentCommand, AgentConnection, AgentRegistry};
//!
//! # async fn demo() -> cincel_acp::Result<()> {
//! let registry = AgentRegistry::load()?;
//! let descriptor = registry.get("claude-acp").expect("agente");
//! let launch = registry.launch_command(descriptor)?;
//!
//! let connection = AgentConnection::start(std::env::current_dir()?);
//! connection
//!     .send(AgentCommand::Spawn { launch, cwd: std::env::current_dir()? })
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! # API summary (for the UI crates)
//!
//! * [`registry`]: [`AgentRegistry::load`]/`load_from`/`fetch`/`parse` build a
//!   registry from the CDN or a cache; [`AgentRegistry::with_custom`] merges
//!   in `settings.json` agents (`CustomAgent`, wins on id clash);
//!   [`AgentRegistry::launch_command`] resolves a [`LaunchSpec`] for any
//!   channel (`custom` > `npx` > `uvx` > `binary`), checking `node`/`uv`
//!   availability and binary install state along the way;
//!   [`AgentRegistry::install`] downloads/verifies/extracts a `binary`
//!   distribution (see [`install::InstallPlan`]) and
//!   [`AgentRegistry::is_installed`] checks it without touching the network.
//! * [`connection`]: [`AgentConnection::start`] takes the sandbox
//!   `project_root` and returns a handle with `send`/`recv`/`shutdown`; drive
//!   it with [`AgentCommand`] and consume [`AgentEvent`]. Sessions: `NewSession`,
//!   `LoadSession`, `ResumeSession` (gated by the agent's advertised
//!   capabilities) all resolve to `AgentEvent::SessionCreated`; `LoadSession`
//!   also replays history as `AgentEvent::Update` before that. Prompts take
//!   `Vec<protocol::PromptBlock>` plus optional review `feedback`, wrapped in
//!   `<user_review_feedback>` (`protocol::build_prompt_content`).
//!   `agentFileChangeReport` is negotiated automatically when the agent
//!   supports it and surfaces as `AgentEvent::FileChangeReport`. Cancellation
//!   (`AgentCommand::Cancel`) answers every pending permission/elicitation of
//!   that session with `cancelled` and, once the turn actually stops,
//!   emits `AgentEvent::ToolCallsCancelled` for tool calls still
//!   pending/in-progress. `fs/read_text_file`/`fs/write_text_file` outside
//!   `project_root` are rejected with `invalid_params` before reaching the UI.
//!   `AgentEvent::Exited`/`Error` carry the last 64 KiB of the agent's stderr.
//! * [`permission_policy`]: the single fixed [`PermissionPolicy`]
//!   (`PermissionPolicy::default()` auto-allows `edit|read|search|think`,
//!   asks for everything else and for sensitive paths);
//!   [`PermissionPolicy::with_sensitive_globs`] replaces the built-in
//!   sensitive-path globs (`globset`) with ones from settings.
//! * [`protocol`]: wire-adjacent helper types — `PromptBlock`,
//!   `McpServerSpec`, `AuthMethodView` (ready-to-copy shell command for
//!   `Terminal` auth methods), `FileChangeReport`.
//!
//! # Etapa 4 (conexiones)
//!
//! * [`AgentConnection::start_with_env`] applies a [`ProcessEnv`] (profile
//!   variable, stripped credential variables, private Node first in `PATH`)
//!   to every spawned agent. `NO_BROWSER` is no longer forced; opt in with
//!   [`LaunchSpec::with_no_browser`].
//! * [`AgentRegistry::launch_command_with_node`] runs `npx` distributions
//!   through a given `node` binary (no global `npx`).
//! * `agentFileChangeReport` uses the real `jetbrains.air` shape
//!   (`{"version":1,"capabilities":[...]}`), sent in
//!   `clientCapabilities._meta` and read from the top-level `_meta` of the
//!   `InitializeResponse`.
//! * [`AgentCommand::Logout`] -> [`AgentEvent::LoggedOut`];
//!   [`AgentCommand::Authenticate`] runs off the command loop and ends with
//!   [`AgentEvent::AuthSucceeded`]/[`AgentEvent::AuthFailed`];
//!   `_auth/status_update` -> [`AgentEvent::AuthStatus`];
//!   `elicitation/complete` -> [`AgentEvent::ElicitationCompleted`].
//!
//! # Etapa 5
//!
//! * [`AgentEvent::AuthRequired`] carries `message`: the agent's reason
//!   ([`auth_required_message`] over the -32000 error's `data`/`message`,
//!   falling back to the last logged-out `_auth/status_update`,
//!   [`logged_out_status_text`]). Verbatim: callers redact it.
//!
//! # Desviaciones (ver `docs/specs/modulos/acp.md` §Desviaciones)
//!
//! * `AuthMethodView::kind` only distinguishes `Terminal` from `Other`: the
//!   ACP v1 wire schema has no `Url` discriminator for agent-handled auth
//!   methods, so a URL/OAuth flow driven through `authenticate` is
//!   indistinguishable from any other agent-handled method at this layer.
//! * [`AgentConnection::start`] keeps its signature (no env) so existing
//!   callers compile; the env-aware constructor is
//!   [`AgentConnection::start_with_env`].

#![deny(missing_docs)]

pub mod connection;
pub mod error;
pub mod install;
pub mod permission_policy;
pub mod process_env;
pub mod protocol;
pub mod registry;

pub use connection::{AgentConnection, CLIENT_NAME, CLIENT_VERSION, client_capabilities};
pub use error::{AcpError, Result};
pub use install::InstallPlan;
pub use permission_policy::{AutoAnswer, PermissionPolicy, pick_allow_option, pick_reject_option};
pub use process_env::ProcessEnv;
pub use protocol::{
    AgentCommand, AgentEvent, AuthAccount, AuthMethodKind, AuthMethodView, AuthStatus,
    AuthStatusKind, ElicitationResponder, FileChangeReport, FsError, McpServerSpec,
    PermissionOutcome, PermissionRequestId, PermissionResponder, PromptBlock,
    agent_supports_auth_status, agent_supports_file_change_report, agent_supports_logout,
    auth_required_message, logged_out_status_text,
};
pub use registry::{
    AgentDescriptor, AgentRegistry, BinaryTarget, CACHE_TTL, CustomAgent, Distribution, LaunchSpec,
    NpxDistribution, REGISTRY_URL, RegistrySource, UvxDistribution, ensure_node_available,
    ensure_uv_available, node_major_version, npx_cli_for_node, pin_version, uv_available,
};

/// Re-export of the protocol crate so downstream crates share the same types.
pub use agent_client_protocol as acp;

impl From<std::io::Error> for AcpError {
    fn from(source: std::io::Error) -> Self {
        AcpError::Io {
            path: std::path::PathBuf::new(),
            source,
        }
    }
}
