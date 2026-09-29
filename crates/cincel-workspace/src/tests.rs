//! GPUI tests for the workspace.
//!
//! They need gpui's test harness, which also turns on its leak detector, so
//! the whole module lives behind the `test-support` feature (see
//! `docs/etapas/etapa-0.md`, finding 7):
//!
//! ```text
//! cargo test -p cincel-workspace --features test-support
//! ```

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use cincel_settings::Config;
use gpui::{
    AppContext as _, Context, Entity, Render, Subscription, TestAppContext, VisualTestContext,
    Window,
};
use gpui_kit::component::dock::DockPlacement;
use gpui_kit::div;
use gpui_kit::prelude::*;

use crate::center::TabContent;
use crate::focus::FocusZone;
use crate::project::{Project, ProjectOptions};
use crate::toast::{ToastKind, Toasts};
use crate::tree_panel::FilesPanel;
use crate::workspace::{Workspace, WorkspaceEvent, WorkspaceOptions};

use crate::test_support::isolate_state;

/// Boots gpui-kit and the workspace with the default configuration.
fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

/// A small project: `src/main.rs`, `src/lib.rs` and `Cargo.toml`.
fn sample_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    let x = 1;\n}\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn suma() {}\n").unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .unwrap();
    dir
}

/// Collects the [`WorkspaceEvent`]s an entity emits.
struct EventCollector {
    events: Rc<RefCell<Vec<WorkspaceEvent>>>,
    _subscription: Subscription,
}

impl EventCollector {
    fn new(panel: &Entity<FilesPanel>, cx: &mut Context<Self>) -> Self {
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let subscription = cx.subscribe(panel, move |_, _, event: &WorkspaceEvent, _| {
            captured.borrow_mut().push(event.clone());
        });
        Self {
            events,
            _subscription: subscription,
        }
    }
}

impl Render for EventCollector {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Opens `root` as a project entity outside any window.
///
/// [`ProjectOptions::inert`] is what makes the tests deterministic: GPUI's
/// test scheduler panics if a foreign thread wakes a foreground task, and the
/// directory walk, the file watcher and `git status` each run on one.
fn open_project(root: &Path, cx: &mut TestAppContext) -> Entity<Project> {
    cx.update(|cx| {
        let settings = crate::settings::settings(cx);
        crate::project::open_with(root, &settings, ProjectOptions::inert(), cx)
            .expect("el proyecto se abre")
    })
}

#[gpui::test]
fn opening_a_project_shows_the_root_entries(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let project = open_project(dir.path(), cx);
    let panel = cx.update(FilesPanel::new);

    panel.update(cx, |panel, cx| panel.set_project(Some(project.clone()), cx));
    cx.run_until_parked();

    let labels = panel.read_with(cx, |panel, cx| panel.visible_labels(cx));
    // Folders first, then files, and nothing from inside the collapsed folder.
    assert_eq!(labels, vec!["src", "Cargo.toml"], "{labels:?}");

    // The walk is complete: the entries of `src` are there, they are just not
    // visible until the folder is expanded.
    let entries = project.read_with(cx, |project, _| project.worktree().len());
    assert_eq!(entries, 4, "src, src/main.rs, src/lib.rs y Cargo.toml");
    let labels = panel.read_with(cx, |panel, cx| panel.visible_labels(cx));
    assert_eq!(labels, vec!["src", "Cargo.toml"], "{labels:?}");
}

#[gpui::test]
fn a_tree_click_emits_open_file(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let project = open_project(dir.path(), cx);
    let panel = cx.update(FilesPanel::new);
    panel.update(cx, |panel, cx| panel.set_project(Some(project), cx));
    cx.run_until_parked();

    let collector = cx.update(|cx| cx.new(|cx| EventCollector::new(&panel, cx)));

    // A single click previews, a double click pins.
    panel.update(cx, |panel, cx| {
        panel.open_for_test(Path::new("Cargo.toml"), false, cx);
        panel.open_for_test(Path::new("Cargo.toml"), true, cx);
    });
    cx.run_until_parked();

    let events = collector.read_with(cx, |collector, _| collector.events.borrow().clone());
    assert_eq!(events.len(), 2, "{events:?}");
    let expected = dir.path().join("Cargo.toml");
    assert_eq!(
        events[0],
        WorkspaceEvent::OpenFile {
            path: expected.clone(),
            pin: false
        }
    );
    assert_eq!(
        events[1],
        WorkspaceEvent::OpenFile {
            path: expected,
            pin: true
        }
    );
}

/// Builds a window with the workspace in it, on `root`.
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

#[gpui::test]
fn opening_a_file_from_the_tree_opens_a_tab(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new("src/main.rs"), true, cx)
    });
    cx.run_until_parked();

    let (count, title, breadcrumb) = workspace.read_with(cx, |workspace, cx| {
        let center = workspace.center().read(cx);
        (
            center.tabs().len(),
            center.active_tab().map(|tab| tab.title.to_string()),
            center.active_tab().map(|tab| tab.relative.clone()),
        )
    });
    assert_eq!(count, 1);
    assert_eq!(title.as_deref(), Some("main.rs"));
    assert_eq!(breadcrumb, Some(PathBuf::from("src/main.rs")));

    // The body of the tab is the code editor, over the file's own buffer.
    let text = workspace.read_with(cx, |workspace, cx| {
        let center = workspace.center().read(cx);
        let tab = center.active_tab().expect("hay pestaña");
        let editor = match &tab.content {
            TabContent::Editor(editor) | TabContent::MarkdownPreview(editor, _) => editor,
        };
        editor.read(cx).text()
    });
    assert_eq!(text, "fn main() {\n    let x = 1;\n}\n");

    // A preview of another file replaces the preview, not the pinned tab.
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new("src/lib.rs"), false, cx)
    });
    cx.run_until_parked();
    let (count, previews) = workspace.read_with(cx, |workspace, cx| {
        let center = workspace.center().read(cx);
        (
            center.tabs().len(),
            center.tabs().iter().filter(|tab| tab.preview).count(),
        )
    });
    assert_eq!(count, 2);
    assert_eq!(previews, 1);
}

#[gpui::test]
fn tabs_can_be_reordered(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    for path in ["src/main.rs", "src/lib.rs", "Cargo.toml"] {
        files.update(cx, |files, cx| {
            files.open_for_test(Path::new(path), true, cx)
        });
    }
    cx.run_until_parked();

    let center = workspace.read_with(cx, |workspace, _| workspace.center().clone());
    let titles = |cx: &mut VisualTestContext| {
        center.read_with(cx, |center, _| {
            center
                .tabs()
                .iter()
                .map(|tab| tab.title.to_string())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(titles(cx), vec!["main.rs", "lib.rs", "Cargo.toml"]);
    assert_eq!(
        center.read_with(cx, |center, _| center.active_index()),
        Some(2)
    );

    // Dragging the last tab to the front is what the drop handler does.
    center.update(cx, |center, cx| center.move_tab(2, 0, cx));
    assert_eq!(titles(cx), vec!["Cargo.toml", "main.rs", "lib.rs"]);
    assert_eq!(
        center.read_with(cx, |center, _| center.active_index()),
        Some(0),
        "la pestaña activa se mueve con ella"
    );
}

#[gpui::test]
fn ctrl_shift_e_toggles_the_tree_panel(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    let open_before = workspace.read_with(cx, |workspace, cx| {
        workspace.is_dock_open(DockPlacement::Right, cx)
    });
    assert!(open_before, "el árbol arranca abierto");

    // The focus starts outside the tree, so the first press only focuses it
    // (`07-etapa5-productividad.md` §7.1, rule 3); the second one, from
    // inside, collapses it.
    cx.simulate_keystrokes("ctrl-shift-e");
    cx.run_until_parked();
    assert_eq!(zone(&workspace, cx), Some(FocusZone::Files));
    assert!(workspace.read_with(cx, |workspace, cx| {
        workspace.is_dock_open(DockPlacement::Right, cx)
    }));
    cx.simulate_keystrokes("ctrl-shift-e");
    cx.run_until_parked();
    let open_after = workspace.read_with(cx, |workspace, cx| {
        workspace.is_dock_open(DockPlacement::Right, cx)
    });
    assert!(!open_after, "Ctrl+Shift+E debería colapsar el árbol");

    cx.simulate_keystrokes("ctrl-shift-e");
    cx.run_until_parked();
    let open_again = workspace.read_with(cx, |workspace, cx| {
        workspace.is_dock_open(DockPlacement::Right, cx)
    });
    assert!(open_again, "y volver a abrirlo");
}

#[gpui::test]
fn alt_j_without_pending_changes_says_so(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    cx.simulate_keystrokes("alt-j");
    cx.run_until_parked();

    let messages = workspace.read_with(cx, |workspace, cx| {
        workspace
            .toasts()
            .read(cx)
            .items()
            .iter()
            .map(|toast| toast.message.to_string())
            .collect::<Vec<_>>()
    });
    assert!(
        messages
            .iter()
            .any(|message| message == "No hay cambios pendientes"),
        "{messages:?}"
    );
}

/// Opens `relative` from the tree, pinned, and returns the workspace's tab
/// area once the editor has the keyboard.
fn open_pinned(
    workspace: &Entity<Workspace>,
    relative: &str,
    cx: &mut VisualTestContext,
) -> Entity<crate::center::CenterPanel> {
    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new(relative), true, cx)
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| workspace.center().clone())
}

#[gpui::test]
fn typing_marks_the_tab_dirty_and_ctrl_s_saves_it(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "src/lib.rs", cx);

    // The editor of the active tab has the keyboard, so typing reaches it.
    let editor = center.read_with(cx, |center, _| {
        center.active_tab().expect("hay pestaña").editor().clone()
    });
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty()));

    cx.simulate_input("día ");
    cx.run_until_parked();
    assert!(
        editor.read_with(cx, |editor, _| editor.is_dirty()),
        "escribir debería ensuciar el buffer"
    );
    assert!(
        center.read_with(cx, |center, cx| center.is_tab_dirty(0, cx)),
        "y la pestaña debería mostrar el punto"
    );
    // Accents survive the trip (one keystroke, two bytes).
    assert!(
        editor
            .read_with(cx, |editor, _| editor.text())
            .starts_with("día ")
    );

    // `Ctrl+S` writes the file and cleans the tab.
    cx.simulate_keystrokes("ctrl-s");
    cx.run_until_parked();
    assert!(
        !editor.read_with(cx, |editor, _| editor.is_dirty()),
        "guardar debería limpiar el punto"
    );
    let on_disk = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert_eq!(on_disk, "día pub fn suma() {}\n");
}

#[gpui::test]
fn a_hot_reload_repaints_the_open_editors(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "src/main.rs", cx);
    let editor = center.read_with(cx, |center, _| {
        center.active_tab().expect("hay pestaña").editor().clone()
    });

    let before = editor.read_with(cx, |editor, _| *editor.theme());
    let before_size = editor.read_with(cx, |editor, _| editor.settings().font_size);

    // The dark theme with a bigger code font, as a settings reload would leave
    // the globals (the test window reports a light desktop, so `mode: dark` is
    // a real change).
    cx.update(|_, cx| {
        let mut config = Config::default();
        config.settings.theme.mode = cincel_settings::ThemeMode::Dark;
        config.settings.buffer_font_size = 22.;
        crate::settings::install(config, cx);
        center.update(cx, |center, cx| center.refresh_editor_style(cx));
    });
    cx.run_until_parked();

    let after = editor.read_with(cx, |editor, _| *editor.theme());
    let after_size = editor.read_with(cx, |editor, _| editor.settings().font_size);
    assert_ne!(
        before.background, after.background,
        "el tema debería cambiar"
    );
    assert_ne!(before_size, after_size);
    assert_eq!(after_size, 22.);
}

/// `settings.json`'s `editor.auto_close_pairs` reaches the editor both when a
/// tab opens (`crate::theme::editor_settings`, read by `TabContent::build`)
/// and on a later hot reload (`EditorView::set_settings`).
#[gpui::test]
fn auto_close_pairs_setting_reaches_the_editor_on_load_and_hot_reload(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| {
        let mut config = Config::default();
        config.settings.editor.auto_close_pairs = false;
        crate::init(config, cx);
    });
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "src/main.rs", cx);
    let editor = center.read_with(cx, |center, _| {
        center.active_tab().expect("hay pestaña").editor().clone()
    });

    // On load: the setting was already off when the tab's editor was built.
    assert!(!editor.read_with(cx, |editor, _| editor.auto_close_pairs()));
    cx.simulate_keystrokes("ctrl-end");
    let before_len = editor.read_with(cx, |editor, _| editor.text().len());
    cx.simulate_input("(");
    cx.run_until_parked();
    let after_len = editor.read_with(cx, |editor, _| editor.text().len());
    assert_eq!(
        after_len,
        before_len + 1,
        "con el ajuste en false no debería autocerrar"
    );

    // Hot reload: flipping it back on reaches the same editor without
    // reopening the tab.
    cx.update(|_, cx| {
        let mut config = Config::default();
        config.settings.editor.auto_close_pairs = true;
        crate::settings::install(config, cx);
        center.update(cx, |center, cx| center.refresh_editor_style(cx));
    });
    cx.run_until_parked();
    assert!(editor.read_with(cx, |editor, _| editor.auto_close_pairs()));
    cx.simulate_keystrokes("ctrl-end");
    let before_len = editor.read_with(cx, |editor, _| editor.text().len());
    cx.simulate_input("[");
    cx.run_until_parked();
    let after_len = editor.read_with(cx, |editor, _| editor.text().len());
    assert_eq!(
        after_len,
        before_len + 2,
        "con el ajuste en true debería autocerrar"
    );
}

/// Writes `name` at the root of `dir` and opens it, pinned, returning the
/// tab area once its editor has the keyboard.
fn open_markdown(
    workspace: &Entity<Workspace>,
    dir: &Path,
    name: &str,
    text: &str,
    cx: &mut VisualTestContext,
) -> Entity<crate::center::CenterPanel> {
    std::fs::write(dir.join(name), text).unwrap();
    // `open_pinned` goes straight through `CenterPanel::open_file_at`, which
    // reads the file off disk itself: no tree rescan needed for a file
    // written just before opening it.
    open_pinned(workspace, name, cx)
}

/// Lets the preview's 150 ms debounce (`crate::markdown_preview`) land.
fn settle_preview(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
}

#[gpui::test]
fn toggling_a_markdown_tab_switches_content_and_back(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_markdown(
        &workspace,
        dir.path(),
        "README.md",
        "# Título\n\nHola.\n",
        cx,
    );

    let is_preview = |cx: &mut VisualTestContext| {
        center.read_with(cx, |center, _| {
            center
                .active_tab()
                .expect("hay pestaña")
                .is_showing_markdown_preview()
        })
    };
    assert!(!is_preview(cx), "arranca mostrando el editor");

    cx.simulate_keystrokes("ctrl-shift-v");
    settle_preview(cx);
    assert!(
        is_preview(cx),
        "Ctrl+Shift+V debería mostrar la vista previa"
    );
    // The tab keeps its title regardless of what it is showing.
    assert_eq!(
        center.read_with(cx, |center, _| center
            .active_tab()
            .unwrap()
            .title
            .to_string()),
        "README.md"
    );

    cx.simulate_keystrokes("ctrl-shift-v");
    settle_preview(cx);
    assert!(!is_preview(cx), "y volver a mostrar el editor");
}

#[gpui::test]
fn typing_then_toggling_shows_the_new_text_in_the_preview(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_markdown(&workspace, dir.path(), "README.md", "Hola", cx);

    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" Mundo");
    cx.run_until_parked();

    cx.simulate_keystrokes("ctrl-shift-v");
    settle_preview(cx);

    let preview_text = center.read_with(cx, |center, cx| {
        let tab = center.active_tab().expect("hay pestaña");
        let TabContent::MarkdownPreview(_, preview) = &tab.content else {
            panic!("la pestaña debería estar en modo vista previa");
        };
        preview.read(cx).text().to_string()
    });
    assert_eq!(preview_text, "Hola Mundo");
}

#[gpui::test]
fn toggle_markdown_preview_is_a_noop_on_a_rust_tab(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "src/main.rs", cx);

    cx.simulate_keystrokes("ctrl-shift-v");
    cx.run_until_parked();

    let content_is_editor = center.read_with(cx, |center, _| {
        matches!(
            center.active_tab().expect("hay pestaña").content,
            TabContent::Editor(_)
        )
    });
    assert!(
        content_is_editor,
        "Ctrl+Shift+V no debería hacer nada en una pestaña de Rust"
    );
}

#[gpui::test]
fn layout_persists_and_restores_the_markdown_preview_state(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_markdown(&workspace, dir.path(), "README.md", "# Hola\n", cx);

    cx.simulate_keystrokes("ctrl-shift-v");
    settle_preview(cx);

    workspace.update(cx, |workspace, cx| workspace.save_layout(cx));
    let root = dir.path().to_path_buf();
    let saved = crate::layout::WorkspaceLayout::load(&root).expect("hay layout guardado");
    assert!(
        saved.tabs.iter().any(|tab| tab.markdown_preview),
        "{:?}",
        saved.tabs
    );

    // A fresh workspace over the same project restores it already showing
    // the preview.
    let (workspace2, cx) = workspace_window(dir.path(), &mut cx.cx);
    let center2 = workspace2.read_with(cx, |workspace, _| workspace.center().clone());
    let restored_preview = center2.read_with(cx, |center, _| {
        center
            .active_tab()
            .expect("hay pestaña restaurada")
            .is_showing_markdown_preview()
    });
    assert!(
        restored_preview,
        "el layout debería reabrir en vista previa"
    );
}

#[gpui::test]
fn closing_a_dirty_tab_asks_first(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new("src/main.rs"), true, cx)
    });
    cx.run_until_parked();

    // Type into the editor, which is what makes the buffer dirty.
    let (project, center) = workspace.read_with(cx, |workspace, _| {
        (
            workspace.project().expect("hay proyecto").clone(),
            workspace.center().clone(),
        )
    });
    let path = center.read_with(cx, |center, _| {
        center.active_tab().expect("hay pestaña").path.clone()
    });
    cx.simulate_input("// sucio");
    cx.run_until_parked();
    assert!(project.read_with(cx, |project, _| project.is_dirty(&path)));

    // `Ctrl+W` asks instead of closing.
    cx.simulate_keystrokes("ctrl-w");
    cx.run_until_parked();
    assert!(
        center.read_with(cx, |center, _| center.is_asking_about_unsaved_changes()),
        "debería preguntar por los cambios sin guardar"
    );
    assert_eq!(center.read_with(cx, |center, _| center.tabs().len()), 1);

    // "Cancelar" leaves everything as it was, "Descartar" closes the tab.
    cx.update(|window, cx| {
        center.update(cx, |center, cx| center.cancel_close_for_test(window, cx))
    });
    assert!(!center.read_with(cx, |center, _| center.is_asking_about_unsaved_changes()));
    cx.simulate_keystrokes("ctrl-w");
    cx.run_until_parked();
    cx.update(|window, cx| {
        center.update(cx, |center, cx| {
            center.discard_and_close_for_test(window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(center.read_with(cx, |center, _| center.tabs().len()), 0);
    assert!(!project.read_with(cx, |project, _| project.buffers().is_open(&path)));
}

#[gpui::test]
fn ctrl_w_keeps_closing_tabs(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    for path in ["src/main.rs", "src/lib.rs", "Cargo.toml"] {
        open_pinned(&workspace, path, cx);
    }
    let center = workspace.read_with(cx, |workspace, _| workspace.center().clone());
    assert_eq!(center.read_with(cx, |center, _| center.tabs().len()), 3);

    // The editor of the active tab has the keyboard; closing it has to hand
    // the keyboard to the next one, or the second `Ctrl+W` finds no context.
    let focused = |cx: &mut VisualTestContext| {
        let editor = center.read_with(cx, |center, _| {
            center.active_tab().map(|tab| tab.editor().clone())
        })?;
        Some(cx.update(|window, cx| {
            use gpui::Focusable as _;
            editor.read(cx).focus_handle(cx).is_focused(window)
        }))
    };
    assert_eq!(focused(cx), Some(true));

    cx.simulate_keystrokes("ctrl-w");
    cx.run_until_parked();
    assert_eq!(center.read_with(cx, |center, _| center.tabs().len()), 2);
    assert_eq!(
        center.read_with(cx, |center, _| center
            .active_tab()
            .map(|tab| tab.title.to_string())),
        Some("lib.rs".to_string())
    );
    assert_eq!(
        focused(cx),
        Some(true),
        "el editor activo debe tener el foco"
    );

    cx.simulate_keystrokes("ctrl-w");
    cx.run_until_parked();
    assert_eq!(
        center.read_with(cx, |center, _| center.tabs().len()),
        1,
        "el segundo Ctrl+W también tiene que cerrar"
    );
    assert_eq!(
        center.read_with(cx, |center, _| center
            .active_tab()
            .map(|tab| tab.title.to_string())),
        Some("main.rs".to_string())
    );

    // And con la última cerrada, el panel se queda el foco para que los
    // comandos globales sigan andando.
    cx.simulate_keystrokes("ctrl-w");
    cx.run_until_parked();
    assert_eq!(center.read_with(cx, |center, _| center.tabs().len()), 0);
    assert_eq!(zone(&workspace, cx), Some(FocusZone::Center));
    // First press focuses the tree, the second collapses it (§7.1).
    cx.simulate_keystrokes("ctrl-shift-e");
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-shift-e");
    cx.run_until_parked();
    assert!(
        !workspace.read_with(cx, |workspace, cx| workspace
            .is_dock_open(DockPlacement::Right, cx)),
        "Ctrl+Shift+E debería seguir funcionando sin pestañas"
    );
}

#[gpui::test]
fn enter_and_escape_answer_the_unsaved_dialog(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "src/lib.rs", cx);

    cx.simulate_input("// sucio\n");
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-w");
    cx.run_until_parked();
    assert!(center.read_with(cx, |center, _| center.is_asking_about_unsaved_changes()));

    // `Esc` cancela: la pestaña sigue abierta y sucia.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!center.read_with(cx, |center, _| center.is_asking_about_unsaved_changes()));
    assert_eq!(center.read_with(cx, |center, _| center.tabs().len()), 1);

    // `Enter` guarda y cierra.
    cx.simulate_keystrokes("ctrl-w");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(center.read_with(cx, |center, _| center.tabs().len()), 0);
    let on_disk = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(on_disk.starts_with("// sucio\n"), "{on_disk:?}");
}

#[gpui::test]
fn keyed_toasts_replace_each_other(cx: &mut TestAppContext) {
    init_test(cx);
    let toasts = cx.update(|cx| cx.new(|_| Toasts::new()));

    toasts.update(cx, |toasts, cx| {
        toasts.push_keyed("zoom", "Zoom 110 %", ToastKind::Info, cx);
        toasts.push_keyed("zoom", "Zoom 120 %", ToastKind::Info, cx);
        toasts.push("Otra cosa", ToastKind::Info, cx);
    });
    cx.run_until_parked();

    let messages = toasts.read_with(cx, |toasts, _| {
        toasts
            .items()
            .iter()
            .map(|toast| toast.message.to_string())
            .collect::<Vec<_>>()
    });
    assert_eq!(messages, vec!["Zoom 120 %", "Otra cosa"], "{messages:?}");
}

#[gpui::test]
fn a_toast_appears_and_expires(cx: &mut TestAppContext) {
    init_test(cx);
    let toasts = cx.update(|cx| cx.new(|_| Toasts::new()));

    toasts.update(cx, |toasts, cx| {
        toasts.push("Hola", ToastKind::Info, cx);
    });
    cx.run_until_parked();
    assert_eq!(toasts.read_with(cx, |toasts, _| toasts.items().len()), 1);

    cx.executor()
        .advance_clock(crate::toast::TOAST_DURATION + Duration::from_millis(100));
    cx.run_until_parked();
    assert!(
        toasts.read_with(cx, |toasts, _| toasts.is_empty()),
        "el aviso debería haberse ido a los 4 s"
    );
}

#[gpui::test]
fn the_layout_of_a_project_is_saved_and_restored(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new("src/main.rs"), true, cx)
    });
    cx.run_until_parked();
    // From the editor the first press focuses the chat, the second one
    // collapses it (§7.1).
    cx.simulate_keystrokes("ctrl-shift-a");
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-shift-a");
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| workspace.save_layout(cx));

    let layout = crate::layout::WorkspaceLayout::load(&std::path::absolute(dir.path()).unwrap())
        .expect("el layout se guardó");
    assert!(!layout.chat.open, "el chat quedó colapsado");
    assert_eq!(layout.tabs.len(), 1);
    assert_eq!(layout.tabs[0].path, PathBuf::from("src/main.rs"));
    assert!(!layout.tabs[0].preview);
    assert_eq!(layout.active, Some(0));
}

#[gpui::test]
fn a_file_that_changes_on_disk_reaches_the_editor(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "src/lib.rs", cx);
    let project = workspace.read_with(cx, |workspace, _| {
        workspace.project().expect("hay proyecto").clone()
    });
    let path = center.read_with(cx, |center, _| center.tabs()[0].path.clone());
    let editor = center.read_with(cx, |center, _| center.tabs()[0].editor().clone());

    // Someone else writes the file while our copy is clean.
    std::fs::write(&path, "pub fn resta() {}\n").unwrap();
    let outcome = project.update(cx, |project, _| {
        project.buffers_mut().reload_from_disk(&path)
    });
    assert_eq!(outcome, cincel_project::ReloadOutcome::Reloaded);
    center.update(cx, |center, cx| {
        center.apply_buffer_changes(&[change(&path, outcome)], cx)
    });
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.text()),
        "pub fn resta() {}\n",
        "el editor debería mostrar lo que hay en el disco"
    );

    // Now the same thing with unsaved changes: the user decides.
    cx.simulate_input("// mío\n");
    cx.run_until_parked();
    std::fs::write(&path, "pub fn otra() {}\n").unwrap();
    let outcome = project.update(cx, |project, _| {
        project.buffers_mut().reload_from_disk(&path)
    });
    assert_eq!(outcome, cincel_project::ReloadOutcome::Conflict);
    center.update(cx, |center, cx| {
        center.apply_buffer_changes(&[change(&path, outcome)], cx)
    });
    cx.run_until_parked();

    let toasts = workspace.read_with(cx, |workspace, _| workspace.toasts().clone());
    let (id, labels) = toasts.read_with(cx, |toasts, _| {
        let toast = toasts.items().last().expect("hay un aviso");
        (
            toast.id,
            toast
                .actions
                .iter()
                .map(|(label, _)| label.to_string())
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(labels, vec!["Recargar del disco", "Mantener mi versión"]);

    toasts.update(cx, |toasts, cx| toasts.run_action(id, 0, cx));
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.text()),
        "pub fn otra() {}\n"
    );
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty()));

    // And a file that disappears is marked in its tab.
    std::fs::remove_file(&path).unwrap();
    let outcome = project.update(cx, |project, _| {
        project.buffers_mut().reload_from_disk(&path)
    });
    assert_eq!(outcome, cincel_project::ReloadOutcome::Deleted);
    center.update(cx, |center, cx| {
        center.apply_buffer_changes(&[change(&path, outcome)], cx)
    });
    assert_eq!(
        center.read_with(cx, |center, _| center.tabs()[0].display_title().to_string()),
        "lib.rs (eliminado)"
    );
}

/// The [`cincel_project::BufferChange`] the watcher would have produced.
fn change(path: &Path, outcome: cincel_project::ReloadOutcome) -> cincel_project::BufferChange {
    cincel_project::BufferChange {
        path: path.to_path_buf(),
        outcome,
    }
}

#[gpui::test]
fn autosave_writes_when_the_window_loses_focus(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    cx.update(|cx| {
        let mut config = Config::default();
        config.settings.files.autosave = cincel_settings::Autosave::OnFocusChange;
        crate::settings::install(config, cx);
    });
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "src/lib.rs", cx);
    let editor = center.read_with(cx, |center, _| center.tabs()[0].editor().clone());

    cx.simulate_input("// auto\n");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |editor, _| editor.is_dirty()));

    // The test platform only reports a deactivation for a window it considers
    // active, so the window is activated first.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    cx.deactivate_window();
    cx.run_until_parked();

    assert!(
        !editor.read_with(cx, |editor, _| editor.is_dirty()),
        "el autoguardado debería haber escrito el archivo"
    );
    let on_disk = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(on_disk.starts_with("// auto\n"), "{on_disk:?}");
}

#[gpui::test]
fn the_layout_remembers_where_the_cursor_was(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let file = dir.path().join("src/largo.rs");
    let lines: String = (0..400).map(|row| format!("// línea {row}\n")).collect();
    std::fs::write(&file, lines).unwrap();

    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "src/largo.rs", cx);
    let editor = center.read_with(cx, |center, _| center.tabs()[0].editor().clone());

    // Walk far enough down that the view has to scroll to follow the cursor.
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input("120");
    cx.simulate_keystrokes("enter right right");
    cx.run_until_parked();

    let (cursor, scroll) = center.read_with(cx, |center, _| {
        let tab = &center.tabs()[0];
        (tab.cursor, tab.scroll)
    });
    assert_eq!((cursor.row, cursor.column), (119, 2));
    assert!(scroll > 0., "la vista debería haber bajado: {scroll}");
    assert_eq!(
        scroll,
        editor.read_with(cx, |editor, _| editor.scroll_row()),
        "la pestaña guarda la fila que reporta el editor"
    );

    workspace.read_with(cx, |workspace, cx| workspace.save_layout(cx));
    let layout = crate::layout::WorkspaceLayout::load(&std::path::absolute(dir.path()).unwrap())
        .expect("el layout se guardó");
    assert_eq!(layout.tabs.len(), 1);
    assert_eq!(layout.tabs[0].cursor, Some([119, 2]));
    assert_eq!(layout.tabs[0].scroll, scroll);

    // A second window on the same project restores both.
    let mut app = cx.cx.clone();
    let (restored, cx) = workspace_window(dir.path(), &mut app);
    cx.run_until_parked();
    let (tabs, cursor, tab_scroll, editor_scroll) = restored.read_with(cx, |workspace, cx| {
        let center = workspace.center().read(cx);
        let tab = center.active_tab().expect("hay pestaña");
        (
            center.tabs().len(),
            tab.cursor,
            tab.scroll,
            tab.editor().read(cx).scroll_row(),
        )
    });
    assert_eq!(tabs, 1);
    assert_eq!((cursor.row, cursor.column), (119, 2), "el cursor vuelve");
    assert_eq!(editor_scroll, scroll, "y la vista está donde estaba");
    assert_eq!(tab_scroll, scroll);
}

#[gpui::test]
fn the_breadcrumb_names_the_function_under_the_cursor(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    let x = 1;\n}\n\nfn segunda(valor: u32) -> u32 {\n    valor + 1\n}\n",
    )
    .unwrap();

    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "src/main.rs", cx);
    // The grammar parses in the background.
    cx.run_until_parked();

    let crumbs = |cx: &mut VisualTestContext| {
        center.read_with(cx, |center, _| {
            let tab = center.active_tab().expect("hay pestaña");
            (
                tab.relative.display().to_string(),
                tab.symbol.as_ref().map(|symbol| symbol.to_string()),
            )
        })
    };

    // The cursor starts on `fn main`.
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    let (path, symbol) = crumbs(cx);
    assert_eq!(path, "src/main.rs");
    assert_eq!(
        symbol.as_deref(),
        Some("main"),
        "debería nombrar la función"
    );

    // And it follows the cursor into the second function.
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input("6");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(crumbs(cx).1.as_deref(), Some("segunda"));

    // Between the two there is no definition to name.
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input("4");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(crumbs(cx).1, None);
}

#[gpui::test]
fn the_keymap_binds_every_workspace_command(cx: &mut TestAppContext) {
    init_test(cx);
    let conversion = cx.update(|cx| {
        let keymap = crate::settings::settings(cx);
        let _ = keymap;
        let config = Config::default();
        crate::keymap::convert(&config.keymap, cx)
    });
    // Every `workspace::*` and `chat::*` command of the default keymap
    // resolves to an action (`editor::*` since Etapa 1, `chat::*` since
    // Etapa 2's `cincel_chat::default_key_bindings` merge in
    // `crate::keymap::built_in_bindings`).
    assert!(
        conversion
            .skipped
            .iter()
            .all(|(_, command, _)| !command.starts_with("workspace::")
                && !command.starts_with("chat::")),
        "{:?}",
        conversion.skipped
    );
    assert!(!conversion.bindings.is_empty());

    // The global commands of `07-etapa5-productividad.md` §11.1 resolve to
    // registered actions with their default keys, even the ones whose
    // feature lands in a later sub-stage.
    for (keystroke, command) in [
        ("ctrl-l", "workspace::focus_next_zone"),
        ("ctrl-p", "workspace::toggle_file_finder"),
        ("ctrl-,", "workspace::open_settings"),
        ("f1", "workspace::show_shortcuts"),
        ("ctrl-n", "workspace::new_file"),
        ("ctrl-q", "workspace::quit"),
        ("ctrl-shift-a", "workspace::toggle_chat"),
        ("ctrl-shift-e", "workspace::toggle_tree"),
    ] {
        let expected = gpui::Keystroke::parse(keystroke).unwrap();
        assert!(
            conversion.bindings.iter().any(|binding| {
                binding.action().name() == command
                    && binding.predicate().is_none()
                    && binding.keystrokes().len() == 1
                    && binding.keystrokes()[0].key() == expected.key.as_str()
                    && binding.keystrokes()[0].modifiers() == &expected.modifiers
            }),
            "{keystroke} debería ser {command}"
        );
    }
    // `workspace::open_connections` and `workspace::focus_chat` exist without
    // a default key.
    for command in ["workspace::open_connections", "workspace::focus_chat"] {
        assert!(
            cx.update(|cx| crate::keymap::resolve_action(command, cx))
                .is_some(),
            "{command} debería estar registrado"
        );
        assert!(
            !conversion
                .bindings
                .iter()
                .any(|binding| binding.action().name() == command),
            "{command} no debería tener atajo por defecto"
        );
    }
}

/// `workspace::focus_chat` lost `Ctrl+L` (§9.2, D9) but still works for a
/// user keymap that binds it.
#[gpui::test]
fn focus_chat_still_focuses_the_chat_composer(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    // Move the keyboard elsewhere first, so the assertion is meaningful, and
    // collapse the chat: the command opens it.
    open_pinned(&workspace, "Cargo.toml", cx);
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_dock(DockPlacement::Left, window, cx)
        })
    });
    cx.run_until_parked();

    cx.dispatch_action(crate::actions::FocusChat);
    cx.run_until_parked();

    // Typing now must land in the composer, not in whatever had the keyboard
    // before: proof the command moved the focus all the way to the input,
    // not just to the panel.
    cx.simulate_input("hola");
    cx.run_until_parked();
    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    let text = chat.read_with(cx, |chat, cx| chat.input_text(cx));
    assert_eq!(text, "hola");

    let dock_open = workspace.read_with(cx, |workspace, cx| {
        workspace.is_dock_open(DockPlacement::Left, cx)
    });
    assert!(
        dock_open,
        "workspace::focus_chat debería abrir el dock del chat si estaba colapsado"
    );
}

#[gpui::test]
fn mentioning_a_file_from_the_tree_writes_the_token_in_the_draft(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new("src/main.rs"), true, cx)
    });
    cx.run_until_parked();
    files.update(cx, |_files, cx| {
        cx.emit(WorkspaceEvent::MentionFile {
            path: dir.path().join("src/main.rs"),
        });
    });
    cx.run_until_parked();

    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    let (mentions, text) = chat.read_with(cx, |chat, cx| {
        (
            chat.mentions()
                .iter()
                .map(|mention| mention.path.clone())
                .collect::<Vec<_>>(),
            chat.input_text(cx),
        )
    });
    assert_eq!(mentions, vec![dir.path().join("src/main.rs")]);
    assert_eq!(text, "@main.rs ", "la mención se escribe en el texto");
}

/// `workspace::zoom_*` reaches the chat (its type scale and its composer's
/// font), the open editors and the pixel sizes of the chrome; `zoom_reset`
/// takes everything back to 100 %.
#[gpui::test]
fn the_zoom_scales_the_chat_and_the_editor_and_resets(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "src/main.rs", cx);
    let editor = center.read_with(cx, |center, _| {
        center.active_tab().expect("hay pestaña").editor().clone()
    });
    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    let buffer_font = cx.update(|_, cx| crate::settings::settings(cx).buffer_font_size);

    workspace.update_in(cx, |workspace, window, cx| workspace.zoom(1.5, window, cx));
    cx.run_until_parked();
    let (body, code, small, label, composer) = chat.read_with(cx, |chat, cx| {
        let settings = chat.settings();
        (
            settings.text_body(),
            settings.text_code(),
            settings.text_small(),
            settings.text_label(),
            chat.composer().read(cx).settings().font_size,
        )
    });
    assert_eq!(body, cincel_chat::settings::TEXT_BODY * 1.5);
    assert_eq!(code, cincel_chat::settings::TEXT_CODE * 1.5);
    assert_eq!(small, cincel_chat::settings::TEXT_SMALL * 1.5);
    assert_eq!(label, cincel_chat::settings::TEXT_LABEL * 1.5);
    assert_eq!(composer, cincel_chat::settings::TEXT_BODY * 1.5);
    let editor_font = editor.read_with(cx, |editor, _| editor.settings().font_size);
    assert_eq!(editor_font, buffer_font * 1.5);
    assert_eq!(cx.update(|_, cx| crate::settings::ui_scale(cx)), 1.5);

    workspace.update_in(cx, |workspace, window, cx| workspace.zoom(1., window, cx));
    cx.run_until_parked();
    let (scale, body) = chat.read_with(cx, |chat, _| {
        (chat.settings().scale, chat.settings().text_body())
    });
    assert_eq!(scale, 1.0);
    assert_eq!(body, cincel_chat::settings::TEXT_BODY);
    let editor_font = editor.read_with(cx, |editor, _| editor.settings().font_size);
    assert_eq!(editor_font, buffer_font);
}

/// The chat's text views select with `text.accent` at 35 % in both themes:
/// the `TextViewStyle` they carry and gpui-kit's own `selection` token (their
/// fallback, and the base theme it syncs to). The code editor keeps the
/// theme's `selection`.
#[gpui::test]
fn the_chat_selection_is_the_accent_at_35_percent_in_both_themes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        for cincel in [
            cincel_settings::Theme::cincel_dark(),
            cincel_settings::Theme::cincel_light(),
        ] {
            let name = cincel.name.clone();
            crate::theme::apply(&cincel, &cincel_settings::Settings::default(), 1.0, cx);
            let colors = crate::theme::ThemeColors::global(cx).clone();
            let expected = colors.text_accent.alpha(0.35);
            let kit = &gpui_kit::component::Theme::global(cx).colors;
            assert_eq!(kit.selection, expected, "{name}: gpui-kit selection");
            let base = gpui_kit::base::Theme::global(cx).tokens.colors.selection;
            assert_eq!(base, expected, "{name}: gpui-base selection");
            let chat = crate::theme::chat_theme(cx);
            let style = cincel_chat::markdown::text_view_style(&chat);
            assert_eq!(style.selection(), expected, "{name}: TextViewStyle");
            // The code editor's selection is untouched.
            let editor = crate::theme::editor_theme(cx);
            assert_eq!(
                editor.selection,
                gpui::Rgba::from(colors.selection),
                "{name}"
            );
        }
    });
}

// ------------------------------------------------------------ conexiones

/// Spec 06 §1 and §10: the window opens with nothing connected, the status
/// bar says "Sin conexión", and choosing a connection puts its chip there
/// (clicking the chip opens the chat's "Conectar" popover).
#[gpui::test]
fn the_window_opens_without_a_connection_and_the_chip_follows_the_choice(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let (agents, chat) = workspace.read_with(cx, |workspace, _| {
        (workspace.agents().clone(), workspace.chat().clone())
    });
    assert!(!agents.read_with(cx, |agents, _| agents.is_agent_running()));
    assert_eq!(
        workspace.read_with(cx, |workspace, cx| workspace.status_connection_label(cx)),
        "Sin conexión"
    );

    let env = crate::test_support::FakeEnv::new();
    let id = env.add_connection("claude-acp", "Claude · personal");
    agents.update(cx, |agents, cx| {
        agents.set_connections(env.connections.clone(), cx)
    });
    assert!(
        !agents.read_with(cx, |agents, _| agents.is_agent_running()),
        "listar conexiones no lanza nada"
    );
    chat.update(cx, |chat, cx| chat.select_connection(&id.to_string(), cx));
    cx.run_until_parked();
    assert!(agents.read_with(cx, |agents, _| agents.is_agent_running()));
    assert_eq!(
        workspace.read_with(cx, |workspace, cx| workspace.status_connection_label(cx)),
        "Claude · personal"
    );

    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| workspace.open_connections(window, cx))
    });
    assert_eq!(
        chat.read_with(cx, |chat, _| chat.popover().clone()),
        cincel_chat::Popover::Connections
    );
    // Deactivating (as a deletion does) takes the chip back to "Sin conexión".
    let modal = agents.read_with(cx, |agents, _| agents.modal().clone());
    cx.update(|window, cx| modal.update(cx, |modal, cx| modal.open_delete(window, cx)));
    cx.update(|_window, cx| modal.update(cx, |modal, cx| modal.pick_for_deletion(id, cx)));
    cx.update(|window, cx| modal.update(cx, |modal, cx| modal.confirm_delete(window, cx)));
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, cx| workspace.status_connection_label(cx)),
        "Sin conexión"
    );
    assert!(!agents.read_with(cx, |agents, _| agents.is_agent_running()));
}

/// "Elegí un agente": at any window size the dialog stays between its
/// bounds (spec 06 §6) and every agent card lies inside it (the grid wraps
/// instead of clipping the last card).
#[gpui::test]
fn the_agent_grid_never_overflows_the_dialog(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    let modal = agents.read_with(cx, |agents, _| agents.modal().clone());
    cx.update(|window, cx| modal.update(cx, |modal, cx| modal.open_connect(window, cx)));
    cx.run_until_parked();

    let mut widths = Vec::new();
    for (width, height) in [(900., 700.), (1400., 900.), (480., 700.)] {
        cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
        cx.run_until_parked();
        let dialog = cx
            .debug_bounds("connections-dialog")
            .expect("el diálogo se pinta");
        assert!(
            dialog.size.width <= gpui::px(560.) && dialog.size.width >= gpui::px(420.),
            "{width}x{height}: ancho {:?}",
            dialog.size.width
        );
        let grid = cx.debug_bounds("agent-grid").expect("la grilla se pinta");
        assert!(
            grid.left() >= dialog.left() && grid.right() <= dialog.right(),
            "{width}x{height}: grilla {grid:?} fuera de {dialog:?}"
        );
        let mut rows = std::collections::BTreeSet::new();
        for index in 0..cincel_connections::AgentKind::ALL.len() {
            let card = cx
                .debug_bounds(Box::leak(format!("agent-card-{index}").into_boxed_str()))
                .unwrap_or_else(|| panic!("{width}x{height}: falta la tarjeta {index}"));
            assert!(
                card.left() >= dialog.left()
                    && card.right() <= dialog.right()
                    && card.top() >= dialog.top()
                    && card.bottom() <= dialog.bottom(),
                "{width}x{height}: tarjeta {index} {card:?} fuera de {dialog:?}"
            );
            assert!(card.size.width >= gpui::px(160.), "{card:?}");
            rows.insert(i64::from(f32::from(card.top()) as i32));
        }
        widths.push((dialog.size.width, rows.len()));
    }
    // Wide windows keep the dialog at its maximum with the three cards in
    // one row; the narrowest one shrinks it and wraps the grid.
    assert_eq!(widths[0], (gpui::px(560.), 1), "{widths:?}");
    assert_eq!(widths[1], (gpui::px(560.), 1), "{widths:?}");
    assert!(widths[2].0 < gpui::px(560.), "{widths:?}");
    assert!(widths[2].1 >= 2, "la grilla pasa a dos filas: {widths:?}");
}

/// Opens `src/long.rs` (one 400-character line) with the render probe on,
/// soft wrap as given, in a 1200×800 window, and returns its editor.
fn open_long_line(
    workspace: &Entity<Workspace>,
    dir: &Path,
    soft_wrap: bool,
    cx: &mut VisualTestContext,
) -> Entity<cincel_editor::EditorView> {
    std::fs::write(dir.join("src/long.rs"), format!("// {}\n", "x".repeat(400))).unwrap();
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(800.)));
    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new("src/long.rs"), true, cx)
    });
    cx.run_until_parked();
    let editor = workspace.read_with(cx, |workspace, cx| {
        let center = workspace.center().read(cx);
        center
            .active_tab()
            .expect("pestaña")
            .content
            .editor()
            .clone()
    });
    editor.update(cx, |editor, cx| {
        editor.set_render_probe(true);
        editor.set_soft_wrap(soft_wrap, cx);
    });
    cx.run_until_parked();
    editor
}

fn last_frame(
    editor: &Entity<cincel_editor::EditorView>,
    cx: &mut VisualTestContext,
) -> cincel_editor::FrameRender {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    editor
        .read_with(cx, |editor, _| editor.render_frames().last().cloned())
        .expect("un cuadro pintado")
}

/// The code never runs under the right dock: the editor ends where the dock
/// starts, paints under a mask no wider than itself, and follows the dock
/// when its border is dragged (soft wrap off, a 400-character line).
#[gpui::test]
fn the_editor_ends_at_the_right_dock_and_follows_its_resize(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let editor = open_long_line(&workspace, dir.path(), false, cx);

    let mut rights = Vec::new();
    for dock in [240., 420., 180.] {
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.set_dock_width(DockPlacement::Right, gpui::px(dock), window, cx)
            })
        });
        let frame = last_frame(&editor, cx);
        let dock_width = workspace
            .read_with(cx, |workspace, cx| {
                workspace.dock_width(DockPlacement::Right, cx)
            })
            .expect("dock derecho");
        assert_eq!(dock_width, gpui::px(dock));
        let dock_left = gpui::px(1200.) - dock_width;
        assert!(
            frame.bounds.right() <= dock_left,
            "dock {dock}: el editor {:?} pasa por debajo del dock (empieza en {dock_left:?})",
            frame.bounds
        );
        let clip = frame.paint_clip.expect("máscara");
        assert!(
            clip.right() <= frame.bounds.right() && clip.left() >= frame.bounds.left(),
            "dock {dock}: máscara {clip:?} fuera de {:?}",
            frame.bounds
        );
        rights.push(frame.bounds.right());
    }
    // Widening the dock shrinks the editor and narrowing it grows it back.
    assert!(rights[1] < rights[0] && rights[2] > rights[0], "{rights:?}");
}

/// With soft wrap (the default) dragging the right dock re-wraps the code:
/// a wider dock leaves a narrower editor and more wrap rows.
#[gpui::test]
fn resizing_the_right_dock_rewraps_the_editor(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let editor = open_long_line(&workspace, dir.path(), true, cx);
    let mut frames = Vec::new();
    for dock in [200., 500.] {
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.set_dock_width(DockPlacement::Right, gpui::px(dock), window, cx)
            })
        });
        frames.push(last_frame(&editor, cx));
    }
    let body = cx
        .debug_bounds("center-body")
        .expect("cuerpo de la pestaña");
    assert_eq!(
        frames[1].bounds.right(),
        body.right(),
        "el editor llena la pestaña"
    );
    assert_eq!(
        frames[0].bounds.size.width - frames[1].bounds.size.width,
        gpui::px(300.),
        "el editor pierde lo que gana el dock"
    );
    assert!(
        frames[1].wrap_rows > frames[0].wrap_rows,
        "{} → {}",
        frames[0].wrap_rows,
        frames[1].wrap_rows
    );
}

// ------------------------------------------------------------------------
// Focus zones: `07-etapa5-productividad.md` §7 (Ctrl+Shift+A / Ctrl+Shift+E)
// and §9 (the Ctrl+L wheel).

/// The zone that holds the keyboard.
fn zone(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<FocusZone> {
    cx.update(|window, cx| workspace.read(cx).focus_zone(window, cx))
}

/// Whether the editor of the active tab has the keyboard.
fn editor_has_focus(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        use gpui::Focusable as _;
        let center = workspace.read(cx).center().read(cx);
        center
            .active_tab()
            .is_some_and(|tab| tab.editor().read(cx).focus_handle(cx).is_focused(window))
    })
}

/// Whether the chat's composer has the keyboard.
fn composer_has_focus(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        use gpui::Focusable as _;
        let chat = workspace.read(cx).chat().read(cx);
        chat.composer().read(cx).focus_handle(cx).is_focused(window)
    })
}

fn dock_open(
    workspace: &Entity<Workspace>,
    placement: DockPlacement,
    cx: &mut VisualTestContext,
) -> bool {
    workspace.read_with(cx, |workspace, cx| workspace.is_dock_open(placement, cx))
}

/// Collapses the dock at `placement` without touching the focus.
fn collapse(workspace: &Entity<Workspace>, placement: DockPlacement, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            if workspace.is_dock_open(placement, cx) {
                workspace.toggle_dock(placement, window, cx);
            }
        })
    });
    cx.run_until_parked();
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
}

/// §7.5: chat hidden + `Ctrl+Shift+A` → visible, cursor in the composer.
#[gpui::test]
fn ctrl_shift_a_opens_the_hidden_chat_with_the_cursor_in_the_composer(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_pinned(&workspace, "src/main.rs", cx);
    collapse(&workspace, DockPlacement::Left, cx);
    assert!(editor_has_focus(&workspace, cx));

    press("ctrl-shift-a", cx);
    assert!(dock_open(&workspace, DockPlacement::Left, cx));
    assert!(composer_has_focus(&workspace, cx));
    cx.simulate_input("hola");
    cx.run_until_parked();
    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    assert_eq!(chat.read_with(cx, |chat, cx| chat.input_text(cx)), "hola");
}

/// §7.5: typing in the composer + `Ctrl+Shift+A` → chat hidden, cursor in
/// the active editor.
#[gpui::test]
fn ctrl_shift_a_from_the_composer_hides_the_chat_and_returns_to_the_editor(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_pinned(&workspace, "src/main.rs", cx);
    press("ctrl-shift-a", cx);
    assert!(composer_has_focus(&workspace, cx));
    cx.simulate_input("borrador");
    cx.run_until_parked();

    press("ctrl-shift-a", cx);
    assert!(!dock_open(&workspace, DockPlacement::Left, cx));
    assert!(editor_has_focus(&workspace, cx));
}

/// §7.5: with the conversation list open (and clicked), `Ctrl+Shift+A` hides
/// the chat: the popover counts as inside the panel.
#[gpui::test]
fn ctrl_shift_a_with_the_conversation_list_open_hides_the_chat(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_pinned(&workspace, "src/main.rs", cx);
    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    chat.update(cx, |chat, cx| {
        chat.toggle_popover(cincel_chat::Popover::Conversations, cx)
    });
    cx.run_until_parked();
    // A click inside the popover is what gives it the keyboard.
    let popover = cx
        .debug_bounds("chat-popover")
        .expect("el popover se pinta");
    cx.simulate_click(popover.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        chat.read_with(cx, |chat, _| chat.popover().clone()),
        cincel_chat::Popover::Conversations,
        "el clic no cierra la lista"
    );
    assert_eq!(zone(&workspace, cx), Some(FocusZone::Chat));

    press("ctrl-shift-a", cx);
    assert!(!dock_open(&workspace, DockPlacement::Left, cx));
    assert!(editor_has_focus(&workspace, cx));
    assert_eq!(
        chat.read_with(cx, |chat, _| chat.popover().clone()),
        cincel_chat::Popover::Closed
    );
}

/// §7.5: chat visible, cursor in the editor + `Ctrl+Shift+A` → focus in the
/// composer, the chat stays visible.
#[gpui::test]
fn ctrl_shift_a_from_the_editor_focuses_the_visible_chat(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_pinned(&workspace, "src/main.rs", cx);
    assert!(dock_open(&workspace, DockPlacement::Left, cx));
    assert!(editor_has_focus(&workspace, cx));

    press("ctrl-shift-a", cx);
    assert!(dock_open(&workspace, DockPlacement::Left, cx));
    assert!(composer_has_focus(&workspace, cx));
}

/// §7.5: tree hidden + `Ctrl+Shift+E` → visible, focus on the selected row
/// (or the first), and the arrows move it.
#[gpui::test]
fn ctrl_shift_e_opens_the_hidden_tree_on_the_first_row(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_pinned(&workspace, "src/main.rs", cx);
    collapse(&workspace, DockPlacement::Right, cx);
    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    let selected = |cx: &mut VisualTestContext| {
        files.read_with(cx, |files, cx| files.tree_state().read(cx).selected_index())
    };
    assert_eq!(selected(cx), None);

    press("ctrl-shift-e", cx);
    assert!(dock_open(&workspace, DockPlacement::Right, cx));
    assert_eq!(zone(&workspace, cx), Some(FocusZone::Files));
    assert_eq!(selected(cx), Some(0), "sin selección, la primera fila");

    press("down", cx);
    assert_eq!(selected(cx), Some(1), "las flechas mueven la selección");

    // Closing and reopening keeps the selection instead of resetting it.
    press("ctrl-shift-e", cx);
    assert!(!dock_open(&workspace, DockPlacement::Right, cx));
    assert!(editor_has_focus(&workspace, cx));
    press("ctrl-shift-e", cx);
    assert_eq!(zone(&workspace, cx), Some(FocusZone::Files));
    assert_eq!(selected(cx), Some(1));
}

/// §7.5: with the connections modal open, neither shortcut does anything.
#[gpui::test]
fn the_panel_shortcuts_do_nothing_with_the_connections_modal_open(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_pinned(&workspace, "src/main.rs", cx);
    collapse(&workspace, DockPlacement::Right, cx);
    let modal = workspace.read_with(cx, |workspace, cx| {
        workspace.agents().read(cx).modal().clone()
    });
    cx.update(|window, cx| modal.update(cx, |modal, cx| modal.open_delete(window, cx)));
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |workspace, cx| workspace.is_modal_open(cx)));
    let focused_before = cx.update(|window, cx| window.focused(cx));

    press("ctrl-shift-a", cx);
    press("ctrl-shift-e", cx);
    press("ctrl-l", cx);
    assert!(
        dock_open(&workspace, DockPlacement::Left, cx),
        "el chat sigue igual"
    );
    assert!(
        !dock_open(&workspace, DockPlacement::Right, cx),
        "el árbol sigue oculto"
    );
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        focused_before,
        "el foco no se movió"
    );
    assert!(modal.read_with(cx, |modal, _| modal.is_open()));
}

/// §7.6: without tabs, closing a panel gives the keyboard to the tab area.
#[gpui::test]
fn closing_a_panel_without_tabs_focuses_the_tab_area(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    press("ctrl-shift-a", cx);
    assert!(composer_has_focus(&workspace, cx));

    press("ctrl-shift-a", cx);
    assert!(!dock_open(&workspace, DockPlacement::Left, cx));
    let center_focused = cx.update(|window, cx| {
        use gpui::Focusable as _;
        workspace
            .read(cx)
            .center()
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(center_focused, "el área de pestañas se queda el foco");
    assert_eq!(zone(&workspace, cx), Some(FocusZone::Center));
}

/// §7.3: a tab in Markdown preview gets the keyboard back on its preview,
/// the only thing of it on screen.
#[gpui::test]
fn closing_a_panel_returns_to_the_markdown_preview(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    std::fs::write(dir.path().join("LEEME.md"), "# Hola\n").unwrap();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_pinned(&workspace, "LEEME.md", cx);
    press("ctrl-shift-v", cx);
    let preview = workspace.read_with(cx, |workspace, cx| {
        match &workspace
            .center()
            .read(cx)
            .active_tab()
            .expect("pestaña")
            .content
        {
            TabContent::MarkdownPreview(_, preview) => preview.clone(),
            TabContent::Editor(_) => panic!("la pestaña debería estar en vista previa"),
        }
    });

    press("ctrl-shift-a", cx);
    press("ctrl-shift-a", cx);
    assert!(!dock_open(&workspace, DockPlacement::Left, cx));
    let preview_focused = cx.update(|window, cx| {
        use gpui::Focusable as _;
        preview.read(cx).focus_handle(cx).is_focused(window)
    });
    assert!(preview_focused);
}

/// §9.3: the three zones visible, focus in the editor: `Ctrl+L` → tree →
/// chat → editor.
#[gpui::test]
fn ctrl_l_cycles_editor_tree_chat(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_pinned(&workspace, "src/main.rs", cx);
    assert_eq!(zone(&workspace, cx), Some(FocusZone::Center));

    press("ctrl-l", cx);
    assert_eq!(zone(&workspace, cx), Some(FocusZone::Files));
    press("ctrl-l", cx);
    assert_eq!(zone(&workspace, cx), Some(FocusZone::Chat));
    assert!(composer_has_focus(&workspace, cx));
    press("ctrl-l", cx);
    assert!(editor_has_focus(&workspace, cx));
    // The wheel never opened or closed anything.
    assert!(dock_open(&workspace, DockPlacement::Left, cx));
    assert!(dock_open(&workspace, DockPlacement::Right, cx));
}

/// §9.3: with the chat hidden, `Ctrl+L` goes editor → tree → editor and the
/// chat stays hidden.
#[gpui::test]
fn ctrl_l_skips_the_hidden_chat(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_pinned(&workspace, "src/main.rs", cx);
    collapse(&workspace, DockPlacement::Left, cx);

    press("ctrl-l", cx);
    assert_eq!(zone(&workspace, cx), Some(FocusZone::Files));
    press("ctrl-l", cx);
    assert!(editor_has_focus(&workspace, cx));
    assert!(!dock_open(&workspace, DockPlacement::Left, cx));

    // With both side panels hidden the editor keeps the keyboard.
    collapse(&workspace, DockPlacement::Right, cx);
    press("ctrl-l", cx);
    assert!(editor_has_focus(&workspace, cx));
    assert!(!dock_open(&workspace, DockPlacement::Right, cx));
}

/// §9.3: from the chat's composer, `Ctrl+L` moves on to the editor (no
/// `Chat` binding keeps it in the chat, D8).
#[gpui::test]
fn ctrl_l_leaves_the_composer_for_the_editor(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_pinned(&workspace, "src/main.rs", cx);
    press("ctrl-shift-a", cx);
    assert!(composer_has_focus(&workspace, cx));

    press("ctrl-l", cx);
    assert!(editor_has_focus(&workspace, cx));
}

/// §9.3: `Ctrl+Shift+L` in the editor still selects the line.
#[gpui::test]
fn ctrl_shift_l_still_selects_the_line(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "src/main.rs", cx);
    assert!(editor_has_focus(&workspace, cx));

    press("ctrl-shift-l", cx);
    let selected = center.read_with(cx, |center, cx| {
        center
            .active_tab()
            .expect("pestaña")
            .editor()
            .read(cx)
            .selected_text()
    });
    assert!(selected.starts_with("fn main() {"), "{selected:?}");
    assert_eq!(zone(&workspace, cx), Some(FocusZone::Center));
}

/// §9.3: with a modal open (today the review panel and the connections
/// modal; the file finder joins in E5-F), `Ctrl+L` does not move the focus.
#[gpui::test]
fn ctrl_l_does_nothing_with_a_modal_open(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    open_pinned(&workspace, "src/main.rs", cx);
    press("ctrl-shift-r", cx);
    assert!(workspace.read_with(cx, |workspace, cx| {
        workspace.review().read(cx).is_panel_open()
    }));
    assert!(workspace.read_with(cx, |workspace, cx| workspace.is_modal_open(cx)));

    press("ctrl-l", cx);
    assert!(editor_has_focus(&workspace, cx));
    press("ctrl-shift-a", cx);
    assert!(editor_has_focus(&workspace, cx));
}

/// §9.1: from outside every zone, `Ctrl+L` goes to the chat.
#[gpui::test]
fn ctrl_l_from_outside_every_zone_goes_to_the_chat(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    // The workspace root holds the keyboard at startup.
    assert_eq!(zone(&workspace, cx), None);

    press("ctrl-l", cx);
    assert!(composer_has_focus(&workspace, cx));
}
