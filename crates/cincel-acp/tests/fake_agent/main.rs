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
//! * `"echo-images"` → answers `N imágenes: <mime> <W>x<H>, …` for the image
//!   blocks it received, decoding each block's base64 data (dimensions come
//!   from the PNG/GIF/JPEG/WebP header; `?x?` when unknown or undecodable).
//! * `"url-elicitation"` → sends a url-mode `elicitation/create`
//!   (`elicitationId: "elic-1"`), then `elicitation/complete` 300 ms later
//!   without waiting, then reports the action it got back.
//!
//! `initialize` announces `promptCapabilities.image = true` (unless
//! `FAKE_NO_IMAGES` is set). With `FAKE_PROMPT_LOG=<path>` every
//! `session/prompt` appends one JSON line to that file with the received
//! `prompt` array, image data replaced by its length and SHA-256 (see
//! `describe_block`), so tests can assert what reached the agent.
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
//! With `FAKE_AUTH_REQUIRED` set, `session/new`, `session/load` and
//! `session/resume` fail with `auth_required` (-32000) whose `message` is the
//! variable's value (it may be empty) and whose `data` is
//! `FAKE_AUTH_REQUIRED_DATA` (parsed as JSON, or a plain string) when set;
//! with `FAKE_AUTH_LOGGED_OUT_DETAIL` also set, a logged-out
//! `_auth/status_update` carrying that `detail` is pushed first.
//!
//! With `FAKE_SHELL_EDITS` set, the default scenario is replaced by edits made
//! straight on disk with a single `execute` tool call that names no path
//! (see `shell_edits_turn` for the format): the review has to find them on
//! its own.
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
    NewSessionResponse, PermissionOption, PermissionOptionKind, PromptCapabilities, PromptRequest,
    PromptResponse, ReadTextFileRequest, RequestPermissionRequest, ResumeSessionRequest,
    ResumeSessionResponse, SessionCapabilities, SessionId, SessionInfoUpdate, SessionNotification,
    SessionResumeCapabilities, SessionUpdate, StopReason, TextContent, ToolCall, ToolCallContent,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind, WriteTextFileRequest,
};
use agent_client_protocol::{Agent, ConnectionTo, Stdio};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};

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
                                .prompt_capabilities(
                                    PromptCapabilities::new()
                                        .image(std::env::var_os("FAKE_NO_IMAGES").is_none()),
                                )
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
            async move |request: NewSessionRequest, responder, cx: ConnectionTo<_>| {
                if let Some(error) = scripted_auth_required(&cx)? {
                    return responder.respond_with_error(error);
                }
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
                if let Some(error) = scripted_auth_required(&cx)? {
                    return responder.respond_with_error(error);
                }
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
            async move |request: ResumeSessionRequest, responder, cx: ConnectionTo<_>| {
                if let Some(error) = scripted_auth_required(&cx)? {
                    return responder.respond_with_error(error);
                }
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

/// Describe one prompt block for `FAKE_PROMPT_LOG`: image data is replaced by
/// its base64 length, decoded length and SHA-256 of the decoded bytes.
fn describe_block(block: &ContentBlock) -> serde_json::Value {
    match block {
        ContentBlock::Text(text) => serde_json::json!({ "type": "text", "text": text.text }),
        ContentBlock::ResourceLink(link) => serde_json::json!({
            "type": "resource_link", "uri": link.uri, "name": link.name
        }),
        ContentBlock::Image(image) => {
            let decoded = BASE64.decode(&image.data);
            serde_json::json!({
                "type": "image",
                "mimeType": image.mime_type,
                "dataLen": image.data.len(),
                "bytes": decoded.as_ref().map(Vec::len).ok(),
                "sha256": decoded.as_ref().ok().map(|bytes| hex_sha256(bytes)),
                "uri": image.uri,
                "decodeError": decoded.is_err(),
            })
        }
        _ => serde_json::json!({ "type": "other" }),
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Append the received prompt to `FAKE_PROMPT_LOG` (one JSON line per
/// `session/prompt`), when set.
fn log_prompt(prompt: &[ContentBlock]) {
    use std::io::Write as _;
    let Some(path) = std::env::var_os("FAKE_PROMPT_LOG") else {
        return;
    };
    let line = serde_json::json!({
        "prompt": prompt.iter().map(describe_block).collect::<Vec<_>>()
    });
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        // One `write_all` so concurrent prompts never interleave a line.
        let _ = file.write_all(format!("{line}\n").as_bytes());
    }
}

/// Pixel size from a PNG, GIF, JPEG or WebP header (no full decode).
fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let be32 = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let le16 = |at: usize| {
        Some(u32::from(u16::from_le_bytes(
            bytes.get(at..at + 2)?.try_into().ok()?,
        )))
    };
    let le24 = |at: usize| {
        let b = bytes.get(at..at + 3)?;
        Some(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
    };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(16)?, be32(20)?));
    }
    if bytes.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return match bytes.get(12..16)? {
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            b"VP8L" => {
                let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            b"VP8 " => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            _ => None,
        };
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut at = 2;
        while at + 4 <= bytes.len() {
            if bytes[at] != 0xff {
                return None;
            }
            let marker = bytes[at + 1];
            let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
            // SOF0..SOF15 except DHT (c4), JPG (c8) and DAC (cc).
            if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                let height = u32::from(u16::from_be_bytes([
                    *bytes.get(at + 5)?,
                    *bytes.get(at + 6)?,
                ]));
                let width = u32::from(u16::from_be_bytes([
                    *bytes.get(at + 7)?,
                    *bytes.get(at + 8)?,
                ]));
                return Some((width, height));
            }
            at += 2 + length;
        }
    }
    None
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

/// The `auth_required` error `FAKE_AUTH_REQUIRED` asks for (see the module
/// docs), after pushing the logged-out status update when requested.
fn scripted_auth_required(
    cx: &ConnectionTo<agent_client_protocol::Client>,
) -> agent_client_protocol::Result<Option<agent_client_protocol::Error>> {
    let Ok(message) = std::env::var("FAKE_AUTH_REQUIRED") else {
        return Ok(None);
    };
    if let Ok(detail) = std::env::var("FAKE_AUTH_LOGGED_OUT_DETAIL") {
        cx.send_notification(UntypedMessage::new(
            "_auth/status_update",
            serde_json::json!({ "authStatus": {
                "kind": "none", "label": "Not logged in", "detail": detail
            } }),
        )?)?;
    }
    let mut error = agent_client_protocol::Error::new(-32000, message);
    if let Ok(data) = std::env::var("FAKE_AUTH_REQUIRED_DATA") {
        let data = serde_json::from_str(&data).unwrap_or(serde_json::Value::String(data));
        error = error.data(data);
    }
    Ok(Some(error))
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
    log_prompt(&request.prompt);

    match script.as_str() {
        "echo-images" => {
            let described: Vec<String> = request
                .prompt
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Image(image) => {
                        let dimensions = BASE64
                            .decode(&image.data)
                            .ok()
                            .and_then(|bytes| image_dimensions(&bytes))
                            .map_or_else(|| "?x?".to_string(), |(w, h)| format!("{w}x{h}"));
                        Some(format!("{} {dimensions}", image.mime_type))
                    }
                    _ => None,
                })
                .collect();
            let text = format!("{} imágenes: {}", described.len(), described.join(", "));
            send_text(cx, &session_id, &text).await;
            StopReason::EndTurn
        }
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
        _ => match std::env::var("FAKE_SHELL_EDITS") {
            Ok(spec) if !spec.is_empty() => {
                shell_edits_turn(cx, &session_id, &state.cwd, &spec).await
            }
            _ => default_turn(cx, &session_id, &state.cwd).await,
        },
    }
}

/// `FAKE_SHELL_EDITS`: the agent changes files straight on disk, the way a
/// `python3 - <<'EOF'` or a `sed -i` run by its shell tool would, and only
/// reports one `execute` tool call that names no path.
///
/// The variable is a list of `ruta=contenido` items separated by `;`, paths
/// relative to the session cwd:
///
/// * `ruta=contenido` overwrites the file;
/// * `ruta=@contenido` creates it (parent folders included);
/// * `ruta=` deletes it;
/// * `ruta=>destino` renames it (a real `mv`).
///
/// In the content, `\n` stands for a newline and `\t` for a tab.
async fn shell_edits_turn(
    cx: &ConnectionTo<agent_client_protocol::Client>,
    session_id: &SessionId,
    cwd: &std::path::Path,
    spec: &str,
) -> StopReason {
    const TOOL_CALL_ID: &str = "shell-1";
    let tool_call = ToolCall::new(TOOL_CALL_ID, "python3 - <<'EOF'")
        .kind(ToolKind::Execute)
        .status(ToolCallStatus::InProgress);
    if cx
        .send_notification(SessionNotification::new(
            session_id.clone(),
            SessionUpdate::ToolCall(tool_call),
        ))
        .is_err()
    {
        return StopReason::Cancelled;
    }
    apply_shell_edits(cwd, spec);
    let completed = ToolCallUpdate::new(
        TOOL_CALL_ID,
        ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
    );
    let _ = cx.send_notification(SessionNotification::new(
        session_id.clone(),
        SessionUpdate::ToolCallUpdate(completed),
    ));
    send_text(cx, session_id, "Listo: cambié los archivos con un script.").await;
    StopReason::EndTurn
}

/// Applies a `FAKE_SHELL_EDITS` list (see [`shell_edits_turn`]).
fn apply_shell_edits(cwd: &std::path::Path, spec: &str) {
    for item in spec.split(';') {
        let item = item.trim();
        let Some((path, value)) = item.split_once('=') else {
            continue;
        };
        let path = cwd.join(path.trim());
        let outcome = if value.is_empty() {
            std::fs::remove_file(&path)
        } else if let Some(destination) = value.strip_prefix('>') {
            std::fs::rename(&path, cwd.join(destination.trim()))
        } else if let Some(content) = value.strip_prefix('@') {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&path, unescape(content))
        } else {
            std::fs::write(&path, unescape(value))
        };
        if let Err(error) = outcome {
            eprintln!("FAKE_SHELL_EDITS: {}: {error}", path.display());
        }
    }
}

/// `\n` and `\t` of a `FAKE_SHELL_EDITS` content.
fn unescape(content: &str) -> String {
    content.replace("\\n", "\n").replace("\\t", "\t")
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
