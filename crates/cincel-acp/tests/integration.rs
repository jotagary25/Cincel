//! End-to-end tests of `AgentConnection` against the fake agent in
//! `tests/fake_agent/`. No real agent (claude/codex) is ever spawned here.

use std::path::Path;
use std::time::Duration;

use cincel_acp::acp::schema::v1::{
    ContentBlock, SessionId, SessionUpdate, StopReason, ToolCallContent, ToolKind,
};
use cincel_acp::{
    AgentCommand, AgentConnection, AgentEvent, AutoAnswer, LaunchSpec, McpServerSpec,
    PermissionOutcome, PermissionPolicy, PromptBlock, pick_allow_option,
};

const FAKE_AGENT: &str = env!("CARGO_BIN_EXE_cincel-acp-fake-agent");
const TIMEOUT: Duration = Duration::from_secs(30);

/// Labels of the events observed, in order, so tests can assert a sequence.
#[derive(Debug, PartialEq, Eq)]
enum Seen {
    Connected,
    SessionCreated,
    MessageChunk(String),
    ToolCall,
    ToolCallUpdate,
    Permission,
    FsRead,
    FsWrite,
    TurnEnded(String),
}

struct Harness {
    connection: AgentConnection,
    seen: Vec<Seen>,
}

impl Harness {
    fn new(project_root: &Path) -> Self {
        Self {
            connection: AgentConnection::start(project_root.to_path_buf()),
            seen: Vec::new(),
        }
    }

    async fn spawn(&self, cwd: &Path) {
        self.connection
            .send(AgentCommand::Spawn {
                launch: LaunchSpec::new(FAKE_AGENT, Vec::new()),
                cwd: cwd.to_path_buf(),
            })
            .await
            .expect("spawn");
    }

    /// Drive events until `Connected`, then create a session and return its id.
    async fn connect_and_new_session(&mut self, cwd: &Path) -> SessionId {
        loop {
            match self.connection.recv().await.expect("evento") {
                AgentEvent::Connected { agent_info, .. } => {
                    assert_eq!(agent_info.expect("agent_info").name, "cincel-fake-agent");
                    self.seen.push(Seen::Connected);
                    self.connection
                        .send(AgentCommand::NewSession {
                            cwd: cwd.to_path_buf(),
                            mcp_servers: Vec::new(),
                        })
                        .await
                        .expect("new session");
                }
                AgentEvent::SessionCreated { session_id, .. } => {
                    self.seen.push(Seen::SessionCreated);
                    return session_id;
                }
                AgentEvent::Stderr(line) => eprintln!("[fake-agent] {line}"),
                other => panic!("evento inesperado antes de SessionCreated: {other:?}"),
            }
        }
    }

    async fn prompt(&self, session_id: SessionId, blocks: Vec<PromptBlock>) {
        self.connection
            .send(AgentCommand::Prompt {
                session_id,
                blocks,
                feedback: None,
            })
            .await
            .expect("prompt");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fake_agent_full_turn_emits_expected_events() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let target = cwd.join("demo.txt");
    std::fs::write(&target, "alfa\nbeta\n").expect("seed file");

    let mut harness = Harness::new(&cwd);
    harness.spawn(&cwd).await;

    let result = tokio::time::timeout(TIMEOUT, drive_full_turn(&mut harness, &cwd, &target)).await;
    harness.connection.shutdown();
    result.expect("el turno no terminó a tiempo");

    let seen = harness.seen;
    assert_eq!(seen[0], Seen::Connected, "secuencia: {seen:?}");
    assert_eq!(seen[1], Seen::SessionCreated, "secuencia: {seen:?}");
    assert_eq!(
        seen[2],
        Seen::MessageChunk("Hola".to_string()),
        "secuencia: {seen:?}"
    );
    assert_eq!(
        seen[3],
        Seen::MessageChunk(" mundo".to_string()),
        "secuencia: {seen:?}"
    );
    assert_eq!(seen[4], Seen::ToolCall, "secuencia: {seen:?}");
    assert_eq!(seen[5], Seen::Permission, "secuencia: {seen:?}");
    assert_eq!(seen[6], Seen::FsRead, "secuencia: {seen:?}");
    assert_eq!(seen[7], Seen::FsWrite, "secuencia: {seen:?}");
    assert!(seen.contains(&Seen::ToolCallUpdate), "secuencia: {seen:?}");
    assert_eq!(
        seen.last(),
        Some(&Seen::TurnEnded("end_turn".to_string())),
        "secuencia: {seen:?}"
    );

    let written = std::fs::read_to_string(&target).expect("leer demo.txt");
    assert_eq!(written, "alfa\nbeta\nescrito por el agente falso\n");
}

/// Consume events until the turn ends, answering everything the way the
/// `chat` example does.
async fn drive_full_turn(harness: &mut Harness, cwd: &Path, target: &Path) {
    let session_id = harness.connect_and_new_session(cwd).await;
    harness
        .prompt(
            session_id.clone(),
            vec![PromptBlock::Text("hola".to_string())],
        )
        .await;

    loop {
        let event = harness.connection.recv().await.expect("evento");
        match event {
            AgentEvent::Update { update, .. } => match *update {
                SessionUpdate::AgentMessageChunk(chunk) => {
                    if let ContentBlock::Text(text) = chunk.content {
                        harness.seen.push(Seen::MessageChunk(text.text));
                    }
                }
                SessionUpdate::ToolCall(call) => {
                    assert_eq!(call.kind, ToolKind::Edit);
                    let diff = call
                        .content
                        .iter()
                        .find_map(|item| match item {
                            ToolCallContent::Diff(diff) => Some(diff),
                            _ => None,
                        })
                        .expect("diff en el tool call");
                    assert_eq!(diff.path.as_path(), target);
                    assert_eq!(diff.old_text.as_deref(), Some("alfa\nbeta\n"));
                    harness.seen.push(Seen::ToolCall);
                }
                SessionUpdate::ToolCallUpdate(_) => harness.seen.push(Seen::ToolCallUpdate),
                _ => {}
            },
            AgentEvent::PermissionRequest {
                tool_call,
                options,
                reply,
                ..
            } => {
                harness.seen.push(Seen::Permission);
                let kind = tool_call.fields.kind.unwrap_or_default();
                assert_eq!(
                    PermissionPolicy::default().decide(kind, &[target]),
                    AutoAnswer::Allow
                );
                let option = pick_allow_option(&options).expect("opción allow");
                assert!(reply.respond(PermissionOutcome::Selected(option.option_id.clone())));
            }
            AgentEvent::FsRead { path, reply, .. } => {
                harness.seen.push(Seen::FsRead);
                let content = std::fs::read_to_string(&path).expect("leer");
                let _ = reply.send(Ok(content));
            }
            AgentEvent::FsWrite {
                path,
                content,
                reply,
                ..
            } => {
                harness.seen.push(Seen::FsWrite);
                std::fs::write(&path, content).expect("escribir");
                let _ = reply.send(Ok(()));
            }
            AgentEvent::TurnEnded { stop_reason, .. } => {
                let label = match stop_reason {
                    StopReason::EndTurn => "end_turn",
                    StopReason::Cancelled => "cancelled",
                    other => {
                        harness.seen.push(Seen::TurnEnded(format!("{other:?}")));
                        return;
                    }
                };
                harness.seen.push(Seen::TurnEnded(label.to_string()));
                return;
            }
            AgentEvent::Error { message, .. } => panic!("error del agente: {message}"),
            AgentEvent::Exited { code, .. } => panic!("el agente salió antes de tiempo: {code:?}"),
            AgentEvent::Stderr(line) => eprintln!("[fake-agent] {line}"),
            AgentEvent::AuthRequired { .. } => panic!("el agente falso no pide auth"),
            other => panic!("evento inesperado: {other:?}"),
        }
    }
}

/// A non-JSON banner on stdout must not break the connection: the transport
/// drops the line and the ACP frames that follow are still parsed.
#[tokio::test(flavor = "multi_thread")]
async fn tolerates_non_json_stdout_lines() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();

    // `sh -c` prints a banner and then execs the real agent on the same stdio.
    let script = format!("echo 'banner no-JSON'; exec {FAKE_AGENT}");
    let launch = LaunchSpec::new("sh", vec!["-c".to_string(), script]);

    let mut connection = AgentConnection::start(cwd.clone());
    connection
        .send(AgentCommand::Spawn {
            launch,
            cwd: cwd.clone(),
        })
        .await
        .expect("spawn");

    let connected = tokio::time::timeout(TIMEOUT, async {
        loop {
            match connection.recv().await.expect("evento") {
                AgentEvent::Connected { agent_info, .. } => return agent_info,
                AgentEvent::Stderr(line) => eprintln!("[fake-agent] {line}"),
                other => panic!("evento inesperado antes de Connected: {other:?}"),
            }
        }
    })
    .await
    .expect("no llegó Connected");

    connection.shutdown();
    assert_eq!(
        connected.expect("agent_info").name,
        "cincel-fake-agent",
        "la conexión sobrevivió a la línea no-JSON"
    );
}

/// Killing the agent process surfaces as `Exited` so the UI can relaunch it.
#[tokio::test(flavor = "multi_thread")]
async fn agent_death_produces_exited() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let launch = LaunchSpec::new("sh", vec!["-c".to_string(), "exit 7".to_string()]);

    let mut connection = AgentConnection::start(workspace.path().to_path_buf());
    connection
        .send(AgentCommand::Spawn {
            launch,
            cwd: workspace.path().to_path_buf(),
        })
        .await
        .expect("spawn");

    let code = tokio::time::timeout(TIMEOUT, async {
        loop {
            match connection.recv().await.expect("evento") {
                AgentEvent::Exited { code, .. } => return code,
                AgentEvent::Error { .. } | AgentEvent::Stderr(_) => {}
                other => panic!("evento inesperado: {other:?}"),
            }
        }
    })
    .await
    .expect("no llegó Exited");

    connection.shutdown();
    assert_eq!(code, Some(7));
}

/// `mcpServers` passed to `NewSession` reach `session/new` and the agent can
/// see them (`docs/specs/modulos/acp.md` §Sesión).
#[tokio::test(flavor = "multi_thread")]
async fn mcp_servers_are_forwarded_to_session_new() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mut harness = Harness::new(&cwd);
    harness.spawn(&cwd).await;

    let outcome = tokio::time::timeout(TIMEOUT, async {
        let session_id = loop {
            match harness.connection.recv().await.expect("evento") {
                AgentEvent::Connected { .. } => {
                    harness
                        .connection
                        .send(AgentCommand::NewSession {
                            cwd: cwd.clone(),
                            mcp_servers: vec![McpServerSpec::new("fs", "/usr/bin/mcp-fs")],
                        })
                        .await
                        .expect("new session");
                }
                AgentEvent::SessionCreated { session_id, .. } => break session_id,
                other => panic!("evento inesperado: {other:?}"),
            }
        };
        harness
            .prompt(session_id, vec![PromptBlock::Text("echo-mcp".to_string())])
            .await;
        loop {
            if let AgentEvent::Update { update, .. } =
                harness.connection.recv().await.expect("evento")
                && let SessionUpdate::AgentMessageChunk(chunk) = *update
                && let ContentBlock::Text(text) = chunk.content
            {
                return text.text;
            }
        }
    })
    .await
    .expect("timeout");

    harness.connection.shutdown();
    assert_eq!(outcome, "mcp_servers=fs");
}

/// `PromptBlock::ResourceLink` (an `@mention`) reaches the agent as
/// `ContentBlock::ResourceLink` with a `file://` URI.
#[tokio::test(flavor = "multi_thread")]
async fn resource_link_blocks_reach_the_agent() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mentioned = cwd.join("notes.md");
    std::fs::write(&mentioned, "hola").expect("seed");

    let mut harness = Harness::new(&cwd);
    harness.spawn(&cwd).await;
    let session_id = harness.connect_and_new_session(&cwd).await;
    harness
        .prompt(
            session_id,
            vec![
                PromptBlock::Text("echo-mention".to_string()),
                PromptBlock::mention(&mentioned),
            ],
        )
        .await;

    let text = tokio::time::timeout(TIMEOUT, async {
        loop {
            if let AgentEvent::Update { update, .. } =
                harness.connection.recv().await.expect("evento")
                && let SessionUpdate::AgentMessageChunk(chunk) = *update
                && let ContentBlock::Text(text) = chunk.content
            {
                return text.text;
            }
        }
    })
    .await
    .expect("timeout");

    harness.connection.shutdown();
    assert_eq!(
        text,
        format!("vi file://{} (notes.md)", mentioned.display())
    );
}

/// `session/load` replays history as `Update` events before completing with
/// `SessionCreated` (`docs/specs/modulos/acp.md` §Sesión).
#[tokio::test(flavor = "multi_thread")]
async fn session_load_replays_history_then_completes() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mut harness = Harness::new(&cwd);
    harness.spawn(&cwd).await;

    let (replayed, completed) = tokio::time::timeout(TIMEOUT, async {
        loop {
            if let AgentEvent::Connected { capabilities, .. } =
                harness.connection.recv().await.expect("evento")
            {
                assert!(
                    capabilities.load_session,
                    "el agente falso anuncia loadSession"
                );
                break;
            }
        }
        harness
            .connection
            .send(AgentCommand::LoadSession {
                session_id: SessionId::new("loaded-1"),
                cwd: cwd.clone(),
                mcp_servers: Vec::new(),
            })
            .await
            .expect("load session");

        let mut replayed = Vec::new();
        loop {
            match harness.connection.recv().await.expect("evento") {
                AgentEvent::Update { update, .. } => {
                    if let SessionUpdate::AgentMessageChunk(chunk) = *update
                        && let ContentBlock::Text(text) = chunk.content
                    {
                        replayed.push(text.text);
                    }
                }
                AgentEvent::SessionCreated { session_id, .. } => {
                    return (replayed, session_id);
                }
                other => panic!("evento inesperado: {other:?}"),
            }
        }
    })
    .await
    .expect("timeout");

    harness.connection.shutdown();
    assert_eq!(
        replayed,
        vec![
            "primer mensaje replayado".to_string(),
            "segundo mensaje replayado".to_string()
        ]
    );
    assert_eq!(completed.0.as_ref(), "loaded-1");
}

/// `session/resume` completes with `SessionCreated` and does *not* replay
/// history first.
#[tokio::test(flavor = "multi_thread")]
async fn session_resume_completes_without_replay() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mut harness = Harness::new(&cwd);
    harness.spawn(&cwd).await;

    let first_event_after_resume = tokio::time::timeout(TIMEOUT, async {
        loop {
            if let AgentEvent::Connected { .. } = harness.connection.recv().await.expect("evento") {
                break;
            }
        }
        harness
            .connection
            .send(AgentCommand::ResumeSession {
                session_id: SessionId::new("resumed-1"),
                cwd: cwd.clone(),
                mcp_servers: Vec::new(),
            })
            .await
            .expect("resume session");
        harness.connection.recv().await.expect("evento")
    })
    .await
    .expect("timeout");

    harness.connection.shutdown();
    assert!(
        matches!(first_event_after_resume, AgentEvent::SessionCreated { .. }),
        "session/resume no debe reproducir historial: {first_event_after_resume:?}"
    );
}

/// `Cancel` answers `cancelled` to a never-ending prompt and marks the still
/// `in_progress` tool call as cancelled (`docs/specs/modulos/acp.md`
/// §Cancelación).
#[tokio::test(flavor = "multi_thread")]
async fn cancel_stops_a_never_ending_prompt_and_flags_tool_calls() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mut harness = Harness::new(&cwd);
    harness.spawn(&cwd).await;
    let session_id = harness.connect_and_new_session(&cwd).await;
    harness
        .prompt(
            session_id.clone(),
            vec![PromptBlock::Text("never-end".to_string())],
        )
        .await;

    let (cancelled_ids, stop_reason) = tokio::time::timeout(TIMEOUT, async {
        // Wait for the tool call to start, then cancel.
        loop {
            if let AgentEvent::Update { update, .. } =
                harness.connection.recv().await.expect("evento")
                && let SessionUpdate::ToolCall(call) = *update
            {
                assert_eq!(call.tool_call_id.0.as_ref(), "long-task");
                break;
            }
        }
        harness
            .connection
            .send(AgentCommand::Cancel {
                session_id: session_id.clone(),
            })
            .await
            .expect("cancel");

        let mut cancelled_ids = Vec::new();
        loop {
            match harness.connection.recv().await.expect("evento") {
                AgentEvent::ToolCallsCancelled { ids, .. } => cancelled_ids = ids,
                AgentEvent::TurnEnded { stop_reason, .. } => return (cancelled_ids, stop_reason),
                other => panic!("evento inesperado: {other:?}"),
            }
        }
    })
    .await
    .expect("timeout");

    harness.connection.shutdown();
    assert_eq!(stop_reason, StopReason::Cancelled);
    assert_eq!(cancelled_ids.len(), 1);
    assert_eq!(cancelled_ids[0].0.as_ref(), "long-task");
}

/// The fixed permission policy never auto-answers an `execute` request: it
/// reaches the user (here, the test), who answers it end to end.
#[tokio::test(flavor = "multi_thread")]
async fn execute_permission_reaches_the_user() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mut harness = Harness::new(&cwd);
    harness.spawn(&cwd).await;
    let session_id = harness.connect_and_new_session(&cwd).await;
    harness
        .prompt(
            session_id,
            vec![PromptBlock::Text("execute-permission".to_string())],
        )
        .await;

    let policy = PermissionPolicy::default();
    let stop_reason = tokio::time::timeout(TIMEOUT, async {
        loop {
            match harness.connection.recv().await.expect("evento") {
                AgentEvent::PermissionRequest {
                    tool_call,
                    options,
                    reply,
                    ..
                } => {
                    let kind = tool_call.fields.kind.unwrap_or_default();
                    assert_eq!(kind, ToolKind::Execute);
                    let paths: Vec<&Path> = Vec::new();
                    assert_eq!(policy.decide(kind, &paths), AutoAnswer::Ask);
                    let option = pick_allow_option(&options).expect("allow");
                    reply.respond(PermissionOutcome::Selected(option.option_id.clone()));
                }
                AgentEvent::TurnEnded { stop_reason, .. } => return stop_reason,
                other => panic!("evento inesperado: {other:?}"),
            }
        }
    })
    .await
    .expect("timeout");

    harness.connection.shutdown();
    assert_eq!(stop_reason, StopReason::EndTurn);
}

/// A `fs/write_text_file` request outside `project_root` never reaches the UI
/// as `AgentEvent::FsWrite`: the connection rejects it with `invalid_params`
/// straight away (`docs/specs/modulos/acp.md` §Handlers).
#[tokio::test(flavor = "multi_thread")]
async fn fs_write_outside_project_root_is_sandboxed() {
    let project = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("otro tempdir");
    let outside_file = outside.path().join("secret.txt");

    let mut harness = Harness::new(project.path());
    harness.spawn(project.path()).await;
    let session_id = harness.connect_and_new_session(project.path()).await;
    harness
        .prompt(
            session_id,
            vec![
                PromptBlock::Text("sandbox-write".to_string()),
                PromptBlock::Text(outside_file.display().to_string()),
            ],
        )
        .await;

    let outcome = tokio::time::timeout(TIMEOUT, async {
        loop {
            match harness.connection.recv().await.expect("evento") {
                AgentEvent::FsWrite { .. } => {
                    panic!("fs/write_text_file fuera del proyecto no debe llegar a la UI")
                }
                AgentEvent::Update { update, .. } => {
                    if let SessionUpdate::AgentMessageChunk(chunk) = *update
                        && let ContentBlock::Text(text) = chunk.content
                    {
                        return text.text;
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .expect("timeout");

    harness.connection.shutdown();
    assert!(outcome.starts_with("rechazado:"), "outcome: {outcome}");
    assert!(
        !outside_file.exists(),
        "el archivo fuera del proyecto no debe crearse"
    );
}

/// `AgentEvent::Exited` carries a bounded tail of the agent's stderr.
#[tokio::test(flavor = "multi_thread")]
async fn stderr_tail_is_bounded_on_exit() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mut harness = Harness::new(&cwd);
    harness.spawn(&cwd).await;
    let session_id = harness.connect_and_new_session(&cwd).await;
    harness
        .prompt(
            session_id,
            vec![PromptBlock::Text("loud-stderr".to_string())],
        )
        .await;

    let stderr_tail = tokio::time::timeout(TIMEOUT, async {
        loop {
            match harness.connection.recv().await.expect("evento") {
                AgentEvent::TurnEnded { .. } => {
                    harness
                        .connection
                        .send(AgentCommand::Shutdown)
                        .await
                        .expect("shutdown command");
                }
                AgentEvent::Exited { stderr_tail, .. } => return stderr_tail,
                _ => {}
            }
        }
    })
    .await
    .expect("timeout");

    harness.connection.shutdown();
    assert!(
        stderr_tail.len() <= 64 * 1024 + 200,
        "el tail debe estar acotado: {} bytes",
        stderr_tail.len()
    );
    assert!(stderr_tail.contains("linea de ruido numero 1999"));
    assert!(
        !stderr_tail.contains("numero 0 para"),
        "las líneas más viejas deben haberse descartado"
    );
}

/// `agentFileChangeReport`, negotiated via `_meta` on `initialize`/`prompt`,
/// surfaces as `AgentEvent::FileChangeReport`.
#[tokio::test(flavor = "multi_thread")]
async fn file_change_report_is_parsed_from_session_info_update() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mut harness = Harness::new(&cwd);
    harness.spawn(&cwd).await;
    let session_id = harness.connect_and_new_session(&cwd).await;
    harness
        .prompt(
            session_id,
            vec![PromptBlock::Text("file-change-report".to_string())],
        )
        .await;

    let report = tokio::time::timeout(TIMEOUT, async {
        loop {
            match harness.connection.recv().await.expect("evento") {
                AgentEvent::FileChangeReport { report, .. } => return report,
                AgentEvent::TurnEnded { .. } => {
                    panic!("el turno terminó antes de recibir el reporte")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("timeout");

    harness.connection.shutdown();
    assert_eq!(
        report.paths,
        vec![std::path::PathBuf::from("/workspace/src/App.ts")]
    );
    assert!(!report.declared_complete);
}

// ------------------------------------------------------------------ Etapa 4

/// Spawns the fake agent with `launch` on a connection built with `env`, and
/// waits for `Connected`, returning the announced capabilities.
async fn connect_with(
    connection: &AgentConnection,
    launch: LaunchSpec,
    cwd: &Path,
) -> Box<cincel_acp::acp::schema::v1::AgentCapabilities> {
    connection
        .send(AgentCommand::Spawn {
            launch,
            cwd: cwd.to_path_buf(),
        })
        .await
        .expect("spawn");
    loop {
        match connection.recv().await.expect("evento") {
            AgentEvent::Connected { capabilities, .. } => return capabilities,
            AgentEvent::Stderr(line) => eprintln!("[fake-agent] {line}"),
            AgentEvent::AuthStatus { .. } => {}
            other => panic!("evento inesperado antes de Connected: {other:?}"),
        }
    }
}

async fn new_session(connection: &AgentConnection, cwd: &Path) -> SessionId {
    connection
        .send(AgentCommand::NewSession {
            cwd: cwd.to_path_buf(),
            mcp_servers: Vec::new(),
        })
        .await
        .expect("new session");
    loop {
        match connection.recv().await.expect("evento") {
            AgentEvent::SessionCreated { session_id, .. } => return session_id,
            AgentEvent::AuthStatus { .. } | AgentEvent::Stderr(_) => {}
            other => panic!("evento inesperado antes de SessionCreated: {other:?}"),
        }
    }
}

/// Sends `echo-env NAME` and returns the agent's `NAME=value` answer.
async fn echo_env(connection: &AgentConnection, session_id: &SessionId, name: &str) -> String {
    connection
        .send(AgentCommand::Prompt {
            session_id: session_id.clone(),
            blocks: vec![
                PromptBlock::Text("echo-env".to_string()),
                PromptBlock::Text(name.to_string()),
            ],
            feedback: None,
        })
        .await
        .expect("prompt");
    let mut answer = None;
    loop {
        match connection.recv().await.expect("evento") {
            AgentEvent::Update { update, .. } => {
                if let SessionUpdate::AgentMessageChunk(chunk) = *update
                    && let ContentBlock::Text(text) = chunk.content
                {
                    answer = Some(text.text);
                }
            }
            AgentEvent::TurnEnded { .. } => return answer.expect("respuesta"),
            AgentEvent::Stderr(_) | AgentEvent::AuthStatus { .. } => {}
            other => panic!("evento inesperado: {other:?}"),
        }
    }
}

fn fake_launch() -> LaunchSpec {
    LaunchSpec::new(FAKE_AGENT, Vec::new())
}

/// `NO_BROWSER` is not injected by default any more, and a launch can still
/// opt in (spec 06 §5.1).
#[tokio::test(flavor = "multi_thread")]
async fn no_browser_is_opt_in_per_launch() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    // Strip any NO_BROWSER inherited from the shell running the tests so the
    // assertion is about what the crate injects.
    let env = cincel_acp::ProcessEnv::new().with_unset("NO_BROWSER");

    let (default_value, opt_in_value) = tokio::time::timeout(TIMEOUT, async {
        let mut plain = AgentConnection::start_with_env(cwd.clone(), env.clone());
        connect_with(&plain, fake_launch(), &cwd).await;
        let session = new_session(&plain, &cwd).await;
        let default_value = echo_env(&plain, &session, "NO_BROWSER").await;
        plain.shutdown();

        let mut opted = AgentConnection::start_with_env(cwd.clone(), env);
        connect_with(&opted, fake_launch().with_no_browser(), &cwd).await;
        let session = new_session(&opted, &cwd).await;
        let opt_in_value = echo_env(&opted, &session, "NO_BROWSER").await;
        opted.shutdown();
        (default_value, opt_in_value)
    })
    .await
    .expect("timeout");

    assert_eq!(default_value, "NO_BROWSER=<unset>");
    assert_eq!(opt_in_value, "NO_BROWSER=1");
}

/// `ProcessEnv` sets the profile variable, strips inherited variables
/// (exact and `PREFIX*`) and puts the private runtime first in `PATH`.
#[tokio::test(flavor = "multi_thread")]
async fn process_env_reaches_the_agent_process() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let profile = workspace.path().join("perfil");
    let env = cincel_acp::ProcessEnv::new()
        .with_set("CLAUDE_CONFIG_DIR", profile.display().to_string())
        .with_unset("CARGO_PKG_*")
        .with_unset("CARGO_MANIFEST_DIR")
        .with_path_prepend("/opt/cincel-test/node/bin");

    let mut connection = AgentConnection::start_with_env(cwd.clone(), env);
    let answers = tokio::time::timeout(TIMEOUT, async {
        connect_with(&connection, fake_launch(), &cwd).await;
        let session = new_session(&connection, &cwd).await;
        let mut answers = Vec::new();
        for name in [
            "CLAUDE_CONFIG_DIR",
            "CARGO_PKG_NAME",
            "CARGO_MANIFEST_DIR",
            "PATH",
        ] {
            answers.push(echo_env(&connection, &session, name).await);
        }
        answers
    })
    .await
    .expect("timeout");
    connection.shutdown();

    assert_eq!(
        answers[0],
        format!("CLAUDE_CONFIG_DIR={}", profile.display())
    );
    assert_eq!(answers[1], "CARGO_PKG_NAME=<unset>");
    assert_eq!(answers[2], "CARGO_MANIFEST_DIR=<unset>");
    assert!(
        answers[3].starts_with("PATH=/opt/cincel-test/node/bin"),
        "PATH: {}",
        answers[3]
    );
}

/// `Logout` sends ACP `logout` when announced and ends with
/// `LoggedOut { ok: true }`; the agent's status push follows.
#[tokio::test(flavor = "multi_thread")]
async fn logout_is_sent_when_announced() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let marker = workspace.path().join("logout-marker");
    let launch = fake_launch().with_env("FAKE_LOGOUT_MARKER", marker.display().to_string());

    let mut connection = AgentConnection::start(cwd.clone());
    let (ok, status_kind) = tokio::time::timeout(TIMEOUT, async {
        let capabilities = connect_with(&connection, launch, &cwd).await;
        assert!(cincel_acp::agent_supports_logout(&capabilities));
        connection.send(AgentCommand::Logout).await.expect("logout");
        let mut ok = None;
        let mut kind = None;
        while ok.is_none() || kind.is_none() {
            match connection.recv().await.expect("evento") {
                AgentEvent::LoggedOut { ok: value } => ok = Some(value),
                AgentEvent::AuthStatus { kind: value, .. } => kind = Some(value),
                AgentEvent::Stderr(_) => {}
                other => panic!("evento inesperado: {other:?}"),
            }
        }
        (ok.unwrap_or(false), kind)
    })
    .await
    .expect("timeout");
    connection.shutdown();

    assert!(ok);
    assert_eq!(status_kind, Some(cincel_acp::AuthStatusKind::None));
    assert!(marker.exists(), "el agente recibió logout");
}

/// Without `auth.logout` nothing is sent and the answer is `ok: false`.
#[tokio::test(flavor = "multi_thread")]
async fn logout_without_capability_is_refused_locally() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let marker = workspace.path().join("logout-marker");
    let launch = fake_launch()
        .with_env("FAKE_NO_LOGOUT", "1")
        .with_env("FAKE_LOGOUT_MARKER", marker.display().to_string());

    let mut connection = AgentConnection::start(cwd.clone());
    let ok = tokio::time::timeout(TIMEOUT, async {
        let capabilities = connect_with(&connection, launch, &cwd).await;
        assert!(!cincel_acp::agent_supports_logout(&capabilities));
        connection.send(AgentCommand::Logout).await.expect("logout");
        loop {
            match connection.recv().await.expect("evento") {
                AgentEvent::LoggedOut { ok } => return ok,
                AgentEvent::Stderr(_) => {}
                other => panic!("evento inesperado: {other:?}"),
            }
        }
    })
    .await
    .expect("timeout");
    connection.shutdown();

    assert!(!ok);
    assert!(!marker.exists(), "no se debe enviar logout sin capability");
}

/// A slow `authenticate` does not block the command loop: a `NewSession`
/// sent after it completes first, then `AuthSucceeded` arrives.
#[tokio::test(flavor = "multi_thread")]
async fn authenticate_runs_off_the_command_loop() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mut connection = AgentConnection::start(cwd.clone());

    let order = tokio::time::timeout(TIMEOUT, async {
        connect_with(&connection, fake_launch(), &cwd).await;
        connection
            .send(AgentCommand::Authenticate {
                method_id: "slow".to_string(),
            })
            .await
            .expect("authenticate");
        connection
            .send(AgentCommand::NewSession {
                cwd: cwd.clone(),
                mcp_servers: Vec::new(),
            })
            .await
            .expect("new session");
        let mut order = Vec::new();
        while order.len() < 3 {
            match connection.recv().await.expect("evento") {
                AgentEvent::SessionCreated { .. } => order.push("session".to_string()),
                AgentEvent::AuthSucceeded { method_id } => order.push(format!("auth:{method_id}")),
                AgentEvent::AuthStatus { account, .. } => order.push(format!(
                    "status:{}",
                    account
                        .and_then(|account| account.email)
                        .unwrap_or_default()
                )),
                AgentEvent::Stderr(_) => {}
                other => panic!("evento inesperado: {other:?}"),
            }
        }
        order
    })
    .await
    .expect("timeout");
    connection.shutdown();

    assert_eq!(order[0], "session", "orden: {order:?}");
    assert!(order.contains(&"auth:slow".to_string()), "orden: {order:?}");
    assert!(
        order.contains(&"status:auth@example.com".to_string()),
        "orden: {order:?}"
    );
}

/// A rejected `authenticate` surfaces as `AuthFailed` with the method id.
#[tokio::test(flavor = "multi_thread")]
async fn authenticate_failure_emits_auth_failed() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mut connection = AgentConnection::start(cwd.clone());

    let (method, message) = tokio::time::timeout(TIMEOUT, async {
        connect_with(&connection, fake_launch(), &cwd).await;
        connection
            .send(AgentCommand::Authenticate {
                method_id: "fail".to_string(),
            })
            .await
            .expect("authenticate");
        loop {
            match connection.recv().await.expect("evento") {
                AgentEvent::AuthFailed { method_id, message } => return (method_id, message),
                AgentEvent::Stderr(_) => {}
                other => panic!("evento inesperado: {other:?}"),
            }
        }
    })
    .await
    .expect("timeout");
    connection.shutdown();

    assert_eq!(method, "fail");
    assert!(!message.is_empty());
}

/// `_auth/status_update` pushed after `initialize` becomes
/// `AgentEvent::AuthStatus`, and the capability is visible on `Connected`.
#[tokio::test(flavor = "multi_thread")]
async fn auth_status_update_is_parsed() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let launch = fake_launch().with_env("FAKE_AUTH_EMAIL", "ana@example.com");
    let mut connection = AgentConnection::start(cwd.clone());

    let (supports, status) = tokio::time::timeout(TIMEOUT, async {
        connection
            .send(AgentCommand::Spawn {
                launch,
                cwd: cwd.clone(),
            })
            .await
            .expect("spawn");
        let mut supports = None;
        let mut status = None;
        while supports.is_none() || status.is_none() {
            match connection.recv().await.expect("evento") {
                AgentEvent::Connected { capabilities, .. } => {
                    supports = Some(cincel_acp::agent_supports_auth_status(&capabilities));
                }
                AgentEvent::AuthStatus {
                    kind,
                    label,
                    account,
                    ..
                } => status = Some((kind, label, account)),
                AgentEvent::Stderr(_) => {}
                other => panic!("evento inesperado: {other:?}"),
            }
        }
        (supports.unwrap_or(false), status.expect("status"))
    })
    .await
    .expect("timeout");
    connection.shutdown();

    assert!(supports);
    let (kind, label, account) = status;
    assert_eq!(kind, cincel_acp::AuthStatusKind::Account);
    assert_eq!(label, "Fake Max");
    let account = account.expect("account");
    assert_eq!(account.email.as_deref(), Some("ana@example.com"));
    assert_eq!(account.organization.as_deref(), Some("Fake Org"));
    assert_eq!(account.plan.as_deref(), Some("max"));
}

/// `elicitation/complete` closes a pending URL elicitation: the UI gets
/// `ElicitationCompleted` with the same id and the agent gets `accept`.
#[tokio::test(flavor = "multi_thread")]
async fn elicitation_complete_closes_the_pending_request() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mut connection = AgentConnection::start(cwd.clone());

    let (asked, completed, answer) = tokio::time::timeout(TIMEOUT, async {
        connect_with(&connection, fake_launch(), &cwd).await;
        let session = new_session(&connection, &cwd).await;
        connection
            .send(AgentCommand::Prompt {
                session_id: session,
                blocks: vec![PromptBlock::Text("url-elicitation".to_string())],
                feedback: None,
            })
            .await
            .expect("prompt");
        let mut asked = None;
        let mut completed = None;
        let mut answer = None;
        loop {
            match connection.recv().await.expect("evento") {
                // Deliberately left unanswered: the completion must close it.
                AgentEvent::Elicitation { id, .. } => asked = Some(id),
                AgentEvent::ElicitationCompleted { id, elicitation_id } => {
                    completed = Some((id, elicitation_id));
                }
                AgentEvent::Update { update, .. } => {
                    if let SessionUpdate::AgentMessageChunk(chunk) = *update
                        && let ContentBlock::Text(text) = chunk.content
                    {
                        answer = Some(text.text);
                    }
                }
                AgentEvent::TurnEnded { .. } => return (asked, completed, answer),
                AgentEvent::Stderr(_) => {}
                other => panic!("evento inesperado: {other:?}"),
            }
        }
    })
    .await
    .expect("timeout");
    connection.shutdown();

    let asked = asked.expect("llegó la elicitación");
    let (completed_id, elicitation_id) = completed.expect("llegó elicitation/complete");
    assert_eq!(asked, completed_id);
    assert_eq!(elicitation_id, "elic-1");
    assert_eq!(answer.as_deref(), Some("elicitation=accept"));
}

// ------------------------------------------------------------------ Etapa 5

/// Spawns the fake agent with the `FAKE_AUTH_*` variables in `vars`, asks
/// for `session/new` (or `session/load` with `load`) and returns the
/// `AuthRequired` event's message.
async fn auth_required_message_with(vars: &[(&str, &str)], load: bool) -> Option<String> {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let mut launch = fake_launch();
    for (name, value) in vars {
        launch = launch.with_env(*name, *value);
    }
    let mut connection = AgentConnection::start(cwd.clone());
    let message = tokio::time::timeout(TIMEOUT, async {
        connect_with(&connection, launch, &cwd).await;
        let command = if load {
            AgentCommand::LoadSession {
                session_id: SessionId::new("vieja"),
                cwd: cwd.clone(),
                mcp_servers: Vec::new(),
            }
        } else {
            AgentCommand::NewSession {
                cwd: cwd.clone(),
                mcp_servers: Vec::new(),
            }
        };
        connection.send(command).await.expect("session");
        loop {
            match connection.recv().await.expect("evento") {
                AgentEvent::AuthRequired { message, .. } => return message,
                AgentEvent::AuthStatus { .. } | AgentEvent::Stderr(_) => {}
                other => panic!("se esperaba AuthRequired: {other:?}"),
            }
        }
    })
    .await
    .expect("timeout");
    connection.shutdown();
    message
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_required_carries_the_agent_message() {
    assert_eq!(
        auth_required_message_with(&[("FAKE_AUTH_REQUIRED", "token revoked")], false)
            .await
            .as_deref(),
        Some("token revoked")
    );
    // Same for `session/load`.
    assert_eq!(
        auth_required_message_with(&[("FAKE_AUTH_REQUIRED", "token revoked")], true)
            .await
            .as_deref(),
        Some("token revoked")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_required_prefers_the_reason_in_data() {
    let message = auth_required_message_with(
        &[
            ("FAKE_AUTH_REQUIRED", "Authentication required"),
            (
                "FAKE_AUTH_REQUIRED_DATA",
                r#"{"reason":"refresh token expired"}"#,
            ),
        ],
        false,
    )
    .await;
    assert_eq!(message.as_deref(), Some("refresh token expired"));
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_required_without_message_is_none() {
    assert_eq!(
        auth_required_message_with(&[("FAKE_AUTH_REQUIRED", "")], false).await,
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_required_falls_back_to_the_logged_out_status() {
    let message = auth_required_message_with(
        &[
            ("FAKE_AUTH_REQUIRED", " "),
            ("FAKE_AUTH_LOGGED_OUT_DETAIL", "session expired upstream"),
        ],
        false,
    )
    .await;
    assert_eq!(message.as_deref(), Some("session expired upstream"));
}
