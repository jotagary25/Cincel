//! Error type shared by the registry and the connection.

use std::path::PathBuf;

/// Errors produced by `asteroid-acp`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AcpError {
    /// The registry could not be downloaded and no usable cache exists.
    #[error("no se pudo obtener el registro de agentes: {0}")]
    RegistryUnavailable(String),

    /// The registry payload could not be parsed.
    #[error("registro de agentes inválido: {0}")]
    RegistryParse(#[from] serde_json::Error),

    /// No agent with the requested id exists in the registry.
    #[error("no existe el agente `{0}` en el registro")]
    UnknownAgent(String),

    /// The distribution kind is known but not implemented yet (E0 only ships `npx`).
    #[error("la distribución `{kind}` del agente `{id}` todavía no está soportada")]
    NotSupportedYet {
        /// Agent id.
        id: String,
        /// Distribution kind (`binary`, `uvx`, ...).
        kind: String,
    },

    /// The agent process could not be spawned.
    #[error("no se pudo lanzar el agente `{program}`: {source}")]
    Spawn {
        /// Program that failed to start.
        program: String,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// Filesystem error while reading or writing the cache.
    #[error("error de E/S en {path}: {source}")]
    Io {
        /// Offending path.
        path: PathBuf,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// The protocol layer failed.
    #[error("error de protocolo ACP: {0}")]
    Protocol(String),

    /// The worker thread is gone.
    #[error("la conexión con el agente está cerrada")]
    Closed,
}

impl From<agent_client_protocol::Error> for AcpError {
    fn from(value: agent_client_protocol::Error) -> Self {
        AcpError::Protocol(format!("{} ({:?})", value.message, value.code))
    }
}

/// Convenient result alias.
pub type Result<T, E = AcpError> = std::result::Result<T, E>;
