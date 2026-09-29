//! Error type of `cincel-connections`. Messages are user-facing (Spanish).

use std::path::PathBuf;

/// Errors produced by `cincel-connections`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectionsError {
    /// Filesystem error.
    #[error("error de E/S en {path}: {source}")]
    Io {
        /// Offending path.
        path: PathBuf,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// `connections.json` could not be parsed or written.
    #[error("el índice de conexiones es inválido: {0}")]
    Index(#[from] serde_json::Error),

    /// No connection with that id.
    #[error("no existe la conexión `{0}`")]
    UnknownConnection(String),

    /// No pending (half-created) profile with that id.
    #[error("no hay un perfil pendiente `{0}`")]
    UnknownPending(String),

    /// The connection label is empty.
    #[error("el nombre de la conexión no puede estar vacío")]
    EmptyLabel,

    /// The agent id is not one Cincel can connect by subscription.
    #[error("el agente `{0}` no admite conexión por suscripción")]
    UnsupportedAgent(String),

    /// A path that was about to be deleted is not inside the connections root.
    #[error("se rechazó borrar `{path}`: está fuera de la carpeta de conexiones")]
    UnsafePath {
        /// Refused path.
        path: PathBuf,
    },

    /// Network failure (runtime or adapter download).
    #[error("sin conexión a internet o servidor no disponible: {0}")]
    Network(String),

    /// The downloaded file did not match its published SHA-256.
    #[error("la suma SHA-256 de `{file}` no coincide (esperada {expected}, obtenida {actual})")]
    ChecksumMismatch {
        /// File name.
        file: String,
        /// Published digest.
        expected: String,
        /// Computed digest.
        actual: String,
    },

    /// `index.json` / `SHASUMS256.txt` did not have what was needed.
    #[error("no se encontró una versión de Node para esta plataforma: {0}")]
    NoNodeRelease(String),

    /// Archive extraction failed.
    #[error("no se pudo extraer `{file}`: {message}")]
    Extract {
        /// Archive name.
        file: String,
        /// Description of the failure.
        message: String,
    },

    /// The private Node runtime is not installed (and there is no network to
    /// install it).
    #[error("falta el entorno de Node privado de Cincel")]
    RuntimeMissing,

    /// The adapter for this agent is not installed.
    #[error("el adaptador de `{0}` no está instalado")]
    AdapterMissing(String),

    /// `npm install` failed.
    #[error("no se pudo instalar el adaptador `{package}`: {message}")]
    InstallFailed {
        /// Package spec.
        package: String,
        /// npm's output tail or the spawn error.
        message: String,
    },

    /// The registry has no entry (or not the expected distribution) for the
    /// agent.
    #[error("el registro de agentes no publica `{0}` como Cincel lo espera")]
    NotInRegistry(String),

    /// The registry publishes no archive of this agent for this machine.
    #[error("{agent} no tiene una versión para esta plataforma ({platform})")]
    NoBinaryForPlatform {
        /// Agent display name.
        agent: String,
        /// Registry platform key (`linux-x86_64`...).
        platform: String,
    },

    /// Not enough free disk space to download and unpack an agent.
    #[error(
        "no hay espacio suficiente en disco para instalar {agent}: hacen falta {} libres en {} y hay {}",
        human_size(*.needed),
        .path.display(),
        human_size(*.available)
    )]
    NotEnoughSpace {
        /// Agent display name.
        agent: String,
        /// Directory whose filesystem is short on space.
        path: PathBuf,
        /// Bytes needed.
        needed: u64,
        /// Bytes available.
        available: u64,
    },

    /// The login pty could not be opened or the command could not start.
    #[error("no se pudo iniciar el inicio de sesión: {0}")]
    LoginSpawn(String),

    /// The operation was cancelled through its
    /// [`CancelToken`](crate::CancelToken); partial files were removed.
    #[error("Cancelado")]
    Cancelled,

    /// The ACP layer failed.
    #[error(transparent)]
    Acp(#[from] cincel_acp::AcpError),
}

/// `1,4 GB`, `320 MB`: decimal units with a comma, as the UI speaks.
#[must_use]
pub fn human_size(bytes: u64) -> String {
    const GB: u64 = 1_000_000_000;
    const MB: u64 = 1_000_000;
    if bytes >= GB {
        let tenths = (bytes + GB / 20) / (GB / 10);
        format!("{},{} GB", tenths / 10, tenths % 10)
    } else {
        format!("{} MB", bytes.div_ceil(MB))
    }
}

/// Convenient result alias.
pub type Result<T, E = ConnectionsError> = std::result::Result<T, E>;

/// Helper to attach a path to an `io::Error`.
pub(crate) fn io_err(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> ConnectionsError {
    let path = path.into();
    move |source| ConnectionsError::Io { path, source }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_the_ui() {
        assert_eq!(human_size(1_400_000_000), "1,4 GB");
        assert_eq!(human_size(1_386_000_000), "1,4 GB");
        assert_eq!(human_size(333_590_110), "334 MB");
        assert_eq!(human_size(0), "0 MB");
        let error = ConnectionsError::NotEnoughSpace {
            agent: "Google Antigravity".to_string(),
            path: PathBuf::from("/datos"),
            needed: 1_400_000_000,
            available: 800_000_000,
        };
        assert_eq!(
            error.to_string(),
            "no hay espacio suficiente en disco para instalar Google Antigravity: hacen falta \
             1,4 GB libres en /datos y hay 800 MB"
        );
    }
}
