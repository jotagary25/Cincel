//! What the connection list and the header button paint
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §3, amended by
//! `docs/specs/10-etapa7-ronda2.md` §3 and §4): the rows have icon, name,
//! agent type and status badge; the header button only the name (its badge is
//! the `HeaderBadge`, see `header_badge_tests`); and nothing of the account
//! (email, plan, "Usado hace…"), which Settings → Conexiones keeps.

use gpui::{AnyWindowHandle, Entity, TestAppContext, VisualTestContext, Window, px, size};

use crate::actions::bind_default_keys;
use crate::model::*;
use crate::panel::ChatPanel;
use crate::settings::ChatSettings;
use crate::theme::ChatTheme;

/// What would show up in the list if the account were painted.
const IDENTITY: &str = "ana@example.com · Max";
/// Idem for the last-use line.
const LAST_USED: &str = "Usado hace 2 h";

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
fn paint(visual: &mut VisualTestContext) {
    visual.simulate_resize(size(px(420.), px(720.)));
    visual.run_until_parked();
}

/// A connection that has an email, a plan and a last-use line.
fn connection(id: &str, agent_id: &str, label: &str) -> ChatConnection {
    ChatConnection {
        id: id.into(),
        agent_id: agent_id.into(),
        label: label.into(),
        agent_name: provider_name(agent_id).to_string(),
        identity: Some(IDENTITY.into()),
        last_used: LAST_USED.into(),
        badge: ConnectionBadge::Connected,
    }
}

fn with_window<R>(
    panel: &Entity<ChatPanel>,
    visual: &mut VisualTestContext,
    f: impl FnOnce(&mut ChatPanel, &mut Window, &mut gpui::Context<ChatPanel>) -> R,
) -> R {
    visual.update(|window, cx| panel.update(cx, |panel, cx| f(panel, window, cx)))
}

/// The three kinds of badge the list and the button have to show.
fn badges() -> [ConnectionBadge; 3] {
    [
        ConnectionBadge::Connected,
        ConnectionBadge::SessionExpired,
        ConnectionBadge::Unavailable {
            reason: "Falta el adaptador de Codex.".into(),
        },
    ]
}

#[gpui::test]
fn the_rows_have_icon_name_type_and_badge_in_two_lines(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    let mut expired = connection("c-antigravity", "antigravity-acp", "Antigravity · personal");
    expired.badge = ConnectionBadge::SessionExpired;
    let mut broken = connection("c-codex", "codex-acp", "Codex · trabajo");
    broken.badge = badges()[2].clone();
    // A very long account line and a long last-use line: if either were still
    // painted under the name, the row would grow a third line.
    let mut verbose = connection("c-claude", "claude-acp", "Claude · personal");
    verbose.identity = Some(format!("{IDENTITY} · {}", "x".repeat(60)));
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(vec![verbose, expired, broken], cx);
        panel.open_connections(cx);
    });
    paint(&mut visual);

    for index in 0..3 {
        let row = match index {
            0 => "connection-row-0",
            1 => "connection-row-1",
            _ => "connection-row-2",
        };
        let bounds = visual
            .debug_bounds(row)
            .unwrap_or_else(|| panic!("{row} se pinta"));
        // 13 px of text at gpui's 1.618 line height plus the row's padding is
        // about 29 px; the type line (11 px) makes it about 49 px, and a third
        // line would make it about 67 px (the exact positions are checked in
        // `connection_rows_tests`).
        assert!(
            bounds.size.height > px(38.) && bounds.size.height < px(60.),
            "{row} debe ser de dos líneas: {bounds:?}"
        );
    }
    for selector in [
        "connection-type-0",
        "connection-type-1",
        "connection-type-2",
    ] {
        assert!(
            visual.debug_bounds(selector).is_some(),
            "{selector}: el tipo de agente se pinta"
        );
    }
    for selector in [
        "connection-badge-0",
        "connection-badge-1",
        "connection-badge-2",
    ] {
        let row_index = selector.trim_start_matches("connection-badge-");
        let row = match row_index {
            "0" => "connection-row-0",
            "1" => "connection-row-1",
            _ => "connection-row-2",
        };
        let badge = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector}: la insignia se pinta"));
        let row = visual.debug_bounds(row).expect("la fila se pinta");
        assert!(
            badge.left() >= row.left() && badge.right() <= row.right(),
            "la insignia cabe dentro de su fila: {badge:?} / {row:?}"
        );
    }
}

#[gpui::test]
fn the_header_button_shows_only_the_name_on_one_line(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(
            vec![connection("c-claude", "claude-acp", "Claude · personal")],
            cx,
        );
        panel.set_active_connection(Some("c-claude".into()), cx);
    });
    paint(&mut visual);
    let button = visual
        .debug_bounds("chat-connection")
        .expect("el botón del encabezado se pinta");
    let name = visual
        .debug_bounds("chat-connection-name")
        .expect("el nombre se pinta");
    let badge = visual
        .debug_bounds("chat-header-badge")
        .expect("la insignia del encabezado se pinta");
    assert!(
        button.contains(&name.origin),
        "el nombre va dentro del botón: {button:?} {name:?}"
    );
    assert!(
        !button.contains(&badge.origin) && button.right() <= badge.left() + px(1.),
        "la insignia va después del botón, no dentro: {button:?} {badge:?}"
    );
    assert!(
        visual.debug_bounds("chat-connection-type").is_none()
            && visual.debug_bounds("chat-connection-badge").is_none(),
        "ni el tipo ni la insignia de la conexión en el botón"
    );
    assert!(
        button.size.height < px(32.),
        "una sola línea dentro del encabezado de 32 px: {button:?}"
    );
}

#[gpui::test]
fn the_header_badge_follows_the_state_of_the_active_connection(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    let mut widths = Vec::new();
    for badge in badges() {
        let mut active = connection("c-codex", "codex-acp", "Codex · trabajo");
        active.badge = badge.clone();
        panel.update(&mut visual, |panel, cx| {
            panel.set_connections(vec![active], cx);
            panel.set_active_connection(Some("c-codex".into()), cx);
        });
        paint(&mut visual);
        let bounds = visual
            .debug_bounds("chat-header-badge")
            .unwrap_or_else(|| panic!("la insignia «{}» se ve en el encabezado", badge.label()));
        widths.push((badge.label(), bounds.size.width));
        let active = panel.read_with(&visual, |panel, _| panel.active_connection().cloned());
        assert_eq!(active.expect("hay conexión activa").badge, badge);
    }
    // "sesión vencida" and "no disponible" are longer words than "conectada":
    // the pill is the text, so a wider pill means the new state was painted.
    assert!(
        widths[1].1 > widths[0].1 && widths[2].1 > widths[0].1,
        "cada estado pinta su propia palabra: {widths:?}"
    );
}

#[gpui::test]
fn a_label_that_is_the_agent_type_still_shows_it_in_the_rows(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(
            vec![
                connection("c-claude", "claude-acp", "Claude"),
                connection("c-codex", "codex-acp", "CODEX"),
                connection("c-other", "codex-acp", "Codex · trabajo"),
            ],
            cx,
        );
        panel.set_active_connection(Some("c-claude".into()), cx);
        panel.open_connections(cx);
    });
    paint(&mut visual);
    assert!(
        visual.debug_bounds("chat-connection-type").is_none(),
        "el botón del encabezado nunca pinta el tipo"
    );
    // R3 (`docs/specs/10-etapa7-ronda2.md`): with two lines the rows always
    // paint the type, even when the name is that very word.
    for selector in [
        "connection-type-0",
        "connection-type-1",
        "connection-type-2",
    ] {
        assert!(
            visual.debug_bounds(selector).is_some(),
            "{selector}: la fila pinta su tipo siempre"
        );
    }
}

#[test]
fn the_type_is_hidden_only_when_the_label_is_exactly_the_type() {
    assert_eq!(visible_agent_type("Claude", "Claude"), None);
    assert_eq!(visible_agent_type("claude", "Claude"), None);
    assert_eq!(visible_agent_type(" Claude ", "Claude"), None);
    assert_eq!(
        visible_agent_type("Claude · personal", "Claude"),
        Some("Claude")
    );
    assert_eq!(visible_agent_type("Claude 2", "Claude"), Some("Claude"));
    assert_eq!(
        visible_agent_type("Trabajo", ""),
        None,
        "sin tipo no hay nada"
    );
    let connection = connection("c", "codex-acp", "Codex");
    assert_eq!(connection.type_label(), None);
}

#[test]
fn identity_and_last_use_stay_in_the_model_for_settings() {
    let connection = connection("c", "claude-acp", "Claude · personal");
    assert_eq!(connection.identity.as_deref(), Some(IDENTITY));
    assert_eq!(connection.last_used, LAST_USED);
    // A saved list from before `agent_name` existed still reads.
    let raw = r#"{"id":"c","agent_id":"claude-acp","label":"L","identity":null,
        "last_used":"Usado hace 1 h","badge":"Connected"}"#;
    let old: ChatConnection = serde_json::from_str(raw).expect("se lee");
    assert_eq!(old.agent_name, "");
}

/// The chat paints a connection only through the pieces of this list. A new
/// line that read `identity` or `last_used` in the header or in a row would
/// bring the account back; Settings → Conexiones is the only place for it.
#[test]
fn the_header_and_the_rows_never_read_the_account() {
    let source = include_str!("render.rs");
    let section = |from: &str, to: &str| -> String {
        let start = source.find(from).unwrap_or_else(|| panic!("falta {from}"));
        let end = source[start..]
            .find(to)
            .unwrap_or_else(|| panic!("falta {to}"));
        source[start..start + end].to_string()
    };
    let header = section("fn render_header", "fn render_banner");
    let rows = section("fn render_connection_rows", "fn render_connection_footer");
    for (name, body) in [("render_header", header), ("render_connection_rows", rows)] {
        for forbidden in [".identity", ".last_used", "summary()"] {
            assert!(
                !body.contains(forbidden),
                "{name} no debe pintar la cuenta ({forbidden})"
            );
        }
    }
}

#[gpui::test]
fn choosing_a_row_still_selects_that_connection(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let sink = seen.clone();
    cx.update(|cx| {
        cx.subscribe(&panel, move |_panel, event: &crate::ChatEvent, _cx| {
            sink.borrow_mut().push(format!("{event:?}"));
        })
        .detach();
    });
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(
            vec![
                connection("c-claude", "claude-acp", "Claude · personal"),
                connection("c-codex", "codex-acp", "Codex · trabajo"),
            ],
            cx,
        );
        panel.open_connections(cx);
    });
    paint(&mut visual);
    let row = visual.debug_bounds("connection-row-1").expect("fila 1");
    visual.simulate_click(row.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    let log = seen.borrow().join("\n");
    assert!(
        log.contains("ConnectionSelected") && log.contains("c-codex"),
        "{log}"
    );
    with_window(&panel, &mut visual, |panel, _, _| {
        assert_eq!(panel.popover(), &crate::Popover::Closed);
    });
}
