//! Paragraphs with `@file` chips are one selectable text
//! (`docs/specs/13-etapa7-ronda5.md` §3.4, D1t–D3t).
//!
//! Selection in gpui-base is window-level: it only works with a
//! `TextSelectionLayer` in the tree (the app gets it from
//! `gpui_component::Root`), so the panel is hosted under one, painted first
//! as `Root` does. The selection is read with `TextSelection::selected_text`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{
    ContentBlock, ContentChunk, SessionId, SessionUpdate, StopReason, TextContent,
};
use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Pixels, Render, Styled as _, TestAppContext, VisualTestContext, Window,
    div, point, px, size,
};
use gpui_kit::base::{TextSelection, TextSelectionLayer};

use crate::actions::bind_default_keys;
use crate::model::*;
use crate::panel::ChatPanel;
use crate::render::{FlowPart, flow_markdown};
use crate::settings::ChatSettings;
use crate::theme::ChatTheme;

/// What the panel emitted, as debug strings.
type Events = Rc<RefCell<Vec<String>>>;

/// The panel under a text selection layer, the way `Root` hosts it.
struct Host {
    panel: Entity<ChatPanel>,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(TextSelectionLayer)
            .child(self.panel.clone())
    }
}

struct Harness {
    panel: Entity<ChatPanel>,
    visual: VisualTestContext,
    events: Events,
}

/// A 420 × 600 window with a connected panel that has a session.
fn open(cx: &mut TestAppContext) -> Harness {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_default_keys(cx);
    });
    let window = cx.add_window(|window, cx| Host {
        panel: cx
            .new(|cx| ChatPanel::new(ChatTheme::default(), ChatSettings::default(), window, cx)),
    });
    let panel = window.update(cx, |host, _, _| host.panel.clone()).unwrap();
    let handle: AnyWindowHandle = window.into();
    let mut visual = VisualTestContext::from_window(handle, cx);
    visual.simulate_resize(size(px(420.), px(600.)));
    let events: Events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|cx| {
        cx.subscribe(&panel, move |_panel, event: &crate::ChatEvent, _cx| {
            sink.borrow_mut().push(format!("{event:?}"));
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
    Harness {
        panel,
        visual,
        events,
    }
}

impl Harness {
    fn frame(&mut self) {
        self.visual.update(|window, _| window.refresh());
        self.visual.run_until_parked();
    }

    /// Sends "mirá @a.rs y @b.rs y decime", typed through the composer with
    /// two real mentions.
    fn send_with_mentions(&mut self) {
        let panel = self.panel.clone();
        self.visual.update(|window, cx| {
            panel.update(cx, |panel, cx| {
                panel.set_input_text("mirá ", window, cx);
                panel.insert_mention(PathBuf::from("/proyecto/src/a.rs"), window, cx);
                panel.set_input_text("mirá @a.rs y ", window, cx);
                panel.insert_mention(PathBuf::from("/proyecto/src/b.rs"), window, cx);
                panel.set_input_text("mirá @a.rs y @b.rs y decime", window, cx);
                panel.send(window, cx);
            });
        });
        self.visual.run_until_parked();
        // The paragraph is parsed off the main thread; a second frame paints
        // the parsed text and its chips.
        self.frame();
        self.frame();
    }

    fn send_plain(&mut self, text: &str) {
        let panel = self.panel.clone();
        self.visual.update(|window, cx| {
            panel.update(cx, |panel, cx| {
                panel.set_input_text(text, window, cx);
                panel.send(window, cx);
            });
        });
        self.visual.run_until_parked();
        self.frame();
    }

    fn answer(&mut self, text: &str) {
        for event in [
            AgentEvent::Update {
                session_id: SessionId::new("s1"),
                update: Box::new(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                    ContentBlock::Text(TextContent::new(text)),
                ))),
            },
            AgentEvent::TurnEnded {
                session_id: SessionId::new("s1"),
                stop_reason: StopReason::EndTurn,
            },
        ] {
            self.panel
                .update(&mut self.visual, |panel, cx| panel.handle_event(event, cx));
            self.frame();
        }
    }

    fn bounds(&mut self, selector: impl Into<String>) -> Bounds<Pixels> {
        // `debug_bounds` takes a `&'static str`; a test can afford the leak.
        let selector: &'static str = Box::leak(selector.into().into_boxed_str());
        self.visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("no se pintó {selector}"))
    }

    /// Drags from just inside the left edge of `paragraph` to just inside its
    /// right edge, along its first line.
    fn drag_across(&mut self, paragraph: Bounds<Pixels>, chip: Bounds<Pixels>) {
        let y = chip.center().y;
        let from = point(paragraph.left() + px(1.), y);
        let to = point(paragraph.right() - px(1.), y);
        self.visual
            .simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        self.frame();
        for step in 1..=4 {
            let x = from.x + (to.x - from.x) * (step as f32 / 4.);
            self.visual
                .simulate_mouse_move(point(x, y), MouseButton::Left, Modifiers::default());
            self.frame();
        }
        self.visual
            .simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        self.frame();
    }

    /// The selected text. A whole paragraph ends with the line break the
    /// library adds after every selected block (an answer's paragraph copies
    /// the same way).
    fn selected_text(&mut self) -> String {
        self.visual.update(TextSelection::selected_text)
    }
}

/// The paragraph's source keeps the text literal and the old spacing.
#[test]
fn the_flow_source_escapes_the_text_and_links_the_chips() {
    let (source, files) = flow_markdown(&[
        FlowPart::Text("mirá *esto* ".into()),
        FlowPart::File(PathBuf::from("/p/a.rs")),
        FlowPart::File(PathBuf::from("/p/b.rs")),
        FlowPart::Text(", y\n  1. decime".into()),
    ]);
    assert_eq!(
        source,
        "mirá \\*esto\\* [\\@a\\.rs](cincel-chip:0) [\\@b\\.rs](cincel-chip:1)\\, y\\\n1\\. decime"
    );
    assert_eq!(
        files,
        vec![PathBuf::from("/p/a.rs"), PathBuf::from("/p/b.rs")]
    );
}

/// D1t and R7: after an earlier exchange, dragging over "mirá @a.rs y @b.rs
/// y decime" selects it and the copied text carries each chip as `@name`.
#[gpui::test]
fn d1_dragging_over_a_paragraph_with_chips_selects_it(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.send_plain("hola");
    h.answer("Hola, ¿en qué te ayudo?");
    h.send_with_mentions();
    let entry = h
        .panel
        .read_with(&h.visual, |panel, _| panel.entries().len())
        - 1;
    assert!(entry > 0, "no es el primer mensaje");
    let paragraph = h.bounds(format!("user-flow-{entry}-0"));
    let chip = h.bounds(format!("message-chip-{entry}-0-0"));
    assert!(
        paragraph.contains(&chip.center()),
        "la ficha va dentro del párrafo: {chip:?} en {paragraph:?}"
    );
    h.drag_across(paragraph, chip);
    assert_eq!(h.selected_text(), "mirá @a.rs y @b.rs y decime\n");
}

/// D2t: the first message of the conversation, with references, selects
/// too, and both chips sit on the text's line, in order.
#[gpui::test]
fn d2_the_first_message_with_chips_selects(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.send_with_mentions();
    let paragraph = h.bounds("user-flow-0-0");
    let first = h.bounds("message-chip-0-0-0");
    let second = h.bounds("message-chip-0-0-1");
    assert!(
        first.right() < second.left(),
        "{first:?} antes de {second:?}"
    );
    assert!(
        (first.center().y - second.center().y).abs() < px(1.),
        "en la misma línea: {first:?} / {second:?}"
    );
    h.drag_across(paragraph, first);
    assert_eq!(h.selected_text(), "mirá @a.rs y @b.rs y decime\n");
}

/// D3t and R8: a click on a chip opens its file, as "Ver en el editor" does.
#[gpui::test]
fn d3_a_click_on_a_chip_opens_the_file(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.send_with_mentions();
    let chip = h.bounds("message-chip-0-0-1");
    h.events.borrow_mut().clear();
    h.visual.simulate_click(chip.center(), Modifiers::default());
    h.frame();
    let events = h.events.borrow().clone();
    assert!(
        events
            .iter()
            .any(|event| event.contains("OpenFileAtHunk") && event.contains("/proyecto/src/b.rs")),
        "{events:?}"
    );
    assert_eq!(h.selected_text(), "", "un clic no selecciona");
}
