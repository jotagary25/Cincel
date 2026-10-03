//! Long code blocks, folded (`docs/specs/10-etapa7-ronda2.md` §10.4): the
//! folded height, the real click on "Ver más" / "Ver menos", the state per
//! block, "Copiar", streaming, indented blocks, user bubbles and the saved
//! JSON.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{
    ContentBlock, ContentChunk, SessionId, SessionUpdate, StopReason, TextContent,
};
use gpui::{
    AnyWindowHandle, Bounds, Context, Entity, EntityId, ListOffset, Modifiers, Pixels, ScrollDelta,
    ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, Window, point, px, size,
};

use crate::actions::bind_default_keys;
use crate::code_folds::{CODE_FOLD_FADE_HEIGHT, CodeBlockKey, SegmentKind};
use crate::markdown::{code_line_height, folded_code_block_height};
use crate::model::*;
use crate::panel::ChatPanel;
use crate::settings::{CODE_BLOCK_HEADER, ChatSettings};
use crate::theme::ChatTheme;

/// What the panel asked to copy, in order.
type Copies = Rc<RefCell<Vec<String>>>;

/// A `width` × `height` window with a connected panel that has a session.
fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (Entity<ChatPanel>, VisualTestContext, Copies) {
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
    visual.simulate_resize(size(px(width), px(height)));
    let copies: Copies = Rc::new(RefCell::new(Vec::new()));
    let sink = copies.clone();
    cx.update(|cx| {
        cx.subscribe(&panel, move |_panel, event: &crate::ChatEvent, _cx| {
            if let crate::ChatEvent::CopyToClipboard(text) = event {
                sink.borrow_mut().push(text.clone());
            }
        })
        .detach();
    });
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
    (panel, visual, copies)
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

fn chunk(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext, text: &str) {
    let event = AgentEvent::Update {
        session_id: SessionId::new("s1"),
        update: Box::new(SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        ))),
    };
    panel.update(visual, |panel, cx| panel.handle_event(event, cx));
    frame(visual);
}

fn end_turn(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) {
    panel.update(visual, |panel, cx| {
        panel.handle_event(
            AgentEvent::TurnEnded {
                session_id: SessionId::new("s1"),
                stop_reason: StopReason::EndTurn,
            },
            cx,
        );
    });
    frame(visual);
}

/// The code lines of a test block.
fn code_lines(count: usize) -> Vec<String> {
    (0..count).map(|n| format!("let x{n} = {n};")).collect()
}

/// A fenced Rust block of `count` short lines (no wrapping in the panel).
fn block(count: usize) -> String {
    format!("```rust\n{}\n```\n", code_lines(count).join("\n"))
}

fn key(entry: usize, piece: usize, segment: usize) -> CodeBlockKey {
    CodeBlockKey {
        entry,
        piece,
        segment,
    }
}

/// `{prefix}-{entry}-{piece}-{segment}`, as `debug_bounds` wants it.
fn selector(prefix: &str, key: CodeBlockKey) -> &'static str {
    Box::leak(format!("{prefix}-{}", key.selector_suffix()).into_boxed_str())
}

fn block_bounds(visual: &mut VisualTestContext, key: CodeBlockKey) -> Option<Bounds<Pixels>> {
    visual.debug_bounds(selector("code-block", key))
}

fn toggle_bounds(visual: &mut VisualTestContext, key: CodeBlockKey) -> Option<Bounds<Pixels>> {
    visual.debug_bounds(selector("code-fold-toggle", key))
}

fn label(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext, key: CodeBlockKey) -> String {
    panel
        .read_with(visual, |panel, _| panel.code_block_toggle_label(key))
        .expect("el bloque tiene botón")
}

/// The code line height at `scale`, as the panel measures it.
fn line_height(visual: &mut VisualTestContext, scale: f32) -> Pixels {
    visual.update(|window, _| code_line_height(window, scale))
}

/// §10.2: the height of a folded block at `scale`.
fn folded_height(visual: &mut VisualTestContext, scale: f32) -> f32 {
    let line = line_height(visual, scale);
    f32::from(folded_code_block_height(line, scale))
}

/// The natural height of a block of `lines` lines at zoom 1: header and top
/// padding, the lines, the bottom padding and the border.
fn whole_height(visual: &mut VisualTestContext, lines: usize) -> f32 {
    let line = f32::from(line_height(visual, 1.));
    (CODE_BLOCK_HEADER + 6.) + line * lines as f32 + 8. + 2.
}

fn click(visual: &mut VisualTestContext, bounds: Bounds<Pixels>) {
    visual.simulate_click(bounds.center(), Modifiers::default());
    visual.run_until_parked();
    frame(visual);
}

fn assert_close(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= 1.,
        "{what}: {actual} px, se esperaban {expected} px (± 1)"
    );
}

/// The `TextViewState` of segment `segment` of the answer at `entry`.
fn segment_view(
    panel: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
    entry: usize,
    segment: usize,
) -> EntityId {
    panel.read_with(visual, |panel, _| match &panel.entries()[entry] {
        Entry::AgentText(text) => text.segments[segment].view.entity_id(),
        other => panic!("se esperaba AgentText, llegó {other:?}"),
    })
}

fn segment_kinds(
    panel: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
    entry: usize,
) -> Vec<SegmentKind> {
    panel.read_with(visual, |panel, _| match &panel.entries()[entry] {
        Entry::AgentText(text) => text.segments.iter().map(|segment| segment.kind).collect(),
        other => panic!("se esperaba AgentText, llegó {other:?}"),
    })
}

/// §10.4: a 30-line block comes folded at the height of §10.2 (± 1 px), with
/// its fade inside the border and "Ver más (18 líneas)" under it.
#[gpui::test]
fn a_thirty_line_block_comes_folded_at_the_height_of_the_spec(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx, 520., 1400.);
    send(&panel, &mut visual, "mostrame el archivo");
    chunk(
        &panel,
        &mut visual,
        &format!("Acá está:\n\n{}\nListo.\n", block(30)),
    );
    let long = key(1, 0, 1);
    assert_eq!(
        segment_kinds(&panel, &mut visual, 1),
        vec![
            SegmentKind::Prose,
            SegmentKind::LongCode {
                lines: 30,
                closed: true
            },
            SegmentKind::Prose
        ]
    );
    let bounds = block_bounds(&mut visual, long).expect("se pinta la caja del bloque");
    let expected = folded_height(&mut visual, 1.);
    assert_close(f32::from(bounds.size.height), expected, "alto plegado");
    // 12 lines and not 30: far shorter than the whole block.
    assert!(f32::from(bounds.size.height) < whole_height(&mut visual, 30) - 100.);
    assert_eq!(label(&panel, &mut visual, long), "Ver más (18 líneas)");

    // The button sits centred, 4 px under the box.
    let toggle = toggle_bounds(&mut visual, long).expect("se pinta el botón");
    assert_close(
        f32::from(toggle.top() - bounds.bottom()),
        4.,
        "separación del pie",
    );
    assert_close(
        f32::from(toggle.center().x),
        f32::from(bounds.center().x),
        "botón centrado",
    );

    // The fade: 40 px, 1 px in from the sides and the bottom.
    let fade = visual
        .debug_bounds(selector("code-fold-fade", long))
        .expect("se pinta el difuminado");
    assert_close(
        f32::from(fade.size.height),
        CODE_FOLD_FADE_HEIGHT,
        "alto del difuminado",
    );
    assert_close(
        f32::from(bounds.bottom() - fade.bottom()),
        1.,
        "margen de abajo",
    );
    assert_close(
        f32::from(fade.left() - bounds.left()),
        1.,
        "margen izquierdo",
    );
    assert_close(
        f32::from(bounds.right() - fade.right()),
        1.,
        "margen derecho",
    );
}

/// §10.4: 20 lines are not folded and have no button; 21 are, with
/// "Ver más (9 líneas)".
#[gpui::test]
fn twenty_lines_are_not_folded_and_twenty_one_are(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx, 520., 1400.);
    send(&panel, &mut visual, "dos bloques");
    chunk(
        &panel,
        &mut visual,
        &format!("{}\nY otro:\n\n{}", block(20), block(21)),
    );
    assert_eq!(
        segment_kinds(&panel, &mut visual, 1),
        vec![
            SegmentKind::Prose,
            SegmentKind::LongCode {
                lines: 21,
                closed: true
            }
        ],
        "el de 20 líneas queda en el texto corriente"
    );
    assert!(
        toggle_bounds(&mut visual, key(1, 0, 0)).is_none(),
        "20: sin botón"
    );
    assert!(
        block_bounds(&mut visual, key(1, 0, 0)).is_none(),
        "20: sin plegar"
    );
    let folded = block_bounds(&mut visual, key(1, 0, 1)).expect("21: plegado");
    assert_close(
        f32::from(folded.size.height),
        folded_height(&mut visual, 1.),
        "alto plegado de 21",
    );
    assert!(toggle_bounds(&mut visual, key(1, 0, 1)).is_some());
    assert_eq!(
        label(&panel, &mut visual, key(1, 0, 1)),
        "Ver más (9 líneas)"
    );
}

/// §10.4: a real click on the button unfolds the block to its 30 lines and
/// the button says "Ver menos"; another click folds it again.
#[gpui::test]
fn a_real_click_unfolds_and_another_folds_again(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx, 520., 1400.);
    send(&panel, &mut visual, "mostrame el archivo");
    chunk(&panel, &mut visual, &format!("Acá está:\n\n{}", block(30)));
    let long = key(1, 0, 1);
    let folded = f32::from(block_bounds(&mut visual, long).unwrap().size.height);

    let toggle = toggle_bounds(&mut visual, long).expect("botón");
    click(&mut visual, toggle);
    assert!(panel.read_with(&visual, |panel, _| panel.is_code_block_expanded(long)));
    let open_block = block_bounds(&mut visual, long).expect("caja");
    assert_close(
        f32::from(open_block.size.height),
        whole_height(&mut visual, 30),
        "desplegado: las 30 líneas",
    );
    assert!(f32::from(open_block.size.height) > folded + 100.);
    assert_eq!(label(&panel, &mut visual, long), "Ver menos");
    assert!(
        visual
            .debug_bounds(selector("code-fold-fade", long))
            .is_none(),
        "desplegado, sin difuminado"
    );

    let toggle = toggle_bounds(&mut visual, long).expect("botón");
    assert!(toggle.top() >= open_block.bottom(), "el pie sigue debajo");
    click(&mut visual, toggle);
    let again = block_bounds(&mut visual, long).expect("caja");
    assert_close(f32::from(again.size.height), folded, "plegado otra vez");
    assert_eq!(label(&panel, &mut visual, long), "Ver más (18 líneas)");
}

/// §10.4: each block keeps its own state (two long blocks in one answer),
/// across more chunks and a zoom change; opening another conversation and
/// coming back leaves them folded.
#[gpui::test]
fn each_block_keeps_its_state_until_the_conversation_is_reopened(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx, 520., 2400.);
    send(&panel, &mut visual, "dos archivos");
    chunk(
        &panel,
        &mut visual,
        &format!("Uno:\n\n{}\nDos:\n\n{}", block(30), block(25)),
    );
    let first = key(1, 0, 1);
    let second = key(1, 0, 3);
    assert_eq!(label(&panel, &mut visual, second), "Ver más (13 líneas)");
    let toggle = toggle_bounds(&mut visual, first).expect("botón del primero");
    click(&mut visual, toggle);
    assert_eq!(label(&panel, &mut visual, first), "Ver menos");
    assert_eq!(
        label(&panel, &mut visual, second),
        "Ver más (13 líneas)",
        "independientes"
    );

    // More chunks arrive: nothing changes.
    chunk(&panel, &mut visual, "\nY una nota al final.\n");
    chunk(&panel, &mut visual, "Otra línea más.\n");
    assert_eq!(label(&panel, &mut visual, first), "Ver menos");
    assert_eq!(label(&panel, &mut visual, second), "Ver más (13 líneas)");
    assert_close(
        f32::from(block_bounds(&mut visual, first).unwrap().size.height),
        whole_height(&mut visual, 30),
        "el primero sigue entero",
    );
    end_turn(&panel, &mut visual);

    // A zoom change: same states, sizes at the new zoom.
    panel.update(&mut visual, |panel, cx| {
        panel.set_settings(
            ChatSettings {
                scale: 1.25,
                ..ChatSettings::default()
            },
            cx,
        );
    });
    frame(&mut visual);
    assert_eq!(label(&panel, &mut visual, first), "Ver menos");
    assert_eq!(label(&panel, &mut visual, second), "Ver más (13 líneas)");
    assert_close(
        f32::from(block_bounds(&mut visual, second).unwrap().size.height),
        folded_height(&mut visual, 1.25),
        "plegado al 125 %",
    );
    panel.update(&mut visual, |panel, cx| {
        panel.set_settings(ChatSettings::default(), cx)
    });
    frame(&mut visual);

    // Another conversation, then this one again (through its JSON, as from
    // disk): folded.
    let mut this = Conversation::new("c1", "claude-acp", "Claude", PathBuf::from("/p"), 10);
    let json =
        serde_json::to_string(&panel.read_with(&visual, |panel, _| panel.entries().to_vec()))
            .expect("se serializa");
    this.entries = serde_json::from_str(&json).expect("se lee");
    let mut other = Conversation::new("c2", "claude-acp", "Claude", PathBuf::from("/p"), 11);
    other.entries = vec![Entry::UserMessage(UserMessage {
        blocks: vec![MessageBlock::Text("otra".into())],
    })];
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.load_conversation(other, window, cx);
    });
    frame(&mut visual);
    with_window(&panel, &mut visual, |panel, window, cx| {
        panel.load_conversation(this, window, cx);
    });
    frame(&mut visual);
    assert_eq!(
        label(&panel, &mut visual, first),
        "Ver más (18 líneas)",
        "volvió plegado"
    );
    assert_eq!(label(&panel, &mut visual, second), "Ver más (13 líneas)");
    assert_close(
        f32::from(block_bounds(&mut visual, first).unwrap().size.height),
        folded_height(&mut visual, 1.),
        "plegado al reabrir",
    );
}

/// §10.4: "Copiar" on a folded block copies the whole block, all 30 lines.
#[gpui::test]
fn copying_a_folded_block_copies_its_thirty_lines(cx: &mut TestAppContext) {
    let (panel, mut visual, copies) = open(cx, 520., 1400.);
    send(&panel, &mut visual, "mostrame el archivo");
    chunk(&panel, &mut visual, &format!("Acá está:\n\n{}", block(30)));
    let long = key(1, 0, 1);
    let bounds = block_bounds(&mut visual, long).expect("caja");
    // "Copiar" is the last thing of the header `gpui-kit` puts 8 px in from
    // the top-right corner (inside the 1 px border); 16 px tall.
    let spot = point(
        bounds.right() - px(1. + 8. + 12.),
        bounds.top() + px(1. + 8. + 8.),
    );
    visual.simulate_click(spot, Modifiers::default());
    visual.run_until_parked();
    let copied = copies.borrow().clone();
    assert_eq!(copied.len(), 1, "un CopyToClipboard: {copied:?}");
    assert_eq!(copied[0].trim_end_matches('\n'), code_lines(30).join("\n"));
    assert!(
        panel.read_with(&visual, |panel, _| !panel.is_code_block_expanded(long)),
        "copiar no despliega"
    );
}

/// §10.4: streamed line by line, the block folds at its 21st line and the
/// number follows; the text before it keeps its state (same entity).
#[gpui::test]
fn a_streamed_block_folds_at_its_twenty_first_line(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx, 520., 1400.);
    send(&panel, &mut visual, "escribilo de a poco");
    chunk(&panel, &mut visual, "Antes del bloque:\n\n```rust\n");
    let before = segment_view(&panel, &mut visual, 1, 0);
    let lines = code_lines(30);
    for (n, line) in lines.iter().enumerate() {
        chunk(&panel, &mut visual, &format!("{line}\n"));
        let count = n + 1;
        let kinds = segment_kinds(&panel, &mut visual, 1);
        if count < 21 {
            assert_eq!(kinds, vec![SegmentKind::Prose], "{count} líneas");
            assert!(toggle_bounds(&mut visual, key(1, 0, 1)).is_none());
        } else {
            assert_eq!(
                kinds,
                vec![
                    SegmentKind::Prose,
                    SegmentKind::LongCode {
                        lines: count,
                        closed: false
                    }
                ],
                "{count} líneas"
            );
            assert!(
                toggle_bounds(&mut visual, key(1, 0, 1)).is_some(),
                "{count}"
            );
            assert_eq!(
                label(&panel, &mut visual, key(1, 0, 1)),
                format!("Ver más ({} líneas)", count - 12)
            );
            assert_close(
                f32::from(block_bounds(&mut visual, key(1, 0, 1)).unwrap().size.height),
                folded_height(&mut visual, 1.),
                "plegado mientras llega",
            );
        }
        assert_eq!(
            segment_view(&panel, &mut visual, 1, 0),
            before,
            "el texto anterior no se vuelve a crear"
        );
    }
    let block_view = segment_view(&panel, &mut visual, 1, 1);
    chunk(&panel, &mut visual, "```\n\nListo.\n");
    assert_eq!(
        segment_kinds(&panel, &mut visual, 1),
        vec![
            SegmentKind::Prose,
            SegmentKind::LongCode {
                lines: 30,
                closed: true
            },
            SegmentKind::Prose
        ]
    );
    assert_eq!(segment_view(&panel, &mut visual, 1, 0), before);
    assert_eq!(segment_view(&panel, &mut visual, 1, 1), block_view);
    let markdown = panel.read_with(&visual, |panel, _| match &panel.entries()[1] {
        Entry::AgentText(text) => text.markdown.clone(),
        _ => unreachable!(),
    });
    assert_eq!(
        markdown,
        format!(
            "Antes del bloque:\n\n```rust\n{}\n```\n\nListo.\n",
            lines.join("\n")
        ),
        "el texto guardado es el que llegó"
    );
}

/// §10.4 (R8): a long block inside a list (indented) is not folded.
#[gpui::test]
fn a_long_block_inside_a_list_is_not_folded(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx, 520., 1400.);
    send(&panel, &mut visual, "pasos");
    let indented: Vec<String> = code_lines(30)
        .iter()
        .map(|line| format!("   {line}"))
        .collect();
    chunk(
        &panel,
        &mut visual,
        &format!(
            "1. Primero:\n\n   ```rust\n{}\n   ```\n2. Después.\n",
            indented.join("\n")
        ),
    );
    assert_eq!(
        segment_kinds(&panel, &mut visual, 1),
        vec![SegmentKind::Prose]
    );
    for segment in 0..3 {
        assert!(toggle_bounds(&mut visual, key(1, 0, segment)).is_none());
        assert!(block_bounds(&mut visual, key(1, 0, segment)).is_none());
    }
    // Shown whole: the answer is taller than the 30 lines.
    let answer = visual.debug_bounds("agent-text-1").expect("respuesta");
    assert!(f32::from(answer.size.height) > whole_height(&mut visual, 30));
}

/// §10.4: a user bubble with a 25-line block shows it folded.
#[gpui::test]
fn a_user_bubble_with_a_long_block_shows_it_folded(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx, 520., 1400.);
    send(&panel, &mut visual, &format!("Mirá esto:\n\n{}", block(25)));
    let long = key(0, 0, 1);
    assert_eq!(
        panel.read_with(&visual, |panel, _| panel.code_block_kind(long)),
        Some(SegmentKind::LongCode {
            lines: 25,
            closed: true
        })
    );
    let bounds = block_bounds(&mut visual, long).expect("plegado en la burbuja");
    assert_close(
        f32::from(bounds.size.height),
        folded_height(&mut visual, 1.),
        "alto plegado en la burbuja",
    );
    let bubble = visual.debug_bounds("user-bubble-0").expect("burbuja");
    assert!(bounds.left() >= bubble.left() && bounds.right() <= bubble.right());
    assert_eq!(label(&panel, &mut visual, long), "Ver más (13 líneas)");
    // Its button works like an answer's.
    let toggle = toggle_bounds(&mut visual, long).expect("botón");
    click(&mut visual, toggle);
    assert_eq!(label(&panel, &mut visual, long), "Ver menos");
    assert_close(
        f32::from(block_bounds(&mut visual, long).unwrap().size.height),
        whole_height(&mut visual, 25),
        "desplegado en la burbuja",
    );
    // One state per segment: the text and the block.
    let views = panel.read_with(&visual, |panel, _| panel.bubble_views.borrow().len());
    assert_eq!(views, 2);
}

/// §10.4: the saved JSON of the conversation keeps its format: an answer is
/// still `markdown` and `streaming`, nothing about segments or folds.
#[gpui::test]
fn the_saved_json_keeps_its_format(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx, 520., 1400.);
    send(&panel, &mut visual, "mostrame el archivo");
    chunk(&panel, &mut visual, &format!("Acá está:\n\n{}", block(30)));
    let toggle = toggle_bounds(&mut visual, key(1, 0, 1)).expect("botón");
    click(&mut visual, toggle);
    let entries = panel.read_with(&visual, |panel, _| panel.entries().to_vec());
    let value = serde_json::to_value(&entries[1]).expect("se serializa");
    let answer = value
        .get("AgentText")
        .and_then(|answer| answer.as_object())
        .expect("una respuesta");
    let mut fields: Vec<&str> = answer.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(fields, vec!["markdown", "streaming"]);
    assert_eq!(
        answer["markdown"].as_str(),
        Some(format!("Acá está:\n\n{}", block(30)).as_str())
    );
    // A file written before this change reads the same.
    let old: Entry =
        serde_json::from_str(r#"{"AgentText": {"markdown": "Hola", "streaming": false}}"#)
            .expect("formato de antes");
    assert!(
        matches!(old, Entry::AgentText(text) if text.markdown == "Hola" && text.segments.is_empty())
    );
    // And the dump of the whole transcript has no new field anywhere.
    let dump =
        serde_json::to_string(&panel.read_with(&visual, |panel, _| panel.export_transcript()))
            .expect("se serializa");
    assert!(
        !dump.contains("segments") && !dump.contains("expanded"),
        "{dump}"
    );
}

/// §10.1: folding or unfolding while following keeps the end in view;
/// scrolled up, the view stays where it was.
#[gpui::test]
fn toggling_keeps_the_end_when_following_and_the_view_otherwise(cx: &mut TestAppContext) {
    let (panel, mut visual, _) = open(cx, 520., 700.);
    send(&panel, &mut visual, "mostrame el archivo");
    chunk(&panel, &mut visual, &format!("Acá está:\n\n{}", block(40)));
    let long = key(1, 0, 1);
    assert!(panel.read_with(&visual, |panel, _| panel.is_following_tail()));
    let toggle = toggle_bounds(&mut visual, long).expect("botón");
    click(&mut visual, toggle);
    frame(&mut visual);
    assert!(panel.read_with(&visual, |panel, _| panel.is_following_tail()));
    assert_eq!(
        panel.read_with(&visual, |panel, _| panel.list.is_scrolled_to_end()),
        Some(true),
        "siguiendo, al final"
    );
    panel.update(&mut visual, |panel, cx| panel.toggle_code_block(long, cx));
    frame(&mut visual);
    end_turn(&panel, &mut visual);

    // Another long answer after it, then scroll up.
    send(&panel, &mut visual, "seguí");
    for n in 0..30 {
        chunk(
            &panel,
            &mut visual,
            &format!("Párrafo {n} de una respuesta larga que ocupa su lugar.\n\n"),
        );
    }
    let spot = visual
        .debug_bounds("chat-transcript")
        .expect("conversación")
        .center();
    visual.simulate_event(ScrollWheelEvent {
        position: spot,
        delta: ScrollDelta::Pixels(point(px(0.), px(600.))),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    frame(&mut visual);
    assert!(!panel.read_with(&visual, |panel, _| panel.is_following_tail()));
    let offset = |visual: &mut VisualTestContext| {
        let ListOffset {
            item_ix,
            offset_in_item,
        } = panel.read_with(visual, |panel, _| panel.transcript_scroll_offset());
        (item_ix, f32::from(offset_in_item))
    };
    let before = offset(&mut visual);
    panel.update(&mut visual, |panel, cx| panel.toggle_code_block(long, cx));
    frame(&mut visual);
    frame(&mut visual);
    assert_eq!(
        offset(&mut visual),
        before,
        "sin seguir, la vista no se mueve"
    );
    assert!(!panel.read_with(&visual, |panel, _| panel.is_following_tail()));
}
