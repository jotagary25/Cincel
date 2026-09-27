//! Error type shared by the registry and the connection.

use std::path::PathBuf;

/// Errors produced by `cincel-acp`.
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

    /// `node` is missing or too old for an `npx` distribution.
    #[error("node no disponible para `npx`: {found}")]
    NodeMissing {
        /// Human-readable description of what was found instead (or nothing).
        found: String,
    },

    /// `uv`/`uvx` is missing for a `uvx` distribution.
    #[error("no se encontró `uvx` en el PATH")]
    UvMissing,

    /// The `binary` distribution for this agent was not installed yet.
    #[error("el agente `{id}` (`{kind}`) no está instalado; llamá a `AgentRegistry::install`")]
    NotInstalled {
        /// Agent id.
        id: String,
        /// Distribution kind, always `"binary"` today.
        kind: String,
    },

    /// The platform has no prebuilt binary for this agent.
    #[error("el agente `{id}` no publica un binario para `{platform}`")]
    NoBinaryForPlatform {
        /// Agent id.
        id: String,
        /// `current_platform_key()` value that had no match.
        platform: String,
    },

    /// The downloaded archive did not match its expected sha256.
    #[error("sha256 inválido para `{url}`: esperado {expected}, obtenido {actual}")]
    ChecksumMismatch {
        /// Archive URL.
        url: String,
        /// Expected digest, lowercase hex.
        expected: String,
        /// Digest actually computed, lowercase hex.
        actual: String,
    },

    /// The download was refused by the caller (`InstallPlan` confirmation).
    #[error("instalación cancelada por el usuario")]
    InstallDeclined,

    /// The archive could not be extracted (unknown format or corrupt data).
    #[error("no se pudo extraer el archivo `{url}`: {message}")]
    ExtractFailed {
        /// Archive URL.
        url: String,
        /// Description of the failure.
        message: String,
    },

    /// A path used by `fs/read_text_file` or `fs/write_text_file` is outside
    /// the project root, or the request parameters are otherwise invalid.
    #[error("parámetros inválidos: {0}")]
    InvalidParams(String),
}

impl From<agent_client_protocol::Error> for AcpError {
    fn from(value: agent_client_protocol::Error) -> Self {
        AcpError::Protocol(format!("{} ({:?})", value.message, value.code))
    }
}

/// Convenient result alias.
pub type Result<T, E = AcpError> = std::result::Result<T, E>;
