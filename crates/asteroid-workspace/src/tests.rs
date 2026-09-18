//! GPUI tests for the workspace.
//!
//! They need gpui's test harness, which also turns on its leak detector, so
//! the whole module lives behind the `test-support` feature (see
//! `docs/etapas/etapa-0.md`, finding 7):
//!
//! ```text
//! cargo test -p asteroid-workspace --features test-support
//! ```

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

use asteroid_settings::Config;
use gpui::{
    AppContext as _, Context, Entity, Render, Subscription, TestAppContext, VisualTestContext,
    Window,
};
use gpui_kit::component::dock::DockPlacement;
use gpui_kit::div;
use gpui_kit::prelude::*;

use crate::center::TabContent;
use crate::project::{Project, ProjectOptions};
use crate::toast::{ToastKind, Toasts};
use crate::tree_panel::FilesPanel;
use crate::workspace::{Workspace, WorkspaceEvent, WorkspaceOptions};

/// Redirects the XDG directories into a temporary one, so the tests never
/// touch the real `recents.json`, `window.json` or `layout.json`.
///
/// `set_var` is process-wide, so it happens exactly once, before any test has
/// had a chance to read those paths.
fn isolate_state() {
    static GUARD: OnceLock<tempfile::TempDir> = OnceLock::new();
    GUARD.get_or_init(|| {
        let dir = tempfile::tempdir().expect("no se pudo crear el directorio temporal");
        // SAFETY: called once, from the first test to run, before any code in
        // this process has resolved an XDG path.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("XDG_STATE_HOME", dir.path().join("state"));
            std::env::set_var("XDG_DATA_HOME", dir.path().join("data"));
            std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
        }
        dir
    });
}

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
        let TabContent::Editor(editor) = &tab.content;
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
fn a_stage_three_command_only_shows_a_toast(cx: &mut TestAppContext) {
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
            .any(|message| message == "Disponible en Etapa 3"),
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
        config.settings.theme.mode = asteroid_settings::ThemeMode::Dark;
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
    assert_eq!(outcome, asteroid_project::ReloadOutcome::Reloaded);
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
    assert_eq!(outcome, asteroid_project::ReloadOutcome::Conflict);
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
    assert_eq!(outcome, asteroid_project::ReloadOutcome::Deleted);
    center.update(cx, |center, cx| {
        center.apply_buffer_changes(&[change(&path, outcome)], cx)
    });
    assert_eq!(
        center.read_with(cx, |center, _| center.tabs()[0].display_title().to_string()),
        "lib.rs (eliminado)"
    );
}

/// The [`asteroid_project::BufferChange`] the watcher would have produced.
fn change(path: &Path, outcome: asteroid_project::ReloadOutcome) -> asteroid_project::BufferChange {
    asteroid_project::BufferChange {
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
        config.settings.files.autosave = asteroid_settings::Autosave::OnFocusChange;
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
    // Every `workspace::*` command of the default keymap resolves to an
    // action; the `editor::*` and `chat::*` ones are skipped until their
    // crates register them.
    assert!(
        conversion
            .skipped
            .iter()
            .all(|(_, command, _)| !command.starts_with("workspace::")),
        "{:?}",
        conversion.skipped
    );
    assert!(!conversion.bindings.is_empty());
}
