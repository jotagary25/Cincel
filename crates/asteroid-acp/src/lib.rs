//! Cliente ACP de Asteroid (etapa E0-b).
//!
//! This crate discovers, launches and talks to ACP agents. It has no GUI
//! dependency: it runs on its own tokio runtime thread and communicates with
//! the UI through two `async-channel`s carrying [`AgentCommand`] and
//! [`AgentEvent`] (`docs/specs/03-arquitectura.md` §3).
//!
//! ```no_run
//! use asteroid_acp::{AgentCommand, AgentConnection, AgentRegistry};
//!
//! # async fn demo() -> asteroid_acp::Result<()> {
//! let registry = AgentRegistry::load()?;
//! let descriptor = registry.get("claude-acp").expect("agente");
//! let launch = registry.launch_command(descriptor)?;
//!
//! let connection = AgentConnection::start();
//! connection
//!     .send(AgentCommand::Spawn { launch, cwd: std::env::current_dir()? })
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! What E0 covers: registry + npx launch, `initialize`, `session/new`,
//! `session/prompt`, `session/cancel`, mode and config options, permission
//! requests and `fs/read_text_file` / `fs/write_text_file`. Everything else in
//! `docs/specs/modulos/acp.md` (custom agents from settings, binary downloads,
//! `session/load`, MCP passthrough, `agentFileChangeReport`, elicitation,
//! `resource_link` mentions and the review feedback block) is Etapa 2 work.

#![deny(missing_docs)]

pub mod autonomy;
pub mod connection;
pub mod error;
pub mod protocol;
pub mod registry;

pub use autonomy::{AutoAnswer, Autonomy, pick_allow_option, pick_reject_option};
pub use connection::{AgentConnection, CLIENT_NAME, CLIENT_VERSION, client_capabilities};
pub use error::{AcpError, Result};
pub use protocol::{
    AgentCommand, AgentEvent, FsError, PermissionOutcome, PermissionRequestId, PermissionResponder,
};
pub use registry::{
    AgentDescriptor, AgentRegistry, BinaryTarget, CACHE_TTL, Distribution, LaunchSpec,
    NpxDistribution, REGISTRY_URL, RegistrySource, UvxDistribution, ensure_node_available,
    node_major_version,
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
