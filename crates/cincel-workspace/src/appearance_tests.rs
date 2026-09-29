//! The desktop's appearance can arrive late: Wayland reports "light" for the
//! first frames and "dark" a moment later. With `theme.mode = "system"` the
//! editors restored at startup must follow that change, or their text keeps
//! the light theme's colors on the dark background and the file looks blank
//! (the author's "README en blanco" report of 2026-09-28).

use std::path::Path;

use cincel_settings::Config;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::project::ProjectOptions;
use crate::settings::AppSettings;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
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
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(800.)));
    cx.run_until_parked();
    (workspace, cx)
}

fn open_readme(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Entity<cincel_editor::EditorView> {
    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new("README.md"), false, cx);
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        let center = workspace.center().read(cx);
        center
            .active_tab()
            .expect("hay pestaña")
            .content
            .editor()
            .clone()
    })
}

#[gpui::test]
fn open_editors_follow_a_late_change_of_the_desktop_appearance(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), "# Cincel\n\nHola.\n").unwrap();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let editor = open_readme(&workspace, cx);

    let dark_before = cx.read(|cx| cx.global::<AppSettings>().system_dark);
    let text_before = editor.read_with(cx, |editor, _| editor.theme().text);

    // The desktop answers late with the other appearance.
    let changed = workspace.update(cx, |workspace, cx| {
        workspace.follow_system_appearance(!dark_before, cx)
    });
    cx.run_until_parked();
    assert!(changed, "la apariencia cambió");

    let in_force = cx.read(crate::theme::editor_theme);
    let (text, background) = editor.read_with(cx, |editor, _| {
        (editor.theme().text, editor.theme().background)
    });
    assert_eq!(
        text, in_force.text,
        "el texto del editor sigue el tema en vigor"
    );
    assert_eq!(
        background, in_force.background,
        "el fondo del editor sigue el tema en vigor"
    );
    assert_ne!(
        text, text_before,
        "el color del texto cambió con la apariencia"
    );

    // The same appearance again changes nothing.
    let changed = workspace.update(cx, |workspace, cx| {
        workspace.follow_system_appearance(!dark_before, cx)
    });
    assert!(!changed);
}
