//! Interaction tests over the GPUI test harness: streaming, tool cards,
//! permissions, the `@` and `/` popovers, the footer selectors and the
//! transcript round trip.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{
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
    // `gpui-kit`'s theme has to exist before the composer is built (its fonts
    // come from it); `bind_default_keys` installs the editor's bindings, which
    // the composer is made of, and then the chat's.
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
        panel.set_connections(
            vec![connection("c-claude", "claude-acp", "Claude · personal")],
            cx,
        );
        panel.set_active_connection(Some("c-claude".into()), cx);
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
                ToolCall::new("t1", "cargo test -p cincel-chat").kind(ToolKind::Execute),
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
    // El `Enter` del composer (el `editor::insert_newline` que toma la caja en
    // la fase de captura) responde el permiso con la primera opción.
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
    // The popover sits right above the composer: 3 rows of 24 px after its
    // 4 px of padding and 1 px border.
    let popover = visual
        .debug_bounds("chat-popover")
        .expect("the popover was painted");
    let composer_top = visual
        .debug_bounds("chat-composer-box")
        .expect("the composer was painted")
        .top();
    // `bottom: 100%` measures from the padding box, inside the 1 px top border.
    assert!(
        (f32::from(popover.bottom()) - f32::from(composer_top)).abs() <= 1.,
        "anchored to the composer: {popover:?} vs {composer_top:?}"
    );
    let second_row = f32::from(popover.top()) + 5. + POPOVER_ROW_HEIGHT * 1.5;
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
fn a_stored_autonomy_is_ignored_when_loading(cx: &mut TestAppContext) {
    // Files written before Etapa 3 carry `"autonomy"`; the key is retired but
    // the transcript must still load.
    let json = r#"{ "version": 1, "agent_id": "claude-acp", "session_id": null,
                    "autonomy": "always_apply", "entries": [] }"#;
    let dump: TranscriptDump = serde_json::from_str(json).expect("un dump viejo carga");
    let (panel, mut visual, recorder) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| panel.import_transcript(dump, cx));
    let exported = panel.update(&mut visual, |panel, _cx| panel.export_transcript());
    let json = serde_json::to_string(&exported).expect("serializa");
    assert!(!json.contains("autonomy"), "{json}");
    assert!(!emitted(&recorder).contains("Autonomy"));
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
                cincel_acp::acp::schema::v1::PlanEntry::new(
                    "Leer el código",
                    PlanEntryPriority::High,
                    PlanEntryStatus::Completed,
                ),
            ]))),
            cx,
        );
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
    use cincel_acp::acp::schema::v1::{AuthMethod, AuthMethodTerminal};
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
    use cincel_acp::acp::schema::v1::PlanEntry as WirePlanEntry;
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
fn the_placeholder_names_the_active_connection(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        assert_eq!(panel.placeholder(), "Conectá un agente para empezar…");
        panel.set_connections(
            vec![connection("c-claude", "claude-acp", "Claude · personal")],
            cx,
        );
        panel.set_active_connection(Some("c-claude".into()), cx);
        assert_eq!(
            panel.placeholder(),
            "Escribí un mensaje para Claude · personal…"
        );
    });
}

#[gpui::test]
fn the_active_connection_survives_a_reload_of_the_list(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(
            vec![
                connection("c-claude", "claude-acp", "Claude · personal"),
                connection("c-antigravity", "antigravity-acp", "Antigravity · personal"),
            ],
            cx,
        );
        panel.set_active_connection(Some("c-antigravity".into()), cx);
        panel.set_connections(
            vec![
                connection("c-antigravity", "antigravity-acp", "Antigravity · trabajo"),
                connection("c-claude", "claude-acp", "Claude · personal"),
            ],
            cx,
        );
        assert_eq!(
            panel
                .active_connection()
                .map(|connection| connection.label.as_str()),
            Some("Antigravity · trabajo"),
            "la conexión activa sigue siendo la misma, con su nombre nuevo"
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
fn the_composer_binds_only_what_the_editor_does_not() {
    let bindings = default_key_bindings();
    let rendered: Vec<String> = bindings
        .iter()
        .map(|binding| format!("{binding:?}"))
        .collect();
    // `Chat > Composer` se guarda como un `Descendant`.
    let composer: Vec<&String> = rendered
        .iter()
        .filter(|row| row.contains("Descendant"))
        .collect();
    assert_eq!(composer.len(), 1, "{composer:?}");
    let row = composer[0];
    assert!(row.contains("Chat") && row.contains("Composer"), "{row}");
    assert!(
        row.contains("chat::newline") && row.contains("shift: true"),
        "{row}"
    );
    assert_eq!(crate::actions::CONTEXT_COMPOSER, "Chat > Composer");
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
fn a_mention_shows_the_name_and_the_relative_directory() {
    let root = PathBuf::from("/proyecto");
    let path = PathBuf::from("/proyecto/crates/cincel-chat/src/panel.rs");
    assert_eq!(file_name_label(&path), "panel.rs");
    assert_eq!(parent_label(Some(&root), &path), "crates/cincel-chat/src");
    assert_eq!(
        relative_label(Some(&root), &path),
        "crates/cincel-chat/src/panel.rs"
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
fn choosing_a_connection_asks_the_workspace_for_a_new_conversation(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(
            vec![
                connection("c-claude", "claude-acp", "Claude · personal"),
                connection("c-codex", "codex-acp", "Codex · trabajo"),
            ],
            cx,
        );
        panel.open_connections(cx);
        panel.select_connection("c-codex", cx);
        assert_eq!(
            panel.popover(),
            &Popover::Closed,
            "elegir cierra el popover"
        );
        assert!(
            panel.active_connection().is_none(),
            "el panel no conecta nada solo: espera al workspace"
        );
    });
    let log = emitted(&recorder);
    assert!(log.contains("ConnectionSelected"), "{log}");
    assert!(log.contains("c-codex"), "{log}");
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
fn loading_a_conversation_restores_its_entries(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(
            vec![connection("c-claude", "claude-acp", "Claude · personal")],
            cx,
        );
        panel.set_active_connection(Some("c-claude".into()), cx);
    });
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.load_conversation(stored_conversation("c1", "codex-acp"), window, cx);
        assert_eq!(panel.entries().len(), 2);
        assert_eq!(
            panel
                .active_connection()
                .map(|connection| connection.id.as_str()),
            Some("c-claude"),
            "la conexión la decide el workspace, no la conversación"
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
                    connection_id: Some("c-claude".into()),
                    group: "Claude · personal".into(),
                },
                ConversationSummary {
                    id: "c2".into(),
                    agent_id: "codex-acp".into(),
                    agent_name: "Codex".into(),
                    title: "Revisar el diff".into(),
                    when: "ayer".into(),
                    session_id: None,
                    connection_id: None,
                    group: LEGACY_CONNECTION_GROUP.into(),
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

// ---------------------------------------------------------------- bubbles

/// Sends `text` as a user message in a 400 px wide panel and returns the
/// bounds of its bubble and of its text.
fn bubble_bounds(
    cx: &mut TestAppContext,
    text: &str,
) -> (gpui::Bounds<gpui::Pixels>, gpui::Bounds<gpui::Pixels>) {
    let (panel, mut visual, _) = open_with_session(cx);
    visual.simulate_resize(size(px(400.), px(720.)));
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text(text, window, cx);
        panel.send(window, cx);
    });
    visual.run_until_parked();
    let bubble = visual
        .debug_bounds("user-bubble-0")
        .expect("la burbuja se pintó");
    let body = visual
        .debug_bounds("user-text-0")
        .expect("el texto se pintó");
    (bubble, body)
}

fn assert_wraps_inside_the_bubble(
    bubble: gpui::Bounds<gpui::Pixels>,
    body: gpui::Bounds<gpui::Pixels>,
) {
    let panel_width = px(400.);
    // Right-aligned, at most 85 % of the panel, never past its right edge.
    assert!(bubble.right() <= panel_width, "{bubble:?}");
    assert!(
        bubble.size.width <= panel_width * crate::settings::BUBBLE_MAX_WIDTH + px(1.),
        "{bubble:?}"
    );
    assert!(bubble.left() > px(0.), "{bubble:?}");
    // The text stays inside the bubble…
    assert!(body.left() >= bubble.left(), "{body:?} fuera de {bubble:?}");
    assert!(
        body.right() <= bubble.right(),
        "{body:?} fuera de {bubble:?}"
    );
    // …because it wrapped onto several lines instead of overflowing.
    assert!(
        body.size.height > px(crate::settings::TEXT_BODY * 3.),
        "no se partió en líneas: {body:?}"
    );
}

#[gpui::test]
fn a_long_message_with_spaces_wraps_inside_its_bubble(cx: &mut TestAppContext) {
    let text = "palabra ".repeat(50);
    assert!(text.trim_end().len() >= 399);
    let (bubble, body) = bubble_bounds(cx, &text);
    assert_wraps_inside_the_bubble(bubble, body);
}

#[gpui::test]
fn a_long_token_without_spaces_breaks_inside_its_bubble(cx: &mut TestAppContext) {
    let text = "x".repeat(400);
    let (bubble, body) = bubble_bounds(cx, &text);
    assert_wraps_inside_the_bubble(bubble, body);
}

#[gpui::test]
fn a_paragraph_with_inline_code_keeps_the_body_size(cx: &mut TestAppContext) {
    // `gpui-kit` lays out a paragraph with inline code from the text style it
    // finds at layout time; inside the transcript list that is the chat's
    // 13 px, so both one-line answers are exactly as tall.
    let (panel, mut visual, _) = open_with_session(cx);
    visual.simulate_resize(size(px(400.), px(720.)));
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(text_chunk("Sin código en esta línea."), cx);
        panel.handle_event(
            AgentEvent::TurnEnded {
                session_id: SessionId::new("s1"),
                stop_reason: StopReason::EndTurn,
            },
            cx,
        );
        panel.handle_event(text_chunk("Con `código` en esta línea."), cx);
    });
    visual.run_until_parked();
    let entries = panel.update(&mut visual, |panel, _cx| panel.entries().len());
    assert_eq!(entries, 3, "texto, separador, texto");
    let plain = visual.debug_bounds("agent-text-0").expect("primer texto");
    let code = visual.debug_bounds("agent-text-2").expect("segundo texto");
    // A paragraph laid out at the window's 16 px instead of the chat's 13 px is
    // ~4.5 px taller (line height 1.5). Fallback fonts (no JetBrains Mono / Inter
    // installed) shift the inline-code line by a fraction of a pixel, so we
    // tolerate small differences and only reject the wrong text size.
    let diff = f32::from(plain.size.height) - f32::from(code.size.height);
    assert!(
        diff.abs() < 3.,
        "la línea con código inline mide distinto: {plain:?} vs {code:?}"
    );
}

// ------------------------------------------------------- markdown composer

#[gpui::test]
fn the_composer_is_a_markdown_editor_without_gutter(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    visual.run_until_parked();
    let (language, settings) = panel.update(&mut visual, |panel, cx| {
        let editor = panel.composer().read(cx);
        (
            editor
                .language()
                .map(|language| language.name().to_string()),
            editor.settings().clone(),
        )
    });
    assert_eq!(language.as_deref(), Some("markdown"));
    assert!(settings.soft_wrap);
    assert_eq!(settings.chrome, cincel_editor::EditorChrome::Minimal);
    assert_eq!(
        settings.auto_height,
        Some(cincel_editor::AutoHeight {
            min_rows: 1,
            max_rows: 8
        })
    );
    assert!(settings.prose_font_family.is_some());
    assert_eq!(settings.font_size, crate::settings::TEXT_BODY);
}

#[gpui::test]
fn shift_enter_breaks_the_line_and_enter_sends_it(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open_with_session(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.focus_input(window, cx);
    });
    visual.run_until_parked();
    visual.simulate_input("- uno");
    visual.simulate_keystrokes("shift-enter");
    visual.simulate_input("- **dos**");
    panel.update(&mut visual, |panel, cx| {
        assert_eq!(panel.input_text(cx), "- uno\n- **dos**");
        assert!(panel.entries().is_empty(), "Shift+Enter no envía");
    });
    visual.simulate_keystrokes("enter");
    panel.update(&mut visual, |panel, cx| {
        assert_eq!(panel.input_text(cx), "");
        match panel.entries() {
            [Entry::UserMessage(message)] => assert_eq!(
                message.blocks,
                vec![MessageBlock::Text("- uno\n- **dos**".into())]
            ),
            other => panic!("{other:?}"),
        }
    });
    assert!(emitted(&recorder).contains("Prompt"));
}

#[gpui::test]
fn the_composer_paints_the_markdown_structure(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("# Plan\n- **sí**\n```python\nx = 1\n```", window, cx);
    });
    visual.run_until_parked();
    let decorations = panel.update(&mut visual, |panel, cx| {
        panel
            .composer()
            .update(cx, |editor, _| editor.decorations().cloned())
            .expect("the composer has a decorator")
    });
    let theme = ChatTheme::default();
    assert_eq!(
        decorations.row_backgrounds,
        vec![(2..5, gpui::Rgba::from(theme.bg_editor))]
    );
    assert_eq!(decorations.monospace_rows, vec![2..5]);
    use cincel_syntax::HighlightId;
    let ids: Vec<HighlightId> = decorations.highlights.iter().map(|(_, id)| *id).collect();
    for id in [
        HighlightId::Keyword,
        HighlightId::Comment,
        HighlightId::Number,
    ] {
        assert!(ids.contains(&id), "{id:?} en {:?}", decorations.highlights);
    }
}

#[gpui::test]
fn the_composer_grows_to_eight_rows_and_then_scrolls(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    visual.simulate_resize(size(px(400.), px(720.)));
    let composer_height = |visual: &mut VisualTestContext| {
        visual
            .debug_bounds("chat-input-box")
            .expect("the input box was painted")
            .size
            .height
    };
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("una", window, cx);
    });
    visual.run_until_parked();
    let one = composer_height(&mut visual);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("1\n2\n3", window, cx);
    });
    visual.run_until_parked();
    let three = composer_height(&mut visual);
    with_window(&panel, &mut visual, |panel, window, cx| {
        let long: Vec<String> = (0..30).map(|n| n.to_string()).collect();
        panel.set_input_text(&long.join("\n"), window, cx);
    });
    visual.run_until_parked();
    let many = composer_height(&mut visual);
    // Rows of 13 × 1.5 px, 6 px of padding above and below, a 1 px border,
    // and never below the box's 40 px minimum.
    let row = crate::settings::TEXT_BODY * 1.5;
    let expected = |rows: f32| (rows * row + 14.).max(crate::settings::INPUT_MIN_HEIGHT);
    assert!((f32::from(one) - expected(1.)).abs() < 1., "{one:?}");
    assert!((f32::from(three) - expected(3.)).abs() < 1., "{three:?}");
    assert!((f32::from(many) - expected(8.)).abs() < 1., "{many:?}");
}

#[gpui::test]
fn a_pending_permission_makes_the_composer_read_only(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("borrador", window, cx);
        panel.request_permission(
            7,
            &ToolCallUpdate::new("t1", ToolCallUpdateFields::new().title("Ejecutar")),
            permission_options(),
            cx,
        );
        panel.focus_input(window, cx);
    });
    visual.run_until_parked();
    visual.simulate_input("x");
    panel.update(&mut visual, |panel, cx| {
        assert_eq!(panel.input_text(cx), "borrador");
        assert!(panel.composer().read(cx).is_read_only());
    });
}

#[gpui::test]
fn the_zoom_reaches_the_composer_font(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    let size = panel.update(&mut visual, |panel, cx| {
        panel.set_settings(
            ChatSettings {
                scale: 1.5,
                ..ChatSettings::default()
            },
            cx,
        );
        panel.composer().read(cx).settings().font_size
    });
    assert_eq!(size, crate::settings::TEXT_BODY * 1.5);
}

// ------------------------------------------------------- markdown bubbles

#[test]
fn a_bubble_is_markdown_except_the_paragraphs_with_mentions() {
    use crate::render::{FlowPart, UserPiece, user_pieces};
    let blocks = vec![
        MessageBlock::Text("mirá ".into()),
        MessageBlock::File(PathBuf::from("/p/main.rs")),
        MessageBlock::Text(
            ", ¿qué tal?\n\n- uno\n- **dos**\n\n```python\nx = 1\n\ny = 2\n```".into(),
        ),
    ];
    let pieces = user_pieces(&blocks);
    assert_eq!(
        pieces,
        vec![
            UserPiece::Flow(vec![
                FlowPart::Text("mirá ".into()),
                FlowPart::File(PathBuf::from("/p/main.rs")),
                FlowPart::Text(", ¿qué tal?".into()),
            ]),
            UserPiece::Markdown("- uno\n- **dos**\n\n```python\nx = 1\n\ny = 2\n```".into()),
        ]
    );
    // Without mentions the whole message is one Markdown piece.
    assert_eq!(
        user_pieces(&[MessageBlock::Text("**hola**".into())]),
        vec![UserPiece::Markdown("**hola**".into())]
    );
}

#[gpui::test]
fn a_sent_message_is_rendered_as_markdown(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    visual.simulate_resize(size(px(400.), px(720.)));
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text(
            "Revisá:\n\n- uno\n- **dos**\n\n```python\ndef suma(a, b):\n    return a + b\n```",
            window,
            cx,
        );
        panel.send(window, cx);
    });
    visual.run_until_parked();
    let views = panel.update(&mut visual, |panel, _cx| panel.bubble_views.borrow().len());
    assert_eq!(views, 1, "one Markdown state for the whole bubble");
    let bubble = visual.debug_bounds("user-bubble-0").expect("bubble");
    let body = visual.debug_bounds("user-text-0").expect("text");
    assert!(body.left() >= bubble.left() && body.right() <= bubble.right());
    // A list, a paragraph and a fenced block are several lines tall.
    assert!(
        body.size.height > px(crate::settings::TEXT_BODY * 5.),
        "{body:?}"
    );
}

// ------------------------------------------------------------- conexiones

/// Resizes the window, which is what makes the test harness draw a frame
/// (and therefore fill the debug bounds).
fn paint(visual: &mut VisualTestContext) {
    visual.simulate_resize(size(px(400.), px(720.)));
    visual.run_until_parked();
}

/// A connected row of the "Conectar" popover.
fn connection(id: &str, agent_id: &str, label: &str) -> ChatConnection {
    ChatConnection {
        id: id.into(),
        agent_id: agent_id.into(),
        label: label.into(),
        identity: Some("gary@example.com · max".into()),
        last_used: "Usado hace 2 h".into(),
        badge: ConnectionBadge::Connected,
    }
}

#[gpui::test]
fn without_a_connection_the_header_and_the_empty_state_offer_conectar(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    paint(&mut visual);
    panel.read_with(&visual, |panel, _| {
        assert!(panel.active_connection().is_none());
        assert_eq!(panel.status(), AgentStatus::Disconnected);
    });
    // The empty state and the header both render the button: the popover is
    // what they open.
    assert!(
        visual.debug_bounds("chat-connection").is_none(),
        "sin conexión no hay etiqueta en el encabezado"
    );
    panel.update(&mut visual, |panel, cx| panel.open_connections(cx));
    assert_eq!(
        panel.read_with(&visual, |panel, _| panel.popover().clone()),
        Popover::Connections
    );
    assert_eq!(NO_CONNECTION_TEXT, "No hay ningún agente conectado");
}

#[gpui::test]
fn the_active_connection_shows_its_label_in_the_header(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(
            vec![connection("c-claude", "claude-acp", "Claude · personal")],
            cx,
        );
        panel.set_active_connection(Some("c-claude".into()), cx);
    });
    paint(&mut visual);
    assert!(
        visual.debug_bounds("chat-connection").is_some(),
        "con una conexión activa, el encabezado muestra su etiqueta"
    );
}

#[gpui::test]
fn the_popover_lists_every_connection_with_its_badge(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        let mut expired = connection("c-antigravity", "antigravity-acp", "Antigravity · personal");
        expired.badge = ConnectionBadge::SessionExpired;
        let mut broken = connection("c-codex", "codex-acp", "Codex · trabajo");
        broken.badge = ConnectionBadge::Unavailable {
            reason: "Falta el adaptador de Codex.".into(),
        };
        panel.set_connections(
            vec![
                connection("c-claude", "claude-acp", "Claude · personal"),
                expired,
                broken,
            ],
            cx,
        );
        panel.open_connections(cx);
    });
    paint(&mut visual);
    for row in ["connection-row-0", "connection-row-1", "connection-row-2"] {
        assert!(visual.debug_bounds(row).is_some(), "{row} se pinta");
    }
    let labels: Vec<&str> = [
        ConnectionBadge::Connected,
        ConnectionBadge::SessionExpired,
        ConnectionBadge::Unavailable {
            reason: String::new(),
        },
    ]
    .iter()
    .map(ConnectionBadge::label)
    .collect();
    assert_eq!(labels, ["Conectada", "Sesión vencida", "No disponible"]);
}

#[gpui::test]
fn the_popover_preselects_the_default_label_without_connecting(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(
            vec![
                connection("c-claude", "claude-acp", "Claude · personal"),
                connection("c-codex", "codex-acp", "Codex · trabajo"),
            ],
            cx,
        );
        panel.set_preselected_connection(Some("c-codex".into()), cx);
        assert_eq!(panel.preselected_connection(), Some("c-codex"));
        assert!(panel.active_connection().is_none());
    });
    assert!(
        !emitted(&recorder).contains("ConnectionSelected"),
        "preseleccionar nunca conecta"
    );
}

#[gpui::test]
fn the_footer_and_the_context_menu_ask_the_workspace(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(
            vec![connection("c-claude", "claude-acp", "Claude · personal")],
            cx,
        );
        panel.open_connections(cx);
        panel.request_new_connection(cx);
        panel.open_connections(cx);
        panel.request_delete_connection(cx);
        panel.open_connection_menu("c-claude", cx);
        assert_eq!(panel.popover(), &Popover::ConnectionMenu("c-claude".into()));
        panel.request_rename_connection("c-claude", cx);
        assert_eq!(panel.popover(), &Popover::Closed);
    });
    let log = emitted(&recorder);
    assert!(log.contains("NewConnection"), "{log}");
    assert!(log.contains("DeleteConnections"), "{log}");
    assert!(log.contains("RenameConnection"), "{log}");
}

#[gpui::test]
fn the_expired_banner_offers_volver_a_conectar(cx: &mut TestAppContext) {
    let (panel, mut visual, recorder) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_banner(
            Some(ConnectionBanner::Expired {
                id: "c-claude".into(),
                label: "Claude · personal".into(),
            }),
            cx,
        );
    });
    paint(&mut visual);
    assert!(visual.debug_bounds("chat-connection-banner").is_some());
    let text = panel.read_with(&visual, |panel, _| {
        panel.banner().map(ConnectionBanner::text)
    });
    assert_eq!(
        text.as_deref(),
        Some("La sesión de «Claude · personal» venció")
    );
    panel.update(&mut visual, |panel, cx| {
        panel.request_reconnect("c-claude", cx);
        panel.request_repair("c-claude", cx);
    });
    let log = emitted(&recorder);
    assert!(log.contains("Reconnect"), "{log}");
    assert!(log.contains("Repair"), "{log}");
}

#[gpui::test]
fn a_failed_authentication_is_shown_not_swallowed(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(
            AgentEvent::AuthFailed {
                method_id: "claude-ai-login".into(),
                message: "token revocado".into(),
            },
            cx,
        );
        let last = panel.entries().last().cloned();
        match last {
            Some(Entry::Notice(notice)) => {
                assert_eq!(notice.level, NoticeLevel::Error);
                assert!(notice.text.contains("token revocado"), "{}", notice.text);
            }
            other => panic!("se esperaba un aviso, llegó {other:?}"),
        }
        assert_eq!(panel.status(), AgentStatus::AuthRequired);
    });
}

#[gpui::test]
fn logout_and_elicitation_completion_leave_a_trace(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open_with_session(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.handle_event(AgentEvent::LoggedOut { ok: true }, cx);
        panel.handle_event(
            AgentEvent::ElicitationCompleted {
                id: cincel_acp::PermissionRequestId(7),
                elicitation_id: "elic-1".into(),
            },
            cx,
        );
        let texts: Vec<String> = panel
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                Entry::Notice(notice) => Some(notice.text.clone()),
                _ => None,
            })
            .collect();
        assert!(
            texts.iter().any(|text| text.contains("Se cerró la sesión")),
            "{texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.contains("navegador")),
            "{texts:?}"
        );
    });
}

#[gpui::test]
fn the_history_groups_by_connection_label(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        let old = Conversation::new("c-old", "claude-acp", "Claude", PathBuf::from("/p"), 1);
        let new = Conversation::new(
            "c-new",
            "claude-acp",
            "Claude · personal",
            PathBuf::from("/p"),
            2,
        )
        .with_connection("c-claude");
        let rows = vec![new.summary("ayer"), old.summary("hace 3 d")];
        assert_eq!(rows[0].group, "Claude · personal");
        assert_eq!(rows[1].group, LEGACY_CONNECTION_GROUP);
        assert_eq!(rows[1].connection_id, None);
        panel.set_conversations(rows, cx);
        panel.toggle_popover(Popover::Conversations, cx);
    });
    paint(&mut visual);
    assert!(visual.debug_bounds("chat-popover").is_some());
}

#[test]
fn an_old_conversation_without_connection_still_parses() {
    let raw = r#"{"version":1,"id":"c1","agent_id":"claude-acp","agent_name":"Claude",
        "session_id":null,"cwd":"/p","created_at":1,"updated_at":2,"title":"t","entries":[]}"#;
    let conversation: Conversation = serde_json::from_str(raw).expect("se lee");
    assert_eq!(conversation.connection_id, None);
    assert_eq!(conversation.summary("ayer").group, LEGACY_CONNECTION_GROUP);
}

#[test]
fn provider_names_and_monograms() {
    assert_eq!(provider_name("claude-acp"), "Claude");
    assert_eq!(provider_name("codex-acp"), "Codex");
    assert_eq!(provider_name("antigravity-acp"), "Antigravity");
    assert_eq!(
        provider_name("gemini"),
        "Agente",
        "Gemini ya no es un proveedor"
    );
    assert_eq!(provider_monogram("codex-acp"), "X");
    assert_eq!(provider_monogram("antigravity-acp"), "A");
    assert_eq!(provider_monogram("otro"), "A");
}
