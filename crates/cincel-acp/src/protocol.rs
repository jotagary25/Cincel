//! Channel vocabulary between the UI thread and the ACP worker thread
//! (`docs/specs/03-arquitectura.md` §3).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, AuthMethod, AvailableCommand, ContentBlock, CreateElicitationRequest,
    CreateElicitationResponse, EnvVariable, ImageContent, Implementation, McpServer,
    McpServerStdio, Meta, PermissionOption, PermissionOptionId, ResourceLink, SessionConfigId,
    SessionConfigOption, SessionConfigOptionValue, SessionId, SessionModeId, SessionModeState,
    SessionUpdate, StopReason, TextContent, ToolCallId, ToolCallUpdate,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use tokio::sync::oneshot;

use crate::error::AcpError;
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
    /// The user (or the permission policy) chose one of the offered options.
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

/// One MCP server Cincel should pass to `session/new` / `session/load` /
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
/// A baseline ACP agent supports `text` and `resource_link`; `image` is only
/// legal when the agent announced `promptCapabilities.image`
/// ([`agent_supports_images`]; `docs/specs/09-etapa7-conexiones-imagenes-comentarios.md`
/// §5.3.3).
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
    /// An attached image, sent as `ContentBlock::Image` with base64 `data`
    /// and **no `uri`** (`codex-acp` would prefer an `http(s):`/`data:` `uri`
    /// over the data). Build it with [`PromptBlock::image`] to validate the
    /// MIME type; the worker validates every image block again before sending.
    Image {
        /// One of [`IMAGE_MIME_TYPES`].
        mime_type: String,
        /// Raw (not base64) image bytes; encoded in `into_wire`.
        data: Arc<[u8]>,
    },
}

/// MIME types Cincel sends as image blocks (the formats the chat accepts).
pub const IMAGE_MIME_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

impl PromptBlock {
    /// Build an image block, rejecting any MIME type outside
    /// [`IMAGE_MIME_TYPES`] (compared ignoring ASCII case, stored lowercase)
    /// and empty data.
    ///
    /// # Errors
    ///
    /// [`AcpError::InvalidParams`] when the MIME type is not a supported
    /// image type or `data` is empty.
    pub fn image(mime_type: &str, data: impl Into<Arc<[u8]>>) -> Result<Self, AcpError> {
        let mime_type = normalize_image_mime(mime_type)?;
        let data = data.into();
        if data.is_empty() {
            return Err(AcpError::InvalidParams(
                "la imagen no tiene datos".to_string(),
            ));
        }
        Ok(PromptBlock::Image { mime_type, data })
    }

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
            PromptBlock::Image { mime_type, data } => {
                ContentBlock::Image(ImageContent::new(BASE64_STANDARD.encode(&data), mime_type))
            }
        }
    }
}

/// Lowercase `mime_type` when it is one of [`IMAGE_MIME_TYPES`].
fn normalize_image_mime(mime_type: &str) -> Result<String, AcpError> {
    let lowered = mime_type.trim().to_ascii_lowercase();
    if IMAGE_MIME_TYPES.contains(&lowered.as_str()) {
        Ok(lowered)
    } else {
        Err(AcpError::InvalidParams(format!(
            "tipo de imagen no soportado: `{mime_type}`"
        )))
    }
}

/// Check the blocks of a prompt before they go on the wire: every
/// [`PromptBlock::Image`] needs a supported MIME type and non-empty data
/// (variants can be built directly, bypassing [`PromptBlock::image`]).
///
/// # Errors
///
/// [`AcpError::InvalidParams`] describing the first invalid image block.
pub fn validate_prompt_blocks(blocks: &[PromptBlock]) -> Result<(), AcpError> {
    for block in blocks {
        if let PromptBlock::Image { mime_type, data } = block {
            normalize_image_mime(mime_type)?;
            if data.is_empty() {
                return Err(AcpError::InvalidParams(
                    "la imagen no tiene datos".to_string(),
                ));
            }
        }
    }
    Ok(())
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

/// Version of the JetBrains "air" `_meta` extension Cincel speaks.
pub const AIR_EXTENSION_VERSION: u64 = 1;

/// Name of the `agentFileChangeReport` capability inside the air
/// `capabilities` array.
pub const AIR_FILE_CHANGE_REPORT_CAPABILITY: &str = "agentFileChangeReport";

/// The `jetbrains.air` object Cincel advertises during `initialize`:
/// `{"version": 1, "capabilities": ["agentFileChangeReport"]}`.
///
/// Verified against `claude-agent-acp` (`src/air-extension.ts`,
/// `clientSupportsAirCapability`) and `codex-acp` (`src/AirExtension.ts`):
/// both read it from `clientCapabilities._meta.jetbrains.air`, requiring an
/// integer `version >= 1` and a `capabilities` array of strings.
/// [`crate::client_capabilities`] embeds it there, and the worker also sends
/// it on the `InitializeRequest`'s own `_meta` (harmless, and what the spec
/// text describes).
#[must_use]
pub fn file_change_report_capability_meta() -> Meta {
    as_meta(serde_json::json!({
        AIR_NAMESPACE: {
            "air": {
                "version": AIR_EXTENSION_VERSION,
                "capabilities": [AIR_FILE_CHANGE_REPORT_CAPABILITY]
            }
        }
    }))
}

/// Build the `_meta` object requesting a report for one `session/prompt`
/// call, keyed by `request_id` (1-128 chars: letters, digits, `.`, `_`, `:`, `-`).
/// `claude-agent-acp` requires the request object to hold exactly `version`
/// and `requestId`.
#[must_use]
pub fn file_change_report_request_meta(request_id: &str) -> Meta {
    as_meta(serde_json::json!({
        AIR_NAMESPACE: {
            "air": {
                "version": AIR_EXTENSION_VERSION,
                "agentFileChangeReportRequest": { "version": 1, "requestId": request_id }
            }
        }
    }))
}

/// Capabilities listed in a `_meta.jetbrains.air` object, or `None` when
/// the object is absent, its `version` is not an integer `>= 1`, or
/// `capabilities` is not an array.
#[must_use]
pub fn air_capabilities(meta: Option<&Meta>) -> Option<Vec<String>> {
    let air = meta?.get(AIR_NAMESPACE)?.get("air")?;
    let version = air.get("version")?.as_u64()?;
    if version < AIR_EXTENSION_VERSION {
        return None;
    }
    let list = air.get("capabilities")?.as_array()?;
    Some(
        list.iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect(),
    )
}

/// Whether the top-level `_meta` of an `InitializeResponse` declares
/// `agentFileChangeReport` in its `jetbrains.air.capabilities` array (the
/// shape both `claude-agent-acp` and `codex-acp` send).
#[must_use]
pub fn agent_supports_file_change_report(meta: Option<&Meta>) -> bool {
    air_capabilities(meta).is_some_and(|capabilities| {
        capabilities
            .iter()
            .any(|capability| capability == AIR_FILE_CHANGE_REPORT_CAPABILITY)
    })
}

/// Whether the agent accepts `image` prompt blocks
/// (`agentCapabilities.promptCapabilities.image`). ACP forbids sending them
/// otherwise, so the chat checks this before attaching anything.
#[must_use]
pub fn agent_supports_images(capabilities: &AgentCapabilities) -> bool {
    capabilities.prompt_capabilities.image
}

/// Whether the agent announced ACP `logout` (`agentCapabilities.auth.logout`).
#[must_use]
pub fn agent_supports_logout(capabilities: &AgentCapabilities) -> bool {
    capabilities.auth.logout.is_some()
}

/// Whether the agent announced the `authStatus` extension
/// (`agentCapabilities._meta.authStatus`), i.e. that it pushes
/// `_auth/status_update`.
#[must_use]
pub fn agent_supports_auth_status(capabilities: &AgentCapabilities) -> bool {
    capabilities
        .meta
        .as_ref()
        .is_some_and(|meta| meta.contains_key("authStatus"))
}

/// Method name of the `authStatus` push extension.
pub const AUTH_STATUS_UPDATE_METHOD: &str = "_auth/status_update";

/// Category of credential an agent reports through `_auth/status_update`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStatusKind {
    /// A subscription account (Claude Pro/Max, ChatGPT...).
    Account,
    /// An API key.
    ApiKey,
    /// A model gateway.
    Gateway,
    /// External cloud credentials (Bedrock, Vertex...).
    External,
    /// Known logged-out state.
    None,
    /// A value this client does not know yet.
    Other(String),
}

impl AuthStatusKind {
    fn parse(text: &str) -> Self {
        match text {
            "account" => Self::Account,
            "api_key" => Self::ApiKey,
            "gateway" => Self::Gateway,
            "external" => Self::External,
            "none" => Self::None,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Account details carried by `_auth/status_update`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuthAccount {
    /// Account email.
    pub email: Option<String>,
    /// Organization name.
    pub organization: Option<String>,
    /// Vendor plan string, not normalized ("Claude Max", "plus"...).
    pub plan: Option<String>,
}

/// Parsed `authStatus` payload of `_auth/status_update`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthStatus {
    /// Credential category.
    pub kind: AuthStatusKind,
    /// Human readable label, usable on its own.
    pub label: String,
    /// Optional second line.
    pub detail: Option<String>,
    /// Account details, when the agent reports them.
    pub account: Option<AuthAccount>,
}

/// Parse the params of an `_auth/status_update` notification
/// (`{"authStatus": {kind, label, detail?, account?}}`). Returns `None` when
/// the payload is malformed.
#[must_use]
pub fn parse_auth_status_update(params: &serde_json::Value) -> Option<AuthStatus> {
    let status = params.get("authStatus")?;
    let kind = AuthStatusKind::parse(status.get("kind")?.as_str()?);
    let text = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    let account = status
        .get("account")
        .filter(|value| value.is_object())
        .map(|account| AuthAccount {
            email: text(account, "email"),
            organization: text(account, "organization"),
            plan: text(account, "plan"),
        });
    Some(AuthStatus {
        kind,
        label: text(status, "label").unwrap_or_default(),
        detail: text(status, "detail"),
        account,
    })
}

/// The reason an `auth_required` (-32000) JSON-RPC error gives, if any.
///
/// Precedence: a text field `message` or `reason` of an object `data`, or
/// `data` itself when it is a string (agents put the specific reason there
/// and keep the generic "Authentication required" in `message`); then the
/// error's `message`. Blank texts count as absent. The text is returned
/// trimmed but otherwise verbatim (not redacted).
#[must_use]
pub fn auth_required_message(message: &str, data: Option<&serde_json::Value>) -> Option<String> {
    let non_blank = |text: &str| {
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    };
    let from_data = data.and_then(|data| match data {
        serde_json::Value::String(text) => non_blank(text),
        serde_json::Value::Object(map) => ["message", "reason"].iter().find_map(|key| {
            map.get(*key)
                .and_then(serde_json::Value::as_str)
                .and_then(non_blank)
        }),
        _ => None,
    });
    from_data.or_else(|| non_blank(message))
}

/// The text a logged-out `_auth/status_update` carries (`detail`, else
/// `label`), used as the fallback reason of [`AgentEvent::AuthRequired`].
/// `None` for any other kind or when both are blank.
#[must_use]
pub fn logged_out_status_text(status: &AuthStatus) -> Option<String> {
    if status.kind != AuthStatusKind::None {
        return None;
    }
    status
        .detail
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .or_else(|| Some(status.label.trim()).filter(|text| !text.is_empty()))
        .map(str::to_string)
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
    /// Authenticate with one of the advertised methods. Runs off the command
    /// loop (other commands keep flowing while the agent waits for the
    /// browser) and ends with [`AgentEvent::AuthSucceeded`] or
    /// [`AgentEvent::AuthFailed`]. On success the UI may retry
    /// [`AgentCommand::NewSession`] (`docs/specs/modulos/acp.md` §Auth).
    Authenticate {
        /// Method id from `initialize`.
        method_id: String,
    },
    /// ACP `logout`, only sent when the agent announced
    /// `agentCapabilities.auth.logout`; ends with [`AgentEvent::LoggedOut`]
    /// (`ok: false` without sending anything when it was not announced).
    Logout,
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
        /// Why the agent asked for authentication, verbatim (see
        /// [`auth_required_message`]): the error's `data.message` /
        /// `data.reason` (or `data` itself when it is a string), else the
        /// error's `message`, else the text of the last logged-out
        /// `_auth/status_update`. `None` when the agent gave no text. Not
        /// redacted: the UI must redact it before showing or logging it.
        message: Option<String>,
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
    /// `authenticate` succeeded.
    AuthSucceeded {
        /// Method id that was used.
        method_id: String,
    },
    /// `authenticate` failed.
    AuthFailed {
        /// Method id that was used.
        method_id: String,
        /// Error message from the agent.
        message: String,
    },
    /// Answer to [`AgentCommand::Logout`].
    LoggedOut {
        /// `true` when the agent confirmed the logout; `false` when it
        /// failed or the agent does not announce `auth.logout`.
        ok: bool,
    },
    /// The agent pushed its identity through the `_auth/status_update`
    /// extension (announced via `agentCapabilities._meta.authStatus`, see
    /// [`agent_supports_auth_status`]).
    AuthStatus {
        /// Credential category.
        kind: AuthStatusKind,
        /// Human readable label ("Claude Max").
        label: String,
        /// Optional second line.
        detail: Option<String>,
        /// Account details, when reported.
        account: Option<AuthAccount>,
    },
    /// The agent sent `elicitation/complete` for a URL elicitation: the UI
    /// should close the matching dialog. If the request was still pending it
    /// has already been answered `accept` on the UI's behalf.
    ElicitationCompleted {
        /// Id of the [`AgentEvent::Elicitation`] being closed.
        id: PermissionRequestId,
        /// The agent's own `elicitationId`.
        elicitation_id: String,
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
    fn auth_required_message_prefers_data_then_message() {
        let data = serde_json::json!({ "reason": "token revoked" });
        assert_eq!(
            auth_required_message("Authentication required", Some(&data)).as_deref(),
            Some("token revoked")
        );
        let data = serde_json::json!({ "message": "  expired  ", "reason": "otro" });
        assert_eq!(
            auth_required_message("Authentication required", Some(&data)).as_deref(),
            Some("expired")
        );
        let data = serde_json::json!("sesión vencida");
        assert_eq!(
            auth_required_message("", Some(&data)).as_deref(),
            Some("sesión vencida")
        );
        let data = serde_json::json!({ "message": " ", "code": 7 });
        assert_eq!(
            auth_required_message("Authentication required", Some(&data)).as_deref(),
            Some("Authentication required")
        );
        assert_eq!(auth_required_message("   ", None), None);
        assert_eq!(
            auth_required_message("", Some(&serde_json::json!(42))),
            None
        );
    }

    #[test]
    fn logged_out_status_text_only_for_kind_none() {
        let status = |kind, label: &str, detail: Option<&str>| AuthStatus {
            kind,
            label: label.to_string(),
            detail: detail.map(str::to_string),
            account: None,
        };
        assert_eq!(
            logged_out_status_text(&status(
                AuthStatusKind::None,
                "Not logged in",
                Some("revoked")
            ))
            .as_deref(),
            Some("revoked")
        );
        assert_eq!(
            logged_out_status_text(&status(AuthStatusKind::None, "Not logged in", Some(" ")))
                .as_deref(),
            Some("Not logged in")
        );
        assert_eq!(
            logged_out_status_text(&status(AuthStatusKind::None, "", None)),
            None
        );
        assert_eq!(
            logged_out_status_text(&status(AuthStatusKind::Account, "Max", Some("x"))),
            None
        );
    }

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
    fn image_block_encodes_base64_without_uri() {
        let bytes: Vec<u8> = (0..=255u8).collect();
        let block = PromptBlock::image("image/png", bytes.clone()).expect("valid image");
        let content =
            build_prompt_content(vec![PromptBlock::Text("mirá".to_string()), block], None);
        assert_eq!(content.len(), 2);
        let ContentBlock::Image(image) = &content[1] else {
            panic!("se esperaba image: {:?}", content[1]);
        };
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.uri, None);
        assert_eq!(
            BASE64_STANDARD.decode(&image.data).expect("base64 valid"),
            bytes
        );
        let json = serde_json::to_value(&content[1]).expect("serializes");
        assert_eq!(json["type"], "image");
        assert_eq!(json["mimeType"], "image/png");
        assert!(
            json.get("uri").is_none(),
            "image blocks carry no uri: {json}"
        );
    }

    #[test]
    fn image_constructor_rejects_bad_mime_and_empty_data() {
        for mime in [
            "",
            "text/plain",
            "image/svg+xml",
            "image/",
            "png",
            "image/bmp",
        ] {
            assert!(
                matches!(
                    PromptBlock::image(mime, vec![1u8]),
                    Err(AcpError::InvalidParams(_))
                ),
                "{mime:?} must be rejected"
            );
        }
        assert!(matches!(
            PromptBlock::image("image/png", Vec::<u8>::new()),
            Err(AcpError::InvalidParams(_))
        ));
        // Case is normalized; every supported type is accepted.
        for mime in IMAGE_MIME_TYPES {
            let block = PromptBlock::image(&mime.to_ascii_uppercase(), vec![1u8]).expect("ok");
            assert!(matches!(&block, PromptBlock::Image { mime_type, .. } if mime_type == mime));
        }
    }

    #[test]
    fn validate_prompt_blocks_catches_directly_built_image_variants() {
        let good = PromptBlock::Image {
            mime_type: "image/jpeg".to_string(),
            data: Arc::from(vec![1u8, 2, 3]),
        };
        let bad_mime = PromptBlock::Image {
            mime_type: "application/pdf".to_string(),
            data: Arc::from(vec![1u8]),
        };
        let empty = PromptBlock::Image {
            mime_type: "image/png".to_string(),
            data: Arc::from(Vec::<u8>::new()),
        };
        assert!(validate_prompt_blocks(&[PromptBlock::Text("x".into()), good.clone()]).is_ok());
        assert!(validate_prompt_blocks(&[good.clone(), bad_mime]).is_err());
        assert!(validate_prompt_blocks(&[good, empty]).is_err());
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
    fn air_capabilities_require_version_and_string_array() {
        // Shape sent by claude-agent-acp / codex-acp at the top level of
        // `InitializeResponse._meta`.
        let real = as_meta(serde_json::json!({
            "steering": { "supported": true },
            "jetbrains": { "air": {
                "version": 1,
                "capabilities": ["sessionFailure", "agentFileChangeReport", "asyncTasks"]
            } }
        }));
        assert!(agent_supports_file_change_report(Some(&real)));

        // The pre-E4 object shape is not what agents understand.
        let legacy = as_meta(serde_json::json!({
            "jetbrains": { "air": { "capabilities": { "agentFileChangeReport": { "version": 1 } } } }
        }));
        assert!(!agent_supports_file_change_report(Some(&legacy)));

        let no_version = as_meta(serde_json::json!({
            "jetbrains": { "air": { "capabilities": ["agentFileChangeReport"] } }
        }));
        assert!(!agent_supports_file_change_report(Some(&no_version)));

        let other_caps = as_meta(serde_json::json!({
            "jetbrains": { "air": { "version": 1, "capabilities": ["asyncTasks"] } }
        }));
        assert!(!agent_supports_file_change_report(Some(&other_caps)));
        assert!(!agent_supports_file_change_report(None));
    }

    #[test]
    fn client_capability_meta_has_the_verified_shape() {
        let meta = file_change_report_capability_meta();
        assert_eq!(
            serde_json::Value::Object(meta),
            serde_json::json!({
                "jetbrains": { "air": { "version": 1, "capabilities": ["agentFileChangeReport"] } }
            })
        );
        let request = file_change_report_request_meta("r-1");
        let inner = &request["jetbrains"]["air"]["agentFileChangeReportRequest"];
        assert_eq!(
            inner,
            &serde_json::json!({ "version": 1, "requestId": "r-1" })
        );
    }

    #[test]
    fn auth_status_update_parses_account_identity() {
        let params = serde_json::json!({
            "authStatus": {
                "kind": "account",
                "label": "Claude Max",
                "account": { "email": "ana@example.com", "organization": "Org", "plan": "max" }
            }
        });
        let status = parse_auth_status_update(&params).expect("status");
        assert_eq!(status.kind, AuthStatusKind::Account);
        assert_eq!(status.label, "Claude Max");
        let account = status.account.expect("account");
        assert_eq!(account.email.as_deref(), Some("ana@example.com"));
        assert_eq!(account.organization.as_deref(), Some("Org"));
        assert_eq!(account.plan.as_deref(), Some("max"));
    }

    #[test]
    fn auth_status_update_tolerates_unknown_kind_and_rejects_garbage() {
        let status = parse_auth_status_update(&serde_json::json!({
            "authStatus": { "kind": "futuro", "label": "x", "detail": "d" }
        }))
        .expect("status");
        assert_eq!(status.kind, AuthStatusKind::Other("futuro".to_string()));
        assert_eq!(status.detail.as_deref(), Some("d"));
        assert!(status.account.is_none());
        let none = parse_auth_status_update(&serde_json::json!({
            "authStatus": { "kind": "none", "label": "Not logged in" }
        }))
        .expect("status");
        assert_eq!(none.kind, AuthStatusKind::None);
        assert!(parse_auth_status_update(&serde_json::json!({ "otra": 1 })).is_none());
        assert!(parse_auth_status_update(&serde_json::json!({ "authStatus": {} })).is_none());
    }

    #[test]
    fn capability_helpers_read_logout_and_auth_status() {
        use agent_client_protocol::schema::v1::{AgentAuthCapabilities, LogoutCapabilities};
        let plain = AgentCapabilities::new();
        assert!(!agent_supports_logout(&plain));
        assert!(!agent_supports_auth_status(&plain));
        let full = AgentCapabilities::new()
            .auth(AgentAuthCapabilities::new().logout(LogoutCapabilities::new()))
            .meta(as_meta(serde_json::json!({ "authStatus": {} })));
        assert!(agent_supports_logout(&full));
        assert!(agent_supports_auth_status(&full));
    }

    #[test]
    fn agent_supports_images_reads_prompt_capabilities() {
        use agent_client_protocol::schema::v1::PromptCapabilities;
        assert!(!agent_supports_images(&AgentCapabilities::new()));
        let with_images =
            AgentCapabilities::new().prompt_capabilities(PromptCapabilities::new().image(true));
        assert!(agent_supports_images(&with_images));
        let without = AgentCapabilities::new()
            .prompt_capabilities(PromptCapabilities::new().embedded_context(true));
        assert!(!agent_supports_images(&without));
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
