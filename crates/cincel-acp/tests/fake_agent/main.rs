//! Scripted ACP v1 agent used by the integration tests.
//!
//! Behavior is picked by the *first* text block of each prompt — a tiny DSL
//! so one binary can drive every scenario in `tests/integration.rs` without
//! extra CLI flags or env vars (those would have to cross `LaunchSpec`, which
//! is exactly what each test wants to exercise):
//!
//! * anything not matched below (in particular `"hola"`, used by the original
//!   Etapa 0 test) → two text chunks, a `tool_call` of kind `edit` carrying a
//!   `diff`, a permission request, `fs/read_text_file`, `fs/write_text_file`,
//!   a `tool_call_update` and `end_turn`.
//! * `"echo-mcp"` → reports the `mcpServers` names received at `session/new`.
//! * `"echo-mention"` → echoes the first `resource_link` block's `uri`/`name`.
//! * `"execute-permission"` → asks permission for an `execute` tool call
//!   (nothing else), to exercise the permission policy end to end.
//! * `"sandbox-write"` → the *second* text block is a path; the agent asks
//!   the client to `fs/write_text_file` it, to probe the sandbox.
//! * `"loud-stderr"` → writes ~2000 lines to stderr, then ends the turn.
//! * `"file-change-report"` → sends a `session_info_update` carrying
//!   `agentFileChangeReport` for the `requestId` in the prompt's `_meta`.
//! * `"never-end"` → opens an `in_progress` tool call and waits for
//!   `session/cancel` (or 30 s) before returning.
//! * `"echo-env"` → the *second* text block is a variable name; the agent
//!   answers `NAME=value` (or `NAME=<unset>`) from its own environment.
//! * `"url-elicitation"` → sends a url-mode `elicitation/create`
//!   (`elicitationId: "elic-1"`), then `elicitation/complete` 300 ms later
//!   without waiting, then reports the action it got back.
//!
//! `initialize` announces `auth.logout` (unless `FAKE_NO_LOGOUT` is set),
//! `agentCapabilities._meta.authStatus`, and the real `jetbrains.air` shape
//! at the top level of `_meta`; `"file-change-report"` only reports when the
//! client advertised `agentFileChangeReport` in `clientCapabilities._meta`.
//! With `FAKE_AUTH_EMAIL` set it pushes `_auth/status_update` right after
//! `initialize`. `authenticate` accepts any method id except `"fail"`
//! (`"slow"` waits 1.5 s first) and pushes a status update on success;
//! `logout` writes `FAKE_LOGOUT_MARKER` (when set) and pushes `kind: none`.
//!
//! `session/load` replays two canned messages before responding; `session/load`
//! and `session/resume` are both advertised as supported so the client can
//! exercise them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::UntypedMessage;
use agent_client_protocol::schema::v1::{
    AgentAuthCapabilities, AgentCapabilities, AuthenticateRequest, AuthenticateResponse,
    CancelNotification, CompleteElicitationNotification, ContentBlock, ContentChunk,
    CreateElicitationRequest, Diff, ElicitationAction, ElicitationSessionScope, ElicitationUrlMode,
    Implementation, InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse,
    LogoutCapabilities, LogoutRequest, LogoutResponse, McpServer, Meta, NewSessionRequest,
    NewSessionResponse, PermissionOption, PermissionOptionKind, PromptRequest, PromptResponse,
    ReadTextFileRequest, RequestPermissionRequest, ResumeSessionRequest, ResumeSessionResponse,
    SessionCapabilities, SessionId, SessionInfoUpdate, SessionNotification,
    SessionResumeCapabilities, SessionUpdate, StopReason, TextContent, ToolCall, ToolCallContent,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind, WriteTextFileRequest,
};
use agent_client_protocol::{Agent, ConnectionTo, Stdio};

/// File the default scenario pretends to edit, relative to the session cwd.
const TARGET_FILE: &str = "demo.txt";

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_session_id() -> SessionId {
    SessionId::new(format!(
        "fake-session-{}",
        SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

#[derive(Default, Clone)]
struct SessionState {
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
}

type Sessions = Arc<Mutex<HashMap<String, SessionState>>>;
type CancelFlags = Arc<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>>;

#[tokio::main]
async fn main() -> agent_client_protocol::Result<()> {
    let sessions: Sessions = Sessions::default();
    let cancel_flags: CancelFlags = CancelFlags::default();

    let sessions_new = sessions.clone();
    let sessions_load = sessions.clone();
    let sessions_resume = sessions.clone();
    let sessions_prompt = sessions.clone();
    let cancel_flags_notification = cancel_flags.clone();
    let cancel_flags_prompt = cancel_flags.clone();

    Agent
        .builder()
        .name("cincel-fake-agent")
        .on_receive_request(
            async move |request: InitializeRequest, responder, cx: ConnectionTo<_>| {
                CLIENT_ADVERTISED_AIR.store(
                    client_advertised_file_change_report(request.client_capabilities.meta.as_ref()),
                    Ordering::Relaxed,
                );
                let mut auth = AgentAuthCapabilities::new();
                if std::env::var_os("FAKE_NO_LOGOUT").is_none() {
                    auth = auth.logout(LogoutCapabilities::new());
                }
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(
                            AgentCapabilities::new()
                                .load_session(true)
                                .session_capabilities(
                                    SessionCapabilities::new()
                                        .resume(SessionResumeCapabilities::new()),
                                )
                                .auth(auth)
                                .meta(auth_status_capability_meta()),
                        )
                        .agent_info(Implementation::new("cincel-fake-agent", "0.1.0"))
                        .meta(file_change_report_capability_meta()),
                )?;
                // Like claude-agent-acp: push the identity right after
                // `initialize`, when the test asks for one.
                if let Ok(email) = std::env::var("FAKE_AUTH_EMAIL") {
                    send_auth_status(&cx, "account", "Fake Max", Some(&email))?;
                }
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: AuthenticateRequest, responder, cx: ConnectionTo<_>| {
                let method = request.method_id.0.to_string();
                cx.clone().spawn(async move {
                    match method.as_str() {
                        "fail" => responder.respond_with_error(
                            agent_client_protocol::Error::invalid_params()
                                .data(serde_json::json!("credenciales rechazadas")),
                        ),
                        other => {
                            if other == "slow" {
                                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                            }
                            responder.respond(AuthenticateResponse::new())?;
                            send_auth_status(&cx, "account", "Fake Max", Some("auth@example.com"))
                        }
                    }
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: LogoutRequest, responder, cx: ConnectionTo<_>| {
                if let Ok(marker) = std::env::var("FAKE_LOGOUT_MARKER") {
                    let _ = std::fs::write(marker, "logout");
                }
                responder.respond(LogoutResponse::new())?;
                send_auth_status(&cx, "none", "Not logged in", None)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _cx| {
                let session_id = next_session_id();
                if let Ok(mut sessions) = sessions_new.lock() {
                    sessions.insert(
                        session_id.0.to_string(),
                        SessionState {
                            cwd: request.cwd.clone(),
                            mcp_servers: request.mcp_servers.clone(),
                        },
                    );
                }
                responder.respond(NewSessionResponse::new(session_id))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: LoadSessionRequest, responder, cx: ConnectionTo<_>| {
                let session_id = request.session_id.clone();
                if let Ok(mut sessions) = sessions_load.lock() {
                    sessions.insert(
                        session_id.0.to_string(),
                        SessionState {
                            cwd: request.cwd.clone(),
                            mcp_servers: request.mcp_servers.clone(),
                        },
                    );
                }
                for text in ["primer mensaje replayado", "segundo mensaje replayado"] {
                    let update = SessionUpdate::AgentMessageChunk(ContentChunk::new(
                        ContentBlock::Text(TextContent::new(text)),
                    ));
                    let _ =
                        cx.send_notification(SessionNotification::new(session_id.clone(), update));
                }
                responder.respond(LoadSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: ResumeSessionRequest, responder, _cx| {
                if let Ok(mut sessions) = sessions_resume.lock() {
                    sessions.insert(
                        request.session_id.0.to_string(),
                        SessionState {
                            cwd: request.cwd.clone(),
                            mcp_servers: request.mcp_servers.clone(),
                        },
                    );
                }
                responder.respond(ResumeSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |notification: CancelNotification, _cx| {
                if let Ok(flags) = cancel_flags_notification.lock()
                    && let Some(notify) = flags.get(&notification.session_id.0.to_string())
                {
                    notify.notify_waiters();
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, cx: ConnectionTo<_>| {
                let sessions = sessions_prompt.clone();
                let cancel_flags = cancel_flags_prompt.clone();
                cx.clone().spawn(async move {
                    let result = run_turn(&cx, request, &sessions, &cancel_flags).await;
                    responder.respond(PromptResponse::new(result))
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}

fn first_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

async fn send_text(
    cx: &ConnectionTo<agent_client_protocol::Client>,
    session_id: &SessionId,
    text: &str,
) {
    let update = SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
        TextContent::new(text),
    )));
    let _ = cx.send_notification(SessionNotification::new(session_id.clone(), update));
}

const AIR_NAMESPACE: &str = "jetbrains";

/// Whether the client advertised `agentFileChangeReport` the way
/// claude-agent-acp checks it (`clientCapabilities._meta.jetbrains.air` with
/// an integer `version >= 1` and a `capabilities` string array).
static CLIENT_ADVERTISED_AIR: AtomicBool = AtomicBool::new(false);

fn client_advertised_file_change_report(meta: Option<&Meta>) -> bool {
    let Some(air) = meta
        .and_then(|meta| meta.get(AIR_NAMESPACE))
        .and_then(|jetbrains| jetbrains.get("air"))
    else {
        return false;
    };
    let version_ok = air
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|version| version >= 1);
    let listed = air
        .get("capabilities")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|list| {
            list.iter()
                .any(|value| value.as_str() == Some("agentFileChangeReport"))
        });
    version_ok && listed
}

/// Top-level `InitializeResponse._meta`, same shape as codex-acp.
fn file_change_report_capability_meta() -> Meta {
    let value = serde_json::json!({
        "steering": { "supported": true },
        AIR_NAMESPACE: { "air": {
            "version": 1,
            "capabilities": ["sessionFailure", "agentFileChangeReport", "asyncTasks"]
        } }
    });
    value.as_object().cloned().unwrap_or_default()
}

fn auth_status_capability_meta() -> Meta {
    serde_json::json!({ "authStatus": {} })
        .as_object()
        .cloned()
        .unwrap_or_default()
}

fn send_auth_status(
    cx: &ConnectionTo<agent_client_protocol::Client>,
    kind: &str,
    label: &str,
    email: Option<&str>,
) -> agent_client_protocol::Result<()> {
    let mut status = serde_json::json!({ "kind": kind, "label": label });
    if let Some(email) = email {
        status["account"] =
            serde_json::json!({ "email": email, "organization": "Fake Org", "plan": "max" });
    }
    cx.send_notification(UntypedMessage::new(
        "_auth/status_update",
        serde_json::json!({ "authStatus": status }),
    )?)
}

fn extract_report_request_id(meta: &Option<Meta>) -> Option<String> {
    meta.as_ref()?
        .get(AIR_NAMESPACE)?
        .get("air")?
        .get("agentFileChangeReportRequest")?
        .get("requestId")?
        .as_str()
        .map(str::to_string)
}

fn build_report_meta(request_id: &str, paths: &[&str]) -> Meta {
    let value = serde_json::json!({
        AIR_NAMESPACE: {
            "air": {
                "agentFileChangeReport": {
                    "version": 1,
                    "requestId": request_id,
                    "status": "reported",
                    "paths": paths,
                    "declaredComplete": false,
                    "truncated": false,
                    "uncertainty": "el agente falso solo simula el reporte"
                }
            }
        }
    });
    value.as_object().cloned().unwrap_or_default()
}

async fn run_turn(
    cx: &ConnectionTo<agent_client_protocol::Client>,
    request: PromptRequest,
    sessions: &Sessions,
    cancel_flags: &CancelFlags,
) -> StopReason {
    let session_id = request.session_id.clone();
    let state = sessions
        .lock()
        .ok()
        .and_then(|sessions| sessions.get(&session_id.0.to_string()).cloned())
        .unwrap_or_default();
    let script = first_text(&request.prompt);

    match script.as_str() {
        "echo-mcp" => {
            let names: Vec<String> = state
                .mcp_servers
                .iter()
                .map(|server| match server {
                    McpServer::Stdio(stdio) => stdio.name.clone(),
                    _ => "?".to_string(),
                })
                .collect();
            send_text(cx, &session_id, &format!("mcp_servers={}", names.join(","))).await;
            StopReason::EndTurn
        }
        "echo-mention" => {
            let link = request.prompt.iter().find_map(|block| match block {
                ContentBlock::ResourceLink(link) => Some(link.clone()),
                _ => None,
            });
            match link {
                Some(link) => {
                    send_text(cx, &session_id, &format!("vi {} ({})", link.uri, link.name)).await;
                }
                None => send_text(cx, &session_id, "sin resource_link").await,
            }
            StopReason::EndTurn
        }
        "execute-permission" => {
            let permission = RequestPermissionRequest::new(
                session_id.clone(),
                ToolCallUpdate::new(
                    "exec-1",
                    ToolCallUpdateFields::new()
                        .kind(ToolKind::Execute)
                        .title("Correr build"),
                ),
                vec![
                    PermissionOption::new(
                        "allow-once",
                        "Permitir una vez",
                        PermissionOptionKind::AllowOnce,
                    ),
                    PermissionOption::new(
                        "reject-once",
                        "Rechazar",
                        PermissionOptionKind::RejectOnce,
                    ),
                ],
            );
            match cx.send_request(permission).block_task().await {
                Ok(_) => StopReason::EndTurn,
                Err(_) => StopReason::Cancelled,
            }
        }
        "sandbox-write" => {
            let path = request
                .prompt
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text(text) => Some(text.text.clone()),
                    _ => None,
                })
                .nth(1)
                .unwrap_or_default();
            let write = WriteTextFileRequest::new(
                session_id.clone(),
                PathBuf::from(&path),
                "fuera del proyecto".to_string(),
            );
            let outcome = match cx.send_request(write).block_task().await {
                Ok(_) => "escrito".to_string(),
                Err(error) => format!("rechazado:{:?}", error.code),
            };
            send_text(cx, &session_id, &outcome).await;
            StopReason::EndTurn
        }
        "loud-stderr" => {
            for line in 0..2000 {
                eprintln!("linea de ruido numero {line} para llenar el buffer de diagnostico");
            }
            StopReason::EndTurn
        }
        "echo-env" => {
            let name = request
                .prompt
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text(text) => Some(text.text.clone()),
                    _ => None,
                })
                .nth(1)
                .unwrap_or_default();
            let value = std::env::var(&name).unwrap_or_else(|_| "<unset>".to_string());
            send_text(cx, &session_id, &format!("{name}={value}")).await;
            StopReason::EndTurn
        }
        "url-elicitation" => {
            let elicitation = CreateElicitationRequest::new(
                ElicitationUrlMode::new(
                    ElicitationSessionScope::new(session_id.clone()),
                    "elic-1",
                    "https://auth.example.test/device",
                ),
                "Entrá al enlace e ingresá el código",
            );
            let pending = cx.send_request(elicitation);
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let _ = cx.send_notification(CompleteElicitationNotification::new("elic-1"));
            match pending.block_task().await {
                Ok(response) => {
                    let label = match response.action {
                        ElicitationAction::Accept(_) => "accept",
                        ElicitationAction::Decline => "decline",
                        _ => "cancel",
                    };
                    send_text(cx, &session_id, &format!("elicitation={label}")).await;
                    StopReason::EndTurn
                }
                Err(_) => StopReason::Cancelled,
            }
        }
        "file-change-report" => {
            if !CLIENT_ADVERTISED_AIR.load(Ordering::Relaxed) {
                send_text(
                    cx,
                    &session_id,
                    "el cliente no anunció agentFileChangeReport",
                )
                .await;
                return StopReason::EndTurn;
            }
            if let Some(request_id) = extract_report_request_id(&request.meta) {
                let info = SessionInfoUpdate::new()
                    .meta(build_report_meta(&request_id, &["/workspace/src/App.ts"]));
                let _ = cx.send_notification(SessionNotification::new(
                    session_id.clone(),
                    SessionUpdate::SessionInfoUpdate(info),
                ));
            }
            StopReason::EndTurn
        }
        "never-end" => {
            let notify = Arc::new(tokio::sync::Notify::new());
            if let Ok(mut flags) = cancel_flags.lock() {
                flags.insert(session_id.0.to_string(), notify.clone());
            }
            let tool_call = ToolCall::new("long-task", "Tarea larga")
                .kind(ToolKind::Execute)
                .status(ToolCallStatus::InProgress);
            let _ = cx.send_notification(SessionNotification::new(
                session_id.clone(),
                SessionUpdate::ToolCall(tool_call),
            ));
            tokio::select! {
                () = notify.notified() => StopReason::Cancelled,
                () = tokio::time::sleep(std::time::Duration::from_secs(30)) => StopReason::EndTurn,
            }
        }
        _ => default_turn(cx, &session_id, &state.cwd).await,
    }
}

/// The original Etapa 0 scripted turn: two chunks, an `edit` tool call with a
/// diff, a permission request, `fs/read_text_file` + `fs/write_text_file`, a
/// `tool_call_update`, `end_turn`.
async fn default_turn(
    cx: &ConnectionTo<agent_client_protocol::Client>,
    session_id: &SessionId,
    cwd: &std::path::Path,
) -> StopReason {
    const TOOL_CALL_ID: &str = "call-1";
    let path = cwd.join(TARGET_FILE);

    for text in ["Hola", " mundo"] {
        send_text(cx, session_id, text).await;
    }

    let diff = Diff::new(path.clone(), "alfa\nBETA\ngamma\n").old_text("alfa\nbeta\n".to_string());
    let tool_call = ToolCall::new(TOOL_CALL_ID, "Editar demo.txt")
        .kind(ToolKind::Edit)
        .status(ToolCallStatus::Pending)
        .content(vec![ToolCallContent::Diff(diff)]);
    if cx
        .send_notification(SessionNotification::new(
            session_id.clone(),
            SessionUpdate::ToolCall(tool_call),
        ))
        .is_err()
    {
        return StopReason::Cancelled;
    }

    let permission = RequestPermissionRequest::new(
        session_id.clone(),
        ToolCallUpdate::new(
            TOOL_CALL_ID,
            ToolCallUpdateFields::new().kind(ToolKind::Edit),
        ),
        vec![
            PermissionOption::new(
                "allow-once",
                "Permitir una vez",
                PermissionOptionKind::AllowOnce,
            ),
            PermissionOption::new("reject-once", "Rechazar", PermissionOptionKind::RejectOnce),
        ],
    );
    match cx.send_request(permission).block_task().await {
        Ok(_) => {}
        Err(_) => return StopReason::Cancelled,
    }

    let read = ReadTextFileRequest::new(session_id.clone(), path.clone());
    let previous = match cx.send_request(read).block_task().await {
        Ok(response) => response.content,
        Err(_) => String::new(),
    };

    let write = WriteTextFileRequest::new(
        session_id.clone(),
        path.clone(),
        format!("{previous}escrito por el agente falso\n"),
    );
    if cx.send_request(write).block_task().await.is_err() {
        return StopReason::Cancelled;
    }

    let completed = ToolCallUpdate::new(
        TOOL_CALL_ID,
        ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
    );
    let _ = cx.send_notification(SessionNotification::new(
        session_id.clone(),
        SessionUpdate::ToolCallUpdate(completed),
    ));

    StopReason::EndTurn
}
