//! Channel vocabulary between the UI thread and the ACP worker thread
//! (`docs/specs/03-arquitectura.md` §3).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, AuthMethod, AvailableCommand, ContentBlock, Implementation,
    PermissionOption, PermissionOptionId, SessionConfigId, SessionConfigOption,
    SessionConfigOptionValue, SessionId, SessionModeId, SessionModeState, SessionUpdate,
    StopReason, ToolCallUpdate,
};
use tokio::sync::oneshot;

use crate::registry::LaunchSpec;

/// Identifies a permission request across the channel boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PermissionRequestId(pub u64);

impl std::fmt::Display for PermissionRequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "perm-{}", self.0)
    }
}

/// What the UI decided about a permission request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionOutcome {
    /// The user (or the autonomy policy) chose one of the offered options.
    Selected(PermissionOptionId),
    /// The turn was cancelled; the agent must be told so.
    Cancelled,
}

/// Shared slot table backing [`PermissionResponder`] and
/// [`AgentCommand::RespondPermission`], so both paths hand back the same
/// oneshot exactly once.
pub(crate) type PermissionSlots =
    Arc<Mutex<HashMap<PermissionRequestId, oneshot::Sender<PermissionOutcome>>>>;

/// Handle the UI uses to answer a [`AgentEvent::PermissionRequest`].
///
/// Dropping it without answering leaves the request pending until the turn is
/// cancelled, exactly like a UI that never shows the dialog.
#[derive(Debug, Clone)]
pub struct PermissionResponder {
    id: PermissionRequestId,
    slots: PermissionSlots,
}

impl PermissionResponder {
    pub(crate) fn new(id: PermissionRequestId, slots: PermissionSlots) -> Self {
        Self { id, slots }
    }

    /// Identifier of the request being answered.
    #[must_use]
    pub fn id(&self) -> PermissionRequestId {
        self.id
    }

    /// Answer the request. Returns `false` if it was already answered.
    pub fn respond(&self, outcome: PermissionOutcome) -> bool {
        let sender = self
            .slots
            .lock()
            .map(|mut slots| slots.remove(&self.id))
            .unwrap_or(None);
        match sender {
            Some(sender) => sender.send(outcome).is_ok(),
            None => false,
        }
    }
}

/// Commands the UI sends to the ACP worker.
#[derive(Debug)]
#[non_exhaustive]
pub enum AgentCommand {
    /// Launch the agent process and negotiate `initialize`.
    Spawn {
        /// Command to run.
        launch: LaunchSpec,
        /// Working directory for the child process.
        cwd: PathBuf,
    },
    /// Create a session rooted at `cwd`.
    NewSession {
        /// Session working directory (absolute).
        cwd: PathBuf,
    },
    /// Send a prompt turn.
    Prompt {
        /// Target session.
        session_id: SessionId,
        /// Prompt content blocks.
        blocks: Vec<ContentBlock>,
    },
    /// Cancel the running turn.
    Cancel {
        /// Target session.
        session_id: SessionId,
    },
    /// Switch the session mode (legacy `session/set_mode`).
    SetMode {
        /// Target session.
        session_id: SessionId,
        /// Mode to activate.
        mode_id: SessionModeId,
    },
    /// Change a session config option (`session/set_config_option`).
    SetConfigOption {
        /// Target session.
        session_id: SessionId,
        /// Option id.
        config_id: SessionConfigId,
        /// New value.
        value: SessionConfigOptionValue,
    },
    /// Answer a pending permission request by id.
    RespondPermission {
        /// Request being answered.
        id: PermissionRequestId,
        /// Chosen outcome.
        outcome: PermissionOutcome,
    },
    /// Authenticate with one of the advertised methods.
    Authenticate {
        /// Method id from `initialize`.
        method_id: String,
    },
    /// Tear the agent down and stop the worker.
    Shutdown,
}

/// Events the ACP worker sends to the UI.
#[derive(Debug)]
#[non_exhaustive]
pub enum AgentEvent {
    /// `initialize` succeeded.
    Connected {
        /// Agent name/version, when advertised.
        agent_info: Option<Implementation>,
        /// Authentication methods the agent offers.
        auth_methods: Vec<AuthMethod>,
        /// Capabilities the agent advertises.
        capabilities: Box<AgentCapabilities>,
    },
    /// `session/new` failed with `auth_required`.
    AuthRequired {
        /// Methods the user can choose from.
        methods: Vec<AuthMethod>,
    },
    /// A session was created.
    SessionCreated {
        /// New session id.
        session_id: SessionId,
        /// Legacy mode state, if supported.
        modes: Option<SessionModeState>,
        /// Modern config options, if supported.
        config_options: Vec<SessionConfigOption>,
        /// Slash commands known at creation time.
        commands: Vec<AvailableCommand>,
    },
    /// A `session/update` notification.
    Update {
        /// Session the update belongs to.
        session_id: SessionId,
        /// The update payload.
        update: Box<SessionUpdate>,
    },
    /// The agent asked for permission to run a tool call.
    PermissionRequest {
        /// Request id (also embedded in `reply`).
        id: PermissionRequestId,
        /// Session the request belongs to.
        session_id: SessionId,
        /// The tool call about to run.
        tool_call: Box<ToolCallUpdate>,
        /// Options offered by the agent.
        options: Vec<PermissionOption>,
        /// Handle used to answer.
        reply: PermissionResponder,
    },
    /// The agent asked the client to read a text file.
    FsRead {
        /// Session the request belongs to.
        session_id: SessionId,
        /// Absolute path.
        path: PathBuf,
        /// 1-based first line to return.
        line: Option<u32>,
        /// Maximum number of lines to return.
        limit: Option<u32>,
        /// Reply channel; `Err` becomes a JSON-RPC error.
        reply: oneshot::Sender<Result<String, FsError>>,
    },
    /// The agent asked the client to write a text file.
    FsWrite {
        /// Session the request belongs to.
        session_id: SessionId,
        /// Absolute path.
        path: PathBuf,
        /// Full new content.
        content: String,
        /// Reply channel; `Err` becomes a JSON-RPC error.
        reply: oneshot::Sender<Result<(), FsError>>,
    },
    /// The prompt turn finished.
    TurnEnded {
        /// Session the turn belonged to.
        session_id: SessionId,
        /// Why the turn stopped.
        stop_reason: StopReason,
    },
    /// One line from the agent's stderr.
    Stderr(String),
    /// The agent process exited.
    Exited {
        /// Exit code, when available.
        code: Option<i32>,
    },
    /// Something went wrong; the connection may still be usable.
    Error(String),
}

/// Failure answering an `fs/*` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    /// The path is outside the project or the arguments are out of range.
    InvalidParams(String),
    /// The file does not exist.
    NotFound(String),
    /// Anything else.
    Internal(String),
}

impl FsError {
    /// Convert into a JSON-RPC error for the agent.
    #[must_use]
    pub fn into_protocol(self) -> agent_client_protocol::Error {
        match self {
            FsError::InvalidParams(message) => {
                agent_client_protocol::Error::invalid_params().data(serde_json::json!(message))
            }
            FsError::NotFound(message) => {
                agent_client_protocol::Error::resource_not_found(Some(message))
            }
            FsError::Internal(message) => {
                agent_client_protocol::Error::internal_error().data(serde_json::json!(message))
            }
        }
    }
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::InvalidParams(message)
            | FsError::NotFound(message)
            | FsError::Internal(message) => f.write_str(message),
        }
    }
}
