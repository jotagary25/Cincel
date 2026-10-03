//! The connection list the chat paints is "name, type, badge"
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §3, D1): the
//! workspace fills `ChatConnection::agent_name`, the "Eliminar conexión" list
//! shows the type and no account, and Settings → Conexiones keeps showing the
//! account.

use std::sync::Arc;

use cincel_chat::ChatConnection;
use cincel_connections::{Connections, NodeVersion, Runtime};
use cincel_settings::Config;
use gpui::{Entity, TestAppContext, VisualTestContext, px, size};

use crate::connection_modal::{ConnectionsModal, agent_type_name};
use crate::project::ProjectOptions;
use crate::settings_view::{SettingsSection, SettingsView};
use crate::test_support::{FAKE_EMAIL, FakeEnv, isolate_state};
use crate::workspace::{Workspace, WorkspaceOptions};

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

/// The engine over the fake environment with a pinned Node, so nothing ever
/// asks nodejs.org.
fn offline_engine(env: &FakeEnv) -> Arc<Connections> {
    let runtime = Runtime::new(env.paths.clone()).with_version(NodeVersion::parse("24.1.0"));
    Arc::new(Connections::new(env.paths.clone()).with_runtime(runtime))
}

/// One saved connection per agent kind: two of them named exactly like their
/// type (so the type is not repeated beside them), one with a longer name.
fn three_connections(env: &FakeEnv) -> [(&'static str, &'static str, &'static str); 3] {
    let rows = [
        ("claude-acp", "Claude · personal", "Claude"),
        ("codex-acp", "codex", "Codex"),
        ("antigravity-acp", "Antigravity", "Antigravity"),
    ];
    for (agent_id, label, _) in rows {
        env.add_connection(agent_id, label);
    }
    rows
}

fn workspace_window(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let options = WorkspaceOptions {
        project: None,
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    (workspace, cx)
}

/// The rows the chat holds after `refresh_connections`, by label.
fn chat_rows(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<ChatConnection> {
    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    chat.read_with(cx, |chat, _| chat.connections().to_vec())
}

#[gpui::test]
fn refresh_connections_fills_the_agent_name_for_the_three_kinds(cx: &mut TestAppContext) {
    init_test(cx);
    let env = FakeEnv::new();
    let rows = three_connections(&env);
    let (workspace, cx) = workspace_window(cx);
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    agents.update(cx, |agents, cx| {
        agents.set_connections(offline_engine(&env), cx)
    });
    cx.run_until_parked();

    let listed = chat_rows(&workspace, cx);
    assert_eq!(listed.len(), 3);
    for (agent_id, label, agent_name) in rows {
        let row = listed
            .iter()
            .find(|row| row.agent_id == agent_id)
            .unwrap_or_else(|| panic!("falta la fila de {agent_id}"));
        assert_eq!(row.label, label);
        assert_eq!(row.agent_name, agent_name, "{agent_id}");
        // The data the chat no longer paints is still there for Settings.
        assert_eq!(
            row.identity.as_deref(),
            Some("ana@example.com · max"),
            "{agent_id}"
        );
        assert!(row.last_used.starts_with("Usado"), "{}", row.last_used);
    }
}

#[test]
fn an_agent_that_is_not_one_of_the_three_keeps_its_registry_id() {
    assert_eq!(agent_type_name("claude-acp"), "Claude");
    assert_eq!(agent_type_name("codex-acp"), "Codex");
    assert_eq!(agent_type_name("antigravity-acp"), "Antigravity");
    assert_eq!(agent_type_name("gemini"), "gemini");
}

#[gpui::test]
fn the_delete_list_shows_icon_name_and_type_but_no_account(cx: &mut TestAppContext) {
    init_test(cx);
    let env = FakeEnv::new();
    let rows = three_connections(&env);
    let engine = offline_engine(&env);
    let (modal, cx) =
        cx.add_window_view(|window, cx| ConnectionsModal::new(engine.clone(), true, window, cx));
    modal.update_in(cx, |modal, window, cx| modal.open_delete(window, cx));
    cx.simulate_resize(size(px(900.), px(700.)));
    cx.run_until_parked();

    // The store lists most recently used first; find each row by its type
    // selector rather than assuming an order.
    let mut with_type = 0;
    for index in 0..rows.len() {
        let (row, agent_type) = match index {
            0 => ("delete-row-0", "delete-row-type-0"),
            1 => ("delete-row-1", "delete-row-type-1"),
            _ => ("delete-row-2", "delete-row-type-2"),
        };
        let bounds = cx
            .debug_bounds(row)
            .unwrap_or_else(|| panic!("{row} se pinta"));
        if let Some(type_bounds) = cx.debug_bounds(agent_type) {
            with_type += 1;
            assert!(
                bounds.contains(&type_bounds.origin),
                "el tipo va dentro de su fila: {bounds:?} {type_bounds:?}"
            );
        }
    }
    assert_eq!(
        with_type, 1,
        "solo «Claude · personal» muestra el tipo; «codex» y «Antigravity» ya lo dicen"
    );
}

/// The delete list paints a connection through its label and its type only.
/// Reading the identity there would bring the account back into a list;
/// Settings → Conexiones is its only place.
#[test]
fn the_delete_list_never_reads_the_account() {
    let source = include_str!("connection_modal.rs");
    let start = source
        .find("DeleteStep::List => {")
        .expect("la lista de borrado");
    let end = source[start..]
        .find("DeleteStep::Confirm {")
        .expect("el paso siguiente");
    let list = &source[start..start + end];
    for forbidden in ["row.identity", "summary()", "last_used"] {
        assert!(
            !list.contains(forbidden),
            "la lista de borrado no pinta la cuenta ({forbidden})"
        );
    }
}

#[gpui::test]
fn settings_connections_still_shows_the_account(cx: &mut TestAppContext) {
    init_test(cx);
    let env = FakeEnv::new();
    three_connections(&env);
    let (workspace, cx) = workspace_window(cx);
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    agents.update(cx, |agents, cx| {
        agents.set_connections(offline_engine(&env), cx)
    });
    cx.update(|window, cx| window.dispatch_action(Box::new(crate::actions::OpenConnections), cx));
    cx.run_until_parked();

    let view: Entity<SettingsView> = workspace
        .read_with(cx, |workspace, cx| {
            workspace.center().read(cx).settings_view().cloned()
        })
        .expect("la pestaña de configuración está abierta");
    assert_eq!(
        view.read_with(cx, |view, _| view.section()),
        SettingsSection::Connections
    );
    let info = view.read_with(cx, |view, _| view.connections_info().clone());
    assert_eq!(info.connections.len(), 3);
    for connection in &info.connections {
        let identity = connection
            .identity
            .as_deref()
            .expect("Configuración conserva la identidad");
        assert!(identity.contains(FAKE_EMAIL), "{identity}");
        assert!(connection.last_used.starts_with("Usado"));
    }
    let texts = view.read_with(cx, |view, cx| view.visible_texts(cx));
    assert!(
        texts.iter().any(|text| text.starts_with("Usado")),
        "«Usado hace…» se ve en Configuración: {texts:?}"
    );
}
