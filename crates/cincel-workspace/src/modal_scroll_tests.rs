//! Regression tests for the mouse wheel over a floating modal reaching the
//! editor behind it (`docs/specs/07-etapa5-productividad.md` §3, §4.4, §5,
//! §10.4): the shortcuts modal (`F1`), the quick file finder (`Ctrl+P`) and
//! the "Nuevo archivo" field (`Ctrl+N`) must all keep the wheel to
//! themselves via `Interactivity::occlude` on their floating container.
//!
//! Kept out of `tests.rs` (this fix runs alongside two other subagents
//! working on other files of the same crate): the `workspace_window` /
//! `open_pinned` pattern is rebuilt locally, same as `file_finder_tests.rs`
//! and `shortcuts_modal_tests.rs` already do, since `tests.rs`'s own
//! helpers are private to that module.

use std::path::Path;

use gpui::{
    Entity, Point, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext,
    point,
};

use crate::project::ProjectOptions;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(cincel_settings::Config::default(), cx));
}

/// A single file long enough that the editor actually has room to scroll.
fn sample_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let lines: String = (0..400).map(|row| format!("// línea {row}\n")).collect();
    std::fs::write(dir.path().join("largo.rs"), lines).unwrap();
    dir
}

fn workspace_window<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let options = WorkspaceOptions {
        project: Some(root.to_path_buf()),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    (workspace, cx)
}

/// Opens `largo.rs` pinned and returns its editor, parked with a known,
/// non-zero scroll position so a wheel event's effect (or lack of it) is
/// unambiguous.
fn open_scrolled_editor(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Entity<cincel_editor::EditorView> {
    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new("largo.rs"), true, cx)
    });
    cx.run_until_parked();
    let editor = workspace.read_with(cx, |workspace, cx| {
        workspace
            .center()
            .read(cx)
            .active_tab()
            .expect("largo.rs se abrió")
            .editor()
            .clone()
    });
    editor.update(cx, |editor, cx| editor.set_scroll_row(50., cx));
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.scroll_row()),
        50.,
        "el archivo tiene que ser lo bastante largo como para no recortar el valor"
    );
    editor
}

/// A wheel event over `selector`'s current bounds, big enough to move the
/// editor's scroll row if it reached it.
fn scroll_over(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("«{selector}» no se pintó"));
    let position: Point<gpui::Pixels> = point(
        bounds.origin.x + bounds.size.width / 2.,
        bounds.origin.y + bounds.size.height / 2.,
    );
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Lines(point(0., -10.)),
        modifiers: gpui::Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
}

/// §5 (`F1`): girar la rueda sobre el modal de atajos no debe mover el
/// archivo abierto detrás.
#[gpui::test]
fn wheel_over_shortcuts_modal_does_not_scroll_the_editor_behind(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let editor = open_scrolled_editor(&workspace, cx);

    cx.simulate_keystrokes("f1");
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |workspace, app| {
        workspace.shortcuts_modal().read(app).is_open()
    }));

    scroll_over("shortcuts-dialog", cx);

    assert_eq!(
        editor.read_with(cx, |editor, _| editor.scroll_row()),
        50.,
        "la rueda sobre el modal de atajos no debe mover el editor de fondo"
    );
}

/// §3 (`Ctrl+P`): lo mismo con el buscador rápido de archivos.
#[gpui::test]
fn wheel_over_file_finder_does_not_scroll_the_editor_behind(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let editor = open_scrolled_editor(&workspace, cx);

    cx.simulate_keystrokes("ctrl-p");
    cx.run_until_parked();
    let finder = workspace
        .read_with(cx, |workspace, _| workspace.file_finder().cloned())
        .expect("el proyecto ya construyó el buscador");
    assert!(finder.read_with(cx, |finder, _| finder.is_open()));

    scroll_over("file-finder", cx);

    assert_eq!(
        editor.read_with(cx, |editor, _| editor.scroll_row()),
        50.,
        "la rueda sobre el buscador no debe mover el editor de fondo"
    );
}

/// §4.4 (`Ctrl+N`): lo mismo con el campo "Nuevo archivo".
#[gpui::test]
fn wheel_over_new_file_prompt_does_not_scroll_the_editor_behind(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let editor = open_scrolled_editor(&workspace, cx);

    cx.simulate_keystrokes("ctrl-n");
    cx.run_until_parked();
    let prompt = workspace
        .read_with(cx, |workspace, _| workspace.new_file_prompt().cloned())
        .expect("el proyecto ya construyó el campo");
    assert!(prompt.read_with(cx, |prompt, _| prompt.is_open()));

    scroll_over("new-file-prompt", cx);

    assert_eq!(
        editor.read_with(cx, |editor, _| editor.scroll_row()),
        50.,
        "la rueda sobre «Nuevo archivo» no debe mover el editor de fondo"
    );
}
