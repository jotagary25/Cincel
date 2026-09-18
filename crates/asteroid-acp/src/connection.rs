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

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AuthCapabilities, AuthMethod, AuthenticateRequest, CancelNotification, ClientCapabilities,
    ErrorCode, FileSystemCapabilities, Implementation, InitializeRequest, NewSessionRequest,
    PromptRequest, ReadTextFileRequest, ReadTextFileResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionNotification, SessionUpdate, SetSessionConfigOptionRequest, SetSessionModeRequest,
    WriteTextFileRequest, WriteTextFileResponse,
};
use agent_client_protocol::{ConnectionTo, Lines};
use futures::{AsyncBufReadExt, StreamExt};
use tokio::io::AsyncBufReadExt as _;
use tokio::sync::oneshot;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::error::{AcpError, Result};
use crate::protocol::{
    AgentCommand, AgentEvent, PermissionOutcome, PermissionRequestId, PermissionResponder,
    PermissionSlots,
};
use crate::registry::LaunchSpec;

/// Client identity announced in `initialize`.
pub const CLIENT_NAME: &str = "asteroid";
/// Client version announced in `initialize`.
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Capabilities Asteroid advertises: filesystem in, terminal out, terminal auth
/// on (Asteroid can re-run the agent invocation in a system terminal).
#[must_use]
pub fn client_capabilities() -> ClientCapabilities {
    ClientCapabilities::new()
        .fs(FileSystemCapabilities::new()
            .read_text_file(true)
            .write_text_file(true))
        .terminal(false)
        .auth(AuthCapabilities::new().terminal(true))
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
    /// # Panics
    ///
    /// Panics if the OS refuses to create the worker thread.
    #[must_use]
    pub fn start() -> Self {
        let (command_tx, command_rx) = async_channel::unbounded::<AgentCommand>();
        let (event_tx, event_rx) = async_channel::unbounded::<AgentEvent>();

        let worker = std::thread::Builder::new()
            .name("asteroid-acp".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = event_tx.send_blocking(AgentEvent::Error(format!(
                            "no se pudo crear el runtime tokio: {error}"
                        )));
                        return;
                    }
                };
                runtime.block_on(worker_main(command_rx, event_tx));
            })
            .expect("no se pudo crear el hilo asteroid-acp");

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

/// State shared between the dispatch handlers and the command loop.
struct Shared {
    events: async_channel::Sender<AgentEvent>,
    permission_slots: PermissionSlots,
    next_permission_id: AtomicU64,
}

impl Shared {
    fn emit(&self, event: AgentEvent) {
        // Unbounded channel: only fails when the UI dropped the receiver.
        if self.events.send_blocking(event).is_err() {
            tracing::debug!("la UI cerró el canal de eventos");
        }
    }
}

/// Worker entry point: wait for `Spawn`, then run one connection at a time.
async fn worker_main(
    commands: async_channel::Receiver<AgentCommand>,
    events: async_channel::Sender<AgentEvent>,
) {
    loop {
        let Ok(command) = commands.recv().await else {
            return;
        };
        match command {
            AgentCommand::Spawn { launch, cwd } => {
                if let Err(error) = run_agent(&launch, &cwd, &commands, &events).await {
                    let _ = events.send(AgentEvent::Error(error.to_string())).await;
                }
            }
            AgentCommand::Shutdown => return,
            other => {
                let _ = events
                    .send(AgentEvent::Error(format!(
                        "comando {other:?} recibido sin agente activo"
                    )))
                    .await;
            }
        }
    }
}

/// Spawn the process, wire the transport and run the connection until the
/// command loop finishes or the child dies.
async fn run_agent(
    launch: &LaunchSpec,
    cwd: &Path,
    commands: &async_channel::Receiver<AgentCommand>,
    events: &async_channel::Sender<AgentEvent>,
) -> Result<()> {
    let mut command = tokio::process::Command::new(&launch.program);
    command
        .args(&launch.args)
        .current_dir(cwd)
        .envs(&launch.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if std::env::var_os("NO_BROWSER").is_none() && !launch.env.contains_key("NO_BROWSER") {
        // Agents must not try to open a browser from a headless child.
        command.env("NO_BROWSER", "1");
    }
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

    // Drain stderr into events; never let it fill the pipe.
    let stderr_events = events.clone();
    let stderr_task = tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if stderr_events.send(AgentEvent::Stderr(line)).await.is_err() {
                break;
            }
        }
    });

    let shared = Arc::new(Shared {
        events: events.clone(),
        permission_slots: PermissionSlots::default(),
        next_permission_id: AtomicU64::new(1),
    });

    let transport = build_transport(stdin, stdout);
    let connection = {
        let shared_notify = shared.clone();
        let shared_permission = shared.clone();
        let shared_read = shared.clone();
        let shared_write = shared.clone();
        let shared_main = shared.clone();
        let commands = commands.clone();
        agent_client_protocol::Client
            .builder()
            .name(CLIENT_NAME)
            .on_receive_notification(
                async move |notification: SessionNotification, _cx| {
                    shared_notify.emit(AgentEvent::Update {
                        session_id: notification.session_id,
                        update: Box::new(notification.update),
                    });
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
            .connect_with(transport, async move |cx: ConnectionTo<_>| {
                command_loop(&shared_main, &commands, &cx).await
            })
    };

    let result = tokio::select! {
        result = connection => result.map_err(AcpError::from),
        status = child.wait() => {
            let code = status.ok().and_then(|status| status.code());
            let _ = events.send(AgentEvent::Exited { code }).await;
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
    let _ = events.send(AgentEvent::Exited { code }).await;
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

fn handle_permission(
    shared: &Arc<Shared>,
    request: RequestPermissionRequest,
    responder: agent_client_protocol::Responder<RequestPermissionResponse>,
    cx: &ConnectionTo<agent_client_protocol::Agent>,
) -> std::result::Result<(), agent_client_protocol::Error> {
    let id = PermissionRequestId(shared.next_permission_id.fetch_add(1, Ordering::Relaxed));
    let (tx, rx) = oneshot::channel();
    if let Ok(mut slots) = shared.permission_slots.lock() {
        slots.insert(id, tx);
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

fn handle_read(
    shared: &Arc<Shared>,
    request: ReadTextFileRequest,
    responder: agent_client_protocol::Responder<ReadTextFileResponse>,
    cx: &ConnectionTo<agent_client_protocol::Agent>,
) -> std::result::Result<(), agent_client_protocol::Error> {
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
                .client_info(Implementation::new(CLIENT_NAME, CLIENT_VERSION)),
        )
        .block_task()
        .await?;

    let auth_methods: Vec<AuthMethod> = initialize.auth_methods.clone();
    shared.emit(AgentEvent::Connected {
        agent_info: initialize.agent_info.clone(),
        auth_methods: auth_methods.clone(),
        capabilities: Box::new(initialize.agent_capabilities.clone()),
    });

    while let Ok(command) = commands.recv().await {
        match command {
            AgentCommand::Shutdown => break,
            AgentCommand::Spawn { .. } => {
                shared.emit(AgentEvent::Error(
                    "ya hay un agente activo en esta conexión".to_string(),
                ));
            }
            AgentCommand::NewSession { cwd } => {
                match cx
                    .send_request(NewSessionRequest::new(cwd))
                    .block_task()
                    .await
                {
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
                    Err(error) => shared.emit(AgentEvent::Error(error.message)),
                }
            }
            AgentCommand::Authenticate { method_id } => {
                if let Err(error) = cx
                    .send_request(AuthenticateRequest::new(method_id))
                    .block_task()
                    .await
                {
                    shared.emit(AgentEvent::Error(error.message));
                }
            }
            AgentCommand::Prompt { session_id, blocks } => {
                // Run the turn off the command loop so `Cancel` still works.
                let shared = shared.clone();
                let cx_prompt = cx.clone();
                cx.spawn(async move {
                    let request = PromptRequest::new(session_id.clone(), blocks);
                    match cx_prompt.send_request(request).block_task().await {
                        Ok(response) => shared.emit(AgentEvent::TurnEnded {
                            session_id,
                            stop_reason: response.stop_reason,
                        }),
                        Err(error) => shared.emit(AgentEvent::Error(error.message)),
                    }
                    Ok(())
                })?;
            }
            AgentCommand::Cancel { session_id } => {
                cx.send_notification(CancelNotification::new(session_id))?;
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
                    shared.emit(AgentEvent::Error(error.message));
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
                    shared.emit(AgentEvent::Error(error.message));
                }
            }
            AgentCommand::RespondPermission { id, outcome } => {
                let sender = shared
                    .permission_slots
                    .lock()
                    .map(|mut slots| slots.remove(&id))
                    .unwrap_or(None);
                match sender {
                    Some(sender) => {
                        let _ = sender.send(outcome);
                    }
                    None => shared.emit(AgentEvent::Error(format!(
                        "no hay un permiso pendiente con id {id}"
                    ))),
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
