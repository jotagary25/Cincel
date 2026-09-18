//! End-to-end test of `AgentConnection` against the fake agent in
//! `tests/fake_agent/`.

use std::path::Path;
use std::time::Duration;

use asteroid_acp::acp::schema::v1::{
    ContentBlock, SessionUpdate, StopReason, TextContent, ToolCallContent, ToolKind,
};
use asteroid_acp::{
    AgentCommand, AgentConnection, AgentEvent, Autonomy, LaunchSpec, PermissionOutcome,
    pick_allow_option,
};

const FAKE_AGENT: &str = env!("CARGO_BIN_EXE_asteroid-acp-fake-agent");
const TIMEOUT: Duration = Duration::from_secs(30);

/// Labels of the events observed, in order, so the test can assert a sequence.
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
    fn new() -> Self {
        Self {
            connection: AgentConnection::start(),
            seen: Vec::new(),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fake_agent_full_turn_emits_expected_events() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let cwd = workspace.path().to_path_buf();
    let target = cwd.join("demo.txt");
    std::fs::write(&target, "alfa\nbeta\n").expect("seed file");

    let mut harness = Harness::new();
    let launch = LaunchSpec::new(FAKE_AGENT, Vec::new());

    harness
        .connection
        .send(AgentCommand::Spawn {
            launch,
            cwd: cwd.clone(),
        })
        .await
        .expect("spawn");

    let result = tokio::time::timeout(TIMEOUT, drive(&mut harness, &cwd, &target)).await;
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
async fn drive(harness: &mut Harness, cwd: &Path, target: &Path) {
    loop {
        let event = harness.connection.recv().await.expect("evento");
        match event {
            AgentEvent::Connected { agent_info, .. } => {
                let info = agent_info.expect("agent_info");
                assert_eq!(info.name, "asteroid-fake-agent");
                harness.seen.push(Seen::Connected);
                harness
                    .connection
                    .send(AgentCommand::NewSession {
                        cwd: cwd.to_path_buf(),
                    })
                    .await
                    .expect("new session");
            }
            AgentEvent::SessionCreated { session_id, .. } => {
                harness.seen.push(Seen::SessionCreated);
                harness
                    .connection
                    .send(AgentCommand::Prompt {
                        session_id,
                        blocks: vec![ContentBlock::Text(TextContent::new("hola"))],
                    })
                    .await
                    .expect("prompt");
            }
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
                    Autonomy::ReviewAfter.decide(kind, &[target]),
                    asteroid_acp::AutoAnswer::Allow
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
            AgentEvent::Error(message) => panic!("error del agente: {message}"),
            AgentEvent::Exited { code } => panic!("el agente salió antes de tiempo: {code:?}"),
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

    let mut connection = AgentConnection::start();
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

    let mut connection = AgentConnection::start();
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
                AgentEvent::Exited { code } => return code,
                AgentEvent::Error(_) | AgentEvent::Stderr(_) => {}
                other => panic!("evento inesperado: {other:?}"),
            }
        }
    })
    .await
    .expect("no llegó Exited");

    connection.shutdown();
    assert_eq!(code, Some(7));
}
