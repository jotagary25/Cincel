//! The chat header over the GPUI test harness
//! (`docs/specs/10-etapa7-ronda2.md` §3): the full connection name, a single
//! badge with the state of the *connection* by priority, nothing else.
//!
//! The painted word is checked through the width of the badge: the pill is its
//! text plus the padding, and the five labels have five different widths.

use gpui::{AnyWindowHandle, Entity, Pixels, TestAppContext, VisualTestContext, Window, px, size};

use crate::actions::bind_default_keys;
use crate::attachments_tests::{connect, open as open_with_session};
use crate::model::*;
use crate::panel::ChatPanel;
use crate::settings::{ChatSettings, TEXT_BODY, TEXT_LABEL};
use crate::theme::ChatTheme;

/// Horizontal padding of the pill (`px_1p5`, 6 px on each side at the default
/// rem size).
const PILL_PADDING: f32 = 12.;

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
    (panel, VisualTestContext::from_window(handle, cx))
}

/// Resizes the window, which is what makes the harness draw a frame.
fn paint(visual: &mut VisualTestContext, width: f32) {
    visual.simulate_resize(size(px(width), px(720.)));
    visual.run_until_parked();
}

fn connection(label: &str, badge: ConnectionBadge) -> ChatConnection {
    ChatConnection {
        id: "c-claude".into(),
        agent_id: "claude-acp".into(),
        label: label.into(),
        agent_name: "Claude".into(),
        identity: Some("ana@example.com · Max".into()),
        last_used: "Usado hace 2 h".into(),
        badge,
    }
}

/// Makes `label` the active connection with the given badge and agent status.
fn activate(
    panel: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
    label: &str,
    badge: ConnectionBadge,
    status: AgentStatus,
) {
    panel.update(visual, |panel, cx| {
        panel.set_connections(vec![connection(label, badge)], cx);
        panel.set_active_connection(Some("c-claude".into()), cx);
        panel.set_status(status, cx);
    });
}

/// Width of `text` shaped like the panel shapes it (default font) at `size`.
fn text_width(visual: &mut VisualTestContext, text: &str, size: f32) -> Pixels {
    visual.update(|window: &mut Window, _| {
        let style = window.text_style();
        let run = gpui::TextRun {
            len: text.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(text.to_string().into(), px(size), &[run], None)
            .width
    })
}

/// The label painted under `chat-header-badge`, found by its width.
fn painted_label(visual: &mut VisualTestContext) -> &'static str {
    let badge = visual
        .debug_bounds("chat-header-badge")
        .expect("la insignia del encabezado se pinta");
    let all = [
        HeaderBadge::Connected,
        HeaderBadge::Disconnected,
        HeaderBadge::AuthRequired,
        HeaderBadge::SessionExpired,
        HeaderBadge::Unavailable {
            reason: String::new(),
        },
    ];
    let matching: Vec<&'static str> = all
        .iter()
        .map(HeaderBadge::label)
        .filter(|label| {
            let expected = text_width(visual, label, TEXT_LABEL) + px(PILL_PADDING);
            (expected - badge.size.width).abs() < px(1.)
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "el ancho de la insignia ({:?}) corresponde a una sola palabra: {matching:?}",
        badge.size.width
    );
    matching[0]
}

fn unavailable() -> ConnectionBadge {
    ConnectionBadge::Unavailable {
        reason: "Falta el adaptador de Claude.".into(),
    }
}

#[test]
fn the_labels_are_the_ones_of_the_spec() {
    assert_eq!(HeaderBadge::Connected.label(), "conectada");
    assert_eq!(HeaderBadge::Disconnected.label(), "desconectada");
    assert_eq!(HeaderBadge::AuthRequired.label(), "autenticación requerida");
    assert_eq!(HeaderBadge::SessionExpired.label(), "sesión vencida");
    assert_eq!(
        HeaderBadge::Unavailable { reason: "x".into() }.label(),
        "no disponible"
    );
}

/// §3.1: the whole table, with `header_badge()` and with the word painted.
#[gpui::test]
fn the_badge_says_the_state_of_the_connection_in_every_case_of_the_table(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    let cases = [
        (
            ConnectionBadge::SessionExpired,
            AgentStatus::Ready,
            HeaderBadge::SessionExpired,
            "sesión vencida",
        ),
        (
            unavailable(),
            AgentStatus::Ready,
            HeaderBadge::Unavailable {
                reason: "Falta el adaptador de Claude.".into(),
            },
            "no disponible",
        ),
        (
            ConnectionBadge::Connected,
            AgentStatus::AuthRequired,
            HeaderBadge::AuthRequired,
            "autenticación requerida",
        ),
        (
            ConnectionBadge::Connected,
            AgentStatus::Disconnected,
            HeaderBadge::Disconnected,
            "desconectada",
        ),
        (
            ConnectionBadge::Connected,
            AgentStatus::Ready,
            HeaderBadge::Connected,
            "conectada",
        ),
        (
            ConnectionBadge::Connected,
            AgentStatus::Thinking,
            HeaderBadge::Connected,
            "conectada",
        ),
        (
            ConnectionBadge::Connected,
            AgentStatus::WaitingPermission,
            HeaderBadge::Connected,
            "conectada",
        ),
    ];
    for (connection_badge, status, expected, word) in cases {
        activate(
            &panel,
            &mut visual,
            "Claude personal",
            connection_badge,
            status,
        );
        paint(&mut visual, 380.);
        let badge = panel.read_with(&visual, |panel, _| panel.header_badge());
        assert_eq!(badge.as_ref(), Some(&expected), "{status:?}");
        assert_eq!(expected.label(), word);
        assert_eq!(painted_label(&mut visual), word, "{status:?}");
    }
}

/// §3.1: the reason of an unavailable connection travels in the badge (it is
/// the tooltip) and the tooltip is wired in the source.
#[gpui::test]
fn the_unavailable_badge_carries_its_reason_as_a_tooltip(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    activate(
        &panel,
        &mut visual,
        "Claude personal",
        unavailable(),
        AgentStatus::Ready,
    );
    paint(&mut visual, 380.);
    let badge = panel.read_with(&visual, |panel, _| panel.header_badge());
    assert_eq!(
        badge,
        Some(HeaderBadge::Unavailable {
            reason: "Falta el adaptador de Claude.".into()
        })
    );
    let source = include_str!("render.rs");
    let start = source
        .find("fn header_badge_pill")
        .expect("falta header_badge_pill");
    let body = &source[start..];
    let body = &body[..body.find("\n}\n").expect("fin de la función")];
    assert!(
        body.contains("HeaderBadge::Unavailable { reason }")
            && body.contains(".tooltip(")
            && body.contains("reason"),
        "la insignia «no disponible» lleva el motivo en un tooltip"
    );
}

/// §3.1, R1: expired session > unavailable > auth required > disconnected >
/// connected, whatever the agent is doing.
#[gpui::test]
fn the_priority_is_expired_then_unavailable_then_auth_then_disconnected(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    let cases = [
        (
            ConnectionBadge::SessionExpired,
            AgentStatus::Thinking,
            "sesión vencida",
        ),
        (
            ConnectionBadge::SessionExpired,
            AgentStatus::AuthRequired,
            "sesión vencida",
        ),
        (
            ConnectionBadge::SessionExpired,
            AgentStatus::Disconnected,
            "sesión vencida",
        ),
        (unavailable(), AgentStatus::AuthRequired, "no disponible"),
        (unavailable(), AgentStatus::Disconnected, "no disponible"),
        (unavailable(), AgentStatus::Thinking, "no disponible"),
    ];
    for (connection_badge, status, word) in cases {
        activate(
            &panel,
            &mut visual,
            "Claude personal",
            connection_badge,
            status,
        );
        paint(&mut visual, 380.);
        let badge = panel
            .read_with(&visual, |panel, _| panel.header_badge())
            .expect("hay insignia");
        assert_eq!(badge.label(), word, "{status:?}");
        assert_eq!(painted_label(&mut visual), word, "{status:?}");
    }
    // AuthRequired beats Disconnected: it is a state of the agent, not a gap.
    activate(
        &panel,
        &mut visual,
        "Claude personal",
        ConnectionBadge::Connected,
        AgentStatus::AuthRequired,
    );
    assert_eq!(
        panel.read_with(&visual, |panel, _| panel.header_badge()),
        Some(HeaderBadge::AuthRequired)
    );
}

/// R2: during a real turn (sent message, thinking, permission) the badge keeps
/// saying "conectada", and the status of the status bar keeps its own words.
#[gpui::test]
fn the_badge_does_not_change_while_the_agent_works(cx: &mut TestAppContext) {
    let (panel, mut visual, _recorded) = open_with_session(cx);
    connect(&panel, &mut visual, false);
    paint(&mut visual, 380.);
    assert_eq!(painted_label(&mut visual), "conectada", "en reposo");

    crate::attachments_tests::with_window(&panel, &mut visual, |panel, window, cx| {
        panel.set_input_text("hola", window, cx);
        panel.send(window, cx);
    });
    visual.run_until_parked();
    assert_eq!(
        panel.read_with(&visual, |panel, _| panel.status()),
        AgentStatus::Thinking,
        "el turno está en curso"
    );
    assert_eq!(
        panel.read_with(&visual, |panel, _| panel.header_badge()),
        Some(HeaderBadge::Connected)
    );
    assert_eq!(painted_label(&mut visual), "conectada", "pensando");

    panel.update(&mut visual, |panel, cx| {
        panel.set_status(AgentStatus::WaitingPermission, cx)
    });
    visual.run_until_parked();
    assert_eq!(painted_label(&mut visual), "conectada", "esperando permiso");
    // `AgentStatus` keeps its words: the status bar uses them.
    assert_eq!(AgentStatus::Thinking.label(), "pensando…");
    assert_eq!(AgentStatus::Ready.label(), "listo");
}

#[gpui::test]
fn there_is_no_badge_without_an_active_connection(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    paint(&mut visual, 380.);
    assert_eq!(
        panel.read_with(&visual, |panel, _| panel.header_badge()),
        None
    );
    assert!(visual.debug_bounds("chat-header-badge").is_none());
    assert!(visual.debug_bounds("chat-connect").is_some());
    assert!(visual.debug_bounds("chat-connection-name").is_none());
}

/// §3.4: exactly one badge, the name complete and nothing else in the button:
/// no agent type and no "Conectada" of the connection.
#[gpui::test]
fn the_header_has_the_name_and_one_badge_and_nothing_else(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    activate(
        &panel,
        &mut visual,
        "Claude personal",
        ConnectionBadge::Connected,
        AgentStatus::Ready,
    );
    paint(&mut visual, 380.);
    assert!(visual.debug_bounds("chat-connection-type").is_none());
    assert!(visual.debug_bounds("chat-connection-badge").is_none());
    let button = visual.debug_bounds("chat-connection").expect("botón");
    let name = visual.debug_bounds("chat-connection-name").expect("nombre");
    let badge = visual.debug_bounds("chat-header-badge").expect("insignia");
    assert!(button.contains(&name.origin));
    assert!(
        !button.contains(&badge.origin),
        "la insignia va fuera del botón"
    );

    // The button is the icon, the name and the caret: its width is the width
    // of the name plus a fixed amount, with room for neither a type word nor
    // a second pill.
    let name_width = text_width(&mut visual, "Claude personal", TEXT_BODY);
    assert!(
        (name.size.width - name_width).abs() < px(1.),
        "el nombre mide lo que mide su texto: {:?} / {name_width:?}",
        name.size.width
    );
    let extra = button.size.width - name.size.width;
    let type_word = text_width(&mut visual, "Claude", TEXT_LABEL);
    assert!(
        extra < px(16. + 16. + 4. * 2. + 12.) + type_word,
        "sin tipo ni insignia dentro del botón: sobran {extra:?}"
    );
    let source = include_str!("render.rs");
    let start = source.find("fn render_header").expect("render_header");
    let header = &source[start..start + source[start..].find("fn render_banner").unwrap()];
    assert!(
        !header.contains("agent_type_label"),
        "sin tipo en el encabezado"
    );
    assert!(
        !header.contains("connection_badge("),
        "sin la insignia vieja"
    );
    assert_eq!(
        header.matches("header_badge_pill(").count(),
        1,
        "una sola insignia"
    );
}

/// §3.4: the height of the header does not change (32 px × scale).
#[gpui::test]
fn the_header_keeps_its_height(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    paint(&mut visual, 380.);
    let without = visual.debug_bounds("chat-header").expect("encabezado");
    assert_eq!(without.size.height, px(32.));
    activate(
        &panel,
        &mut visual,
        "Claude personal",
        ConnectionBadge::Connected,
        AgentStatus::Ready,
    );
    paint(&mut visual, 380.);
    let with = visual.debug_bounds("chat-header").expect("encabezado");
    assert_eq!(with.size.height, px(32.));
    activate(
        &panel,
        &mut visual,
        &"x".repeat(60),
        ConnectionBadge::SessionExpired,
        AgentStatus::Ready,
    );
    paint(&mut visual, 280.);
    let long = visual.debug_bounds("chat-header").expect("encabezado");
    assert_eq!(long.size.height, px(32.), "ni con un nombre larguísimo");
}

/// §3.4, R4: at 380 px a name of 20 characters is whole; at 280 px a name of
/// 40 is cut (the badge and the buttons keep their size and stay in view) and
/// the name carries the full text as a tooltip.
#[gpui::test]
fn the_name_is_whole_when_it_fits_and_cut_when_it_does_not(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);

    let short = "Claude de trabajo 01"; // 20 characters
    assert_eq!(short.chars().count(), 20);
    activate(
        &panel,
        &mut visual,
        short,
        ConnectionBadge::Connected,
        AgentStatus::Ready,
    );
    paint(&mut visual, 380.);
    let name = visual.debug_bounds("chat-connection-name").expect("nombre");
    let wanted = text_width(&mut visual, short, TEXT_BODY);
    assert!(
        name.size.width >= wanted - px(0.5),
        "a 380 px el nombre de 20 caracteres no se corta: {:?} < {wanted:?}",
        name.size.width
    );

    let long = "Claude de la oficina del centro, cuenta 02"; // 42 characters
    assert!(long.chars().count() >= 40);
    activate(
        &panel,
        &mut visual,
        long,
        ConnectionBadge::Connected,
        AgentStatus::Ready,
    );
    paint(&mut visual, 280.);
    let name = visual.debug_bounds("chat-connection-name").expect("nombre");
    let badge = visual.debug_bounds("chat-header-badge").expect("insignia");
    let new = visual.debug_bounds("chat-new-session").expect("nueva");
    let history = visual.debug_bounds("chat-sessions").expect("historial");
    let wanted = text_width(&mut visual, long, TEXT_BODY);
    assert!(
        name.size.width < wanted,
        "a 280 px un nombre de 40 se corta: {:?} / {wanted:?}",
        name.size.width
    );
    assert!(
        name.right() <= badge.left(),
        "el nombre no pisa la insignia"
    );
    assert!(
        badge.right() <= new.left(),
        "la insignia no pisa los botones"
    );
    assert!(history.right() <= px(280.), "los botones quedan a la vista");
    assert_eq!(new.size.width, px(22.), "los botones no se encogen");
    assert_eq!(history.size.width, px(22.));
    let badge_width = text_width(&mut visual, "conectada", TEXT_LABEL) + px(PILL_PADDING);
    assert!(
        (badge.size.width - badge_width).abs() < px(1.),
        "la insignia no se encoge"
    );

    // The tooltip carries the whole name (it is always there; harmless when
    // the name is whole).
    let source = include_str!("render.rs");
    let start = source.find("fn render_header").expect("render_header");
    let header = &source[start..start + source[start..].find("fn render_banner").unwrap()];
    let name_part = &header[header.find("chat-connection-name").expect("nombre")..];
    let name_part = &name_part[..name_part.find(".child(name)").expect("hijo")];
    for needed in [
        ".min_w(px(0.))",
        ".flex_shrink(1.)",
        ".truncate()",
        ".tooltip(",
    ] {
        assert!(name_part.contains(needed), "el nombre lleva {needed}");
    }
}
