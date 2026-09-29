//! Tests of the title bar's menu, "Nuevo archivo", the quit dialog and the
//! panel toggle buttons (`crate::title_menu`, `crate::new_file`,
//! `docs/specs/07-etapa5-productividad.md` §8.5).
//!
//! Kept out of `tests.rs` (E5-I only touches new files while E5-J works on
//! `cincel-workspace` in parallel): the pure half needs no `App` at all, and
//! the `TestAppContext` half follows the same `workspace_window` / `press`
//! pattern the rest of the crate's test modules use, rebuilt locally since
//! `tests.rs`'s own helpers are private to that module.

use std::path::Path;

use cincel_settings::Config;
use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use gpui_kit::component::dock::DockPlacement;

use crate::focus::FocusZone;
use crate::new_file::{NewFilePrompt, validate_new_file_name};
use crate::project::ProjectOptions;
use crate::test_support::isolate_state;
use crate::title_menu::recent_label_with_home;
use crate::workspace::{Workspace, WorkspaceOptions};

// ------------------------------------------------------------- pure tests

/// §8.5: "validación del nombre de 'Nuevo archivo' (vacío, `..`, absoluto,
/// subcarpetas)".
#[test]
fn validate_new_file_name_covers_the_whole_table() {
    assert!(validate_new_file_name("").is_err());
    assert!(validate_new_file_name("   ").is_err());
    assert_eq!(
        validate_new_file_name("").unwrap_err(),
        "El nombre no puede estar vacío"
    );

    assert_eq!(
        validate_new_file_name("../fuera.rs").unwrap_err(),
        "El archivo tiene que quedar dentro del proyecto"
    );
    assert_eq!(
        validate_new_file_name("/etc/passwd").unwrap_err(),
        "El archivo tiene que quedar dentro del proyecto"
    );
    assert_eq!(
        validate_new_file_name("a/../../fuera.rs").unwrap_err(),
        "El archivo tiene que quedar dentro del proyecto"
    );

    assert_eq!(
        validate_new_file_name("nota.md").unwrap(),
        Path::new("nota.md")
    );
    assert_eq!(
        validate_new_file_name("a/b.rs").unwrap(),
        Path::new("a/b.rs")
    );
}

/// §8.5: "etiqueta de reciente con `~`".
#[test]
fn recent_label_folds_the_home_directory() {
    let home = Path::new("/home/ana");
    assert_eq!(
        recent_label_with_home(Path::new("/home/ana/asteroid"), Some(home)),
        "~/asteroid"
    );
    assert_eq!(recent_label_with_home(home, Some(home)), "~");
    assert_eq!(
        recent_label_with_home(Path::new("/srv/otro"), Some(home)),
        "/srv/otro"
    );
    assert_eq!(
        recent_label_with_home(Path::new("/home/ana/asteroid"), None),
        "/home/ana/asteroid"
    );
}

// ------------------------------------------------------------- GPUI tests

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

/// `src/main.rs` and `src/lib.rs`, plus a `docs/` folder — enough to give
/// "Nuevo archivo" a subfolder to nest under and the tree something to
/// select.
fn sample_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "\n").unwrap();
    std::fs::create_dir_all(dir.path().join("docs")).unwrap();
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

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
}

fn new_file_prompt(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Entity<NewFilePrompt> {
    workspace
        .read_with(cx, |workspace, _| workspace.new_file_prompt().cloned())
        .expect("el proyecto ya construyó el campo (Workspace::open_project)")
}

/// Opens `relative` from the tree, pinned, and types `text` into its editor
/// so the tab counts as dirty (same recipe `tests.rs` uses).
fn dirty_a_tab(
    workspace: &Entity<Workspace>,
    relative: &str,
    text: &str,
    cx: &mut VisualTestContext,
) {
    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new(relative), true, cx)
    });
    cx.run_until_parked();
    cx.simulate_input(text);
    cx.run_until_parked();
}

/// §8.4: "'Carpetas recientes' lista las recientes; elegir una la abre en la
/// misma ventana."
#[gpui::test]
fn choosing_a_recent_folder_replaces_the_open_project(cx: &mut TestAppContext) {
    init_test(cx);
    let dir_a = sample_project();
    let dir_b = sample_project();
    let (workspace, cx) = workspace_window(dir_a.path(), cx);

    let mut recents = cincel_project::Recents::new();
    recents.push(dir_b.path());
    workspace.update(cx, |workspace, _| workspace.set_recents(recents));

    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_recent(dir_b.path(), window, cx)
        })
    });
    cx.run_until_parked();

    let root = workspace.read_with(cx, |workspace, cx| {
        workspace.project().unwrap().read(cx).root().to_path_buf()
    });
    assert_eq!(root, dir_b.path());
}

/// §8.4: "'Nuevo archivo…' con `docs/nota.md` crea la carpeta y el archivo y
/// lo abre fijado; repetir dice 'Ya existe «docs/nota.md»'." `Ctrl+N` opens
/// the field over the project root (nothing selected in the tree), so a
/// typed subfolder is what creates `docs/nota.md` here.
#[gpui::test]
fn ctrl_n_creates_and_opens_the_file_pinned(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    press("ctrl-n", cx);
    let prompt = new_file_prompt(&workspace, cx);
    assert!(prompt.read_with(cx, |prompt, _| prompt.is_open()));

    cx.simulate_input("docs/nota.md");
    press("enter", cx);

    assert!(!prompt.read_with(cx, |prompt, _| prompt.is_open()));
    assert!(dir.path().join("docs/nota.md").is_file());

    let center = workspace.read_with(cx, |workspace, _| workspace.center().clone());
    let (path, preview) = center.read_with(cx, |center, _| {
        let tab = center.active_tab().expect("la pestaña quedó activa");
        (tab.path.clone(), tab.preview)
    });
    assert_eq!(path, dir.path().join("docs/nota.md"));
    assert!(!preview, "Ctrl+N abre fijado, no en previsualización");

    let editor_focused = cx.update(|window, cx| {
        center
            .read(cx)
            .active_tab()
            .unwrap()
            .editor()
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
    });
    assert!(editor_focused, "el foco tiene que quedar en el editor");

    // Repeating it says "Ya existe «docs/nota.md»" (§8.2) and does not
    // touch the file or leave the field.
    press("ctrl-n", cx);
    cx.simulate_input("docs/nota.md");
    press("enter", cx);
    let error = prompt.read_with(cx, |prompt, _| prompt.error().map(ToString::to_string));
    assert_eq!(error.as_deref(), Some("Ya existe «docs/nota.md»"));
    assert!(prompt.read_with(cx, |prompt, _| prompt.is_open()));
}

/// §8.2: a path leaving the project (`..`) is rejected and nothing is
/// created.
#[gpui::test]
fn a_path_outside_the_project_is_rejected(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    press("ctrl-n", cx);
    let prompt = new_file_prompt(&workspace, cx);
    cx.simulate_input("../fuera.rs");
    press("enter", cx);

    let error = prompt.read_with(cx, |prompt, _| prompt.error().map(ToString::to_string));
    assert_eq!(
        error.as_deref(),
        Some("El archivo tiene que quedar dentro del proyecto")
    );
    assert!(prompt.read_with(cx, |prompt, _| prompt.is_open()));
    assert!(!dir.path().parent().unwrap().join("fuera.rs").exists());
}

/// §8.4: "`Ctrl+Q` con un archivo sucio muestra el diálogo; 'Cancelar' no
/// cierra."
#[gpui::test]
fn ctrl_q_with_a_dirty_file_opens_the_dialog_and_cancel_keeps_it_open(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    dirty_a_tab(&workspace, "src/lib.rs", "// sucio\n", cx);

    press("ctrl-q", cx);
    assert!(workspace.read_with(cx, |workspace, _| workspace.is_quit_dialog_open()));

    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.cancel_quit_for_test(window, cx)
        })
    });
    cx.run_until_parked();

    assert!(!workspace.read_with(cx, |workspace, _| workspace.is_quit_dialog_open()));
    // The window is still here and the change is still there to ask about
    // again: "Cancelar" neither closed nor discarded anything.
    let center = workspace.read_with(cx, |workspace, _| workspace.center().clone());
    let still_dirty = center.read_with(cx, |center, cx| center.is_tab_dirty(0, cx));
    assert!(still_dirty, "Cancelar no tiene que descartar el cambio");
}

/// §8.4/§8.5: "la `×` con un archivo sucio muestra el mismo diálogo" —
/// simulated as the harness allows, on `Workspace::should_close`, the exact
/// function `Workspace::open_window`'s `on_window_should_close` calls.
#[gpui::test]
fn the_window_close_hook_asks_the_same_question_as_ctrl_q(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    dirty_a_tab(&workspace, "src/lib.rs", "// sucio\n", cx);

    let may_close = cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| workspace.should_close(window, cx))
    });
    assert!(!may_close, "con un archivo sucio la ventana no cierra sola");
    assert!(workspace.read_with(cx, |workspace, _| workspace.is_quit_dialog_open()));

    // Once the dialog already confirmed (`quit_confirmed`), `should_close`
    // answers `true` without asking again — the real confirm button sets
    // this flag itself and then closes the window, which a unit test
    // cannot do and keep going, so this pokes the flag directly (`pub(crate)`,
    // same crate) to exercise that branch in isolation.
    workspace.update(cx, |workspace, _| workspace.set_quit_confirmed(true));
    let may_close_now = cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| workspace.should_close(window, cx))
    });
    assert!(
        may_close_now,
        "una vez confirmado, should_close ya no vuelve a preguntar"
    );
}

/// The title bar's `×` (`TitleBar::on_close_window` →
/// `Workspace::close_from_title_bar`) asks like `Alt+F4` does: with a dirty
/// file the dialog opens and the window stays; with nothing to ask about the
/// window goes away. gpui-kit only draws its window controls under
/// client-side decorations, and GPUI's test window always reports
/// server-side ones, so the button itself is not on screen here: this calls
/// the exact function its click handler calls.
#[gpui::test]
fn the_title_bar_close_button_asks_before_closing(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    dirty_a_tab(&workspace, "src/lib.rs", "// sucio\n", cx);

    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.close_from_title_bar(window, cx)
        })
    });
    cx.run_until_parked();
    assert!(
        workspace.read_with(cx, |workspace, _| workspace.is_quit_dialog_open()),
        "la × pregunta por el archivo sin guardar"
    );
    assert_eq!(cx.windows().len(), 1, "la ventana sigue abierta");

    // "Salir sin guardar" already answered: the next `×` closes for real.
    workspace.update(cx, |workspace, _| workspace.set_quit_confirmed(true));
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.close_from_title_bar(window, cx)
        })
    });
    cx.run_until_parked();
    assert!(
        cx.windows().is_empty(),
        "sin nada que preguntar, la × cierra"
    );
}

/// §8.4: "sin archivos sucios cierra directamente como hoy."
#[gpui::test]
fn should_close_is_true_right_away_without_unsaved_files(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    let may_close = cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| workspace.should_close(window, cx))
    });
    assert!(may_close);
    assert!(!workspace.read_with(cx, |workspace, _| workspace.is_quit_dialog_open()));
}

/// §8.4: "Los botones de chat y archivos siguen la regla de §7.1 y su icono
/// cambia." The click handler calls exactly `Workspace::toggle_focus`
/// (`crate::title_menu::render_panel_toggle`); clicking through a real
/// pixel hit-test would need `gpui-kit`'s `test-support` feature, which this
/// crate's `Cargo.toml` does not enable (out of scope for E5-I to add), so
/// this exercises the same call the button's `on_click` makes.
#[gpui::test]
fn the_panel_buttons_follow_the_focus_rule(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    let chat_open_before = workspace.read_with(cx, |workspace, cx| {
        workspace.is_dock_open(DockPlacement::Left, cx)
    });
    assert!(chat_open_before);

    // §7.1, case 3: visible but the focus is elsewhere (nothing is open
    // yet) → the button's click focuses it without closing anything.
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_focus(FocusZone::Chat, window, cx)
        })
    });
    cx.run_until_parked();
    assert!(
        workspace.read_with(cx, |workspace, cx| workspace
            .is_dock_open(DockPlacement::Left, cx)),
        "el primer clic solo enfoca, no cierra"
    );

    // §7.1, case 2: visible with the focus now inside → the same click
    // closes it and gives the keyboard back to the center.
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_focus(FocusZone::Chat, window, cx)
        })
    });
    cx.run_until_parked();
    assert!(
        !workspace.read_with(cx, |workspace, cx| workspace
            .is_dock_open(DockPlacement::Left, cx)),
        "el chat visible + foco dentro se cierra"
    );
}

/// §9.3-style check (like `file_finder_tests`'s and `shortcuts_modal_tests`'s
/// equivalents): with the "Nuevo archivo" field open, `Ctrl+L` does nothing
/// (§7.4, "modal abierto").
#[gpui::test]
fn ctrl_l_does_nothing_with_the_new_file_field_open(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    press("ctrl-n", cx);
    let prompt = new_file_prompt(&workspace, cx);
    assert!(prompt.read_with(cx, |prompt, _| prompt.is_open()));
    assert!(workspace.read_with(cx, |workspace, cx| workspace.is_modal_open(cx)));

    press("ctrl-l", cx);
    assert!(
        prompt.read_with(cx, |prompt, _| prompt.is_open()),
        "Ctrl+L no tiene que cerrar el campo"
    );
    let field_focused = cx.update(|window, cx| {
        prompt
            .read(cx)
            .input()
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
    });
    assert!(field_focused, "el foco tiene que seguir en el campo");
}
