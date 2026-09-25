//! Demo of the chat panel: opens a window with a scripted agent that streams
//! markdown, runs a tool call with `+N −M` stats and asks for permission.
//!
//! ```text
//! cargo run -p asteroid-chat --example chat_demo
//! cargo run -p asteroid-chat --example chat_demo -- --smoke-test
//! ```
//!
//! Nothing here talks to a real agent: the events are the same
//! `asteroid_acp::AgentEvent`s the ACP worker would send, so the panel cannot
//! tell the difference. What the panel emits is printed on stdout, which is
//! exactly what the workspace would forward to the worker.

use std::path::PathBuf;
use std::time::Duration;

use asteroid_acp::AgentEvent;
use asteroid_acp::acp::schema::v1::{
    AvailableCommand, AvailableCommandsUpdate, Content, ContentBlock, ContentChunk, Diff,
    PermissionOption, PermissionOptionKind, Plan, PlanEntry, PlanEntryPriority, PlanEntryStatus,
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelect,
    SessionConfigSelectOption, SessionId, SessionUpdate, StopReason, TextContent, ToolCall,
    ToolCallContent, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use asteroid_chat::{
    ChatAgent, ChatEvent, ChatPanel, ChatSettings, ChatTheme, ConversationSummary,
};
use gpui::{
    App, AppContext, Bounds, Entity, Focusable, KeyBinding, WindowBounds, WindowOptions, actions,
    px, size,
};
use gpui_kit::assets::AllAssets;

actions!(
    chat_demo,
    [
        /// Closes the demo.
        Quit
    ]
);

/// How long `--smoke-test` lets the window live once the script is over.
const SMOKE_TAIL: Duration = Duration::from_millis(200);
/// Delay between scripted events, so the streaming is visible.
const STEP: Duration = Duration::from_millis(90);

fn session() -> SessionId {
    SessionId::new("demo")
}

fn chunk(text: &str) -> AgentEvent {
    AgentEvent::Update {
        session_id: session(),
        update: Box::new(SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        ))),
    }
}

fn thought(text: &str) -> AgentEvent {
    AgentEvent::Update {
        session_id: session(),
        update: Box::new(SessionUpdate::AgentThoughtChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        ))),
    }
}

fn update(update: SessionUpdate) -> AgentEvent {
    AgentEvent::Update {
        session_id: session(),
        update: Box::new(update),
    }
}

/// A shell command that printed `lines` lines, to exercise the "ver más" of a
/// command card (the cap is 20).
fn long_output(lines: usize) -> String {
    (1..=lines)
        .map(|row| format!("test asteroid_chat::caso_{row} ... ok"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn model_option() -> SessionConfigOption {
    let mut option = SessionConfigOption::new(
        "model",
        "Modelo",
        SessionConfigKind::Select(SessionConfigSelect::new(
            "sonnet",
            vec![
                SessionConfigSelectOption::new("sonnet", "Sonnet"),
                SessionConfigSelectOption::new("opus", "Opus"),
            ],
        )),
    );
    option.category = Some(SessionConfigOptionCategory::Model);
    option
}

fn thought_option() -> SessionConfigOption {
    let mut option = SessionConfigOption::new(
        "thinking",
        "Esfuerzo",
        SessionConfigKind::Select(SessionConfigSelect::new(
            "medio",
            vec![
                SessionConfigSelectOption::new("bajo", "Bajo"),
                SessionConfigSelectOption::new("medio", "Medio"),
                SessionConfigSelectOption::new("alto", "Alto"),
            ],
        )),
    );
    option.category = Some(SessionConfigOptionCategory::ThoughtLevel);
    option
}

/// `session/new` answered: the demo starts here, before the user types.
fn session_created() -> AgentEvent {
    AgentEvent::SessionCreated {
        session_id: session(),
        modes: None,
        config_options: vec![model_option(), thought_option()],
        commands: vec![
            AvailableCommand::new("create_plan", "Arma un plan antes de tocar código"),
            AvailableCommand::new("review", "Revisa lo que acabás de escribir"),
        ],
    }
}

/// The scripted turn: everything a real agent would send, in order.
fn script() -> Vec<AgentEvent> {
    vec![
        thought("Reviso el módulo"),
        thought(" y decido por dónde empezar:"),
        thought(" main.rs repite la suma en dos lugares."),
        update(SessionUpdate::Plan(Plan::new(vec![
            PlanEntry::new(
                "Leer src/main.rs",
                PlanEntryPriority::High,
                PlanEntryStatus::Completed,
            ),
            PlanEntry::new(
                "Extraer la función",
                PlanEntryPriority::High,
                PlanEntryStatus::InProgress,
            ),
            PlanEntry::new(
                "Correr los tests",
                PlanEntryPriority::Medium,
                PlanEntryStatus::Pending,
            ),
        ]))),
        chunk("Voy a extraer el cálculo a su propia función.\n\n"),
        chunk("Queda así:\n\n```rust\nfn suma"),
        chunk("(a: i32, b: i32) -> i32 {\n    a + b\n}\n```\n\n"),
        chunk("Y `main` la usa en lugar de repetir la cuenta."),
        update(SessionUpdate::ToolCall(
            ToolCall::new("t-read", "Leer src/main.rs").kind(ToolKind::Read),
        )),
        update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t-read",
            ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
        ))),
        update(SessionUpdate::ToolCall(
            ToolCall::new("t-edit", "Editar src/main.rs")
                .kind(ToolKind::Edit)
                .status(ToolCallStatus::InProgress)
                .content(vec![ToolCallContent::Diff(Diff::new(
                    "/proyecto/src/main.rs",
                    "fn suma(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn main() {\n    println!(\"{}\", suma(1, 2));\n}\n",
                ))]),
        )),
        update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t-edit",
            ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
        ))),
        update(SessionUpdate::AvailableCommandsUpdate(
            AvailableCommandsUpdate::new(vec![
                AvailableCommand::new("create_plan", "Arma un plan antes de tocar código"),
                AvailableCommand::new("review", "Revisa lo que acabás de escribir"),
                AvailableCommand::new("test", "Corre la suite"),
            ]),
        )),
        // Una orden que salió bien: se pliega sola y solo deja su fila.
        update(SessionUpdate::ToolCall(
            ToolCall::new("t-fmt", "cargo fmt -p asteroid-chat").kind(ToolKind::Execute),
        )),
        update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t-fmt",
            ToolCallUpdateFields::new()
                .status(ToolCallStatus::Completed)
                .content(vec![ToolCallContent::Content(Content::new(
                    ContentBlock::Text(TextContent::new("sin cambios")),
                ))]),
        ))),
        // Y una que falló: se abre sola, con su salida recortada a 20 líneas.
        update(SessionUpdate::ToolCall(
            ToolCall::new("t-clippy", "cargo clippy -p asteroid-chat").kind(ToolKind::Execute),
        )),
        // El agente la manda envuelta en ```console y con su código de salida:
        // la tarjeta saca la valla y muestra el código en la cabecera.
        update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t-clippy",
            ToolCallUpdateFields::new()
                .status(ToolCallStatus::Failed)
                .content(vec![ToolCallContent::Content(Content::new(
                    ContentBlock::Text(TextContent::new(format!(
                        "```console\n{}\nerror: exit code: 101\n```",
                        long_output(24)
                    ))),
                ))]),
        ))),
        update(SessionUpdate::ToolCall(
            ToolCall::new("t-run", "cargo test -p asteroid-chat").kind(ToolKind::Execute),
        )),
    ]
}

fn parse_smoke_test() -> bool {
    std::env::args().skip(1).any(|arg| arg == "--smoke-test")
}

fn main() {
    let smoke_test = parse_smoke_test();

    // `AllAssets` embeds the whole Lucide catalog: the tool cards pick an icon
    // per tool kind (`02-visual.md` §7), outside gpui-kit's own subset.
    gpui_kit::application()
        .with_assets(AllAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            asteroid_chat::init(cx);
            cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

            let bounds = Bounds::centered(None, size(px(420.), px(720.)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        ..Default::default()
                    },
                    |window, cx| {
                        cx.new(|cx| {
                            ChatPanel::new(
                                ChatTheme::default(),
                                ChatSettings::default(),
                                window,
                                cx,
                            )
                        })
                    },
                )
                .expect("no se pudo abrir la ventana");

            window
                .update(cx, |panel, window, cx| {
                    panel.set_agents(
                        vec![
                            ChatAgent::installed("claude-acp", "Claude"),
                            ChatAgent {
                                id: "gemini".into(),
                                name: "Gemini".into(),
                                installed: false,
                                hint: Some("npx @google/gemini-cli --experimental-acp".into()),
                            },
                        ],
                        cx,
                    );
                    panel.set_active_agent("claude-acp", cx);
                    panel.set_conversations(vec![
                        ConversationSummary {
                            id: "c-hoy".into(),
                            agent_id: "claude-acp".into(),
                            agent_name: "Claude".into(),
                            title: "Revisar el diff inline".into(),
                            when: "hace 5 min".into(),
                            session_id: Some("s-hoy".into()),
                        },
                        ConversationSummary {
                            id: "c-ayer".into(),
                            agent_id: "claude-acp".into(),
                            agent_name: "Claude".into(),
                            title: "Extraer el parser".into(),
                            when: "ayer".into(),
                            session_id: Some("s-ayer".into()),
                        },
                        ConversationSummary {
                            id: "c-gemini".into(),
                            agent_id: "gemini".into(),
                            agent_name: "Gemini".into(),
                            title: "Probar el selector de modelo".into(),
                            when: "hace 2 d".into(),
                            session_id: None,
                        },
                    ], cx);
                    // Con la raíz puesta, el `@` muestra `main.rs` y `src`, no
                    // `/proyecto/src/main.rs`.
                    panel.set_project_root(Some(PathBuf::from("/proyecto")), cx);
                    panel.set_file_candidates(
                        vec![
                            PathBuf::from("/proyecto/src/main.rs"),
                            PathBuf::from("/proyecto/src/lib.rs"),
                            PathBuf::from("/proyecto/Cargo.toml"),
                        ],
                        cx,
                    );
                    let handle = panel.focus_handle(cx);
                    window.focus(&handle, cx);
                    cx.activate(true);

                    let entity: Entity<ChatPanel> = cx.entity();
                    cx.subscribe(&entity, |_panel, _entity, event: &ChatEvent, _cx| {
                        println!("chat → {event:?}");
                    })
                    .detach();
                })
                .expect("no se pudo preparar el panel");

            cx.spawn(async move |cx| {
                cx.background_executor().timer(STEP).await;
                if window
                    .update(cx, |panel, _window, cx| {
                        panel.handle_event(session_created(), cx)
                    })
                    .is_err()
                {
                    return;
                }

                // La mención va **dentro** del texto, donde el autor la
                // escribe, y viaja como bloques ordenados
                // (`docs/etapas/etapa-2.md` § correcciones).
                cx.background_executor().timer(STEP).await;
                if window
                    .update(cx, |panel, window, cx| {
                        panel.set_input_text("mirá este archivo ", window, cx);
                        panel.insert_mention(PathBuf::from("/proyecto/src/main.rs"), window, cx);
                        let text =
                            format!("{}, ¿qué te parece?", panel.input_text(cx).trim_end());
                        panel.set_input_text(&text, window, cx);
                        panel.send(window, cx);
                    })
                    .is_err()
                {
                    return;
                }

                for event in script() {
                    cx.background_executor().timer(STEP).await;
                    if window
                        .update(cx, |panel, _window, cx| panel.handle_event(event, cx))
                        .is_err()
                    {
                        return;
                    }
                }

                // The permission request carries a responder only `asteroid-acp`
                // can build, so the demo uses the panel's own seam.
                cx.background_executor().timer(STEP).await;
                window
                    .update(cx, |panel, _window, cx| {
                        panel.set_tool_stats(&ToolCallId::new("t-edit"), 6, 2, cx);
                        panel.request_permission(
                            1,
                            &ToolCallUpdate::new(
                                "t-run",
                                ToolCallUpdateFields::new()
                                    .title("Ejecutar `cargo test -p asteroid-chat`")
                                    .content(vec![ToolCallContent::Content(
                                        asteroid_acp::acp::schema::v1::Content::new(
                                            ContentBlock::Text(TextContent::new(
                                                "cargo test -p asteroid-chat --features test-support",
                                            )),
                                        ),
                                    )]),
                            ),
                            vec![
                                PermissionOption::new(
                                    "allow",
                                    "Permitir",
                                    PermissionOptionKind::AllowOnce,
                                ),
                                PermissionOption::new(
                                    "always",
                                    "Permitir siempre",
                                    PermissionOptionKind::AllowAlways,
                                ),
                                PermissionOption::new(
                                    "reject",
                                    "Rechazar",
                                    PermissionOptionKind::RejectOnce,
                                ),
                            ],
                            cx,
                        );
                    })
                    .ok();

                if !smoke_test {
                    return;
                }

                // Answer it ourselves and close the turn, so the smoke test
                // exercises the compacted card and the separator too.
                cx.background_executor().timer(STEP).await;
                window
                    .update(cx, |panel, _window, cx| {
                        panel.accept_permission(cx);
                        panel.handle_event(
                            AgentEvent::TurnEnded {
                                session_id: session(),
                                stop_reason: StopReason::EndTurn,
                            },
                            cx,
                        );
                        println!("entradas: {}", panel.entries().len());
                    })
                    .ok();

                cx.background_executor().timer(SMOKE_TAIL).await;
                // Close the window before quitting so no entity handle outlives
                // the app (gpui's leak detection is strict).
                window
                    .update(cx, |_panel, window, _cx| window.remove_window())
                    .ok();
                cx.background_executor().timer(SMOKE_TAIL).await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
}
