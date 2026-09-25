//! Channel vocabulary between the UI thread and the ACP worker thread
//! (`docs/specs/03-arquitectura.md` §3).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, AuthMethod, AvailableCommand, ContentBlock, CreateElicitationRequest,
    CreateElicitationResponse, EnvVariable, Implementation, McpServer, McpServerStdio, Meta,
    PermissionOption, PermissionOptionId, ResourceLink, SessionConfigId, SessionConfigOption,
    SessionConfigOptionValue, SessionId, SessionModeId, SessionModeState, SessionUpdate,
    StopReason, TextContent, ToolCallId, ToolCallUpdate,
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
/// oneshot exactly once. Each slot also remembers the owning session so
/// [`AgentCommand::Cancel`] can answer every pending request of that session
/// with `cancelled` (`docs/specs/modulos/acp.md` §Cancelación).
pub(crate) type PermissionSlots =
    Arc<Mutex<HashMap<PermissionRequestId, (SessionId, oneshot::Sender<PermissionOutcome>)>>>;

/// Same shape as [`PermissionSlots`] for `elicitation/create` requests. The
/// session id is `None` for request-scoped elicitations (outside any
/// session, e.g. during auth).
pub(crate) type ElicitationSlots = Arc<
    Mutex<
        HashMap<
            PermissionRequestId,
            (
                Option<SessionId>,
                oneshot::Sender<CreateElicitationResponse>,
            ),
        >,
    >,
>;

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
            .map(|mut slots| slots.remove(&self.id).map(|(_, sender)| sender))
            .unwrap_or(None);
        match sender {
            Some(sender) => sender.send(outcome).is_ok(),
            None => false,
        }
    }
}

/// Handle the UI uses to answer a [`AgentEvent::Elicitation`]. Mirrors
/// [`PermissionResponder`].
#[derive(Debug, Clone)]
pub struct ElicitationResponder {
    id: PermissionRequestId,
    slots: ElicitationSlots,
}

impl ElicitationResponder {
    pub(crate) fn new(id: PermissionRequestId, slots: ElicitationSlots) -> Self {
        Self { id, slots }
    }

    /// Identifier of the request being answered.
    #[must_use]
    pub fn id(&self) -> PermissionRequestId {
        self.id
    }

    /// Answer the request. Returns `false` if it was already answered.
    pub fn respond(&self, response: CreateElicitationResponse) -> bool {
        let sender = self
            .slots
            .lock()
            .map(|mut slots| slots.remove(&self.id).map(|(_, sender)| sender))
            .unwrap_or(None);
        match sender {
            Some(sender) => sender.send(response).is_ok(),
            None => false,
        }
    }
}

/// One MCP server Asteroid should pass to `session/new` / `session/load` /
/// `session/resume`. Stdio only: E2 has no UI for HTTP/SSE servers yet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpServerSpec {
    /// Human-readable name for the server.
    pub name: String,
    /// Absolute path to the server executable.
    pub command: PathBuf,
    /// Arguments for `command`.
    pub args: Vec<String>,
    /// Environment variables for the server process.
    pub env: BTreeMap<String, String>,
}

impl McpServerSpec {
    /// Build a spec with no arguments or environment.
    #[must_use]
    pub fn new(name: impl Into<String>, command: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }
    }

    /// Map onto the wire `McpServer::Stdio` variant.
    #[must_use]
    pub fn into_wire(self) -> McpServer {
        let env = self
            .env
            .into_iter()
            .map(|(name, value)| EnvVariable::new(name, value))
            .collect();
        McpServer::Stdio(
            McpServerStdio::new(self.name, self.command)
                .args(self.args)
                .env(env),
        )
    }
}

/// One block of a [`AgentCommand::Prompt`].
///
/// A baseline ACP agent supports `text` and `resource_link` (the two variants
/// modeled here); richer blocks are Etapa-3+ work
/// (`docs/specs/modulos/acp.md` §Prompt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptBlock {
    /// Plain text typed by the user.
    Text(String),
    /// A `@archivo` mention, sent as `ContentBlock::ResourceLink`.
    ResourceLink {
        /// `file://` URI of the mentioned resource. Must be an absolute path.
        uri: String,
        /// Display name, usually the file name.
        name: String,
        /// MIME type, when known.
        mime_type: Option<String>,
    },
}

impl PromptBlock {
    /// Build a `file://` resource link block for an absolute path.
    #[must_use]
    pub fn mention(path: &std::path::Path) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        PromptBlock::ResourceLink {
            uri: format!("file://{}", path.display()),
            name,
            mime_type: None,
        }
    }

    fn into_wire(self) -> ContentBlock {
        match self {
            PromptBlock::Text(text) => ContentBlock::Text(TextContent::new(text)),
            PromptBlock::ResourceLink {
                uri,
                name,
                mime_type,
            } => ContentBlock::ResourceLink(ResourceLink::new(name, uri).mime_type(mime_type)),
        }
    }
}

/// Text wrapper the review panel's feedback is sent back to the agent in
/// (`docs/specs/modulos/acp.md` §Prompt): `ReviewStore::report_for_agent`,
/// when it returns text, is prepended as its own block.
#[must_use]
pub fn wrap_review_feedback(feedback: &str) -> String {
    format!("<user_review_feedback>\n{feedback}\n</user_review_feedback>")
}

/// Map a [`AgentCommand::Prompt`]'s blocks (plus optional review feedback,
/// wrapped with [`wrap_review_feedback`]) onto wire `ContentBlock`s.
#[must_use]
pub fn build_prompt_content(
    blocks: Vec<PromptBlock>,
    feedback: Option<String>,
) -> Vec<ContentBlock> {
    let mut content = Vec::with_capacity(blocks.len() + 1);
    if let Some(feedback) = feedback {
        content.push(ContentBlock::Text(TextContent::new(wrap_review_feedback(
            &feedback,
        ))));
    }
    content.extend(blocks.into_iter().map(PromptBlock::into_wire));
    content
}

/// Client-friendly view of an [`AuthMethod`], with a ready-to-copy shell
/// string for terminal-based methods (`docs/specs/modulos/acp.md` §Auth).
///
/// The wire schema only distinguishes `Terminal` (client runs the agent
/// interactively) from agent-handled methods; it has no separate `Url`
/// discriminator; agent-handled methods that happen to be URL/OAuth flows are
/// exposed as [`AuthMethodKind::Other`] (see "Desviaciones" in
/// `docs/specs/modulos/acp.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthMethodView {
    /// Method id, passed back verbatim to [`AgentCommand::Authenticate`].
    pub id: String,
    /// Human readable name.
    pub name: String,
    /// Optional longer description.
    pub description: Option<String>,
    /// How the client should drive this method.
    pub kind: AuthMethodKind,
}

/// How the client should drive one [`AuthMethodView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethodKind {
    /// The client should run this command interactively in a system
    /// terminal; the client must not call `authenticate` for it.
    Terminal {
        /// Program to run.
        command: String,
        /// Arguments for `command`.
        args: Vec<String>,
    },
    /// The agent handles this method itself through `authenticate`.
    Other,
}

impl AuthMethodView {
    /// Describe `method`, resolving `Terminal`'s shell command against the
    /// agent's own [`LaunchSpec`].
    #[must_use]
    pub fn describe(method: &AuthMethod, launch: &LaunchSpec) -> Self {
        match method {
            AuthMethod::Terminal(terminal) => {
                let mut args = launch.args.clone();
                args.extend(terminal.args.iter().cloned());
                AuthMethodView {
                    id: terminal.id.0.to_string(),
                    name: terminal.name.clone(),
                    description: terminal.description.clone(),
                    kind: AuthMethodKind::Terminal {
                        command: launch.program.clone(),
                        args,
                    },
                }
            }
            other => AuthMethodView {
                id: other.id().0.to_string(),
                name: other.name().to_string(),
                description: other.description().map(str::to_string),
                kind: AuthMethodKind::Other,
            },
        }
    }

    /// Ready-to-copy shell command for [`AuthMethodKind::Terminal`].
    #[must_use]
    pub fn shell_command(&self) -> Option<String> {
        match &self.kind {
            AuthMethodKind::Terminal { command, args } => {
                let mut out = command.clone();
                for arg in args {
                    out.push(' ');
                    out.push_str(arg);
                }
                Some(out)
            }
            AuthMethodKind::Other => None,
        }
    }
}

/// One `agentFileChangeReport` result for a prompt turn
/// (`docs/research/01-acp.md` §1; the JetBrains "air" `_meta` convention also
/// used by `claude-agent-acp` and `codex-acp`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileChangeReport {
    /// Absolute, normalized paths touched during the turn.
    pub paths: Vec<PathBuf>,
    /// Whether the agent believes the list is complete.
    pub declared_complete: bool,
    /// Whether the list was capped before reaching every path.
    pub truncated: bool,
    /// Free-form explanation of any gap, when `declared_complete` is `false`.
    pub uncertainty: Option<String>,
}

const AIR_NAMESPACE: &str = "jetbrains";

/// Build the `_meta` object that declares `agentFileChangeReport` support
/// during `initialize`. Both sides must advertise it under
/// `_meta.jetbrains.air.capabilities`.
#[must_use]
pub fn file_change_report_capability_meta() -> Meta {
    as_meta(serde_json::json!({
        AIR_NAMESPACE: { "air": { "capabilities": { "agentFileChangeReport": { "version": 1 } } } }
    }))
}

/// Build the `_meta` object requesting a report for one `session/prompt`
/// call, keyed by `request_id` (1-128 chars: letters, digits, `.`, `_`, `:`, `-`).
#[must_use]
pub fn file_change_report_request_meta(request_id: &str) -> Meta {
    as_meta(serde_json::json!({
        AIR_NAMESPACE: {
            "air": {
                "agentFileChangeReportRequest": { "version": 1, "requestId": request_id }
            }
        }
    }))
}

/// Whether `capabilities` (from `initialize`) declares `agentFileChangeReport`
/// support under `_meta.jetbrains.air.capabilities`.
#[must_use]
pub fn agent_supports_file_change_report(meta: Option<&Meta>) -> bool {
    meta.and_then(|meta| meta.get(AIR_NAMESPACE))
        .and_then(|jetbrains| jetbrains.get("air"))
        .and_then(|air| air.get("capabilities"))
        .and_then(|capabilities| capabilities.get("agentFileChangeReport"))
        .is_some()
}

/// Parse a `session_info_update`'s `_meta` for a matching
/// `agentFileChangeReport`. Returns `None` when absent, malformed, or for a
/// different `requestId` (stale/duplicate reports must be ignored per spec).
#[must_use]
pub fn parse_file_change_report(
    meta: Option<&Meta>,
    expected_request_id: &str,
) -> Option<FileChangeReport> {
    let report = meta?
        .get(AIR_NAMESPACE)?
        .get("air")?
        .get("agentFileChangeReport")?;
    if report.get("requestId").and_then(|value| value.as_str()) != Some(expected_request_id) {
        return None;
    }
    if report.get("status").and_then(|value| value.as_str()) != Some("reported") {
        return None;
    }
    let paths = report
        .get("paths")?
        .as_array()?
        .iter()
        .filter_map(|value| value.as_str())
        .map(PathBuf::from)
        .collect();
    Some(FileChangeReport {
        paths,
        declared_complete: report
            .get("declaredComplete")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        truncated: report
            .get("truncated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        uncertainty: report
            .get("uncertainty")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    })
}

fn as_meta(value: serde_json::Value) -> Meta {
    match value {
        serde_json::Value::Object(map) => map,
        _ => Meta::default(),
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
        /// MCP servers to connect to, if any.
        mcp_servers: Vec<McpServerSpec>,
    },
    /// Load a previous session (`session/load`); only valid when the agent
    /// announced `agentCapabilities.loadSession`. The worker replays the
    /// conversation as [`AgentEvent::Update`] before emitting
    /// [`AgentEvent::SessionCreated`].
    LoadSession {
        /// Session id to load.
        session_id: SessionId,
        /// Session working directory (absolute).
        cwd: PathBuf,
        /// MCP servers to connect to, if any.
        mcp_servers: Vec<McpServerSpec>,
    },
    /// Resume a previous session without replaying history
    /// (`session/resume`); only valid when the agent announced
    /// `agentCapabilities.sessionCapabilities.resume`.
    ResumeSession {
        /// Session id to resume.
        session_id: SessionId,
        /// Session working directory (absolute).
        cwd: PathBuf,
        /// MCP servers to connect to, if any.
        mcp_servers: Vec<McpServerSpec>,
    },
    /// Send a prompt turn.
    Prompt {
        /// Target session.
        session_id: SessionId,
        /// Prompt content blocks.
        blocks: Vec<PromptBlock>,
        /// Review feedback to prepend as a `<user_review_feedback>` block, if
        /// `ReviewStore::report_for_agent` returned any.
        feedback: Option<String>,
    },
    /// Cancel the running turn: sends `session/cancel`, answers `cancelled`
    /// to every pending permission request of this session, and — once the
    /// agent's `PromptResponse` comes back with `stopReason: cancelled` —
    /// emits [`AgentEvent::ToolCallsCancelled`] for tool calls still
    /// `pending`/`in_progress`.
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
    /// Answer a pending elicitation request by id (alternative to
    /// [`AgentEvent::Elicitation`]'s embedded `reply`, for UIs that need to
    /// hand the request across a render cycle).
    RespondElicitation {
        /// Request being answered.
        id: PermissionRequestId,
        /// Response sent back to the agent.
        response: CreateElicitationResponse,
    },
    /// Authenticate with one of the advertised methods. On success the UI
    /// may retry [`AgentCommand::NewSession`] (`docs/specs/modulos/acp.md`
    /// §Auth).
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
    /// `session/new`, `session/load` or `session/resume` failed with
    /// `auth_required`.
    AuthRequired {
        /// Methods the user can choose from.
        methods: Vec<AuthMethod>,
    },
    /// A session was created, loaded or resumed and is ready for prompts.
    SessionCreated {
        /// Session id.
        session_id: SessionId,
        /// Legacy mode state, if supported.
        modes: Option<SessionModeState>,
        /// Modern config options, if supported.
        config_options: Vec<SessionConfigOption>,
        /// Slash commands known at creation time.
        commands: Vec<AvailableCommand>,
    },
    /// A `session/update` notification, including the replay stream of a
    /// `session/load` (`docs/specs/modulos/acp.md` §Sesión).
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
    /// The agent asked the user for structured input (`elicitation/create`),
    /// form or url mode alike (`docs/specs/modulos/acp.md` §Elicitation).
    Elicitation {
        /// Request id (also embedded in `reply`).
        id: PermissionRequestId,
        /// Session the request belongs to, when session-scoped.
        session_id: Option<SessionId>,
        /// Request as received from the agent.
        request: Box<CreateElicitationRequest>,
        /// Handle used to answer.
        reply: ElicitationResponder,
    },
    /// The prompt turn finished.
    TurnEnded {
        /// Session the turn belonged to.
        session_id: SessionId,
        /// Why the turn stopped.
        stop_reason: StopReason,
    },
    /// Tool calls that were still `pending`/`in_progress` when the turn
    /// ended with `stopReason: cancelled` (`docs/specs/modulos/acp.md`
    /// §Cancelación). Sent right before [`AgentEvent::TurnEnded`].
    ToolCallsCancelled {
        /// Session the tool calls belong to.
        session_id: SessionId,
        /// Ids of the cancelled tool calls.
        ids: Vec<ToolCallId>,
    },
    /// The agent's `agentFileChangeReport` for the turn that just ended,
    /// carried on a `session_info_update` (`docs/research/01-acp.md` §1).
    FileChangeReport {
        /// Session the report belongs to.
        session_id: SessionId,
        /// Parsed report.
        report: FileChangeReport,
    },
    /// One line from the agent's stderr.
    Stderr(String),
    /// The agent process exited.
    Exited {
        /// Exit code, when available.
        code: Option<i32>,
        /// Last 64 KiB of the agent's stderr, for diagnostics.
        stderr_tail: String,
    },
    /// Something went wrong; the connection may still be usable.
    Error {
        /// Human readable description.
        message: String,
        /// Last 64 KiB of the agent's stderr, when a process was running.
        stderr_tail: String,
    },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_content_prepends_wrapped_feedback() {
        let blocks = vec![PromptBlock::Text("hola".to_string())];
        let content = build_prompt_content(blocks, Some("cambiá esto".to_string()));
        assert_eq!(content.len(), 2);
        match &content[0] {
            ContentBlock::Text(text) => {
                assert_eq!(
                    text.text,
                    "<user_review_feedback>\ncambiá esto\n</user_review_feedback>"
                );
            }
            other => panic!("se esperaba texto: {other:?}"),
        }
        match &content[1] {
            ContentBlock::Text(text) => assert_eq!(text.text, "hola"),
            other => panic!("se esperaba texto: {other:?}"),
        }
    }

    #[test]
    fn build_prompt_content_without_feedback_only_has_blocks() {
        let blocks = vec![PromptBlock::Text("hola".to_string())];
        let content = build_prompt_content(blocks, None);
        assert_eq!(content.len(), 1);
    }

    #[test]
    fn resource_link_mention_uses_file_uri_and_basename() {
        let block = PromptBlock::mention(std::path::Path::new("/proj/src/main.rs"));
        match block.into_wire() {
            ContentBlock::ResourceLink(link) => {
                assert_eq!(link.uri, "file:///proj/src/main.rs");
                assert_eq!(link.name, "main.rs");
            }
            other => panic!("se esperaba resource_link: {other:?}"),
        }
    }

    #[test]
    fn mcp_server_spec_maps_to_stdio_variant() {
        let mut env = BTreeMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let spec = McpServerSpec {
            name: "fs".to_string(),
            command: PathBuf::from("/usr/bin/mcp-fs"),
            args: vec!["--stdio".to_string()],
            env,
        };
        match spec.into_wire() {
            McpServer::Stdio(stdio) => {
                assert_eq!(stdio.name, "fs");
                assert_eq!(stdio.command, PathBuf::from("/usr/bin/mcp-fs"));
                assert_eq!(stdio.args, vec!["--stdio"]);
                assert_eq!(stdio.env.len(), 1);
            }
            other => panic!("se esperaba stdio: {other:?}"),
        }
    }

    #[test]
    fn file_change_report_roundtrips_through_meta() {
        let request_meta = file_change_report_request_meta("req-1");
        assert!(agent_supports_file_change_report(Some(
            &file_change_report_capability_meta()
        )));

        // The request meta alone (no "capabilities" key) does not count as a
        // capability announcement.
        assert!(!agent_supports_file_change_report(Some(&request_meta)));

        let response_meta = as_meta(serde_json::json!({
            "jetbrains": {
                "air": {
                    "agentFileChangeReport": {
                        "version": 1,
                        "requestId": "req-1",
                        "status": "reported",
                        "paths": ["/workspace/src/App.ts"],
                        "declaredComplete": false,
                        "truncated": false,
                        "uncertainty": "puede omitir renombrados"
                    }
                }
            }
        }));
        let report = parse_file_change_report(Some(&response_meta), "req-1").expect("report");
        assert_eq!(report.paths, vec![PathBuf::from("/workspace/src/App.ts")]);
        assert!(!report.declared_complete);
        assert_eq!(
            report.uncertainty.as_deref(),
            Some("puede omitir renombrados")
        );
    }

    #[test]
    fn file_change_report_ignores_mismatched_request_id() {
        let response_meta = as_meta(serde_json::json!({
            "jetbrains": {
                "air": {
                    "agentFileChangeReport": {
                        "requestId": "otro-turno",
                        "status": "reported",
                        "paths": []
                    }
                }
            }
        }));
        assert!(parse_file_change_report(Some(&response_meta), "req-1").is_none());
    }

    #[test]
    fn auth_method_view_builds_shell_command_for_terminal() {
        use agent_client_protocol::schema::v1::AuthMethodTerminal;
        let launch = LaunchSpec::new(
            "npx",
            vec!["-y".to_string(), "claude-agent-acp".to_string()],
        );
        let method = AuthMethod::Terminal(
            AuthMethodTerminal::new("claude-login", "Log in with Claude")
                .args(vec!["--cli".to_string()]),
        );
        let view = AuthMethodView::describe(&method, &launch);
        assert_eq!(
            view.shell_command().as_deref(),
            Some("npx -y claude-agent-acp --cli")
        );
    }
}
