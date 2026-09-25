//! Interaction tests over the GPUI test harness: streaming, tool cards,
//! permissions, the `@` and `/` popovers, the footer selectors and the
//! transcript round trip.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use asteroid_acp::AgentEvent;
use asteroid_acp::acp::schema::v1::{
    AvailableCommand, AvailableCommandsUpdate, ConfigOptionUpdate, Content, ContentBlock,
    ContentChunk, Diff, PermissionOption, PermissionOptionKind, Plan, PlanEntryPriority,
    PlanEntryStatus, SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigSelect, SessionConfigSelectOption, SessionId, SessionMode, SessionModeState,
    SessionUpdate, StopReason, TextContent, ToolCall, ToolCallContent, ToolCallId, ToolCallStatus,
    ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use gpui::{
    AnyWindowHandle, Context, Entity, Modifiers, TestAppContext, VisualTestContext, Window, point,
    px, size,
};

use crate::actions::{bind_default_keys, default_key_bindings};
use crate::model::*;
use crate::panel::{ChatPanel, Popover, trigger_at};
use crate::settings::{ChatSettings, POPOVER_ROW_HEIGHT};
use crate::theme::ChatTheme;

/// Every [`crate::ChatEvent`] the panel emitted, as its `Debug` string.
type Recorder = Rc<RefCell<Vec<String>>>;

/// Opens a window with a panel and records what it emits.
fn open(cx: &mut TestAppContext) -> (Entity<ChatPanel>, VisualTestContext, Recorder) {
    // `gpui-kit`'s theme and `Input` bindings have to exist before the
    // composer's `TextareaState` is built.
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_default_keys(cx);
    });
    let window = cx.add_window(|window, cx| {
        ChatPanel::new(ChatTheme::default(), ChatSettings::default(), window, cx)
    });
    let panel = window.update(cx, |_, _, cx| cx.entity()).unwrap();
    let handle: AnyWindowHandle = window.into();
    let visual = VisualTestContext::from_window(handle, cx);

    let recorder: Recorder = Rc::new(RefCell::new(Vec::new()));
    let sink = recorder.clone();
    cx.update(|cx| {
        cx.subscribe(&panel, move |_panel, event: &crate::ChatEvent, _cx| {
            sink.borrow_mut().push(format!("{event:?}"));
        })
        .detach();
    });
    (panel, visual, recorder)
}

/// Opens a panel that already has a session, which is what sending needs.
fn open_with_session(cx: &mut TestAppContext) -> (Entity<ChatPanel>, VisualTestContext, Recorder) {
    let (panel, mut visual, recorder) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_agents(vec![ChatAgent::installed("claude-acp", "Claude")], cx);
        panel.set_active_agent("claude-acp", cx);
        panel.handle_event(
            AgentEvent::SessionCreated {
                session_id: SessionId::new("s1"),
                modes: None,
                config_options: Vec::new(),
                commands: Vec::new(),
            },
            cx,
        );
    });
    recorder.borrow_mut().clear();
    (panel, visual, recorder)
}

fn text_chunk(text: &str) -> AgentEvent {
    AgentEvent::Update {
        session_id: SessionId::new("s1"),
        update: Box::new(SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        ))),
    }
}

fn thought_chunk(text: &str) -> AgentEvent {
    AgentEvent::Update {
        session_id: SessionId::new("s1"),
        update: Box::new(SessionUpdate::AgentThoughtChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        ))),
    }
}

fn update(update: SessionUpdate) -> AgentEvent {
    AgentEvent::Update {
        session_id: SessionId::new("s1"),
        update: Box::new(update),
    }
}

fn permission_options() -> Vec<PermissionOption> {
    vec![
        PermissionOption::new("allow", "Permitir", PermissionOptionKind::AllowOnce),
        PermissionOption::new(
            "always",
            "Permitir siempre",
            PermissionOptionKind::AllowAlways,
        ),
        PermissionOption::new("reject", "Rechazar", PermissionOptionKind::RejectOnce),
    ]
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

/// Runs `f` with both a `Window` and the panel's `Context`.
fn with_window<R>(
    panel: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
    f: impl FnOnce(&mut ChatPanel, &mut Window, &mut Context<ChatPanel>) -> R,
) -> R {
    visual.update(|window, cx| panel.update(cx, |panel, cx| f(panel, window, cx)))
}

/// Everything the recorder saw, joined, for `contains` assertions.
fn emitted(recorder: &Recorder) -> String {
    recorder.borrow().join("\n")
}

// --------------------------------------------------------------- streaming

#[gpui::test]
fn streamed_chunks_land_in_one_entry(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(text_chunk("Hola"), cx);
        panel.handle_event(text_chunk(", "), cx);
        panel.handle_event(text_chunk("mundo"), cx);
    });
    panel.update(&mut visual, |panel, _cx| {
        assert_eq!(panel.entries().len(), 1);
        match &panel.entries()[0] {
            Entry::AgentText(text) => {
                assert_eq!(text.markdown, "Hola, mundo");
                assert!(text.streaming);
                assert!(text.view.is_some(), "falta el estado de markdown");
            }
            other => panic!("se esperaba AgentText, llegó {other:?}"),
        }
    });
}

#[gpui::test]
fn a_tool_call_between_chunks_starts_a_new_entry(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(text_chunk("uno"), cx);
        panel.handle_event(
            update(SessionUpdate::ToolCall(ToolCall::new("t1", "Leer main.rs"))),
            cx,
        );
        panel.handle_event(text_chunk("dos"), cx);
        assert_eq!(panel.entries().len(), 3);
        assert!(matches!(panel.entries()[2], Entry::AgentText(_)));
    });
}

#[gpui::test]
fn the_turn_end_closes_the_stream_and_adds_a_separator(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(text_chunk("hola"), cx);
        panel.handle_event(
            AgentEvent::TurnEnded {
                session_id: SessionId::new("s1"),
                stop_reason: StopReason::EndTurn,
            },
            cx,
        );
        match &panel.entries()[0] {
            Entry::AgentText(text) => assert!(!text.streaming),
            other => panic!("se esperaba AgentText, llegó {other:?}"),
        }
        match &panel.entries()[1] {
            Entry::TurnSeparator(separator) => {
                assert_eq!(separator.stop_reason, "turno terminado");
            }
            other => panic!("se esperaba TurnSeparator, llegó {other:?}"),
        }
        assert_eq!(panel.status(), AgentStatus::Ready);
    });
}

#[gpui::test]
fn a_thought_starts_collapsed(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(thought_chunk("mmm"), cx);
        panel.handle_event(thought_chunk("…"), cx);
        match &panel.entries()[0] {
            Entry::AgentThought(thought) => {
                assert_eq!(thought.text, "mmm…");
                assert!(thought.collapsed);
            }
            other => panic!("se esperaba AgentThought, llegó {other:?}"),
        }
        panel.toggle_entry(0, cx);
        match &panel.entries()[0] {
            Entry::AgentThought(thought) => assert!(!thought.collapsed),
            other => panic!("se esperaba AgentThought, llegó {other:?}"),
        }
    });
}

#[gpui::test]
fn every_thought_chunk_lands_in_the_same_collapsed_entry(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        for chunk in ["Reviso el módulo", " y decido", " por dónde empezar."] {
            panel.handle_event(thought_chunk(chunk), cx);
        }
        assert_eq!(panel.entries().len(), 1, "un solo bloque de pensamiento");
        match &panel.entries()[0] {
            Entry::AgentThought(thought) => {
                assert_eq!(thought.text, "Reviso el módulo y decido por dónde empezar.");
                assert!(thought.collapsed, "arranca plegado");
                assert!(thought.view.is_some());
            }
            other => panic!("se esperaba AgentThought, llegó {other:?}"),
        }
        assert_eq!(panel.status(), AgentStatus::Thinking);
        // El texto del agente cierra el pensamiento y abre otra entrada.
        panel.handle_event(text_chunk("Listo."), cx);
        assert_eq!(panel.entries().len(), 2);
    });
}

// -------------------------------------------------------------- tool calls

#[gpui::test]
fn a_tool_call_walks_through_its_states(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            update(SessionUpdate::ToolCall(
                ToolCall::new("t1", "Ejecutar cargo test").kind(ToolKind::Execute),
            )),
            cx,
        );
        match &panel.entries()[0] {
            Entry::ToolCall(call) => assert_eq!(call.status, ToolCallStatus::Pending),
            other => panic!("se esperaba ToolCall, llegó {other:?}"),
        }

        panel.handle_event(
            update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "t1",
                ToolCallUpdateFields::new().status(ToolCallStatus::InProgress),
            ))),
            cx,
        );
        match &panel.entries()[0] {
            Entry::ToolCall(call) => assert_eq!(call.status, ToolCallStatus::InProgress),
            other => panic!("se esperaba ToolCall, llegó {other:?}"),
        }

        panel.handle_event(
            update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "t1",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Completed)
                    .content(vec![ToolCallContent::Content(Content::new(
                        ContentBlock::Text(TextContent::new("ok, 12 tests")),
                    ))]),
            ))),
            cx,
        );
        match &panel.entries()[0] {
            Entry::ToolCall(call) => {
                assert_eq!(call.status, ToolCallStatus::Completed);
                // An `execute` card shows the command plus its output.
                assert!(matches!(
                    call.content.first(),
                    Some(ToolContent::Command { .. })
                ));
            }
            other => panic!("se esperaba ToolCall, llegó {other:?}"),
        }
        assert_eq!(panel.entries().len(), 1, "la tarjeta no se duplica");
    });
}

#[gpui::test]
fn a_command_card_folds_when_it_works_and_opens_when_it_fails(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            update(SessionUpdate::ToolCall(
                ToolCall::new("t1", "cargo test -p asteroid-chat").kind(ToolKind::Execute),
            )),
            cx,
        );
        panel.handle_event(
            update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "t1",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Completed)
                    .content(vec![ToolCallContent::Content(Content::new(
                        ContentBlock::Text(TextContent::new("ok, 12 tests")),
                    ))]),
            ))),
            cx,
        );
        match &panel.entries()[0] {
            Entry::ToolCall(call) => {
                assert!(!call.expanded, "una orden que salió bien queda plegada");
                assert!(matches!(
                    call.content.first(),
                    Some(ToolContent::Command { .. })
                ));
            }
            other => panic!("se esperaba ToolCall, llegó {other:?}"),
        }

        panel.handle_event(
            update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "t1",
                ToolCallUpdateFields::new().status(ToolCallStatus::Failed),
            ))),
            cx,
        );
        match &panel.entries()[0] {
            Entry::ToolCall(call) => assert!(call.expanded, "una que falla se abre sola"),
            other => panic!("se esperaba ToolCall, llegó {other:?}"),
        }

        // "ver más" es estado del panel, no del agente.
        panel.toggle_tool_output(0, cx);
        match &panel.entries()[0] {
            Entry::ToolCall(call) => assert!(call.output_expanded),
            other => panic!("se esperaba ToolCall, llegó {other:?}"),
        }
    });
}

#[gpui::test]
fn a_cancelled_turn_marks_the_open_tool_calls(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            update(SessionUpdate::ToolCall(ToolCall::new("t1", "Buscar"))),
            cx,
        );
        panel.handle_event(
            AgentEvent::ToolCallsCancelled {
                session_id: SessionId::new("s1"),
                ids: vec![ToolCallId::new("t1")],
            },
            cx,
        );
        match &panel.entries()[0] {
            Entry::ToolCall(call) => assert_eq!(call.status, ToolCallStatus::Failed),
            other => panic!("se esperaba ToolCall, llegó {other:?}"),
        }
    });
}

#[gpui::test]
fn the_workspace_sets_the_edit_stats(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            update(SessionUpdate::ToolCall(
                ToolCall::new("t1", "Editar main.rs").kind(ToolKind::Edit),
            )),
            cx,
        );
        panel.set_tool_stats(&ToolCallId::new("t1"), 7, 2, cx);
        match &panel.entries()[0] {
            Entry::ToolCall(call) => assert_eq!(call.stats, Some((7, 2))),
            other => panic!("se esperaba ToolCall, llegó {other:?}"),
        }
    });
}

#[gpui::test]
fn an_edit_card_never_keeps_the_whole_diff(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    let long = (1..=40)
        .map(|row| format!("línea {row}"))
        .collect::<Vec<_>>()
        .join("\n");
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            update(SessionUpdate::ToolCall(
                ToolCall::new("t1", "Editar main.rs")
                    .kind(ToolKind::Edit)
                    .content(vec![ToolCallContent::Diff(Diff::new(
                        "/proyecto/src/main.rs",
                        long.clone(),
                    ))]),
            )),
            cx,
        );
        match &panel.entries()[0] {
            Entry::ToolCall(call) => match call.content.first() {
                Some(ToolContent::Edit { path, preview }) => {
                    assert_eq!(path, &PathBuf::from("/proyecto/src/main.rs"));
                    assert_eq!(preview.len(), 3, "como mucho 3 líneas de contexto");
                }
                other => panic!("se esperaba Edit, llegó {other:?}"),
            },
            other => panic!("se esperaba ToolCall, llegó {other:?}"),
        }
        panel.open_file_at_hunk(PathBuf::from("/proyecto/src/main.rs"), cx);
    });
    assert!(emitted(&recorder).contains("OpenFileAtHunk"));
}

// -------------------------------------------------------------- permissions

#[gpui::test]
fn a_permission_blocks_sending(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.request_permission(
            7,
            &ToolCallUpdate::new("t1", ToolCallUpdateFields::new().title("Borrar .env")),
            permission_options(),
            cx,
        );
        assert!(panel.is_awaiting_permission());
        assert_eq!(panel.status(), AgentStatus::WaitingPermission);
    });
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("hola", window, cx);
        panel.send(window, cx);
    });
    assert!(
        !emitted(&recorder).contains("Prompt"),
        "no debería enviarse con un permiso pendiente"
    );
}

#[gpui::test]
fn enter_answers_the_first_option_and_compacts_the_card(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.request_permission(
            7,
            &ToolCallUpdate::new("t1", ToolCallUpdateFields::new().title("Escribir main.rs")),
            permission_options(),
            cx,
        );
        panel.accept_permission(cx);
        assert!(!panel.is_awaiting_permission());
        match panel.entries().last() {
            Some(Entry::Permission(permission)) => {
                assert_eq!(permission.answered.as_deref(), Some("Permitir"));
                assert!(permission.allowed);
            }
            other => panic!("se esperaba Permission, llegó {other:?}"),
        }
    });
    let log = emitted(&recorder);
    assert!(log.contains("RespondPermission"), "{log}");
    assert!(log.contains("allow"), "{log}");
}

#[gpui::test]
fn escape_answers_the_first_reject_option(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.request_permission(
            9,
            &ToolCallUpdate::new("t1", ToolCallUpdateFields::new().title("Borrar Cargo.lock")),
            permission_options(),
            cx,
        );
        panel.reject_permission(cx);
        match panel.entries().last() {
            Some(Entry::Permission(permission)) => {
                assert_eq!(permission.answered.as_deref(), Some("Rechazar"));
                assert!(!permission.allowed);
            }
            other => panic!("se esperaba Permission, llegó {other:?}"),
        }
    });
    let log = emitted(&recorder);
    assert!(log.contains("RespondPermission"), "{log}");
    assert!(log.contains("reject"), "{log}");
}

#[gpui::test]
fn enter_answers_the_permission_with_the_composer_focused(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.focus_input(window, cx);
        panel.request_permission(
            4,
            &ToolCallUpdate::new("t1", ToolCallUpdateFields::new().title("Correr cargo test")),
            permission_options(),
            cx,
        );
    });
    visual.run_until_parked();
    // La tecla atraviesa el selector (que no tiene nada abierto) y el textarea
    // hasta `chat::accept_permission`.
    visual.simulate_keystrokes("enter");
    panel.update(&mut visual, |panel, _cx| {
        assert!(!panel.is_awaiting_permission());
    });
    let log = emitted(&recorder);
    assert!(log.contains("RespondPermission"), "{log}");
    assert!(log.contains("allow"), "{log}");
}

#[gpui::test]
fn the_permission_context_is_active_only_while_it_waits(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        assert!(!format!("{:?}", panel.key_context()).contains("permission"));
        panel.request_permission(
            1,
            &ToolCallUpdate::new("t1", ToolCallUpdateFields::new().title("Correr rm")),
            permission_options(),
            cx,
        );
        assert!(format!("{:?}", panel.key_context()).contains("permission"));
        panel.accept_permission(cx);
        assert!(!format!("{:?}", panel.key_context()).contains("permission"));
    });
}

// ----------------------------------------------------------- mentions and /

#[gpui::test]
fn the_at_picker_writes_the_token_where_the_caret_is(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_file_candidates(
            vec![
                PathBuf::from("/proyecto/src/main.rs"),
                PathBuf::from("/proyecto/src/lib.rs"),
            ],
            cx,
        );
        panel.set_input_text("mirá @mai", window, cx);
    });
    visual.run_until_parked();

    panel.update(&mut visual, |panel, _cx| {
        assert!(
            matches!(panel.popover(), Popover::Files { .. }),
            "el `@` debería abrir el selector: {:?}",
            panel.popover()
        );
        let files = panel.filtered_files();
        assert_eq!(files, vec![PathBuf::from("/proyecto/src/main.rs")]);
    });

    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.confirm_popover(window, cx);
        assert_eq!(
            panel.mentions(),
            [Mention {
                token: "@main.rs".to_string(),
                path: PathBuf::from("/proyecto/src/main.rs"),
            }],
            "la mención queda mapeada al path absoluto"
        );
        assert_eq!(
            panel.input_text(cx),
            "mirá @main.rs ",
            "el token se escribe en el texto, no arriba del input"
        );
        panel.send(window, cx);
    });

    let log = emitted(&recorder);
    assert!(log.contains("ResourceLink"), "{log}");
    assert!(log.contains("file:///proyecto/src/main.rs"), "{log}");
}

#[gpui::test]
fn a_mention_in_the_middle_splits_the_prompt_in_order(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("mira este archivo ", window, cx);
        panel.insert_mention(PathBuf::from("/proyecto/calculadora.py"), window, cx);
        assert_eq!(panel.input_text(cx), "mira este archivo @calculadora.py ");
        let text = format!("{}, que te parece", panel.input_text(cx).trim_end());
        panel.set_input_text(&text, window, cx);

        let blocks = panel.draft_blocks(cx);
        assert_eq!(blocks.len(), 3, "{blocks:?}");
        // El espaciado que escribió el autor viaja tal cual; solo se recortan
        // los bordes del mensaje.
        assert!(matches!(&blocks[0], MessageBlock::Text(text) if text == "mira este archivo "));
        assert!(
            matches!(&blocks[1], MessageBlock::File(path) if path == std::path::Path::new("/proyecto/calculadora.py"))
        );
        assert!(matches!(&blocks[2], MessageBlock::Text(text) if text == ", que te parece"));

        panel.send(window, cx);
    });

    let log = emitted(&recorder);
    assert!(log.contains("file:///proyecto/calculadora.py"), "{log}");
    assert!(log.contains("que te parece"), "{log}");
}

#[gpui::test]
fn a_token_deleted_by_hand_is_not_a_mention_any_more(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.insert_mention(PathBuf::from("/proyecto/calculadora.py"), window, cx);
        // El autor borra el token a mano y deja solo su texto.
        panel.set_input_text("revisá la calculadora", window, cx);
        assert!(
            panel
                .draft_blocks(cx)
                .iter()
                .all(|block| !matches!(block, MessageBlock::File(_))),
            "un token borrado no viaja como enlace"
        );
        panel.send(window, cx);
    });
    let log = emitted(&recorder);
    assert!(!log.contains("ResourceLink"), "{log}");
    assert!(log.contains("revisá la calculadora"), "{log}");
}

#[gpui::test]
fn two_files_with_the_same_name_get_different_tokens(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_project_root(Some(PathBuf::from("/proyecto")), cx);
        panel.insert_mention(PathBuf::from("/proyecto/src/main.rs"), window, cx);
        panel.insert_mention(PathBuf::from("/proyecto/tests/main.rs"), window, cx);

        let tokens: Vec<&str> = panel
            .mentions()
            .iter()
            .map(|mention| mention.token.as_str())
            .collect();
        assert_eq!(tokens, ["@src/main.rs", "@tests/main.rs"]);
        assert_eq!(
            panel.input_text(cx),
            "@src/main.rs @tests/main.rs ",
            "el token ya escrito también se desambigua"
        );

        let blocks = panel.draft_blocks(cx);
        assert_eq!(
            blocks
                .iter()
                .filter(|block| matches!(block, MessageBlock::File(_)))
                .count(),
            2,
            "{blocks:?}"
        );
    });
}

#[gpui::test]
fn the_at_picker_asks_the_workspace_for_the_files(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("@lib", window, cx)
    });
    visual.run_until_parked();
    let log = emitted(&recorder);
    assert!(log.contains("RequestFileList"), "{log}");
    assert!(log.contains("lib"), "{log}");
}

#[gpui::test]
fn the_slash_picker_inserts_the_command(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            update(SessionUpdate::AvailableCommandsUpdate(
                AvailableCommandsUpdate::new(vec![
                    AvailableCommand::new("create_plan", "Arma un plan"),
                    AvailableCommand::new("review", "Revisa el código"),
                ]),
            )),
            cx,
        );
    });
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("/cre", window, cx)
    });
    visual.run_until_parked();
    with_window(&panel, &mut visual, |panel, window, cx| {
        assert_eq!(panel.filtered_commands().len(), 1);
        panel.confirm_popover(window, cx);
        assert_eq!(panel.input_text(cx), "/create_plan ");
        assert_eq!(panel.popover(), &Popover::Closed);
    });
}

/// The four candidates of the picker tests, in worktree order.
///
/// `lic` matches three of them at three different ranks: the name starts with
/// it, the name contains it, the directory contains it.
fn picker_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/proyecto/publicar.rs"),
        PathBuf::from("/proyecto/publico/index.html"),
        PathBuf::from("/proyecto/src/licencia.rs"),
        PathBuf::from("/proyecto/src/lib.rs"),
    ]
}

/// A panel with a session, a project root, the candidates above, the composer
/// focused and `@lic` already typed, in a window of a known size (the mouse
/// test clicks at absolute coordinates).
fn open_file_picker(cx: &mut TestAppContext) -> (Entity<ChatPanel>, VisualTestContext, Recorder) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    visual.simulate_resize(size(px(400.), px(720.)));
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_project_root(Some(PathBuf::from("/proyecto")), cx);
        panel.set_file_candidates(picker_candidates(), cx);
        panel.focus_input(window, cx);
        panel.set_input_text("mirá @lic", window, cx);
    });
    visual.run_until_parked();
    recorder.borrow_mut().clear();
    (panel, visual, recorder)
}

#[gpui::test]
fn the_picker_puts_the_name_matches_first(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_file_picker(cx);
    panel.update(&mut visual, |panel, _cx| {
        assert_eq!(
            panel.filtered_files(),
            vec![
                // el nombre empieza con `lic`
                PathBuf::from("/proyecto/src/licencia.rs"),
                // el nombre lo contiene
                PathBuf::from("/proyecto/publicar.rs"),
                // solo la carpeta lo contiene
                PathBuf::from("/proyecto/publico/index.html"),
            ]
        );
    });
}

#[gpui::test]
fn enter_confirms_the_mention_and_does_not_send(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_file_picker(cx);
    visual.simulate_keystrokes("enter");

    panel.update(&mut visual, |panel, cx| {
        assert_eq!(
            panel
                .mentions()
                .iter()
                .map(|mention| mention.path.clone())
                .collect::<Vec<_>>(),
            [PathBuf::from("/proyecto/src/licencia.rs")],
            "`Enter` elige la fila resaltada"
        );
        assert_eq!(panel.popover(), &Popover::Closed);
        assert_eq!(panel.input_text(cx), "mirá @licencia.rs ");
        assert!(
            panel.entries().is_empty(),
            "no debería haberse enviado nada: {:?}",
            panel.entries()
        );
    });
    assert!(
        !emitted(&recorder).contains("Prompt"),
        "`Enter` sobre el selector no envía el mensaje: {}",
        emitted(&recorder)
    );
}

#[gpui::test]
fn the_arrows_move_the_selection_and_tab_confirms_it(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_file_picker(cx);
    visual.simulate_keystrokes("down");
    panel.update(&mut visual, |panel, _cx| {
        assert_eq!(panel.popover_selection(), Some(1), "`↓` baja una fila");
    });
    visual.simulate_keystrokes("down up");
    panel.update(&mut visual, |panel, _cx| {
        assert_eq!(panel.popover_selection(), Some(1), "`↑` vuelve");
    });
    visual.simulate_keystrokes("tab");
    panel.update(&mut visual, |panel, _cx| {
        assert_eq!(
            panel
                .mentions()
                .iter()
                .map(|mention| mention.path.clone())
                .collect::<Vec<_>>(),
            [PathBuf::from("/proyecto/publicar.rs")],
            "`Tab` confirma la fila seleccionada"
        );
    });
    assert!(!emitted(&recorder).contains("Prompt"));
}

#[gpui::test]
fn escape_closes_the_picker_without_cancelling_the_turn(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_file_picker(cx);
    visual.simulate_keystrokes("escape");
    panel.update(&mut visual, |panel, cx| {
        assert_eq!(panel.popover(), &Popover::Closed);
        assert_eq!(panel.input_text(cx), "mirá @lic", "el texto queda intacto");
        assert!(panel.mentions().is_empty());
    });
    let log = emitted(&recorder);
    assert!(!log.contains("Cancel"), "{log}");
}

#[gpui::test]
fn clicking_a_row_inserts_that_mention_without_sending(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_file_picker(cx);
    // The popover floats above the composer: 3 rows of 24 px plus its 4 px of
    // padding, anchored 108 px over the bottom edge of a 720 px window.
    let rows = 3.;
    let top = 720. - 108. - (rows * POPOVER_ROW_HEIGHT + 8.);
    let second_row = top + 4. + POPOVER_ROW_HEIGHT * 1.5;
    visual.simulate_click(point(px(60.), px(second_row)), Modifiers::default());
    visual.run_until_parked();

    panel.update(&mut visual, |panel, _cx| {
        assert_eq!(
            panel
                .mentions()
                .iter()
                .map(|mention| mention.path.clone())
                .collect::<Vec<_>>(),
            [PathBuf::from("/proyecto/publicar.rs")],
            "el clic inserta la fila que se tocó"
        );
        assert_eq!(panel.popover(), &Popover::Closed);
        assert!(
            panel.entries().is_empty(),
            "el clic no envía: {:?}",
            panel.entries()
        );
    });
    assert!(
        !emitted(&recorder).contains("Prompt"),
        "{}",
        emitted(&recorder)
    );
}

#[gpui::test]
fn enter_still_sends_when_no_popover_is_open(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.focus_input(window, cx);
        panel.set_input_text("hola", window, cx);
    });
    visual.run_until_parked();
    visual.simulate_keystrokes("enter");
    panel.update(&mut visual, |panel, cx| {
        assert_eq!(panel.input_text(cx), "");
        assert_eq!(panel.entries().len(), 1, "un solo mensaje, no dos");
    });
    assert!(
        emitted(&recorder).contains("Prompt"),
        "{}",
        emitted(&recorder)
    );
}

// ------------------------------------------------------------- selectors

#[gpui::test]
fn changing_a_config_option_emits_the_command(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            update(SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
                vec![model_option()],
            ))),
            cx,
        );
        assert_eq!(panel.config_options().len(), 1);
        let id = panel.config_options()[0].id.clone();
        panel.choose_config_value(&id, "opus".into(), cx);
    });
    let log = emitted(&recorder);
    assert!(log.contains("SetConfigOption"), "{log}");
    assert!(log.contains("opus"), "{log}");
}

#[gpui::test]
fn the_agents_answer_overrides_the_optimistic_value(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            update(SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
                vec![model_option()],
            ))),
            cx,
        );
        let id = panel.config_options()[0].id.clone();
        panel.choose_config_value(&id, "opus".into(), cx);
        assert_eq!(
            crate::panel::current_label(&panel.config_options()[0]),
            "Opus"
        );
        // The agent says it stayed on Sonnet: the footer follows the agent.
        panel.handle_event(
            update(SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
                vec![model_option()],
            ))),
            cx,
        );
        assert_eq!(
            crate::panel::current_label(&panel.config_options()[0]),
            "Sonnet"
        );
    });
}

#[gpui::test]
fn changing_the_legacy_mode_emits_set_mode(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            AgentEvent::SessionCreated {
                session_id: SessionId::new("s1"),
                modes: Some(SessionModeState::new(
                    "normal",
                    vec![
                        SessionMode::new("normal", "Normal"),
                        SessionMode::new("plan", "Plan"),
                    ],
                )),
                config_options: Vec::new(),
                commands: Vec::new(),
            },
            cx,
        );
        panel.choose_mode("plan".into(), cx);
    });
    let log = emitted(&recorder);
    assert!(log.contains("SetMode"), "{log}");
    assert!(log.contains("plan"), "{log}");
}

#[gpui::test]
fn changing_the_autonomy_tells_the_workspace(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        assert_eq!(panel.autonomy(), asteroid_acp::AutonomyMode::ReviewAfter);
        panel.set_autonomy(asteroid_acp::AutonomyMode::AskBefore, cx);
        assert_eq!(panel.autonomy(), asteroid_acp::AutonomyMode::AskBefore);
    });
    assert!(emitted(&recorder).contains("AutonomyChanged(AskBefore)"));
}

// ------------------------------------------------------------ transcript

#[gpui::test]
fn the_transcript_survives_a_round_trip(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(text_chunk("# Título\n\n```rust\nfn main() {}\n```"), cx);
        panel.handle_event(
            update(SessionUpdate::ToolCall(
                ToolCall::new("t1", "Editar main.rs").kind(ToolKind::Edit),
            )),
            cx,
        );
        panel.set_tool_stats(&ToolCallId::new("t1"), 3, 1, cx);
        panel.handle_event(
            update(SessionUpdate::Plan(Plan::new(vec![
                asteroid_acp::acp::schema::v1::PlanEntry::new(
                    "Leer el código",
                    PlanEntryPriority::High,
                    PlanEntryStatus::Completed,
                ),
            ]))),
            cx,
        );
        panel.set_autonomy(asteroid_acp::AutonomyMode::AlwaysApply, cx);
    });

    let dump = panel.update(&mut visual, |panel, _cx| panel.export_transcript());
    let json = serde_json::to_string(&dump).expect("serializa");
    let restored: TranscriptDump = serde_json::from_str(&json).expect("deserializa");

    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.clear(window, cx);
        assert!(panel.entries().is_empty());
        panel.import_transcript(restored, cx);
    });

    panel.update(&mut visual, |panel, _cx| {
        assert_eq!(panel.entries().len(), 3);
        assert_eq!(panel.autonomy(), asteroid_acp::AutonomyMode::AlwaysApply);
        match &panel.entries()[0] {
            Entry::AgentText(text) => {
                assert!(text.markdown.starts_with("# Título"));
                assert!(!text.streaming, "un transcript restaurado no está en vivo");
                assert!(
                    text.view.is_some(),
                    "el markdown se reconstruye al importar"
                );
            }
            other => panic!("se esperaba AgentText, llegó {other:?}"),
        }
        match &panel.entries()[1] {
            Entry::ToolCall(call) => assert_eq!(call.stats, Some((3, 1))),
            other => panic!("se esperaba ToolCall, llegó {other:?}"),
        }
        match &panel.entries()[2] {
            Entry::Plan(plan) => assert_eq!(plan.items.len(), 1),
            other => panic!("se esperaba Plan, llegó {other:?}"),
        }
    });
}

#[gpui::test]
fn an_imported_permission_is_no_longer_pending(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.request_permission(
            3,
            &ToolCallUpdate::new("t1", ToolCallUpdateFields::new().title("Escribir")),
            permission_options(),
            cx,
        );
    });
    let dump = panel.update(&mut visual, |panel, _cx| panel.export_transcript());
    panel.update(&mut visual, |panel, cx| {
        panel.import_transcript(dump, cx);
        assert!(!panel.is_awaiting_permission());
        match panel.entries().last() {
            Some(Entry::Permission(permission)) => {
                assert_eq!(permission.answered.as_deref(), Some("Sin responder"));
            }
            other => panic!("se esperaba Permission, llegó {other:?}"),
        }
    });
}

// ----------------------------------------------------------------- varios

#[gpui::test]
fn sending_emits_the_prompt_and_clears_the_input(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("¿qué hace esta función?", window, cx);
        panel.send(window, cx);
        assert_eq!(panel.input_text(cx), "");
        assert_eq!(panel.status(), AgentStatus::Thinking);
        assert!(matches!(panel.entries()[0], Entry::UserMessage(_)));
    });
    let log = emitted(&recorder);
    assert!(log.contains("Prompt"), "{log}");
    assert!(log.contains("qué hace esta función"), "{log}");
}

#[gpui::test]
fn sending_without_a_session_leaves_a_notice(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("hola", window, cx);
        panel.send(window, cx);
    });
    panel.update(&mut visual, |panel, _cx| {
        assert!(matches!(panel.entries().last(), Some(Entry::Notice(_))));
    });
    assert!(!emitted(&recorder).contains("Prompt"));
}

#[gpui::test]
fn cancelling_a_turn_emits_cancel(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(text_chunk("pensando"), cx);
        panel.cancel_turn(cx);
        assert_eq!(panel.status(), AgentStatus::Ready);
    });
    assert!(emitted(&recorder).contains("Cancel"));
}

#[gpui::test]
fn a_dead_agent_shows_its_last_lines(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(AgentEvent::Stderr("no se pudo abrir el socket".into()), cx);
        panel.handle_event(
            AgentEvent::Exited {
                code: Some(1),
                stderr_tail: String::new(),
            },
            cx,
        );
        assert_eq!(panel.status(), AgentStatus::Disconnected);
        match panel.entries().last() {
            Some(Entry::Notice(notice)) => {
                assert_eq!(notice.level, NoticeLevel::Error);
                assert!(notice.text.contains("no se pudo abrir el socket"));
            }
            other => panic!("se esperaba Notice, llegó {other:?}"),
        }
    });
}

#[gpui::test]
fn an_auth_required_event_shows_the_card(cx: &mut TestAppContext) {
    use asteroid_acp::acp::schema::v1::{AuthMethod, AuthMethodTerminal};
    let (panel, mut visual, _) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            AgentEvent::AuthRequired {
                methods: vec![AuthMethod::Terminal(
                    AuthMethodTerminal::new("terminal", "Iniciar sesión")
                        .args(vec!["auth".to_string(), "login".to_string()]),
                )],
            },
            cx,
        );
        assert_eq!(panel.status(), AgentStatus::AuthRequired);
        match panel.entries().last() {
            Some(Entry::AuthRequired(auth)) => {
                assert_eq!(auth.command(), Some("auth login"));
            }
            other => panic!("se esperaba AuthRequired, llegó {other:?}"),
        }
    });
}

#[gpui::test]
fn a_plan_update_replaces_the_previous_one(cx: &mut TestAppContext) {
    use asteroid_acp::acp::schema::v1::PlanEntry as WirePlanEntry;
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            update(SessionUpdate::Plan(Plan::new(vec![WirePlanEntry::new(
                "uno",
                PlanEntryPriority::High,
                PlanEntryStatus::Pending,
            )]))),
            cx,
        );
        panel.handle_event(
            update(SessionUpdate::Plan(Plan::new(vec![
                WirePlanEntry::new("uno", PlanEntryPriority::High, PlanEntryStatus::Completed),
                WirePlanEntry::new("dos", PlanEntryPriority::Low, PlanEntryStatus::Pending),
            ]))),
            cx,
        );
        assert_eq!(panel.entries().len(), 1, "el plan no se duplica");
        match &panel.entries()[0] {
            Entry::Plan(plan) => {
                assert_eq!(plan.items.len(), 2);
                assert_eq!(plan.items[0].status, PlanEntryStatus::Completed);
            }
            other => panic!("se esperaba Plan, llegó {other:?}"),
        }
    });
}

#[gpui::test]
fn the_placeholder_names_the_active_agent(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_agents(vec![ChatAgent::installed("claude-acp", "Claude")], cx);
        panel.set_active_agent("claude-acp", cx);
        assert_eq!(panel.placeholder(), "Escribí un mensaje para Claude…");
    });
}

#[gpui::test]
fn the_agent_selector_keeps_the_choice_when_the_list_is_reloaded(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_agents(
            vec![
                ChatAgent::installed("claude-acp", "Claude"),
                ChatAgent {
                    id: "gemini".into(),
                    name: "Gemini".into(),
                    installed: false,
                    hint: Some("npx @google/gemini-cli".into()),
                },
            ],
            cx,
        );
        panel.set_active_agent("gemini", cx);
        panel.set_agents(
            vec![
                ChatAgent::installed("gemini", "Gemini"),
                ChatAgent::installed("claude-acp", "Claude"),
            ],
            cx,
        );
        assert_eq!(
            panel.active_agent().map(|agent| agent.id.as_str()),
            Some("gemini")
        );
    });
}

// --------------------------------------------------------------- unit bits

#[test]
fn the_trigger_is_only_recognised_at_the_start_of_a_word() {
    assert_eq!(
        trigger_at("mira @mai", 9),
        Some(('@', 5, "mai".to_string()))
    );
    assert_eq!(trigger_at("/plan", 5), Some(('/', 0, "plan".to_string())));
    assert_eq!(trigger_at("correo@dominio", 14), None);
    assert_eq!(trigger_at("src/main.rs", 11), None);
    assert_eq!(trigger_at("sin nada", 8), None);
}

#[test]
fn the_default_bindings_cover_the_spec_rows() {
    let bindings = default_key_bindings();
    let rendered: Vec<String> = bindings
        .iter()
        .map(|binding| format!("{binding:?}"))
        .collect();
    let all = rendered.join("\n");
    for expected in [
        "chat::send",
        "chat::newline",
        "chat::cancel_turn",
        "chat::focus_input",
        "chat::new_session",
        "chat::accept_permission",
        "chat::reject_permission",
    ] {
        assert!(all.contains(expected), "falta {expected} en {all}");
    }
}

#[test]
fn the_popover_keys_are_bound_on_the_composer_too() {
    let bindings = default_key_bindings();
    let rendered: Vec<String> = bindings
        .iter()
        .map(|binding| format!("{binding:?}"))
        .collect();
    // `Chat > Input` se guarda como un `Descendant`, que es lo que le da al
    // selector la misma profundidad que al textarea.
    let composer: Vec<&String> = rendered
        .iter()
        .filter(|row| row.contains("Descendant"))
        .collect();
    let all = composer
        .iter()
        .map(|row| row.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(all.contains("Chat") && all.contains("Input"), "{all}");
    // Sin estas cinco, gpui-kit's `Input` se queda con la tecla y el selector
    // no responde ni al `Enter` ni a las flechas.
    for expected in [
        "chat::popover_next",
        "chat::popover_prev",
        "chat::popover_confirm",
        "chat::close_popover",
    ] {
        assert!(all.contains(expected), "falta {expected} en {all}");
    }
    assert_eq!(composer.len(), 5, "up, down, enter, tab y escape: {all}");
    // Y llegan últimas, que es lo que rompe el empate de profundidad contra
    // las de gpui-kit a favor del chat.
    let first_composer = rendered
        .iter()
        .position(|row| row.contains("Descendant"))
        .expect("no hay bindings del composer");
    assert_eq!(first_composer, rendered.len() - 5);
}

#[test]
fn the_status_labels_are_the_ones_of_the_spec() {
    assert_eq!(AgentStatus::Ready.label(), "listo");
    assert_eq!(AgentStatus::Thinking.label(), "pensando…");
    assert_eq!(AgentStatus::WaitingPermission.label(), "esperando permiso");
    assert_eq!(AgentStatus::Disconnected.label(), "desconectado");
    assert_eq!(AgentStatus::AuthRequired.label(), "autenticación requerida");
}

#[test]
fn the_autonomy_labels_say_what_happens() {
    let labels: Vec<&str> = AUTONOMY_VALUES.iter().map(|v| autonomy_label(*v)).collect();
    assert_eq!(
        labels,
        [
            "Aplicar y revisar después",
            "Preguntar antes de cada cambio",
            "Aplicar sin revisar"
        ]
    );
    // Every value explains itself in one sentence (`01-producto.md` §F5).
    for value in AUTONOMY_VALUES {
        let tooltip = autonomy_tooltip(value);
        assert!(tooltip.ends_with('.'), "{tooltip}");
        assert!(tooltip.len() > 40, "{tooltip}");
    }
}

#[test]
fn a_mention_shows_the_name_and_the_relative_directory() {
    let root = PathBuf::from("/proyecto");
    let path = PathBuf::from("/proyecto/crates/asteroid-chat/src/panel.rs");
    assert_eq!(file_name_label(&path), "panel.rs");
    assert_eq!(parent_label(Some(&root), &path), "crates/asteroid-chat/src");
    assert_eq!(
        relative_label(Some(&root), &path),
        "crates/asteroid-chat/src/panel.rs"
    );
    assert_eq!(chip_label(&path), "@panel.rs");
    // A file at the root has no directory to show, and one outside the project
    // keeps its absolute path.
    assert_eq!(parent_label(Some(&root), &root.join("Cargo.toml")), "");
    assert_eq!(
        relative_label(Some(&root), &PathBuf::from("/etc/hosts")),
        "/etc/hosts"
    );
}

// ------------------------------------------------------------ conversaciones

/// A stored conversation with one user message and one agent answer.
fn stored_conversation(id: &str, agent_id: &str) -> Conversation {
    let mut conversation =
        Conversation::new(id, agent_id, "Claude", PathBuf::from("/proyecto"), 10);
    conversation.session_id = Some("s-vieja".to_string());
    conversation.title = "Extraer el parser".to_string();
    conversation.entries = vec![
        Entry::UserMessage(UserMessage {
            blocks: vec![MessageBlock::Text("Extraer el parser".to_string())],
        }),
        Entry::AgentText(AgentText {
            markdown: "primer mensaje replayadosegundo mensaje replayado".to_string(),
            streaming: false,
            view: None,
        }),
    ];
    conversation
}

#[gpui::test]
fn choosing_an_agent_asks_the_workspace_for_a_new_conversation(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_agents(
            vec![
                ChatAgent::installed("claude-acp", "Claude"),
                ChatAgent::installed("codex-acp", "Codex"),
            ],
            cx,
        );
        panel.set_active_agent("codex-acp", cx);
    });
    let log = emitted(&recorder);
    assert!(log.contains("AgentSelected"), "{log}");
    assert!(log.contains("codex-acp"), "{log}");
}

#[gpui::test]
fn the_plus_button_asks_for_a_new_conversation(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.new_session(window, cx);
    });
    assert!(emitted(&recorder).contains("NewConversation"));
}

#[gpui::test]
fn starting_a_new_conversation_empties_the_panel(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(text_chunk("algo"), cx);
    });
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.start_new_conversation(window, cx);
        assert!(panel.entries().is_empty());
        assert!(panel.session_id().is_none());
        assert!(panel.history_notice().is_none());
    });
}

#[gpui::test]
fn loading_a_conversation_restores_its_entries_and_agent(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_agents(
            vec![
                ChatAgent::installed("claude-acp", "Claude"),
                ChatAgent::installed("codex-acp", "Codex"),
            ],
            cx,
        );
        panel.set_active_agent("claude-acp", cx);
    });
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.load_conversation(stored_conversation("c1", "codex-acp"), window, cx);
        assert_eq!(panel.entries().len(), 2);
        assert_eq!(
            panel.active_agent().map(|agent| agent.id.as_str()),
            Some("codex-acp"),
            "la conversación trae su agente"
        );
        assert_eq!(panel.active_conversation(), Some("c1"));
        assert!(
            panel.session_id().is_none(),
            "reabrir la sesión ACP es tarea del workspace"
        );
        match &panel.entries()[1] {
            Entry::AgentText(text) => assert!(text.view.is_some(), "el markdown se reconstruye"),
            other => panic!("se esperaba AgentText, llegó {other:?}"),
        }
    });
}

#[gpui::test]
fn a_replayed_session_does_not_duplicate_what_is_on_screen(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.load_conversation(stored_conversation("c1", "claude-acp"), window, cx);
        panel.begin_replay(cx);
        assert!(panel.is_replaying());
        // Lo que `session/load` vuelve a mandar ya está en pantalla.
        panel.handle_event(text_chunk("primer mensaje replayado"), cx);
        panel.handle_event(text_chunk("segundo mensaje replayado"), cx);
        assert_eq!(panel.entries().len(), 2, "{:?}", panel.entries());
        // Lo que no está sí se agrega.
        panel.handle_event(text_chunk("esto es nuevo"), cx);
        assert_eq!(panel.entries().len(), 3);
        // `SessionCreated` cierra el replay.
        panel.handle_event(
            AgentEvent::SessionCreated {
                session_id: SessionId::new("s-nueva"),
                modes: None,
                config_options: Vec::new(),
                commands: Vec::new(),
            },
            cx,
        );
        assert!(!panel.is_replaying());
    });
}

#[gpui::test]
fn the_read_only_notice_goes_away_with_the_first_message(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_history_notice(Some(READ_ONLY_HISTORY_NOTICE.to_string()), cx);
        assert!(panel.history_notice().is_some());
    });
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("seguimos", window, cx);
        panel.send(window, cx);
        assert!(panel.history_notice().is_none());
    });
}

#[gpui::test]
fn the_history_rows_are_what_the_workspace_sent(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_conversations(
            vec![
                ConversationSummary {
                    id: "c1".into(),
                    agent_id: "claude-acp".into(),
                    agent_name: "Claude".into(),
                    title: "Extraer el parser".into(),
                    when: "hace 5 min".into(),
                    session_id: Some("s1".into()),
                },
                ConversationSummary {
                    id: "c2".into(),
                    agent_id: "codex-acp".into(),
                    agent_name: "Codex".into(),
                    title: "Revisar el diff".into(),
                    when: "ayer".into(),
                    session_id: None,
                },
            ],
            cx,
        );
        assert_eq!(panel.conversations().len(), 2);
        panel.open_conversation("c2", cx);
        panel.delete_conversation("c1", cx);
        assert_eq!(panel.conversations().len(), 1, "el ✕ saca la fila al toque");
    });
    let log = emitted(&recorder);
    assert!(log.contains("OpenConversation"), "{log}");
    assert!(log.contains("DeleteConversation"), "{log}");
}

// --------------------------------------------------------- tarjeta de orden

#[test]
fn the_output_of_a_command_loses_its_fences() {
    assert_eq!(
        strip_code_fences("```console\nhola\nchau\n```"),
        "hola\nchau"
    );
    assert_eq!(strip_code_fences("```\nhola\n```"), "hola");
    assert_eq!(strip_code_fences("hola\nchau"), "hola\nchau");
    // Una valla suelta en el medio no es un envoltorio.
    assert_eq!(strip_code_fences("hola\n```\nchau"), "hola\n```\nchau");
}

#[gpui::test]
fn a_command_card_strips_the_fences_and_shows_the_exit_code(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            update(SessionUpdate::ToolCall(
                ToolCall::new("t1", "ls -la").kind(ToolKind::Execute),
            )),
            cx,
        );
        panel.handle_event(
            update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "t1",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Failed)
                    .content(vec![ToolCallContent::Content(Content::new(
                        ContentBlock::Text(TextContent::new(
                            "```console\nls: no existe\nexit code: 2\n```",
                        )),
                    ))]),
            ))),
            cx,
        );
        match &panel.entries()[0] {
            Entry::ToolCall(call) => match &call.content[0] {
                ToolContent::Command { command, output } => {
                    assert_eq!(command, "ls -la");
                    assert_eq!(output, "ls: no existe\nexit code: 2");
                    assert_eq!(exit_code_in(output), Some(2));
                }
                other => panic!("se esperaba Command, llegó {other:?}"),
            },
            other => panic!("se esperaba ToolCall, llegó {other:?}"),
        }
    });
}

#[test]
fn the_exit_code_is_read_however_the_agent_words_it() {
    assert_eq!(exit_code_in("exit status 127"), Some(127));
    assert_eq!(exit_code_in("Command exited with code 1"), Some(1));
    assert_eq!(exit_code_in("código de salida: 3"), Some(3));
    assert_eq!(exit_code_in("todo bien"), None);
}

#[test]
fn a_title_is_the_first_user_message_cut_to_sixty() {
    let entries = vec![Entry::UserMessage(UserMessage {
        blocks: vec![MessageBlock::Text("a".repeat(100))],
    })];
    let title = conversation_title(&entries);
    assert_eq!(title.chars().count(), TITLE_MAX_CHARS);
    assert!(title.ends_with('…'));
    assert_eq!(conversation_title(&[]), UNTITLED_CONVERSATION);
}
