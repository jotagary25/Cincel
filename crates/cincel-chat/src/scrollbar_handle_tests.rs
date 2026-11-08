//! The transcript's scrollbar next to the dock's resize handle
//! (`docs/specs/13-etapa7-ronda5.md` §3.3, C1–C3).
//!
//! The panel is hosted the way the Left dock hosts it: gpui-base's handle is
//! an occluding element painted after the panel over the strip from −6 to
//! −1 px of its right edge (`resizable/resize_handle.rs`, `Side::Left`). The
//! host reproduces that strip and counts the presses it gets, plus a
//! window-level bubble listener painted before everything, which only sees a
//! press nobody stopped.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{
    ContentBlock, ContentChunk, SessionId, SessionUpdate, StopReason, TextContent,
};
use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    Modifiers, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, canvas, div, point, px, size,
};

use crate::actions::bind_default_keys;
use crate::model::*;
use crate::panel::ChatPanel;
use crate::scrollbar::{SCROLLBAR_FADE, SCROLLBAR_HOLD, SCROLLBAR_INSET, SCROLLBAR_WIDTH};
use crate::settings::ChatSettings;
use crate::theme::ChatTheme;

/// The dock's resize handle strip: from `right − 6` to `right − 1`.
const HANDLE_RIGHT: f32 = 1.;
const HANDLE_WIDTH: f32 = 5.;

/// The panel inside a stand-in for the Left dock.
struct Dock {
    panel: Entity<ChatPanel>,
    /// Presses the resize handle received.
    handle_presses: Rc<Cell<u32>>,
    /// Presses that reached the bottom of the bubble phase (not stopped).
    unstopped_presses: Rc<Cell<u32>>,
}

impl Render for Dock {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let unstopped = self.unstopped_presses.clone();
        let handle = self.handle_presses.clone();
        div()
            .relative()
            .size_full()
            .child(
                // Painted first, so its bubble listener runs last.
                canvas(
                    |_, _, _| (),
                    move |_, (), window, _| {
                        let unstopped = unstopped.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, _, _| {
                            if phase.bubble() && event.button == MouseButton::Left {
                                unstopped.set(unstopped.get() + 1);
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(self.panel.clone())
            .child(
                div()
                    .id("dock-resize-handle")
                    .occlude()
                    .absolute()
                    .top_0()
                    .right(px(HANDLE_RIGHT))
                    .h_full()
                    .w(px(HANDLE_WIDTH))
                    .on_mouse_down(MouseButton::Left, move |_, _, _| {
                        handle.set(handle.get() + 1);
                    }),
            )
    }
}

struct Harness {
    panel: Entity<ChatPanel>,
    visual: VisualTestContext,
    handle_presses: Rc<Cell<u32>>,
    unstopped_presses: Rc<Cell<u32>>,
}

/// A 380 × 500 dock with a connected panel that has a long, finished answer.
fn open(cx: &mut TestAppContext) -> Harness {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_default_keys(cx);
    });
    let handle_presses = Rc::new(Cell::new(0));
    let unstopped_presses = Rc::new(Cell::new(0));
    let window = cx.add_window({
        let handle_presses = handle_presses.clone();
        let unstopped_presses = unstopped_presses.clone();
        move |window, cx| Dock {
            panel: cx.new(|cx| {
                ChatPanel::new(ChatTheme::default(), ChatSettings::default(), window, cx)
            }),
            handle_presses,
            unstopped_presses,
        }
    });
    let panel = window.update(cx, |dock, _, _| dock.panel.clone()).unwrap();
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
    let mut harness = Harness {
        panel,
        visual,
        handle_presses,
        unstopped_presses,
    };
    harness.long_conversation();
    harness
}

impl Harness {
    fn frame(&mut self) {
        self.visual.update(|window, _| window.refresh());
        self.visual.run_until_parked();
    }

    fn event(&mut self, event: AgentEvent) {
        self.panel
            .update(&mut self.visual, |panel, cx| panel.handle_event(event, cx));
        self.frame();
    }

    fn long_conversation(&mut self) {
        let panel = self.panel.clone();
        self.visual.update(|window, cx| {
            panel.update(cx, |panel, cx| {
                panel.set_input_text("contame algo largo", window, cx);
                panel.send(window, cx);
            });
        });
        self.visual.run_until_parked();
        for n in 0..120 {
            self.event(AgentEvent::Update {
                session_id: SessionId::new("s1"),
                update: Box::new(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                    ContentBlock::Text(TextContent::new(format!(
                        "Fragmento {n}: una línea larga de respuesta que ocupa el ancho del \
                         panel y obliga a desplazarse para leerla entera.\n\n"
                    ))),
                ))),
            });
        }
        self.event(AgentEvent::TurnEnded {
            session_id: SessionId::new("s1"),
            stop_reason: StopReason::EndTurn,
        });
        self.visual
            .executor()
            .advance_clock(SCROLLBAR_HOLD + SCROLLBAR_FADE + Duration::from_millis(50));
        self.visual.run_until_parked();
        self.frame();
    }

    /// The mouse over the middle of the transcript, so the bar shows.
    fn show_bar(&mut self) -> gpui::Bounds<Pixels> {
        let center = self
            .visual
            .debug_bounds("chat-transcript")
            .expect("conversación")
            .center();
        self.visual
            .simulate_mouse_move(center, None, Modifiers::default());
        self.frame();
        self.panel
            .read_with(&self.visual, |panel, _| panel.scrollbar_geometry())
            .expect("barra visible")
            .track
    }

    fn transcript_right(&mut self) -> Pixels {
        self.visual
            .debug_bounds("chat-transcript")
            .expect("conversación")
            .right()
    }

    fn press(&mut self, x: Pixels, y: Pixels) {
        self.visual
            .simulate_mouse_down(point(x, y), MouseButton::Left, Modifiers::default());
        self.frame();
    }

    fn dragging(&self) -> bool {
        self.panel
            .read_with(&self.visual, |panel, _| panel.is_dragging_scrollbar())
    }
}

/// R5: the track is 8 px wide at `[right − 14, right − 6]`, the thumb inside.
#[gpui::test]
fn the_track_sits_six_pixels_in_from_the_edge(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let right = h.transcript_right();
    assert_eq!(right, px(380.), "el panel llena el dock");
    let bar = {
        h.show_bar();
        h.panel
            .read_with(&h.visual, |panel, _| panel.scrollbar_geometry())
            .expect("barra visible")
    };
    assert_eq!(bar.track.left(), right - px(14.));
    assert_eq!(bar.track.right(), right - px(SCROLLBAR_INSET));
    assert_eq!(f32::from(bar.track.size.width), SCROLLBAR_WIDTH);
    assert!(bar.thumb.left() >= bar.track.left() && bar.thumb.right() <= bar.track.right());
}

/// C1 and R6: with the bar visible, a press on the handle's strip
/// (`right − 3`) is the handle's: the bar does not drag and the press is not
/// stopped.
#[gpui::test]
fn c1_a_press_on_the_handle_strip_is_not_the_bars(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let track = h.show_bar();
    let right = h.transcript_right();
    h.press(right - px(3.), track.center().y);
    assert!(!h.dragging(), "la barra no arrastra");
    assert_eq!(h.handle_presses.get(), 1, "el asa recibe el clic");
    assert_eq!(h.unstopped_presses.get(), 1, "el evento no se detiene");
    let (offset, max) = h.panel.read_with(&h.visual, |panel, _| {
        (
            f32::from(-panel.list.scroll_px_offset_for_scrollbar().y),
            f32::from(panel.list.max_offset_for_scrollbar().y),
        )
    });
    assert!(
        (offset - max).abs() <= 1.,
        "la vista no salta: {offset} de {max}"
    );
}

/// C2 and R6: a press on the track (`right − 10`) still drags the bar, and
/// the press is the bar's alone.
#[gpui::test]
fn c2_a_press_on_the_track_drags(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let bar = {
        h.show_bar();
        h.panel
            .read_with(&h.visual, |panel, _| panel.scrollbar_geometry())
            .expect("barra visible")
    };
    let right = h.transcript_right();
    h.press(right - px(10.), bar.thumb.center().y);
    assert!(h.dragging(), "la barra arrastra");
    assert_eq!(h.handle_presses.get(), 0, "el asa no lo ve");
    assert_eq!(h.unstopped_presses.get(), 0, "la barra lo detiene");

    // The drag still moves the list.
    h.visual.simulate_mouse_move(
        point(right - px(10.), bar.track.top() - px(80.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    h.frame();
    let offset = h.panel.read_with(&h.visual, |panel, _| {
        f32::from(-panel.list.scroll_px_offset_for_scrollbar().y)
    });
    assert!(offset.abs() <= 0.5, "al principio: {offset}");
    h.visual.simulate_mouse_up(
        point(right - px(10.), bar.track.top() - px(80.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    h.frame();
    assert!(!h.dragging());
}
