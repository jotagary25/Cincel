//! The agent's activity row at the foot of the transcript
//! (`docs/specs/10-etapa7-ronda2.md` §8.4), driven with simulated
//! [`AgentEvent`]s like `tests.rs`: "Pensando…" before anything arrives,
//! "Trabajando…" with a tool running, "Escribiendo…" while the answer
//! streams, "Esperando permiso…" with a request pending, no doubled row under
//! a live thought, nothing once the turn ends or is cancelled, and nothing of
//! it in the stored conversation.

use std::path::PathBuf;

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{
    ContentBlock, ContentChunk, PermissionOption, PermissionOptionKind, SessionId, SessionUpdate,
    StopReason, TextContent, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
    ToolKind,
};
use gpui::{AnyWindowHandle, Context, Entity, TestAppContext, VisualTestContext, Window, px, size};

use crate::actions::bind_default_keys;
use crate::model::*;
use crate::panel::ChatPanel;
use crate::settings::ChatSettings;
use crate::theme::ChatTheme;

const ROW: &str = "chat-activity-row";

/// A 400 × 720 window with a connected panel that has a session.
fn open(cx: &mut TestAppContext) -> (Entity<ChatPanel>, VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_default_keys(cx);
    });
    let window = cx.add_window(|window, cx| {
        ChatPanel::new(ChatTheme::default(), ChatSettings::default(), window, cx)
    });
    let panel = window.update(cx, |_, _, cx| cx.entity()).unwrap();
    let handle: AnyWindowHandle = window.into();
    let mut visual = VisualTestContext::from_window(handle, cx);
    visual.simulate_resize(size(px(400.), px(720.)));
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(
            vec![ChatConnection {
                id: "c-claude".into(),
                agent_id: "claude-acp".into(),
                label: "Claude".into(),
                agent_name: "Claude".into(),
                identity: None,
                last_used: String::new(),
                badge: ConnectionBadge::Connected,
            }],
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
    visual.run_until_parked();
    (panel, visual)
}

fn with_window<R>(
    panel: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
    f: impl FnOnce(&mut ChatPanel, &mut Window, &mut Context<ChatPanel>) -> R,
) -> R {
    visual.update(|window, cx| panel.update(cx, |panel, cx| f(panel, window, cx)))
}

fn send(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext, text: &str) {
    with_window(panel, visual, |panel, window, cx| {
        panel.set_input_text(text, window, cx);
        panel.send(window, cx);
    });
    visual.run_until_parked();
}

fn event(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext, event: AgentEvent) {
    panel.update(visual, |panel, cx| panel.handle_event(event, cx));
    visual.run_until_parked();
}

fn session_update(update: SessionUpdate) -> AgentEvent {
    AgentEvent::Update {
        session_id: SessionId::new("s1"),
        update: Box::new(update),
    }
}

fn text_chunk(text: &str) -> AgentEvent {
    session_update(SessionUpdate::AgentMessageChunk(ContentChunk::new(
        ContentBlock::Text(TextContent::new(text)),
    )))
}

fn thought_chunk(text: &str) -> AgentEvent {
    session_update(SessionUpdate::AgentThoughtChunk(ContentChunk::new(
        ContentBlock::Text(TextContent::new(text)),
    )))
}

fn tool_call(id: &str) -> AgentEvent {
    session_update(SessionUpdate::ToolCall(
        ToolCall::new(id.to_string(), "Leer main.rs").kind(ToolKind::Read),
    ))
}

fn tool_status(id: &str, status: ToolCallStatus) -> AgentEvent {
    session_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        id.to_string(),
        ToolCallUpdateFields::new().status(status),
    )))
}

fn turn_ended(reason: StopReason) -> AgentEvent {
    AgentEvent::TurnEnded {
        session_id: SessionId::new("s1"),
        stop_reason: reason,
    }
}

/// What the panel says and whether the row is painted, after a fresh frame.
fn shown(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) -> Option<&'static str> {
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    let activity = panel.read_with(visual, |panel, _| panel.activity());
    let painted = visual.debug_bounds(ROW).is_some();
    assert_eq!(
        painted,
        activity.is_some(),
        "la fila se pinta exactamente cuando hay actividad ({activity:?})"
    );
    activity.map(AgentActivity::label)
}

/// The row sits under the last item of the list, inside the transcript.
fn assert_row_at_the_foot(visual: &mut VisualTestContext) {
    let row = visual.debug_bounds(ROW).expect("fila de actividad");
    let transcript = visual
        .debug_bounds("chat-transcript")
        .expect("conversación");
    assert!(
        row.top() >= transcript.top() && row.bottom() <= transcript.bottom(),
        "{row:?} dentro de {transcript:?}"
    );
    assert!(
        (f32::from(row.size.height) - 28.).abs() < 0.5,
        "alto TOOL_ROW_HEIGHT: {row:?}"
    );
}

/// §8.4: right after sending, before any chunk, the row says "Pensando…".
#[gpui::test]
fn after_sending_and_before_any_chunk_the_row_says_pensando(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    assert_eq!(shown(&panel, &mut visual), None, "sin turno, sin fila");
    send(&panel, &mut visual, "hola");
    assert_eq!(shown(&panel, &mut visual), Some("Pensando…"));
    assert_row_at_the_foot(&mut visual);
}

/// §8.4: the whole sequence think → tool → think → write → end, plus the
/// permission request.
#[gpui::test]
fn the_row_follows_the_last_signal_of_the_turn(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    send(&panel, &mut visual, "arreglá el test");
    assert_eq!(shown(&panel, &mut visual), Some("Pensando…"));

    // Think: the live "Pensando… (N s)" row is the last item and plays the
    // part; no second row under it.
    event(&panel, &mut visual, thought_chunk("Miro el archivo"));
    assert_eq!(shown(&panel, &mut visual), None, "no se duplica");
    panel.read_with(&visual, |panel, _| {
        assert!(matches!(
            panel.entries().last(),
            Some(Entry::AgentThought(_))
        ));
    });

    // Tool.
    event(&panel, &mut visual, tool_call("t1"));
    assert_eq!(shown(&panel, &mut visual), Some("Trabajando…"));
    event(
        &panel,
        &mut visual,
        tool_status("t1", ToolCallStatus::InProgress),
    );
    assert_eq!(shown(&panel, &mut visual), Some("Trabajando…"));
    assert_row_at_the_foot(&mut visual);
    event(
        &panel,
        &mut visual,
        tool_status("t1", ToolCallStatus::Completed),
    );
    assert_eq!(
        shown(&panel, &mut visual),
        Some("Pensando…"),
        "la herramienta terminó: el agente decide qué sigue"
    );

    // Think again: a new live thought is the last item.
    event(&panel, &mut visual, thought_chunk("Ya sé qué cambiar"));
    assert_eq!(shown(&panel, &mut visual), None, "no se duplica");

    // A second tool, then a permission request for it.
    event(&panel, &mut visual, tool_call("t2"));
    assert_eq!(shown(&panel, &mut visual), Some("Trabajando…"));
    panel.update(&mut visual, |panel, cx| {
        panel.request_permission(
            7,
            &ToolCallUpdate::new("t2", ToolCallUpdateFields::new().title("Editar main.rs")),
            vec![
                PermissionOption::new("allow", "Permitir", PermissionOptionKind::AllowOnce),
                PermissionOption::new("reject", "Rechazar", PermissionOptionKind::RejectOnce),
            ],
            cx,
        );
    });
    visual.run_until_parked();
    assert_eq!(shown(&panel, &mut visual), Some("Esperando permiso…"));
    assert_row_at_the_foot(&mut visual);
    panel.update(&mut visual, |panel, cx| panel.accept_permission(cx));
    visual.run_until_parked();
    assert_eq!(
        shown(&panel, &mut visual),
        Some("Trabajando…"),
        "permiso dado: la herramienta sigue en curso"
    );
    event(
        &panel,
        &mut visual,
        tool_status("t2", ToolCallStatus::Completed),
    );

    // Write: from the first chunk of the answer.
    event(&panel, &mut visual, text_chunk("Listo, "));
    assert_eq!(shown(&panel, &mut visual), Some("Escribiendo…"));
    event(&panel, &mut visual, text_chunk("cambié la línea 3."));
    assert_eq!(shown(&panel, &mut visual), Some("Escribiendo…"));
    assert_row_at_the_foot(&mut visual);

    // End.
    event(&panel, &mut visual, turn_ended(StopReason::EndTurn));
    assert_eq!(shown(&panel, &mut visual), None, "terminó el turno");

    // The next turn starts over at "Pensando…".
    send(&panel, &mut visual, "gracias");
    assert_eq!(shown(&panel, &mut visual), Some("Pensando…"));
}

/// §8.4: cancelling the turn takes the row away at once, before the
/// agent's `TurnEnded`.
#[gpui::test]
fn cancelling_the_turn_removes_the_row(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    send(&panel, &mut visual, "hola");
    event(&panel, &mut visual, text_chunk("Empiezo"));
    assert_eq!(shown(&panel, &mut visual), Some("Escribiendo…"));
    panel.update(&mut visual, |panel, cx| panel.cancel_turn(cx));
    visual.run_until_parked();
    assert_eq!(shown(&panel, &mut visual), None, "cancelado");
    event(&panel, &mut visual, turn_ended(StopReason::Cancelled));
    assert_eq!(shown(&panel, &mut visual), None);
}

/// §8.4: a turn closed without `TurnEnded` (`abort_turn`) or an agent that
/// exits leaves no row behind either.
#[gpui::test]
fn an_aborted_turn_or_an_exited_agent_leaves_no_row(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    send(&panel, &mut visual, "hola");
    event(&panel, &mut visual, tool_call("t1"));
    assert_eq!(shown(&panel, &mut visual), Some("Trabajando…"));
    panel.update(&mut visual, |panel, cx| panel.abort_turn(cx));
    visual.run_until_parked();
    assert_eq!(shown(&panel, &mut visual), None);

    send(&panel, &mut visual, "otra vez");
    assert_eq!(shown(&panel, &mut visual), Some("Pensando…"));
    event(
        &panel,
        &mut visual,
        AgentEvent::Exited {
            code: Some(1),
            stderr_tail: String::new(),
        },
    );
    assert_eq!(shown(&panel, &mut visual), None);
}

/// §8.4: the row is a state, not an entry: the stored conversation (and the
/// transcript dump) hold the same entries with or without it, and none of
/// its texts.
#[gpui::test]
fn the_stored_conversation_does_not_contain_the_row(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    send(&panel, &mut visual, "hola");
    event(&panel, &mut visual, tool_call("t1"));
    assert_eq!(shown(&panel, &mut visual), Some("Trabajando…"));

    let (entries, dump) = panel.read_with(&visual, |panel, _| {
        (panel.entries().to_vec(), panel.export_transcript())
    });
    assert_eq!(entries.len(), 2, "el mensaje y la herramienta, nada más");
    assert!(matches!(entries[0], Entry::UserMessage(_)));
    assert!(matches!(entries[1], Entry::ToolCall(_)));

    let mut conversation = Conversation::new("c1", "claude-acp", "Claude", PathBuf::from("/p"), 0);
    conversation.entries = entries;
    let stored = serde_json::to_string(&conversation).expect("json");
    let dumped = serde_json::to_string(&dump).expect("json");
    for label in [
        AgentActivity::Thinking,
        AgentActivity::Working,
        AgentActivity::Writing,
        AgentActivity::WaitingPermission,
    ]
    .map(AgentActivity::label)
    {
        assert!(!stored.contains(label), "{label} en la conversación");
        assert!(!dumped.contains(label), "{label} en el volcado");
    }

    // Read back after the turn, the conversation has the same two entries and
    // no row once it is on screen.
    event(&panel, &mut visual, turn_ended(StopReason::EndTurn));
    let back: Conversation = serde_json::from_str(&stored).expect("se lee");
    assert_eq!(back.entries.len(), 2);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.load_conversation(back, window, cx);
    });
    visual.run_until_parked();
    panel.read_with(&visual, |panel, _| assert_eq!(panel.entries().len(), 2));
    assert_eq!(shown(&panel, &mut visual), None);
}
