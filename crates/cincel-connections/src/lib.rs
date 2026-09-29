//! `cincel-connections`: isolated agent connections
//! (`docs/specs/06-etapa4-conexiones-y-cincel.md` §2-§4, §8).
//!
//! No GPUI. One connection = one agent + one private profile directory +
//! one label. This crate owns:
//!
//! * [`ConnectionStore`]: `connections.json` and `connections/<uuid>/`
//!   (0700), never holding tokens;
//! * [`Profile`]: the profile variable per agent (`CLAUDE_CONFIG_DIR`,
//!   `CODEX_HOME`, `GEMINI_HOME`), stripped credential variables, seed
//!   files and credential detection ([`ConnectionStatus`]);
//! * [`Runtime`]: the private Node LTS (download, SHA-256, resume, retry),
//!   only for npm adapters;
//! * [`Adapters`]: npm installs of the official adapters (Claude, Codex) and
//!   `binary` archives (Antigravity: download with progress and resume,
//!   SHA-256 when published, disk-space check) at the registry's version;
//! * [`LoginSession`]: the provider login in a hidden pty (Claude, Codex) or
//!   through ACP `authenticate` (Antigravity), streaming [`LoginEvent`]s;
//! * [`disconnect()`]: logout + safe profile removal;
//! * [`CancelToken`]: real cancellation of the downloads and installs of
//!   "Preparando…" and of the updates (`docs/specs/07-etapa5-productividad.md`
//!   §10.3), which leaves no partial file behind.
//!
//! [`Connections`] ties them together for the UI (see
//! `docs/specs/modulos/connections.md` for the protocol).

#![deny(missing_docs)]

mod adapters;
mod cancel;
mod disconnect;
mod envutil;
mod error;
mod identity;
mod login;
mod paths;
mod profile;
mod runtime;
mod scanner;
mod store;

use std::time::Duration;

pub use adapters::{
    AdapterInstall, AdapterProgress, Adapters, BinaryPlan, InstallKind, NpmInstaller,
    PackageInstaller, binary_plan, download_size_hint, free_space, install_space_needed,
    registry_version,
};
pub use cancel::CancelToken;
pub use disconnect::{DisconnectReport, LogoutStep, disconnect};
pub use error::{ConnectionsError, Result, human_size};
pub use identity::{
    AcpIdentityProbe, CliStatusProbe, IdentityProbe, ProbeOutcome, ProfileProbe,
    parse_claude_status, parse_codex_status,
};
pub use login::{
    AcpLogin, LOGIN_TIMEOUT, LoginCommand, LoginEvent, LoginFailure, LoginOptions, LoginPlan,
    LoginSession, PendingCleanup, default_probe, login_event_log_line, plan_login,
};
pub use paths::{CincelPaths, remove_dir_within};
pub use profile::{
    AdapterSource, AgentKind, ConnectionStatus, LoginMethod, Profile, RETIRED_AGENTS,
    STRIPPED_VARIABLES, retired_reason,
};
pub use runtime::{
    Downloader, HttpDownloader, MIN_NODE_MAJOR, NODE_DIST_URL, NodePaths, NodeVersion, RangeBody,
    Runtime, RuntimeProgress, keep_runtime_entry, node_platform, pick_lts, pick_major, sha256_file,
};
pub use scanner::{
    OutputScanner, find_code, find_login_urls, is_login_url, redact_line, strip_ansi,
};
pub use store::{
    Connection, ConnectionStore, Identity, PendingConnection, now_secs, suggest_label,
};

use cincel_acp::{AgentRegistry, LaunchSpec, ProcessEnv};
use uuid::Uuid;

/// How to launch the agent of a connection: pass `launch` in
/// `AgentCommand::Spawn` and `env` to
/// [`cincel_acp::AgentConnection::start_with_env`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLaunch {
    /// Private `node` + adapter script + registry args, or the unpacked
    /// binary + registry args.
    pub launch: LaunchSpec,
    /// Profile variable, stripped credentials, private Node first in `PATH`
    /// (npm adapters), `BROWSER` neutralized (Antigravity; logins of every
    /// agent get it through [`Profile::login_env`]).
    pub env: ProcessEnv,
}

/// Progress of [`Connections::prepare`] (spec 06 §4 F2 step 2, F5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareProgress {
    /// Private Node runtime (never for binary agents).
    Runtime(RuntimeProgress),
    /// Adapter install (npm install, or the binary download).
    Adapter(AdapterProgress),
}

/// What [`Connections::prune_unused`] removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// Runtime directories (`node-v24.1.0`).
    pub runtimes: Vec<String>,
    /// Adapter versions, as `(agent_id, version)`.
    pub adapters: Vec<(String, String)>,
}

/// Facade over store, runtime and adapters.
#[derive(Debug)]
pub struct Connections {
    store: ConnectionStore,
    runtime: Runtime,
    adapters: Adapters,
}

impl Connections {
    /// Everything under `paths`, with the real network and npm.
    #[must_use]
    pub fn new(paths: CincelPaths) -> Self {
        Self {
            store: ConnectionStore::new(paths.clone()),
            runtime: Runtime::new(paths.clone()),
            adapters: Adapters::new(paths),
        }
    }

    /// Replace the runtime manager (tests, settings' `node_version`).
    #[must_use]
    pub fn with_runtime(mut self, runtime: Runtime) -> Self {
        self.runtime = runtime;
        self
    }

    /// Replace the adapters manager (tests).
    #[must_use]
    pub fn with_adapters(mut self, adapters: Adapters) -> Self {
        self.adapters = adapters;
        self
    }

    /// The connection index.
    #[must_use]
    pub fn store(&self) -> &ConnectionStore {
        &self.store
    }

    /// The private runtime.
    #[must_use]
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// The adapters.
    #[must_use]
    pub fn adapters(&self) -> &Adapters {
        &self.adapters
    }

    /// Status of one connection, computed from the profile and what is
    /// installed (never from the network).
    #[must_use]
    pub fn status(&self, connection: &Connection) -> ConnectionStatus {
        if let Some(reason) = retired_reason(&connection.agent_id) {
            return ConnectionStatus::Unavailable {
                reason: reason.to_string(),
            };
        }
        let Ok(profile) = self.store.profile(connection) else {
            return ConnectionStatus::Unavailable {
                reason: format!("agente desconocido «{}»", connection.agent_id),
            };
        };
        let kind = profile.kind();
        if kind.needs_node() && self.runtime.installed().is_none() {
            return ConnectionStatus::Unavailable {
                reason: "Falta el entorno de Node de Cincel.".to_string(),
            };
        }
        if self.adapters.installed(&connection.agent_id).is_none() {
            return ConnectionStatus::Unavailable {
                reason: if kind.needs_node() {
                    format!("Falta el adaptador de {}.", kind.display_name())
                } else {
                    format!("Falta el agente de {}.", kind.full_name())
                },
            };
        }
        if !profile.dir().is_dir() {
            return ConnectionStatus::Unavailable {
                reason: "Falta la carpeta del perfil.".to_string(),
            };
        }
        if profile.has_credentials() {
            ConnectionStatus::Connected
        } else {
            ConnectionStatus::SessionExpired
        }
    }

    /// Whether everything needed to run `kind` is installed (the agent
    /// grid's "instalado").
    #[must_use]
    pub fn is_ready(&self, kind: AgentKind) -> bool {
        (!kind.needs_node() || self.runtime.installed().is_some())
            && self.adapters.installed(kind.agent_id()).is_some()
    }

    /// Every connection with its status, most recently used first.
    ///
    /// # Errors
    ///
    /// Index read errors.
    pub fn list_with_status(&self) -> Result<Vec<(Connection, ConnectionStatus)>> {
        Ok(self
            .store
            .list()?
            .into_iter()
            .map(|connection| {
                let status = self.status(&connection);
                (connection, status)
            })
            .collect())
    }

    /// Download what is missing to run `kind`: the private runtime (npm
    /// adapters only), then the adapter at the registry's version (npm
    /// install, or the binary archive). With `registry = None` (offline)
    /// only what is installed is used. Never touches credentials
    /// ("Reparar"). Returns the runtime when the agent needs one.
    ///
    /// Cancelling `cancel` (the modal's "Cancelar") stops the download or
    /// `npm install` in progress and removes what it left half-done; the
    /// call then returns [`ConnectionsError::Cancelled`]. A second
    /// preparation of the same agent (or of another npm agent, for the
    /// shared runtime) waits for a running one to finish.
    ///
    /// # Errors
    ///
    /// Runtime or install errors, [`ConnectionsError::Cancelled`].
    pub fn prepare(
        &self,
        registry: Option<&AgentRegistry>,
        kind: AgentKind,
        progress: &mut dyn FnMut(PrepareProgress),
        cancel: &CancelToken,
    ) -> Result<(Option<NodePaths>, AdapterInstall)> {
        cancel.check()?;
        let node = if kind.needs_node() {
            Some(match (&registry, self.runtime.installed()) {
                (None, Some(node)) => node,
                (None, None) => return Err(ConnectionsError::RuntimeMissing),
                (Some(_), _) => self
                    .runtime
                    .ensure(&mut |step| progress(PrepareProgress::Runtime(step)), cancel)?,
            })
        } else {
            None
        };
        let adapter = self.adapters.ensure(
            registry,
            kind,
            node.as_ref(),
            &mut |step| {
                progress(PrepareProgress::Adapter(step));
            },
            cancel,
        )?;
        Ok((node, adapter))
    }

    /// Start-up cleanup (`docs/specs/07-etapa5-productividad.md` §10.4, D13):
    /// remove adapter versions that are neither `current` nor in `in_use`
    /// ([`Adapters::prune_unused`]) and, when `in_use` is empty (nothing
    /// runs yet), runtimes other than `current` ([`Runtime::prune_old`]).
    /// Call it once at start-up before launching any agent (and, with the
    /// live connections' versions, when a connection stops).
    ///
    /// # Errors
    ///
    /// I/O errors removing a directory.
    pub fn prune_unused(&self, in_use: &[(&str, &str)]) -> Result<PruneReport> {
        let adapters = self.adapters.prune_unused(in_use)?;
        let runtimes = if in_use.is_empty() {
            self.runtime.prune_old()?
        } else {
            Vec::new()
        };
        Ok(PruneReport { runtimes, adapters })
    }

    /// The private runtime when `kind` needs one.
    fn node_for(&self, kind: AgentKind) -> Result<Option<NodePaths>> {
        if !kind.needs_node() {
            return Ok(None);
        }
        self.runtime
            .installed()
            .map(Some)
            .ok_or(ConnectionsError::RuntimeMissing)
    }

    /// Launch data for a saved connection.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::RuntimeMissing`], [`ConnectionsError::AdapterMissing`].
    pub fn agent_launch(&self, connection: &Connection) -> Result<AgentLaunch> {
        let profile = self.store.profile(connection)?;
        let node = self.node_for(profile.kind())?;
        Ok(AgentLaunch {
            launch: self
                .adapters
                .launch_spec(&connection.agent_id, node.as_ref())?,
            env: profile.process_env(node.as_ref().map(|node| node.bin_dir.as_path())),
        })
    }

    /// Start the login of a new connection (F2): the pending profile is
    /// removed if the login is cancelled.
    ///
    /// # Errors
    ///
    /// Runtime or adapter missing.
    pub fn start_login(&self, pending: &PendingConnection) -> Result<LoginSession> {
        let node = self.node_for(pending.kind)?;
        LoginSession::start(
            &pending.profile,
            node.as_ref(),
            &self.adapters,
            Some(PendingCleanup {
                root: self.store.paths().connections_dir(),
                dir: pending.profile.dir().to_path_buf(),
            }),
        )
    }

    /// "Volver a conectar" (F4): re-run the login on the existing profile,
    /// keeping label and conversations. Cancelling never removes the
    /// profile. For Antigravity it is the same ACP `authenticate` login.
    ///
    /// # Errors
    ///
    /// Unknown connection, runtime or adapter missing.
    pub fn start_relogin(&self, id: Uuid) -> Result<LoginSession> {
        let connection = self.store.get(id)?;
        let profile = self.store.profile(&connection)?;
        let node = self.node_for(profile.kind())?;
        LoginSession::start(&profile, node.as_ref(), &self.adapters, None)
    }

    /// Delete a connection (F3): ACP `logout` with its profile (when the
    /// agent announces it: Claude, Codex, Antigravity), then the profile and
    /// the index entry. Conversations are the chat's business. A connection
    /// of a retired agent (Gemini) is deleted without logout.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::UnknownConnection`].
    pub fn disconnect(&self, id: Uuid) -> Result<DisconnectReport> {
        let connection = self.store.get(id)?;
        let launch = connection.kind().ok().and_then(|kind| {
            let node = self.node_for(kind).ok()?;
            let launch = self
                .adapters
                .launch_spec(&connection.agent_id, node.as_ref())
                .ok()?;
            Some((launch, node.map(|node| node.bin_dir)))
        });
        disconnect(
            &self.store,
            id,
            launch
                .as_ref()
                .map(|(launch, bin)| (launch.clone(), bin.as_deref())),
            Duration::from_secs(30),
        )
    }
}
