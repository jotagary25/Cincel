//! Minimal ACP agent for the identity probe, ACP login and disconnect
//! tests. Behavior comes from environment variables:
//!
//! * `FAKE_NO_LOGOUT`: do not announce `auth.logout`;
//! * `FAKE_NO_AUTH_STATUS`: do not announce `_meta.authStatus` nor push
//!   (Antigravity announces neither);
//! * `FAKE_AUTH_EMAIL`: push `_auth/status_update` (`kind: account`) after
//!   `initialize`; without it the push is `kind: none`;
//! * `FAKE_LOGOUT_MARKER`: on `logout`, write the value of the profile
//!   variable (`CLAUDE_CONFIG_DIR`, else `CODEX_HOME`, else `GEMINI_HOME`)
//!   to this file;
//! * `FAKE_ENV_MARKER`: right after `initialize`, write `GEMINI_HOME`,
//!   `BROWSER` and the process id as seen by the process
//!   (`GEMINI_HOME=...\nBROWSER=...\nPID=...`).
//!
//! `authenticate` behaves like Antigravity's `oauth-personal` (announced in
//! `initialize` as "Log in with Google"):
//!
//! * `FAKE_AUTH_URL`: print `Open the following link to authenticate the ACP
//!   server: <url>` to stderr (without it, nothing is printed);
//! * `FAKE_AUTH_ELICIT_URL`: send an `elicitation/create` in `url` mode with
//!   this link instead, and wait for the answer;
//! * `FAKE_AUTH_DELAY_MS`: wait this long before answering (default 200),
//!   standing in for the user approving in the browser;
//! * `FAKE_AUTH_FAIL`: answer with an error;
//! * `FAKE_AUTH_EXIT`: exit with code 3 instead of answering;
//! * `FAKE_AUTH_NO_TOKEN`: answer success without writing the token file;
//! * otherwise write `<GEMINI_HOME>/antigravity-acp/acp_token.json` (with
//!   `FAKE_TOKEN_EMAIL` as its `email` when set) and answer success.
//!
//! `logout` also removes that token file.

use std::path::PathBuf;
use std::time::Duration;

use cincel_acp::acp::schema::v1::{
    AgentAuthCapabilities, AgentCapabilities, AuthMethod, AuthMethodAgent, AuthenticateRequest,
    AuthenticateResponse, CreateElicitationRequest, ElicitationRequestScope, ElicitationUrlMode,
    Implementation, InitializeRequest, InitializeResponse, LogoutCapabilities, LogoutRequest,
    LogoutResponse, NewSessionRequest, NewSessionResponse, RequestId, SessionId,
};
use cincel_acp::acp::{Agent, ConnectionTo, Stdio, UntypedMessage};

fn status_update(email: Option<&str>) -> serde_json::Value {
    match email {
        Some(email) => serde_json::json!({ "authStatus": {
            "kind": "account",
            "label": "Fake Max",
            "account": { "email": email, "organization": "Fake Org", "plan": "max" }
        } }),
        None => serde_json::json!({ "authStatus": { "kind": "none", "label": "Not logged in" } }),
    }
}

fn token_file() -> Option<PathBuf> {
    let home = std::env::var_os("GEMINI_HOME")?;
    Some(
        PathBuf::from(home)
            .join("antigravity-acp")
            .join("acp_token.json"),
    )
}

fn write_token() {
    let Some(path) = token_file() else {
        return;
    };
    let mut token = serde_json::json!({
        "client_id": "fake.apps.googleusercontent.com",
        "refresh_token": "fake-refresh",
        "token_uri": "https://oauth2.googleapis.com/token",
        "scopes": ["openid"],
        "project_id": "fake-project"
    });
    if let Ok(email) = std::env::var("FAKE_TOKEN_EMAIL") {
        token["email"] = serde_json::Value::String(email);
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, token.to_string());
}

#[tokio::main]
async fn main() -> cincel_acp::acp::Result<()> {
    Agent
        .builder()
        .name("cincel-connections-fake-agent")
        .on_receive_request(
            async move |request: InitializeRequest, responder, cx: ConnectionTo<_>| {
                let mut auth = AgentAuthCapabilities::new();
                if std::env::var_os("FAKE_NO_LOGOUT").is_none() {
                    auth = auth.logout(LogoutCapabilities::new());
                }
                let announce_status = std::env::var_os("FAKE_NO_AUTH_STATUS").is_none();
                let mut capabilities = AgentCapabilities::new().auth(auth);
                if announce_status {
                    capabilities = capabilities.meta(
                        serde_json::json!({ "authStatus": {} })
                            .as_object()
                            .cloned()
                            .unwrap_or_default(),
                    );
                }
                if let Ok(marker) = std::env::var("FAKE_ENV_MARKER") {
                    let seen = |name: &str| std::env::var(name).unwrap_or_default();
                    let _ = std::fs::write(
                        marker,
                        format!(
                            "GEMINI_HOME={}\nBROWSER={}\nPID={}\n",
                            seen("GEMINI_HOME"),
                            seen("BROWSER"),
                            std::process::id()
                        ),
                    );
                }
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(capabilities)
                        .auth_methods(vec![AuthMethod::Agent(AuthMethodAgent::new(
                            "oauth-personal",
                            "Log in with Google",
                        ))])
                        .agent_info(Implementation::new(
                            "cincel-connections-fake-agent",
                            "0.1.0",
                        )),
                )?;
                if announce_status {
                    let email = std::env::var("FAKE_AUTH_EMAIL").ok();
                    cx.send_notification(UntypedMessage::new(
                        "_auth/status_update",
                        status_update(email.as_deref()),
                    )?)?;
                }
                Ok(())
            },
            cincel_acp::acp::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: AuthenticateRequest, responder, cx: ConnectionTo<_>| {
                cx.clone().spawn(async move {
                    if let Ok(url) = std::env::var("FAKE_AUTH_URL") {
                        eprintln!("Open the following link to authenticate the ACP server: {url}");
                    }
                    if let Ok(url) = std::env::var("FAKE_AUTH_ELICIT_URL") {
                        let elicitation = CreateElicitationRequest::new(
                            ElicitationUrlMode::new(
                                ElicitationRequestScope::new(RequestId::Str("auth".into())),
                                "login-1",
                                url,
                            ),
                            "Iniciá sesión con Google",
                        );
                        let _ = cx.send_request(elicitation).block_task().await;
                    }
                    let delay = std::env::var("FAKE_AUTH_DELAY_MS")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(200);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    if std::env::var_os("FAKE_AUTH_EXIT").is_some() {
                        std::process::exit(3);
                    }
                    if std::env::var_os("FAKE_AUTH_FAIL").is_some() {
                        return responder.respond_with_error(
                            cincel_acp::acp::Error::invalid_params()
                                .data(serde_json::json!("acceso denegado")),
                        );
                    }
                    if std::env::var_os("FAKE_AUTH_NO_TOKEN").is_none() {
                        write_token();
                    }
                    responder.respond(AuthenticateResponse::new())
                })
            },
            cincel_acp::acp::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: LogoutRequest, responder, _cx| {
                if let Ok(marker) = std::env::var("FAKE_LOGOUT_MARKER") {
                    let profile = std::env::var("CLAUDE_CONFIG_DIR")
                        .or_else(|_| std::env::var("CODEX_HOME"))
                        .or_else(|_| std::env::var("GEMINI_HOME"))
                        .unwrap_or_else(|_| "<sin variable de perfil>".to_string());
                    let _ = std::fs::write(marker, profile);
                }
                if let Some(token) = token_file() {
                    let _ = std::fs::remove_file(token);
                }
                responder.respond(LogoutResponse::new())
            },
            cincel_acp::acp::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: NewSessionRequest, responder, _cx| {
                responder.respond(NewSessionResponse::new(SessionId::new("fake-1")))
            },
            cincel_acp::acp::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}
