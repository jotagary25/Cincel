//! Demo of the chat panel: opens a window with a scripted agent that streams
//! markdown, runs a tool call with `+N −M` stats and asks for permission.
//!
//! ```text
//! cargo run -p cincel-chat --example chat_demo
//! cargo run -p cincel-chat --example chat_demo -- --long-messages
//! cargo run -p cincel-chat --example chat_demo -- --smoke-test
//! cargo run -p cincel-chat --example chat_demo -- --markdown-composer
//! ```
//!
//! `--markdown-composer` writes a Markdown message (a list, bold, inline code
//! and a fenced Python block) into the composer, leaves it there for
//! [`COMPOSER_PAUSE`] so the highlighting can be looked at, and then sends it,
//! so the rendered bubble can be looked at too.
//!
//! The script opens with two 400-character user messages (one with spaces,
//! one without) so the bubble's wrapping can be judged at a glance;
//! `--long-messages` stops right after them.
//!
//! Nothing here talks to a real agent: the events are the same
//! `cincel_acp::AgentEvent`s the ACP worker would send, so the panel cannot
//! tell the difference. What the panel emits is printed on stdout, which is
//! exactly what the workspace would forward to the worker.

use std::path::PathBuf;
use std::time::Duration;

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{
    AvailableCommand, AvailableCommandsUpdate, Content, ContentBlock, ContentChunk, Diff,
    PermissionOption, PermissionOptionKind, Plan, PlanEntry, PlanEntryPriority, PlanEntryStatus,
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelect,
    SessionConfigSelectOption, SessionId, SessionUpdate, StopReason, TextContent, ToolCall,
    ToolCallContent, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use cincel_chat::{
    ChatConnection, ChatEvent, ChatPanel, ChatSettings, ChatTheme, ConnectionBadge,
    ConversationSummary,
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
/// How long `--markdown-composer` leaves the draft in the composer.
const COMPOSER_PAUSE: Duration = Duration::from_secs(8);
/// The draft of `--markdown-composer`.
const MARKDOWN_DRAFT: &str = "## Plan\n\nRevisá **esto** antes de seguir:\n\n- el `parser` nuevo\n- los [tests](https://example.com)\n1. correr *todo*\n\n```python\ndef suma(a, b):\n    # suma dos números\n    return a + b\n```";

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
        .map(|row| format!("test cincel_chat::caso_{row} ... ok"))
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
            ToolCall::new("t-fmt", "cargo fmt -p cincel-chat").kind(ToolKind::Execute),
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
            ToolCall::new("t-clippy", "cargo clippy -p cincel-chat").kind(ToolKind::Execute),
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
            ToolCall::new("t-run", "cargo test -p cincel-chat").kind(ToolKind::Execute),
        )),
    ]
}

fn parse_flag(flag: &str) -> bool {
    std::env::args().skip(1).any(|arg| arg == flag)
}

/// The two long user messages and the agent's answers, each its own turn.
fn long_turns() -> Vec<(String, Vec<AgentEvent>)> {
    let spaced = "Quiero que revises el módulo de revisión entero: cómo se \
        agrupan los hunks, qué pasa cuando el agente vuelve a escribir sobre \
        un hunk ya aceptado, si el rebase de las líneas se sostiene cuando el \
        usuario edita a mano en el medio, y que me digas en qué casos el \
        contador de pendientes puede quedar desfasado respecto de lo que se \
        ve en el editor. Sin tocar código todavía."
        .to_string();
    let unbroken = "a1b2c3d4e5".repeat(40);
    let ended = || AgentEvent::TurnEnded {
        session_id: session(),
        stop_reason: StopReason::EndTurn,
    };
    vec![
        (
            spaced,
            vec![
                chunk("Lo leo primero. El punto delicado es `rebase_hunks`:\n\n"),
                chunk("```rust\nfn rebase(hunk: &mut Hunk, edit: &Edit) {\n    "),
                chunk("hunk.range = edit.map(hunk.range.clone());\n}\n```\n\n"),
                chunk("Si el usuario escribe dentro del rango, el hunk se parte en dos."),
                ended(),
            ],
        ),
        (
            unbroken,
            vec![
                chunk("Ese texto no tiene espacios: igual se corta dentro de la burbuja."),
                ended(),
            ],
        ),
    ]
}

fn main() {
    let smoke_test = parse_flag("--smoke-test");
    let long_only = parse_flag("--long-messages");
    let markdown_composer = parse_flag("--markdown-composer");

    // `AllAssets` embeds the whole Lucide catalog: the tool cards pick an icon
    // per tool kind (`02-visual.md` §7), outside gpui-kit's own subset.
    gpui_kit::application()
        .with_assets(AllAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            cincel_chat::init(cx);
            cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

            let bounds = Bounds::centered(None, size(px(420.), px(760.)), cx);
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
                    panel.set_connections(
                        vec![
                            ChatConnection {
                                id: "c-claude".into(),
                                agent_id: "claude-acp".into(),
                                label: "Claude · personal".into(),
                                identity: Some("gary@… · Max".into()),
                                last_used: "Usado hace 2 h".into(),
                                badge: ConnectionBadge::Connected,
                            },
                            ChatConnection {
                                id: "c-antigravity".into(),
                                agent_id: "antigravity-acp".into(),
                                label: "Antigravity · personal".into(),
                                identity: None,
                                last_used: "Usado hace 3 d".into(),
                                badge: ConnectionBadge::SessionExpired,
                            },
                        ],
                        cx,
                    );
                    panel.set_active_connection(Some("c-claude".into()), cx);
                    panel.set_conversations(vec![
                        ConversationSummary {
                            id: "c-hoy".into(),
                            agent_id: "claude-acp".into(),
                            agent_name: "Claude".into(),
                            title: "Revisar el diff inline".into(),
                            when: "hace 5 min".into(),
                            session_id: Some("s-hoy".into()),
                            connection_id: Some("c-claude".into()),
                            group: "Claude · personal".into(),
                        },
                        ConversationSummary {
                            id: "c-ayer".into(),
                            agent_id: "claude-acp".into(),
                            agent_name: "Claude".into(),
                            title: "Extraer el parser".into(),
                            when: "ayer".into(),
                            session_id: Some("s-ayer".into()),
                            connection_id: Some("c-claude".into()),
                            group: "Claude · personal".into(),
                        },
                        ConversationSummary {
                            id: "c-antigravity".into(),
                            agent_id: "antigravity-acp".into(),
                            agent_name: "Antigravity".into(),
                            title: "Probar el selector de modelo".into(),
                            when: "hace 2 d".into(),
                            session_id: None,
                            connection_id: Some("c-antigravity".into()),
                            group: "Antigravity · personal".into(),
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

                // El composer con Markdown: se deja el borrador a la vista y
                // después se envía, para ver el resaltado y la burbuja.
                if markdown_composer {
                    if window
                        .update(cx, |panel, window, cx| {
                            panel.focus_input(window, cx);
                            panel.set_input_text(MARKDOWN_DRAFT, window, cx);
                        })
                        .is_err()
                    {
                        return;
                    }
                    let pause = if smoke_test { STEP } else { COMPOSER_PAUSE };
                    cx.background_executor().timer(pause).await;
                    if window
                        .update(cx, |panel, window, cx| panel.send(window, cx))
                        .is_err()
                    {
                        return;
                    }
                    cx.background_executor().timer(STEP).await;
                    window
                        .update(cx, |panel, _window, cx| {
                            panel.handle_event(
                                chunk("Listo: `suma` ya está. Probala con:\n\n```python\nprint(suma(1, 2))\n```"),
                                cx,
                            );
                            panel.handle_event(
                                AgentEvent::TurnEnded {
                                    session_id: session(),
                                    stop_reason: StopReason::EndTurn,
                                },
                                cx,
                            );
                        })
                        .ok();
                    if !smoke_test {
                        return;
                    }
                }

                // Dos mensajes largos (con y sin espacios): la burbuja tiene
                // que partirlos a su ancho (`docs/etapas/etapa-3.md`).
                for (text, answer) in long_turns() {
                    cx.background_executor().timer(STEP).await;
                    if window
                        .update(cx, |panel, window, cx| {
                            panel.set_input_text(&text, window, cx);
                            panel.send(window, cx);
                        })
                        .is_err()
                    {
                        return;
                    }
                    for event in answer {
                        cx.background_executor().timer(STEP).await;
                        if window
                            .update(cx, |panel, _window, cx| panel.handle_event(event, cx))
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                if long_only && !smoke_test {
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

                // The permission request carries a responder only `cincel-acp`
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
                                    .title("Ejecutar `cargo test -p cincel-chat`")
                                    .content(vec![ToolCallContent::Content(
                                        cincel_acp::acp::schema::v1::Content::new(
                                            ContentBlock::Text(TextContent::new(
                                                "cargo test -p cincel-chat --features test-support",
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
