//! The rows of the connection menu over the GPUI test harness
//! (`docs/specs/10-etapa7-ronda2.md` §4): the icon on the left, the name above
//! and the agent type below (always), the badge on the right, no account.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyWindowHandle, Bounds, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext, px, size,
};

use crate::actions::bind_default_keys;
use crate::model::*;
use crate::panel::ChatPanel;
use crate::settings::ChatSettings;
use crate::theme::ChatTheme;

/// What would show up in the rows if the account were painted.
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

fn paint(visual: &mut VisualTestContext) {
    visual.simulate_resize(size(px(420.), px(720.)));
    visual.run_until_parked();
}

/// A connection that has an email, a plan and a last-use line.
fn connection(id: &str, agent_id: &str, label: &str, badge: ConnectionBadge) -> ChatConnection {
    ChatConnection {
        id: id.into(),
        agent_id: agent_id.into(),
        label: label.into(),
        agent_name: provider_name(agent_id).to_string(),
        identity: Some(IDENTITY.into()),
        last_used: LAST_USED.into(),
        badge,
    }
}

/// The three kinds of badge, one per row, with names that are and are not the
/// agent's type.
fn three_rows() -> Vec<ChatConnection> {
    vec![
        connection(
            "c-claude",
            "claude-acp",
            "Claude",
            ConnectionBadge::Connected,
        ),
        connection(
            "c-codex",
            "codex-acp",
            "Codex · trabajo",
            ConnectionBadge::SessionExpired,
        ),
        connection(
            "c-antigravity",
            "antigravity-acp",
            "Antigravity",
            ConnectionBadge::Unavailable {
                reason: "Falta el adaptador de Antigravity.".into(),
            },
        ),
    ]
}

fn bounds(visual: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} se pinta"))
}

const ROWS: [[&str; 5]; 3] = [
    [
        "connection-row-0",
        "connection-icon-0",
        "connection-name-0",
        "connection-type-0",
        "connection-badge-0",
    ],
    [
        "connection-row-1",
        "connection-icon-1",
        "connection-name-1",
        "connection-type-1",
        "connection-badge-1",
    ],
    [
        "connection-row-2",
        "connection-icon-2",
        "connection-name-2",
        "connection-type-2",
        "connection-badge-2",
    ],
];

/// §4.4: the name above, the type below with the same left edge, the icon to
/// the left of both and centred in the row, the badge to the right.
#[gpui::test]
fn each_row_has_icon_then_name_over_type_then_badge(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(three_rows(), cx);
        panel.open_connections(cx);
    });
    paint(&mut visual);

    for [row, icon, name, kind, badge] in ROWS {
        let row = bounds(&mut visual, row);
        let icon = bounds(&mut visual, icon);
        let name = bounds(&mut visual, name);
        let kind = bounds(&mut visual, kind);
        let badge = bounds(&mut visual, badge);
        assert!(
            kind.top() > name.top() && kind.top() >= name.bottom(),
            "el tipo va debajo del nombre: {name:?} / {kind:?}"
        );
        assert_eq!(
            kind.left(),
            name.left(),
            "el nombre y el tipo empiezan en la misma x"
        );
        assert!(
            icon.right() <= name.left() && icon.right() <= kind.left(),
            "el icono está a la izquierda de las dos líneas: {icon:?}"
        );
        assert_eq!(icon.size.width, px(18.), "icono de 18 px");
        assert!(
            badge.left() >= name.right() && badge.left() >= kind.right(),
            "la insignia está a la derecha: {badge:?}"
        );
        assert!(
            row.contains(&icon.origin) && row.contains(&badge.origin),
            "todo dentro de la fila"
        );
        // The icon is centred in the row's height.
        let icon_center = icon.top() + icon.size.height / 2.;
        let row_center = row.top() + row.size.height / 2.;
        assert!(
            (icon_center - row_center).abs() < px(1.),
            "el icono va centrado en el alto de la fila: {icon:?} / {row:?}"
        );
        assert!(
            (badge.top() + badge.size.height / 2. - row_center).abs() < px(1.),
            "la insignia también"
        );
    }
}

/// R3: a connection named exactly like its type paints the type anyway.
#[gpui::test]
fn the_type_is_always_painted_even_when_the_name_is_the_type(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(three_rows(), cx);
        panel.open_connections(cx);
    });
    paint(&mut visual);
    // Row 0 is "Claude" of type Claude; row 2 is "Antigravity" of type
    // Antigravity; row 1 has another name.
    for selector in ["connection-name-0", "connection-type-0"] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    for selector in ["connection-name-2", "connection-type-2"] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    let name = bounds(&mut visual, "connection-name-0");
    let kind = bounds(&mut visual, "connection-type-0");
    assert!(
        kind.top() >= name.bottom(),
        "«Claude» arriba y «Claude» abajo"
    );

    // The type is the connection's `agent_name`, not the label with the
    // duplicate hidden.
    let source = include_str!("render.rs");
    let start = source
        .find("fn render_connection_rows")
        .expect("render_connection_rows");
    let end = start
        + source[start..]
            .find("fn render_connection_footer")
            .expect("pie");
    let rows = &source[start..end];
    assert!(rows.contains("connection.agent_name"));
    assert!(
        !rows.contains("agent_type_label") && !rows.contains("type_label()"),
        "la regla D2 de ocultar el tipo repetido ya no aplica al menú"
    );
}

/// An old saved list without `agent_name` still paints a second line.
#[gpui::test]
fn a_connection_without_agent_name_falls_back_to_the_provider(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    let mut old = connection("c", "codex-acp", "Trabajo", ConnectionBadge::Connected);
    old.agent_name = String::new();
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(vec![old], cx);
        panel.open_connections(cx);
    });
    paint(&mut visual);
    let name = bounds(&mut visual, "connection-name-0");
    let kind = bounds(&mut visual, "connection-type-0");
    assert!(kind.top() >= name.bottom());
}

/// §4.4: no email, no plan, no "Usado hace…": the rows measure the same with
/// and without an identity, however long it is, and the source of the rows
/// never reads it.
#[gpui::test]
fn the_rows_never_show_the_account(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    let plain: Vec<ChatConnection> = three_rows()
        .into_iter()
        .map(|mut connection| {
            connection.identity = None;
            connection.last_used = String::new();
            connection
        })
        .collect();
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(plain, cx);
        panel.open_connections(cx);
    });
    paint(&mut visual);
    let without: Vec<Bounds<Pixels>> = ROWS
        .iter()
        .map(|[row, ..]| bounds(&mut visual, row))
        .collect();

    let mut verbose = three_rows();
    for connection in &mut verbose {
        connection.identity = Some(format!("{IDENTITY} · Plan {}", "x".repeat(80)));
        connection.last_used = format!("{LAST_USED} {}", "y".repeat(80));
    }
    panel.update(&mut visual, |panel, cx| panel.set_connections(verbose, cx));
    paint(&mut visual);
    for (index, [row, ..]) in ROWS.iter().enumerate() {
        let with = bounds(&mut visual, row);
        assert_eq!(
            with.size, without[index].size,
            "la fila {index} mide igual con y sin correo, plan o «Usado»"
        );
    }

    let source = include_str!("render.rs");
    let start = source
        .find("fn render_connection_rows")
        .expect("render_connection_rows");
    let end = start
        + source[start..]
            .find("fn render_connection_footer")
            .expect("pie");
    for forbidden in [".identity", ".last_used", "summary()"] {
        assert!(
            !source[start..end].contains(forbidden),
            "las filas no leen {forbidden}"
        );
    }
}

/// The popover rows are all the same height (two lines each).
#[gpui::test]
fn all_the_rows_are_two_lines_tall(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(three_rows(), cx);
        panel.open_connections(cx);
    });
    paint(&mut visual);
    let heights: Vec<Pixels> = ROWS
        .iter()
        .map(|[row, ..]| bounds(&mut visual, row).size.height)
        .collect();
    assert!(
        heights.windows(2).all(|pair| pair[0] == pair[1]),
        "{heights:?}"
    );
    let name = bounds(&mut visual, "connection-name-0");
    let kind = bounds(&mut visual, "connection-type-0");
    // Padding of `py_1` (4 px) above and below, the two lines and the 2 px gap.
    let expected = px(4.) * 2. + name.size.height + px(2.) + kind.size.height;
    assert!(
        (heights[0] - expected).abs() < px(1.),
        "alto de la fila = padding + nombre + hueco + tipo: {:?} / {expected:?}",
        heights[0]
    );
}

/// A real click, on the row's second line, still selects the connection.
#[gpui::test]
fn clicking_the_row_still_selects_the_connection(cx: &mut TestAppContext) {
    let (panel, mut visual) = open(cx);
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = seen.clone();
    cx.update(|cx| {
        cx.subscribe(&panel, move |_panel, event: &crate::ChatEvent, _cx| {
            sink.borrow_mut().push(format!("{event:?}"));
        })
        .detach();
    });
    panel.update(&mut visual, |panel, cx| {
        panel.set_connections(three_rows(), cx);
        panel.open_connections(cx);
    });
    paint(&mut visual);

    let kind = bounds(&mut visual, "connection-type-1");
    visual.simulate_click(kind.center(), Modifiers::default());
    visual.run_until_parked();
    let log = seen.borrow().join("\n");
    assert!(
        log.contains("ConnectionSelected") && log.contains("c-codex"),
        "{log}"
    );
    assert_eq!(
        panel.read_with(&visual, |panel, _| panel.popover().clone()),
        crate::Popover::Closed
    );
}
