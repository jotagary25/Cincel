//! Minimal ACP v1 agent used by the integration tests.
//!
//! It answers `initialize` and `session/new`, and on `session/prompt` it
//! exercises the whole client surface: two text chunks, a `tool_call` of kind
//! `edit` carrying a `diff`, a permission request, `fs/read_text_file`,
//! `fs/write_text_file`, a `tool_call_update` and `end_turn`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, Diff, Implementation, InitializeRequest,
    InitializeResponse, NewSessionRequest, NewSessionResponse, PermissionOption,
    PermissionOptionKind, PromptRequest, PromptResponse, ReadTextFileRequest,
    RequestPermissionRequest, SessionId, SessionNotification, SessionUpdate, StopReason,
    TextContent, ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
    ToolKind, WriteTextFileRequest,
};
use agent_client_protocol::{Agent, ConnectionTo, Stdio};

const SESSION_ID: &str = "fake-session-1";
const TOOL_CALL_ID: &str = "call-1";
/// File the fake agent pretends to edit, relative to the session cwd.
const TARGET_FILE: &str = "demo.txt";

#[tokio::main]
async fn main() -> agent_client_protocol::Result<()> {
    let cwd: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let cwd_new_session = cwd.clone();
    let cwd_prompt = cwd.clone();

    Agent
        .builder()
        .name("asteroid-fake-agent")
        .on_receive_request(
            async move |request: InitializeRequest, responder, _cx| {
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(AgentCapabilities::new())
                        .agent_info(Implementation::new("asteroid-fake-agent", "0.1.0")),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _cx| {
                if let Ok(mut slot) = cwd_new_session.lock() {
                    *slot = Some(request.cwd.clone());
                }
                responder.respond(NewSessionResponse::new(SessionId::new(SESSION_ID)))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, cx: ConnectionTo<_>| {
                let cwd = cwd_prompt
                    .lock()
                    .ok()
                    .and_then(|slot| slot.clone())
                    .unwrap_or_else(|| PathBuf::from("/"));
                let session_id = request.session_id.clone();
                // Run the turn outside the dispatch loop: it sends requests to
                // the client and must not block incoming traffic.
                cx.clone().spawn(async move {
                    let result = run_turn(&cx, session_id, cwd).await;
                    responder.respond(PromptResponse::new(result))
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}

async fn run_turn(
    cx: &ConnectionTo<agent_client_protocol::Client>,
    session_id: SessionId,
    cwd: PathBuf,
) -> StopReason {
    let path = cwd.join(TARGET_FILE);

    for text in ["Hola", " mundo"] {
        let update = SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
            TextContent::new(text),
        )));
        if cx
            .send_notification(SessionNotification::new(session_id.clone(), update))
            .is_err()
        {
            return StopReason::Cancelled;
        }
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
        session_id,
        SessionUpdate::ToolCallUpdate(completed),
    ));

    StopReason::EndTurn
}
