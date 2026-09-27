//! Delete a connection safely (`docs/specs/06-etapa4-conexiones-y-cincel.md`
//! §4 F3, `docs/research/06-acp-estado-y-cuentas.md` §4): launch the adapter
//! with **that** connection's profile → `initialize` → ACP `logout` if
//! announced → kill the process group → remove `connections/<id>/` (only if
//! inside the connections root) → drop it from the index.
//!
//! The logout always runs with the profile variable set (it is built here
//! from the profile, never taken from the caller): a logout without it would
//! close the user's global session.

use std::path::Path;
use std::time::Duration;

use cincel_acp::{AgentCommand, AgentConnection, AgentEvent, LaunchSpec};
use uuid::Uuid;

use crate::error::Result;
use crate::store::ConnectionStore;

/// Outcome of the ACP logout step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogoutStep {
    /// The agent confirmed the logout.
    LoggedOut,
    /// The agent does not announce `auth.logout`, or is no longer supported
    /// (a retired Gemini connection): direct delete.
    NotSupported,
    /// Not attempted (adapter or runtime missing).
    Skipped(String),
    /// Attempted and failed (timeout, agent error, spawn failure).
    Failed(String),
}

/// Which steps of [`disconnect`] succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectReport {
    /// The logout step.
    pub logout: LogoutStep,
    /// The agent process (and its group) was stopped, or never started.
    pub process_stopped: bool,
    /// The profile directory is gone.
    pub profile_removed: bool,
    /// The connection is no longer in the index.
    pub index_removed: bool,
    /// Error removing the profile, when it failed (the index entry is then
    /// kept so the credentials are not orphaned).
    pub profile_error: Option<String>,
}

impl DisconnectReport {
    /// Whether the connection is fully forgotten locally.
    #[must_use]
    pub fn forgotten(&self) -> bool {
        self.profile_removed && self.index_removed
    }
}

/// Delete connection `id`. `adapter` is the adapter launch plus the private
/// runtime's `bin/` when the adapter needs Node (from
/// [`crate::Adapters::launch_spec`] and [`crate::NodePaths::bin_dir`]);
/// `None` skips the logout (adapter or runtime missing) and deletes
/// directly. A connection of an agent Cincel no longer supports is deleted
/// without launching anything.
///
/// # Errors
///
/// [`crate::ConnectionsError::UnknownConnection`]; later failures are
/// reported in [`DisconnectReport`].
pub fn disconnect(
    store: &ConnectionStore,
    id: Uuid,
    adapter: Option<(LaunchSpec, Option<&Path>)>,
    timeout: Duration,
) -> Result<DisconnectReport> {
    let connection = store.get(id)?;
    let Ok(profile) = store.profile(&connection) else {
        // Retired agent (Gemini): nothing can log it out any more.
        return Ok(forget(store, id, LogoutStep::NotSupported));
    };

    let (logout, process_stopped) = match adapter {
        None => (
            LogoutStep::Skipped("falta el adaptador o el entorno de Node".to_string()),
            true,
        ),
        Some(_) if !profile.dir().is_dir() => (
            LogoutStep::Skipped("la carpeta del perfil ya no existe".to_string()),
            true,
        ),
        Some((launch, node_bin)) => {
            let env = profile.process_env(node_bin);
            logout_with(&launch, env, profile.dir(), timeout)
        }
    };
    let mut report = forget(store, id, logout);
    report.process_stopped = process_stopped;
    Ok(report)
}

/// Remove the profile directory and, only if that worked, the index entry.
fn forget(store: &ConnectionStore, id: Uuid, logout: LogoutStep) -> DisconnectReport {
    let (profile_removed, profile_error) = match store.remove_profile_dir(id) {
        Ok(_) => (true, None),
        Err(error) => (false, Some(error.to_string())),
    };
    let index_removed = profile_removed && store.remove_from_index(id).is_ok();
    DisconnectReport {
        logout,
        process_stopped: true,
        profile_removed,
        index_removed,
        profile_error,
    }
}

fn logout_with(
    launch: &LaunchSpec,
    env: cincel_acp::ProcessEnv,
    profile_dir: &Path,
    timeout: Duration,
) -> (LogoutStep, bool) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    else {
        return (
            LogoutStep::Failed("no se pudo crear el runtime".to_string()),
            true,
        );
    };
    let mut connection = AgentConnection::start_with_env(profile_dir.to_path_buf(), env);
    let step = runtime.block_on(async {
        if connection
            .send(AgentCommand::Spawn {
                launch: launch.clone(),
                cwd: profile_dir.to_path_buf(),
            })
            .await
            .is_err()
        {
            return LogoutStep::Failed("la conexión con el agente se cerró".to_string());
        }
        let run = async {
            loop {
                match connection.recv().await {
                    Ok(AgentEvent::Connected { capabilities, .. }) => {
                        if !cincel_acp::agent_supports_logout(&capabilities) {
                            return LogoutStep::NotSupported;
                        }
                        if connection.send(AgentCommand::Logout).await.is_err() {
                            return LogoutStep::Failed("la conexión se cerró".to_string());
                        }
                    }
                    Ok(AgentEvent::LoggedOut { ok: true }) => return LogoutStep::LoggedOut,
                    Ok(AgentEvent::LoggedOut { ok: false }) => {
                        return LogoutStep::Failed(
                            "el agente rechazó el cierre de sesión".to_string(),
                        );
                    }
                    Ok(AgentEvent::Exited { code, .. }) => {
                        return LogoutStep::Failed(format!(
                            "el agente terminó antes de cerrar la sesión ({code:?})"
                        ));
                    }
                    Ok(AgentEvent::Error { message, .. }) => {
                        return LogoutStep::Failed(message);
                    }
                    Ok(_) => {}
                    Err(_) => return LogoutStep::Failed("la conexión se cerró".to_string()),
                }
            }
        };
        tokio::time::timeout(timeout, run)
            .await
            .unwrap_or_else(|_| LogoutStep::Failed("tiempo de espera agotado".to_string()))
    });
    // Shutdown kills the agent's whole process group.
    connection.shutdown();
    (step, true)
}
