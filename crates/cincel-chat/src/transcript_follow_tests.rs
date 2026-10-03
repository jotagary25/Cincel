//! Following the end of the transcript and the "Ir al final" arrow
//! (`docs/specs/10-etapa7-ronda2.md` §9.4), over a 380 × 500 window with real
//! wheel events on `chat-transcript` and a real click on the arrow.

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{
    ContentBlock, ContentChunk, PermissionOption, PermissionOptionKind, SessionId, SessionUpdate,
    StopReason, TextContent, ToolCall, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use gpui::{
    AnyWindowHandle, Context, Entity, ListOffset, Modifiers, ScrollDelta, ScrollWheelEvent,
    TestAppContext, TouchPhase, VisualTestContext, Window, point, px, size,
};

use crate::actions::bind_default_keys;
use crate::model::*;
use crate::panel::ChatPanel;
use crate::settings::ChatSettings;
use crate::theme::ChatTheme;

const ARROW: &str = "chat-jump-to-end";

/// A 380 × 500 window with a connected panel that has a session.
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
    visual.simulate_resize(size(px(380.), px(500.)));
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

/// Draws a fresh frame: what depends on the last layout is read after it.
fn frame(visual: &mut VisualTestContext) {
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
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
    frame(visual);
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

fn long_chunk(n: usize) -> AgentEvent {
    text_chunk(&format!(
        "Fragmento {n}: una línea larga de respuesta que ocupa el ancho del panel \
         y obliga a desplazarse para leerla entera.\n\n"
    ))
}

/// `count` long chunks of the answer, a frame after each.
fn stream(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext, from: usize, count: usize) {
    for n in from..from + count {
        event(panel, visual, long_chunk(n));
    }
}

/// A wheel turn over the middle of the transcript; a positive `dy` scrolls
/// up (towards the beginning).
fn wheel(visual: &mut VisualTestContext, dy: f32) {
    let spot = visual
        .debug_bounds("chat-transcript")
        .expect("conversación")
        .center();
    visual.simulate_event(ScrollWheelEvent {
        position: spot,
        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    frame(visual);
}

fn following(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) -> bool {
    panel.read_with(visual, |panel, _| panel.is_following_tail())
}

fn at_end(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) -> Option<bool> {
    panel.read_with(visual, |panel, _| panel.list.is_scrolled_to_end())
}

/// `transcript_scroll_offset()` as `(item, offset in the item)`: `ListOffset`
/// has no `PartialEq`.
fn offset(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) -> (usize, f32) {
    let ListOffset {
        item_ix,
        offset_in_item,
    } = panel.read_with(visual, |panel, _| panel.transcript_scroll_offset());
    (item_ix, f32::from(offset_in_item))
}

/// The arrow, as the panel decides it and as it is painted (they agree).
fn arrow(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) -> bool {
    frame(visual);
    let shows = panel.read_with(visual, |panel, _| panel.shows_jump_to_end());
    let painted = visual.debug_bounds(ARROW).is_some();
    assert_eq!(shows, painted, "la flecha se pinta cuando corresponde");
    shows
}

/// A long answer on screen, following the end.
fn long_answer(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) {
    send(panel, visual, "contame algo largo");
    stream(panel, visual, 0, 200);
}

/// §9.4: following, 200 long chunks leave the view at the end.
#[gpui::test]
fn following_two_hundred_long_chunks_keep_the_view_at_the_end(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    long_answer(&panel, &mut visual);
    assert!(following(&panel, &mut visual));
    assert_eq!(at_end(&panel, &mut visual), Some(true));
    assert!(!arrow(&panel, &mut visual), "siguiendo, sin flecha");
}

/// §9.4: after scrolling up, nothing that arrives moves the view: chunks, a
/// tool call, a permission request, the end of the turn and a zoom change.
#[gpui::test]
fn after_scrolling_up_nothing_moves_the_view(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    long_answer(&panel, &mut visual);
    wheel(&mut visual, 3000.);
    assert!(!following(&panel, &mut visual), "dejó de seguir");
    assert_eq!(at_end(&panel, &mut visual), Some(false));
    assert!(arrow(&panel, &mut visual));
    let before = offset(&panel, &mut visual);

    stream(&panel, &mut visual, 200, 20);
    assert_eq!(offset(&panel, &mut visual), before, "fragmentos");

    event(
        &panel,
        &mut visual,
        session_update(SessionUpdate::ToolCall(
            ToolCall::new("t1", "Ejecutar cargo test").kind(ToolKind::Execute),
        )),
    );
    assert_eq!(offset(&panel, &mut visual), before, "herramienta");

    panel.update(&mut visual, |panel, cx| {
        panel.request_permission(
            7,
            &ToolCallUpdate::new(
                "t1",
                ToolCallUpdateFields::new().title("Ejecutar cargo test"),
            ),
            vec![
                PermissionOption::new("allow", "Permitir", PermissionOptionKind::AllowOnce),
                PermissionOption::new("reject", "Rechazar", PermissionOptionKind::RejectOnce),
            ],
            cx,
        );
    });
    frame(&mut visual);
    assert_eq!(offset(&panel, &mut visual), before, "pedido de permiso");
    panel.update(&mut visual, |panel, cx| panel.accept_permission(cx));
    frame(&mut visual);
    assert_eq!(offset(&panel, &mut visual), before, "respuesta al permiso");

    event(
        &panel,
        &mut visual,
        AgentEvent::TurnEnded {
            session_id: SessionId::new("s1"),
            stop_reason: StopReason::EndTurn,
        },
    );
    assert_eq!(offset(&panel, &mut visual), before, "fin del turno");

    for scale in [1.25, 0.9, 1.0] {
        panel.update(&mut visual, |panel, cx| {
            panel.set_settings(
                ChatSettings {
                    scale,
                    ..ChatSettings::default()
                },
                cx,
            );
        });
        frame(&mut visual);
        frame(&mut visual);
        assert_eq!(offset(&panel, &mut visual), before, "zoom {scale}");
    }

    panel.update(&mut visual, |panel, cx| {
        panel.set_theme(ChatTheme::default(), cx);
    });
    frame(&mut visual);
    assert_eq!(offset(&panel, &mut visual), before, "tema");
    assert!(!following(&panel, &mut visual), "sigue sin seguir");
    assert!(arrow(&panel, &mut visual), "y la flecha sigue ahí");
}

/// §9.4: scrolling back down to the end follows again: the next chunk
/// leaves the view at the end.
#[gpui::test]
fn scrolling_down_to_the_end_follows_again(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    long_answer(&panel, &mut visual);
    wheel(&mut visual, 2000.);
    assert!(!following(&panel, &mut visual));
    wheel(&mut visual, -100_000.);
    assert!(following(&panel, &mut visual), "volvió a seguir");
    assert!(!arrow(&panel, &mut visual));
    stream(&panel, &mut visual, 200, 5);
    assert_eq!(at_end(&panel, &mut visual), Some(true));
    assert!(following(&panel, &mut visual));
}

/// §9.4: the arrow shows only after scrolling up; a real click on it goes to
/// the end, follows again, and it goes away.
#[gpui::test]
fn the_arrow_shows_after_scrolling_up_and_a_click_goes_to_the_end(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    long_answer(&panel, &mut visual);
    assert!(!arrow(&panel, &mut visual), "antes de subir, no");
    wheel(&mut visual, 1500.);
    assert!(arrow(&panel, &mut visual), "tras subir, sí");

    // §9.2: 28 px, centred over the transcript, 12 px above its bottom.
    let button = visual.debug_bounds(ARROW).expect("flecha");
    let transcript = visual
        .debug_bounds("chat-transcript")
        .expect("conversación");
    assert!(
        (f32::from(button.size.width) - 28.).abs() < 0.5,
        "{button:?}"
    );
    assert!(
        (f32::from(button.size.height) - 28.).abs() < 0.5,
        "{button:?}"
    );
    assert!(
        (f32::from(button.center().x) - f32::from(transcript.center().x)).abs() < 1.,
        "centrada: {button:?} en {transcript:?}"
    );
    assert!(
        (f32::from(transcript.bottom() - button.bottom()) - 12.).abs() < 0.5,
        "a 12 px del borde: {button:?} en {transcript:?}"
    );

    // Chunks keep arriving while it is up: it stays.
    stream(&panel, &mut visual, 200, 3);
    assert!(arrow(&panel, &mut visual));

    let spot = visual.debug_bounds(ARROW).expect("flecha").center();
    visual.simulate_click(spot, Modifiers::default());
    visual.run_until_parked();
    frame(&mut visual);
    assert!(following(&panel, &mut visual), "sigue otra vez");
    assert_eq!(at_end(&panel, &mut visual), Some(true), "al final");
    assert!(!arrow(&panel, &mut visual), "la flecha se fue");
}

/// §9.4: sending a message while scrolled up goes to the end and follows.
#[gpui::test]
fn sending_while_scrolled_up_goes_to_the_end_and_follows(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    long_answer(&panel, &mut visual);
    event(
        &panel,
        &mut visual,
        AgentEvent::TurnEnded {
            session_id: SessionId::new("s1"),
            stop_reason: StopReason::EndTurn,
        },
    );
    wheel(&mut visual, 2500.);
    assert!(!following(&panel, &mut visual));
    assert!(arrow(&panel, &mut visual));
    send(&panel, &mut visual, "seguí");
    frame(&mut visual);
    assert!(following(&panel, &mut visual), "sigue");
    assert_eq!(at_end(&panel, &mut visual), Some(true), "al final");
    assert!(!arrow(&panel, &mut visual));
    // And what the agent answers next stays in view.
    stream(&panel, &mut visual, 0, 10);
    assert_eq!(at_end(&panel, &mut visual), Some(true));
}

/// §9.4: a short conversation (nothing to scroll) never shows the arrow,
/// whatever the wheel does.
#[gpui::test]
fn a_short_conversation_never_shows_the_arrow(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    send(&panel, &mut visual, "hola");
    assert!(!arrow(&panel, &mut visual));
    event(&panel, &mut visual, text_chunk("Hola, ¿en qué te ayudo?"));
    assert!(!arrow(&panel, &mut visual));
    for dy in [400., -400., 5000., 1.] {
        wheel(&mut visual, dy);
        assert!(!arrow(&panel, &mut visual), "rueda {dy}");
    }
    event(
        &panel,
        &mut visual,
        AgentEvent::TurnEnded {
            session_id: SessionId::new("s1"),
            stop_reason: StopReason::EndTurn,
        },
    );
    assert!(!arrow(&panel, &mut visual));
    assert_eq!(at_end(&panel, &mut visual), None, "no hay desplazamiento");
}
