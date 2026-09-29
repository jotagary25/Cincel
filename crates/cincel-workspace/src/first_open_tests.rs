//! The first time a Markdown file opens in a fresh window it paints its text
//! (the "README.md en blanco" report of Etapa 5): opened from the tree, and
//! restored from the layout with the cursor far along a long line.

use std::path::Path;
use std::process::Command;

use cincel_editor::{EditorView, FrameRender, RowKind};
use cincel_settings::Config;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::project::ProjectOptions;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// A Markdown file with long lines, headings, lists and inline code.
fn readme() -> String {
    let long = "Editor de código minimalista, escrito en Rust con GPUI, con chat integrado \
                para agentes de programación usando tus propias suscripciones y revisión \
                de cada cambio dentro del editor, segmento por segmento.";
    let mut text = String::from("# Cincel\n\n");
    for section in 0..2 {
        text.push_str(&format!("{long}\n\n## Sección {section}\n"));
        text.push_str("- Buscador rápido de archivos (`Ctrl+P`).\n- Modal de atajos (`F1`).\n\n");
    }
    text
}

/// A repository whose `README.md` differs from `HEAD` (so the git gutter has
/// something to say while the tab opens), plus another file.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), "# viejo\n").unwrap();
    std::fs::write(dir.path().join("LICENSE"), "MIT\n").unwrap();
    let committed = git(dir.path(), &["init", "-q"])
        && git(dir.path(), &["config", "user.email", "test@cincel"])
        && git(dir.path(), &["config", "user.name", "Test"])
        && git(dir.path(), &["add", "."])
        && git(dir.path(), &["commit", "-qm", "inicial"]);
    if !committed {
        eprintln!("sin git utilizable: se prueba sin el margen de git");
    }
    std::fs::write(dir.path().join("README.md"), readme()).unwrap();
    dir
}

/// A workspace window on `root`, sized like a real one, with the git gutter.
fn workspace_window<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let options = WorkspaceOptions {
        project: Some(root.to_path_buf()),
        project_options: ProjectOptions {
            git_gutter: true,
            ..ProjectOptions::inert()
        },
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(800.)));
    cx.run_until_parked();
    (workspace, cx)
}

fn open_from_tree(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<EditorView> {
    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new("README.md"), false, cx)
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

fn last_frame(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> FrameRender {
    editor.update(cx, |editor, _| editor.set_render_probe(true));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    editor
        .read_with(cx, |editor, _| editor.render_frames().last().cloned())
        .expect("un cuadro pintado")
}

/// The frame paints the buffer from its first line: rows with line numbers,
/// at the buffer's version, inside a real width, not scrolled away.
fn assert_paints_the_text(editor: &Entity<EditorView>, cx: &mut VisualTestContext) {
    let frame = last_frame(editor, cx);
    let (version, line_count, text) = editor.read_with(cx, |editor, _| {
        let snapshot = editor.buffer().lock().snapshot();
        (snapshot.version(), snapshot.line_count(), editor.text())
    });
    assert_eq!(text, readme(), "the tab holds the file");
    assert_eq!(
        frame.text_version, version,
        "the frame is of the loaded text"
    );
    assert!(!frame.placeholder, "not the empty-buffer placeholder");
    assert!(
        frame.bounds.size.width > gpui::px(200.),
        "{:?}",
        frame.bounds
    );
    let numbered: Vec<u32> = frame
        .rows
        .iter()
        .filter(|row| row.line_number && row.kind == RowKind::Normal)
        .map(|row| row.display_row)
        .collect();
    assert_eq!(
        numbered,
        (0..line_count).collect::<Vec<_>>(),
        "every line painted, from the first one"
    );
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.scroll_row()),
        0.,
        "the whole file fits: nothing to scroll"
    );
}

#[gpui::test]
fn a_markdown_file_opened_from_the_tree_paints_its_text_the_first_time(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let editor = open_from_tree(&workspace, cx);
    let language = editor.read_with(cx, |editor, _| editor.language().map(|l| l.name()));
    assert_eq!(language, Some("markdown"));
    assert_paints_the_text(&editor, cx);
}

#[gpui::test]
fn a_restored_markdown_tab_paints_its_text_the_first_time(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = project();
    // The layout the report left behind: the README active, the cursor on
    // column 154 of a long line, and another tab.
    let root = std::path::absolute(dir.path()).unwrap();
    let layout = crate::layout::WorkspaceLayout::path_for(&root).unwrap();
    std::fs::create_dir_all(layout.parent().unwrap()).unwrap();
    std::fs::write(
        &layout,
        r#"{"version":1,"chat":{"size":493.0,"open":true},"tree":{"size":240.0,"open":true},
"tabs":[{"path":"README.md","preview":false,"scroll":0.0,"cursor":[2,154],"markdown_preview":false},
{"path":"LICENSE","preview":false,"scroll":0.0,"cursor":[0,0],"markdown_preview":false}],"active":0}"#,
    )
    .unwrap();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    // The tree click lands on the tab that is already open.
    let editor = open_from_tree(&workspace, cx);
    assert_paints_the_text(&editor, cx);
}
