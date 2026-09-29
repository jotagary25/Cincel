//! Tests of the keyboard shortcuts modal (`crate::shortcuts_modal`,
//! `docs/specs/07-etapa5-productividad.md` §5.3/§5.4).
//!
//! Kept out of `tests.rs` (E5-H only touches new files while other
//! sub-stages work on `cincel-workspace` in parallel): the pure half needs no
//! `App` at all, and the `TestAppContext` half follows the same
//! `workspace_window` / `press` pattern the rest of the crate's test modules
//! use, rebuilt locally since `tests.rs`'s own helpers are private to that
//! module.

use std::path::Path;

use cincel_settings::{Config, Keystroke, Paths};
use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};

use crate::project::ProjectOptions;
use crate::shortcuts_modal::{
    ShortcutCategory, ShortcutsModal, all_default_commands, category, description, format_keystroke,
};
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

// ------------------------------------------------------------- pure tests

#[test]
fn every_default_command_has_a_spanish_description() {
    // §5.3: "Todos los comandos del keymap por defecto y de los widgets
    // tienen descripción en español."
    for command in all_default_commands() {
        assert!(
            description(&command).is_some(),
            "falta descripción para «{command}»"
        );
    }
}

#[test]
fn format_keystroke_matches_the_spanish_table() {
    let cases = [
        ("ctrl-shift-a", "Ctrl+Shift+A"),
        ("ctrl-p", "Ctrl+P"),
        ("enter", "Enter"),
        ("escape", "Esc"),
        ("backspace", "Backspace"),
        ("delete", "Supr"),
        ("up", "↑"),
        ("down", "↓"),
        ("left", "←"),
        ("right", "→"),
        ("pageup", "RePág"),
        ("pagedown", "AvPág"),
        ("f1", "F1"),
        ("f7", "F7"),
    ];
    for (source, expected) in cases {
        let keystroke = Keystroke::parse(source).expect(source);
        assert_eq!(format_keystroke(&keystroke), expected, "{source}");
    }
}

#[test]
fn category_follows_the_explicit_table() {
    assert_eq!(category("review::accept_hunk"), ShortcutCategory::Review);
    assert_eq!(category("workspace::next_change"), ShortcutCategory::Review);
    assert_eq!(
        category("workspace::open_review_panel"),
        ShortcutCategory::Review
    );
    assert_eq!(category("chat::send"), ShortcutCategory::Chat);
    assert_eq!(category("workspace::toggle_chat"), ShortcutCategory::Chat);
    assert_eq!(
        category("workspace::mention_in_chat"),
        ShortcutCategory::Chat
    );
    assert_eq!(
        category("workspace::open_connections"),
        ShortcutCategory::Connections
    );
    assert_eq!(category("editor::save"), ShortcutCategory::Editor);
    assert_eq!(category("editor::save_all"), ShortcutCategory::Editor);
    assert_eq!(
        category("workspace::open_folder"),
        ShortcutCategory::General
    );
    assert_eq!(category("file_finder::confirm"), ShortcutCategory::General);
    assert_eq!(category("shortcuts::dismiss"), ShortcutCategory::General);
}

// ------------------------------------------------------------- GPUI tests

/// Boots a fresh configuration directory, with `keymap` written as the
/// user's `keymap.json` when given.
fn init_test(keymap: Option<&str>, cx: &mut TestAppContext) -> (tempfile::TempDir, Paths) {
    isolate_state();
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path().join("config"));
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    if let Some(text) = keymap {
        std::fs::write(&paths.keymap, text).unwrap();
    }
    let config = Config::load_from(&paths).value;
    cx.update(|cx| crate::init(config, cx));
    (dir, paths)
}

/// A tiny project, for the tests that need the chat panel to actually take
/// the focus (`docs/specs/07-etapa5-productividad.md` §7.1: without a
/// project, `Ctrl+Shift+A` only remembers the dock's visibility).
fn sample_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
    dir
}

fn workspace_window<'a>(
    root: Option<&Path>,
    cx: &'a mut TestAppContext,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let options = WorkspaceOptions {
        project: root.map(Path::to_path_buf),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    (workspace, cx)
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
}

fn modal(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<ShortcutsModal> {
    workspace.read_with(cx, |workspace, _| workspace.shortcuts_modal().clone())
}

/// §5.3: "`F1` y el botón de la barra de estado abren el modal con el foco
/// en el buscador." (the button calls the exact same
/// `Workspace::show_shortcuts` this exercises through the key binding).
#[gpui::test]
fn f1_opens_with_the_focus_in_the_search_field(cx: &mut TestAppContext) {
    init_test(None, cx);
    let (workspace, cx) = workspace_window(None, cx);

    press("f1", cx);

    let modal = modal(&workspace, cx);
    assert!(modal.read_with(cx, |modal, _| modal.is_open()));
    let query_focused = cx.update(|window, cx| {
        modal
            .read(cx)
            .query()
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
    });
    assert!(query_focused, "F1 tiene que enfocar el buscador");

    // `F1` again closes it, like the other floating panels.
    press("f1", cx);
    assert!(!modal.read_with(cx, |modal, _| modal.is_open()));
}

/// §5.3: "Con `keymap.json` = `[{"bindings":{"ctrl-shift-a":
/// "workspace::toggle_tree"}}]`, la fila de `Ctrl+Shift+A` dice
/// 'Personalizado' y 'reemplaza a «Mostrar u ocultar el chat»'."
#[gpui::test]
fn a_user_override_shows_the_custom_badge_and_what_it_replaces(cx: &mut TestAppContext) {
    init_test(
        Some(r#"[{"bindings":{"ctrl-shift-a":"workspace::toggle_tree"}}]"#),
        cx,
    );
    let (workspace, cx) = workspace_window(None, cx);

    press("f1", cx);
    let modal = modal(&workspace, cx);
    let rows = modal.read_with(cx, |modal, cx| modal.visible_rows(cx));
    let row = rows
        .iter()
        .find(|row| row.identity == "workspace::toggle_tree")
        .expect("workspace::toggle_tree aparece en la lista");

    assert!(row.custom, "{row:?}");
    assert!(row.keys.iter().any(|key| key == "Ctrl+Shift+A"), "{row:?}");
    assert!(
        row.notes
            .iter()
            .any(|note| note == "reemplaza a «Mostrar u ocultar el chat»"),
        "{row:?}"
    );
}

/// §5.3: "Con `{"context":"Editor","bindings":{"ctrl-g":null}}`, 'Ir a la
/// línea' aparece atenuada con 'Desactivado en tu keymap.json'."
#[gpui::test]
fn a_null_override_shows_the_disabled_note(cx: &mut TestAppContext) {
    init_test(
        Some(r#"[{"context":"Editor","bindings":{"ctrl-g":null}}]"#),
        cx,
    );
    let (workspace, cx) = workspace_window(None, cx);

    press("f1", cx);
    let modal = modal(&workspace, cx);
    let rows = modal.read_with(cx, |modal, cx| modal.visible_rows(cx));
    let row = rows
        .iter()
        .find(|row| row.identity == "editor::go_to_line")
        .expect("editor::go_to_line aparece en la lista, desactivado");

    assert!(row.keys.is_empty(), "{row:?}");
    assert_eq!(row.disabled_keys, vec!["Ctrl+G".to_string()]);
    assert_eq!(
        row.disabled_note.as_deref(),
        Some("Desactivado en tu keymap.json")
    );
}

/// §5.3: "Buscar 'ctrl+p' deja solo 'Buscar archivos'."
#[gpui::test]
fn searching_ctrl_plus_p_leaves_only_the_file_finder_row(cx: &mut TestAppContext) {
    init_test(None, cx);
    let (workspace, cx) = workspace_window(None, cx);

    press("f1", cx);
    cx.simulate_input("ctrl+p");
    cx.run_until_parked();

    let modal = modal(&workspace, cx);
    let rows = modal.read_with(cx, |modal, cx| modal.visible_rows(cx));
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].description, "Buscar archivos");

    // The dash spelling works too (§5.1: "escribir «ctrl+p» o «ctrl-p»"):
    // closing and reopening re-selects the field's text (like the file
    // finder), so the next typed text replaces it.
    press("f1", cx);
    press("f1", cx);
    cx.simulate_input("ctrl-p");
    cx.run_until_parked();
    let rows_dash = modal.read_with(cx, |modal, cx| modal.visible_rows(cx));
    assert_eq!(rows_dash.len(), 1, "{rows_dash:?}");
    assert_eq!(rows_dash[0].description, "Buscar archivos");
}

/// §9.3-style check (like `file_finder_tests`'s equivalent): with the modal
/// open, neither `Ctrl+L` nor `Ctrl+Shift+A` do anything (§7.4, "modal
/// abierto").
#[gpui::test]
fn ctrl_l_and_ctrl_shift_a_do_nothing_with_the_shortcuts_modal_open(cx: &mut TestAppContext) {
    init_test(None, cx);
    let (workspace, cx) = workspace_window(None, cx);

    press("f1", cx);
    let modal = modal(&workspace, cx);
    assert!(modal.read_with(cx, |modal, _| modal.is_open()));
    assert!(workspace.read_with(cx, |workspace, cx| workspace.is_modal_open(cx)));

    let chat_dock_open_before = workspace.read_with(cx, |workspace, cx| {
        workspace.is_dock_open(gpui_kit::component::dock::DockPlacement::Left, cx)
    });

    press("ctrl-l", cx);
    assert!(
        modal.read_with(cx, |modal, _| modal.is_open()),
        "Ctrl+L no tiene que mover el foco con el modal abierto"
    );

    press("ctrl-shift-a", cx);
    assert!(
        modal.read_with(cx, |modal, _| modal.is_open()),
        "Ctrl+Shift+A no tiene que hacer nada con el modal abierto"
    );
    let chat_dock_open_after = workspace.read_with(cx, |workspace, cx| {
        workspace.is_dock_open(gpui_kit::component::dock::DockPlacement::Left, cx)
    });
    assert_eq!(chat_dock_open_before, chat_dock_open_after);
}

/// §5.1: "`Esc` ... cierra y devuelve el foco" — the same rule the file
/// finder follows.
#[gpui::test]
fn escape_restores_the_previous_focus(cx: &mut TestAppContext) {
    init_test(None, cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(Some(dir.path()), cx);

    // Focus the chat's composer first (needs a project, §7.1), then open the
    // modal from there.
    press("ctrl-shift-a", cx);
    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    let composer_focused_before = cx.update(|window, cx| {
        chat.read(cx)
            .composer()
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(composer_focused_before, "el compositor tenía el foco");

    press("f1", cx);
    let modal = modal(&workspace, cx);
    assert!(modal.read_with(cx, |modal, _| modal.is_open()));

    press("escape", cx);

    assert!(!modal.read_with(cx, |modal, _| modal.is_open()));
    let composer_focused_after = cx.update(|window, cx| {
        chat.read(cx)
            .composer()
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(
        composer_focused_after,
        "Esc tiene que devolver el foco al chat"
    );
}

/// §5.1: "Recargar `keymap.json` con el modal abierto lo actualiza."
/// [`ShortcutsModal::visible_rows`] recomputes straight from
/// `crate::settings::keymap` on every call, so this checks that a reload
/// changes what it returns without needing to force a repaint.
#[gpui::test]
fn reloading_the_keymap_refreshes_the_open_modal(cx: &mut TestAppContext) {
    let (_dir, paths) = init_test(None, cx);
    let (workspace, cx) = workspace_window(None, cx);

    press("f1", cx);
    let modal = modal(&workspace, cx);
    let had_custom_before = modal.read_with(cx, |modal, cx| {
        modal.visible_rows(cx).iter().any(|row| row.custom)
    });
    assert!(!had_custom_before);

    std::fs::write(
        &paths.keymap,
        r#"[{"bindings":{"ctrl-shift-a":"workspace::toggle_tree"}}]"#,
    )
    .unwrap();
    cx.update(|_, cx| {
        crate::settings::reload(cincel_settings::SettingsEvent::KeymapChanged, cx);
    });
    cx.run_until_parked();

    let has_custom_after = modal.read_with(cx, |modal, cx| {
        modal.visible_rows(cx).iter().any(|row| row.custom)
    });
    assert!(
        has_custom_after,
        "el modal tiene que reflejar el keymap recargado"
    );
}
