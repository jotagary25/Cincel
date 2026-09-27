//! Ejemplo de consola: lanza un agente del registro y le manda un prompt.
//!
//! ```bash
//! cargo run -p cincel-acp --example chat -- \
//!     --agent claude-acp [--cwd DIR] [--mention archivo.rs] \
//!     [--cancel-after SEGUNDOS] "Respondé solo con la palabra: hola"
//!
//! # Solo `initialize`: imprime `agentCapabilities` y sale (no gasta tokens).
//! cargo run -p cincel-acp --example chat -- --agent claude-acp --capabilities-only
//! ```
//!
//! Sirve `fs/read_text_file` y `fs/write_text_file` directamente contra el
//! disco: eso es cosa del ejemplo, en Cincel lo resuelve el `BufferStore`.
//! `project_root` de la sandbox es el mismo `--cwd` (o el directorio temporal
//! cuando no se pasa).
//!
//! Códigos de salida: `0` turno terminado, `2` error de uso o del agente,
//! `3` hace falta autenticarse.

use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use cincel_acp::acp::schema::v1::{
    AuthMethod, ContentBlock, CreateElicitationResponse, ElicitationAction, SessionUpdate,
    StopReason, ToolCallContent, ToolKind,
};
use cincel_acp::connection::tool_call_paths;
use cincel_acp::{
    AgentCommand, AgentConnection, AgentEvent, AuthMethodKind, AuthMethodView, AutoAnswer,
    LaunchSpec, PermissionOutcome, PermissionPolicy, PromptBlock, pick_allow_option,
    pick_reject_option,
};

const USAGE: &str = "\
uso: chat --agent <id> [--cwd DIR]
          [--timeout SEGUNDOS] [--mention ARCHIVO]... [--cancel-after SEGUNDOS]
          \"texto del prompt\"
     chat --agent <id> --capabilities-only";

struct Args {
    agent: String,
    cwd: Option<PathBuf>,
    timeout: Duration,
    mentions: Vec<PathBuf>,
    cancel_after: Option<Duration>,
    /// Negotiate `initialize`, print what the agent announced and exit
    /// without creating a session or sending a prompt.
    capabilities_only: bool,
    prompt: String,
}

fn parse_args() -> Result<Args, String> {
    let mut agent = None;
    let mut cwd = None;
    let mut timeout = Duration::from_secs(600);
    let mut mentions = Vec::new();
    let mut cancel_after = None;
    let mut capabilities_only = false;
    let mut prompt = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--agent" => agent = Some(args.next().ok_or("falta el valor de --agent")?),
            "--cwd" => cwd = Some(PathBuf::from(args.next().ok_or("falta el valor de --cwd")?)),
            "--timeout" => {
                let value = args.next().ok_or("falta el valor de --timeout")?;
                let secs: u64 = value.parse().map_err(|_| "--timeout debe ser un número")?;
                timeout = Duration::from_secs(secs);
            }
            "--mention" => {
                let value = args.next().ok_or("falta el valor de --mention")?;
                mentions.push(PathBuf::from(value));
            }
            "--cancel-after" => {
                let value = args.next().ok_or("falta el valor de --cancel-after")?;
                let secs: u64 = value
                    .parse()
                    .map_err(|_| "--cancel-after debe ser un número")?;
                cancel_after = Some(Duration::from_secs(secs));
            }
            "--capabilities-only" | "--print-capabilities" => capabilities_only = true,
            "-h" | "--help" => return Err(USAGE.to_string()),
            other if other.starts_with("--") => return Err(format!("opción desconocida: {other}")),
            other => prompt = Some(other.to_string()),
        }
    }

    Ok(Args {
        agent: agent.ok_or("falta --agent")?,
        cwd,
        timeout,
        mentions,
        cancel_after,
        capabilities_only,
        // `--capabilities-only` no manda nada, así que no necesita texto.
        prompt: match prompt {
            Some(prompt) => prompt,
            None if capabilities_only => String::new(),
            None => return Err("falta el texto del prompt".to_string()),
        },
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match run(args).await {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

async fn run(args: Args) -> Result<ExitCode, String> {
    let registry = cincel_acp::AgentRegistry::load().map_err(|error| error.to_string())?;
    let descriptor = registry
        .get(&args.agent)
        .ok_or_else(|| format!("no existe el agente `{}` en el registro", args.agent))?;
    let launch = registry
        .launch_command(descriptor)
        .map_err(|error| error.to_string())?;

    // El directorio temporal debe vivir hasta el final del turno.
    let scratch = if args.cwd.is_none() {
        Some(tempfile::tempdir().map_err(|error| error.to_string())?)
    } else {
        None
    };
    let cwd = match (&args.cwd, &scratch) {
        (Some(path), _) => std::fs::canonicalize(path).map_err(|error| error.to_string())?,
        (None, Some(dir)) => dir.path().to_path_buf(),
        (None, None) => unreachable!(),
    };

    println!(
        "agente:    {} {} ({})",
        descriptor.name, descriptor.version, descriptor.id
    );
    println!("comando:   {}", launch.to_shell_string());
    println!("cwd:       {}", cwd.display());
    println!("registro:  {:?}", registry.source());
    if !args.mentions.is_empty() {
        println!(
            "menciones: {}",
            args.mentions
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if let Some(cancel_after) = args.cancel_after {
        println!("cancela en: {cancel_after:?}");
    }
    println!("---");

    // `project_root` de la sandbox = `cwd` de la sesión en este ejemplo.
    let mut connection = AgentConnection::start(cwd.clone());
    connection
        .send(AgentCommand::Spawn {
            launch: launch.clone(),
            cwd: cwd.clone(),
        })
        .await
        .map_err(|error| error.to_string())?;

    if args.capabilities_only {
        let outcome =
            tokio::time::timeout(args.timeout, print_capabilities(&connection, &launch)).await;
        connection.shutdown();
        return match outcome {
            Ok(result) => result,
            Err(_) => Err(format!("initialize excedió {:?}", args.timeout)),
        };
    }

    let session = Session {
        policy: PermissionPolicy::default(),
        cwd: cwd.clone(),
        launch,
        prompt: args.prompt.clone(),
        mentions: args.mentions.clone(),
        cancel_after: args.cancel_after,
        tool_titles: HashMap::new(),
    };

    let outcome = tokio::time::timeout(args.timeout, event_loop(&connection, session)).await;
    connection.shutdown();

    match outcome {
        Ok(Ok(code)) => Ok(code),
        Ok(Err(message)) => Err(message),
        Err(_) => Err(format!("el turno excedió {:?}", args.timeout)),
    }
}

/// Espera el `Connected` de `initialize`, imprime lo que anunció el agente y
/// vuelve. No crea sesión ni manda prompt: solo negocia el protocolo, así que
/// no consume tokens del modelo.
async fn print_capabilities(
    connection: &AgentConnection,
    launch: &LaunchSpec,
) -> Result<ExitCode, String> {
    loop {
        match connection.recv().await.map_err(|error| error.to_string())? {
            AgentEvent::Connected {
                agent_info,
                auth_methods,
                capabilities,
            } => {
                match agent_info {
                    Some(info) => println!("conectado: {} {}", info.name, info.version),
                    None => println!("conectado: (el agente no informó nombre)"),
                }
                println!("loadSession:            {}", capabilities.load_session);
                println!(
                    "sessionCapabilities.resume:  {}",
                    capabilities.session_capabilities.resume.is_some()
                );
                println!(
                    "sessionCapabilities.list:    {}",
                    capabilities.session_capabilities.list.is_some()
                );
                println!("agentCapabilities: {capabilities:?}");
                print_auth_methods(&auth_methods, launch);
                return Ok(ExitCode::SUCCESS);
            }
            AgentEvent::AuthRequired { methods } => {
                print_auth_methods(&methods, launch);
                return Ok(ExitCode::from(3));
            }
            AgentEvent::Exited { code, stderr_tail } => {
                return Err(format!("el agente se cerró ({code:?}): {stderr_tail}"));
            }
            AgentEvent::Error { message, .. } => return Err(message),
            _ => {}
        }
    }
}

struct Session {
    policy: PermissionPolicy,
    cwd: PathBuf,
    launch: LaunchSpec,
    prompt: String,
    mentions: Vec<PathBuf>,
    cancel_after: Option<Duration>,
    tool_titles: HashMap<String, String>,
}

async fn event_loop(
    connection: &AgentConnection,
    mut session: Session,
) -> Result<ExitCode, String> {
    let mut prompt_sent = false;
    let mut streaming = false;

    loop {
        let event = connection.recv().await.map_err(|error| error.to_string())?;
        match event {
            AgentEvent::Connected {
                agent_info,
                auth_methods,
                capabilities,
            } => {
                match agent_info {
                    Some(info) => println!("conectado: {} {}", info.name, info.version),
                    None => println!("conectado: (el agente no informó nombre)"),
                }
                println!("capacidades del agente: {capabilities:?}");
                print_auth_methods(&auth_methods, &session.launch);
                connection
                    .send(AgentCommand::NewSession {
                        cwd: session.cwd.clone(),
                        mcp_servers: Vec::new(),
                    })
                    .await
                    .map_err(|error| error.to_string())?;
            }
            AgentEvent::AuthRequired { methods } => {
                println!("---");
                println!("hace falta autenticarse antes de crear la sesión.");
                print_auth_methods(&methods, &session.launch);
                return Ok(ExitCode::from(3));
            }
            AgentEvent::SessionCreated {
                session_id,
                modes,
                config_options,
                commands,
            } => {
                println!("sesión: {}", session_id.0);
                if let Some(modes) = &modes {
                    println!("modo actual: {}", modes.current_mode_id.0);
                }
                if !config_options.is_empty() {
                    let names: Vec<&str> = config_options
                        .iter()
                        .map(|option| option.name.as_str())
                        .collect();
                    println!("opciones de config: {}", names.join(", "));
                }
                if !commands.is_empty() {
                    println!("comandos: {}", commands.len());
                }
                println!("---");
                if !prompt_sent {
                    prompt_sent = true;
                    let mut blocks = vec![PromptBlock::Text(session.prompt.clone())];
                    blocks.extend(
                        session
                            .mentions
                            .iter()
                            .map(|path| PromptBlock::mention(path)),
                    );
                    connection
                        .send(AgentCommand::Prompt {
                            session_id: session_id.clone(),
                            blocks,
                            feedback: None,
                        })
                        .await
                        .map_err(|error| error.to_string())?;
                    if let Some(cancel_after) = session.cancel_after {
                        let commands = connection.commands().clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(cancel_after).await;
                            eprintln!("[chat] cancelando el turno tras {cancel_after:?}");
                            let _ = commands.send(AgentCommand::Cancel { session_id }).await;
                        });
                    }
                }
            }
            AgentEvent::Update { update, .. } => {
                handle_update(&mut session, *update, &mut streaming);
            }
            AgentEvent::PermissionRequest {
                tool_call,
                options,
                reply,
                ..
            } => {
                end_stream(&mut streaming);
                let kind = tool_call.fields.kind.unwrap_or_default();
                let title = tool_call
                    .fields
                    .title
                    .clone()
                    .or_else(|| session.tool_titles.get(&*tool_call.tool_call_id.0).cloned())
                    .unwrap_or_else(|| tool_call.tool_call_id.0.to_string());
                let paths = tool_call_paths(&tool_call);

                let decision = session.policy.decide(kind, &paths);
                let allow = match decision {
                    AutoAnswer::Allow => {
                        println!(
                            "[permiso] {} «{}» → permitido por política",
                            kind_label(kind),
                            title
                        );
                        true
                    }
                    AutoAnswer::Ask => ask_on_terminal(kind, &title, &paths),
                };

                let chosen = if allow {
                    pick_allow_option(&options)
                } else {
                    pick_reject_option(&options)
                };
                match chosen {
                    Some(option) => {
                        reply.respond(PermissionOutcome::Selected(option.option_id.clone()));
                    }
                    None => {
                        reply.respond(PermissionOutcome::Cancelled);
                    }
                }
            }
            AgentEvent::FsRead {
                path,
                line,
                limit,
                reply,
                ..
            } => {
                let _ = reply.send(read_text_file(&path, line, limit));
            }
            AgentEvent::FsWrite {
                path,
                content,
                reply,
                ..
            } => {
                let _ = reply.send(write_text_file(&path, &content));
            }
            AgentEvent::TurnEnded { stop_reason, .. } => {
                end_stream(&mut streaming);
                println!("---");
                println!("stop reason: {}", stop_reason_label(stop_reason));
                return Ok(ExitCode::SUCCESS);
            }
            AgentEvent::Stderr(line) => eprintln!("[agente] {line}"),
            AgentEvent::Exited { code, stderr_tail } => {
                end_stream(&mut streaming);
                let tail = if stderr_tail.is_empty() {
                    String::new()
                } else {
                    format!("\n--- stderr ---\n{stderr_tail}")
                };
                return Err(format!("el agente terminó (código {code:?}){tail}"));
            }
            AgentEvent::Error {
                message,
                stderr_tail,
            } => {
                if stderr_tail.is_empty() {
                    return Err(message);
                }
                return Err(format!("{message}\n--- stderr ---\n{stderr_tail}"));
            }
            AgentEvent::ToolCallsCancelled { ids, .. } => {
                end_stream(&mut streaming);
                println!(
                    "[cancelado] {} tool call(s): {}",
                    ids.len(),
                    ids.iter()
                        .map(|id| id.0.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            AgentEvent::FileChangeReport { report, .. } => {
                end_stream(&mut streaming);
                println!(
                    "[archivos] {} ruta(s), completo={}",
                    report.paths.len(),
                    report.declared_complete
                );
            }
            AgentEvent::Elicitation { request, reply, .. } => {
                end_stream(&mut streaming);
                println!(
                    "[elicitation] {} → declinado automáticamente (sin UI interactiva)",
                    request.message
                );
                reply.respond(CreateElicitationResponse::new(ElicitationAction::Decline));
            }
            other => eprintln!("[evento no manejado] {other:?}"),
        }
    }
}

/// Close the streamed line before printing anything else on stdout.
fn end_stream(streaming: &mut bool) {
    if *streaming {
        println!();
        *streaming = false;
    }
}

fn handle_update(session: &mut Session, update: SessionUpdate, streaming: &mut bool) {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            if let ContentBlock::Text(text) = chunk.content {
                use std::io::Write as _;
                print!("{}", text.text);
                let _ = std::io::stdout().flush();
                *streaming = true;
            }
        }
        SessionUpdate::AgentThoughtChunk(_) => {}
        SessionUpdate::ToolCall(call) => {
            end_stream(streaming);
            session
                .tool_titles
                .insert(call.tool_call_id.0.to_string(), call.title.clone());
            println!(
                "[tool] {} «{}» [{}]{}",
                kind_label(call.kind),
                call.title,
                status_label(call.status),
                summarize_content(&call.content)
            );
        }
        SessionUpdate::ToolCallUpdate(update) => {
            end_stream(streaming);
            let title = update
                .fields
                .title
                .clone()
                .or_else(|| session.tool_titles.get(&*update.tool_call_id.0).cloned())
                .unwrap_or_else(|| update.tool_call_id.0.to_string());
            let kind = update.fields.kind.unwrap_or_default();
            let status = update.fields.status.unwrap_or_default();
            let content = update.fields.content.clone().unwrap_or_default();
            println!(
                "[tool] {} «{}» [{}]{}",
                kind_label(kind),
                title,
                status_label(status),
                summarize_content(&content)
            );
        }
        SessionUpdate::Plan(plan) => {
            end_stream(streaming);
            println!("[plan] {} entradas", plan.entries.len());
        }
        SessionUpdate::CurrentModeUpdate(mode) => {
            end_stream(streaming);
            println!("[modo] {}", mode.current_mode_id.0);
        }
        SessionUpdate::AvailableCommandsUpdate(commands) => {
            end_stream(streaming);
            println!("[comandos] {}", commands.available_commands.len());
        }
        _ => {}
    }
}

/// `path (+added/-removed)` for every diff in a tool call's content.
fn summarize_content(content: &[ToolCallContent]) -> String {
    let mut out = String::new();
    for item in content {
        if let ToolCallContent::Diff(diff) = item {
            let (added, removed) = count_diff_lines(diff.old_text.as_deref(), &diff.new_text);
            out.push_str(&format!(" {} (+{added}/-{removed})", diff.path.display()));
        }
    }
    out
}

/// Approximate added/removed line counts as a multiset difference. Good enough
/// for a one-line summary; the real review UI (Etapa 1) computes real hunks.
fn count_diff_lines(old_text: Option<&str>, new_text: &str) -> (usize, usize) {
    let Some(old_text) = old_text else {
        return (new_text.lines().count(), 0);
    };
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for line in old_text.lines() {
        *counts.entry(line).or_default() -= 1;
    }
    for line in new_text.lines() {
        *counts.entry(line).or_default() += 1;
    }
    let mut added = 0usize;
    let mut removed = 0usize;
    for delta in counts.values() {
        if *delta > 0 {
            added += *delta as usize;
        } else {
            removed += delta.unsigned_abs() as usize;
        }
    }
    (added, removed)
}

fn ask_on_terminal(kind: ToolKind, title: &str, paths: &[PathBuf]) -> bool {
    let files = if paths.is_empty() {
        String::new()
    } else {
        format!(
            " ({})",
            paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    if !std::io::stdin().is_terminal() {
        println!(
            "[permiso] {} «{title}»{files} → rechazado (no hay terminal interactiva)",
            kind_label(kind)
        );
        return false;
    }
    use std::io::Write as _;
    print!(
        "[permiso] {} «{title}»{files} ¿permitir? [s/N] ",
        kind_label(kind)
    );
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(
        answer.trim().to_lowercase().as_str(),
        "s" | "si" | "sí" | "y" | "yes"
    )
}

fn read_text_file(
    path: &Path,
    line: Option<u32>,
    limit: Option<u32>,
) -> Result<String, cincel_acp::FsError> {
    let content = std::fs::read_to_string(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => cincel_acp::FsError::NotFound(path.display().to_string()),
        _ => cincel_acp::FsError::Internal(error.to_string()),
    })?;
    let start = line.unwrap_or(1);
    if start == 0 {
        return Err(cincel_acp::FsError::InvalidParams(
            "`line` es 1-based".to_string(),
        ));
    }
    let lines: Vec<&str> = content.lines().collect();
    let first = (start - 1) as usize;
    if first > lines.len() {
        return Err(cincel_acp::FsError::InvalidParams(format!(
            "`line` {start} supera el total ({})",
            lines.len()
        )));
    }
    let slice = match limit {
        Some(limit) => &lines[first..(first + limit as usize).min(lines.len())],
        None => &lines[first..],
    };
    let mut out = slice.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    println!("[fs] leído {} ({} líneas)", path.display(), slice.len());
    Ok(out)
}

fn write_text_file(path: &Path, content: &str) -> Result<(), cincel_acp::FsError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| cincel_acp::FsError::Internal(error.to_string()))?;
    }
    std::fs::write(path, content)
        .map_err(|error| cincel_acp::FsError::Internal(error.to_string()))?;
    println!("[fs] escrito {} ({} bytes)", path.display(), content.len());
    Ok(())
}

fn print_auth_methods(methods: &[AuthMethod], launch: &LaunchSpec) {
    if methods.is_empty() {
        println!("métodos de auth: ninguno (el agente usa la sesión del sistema)");
        return;
    }
    println!("métodos de auth:");
    for method in methods {
        let view = AuthMethodView::describe(method, launch);
        match &view.kind {
            AuthMethodKind::Terminal { .. } => {
                let command = view.shell_command().unwrap_or_default();
                println!("  - {} ({}) → {command}", view.name, view.id);
            }
            AuthMethodKind::Other => {
                println!("  - {} ({}) → session/authenticate", view.name, view.id);
            }
        }
    }
}

fn kind_label(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "read",
        ToolKind::Edit => "edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "execute",
        ToolKind::Think => "think",
        ToolKind::Fetch => "fetch",
        ToolKind::SwitchMode => "switch_mode",
        _ => "other",
    }
}

fn status_label(status: cincel_acp::acp::schema::v1::ToolCallStatus) -> &'static str {
    use cincel_acp::acp::schema::v1::ToolCallStatus;
    match status {
        ToolCallStatus::Pending => "pending",
        ToolCallStatus::InProgress => "in_progress",
        ToolCallStatus::Completed => "completed",
        ToolCallStatus::Failed => "failed",
        _ => "?",
    }
}

fn stop_reason_label(reason: StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Refusal => "refusal",
        StopReason::Cancelled => "cancelled",
        _ => "?",
    }
}
