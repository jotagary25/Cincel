//! `AgentConnection`: spawn an agent process and drive an ACP v1 connection on
//! a dedicated tokio runtime thread.
//!
//! The UI never touches this module's internals: it pushes [`AgentCommand`]s
//! into one `async-channel` and reads [`AgentEvent`]s from another
//! (`docs/specs/03-arquitectura.md` §3).
//!
//! # Non-JSON stdout lines
//!
//! The SDK's `AcpAgent` transport fails the connection on malformed frames, so
//! this module builds its own transport out of [`agent_client_protocol::Lines`]
//! and filters the incoming stream: a line that does not start with `{` or `[`
//! is logged at `warn` and dropped instead of reaching the JSON-RPC engine.
//! Lines are read with `futures::AsyncBufReadExt::lines`, which grows the
//! buffer as needed (multi-megabyte frames are fine).
//!
//! # Filesystem sandbox
//!
//! [`AgentConnection::start`] takes the project root once, for the whole
//! connection's lifetime. `fs/read_text_file` and `fs/write_text_file`
//! requests for a path outside that root are answered with `invalid_params`
//! immediately (logged at `warn`) and never reach the UI as
//! [`AgentEvent::FsRead`]/[`AgentEvent::FsWrite`]. `line > total` on a read is
//! *not* checked here: the UI answers from `BufferStore`'s in-memory content
//! and is in a better position to report that boundary
//! (`docs/specs/modulos/acp.md` §Handlers).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AuthCapabilities, AuthMethod, AuthenticateRequest, CancelNotification, ClientCapabilities,
    CompleteElicitationNotification, CreateElicitationRequest, CreateElicitationResponse,
    ElicitationAcceptAction, ElicitationAction, ElicitationCapabilities,
    ElicitationFormCapabilities, ElicitationMode, ElicitationScope, ElicitationUrlCapabilities,
    ErrorCode, FileSystemCapabilities, Implementation, InitializeRequest, LoadSessionRequest,
    LogoutRequest, NewSessionRequest, PromptRequest, ReadTextFileRequest, ReadTextFileResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    ResumeSessionRequest, SelectedPermissionOutcome, SessionId, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionModeRequest, StopReason, ToolCallId, ToolCallStatus,
    WriteTextFileRequest, WriteTextFileResponse,
};
use agent_client_protocol::{ConnectionTo, Lines, UntypedMessage};
use futures::{AsyncBufReadExt, StreamExt};
use tokio::io::AsyncBufReadExt as _;
use tokio::sync::oneshot;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::error::{AcpError, Result};
use crate::process_env::ProcessEnv;
use crate::protocol::{
    self, AgentCommand, AgentEvent, ElicitationResponder, ElicitationSlots, PermissionOutcome,
    PermissionRequestId, PermissionResponder, PermissionSlots,
};
use crate::registry::LaunchSpec;

/// Client identity announced in `initialize`.
pub const CLIENT_NAME: &str = "cincel";
/// Client version announced in `initialize`.
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Last N bytes of stderr kept for diagnostics (`docs/specs/modulos/acp.md`).
const STDERR_TAIL_CAP: usize = 64 * 1024;

/// Capabilities Cincel advertises: filesystem in, terminal out, terminal auth
/// on (Cincel can re-run the agent invocation in a system terminal),
/// elicitation (form and url) since the client renders both, and the
/// `jetbrains.air` extension with `agentFileChangeReport` in `_meta`
/// (where `claude-agent-acp` and `codex-acp` look for it).
#[must_use]
pub fn client_capabilities() -> ClientCapabilities {
    ClientCapabilities::new()
        .fs(FileSystemCapabilities::new()
            .read_text_file(true)
            .write_text_file(true))
        .terminal(false)
        .auth(AuthCapabilities::new().terminal(true))
        .elicitation(
            ElicitationCapabilities::new()
                .form(ElicitationFormCapabilities::new())
                .url(ElicitationUrlCapabilities::new()),
        )
        .meta(protocol::file_change_report_capability_meta())
}

/// Handle to the worker thread driving one agent.
#[derive(Debug)]
pub struct AgentConnection {
    commands: async_channel::Sender<AgentCommand>,
    events: async_channel::Receiver<AgentEvent>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl AgentConnection {
    /// Start the worker thread. The agent process itself is only launched when
    /// an [`AgentCommand::Spawn`] arrives.
    ///
    /// `project_root` is the sandbox boundary for `fs/read_text_file` and
    /// `fs/write_text_file` for the lifetime of this connection.
    ///
    /// # Panics
    ///
    /// Panics if the OS refuses to create the worker thread.
    #[must_use]
    pub fn start(project_root: impl Into<PathBuf>) -> Self {
        Self::start_with_env(project_root, ProcessEnv::default())
    }

    /// Same as [`AgentConnection::start`], applying `env` to every agent
    /// process this connection spawns: the profile variable to set, the
    /// provider credential variables to strip and the private Node runtime
    /// to put first in `PATH` (see [`ProcessEnv`] for the exact order).
    ///
    /// # Panics
    ///
    /// Panics if the OS refuses to create the worker thread.
    #[must_use]
    pub fn start_with_env(project_root: impl Into<PathBuf>, env: ProcessEnv) -> Self {
        let project_root = project_root.into();
        let (command_tx, command_rx) = async_channel::unbounded::<AgentCommand>();
        let (event_tx, event_rx) = async_channel::unbounded::<AgentEvent>();

        let worker = std::thread::Builder::new()
            .name("cincel-acp".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = event_tx.send_blocking(AgentEvent::Error {
                            message: format!("no se pudo crear el runtime tokio: {error}"),
                            stderr_tail: String::new(),
                        });
                        return;
                    }
                };
                runtime.block_on(worker_main(command_rx, event_tx, project_root, env));
            })
            .expect("no se pudo crear el hilo cincel-acp");

        Self {
            commands: command_tx,
            events: event_rx,
            worker: Some(worker),
        }
    }

    /// Sender used by the UI to push commands.
    #[must_use]
    pub fn commands(&self) -> &async_channel::Sender<AgentCommand> {
        &self.commands
    }

    /// Receiver the UI drains for events.
    #[must_use]
    pub fn events(&self) -> &async_channel::Receiver<AgentEvent> {
        &self.events
    }

    /// Send one command.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::Closed`] when the worker is gone.
    pub async fn send(&self, command: AgentCommand) -> Result<()> {
        self.commands
            .send(command)
            .await
            .map_err(|_| AcpError::Closed)
    }

    /// Receive one event.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::Closed`] when the worker is gone.
    pub async fn recv(&self) -> Result<AgentEvent> {
        self.events.recv().await.map_err(|_| AcpError::Closed)
    }

    /// Ask the worker to shut down and wait for the thread to finish.
    pub fn shutdown(&mut self) {
        let _ = self.commands.send_blocking(AgentCommand::Shutdown);
        self.commands.close();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for AgentConnection {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Ring buffer keeping the last [`STDERR_TAIL_CAP`] bytes of an agent's
/// stderr, line by line.
#[derive(Debug, Clone, Default)]
struct StderrTail {
    inner: Arc<Mutex<StderrTailInner>>,
}

#[derive(Debug, Default)]
struct StderrTailInner {
    lines: std::collections::VecDeque<String>,
    bytes: usize,
}

impl StderrTail {
    fn push_line(&self, line: &str) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        inner.bytes += line.len() + 1;
        inner.lines.push_back(line.to_string());
        while inner.bytes > STDERR_TAIL_CAP {
            match inner.lines.pop_front() {
                Some(removed) => inner.bytes = inner.bytes.saturating_sub(removed.len() + 1),
                None => break,
            }
        }
    }

    fn snapshot(&self) -> String {
        self.inner
            .lock()
            .map(|inner| inner.lines.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }
}

/// State shared between the dispatch handlers and the command loop.
struct Shared {
    events: async_channel::Sender<AgentEvent>,
    project_root: PathBuf,
    permission_slots: PermissionSlots,
    elicitation_slots: ElicitationSlots,
    next_request_id: AtomicU64,
    stderr: StderrTail,
    /// `session_id -> tool_call_id -> status` for tool calls still
    /// `pending`/`in_progress`, used to answer cancellation
    /// (`docs/specs/modulos/acp.md` §Cancelación).
    tool_calls: Mutex<HashMap<SessionId, HashMap<ToolCallId, ToolCallStatus>>>,
    /// `session_id -> requestId` for the in-flight `agentFileChangeReport`
    /// request of that session's current turn.
    pending_file_change_requests: Mutex<HashMap<SessionId, String>>,
    /// Agent `elicitationId` -> our request id, for URL elicitations, so
    /// `elicitation/complete` can close the right dialog.
    url_elicitations: Mutex<HashMap<String, PermissionRequestId>>,
}

impl Shared {
    fn emit(&self, event: AgentEvent) {
        // Unbounded channel: only fails when the UI dropped the receiver.
        if self.events.send_blocking(event).is_err() {
            tracing::debug!("la UI cerró el canal de eventos");
        }
    }

    fn error(&self, message: impl Into<String>) {
        self.emit(AgentEvent::Error {
            message: message.into(),
            stderr_tail: self.stderr.snapshot(),
        });
    }

    fn next_id(&self) -> PermissionRequestId {
        PermissionRequestId(self.next_request_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Update the last known status of one tool call from a notification.
    fn track_tool_call(&self, session_id: &SessionId, update: &SessionUpdate) {
        let (id, status) = match update {
            SessionUpdate::ToolCall(call) => (call.tool_call_id.clone(), Some(call.status)),
            SessionUpdate::ToolCallUpdate(update) => {
                (update.tool_call_id.clone(), update.fields.status)
            }
            _ => return,
        };
        let Some(status) = status else { return };
        let Ok(mut map) = self.tool_calls.lock() else {
            return;
        };
        let bucket = map.entry(session_id.clone()).or_default();
        match status {
            ToolCallStatus::Completed | ToolCallStatus::Failed => {
                bucket.remove(&id);
            }
            other => {
                bucket.insert(id, other);
            }
        }
    }

    /// Ids still `pending`/`in_progress` for a session, and forget them.
    fn take_pending_tool_calls(&self, session_id: &SessionId) -> Vec<ToolCallId> {
        self.tool_calls
            .lock()
            .ok()
            .and_then(|mut map| map.remove(session_id))
            .map(|bucket| bucket.into_keys().collect())
            .unwrap_or_default()
    }

    /// Answer `cancelled` to every pending permission request of `session_id`.
    fn cancel_permissions_for(&self, session_id: &SessionId) {
        let Ok(mut slots) = self.permission_slots.lock() else {
            return;
        };
        let matching: Vec<PermissionRequestId> = slots
            .iter()
            .filter(|(_, (session, _))| session == session_id)
            .map(|(id, _)| *id)
            .collect();
        for id in matching {
            if let Some((_, sender)) = slots.remove(&id) {
                let _ = sender.send(PermissionOutcome::Cancelled);
            }
        }
    }

    /// Answer `cancel` to every pending elicitation of `session_id`.
    fn cancel_elicitations_for(&self, session_id: &SessionId) {
        let Ok(mut slots) = self.elicitation_slots.lock() else {
            return;
        };
        let matching: Vec<PermissionRequestId> = slots
            .iter()
            .filter(|(_, (session, _))| session.as_ref() == Some(session_id))
            .map(|(id, _)| *id)
            .collect();
        for id in matching {
            if let Some((_, sender)) = slots.remove(&id) {
                let _ = sender.send(CreateElicitationResponse::new(
                    agent_client_protocol::schema::v1::ElicitationAction::Cancel,
                ));
            }
        }
    }
}

/// Worker entry point: wait for `Spawn`, then run one connection at a time.
async fn worker_main(
    commands: async_channel::Receiver<AgentCommand>,
    events: async_channel::Sender<AgentEvent>,
    project_root: PathBuf,
    env: ProcessEnv,
) {
    loop {
        let Ok(command) = commands.recv().await else {
            return;
        };
        match command {
            AgentCommand::Spawn { launch, cwd } => {
                if let Err(error) =
                    run_agent(&launch, &env, &cwd, &project_root, &commands, &events).await
                {
                    let _ = events
                        .send(AgentEvent::Error {
                            message: error.to_string(),
                            stderr_tail: String::new(),
                        })
                        .await;
                }
            }
            AgentCommand::Shutdown => return,
            other => {
                let _ = events
                    .send(AgentEvent::Error {
                        message: format!("comando {other:?} recibido sin agente activo"),
                        stderr_tail: String::new(),
                    })
                    .await;
            }
        }
    }
}

/// Spawn the process, wire the transport and run the connection until the
/// command loop finishes or the child dies.
async fn run_agent(
    launch: &LaunchSpec,
    env: &ProcessEnv,
    cwd: &Path,
    project_root: &Path,
    commands: &async_channel::Receiver<AgentCommand>,
    events: &async_channel::Sender<AgentEvent>,
) -> Result<()> {
    let mut command = tokio::process::Command::new(&launch.program);
    command
        .args(&launch.args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // `NO_BROWSER` is no longer forced: it hides the best login methods of
    // Claude, Codex and Gemini. A caller that wants it opts in through
    // `LaunchSpec::env` (see `LaunchSpec::with_no_browser`).
    env.apply(&mut command, &launch.env);
    #[cfg(unix)]
    {
        // Own process group so we can kill `npx` and the real agent together.
        command.process_group(0);
    }

    let mut child = command.spawn().map_err(|source| AcpError::Spawn {
        program: launch.program.clone(),
        source,
    })?;
    let pid = child.id();
    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let stderr_tail = StderrTail::default();

    // Drain stderr into events (and the ring buffer); never let it fill the pipe.
    let stderr_events = events.clone();
    let stderr_tail_task = stderr_tail.clone();
    let stderr_task = tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            stderr_tail_task.push_line(&line);
            if stderr_events.send(AgentEvent::Stderr(line)).await.is_err() {
                break;
            }
        }
    });

    let shared = Arc::new(Shared {
        events: events.clone(),
        project_root: project_root.to_path_buf(),
        permission_slots: PermissionSlots::default(),
        elicitation_slots: ElicitationSlots::default(),
        next_request_id: AtomicU64::new(1),
        stderr: stderr_tail.clone(),
        tool_calls: Mutex::new(HashMap::new()),
        pending_file_change_requests: Mutex::new(HashMap::new()),
        url_elicitations: Mutex::new(HashMap::new()),
    });

    let transport = build_transport(stdin, stdout);
    let connection = {
        let shared_notify = shared.clone();
        let shared_permission = shared.clone();
        let shared_read = shared.clone();
        let shared_write = shared.clone();
        let shared_elicitation = shared.clone();
        let shared_complete = shared.clone();
        let shared_ext = shared.clone();
        let shared_main = shared.clone();
        let commands = commands.clone();
        agent_client_protocol::Client
            .builder()
            .name(CLIENT_NAME)
            .on_receive_notification(
                async move |notification: SessionNotification, _cx| {
                    handle_notification(&shared_notify, notification);
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .on_receive_notification(
                async move |notification: CompleteElicitationNotification, _cx| {
                    handle_elicitation_complete(&shared_complete, &notification);
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            // Untyped fallback, registered after every typed notification
            // handler: extension notifications (`_auth/status_update`).
            .on_receive_notification(
                async move |notification: UntypedMessage, _cx| {
                    handle_ext_notification(&shared_ext, &notification);
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .on_receive_request(
                async move |request: RequestPermissionRequest, responder, cx: ConnectionTo<_>| {
                    handle_permission(&shared_permission, request, responder, &cx)
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: ReadTextFileRequest, responder, cx: ConnectionTo<_>| {
                    handle_read(&shared_read, request, responder, &cx)
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: WriteTextFileRequest, responder, cx: ConnectionTo<_>| {
                    handle_write(&shared_write, request, responder, &cx)
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: CreateElicitationRequest, responder, cx: ConnectionTo<_>| {
                    handle_elicitation(&shared_elicitation, request, responder, &cx)
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_with(transport, async move |cx: ConnectionTo<_>| {
                command_loop(&shared_main, &commands, &cx).await
            })
    };

    let result = tokio::select! {
        result = connection => result.map_err(AcpError::from),
        status = child.wait() => {
            let code = status.ok().and_then(|status| status.code());
            let _ = events.send(AgentEvent::Exited { code, stderr_tail: stderr_tail.snapshot() }).await;
            return Ok(());
        }
    };

    // The connection is done: tear the whole process group down.
    kill_process_group(pid);
    let code = match child.wait().await {
        Ok(status) => status.code(),
        Err(_) => None,
    };
    stderr_task.abort();
    let _ = events
        .send(AgentEvent::Exited {
            code,
            stderr_tail: stderr_tail.snapshot(),
        })
        .await;
    result.map(|_| ())
}

fn build_transport(
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
) -> Lines<
    impl futures::Sink<String, Error = std::io::Error> + Send + 'static,
    impl futures::Stream<Item = std::io::Result<String>> + Send + 'static,
> {
    let reader = futures::io::BufReader::new(stdout.compat());
    // `lines()` has no maximum line length: multi-MB frames are read whole.
    let incoming = Box::pin(reader.lines().filter_map(|line| async move {
        match line {
            Ok(text) => {
                let trimmed = text.trim_start();
                if trimmed.is_empty() {
                    None
                } else if trimmed.starts_with('{') || trimmed.starts_with('[') {
                    Some(Ok(text))
                } else {
                    // Tolerate agents that print banners on stdout.
                    tracing::warn!(line = %text, "línea no-JSON en stdout del agente, ignorada");
                    None
                }
            }
            Err(error) => Some(Err(error)),
        }
    }));

    let outgoing = futures::sink::unfold(
        Box::pin(stdin.compat_write()),
        async move |mut writer, line: String| {
            use futures::AsyncWriteExt as _;
            let mut bytes = line.into_bytes();
            bytes.push(b'\n');
            writer.write_all(&bytes).await?;
            writer.flush().await?;
            Ok::<_, std::io::Error>(writer)
        },
    );

    Lines::new(outgoing, incoming)
}

/// Whether `path` lexically lies within `root`, without touching disk (a
/// path for `fs/write_text_file` may not exist yet, so `canonicalize` is not
/// an option). `..`/`.` components are resolved lexically first.
fn path_within_root(path: &Path, root: &Path) -> bool {
    normalize_lexically(path).starts_with(normalize_lexically(root))
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn handle_notification(shared: &Arc<Shared>, notification: SessionNotification) {
    let session_id = notification.session_id.clone();
    shared.track_tool_call(&session_id, &notification.update);

    if let SessionUpdate::SessionInfoUpdate(info) = &notification.update {
        let expected = shared
            .pending_file_change_requests
            .lock()
            .ok()
            .and_then(|map| map.get(&session_id).cloned());
        if let Some(expected) = expected
            && let Some(report) = protocol::parse_file_change_report(info.meta.as_ref(), &expected)
        {
            shared.emit(AgentEvent::FileChangeReport {
                session_id: session_id.clone(),
                report,
            });
        }
    }

    shared.emit(AgentEvent::Update {
        session_id,
        update: Box::new(notification.update),
    });
}

fn handle_permission(
    shared: &Arc<Shared>,
    request: RequestPermissionRequest,
    responder: agent_client_protocol::Responder<RequestPermissionResponse>,
    cx: &ConnectionTo<agent_client_protocol::Agent>,
) -> std::result::Result<(), agent_client_protocol::Error> {
    let id = shared.next_id();
    let (tx, rx) = oneshot::channel();
    if let Ok(mut slots) = shared.permission_slots.lock() {
        slots.insert(id, (request.session_id.clone(), tx));
    }

    shared.emit(AgentEvent::PermissionRequest {
        id,
        session_id: request.session_id.clone(),
        tool_call: Box::new(request.tool_call.clone()),
        options: request.options.clone(),
        reply: PermissionResponder::new(id, shared.permission_slots.clone()),
    });

    // Answer outside the dispatch loop so session updates keep flowing while
    // the user thinks. There is deliberately no timeout (acp.md §Handlers).
    let slots = shared.permission_slots.clone();
    cx.spawn(async move {
        let outcome = rx.await.unwrap_or(PermissionOutcome::Cancelled);
        if let Ok(mut slots) = slots.lock() {
            slots.remove(&id);
        }
        let outcome = match outcome {
            PermissionOutcome::Selected(option_id) => {
                RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id))
            }
            PermissionOutcome::Cancelled => RequestPermissionOutcome::Cancelled,
        };
        responder.respond(RequestPermissionResponse::new(outcome))
    })
}

fn handle_elicitation(
    shared: &Arc<Shared>,
    request: CreateElicitationRequest,
    responder: agent_client_protocol::Responder<CreateElicitationResponse>,
    cx: &ConnectionTo<agent_client_protocol::Agent>,
) -> std::result::Result<(), agent_client_protocol::Error> {
    let id = shared.next_id();
    let session_id = match request.scope() {
        ElicitationScope::Session(scope) => Some(scope.session_id.clone()),
        _ => None,
    };
    let (tx, rx) = oneshot::channel();
    if let Ok(mut slots) = shared.elicitation_slots.lock() {
        slots.insert(id, (session_id.clone(), tx));
    }
    if let ElicitationMode::Url(url) = &request.mode
        && let Ok(mut map) = shared.url_elicitations.lock()
    {
        map.insert(url.elicitation_id.0.to_string(), id);
    }

    shared.emit(AgentEvent::Elicitation {
        id,
        session_id,
        request: Box::new(request),
        reply: ElicitationResponder::new(id, shared.elicitation_slots.clone()),
    });

    let slots = shared.elicitation_slots.clone();
    cx.spawn(async move {
        let response = rx.await.unwrap_or_else(|_| {
            CreateElicitationResponse::new(
                agent_client_protocol::schema::v1::ElicitationAction::Cancel,
            )
        });
        if let Ok(mut slots) = slots.lock() {
            slots.remove(&id);
        }
        responder.respond(response)
    })
}

/// `elicitation/complete`: answer the matching request `accept` if the UI has
/// not answered it yet (the agent no longer waits for it; codex-acp races
/// login completion against the answer) and tell the UI to close it.
fn handle_elicitation_complete(
    shared: &Arc<Shared>,
    notification: &CompleteElicitationNotification,
) {
    let elicitation_id = notification.elicitation_id.0.to_string();
    let id = shared
        .url_elicitations
        .lock()
        .ok()
        .and_then(|mut map| map.remove(&elicitation_id));
    let Some(id) = id else {
        tracing::debug!(%elicitation_id, "elicitation/complete sin elicitación conocida");
        return;
    };
    let pending = shared
        .elicitation_slots
        .lock()
        .map(|mut slots| slots.remove(&id))
        .unwrap_or(None);
    if let Some((_, sender)) = pending {
        let _ = sender.send(CreateElicitationResponse::new(ElicitationAction::Accept(
            ElicitationAcceptAction::new(),
        )));
    }
    shared.emit(AgentEvent::ElicitationCompleted { id, elicitation_id });
}

/// Extension notifications. Only `_auth/status_update` is understood; the
/// rest are logged and dropped, as the spec asks of unknown `_` methods.
fn handle_ext_notification(shared: &Arc<Shared>, notification: &UntypedMessage) {
    if notification.method() != protocol::AUTH_STATUS_UPDATE_METHOD {
        tracing::debug!(method = notification.method(), "notificación ignorada");
        return;
    }
    match protocol::parse_auth_status_update(notification.params()) {
        Some(status) => shared.emit(AgentEvent::AuthStatus {
            kind: status.kind,
            label: status.label,
            detail: status.detail,
            account: status.account,
        }),
        None => tracing::warn!("_auth/status_update con formato inválido, ignorado"),
    }
}

fn handle_read(
    shared: &Arc<Shared>,
    request: ReadTextFileRequest,
    responder: agent_client_protocol::Responder<ReadTextFileResponse>,
    cx: &ConnectionTo<agent_client_protocol::Agent>,
) -> std::result::Result<(), agent_client_protocol::Error> {
    if !path_within_root(&request.path, &shared.project_root) {
        tracing::warn!(
            path = %request.path.display(),
            root = %shared.project_root.display(),
            "fs/read_text_file fuera del proyecto, rechazado"
        );
        return responder.respond_with_error(
            crate::protocol::FsError::InvalidParams(format!(
                "`{}` está fuera del proyecto",
                request.path.display()
            ))
            .into_protocol(),
        );
    }
    let (tx, rx) = oneshot::channel();
    shared.emit(AgentEvent::FsRead {
        session_id: request.session_id,
        path: request.path,
        line: request.line,
        limit: request.limit,
        reply: tx,
    });
    cx.spawn(async move {
        match rx.await {
            Ok(Ok(content)) => responder.respond(ReadTextFileResponse::new(content)),
            Ok(Err(error)) => responder.respond_with_error(error.into_protocol()),
            Err(_) => responder.respond_with_internal_error("la UI no respondió"),
        }
    })
}

fn handle_write(
    shared: &Arc<Shared>,
    request: WriteTextFileRequest,
    responder: agent_client_protocol::Responder<WriteTextFileResponse>,
    cx: &ConnectionTo<agent_client_protocol::Agent>,
) -> std::result::Result<(), agent_client_protocol::Error> {
    if !path_within_root(&request.path, &shared.project_root) {
        tracing::warn!(
            path = %request.path.display(),
            root = %shared.project_root.display(),
            "fs/write_text_file fuera del proyecto, rechazado"
        );
        return responder.respond_with_error(
            crate::protocol::FsError::InvalidParams(format!(
                "`{}` está fuera del proyecto",
                request.path.display()
            ))
            .into_protocol(),
        );
    }
    let (tx, rx) = oneshot::channel();
    shared.emit(AgentEvent::FsWrite {
        session_id: request.session_id,
        path: request.path,
        content: request.content,
        reply: tx,
    });
    cx.spawn(async move {
        match rx.await {
            Ok(Ok(())) => responder.respond(WriteTextFileResponse::new()),
            Ok(Err(error)) => responder.respond_with_error(error.into_protocol()),
            Err(_) => responder.respond_with_internal_error("la UI no respondió"),
        }
    })
}

/// Drive `initialize` and then serve UI commands until `Shutdown`.
async fn command_loop(
    shared: &Arc<Shared>,
    commands: &async_channel::Receiver<AgentCommand>,
    cx: &ConnectionTo<agent_client_protocol::Agent>,
) -> std::result::Result<(), agent_client_protocol::Error> {
    let initialize = cx
        .send_request(
            InitializeRequest::new(ProtocolVersion::V1)
                .client_capabilities(client_capabilities())
                .client_info(Implementation::new(CLIENT_NAME, CLIENT_VERSION))
                .meta(protocol::file_change_report_capability_meta()),
        )
        .block_task()
        .await?;

    let auth_methods: Vec<AuthMethod> = initialize.auth_methods.clone();
    let file_change_report_supported =
        protocol::agent_supports_file_change_report(initialize.meta.as_ref());
    let logout_supported = protocol::agent_supports_logout(&initialize.agent_capabilities);
    shared.emit(AgentEvent::Connected {
        agent_info: initialize.agent_info.clone(),
        auth_methods: auth_methods.clone(),
        capabilities: Box::new(initialize.agent_capabilities.clone()),
    });

    while let Ok(command) = commands.recv().await {
        match command {
            AgentCommand::Shutdown => break,
            AgentCommand::Spawn { .. } => {
                shared.error("ya hay un agente activo en esta conexión");
            }
            AgentCommand::NewSession { cwd, mcp_servers } => {
                let request = NewSessionRequest::new(cwd).mcp_servers(
                    mcp_servers
                        .into_iter()
                        .map(protocol::McpServerSpec::into_wire)
                        .collect(),
                );
                match cx.send_request(request).block_task().await {
                    Ok(response) => shared.emit(AgentEvent::SessionCreated {
                        session_id: response.session_id,
                        modes: response.modes,
                        config_options: response.config_options.unwrap_or_default(),
                        commands: Vec::new(),
                    }),
                    Err(error) if error.code == ErrorCode::AuthRequired => {
                        shared.emit(AgentEvent::AuthRequired {
                            methods: auth_methods.clone(),
                        });
                    }
                    Err(error) => shared.error(error.message),
                }
            }
            AgentCommand::LoadSession {
                session_id,
                cwd,
                mcp_servers,
            } => {
                let request = LoadSessionRequest::new(session_id.clone(), cwd).mcp_servers(
                    mcp_servers
                        .into_iter()
                        .map(protocol::McpServerSpec::into_wire)
                        .collect(),
                );
                // The agent replays history as `session/update` notifications
                // while this request is outstanding; `handle_notification`
                // turns those into `AgentEvent::Update` as usual.
                match cx.send_request(request).block_task().await {
                    Ok(response) => shared.emit(AgentEvent::SessionCreated {
                        session_id,
                        modes: response.modes,
                        config_options: response.config_options.unwrap_or_default(),
                        commands: Vec::new(),
                    }),
                    Err(error) if error.code == ErrorCode::AuthRequired => {
                        shared.emit(AgentEvent::AuthRequired {
                            methods: auth_methods.clone(),
                        });
                    }
                    Err(error) => shared.error(error.message),
                }
            }
            AgentCommand::ResumeSession {
                session_id,
                cwd,
                mcp_servers,
            } => {
                let request = ResumeSessionRequest::new(session_id.clone(), cwd).mcp_servers(
                    mcp_servers
                        .into_iter()
                        .map(protocol::McpServerSpec::into_wire)
                        .collect(),
                );
                match cx.send_request(request).block_task().await {
                    Ok(response) => shared.emit(AgentEvent::SessionCreated {
                        session_id,
                        modes: response.modes,
                        config_options: response.config_options.unwrap_or_default(),
                        commands: Vec::new(),
                    }),
                    Err(error) if error.code == ErrorCode::AuthRequired => {
                        shared.emit(AgentEvent::AuthRequired {
                            methods: auth_methods.clone(),
                        });
                    }
                    Err(error) => shared.error(error.message),
                }
            }
            AgentCommand::Authenticate { method_id } => {
                // Off the command loop: an OAuth `authenticate` can wait for
                // the browser for minutes.
                let shared = shared.clone();
                let cx_auth = cx.clone();
                cx.spawn(async move {
                    let result = cx_auth
                        .send_request(AuthenticateRequest::new(method_id.clone()))
                        .block_task()
                        .await;
                    match result {
                        Ok(_) => shared.emit(AgentEvent::AuthSucceeded { method_id }),
                        Err(error) => shared.emit(AgentEvent::AuthFailed {
                            method_id,
                            message: error.message,
                        }),
                    }
                    Ok(())
                })?;
            }
            AgentCommand::Logout => {
                if !logout_supported {
                    tracing::warn!("el agente no anuncia auth.logout; no se envía logout");
                    shared.emit(AgentEvent::LoggedOut { ok: false });
                    continue;
                }
                let shared = shared.clone();
                let cx_logout = cx.clone();
                cx.spawn(async move {
                    let ok = match cx_logout
                        .send_request(LogoutRequest::new())
                        .block_task()
                        .await
                    {
                        Ok(_) => true,
                        Err(error) => {
                            tracing::warn!(message = %error.message, "logout falló");
                            false
                        }
                    };
                    shared.emit(AgentEvent::LoggedOut { ok });
                    Ok(())
                })?;
            }
            AgentCommand::Prompt {
                session_id,
                blocks,
                feedback,
            } => {
                // Run the turn off the command loop so `Cancel` still works.
                let shared = shared.clone();
                let cx_prompt = cx.clone();
                cx.spawn(async move {
                    let content = protocol::build_prompt_content(blocks, feedback);
                    let mut request = PromptRequest::new(session_id.clone(), content);
                    if file_change_report_supported {
                        let request_id = format!("cincel-{}", shared.next_id().0);
                        if let Ok(mut pending) = shared.pending_file_change_requests.lock() {
                            pending.insert(session_id.clone(), request_id.clone());
                        }
                        request =
                            request.meta(protocol::file_change_report_request_meta(&request_id));
                    }
                    match cx_prompt.send_request(request).block_task().await {
                        Ok(response) => {
                            if response.stop_reason == StopReason::Cancelled {
                                let ids = shared.take_pending_tool_calls(&session_id);
                                if !ids.is_empty() {
                                    shared.emit(AgentEvent::ToolCallsCancelled {
                                        session_id: session_id.clone(),
                                        ids,
                                    });
                                }
                            } else {
                                shared.take_pending_tool_calls(&session_id);
                            }
                            if let Ok(mut pending) = shared.pending_file_change_requests.lock() {
                                pending.remove(&session_id);
                            }
                            shared.emit(AgentEvent::TurnEnded {
                                session_id,
                                stop_reason: response.stop_reason,
                            });
                        }
                        Err(error) => shared.error(error.message),
                    }
                    Ok(())
                })?;
            }
            AgentCommand::Cancel { session_id } => {
                cx.send_notification(CancelNotification::new(session_id.clone()))?;
                shared.cancel_permissions_for(&session_id);
                shared.cancel_elicitations_for(&session_id);
            }
            AgentCommand::SetMode {
                session_id,
                mode_id,
            } => {
                if let Err(error) = cx
                    .send_request(SetSessionModeRequest::new(session_id, mode_id))
                    .block_task()
                    .await
                {
                    shared.error(error.message);
                }
            }
            AgentCommand::SetConfigOption {
                session_id,
                config_id,
                value,
            } => {
                if let Err(error) = cx
                    .send_request(SetSessionConfigOptionRequest::new(
                        session_id, config_id, value,
                    ))
                    .block_task()
                    .await
                {
                    shared.error(error.message);
                }
            }
            AgentCommand::RespondPermission { id, outcome } => {
                let sender = shared
                    .permission_slots
                    .lock()
                    .map(|mut slots| slots.remove(&id))
                    .unwrap_or(None);
                match sender {
                    Some((_, sender)) => {
                        let _ = sender.send(outcome);
                    }
                    None => shared.error(format!("no hay un permiso pendiente con id {id}")),
                }
            }
            AgentCommand::RespondElicitation { id, response } => {
                let sender = shared
                    .elicitation_slots
                    .lock()
                    .map(|mut slots| slots.remove(&id))
                    .unwrap_or(None);
                match sender {
                    Some((_, sender)) => {
                        let _ = sender.send(response);
                    }
                    None => {
                        shared.error(format!("no hay una elicitación pendiente con id {id}"));
                    }
                }
            }
        }
    }

    Ok(())
}

/// SIGKILL the agent's whole process group (`npx` leaves a wrapper behind).
fn kill_process_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid
        && let Some(pid) = rustix::process::Pid::from_raw(pid.cast_signed())
    {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Convenience for building `Vec<PathBuf>` out of a tool call's locations.
#[must_use]
pub fn tool_call_paths(update: &agent_client_protocol::schema::v1::ToolCallUpdate) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = update
        .fields
        .locations
        .iter()
        .flatten()
        .map(|location| location.path.clone())
        .collect();
    if let Some(content) = &update.fields.content {
        for item in content {
            if let agent_client_protocol::schema::v1::ToolCallContent::Diff(diff) = item {
                paths.push(diff.path.clone());
            }
        }
    }
    paths
}

/// Extract the plain text of a `SessionUpdate` chunk, if it carries any.
#[must_use]
pub fn chunk_text(update: &SessionUpdate) -> Option<&str> {
    let chunk = match update {
        SessionUpdate::AgentMessageChunk(chunk)
        | SessionUpdate::AgentThoughtChunk(chunk)
        | SessionUpdate::UserMessageChunk(chunk) => chunk,
        _ => return None,
    };
    match &chunk.content {
        agent_client_protocol::schema::v1::ContentBlock::Text(text) => Some(&text.text),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_within_root_accepts_nested_paths() {
        let root = Path::new("/home/user/project");
        assert!(path_within_root(
            Path::new("/home/user/project/src/main.rs"),
            root
        ));
        assert!(path_within_root(Path::new("/home/user/project"), root));
    }

    #[test]
    fn path_within_root_rejects_paths_outside_and_traversal() {
        let root = Path::new("/home/user/project");
        assert!(!path_within_root(
            Path::new("/home/user/other/secret.txt"),
            root
        ));
        assert!(!path_within_root(
            Path::new("/home/user/project/../other/secret.txt"),
            root
        ));
        // Prefix collision without a path boundary must not pass.
        assert!(!path_within_root(
            Path::new("/home/user/project-evil/x"),
            root
        ));
    }

    #[test]
    fn stderr_tail_keeps_only_the_last_bytes() {
        let tail = StderrTail::default();
        let long_line = "x".repeat(1024);
        for _ in 0..100 {
            tail.push_line(&long_line);
        }
        let snapshot = tail.snapshot();
        assert!(snapshot.len() <= STDERR_TAIL_CAP + long_line.len());
        assert!(snapshot.ends_with(&long_line));
    }
}
