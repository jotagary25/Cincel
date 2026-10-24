//! The transcript's scrollbar (`docs/specs/12-etapa7-ronda4.md` §4.3): B1
//! to B6 over a 380 × 500 window, with the real panel, a simulated mouse and
//! the test clock. The track and the thumb are read from
//! [`ChatPanel::scrollbar_geometry`], what the last frame laid out.

use std::time::Duration;

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{
    ContentBlock, ContentChunk, SessionId, SessionUpdate, StopReason, TextContent,
};
use gpui::{
    AnyWindowHandle, Context, Entity, Modifiers, MouseButton, Point, ScrollDelta, ScrollWheelEvent,
    TestAppContext, TouchPhase, VisualTestContext, Window, point, px, size,
};

use crate::actions::bind_default_keys;
use crate::model::*;
use crate::panel::ChatPanel;
use crate::scrollbar::{
    SCROLLBAR_FADE, SCROLLBAR_HOLD, SCROLLBAR_MIN_THUMB, SCROLLBAR_WIDTH, ScrollbarGeometry,
};
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

/// Draws a fresh frame.
fn frame(visual: &mut VisualTestContext) {
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
}

/// Moves the test clock (timers included) and draws a frame.
fn wait(visual: &mut VisualTestContext, duration: Duration) {
    visual.executor().advance_clock(duration);
    visual.run_until_parked();
    frame(visual);
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

fn long_chunk(n: usize) -> AgentEvent {
    AgentEvent::Update {
        session_id: SessionId::new("s1"),
        update: Box::new(SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(format!(
                "Fragmento {n}: una línea larga de respuesta que ocupa el ancho del panel \
                 y obliga a desplazarse para leerla entera.\n\n"
            ))),
        ))),
    }
}

fn stream(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext, from: usize, count: usize) {
    for n in from..from + count {
        event(panel, visual, long_chunk(n));
    }
}

fn end_turn(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) {
    event(
        panel,
        visual,
        AgentEvent::TurnEnded {
            session_id: SessionId::new("s1"),
            stop_reason: StopReason::EndTurn,
        },
    );
}

/// A long finished answer, the bar already faded out.
fn long_conversation(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) {
    send(panel, visual, "contame algo largo");
    stream(panel, visual, 0, 120);
    end_turn(panel, visual);
    settle(visual);
}

/// Lets any visible bar fade out completely.
fn settle(visual: &mut VisualTestContext) {
    wait(
        visual,
        SCROLLBAR_HOLD + SCROLLBAR_FADE + Duration::from_millis(50),
    );
}

fn geometry(
    panel: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
) -> Option<ScrollbarGeometry> {
    panel.read_with(visual, |panel, _| panel.scrollbar_geometry())
}

fn transcript_center(visual: &mut VisualTestContext) -> Point<gpui::Pixels> {
    visual
        .debug_bounds("chat-transcript")
        .expect("conversación")
        .center()
}

/// The mouse moves over the middle of the transcript.
fn hover(visual: &mut VisualTestContext) {
    let spot = transcript_center(visual);
    visual.simulate_mouse_move(spot, None, Modifiers::default());
    frame(visual);
}

/// `(scrolled, max)` of the list, in pixels.
fn scroll(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) -> (f32, f32) {
    panel.read_with(visual, |panel, _| {
        let max = f32::from(panel.list.max_offset_for_scrollbar().y);
        let offset = f32::from(-panel.list.scroll_px_offset_for_scrollbar().y);
        (offset.clamp(0., max), max)
    })
}

fn following(panel: &Entity<ChatPanel>, visual: &mut VisualTestContext) -> bool {
    panel.read_with(visual, |panel, _| panel.is_following_tail())
}

/// The thumb's expected height for the list's current numbers.
fn expected_thumb(track: f32, max: f32) -> f32 {
    (track / (max + track) * track)
        .max(SCROLLBAR_MIN_THUMB)
        .min(track)
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

/// B1: everything fits, so there is no bar, whatever the mouse does.
#[gpui::test]
fn b1_a_short_transcript_has_no_scrollbar(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    send(&panel, &mut visual, "hola");
    event(&panel, &mut visual, long_chunk(0));
    end_turn(&panel, &mut visual);
    assert_eq!(scroll(&panel, &mut visual).1, 0., "nada que desplazar");
    hover(&mut visual);
    assert_eq!(geometry(&panel, &mut visual), None);
    let center = transcript_center(&mut visual);
    for dy in [-40., 40.] {
        visual.simulate_mouse_move(
            point(center.x, center.y + px(dy)),
            None,
            Modifiers::default(),
        );
        frame(&mut visual);
        assert_eq!(geometry(&panel, &mut visual), None, "movimiento {dy}");
    }
}

/// B2: a long transcript shows no bar until the mouse moves over it; then
/// an 8 px track against the transcript's right edge and a proportional
/// thumb where the scroll is.
#[gpui::test]
fn b2_the_mouse_shows_a_proportional_thumb(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    long_conversation(&panel, &mut visual);
    frame(&mut visual);
    assert_eq!(
        geometry(&panel, &mut visual),
        None,
        "sin ratón ni desplazamiento, no hay barra"
    );

    hover(&mut visual);
    let bar = geometry(&panel, &mut visual).expect("barra visible");
    assert_eq!(bar.alpha, 1.);
    let transcript = visual
        .debug_bounds("chat-transcript")
        .expect("conversación");
    assert!(close(
        f32::from(bar.track.size.width),
        SCROLLBAR_WIDTH,
        0.01
    ));
    assert!(close(
        f32::from(bar.track.right()),
        f32::from(transcript.right()),
        0.5
    ));
    assert!(close(
        f32::from(bar.track.size.height),
        f32::from(transcript.size.height),
        0.5
    ));
    let (offset, max) = scroll(&panel, &mut visual);
    assert!(max > 0.);
    let track = f32::from(bar.track.size.height);
    let thumb = f32::from(bar.thumb.size.height);
    assert!(close(thumb, expected_thumb(track, max), 0.5), "{bar:?}");
    assert!(thumb >= SCROLLBAR_MIN_THUMB && thumb <= track);
    assert!(close(offset, max, 1.), "al final");
    assert!(
        close(
            f32::from(bar.thumb.bottom()),
            f32::from(bar.track.bottom()),
            0.5
        ),
        "al final, el pulgar abajo: {bar:?}"
    );

    // The wheel up: the thumb is where the scroll is.
    let spot = transcript_center(&mut visual);
    visual.simulate_event(ScrollWheelEvent {
        position: spot,
        delta: ScrollDelta::Pixels(point(px(0.), px(1500.))),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    frame(&mut visual);
    let bar = geometry(&panel, &mut visual).expect("barra visible");
    let (offset, max) = scroll(&panel, &mut visual);
    let travel = track - f32::from(bar.thumb.size.height);
    assert!(
        close(
            f32::from(bar.thumb.top() - bar.track.top()),
            offset / max * travel,
            0.5
        ),
        "{bar:?}, {offset} de {max}"
    );
}

/// B3 and R9: after 1000 ms without the mouse or a scroll it fades, and
/// 200 ms later it is gone; while invisible it asks for no frames.
#[gpui::test]
fn b3_it_fades_out_after_the_hold(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    long_conversation(&panel, &mut visual);
    hover(&mut visual);
    assert_eq!(geometry(&panel, &mut visual).map(|bar| bar.alpha), Some(1.));

    wait(&mut visual, Duration::from_millis(900));
    assert_eq!(
        geometry(&panel, &mut visual).map(|bar| bar.alpha),
        Some(1.),
        "a los 900 ms sigue entera"
    );
    wait(&mut visual, Duration::from_millis(200));
    let alpha = geometry(&panel, &mut visual)
        .expect("desvaneciéndose")
        .alpha;
    assert!(
        alpha > 0. && alpha < 1.,
        "a los 1100 ms se desvanece: {alpha}"
    );
    let fading_frames = panel.read_with(&visual, |panel, _| panel.scrollbar_frames_requested());
    assert!(fading_frames > 0, "el desvanecimiento pide cuadros");

    wait(&mut visual, Duration::from_millis(100));
    assert_eq!(geometry(&panel, &mut visual), None, "a los 1200 ms no está");
    let gone_frames = panel.read_with(&visual, |panel, _| panel.scrollbar_frames_requested());
    for _ in 0..3 {
        frame(&mut visual);
    }
    assert_eq!(geometry(&panel, &mut visual), None);
    assert_eq!(
        panel.read_with(&visual, |panel, _| panel.scrollbar_frames_requested()),
        gone_frames,
        "invisible, no pide cuadros"
    );

    // R6: a scroll shows it again.
    let spot = transcript_center(&mut visual);
    visual.simulate_event(ScrollWheelEvent {
        position: spot,
        delta: ScrollDelta::Pixels(point(px(0.), px(300.))),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    frame(&mut visual);
    assert_eq!(
        geometry(&panel, &mut visual).map(|bar| bar.alpha),
        Some(1.),
        "la rueda la muestra"
    );
}

/// R6: while the mouse rests on the track, it does not fade.
#[gpui::test]
fn resting_on_the_track_keeps_it_visible(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    long_conversation(&panel, &mut visual);
    hover(&mut visual);
    let bar = geometry(&panel, &mut visual).expect("barra visible");
    let on_track = point(bar.track.center().x, bar.track.top() + px(10.));
    visual.simulate_mouse_move(on_track, None, Modifiers::default());
    frame(&mut visual);
    settle(&mut visual);
    assert_eq!(
        geometry(&panel, &mut visual).map(|bar| bar.alpha),
        Some(1.),
        "sobre la pista no se va"
    );
    // Leaving the track starts the hold again.
    let spot = transcript_center(&mut visual);
    visual.simulate_mouse_move(spot, None, Modifiers::default());
    frame(&mut visual);
    settle(&mut visual);
    assert_eq!(geometry(&panel, &mut visual), None);
}

/// B4: dragging the thumb to the bottom leaves the list at the end, to the
/// top at the beginning, and in the middle at the proportional offset.
#[gpui::test]
fn b4_dragging_the_thumb_scrolls_in_proportion(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    long_conversation(&panel, &mut visual);
    hover(&mut visual);
    let bar = geometry(&panel, &mut visual).expect("barra visible");
    let grab = bar.thumb.center();
    let grab_offset = grab.y - bar.thumb.top();
    let travel = f32::from(bar.track.size.height - bar.thumb.size.height);

    visual.simulate_mouse_down(grab, MouseButton::Left, Modifiers::default());
    frame(&mut visual);
    assert!(panel.read_with(&visual, |panel, _| panel.is_dragging_scrollbar()));

    // To the top (past it).
    visual.simulate_mouse_move(
        point(grab.x, bar.track.top() - px(80.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    frame(&mut visual);
    assert_eq!(scroll(&panel, &mut visual).0, 0., "al principio");
    assert!(!following(&panel, &mut visual));

    // To the middle.
    let middle = bar.track.top() + grab_offset + px(travel / 2.);
    visual.simulate_mouse_move(
        point(grab.x, middle),
        MouseButton::Left,
        Modifiers::default(),
    );
    frame(&mut visual);
    let (offset, max) = scroll(&panel, &mut visual);
    assert!(close(offset, max / 2., 2.), "{offset} de {max}");
    let thumb = geometry(&panel, &mut visual).expect("barra visible").thumb;
    assert!(
        close(f32::from(thumb.top() - bar.track.top()), travel / 2., 1.),
        "{thumb:?}"
    );

    // To the bottom (past it).
    visual.simulate_mouse_move(
        point(grab.x, bar.track.bottom() + px(80.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    frame(&mut visual);
    visual.simulate_mouse_up(
        point(grab.x, bar.track.bottom() + px(80.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    frame(&mut visual);
    assert!(!panel.read_with(&visual, |panel, _| panel.is_dragging_scrollbar()));
    assert_eq!(
        panel.read_with(&visual, |panel, _| panel.list.is_scrolled_to_end()),
        Some(true),
        "al final"
    );
    assert!(
        following(&panel, &mut visual),
        "abajo del todo vuelve a seguir"
    );
    assert!(visual.debug_bounds(ARROW).is_none());
}

/// B5: a click on the track outside the thumb centres the thumb there; the
/// press is the bar's and does not move the keyboard focus (R9).
#[gpui::test]
fn b5_a_click_on_the_track_centres_the_thumb(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    long_conversation(&panel, &mut visual);
    let focused_before = visual.update(|window, cx| window.focused(cx));
    hover(&mut visual);
    let bar = geometry(&panel, &mut visual).expect("barra visible");
    let click = point(
        bar.track.center().x,
        bar.track.top() + bar.track.size.height * 0.35,
    );
    assert!(!bar.thumb.contains(&click), "fuera del pulgar: {bar:?}");

    visual.simulate_mouse_down(click, MouseButton::Left, Modifiers::default());
    frame(&mut visual);
    visual.simulate_mouse_up(click, MouseButton::Left, Modifiers::default());
    frame(&mut visual);
    let thumb = geometry(&panel, &mut visual).expect("barra visible").thumb;
    assert!(
        close(f32::from(thumb.center().y), f32::from(click.y), 1.),
        "centrado en el clic: {thumb:?} / {click:?}"
    );
    assert!(!following(&panel, &mut visual));
    assert!(
        visual.debug_bounds(ARROW).is_some(),
        "aparece «Ir al final»"
    );
    let focused_after = visual.update(|window, cx| window.focused(cx));
    assert_eq!(focused_before, focused_after, "el foco no se mueve");
}

/// B6 and R8: while the agent writes and the tail is followed, the bar shows
/// with the thumb at the bottom; dragging it up stops following and shows
/// "Ir al final"; what arrives next does not pull the view down.
#[gpui::test]
fn b6_dragging_while_the_agent_writes_stops_following(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    send(&panel, &mut visual, "contame algo largo");
    stream(&panel, &mut visual, 0, 120);
    assert!(following(&panel, &mut visual));
    let bar = geometry(&panel, &mut visual).expect("siguiendo la cola, la barra se ve");
    assert_eq!(bar.alpha, 1.);
    assert!(
        close(
            f32::from(bar.thumb.bottom()),
            f32::from(bar.track.bottom()),
            0.5
        ),
        "pulgar abajo: {bar:?}"
    );
    assert!(visual.debug_bounds(ARROW).is_none());

    let grab = bar.thumb.center();
    visual.simulate_mouse_down(grab, MouseButton::Left, Modifiers::default());
    frame(&mut visual);
    visual.simulate_mouse_move(
        point(grab.x, grab.y - px(120.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    frame(&mut visual);
    assert!(
        !following(&panel, &mut visual),
        "el arrastre pausa el seguimiento"
    );
    let before = scroll(&panel, &mut visual).0;

    stream(&panel, &mut visual, 120, 10);
    assert!(
        close(scroll(&panel, &mut visual).0, before, 0.5),
        "la respuesta no arrastra la vista"
    );
    visual.simulate_mouse_up(
        point(grab.x, grab.y - px(120.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    frame(&mut visual);
    assert!(!following(&panel, &mut visual));
    assert!(
        visual.debug_bounds(ARROW).is_some(),
        "aparece «Ir al final»"
    );
    stream(&panel, &mut visual, 130, 5);
    assert!(close(scroll(&panel, &mut visual).0, before, 0.5));
}
