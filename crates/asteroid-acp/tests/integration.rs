//! End-to-end tests of `AgentConnection` against the fake agent in
//! `tests/fake_agent/`. No real agent (claude/codex) is ever spawned here.

use std::path::Path;
use std::time::Duration;

use asteroid_acp::acp::schema::v1::{
    ContentBlock, SessionId, SessionUpdate, StopReason, ToolCallContent, ToolKind,
};
use asteroid_acp::{
    AgentCommand, AgentConnection, AgentEvent, AutoAnswer, Autonomy, AutonomyMode, LaunchSpec,
    McpServerSpec, PermissionOutcome, PromptBlock, pick_allow_option,
};

const FAKE_AGENT: &str = env!("CARGO_BIN_EXE_asteroid-acp-fake-agent");
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
                    assert_eq!(agent_info.expect("agent_info").name, "asteroid-fake-agent");
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
                    Autonomy::new(AutonomyMode::ReviewAfter).decide(kind, &[target]),
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
        "asteroid-fake-agent",
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

/// `AlwaysApply` autonomy auto-answers an `execute` permission request end to
/// end, without any user interaction.
#[tokio::test(flavor = "multi_thread")]
async fn execute_permission_is_auto_answered_by_always_apply_autonomy() {
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

    let autonomy = Autonomy::new(AutonomyMode::AlwaysApply);
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
                    assert_eq!(autonomy.decide(kind, &paths), AutoAnswer::Allow);
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
