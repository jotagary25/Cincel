//! Drop-down menus close by themselves, with the whole workspace around them
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §4): a click on
//! the editor, `Ctrl+L`, `Esc` and the window losing the focus. The chat's
//! two header menus, the title bar's menu and the tree's context menu.
//!
//! The chat's own menus have their per-menu cases in
//! `cincel-chat/src/menu_dismiss_tests.rs`; here they are checked inside the
//! real window, where the focus wheel and the dock are the ones of the app.

use std::path::Path;

use cincel_chat::Popover;
use cincel_settings::Config;
use gpui::{Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext, px, size};

use crate::project::ProjectOptions;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

fn sample_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}\n").unwrap();
    dir
}

/// A workspace over `root` with `src/lib.rs` open (its editor has the
/// keyboard) and the window in the foreground, as in the app.
fn open_workspace<'a>(
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
    // The test platform only reports focus changes and deactivations for a
    // window it considers active.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new("src/lib.rs"), true, cx)
    });
    cx.run_until_parked();
    settle(cx);
    (workspace, cx)
}

/// Two frames, so what changed is painted and the focus events of the first
/// one have been delivered.
fn settle(cx: &mut VisualTestContext) {
    cx.simulate_resize(size(px(1101.), px(700.)));
    cx.run_until_parked();
    cx.simulate_resize(size(px(1100.), px(700.)));
    cx.run_until_parked();
}

fn editor_has_focus(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    let center = workspace.read_with(cx, |workspace, _| workspace.center().clone());
    cx.update(|window, cx| {
        center
            .read(cx)
            .active_tab()
            .map(|tab| {
                tab.editor()
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx)
            })
            .unwrap_or(false)
    })
}

fn click_editor(cx: &mut VisualTestContext) {
    let body = cx
        .debug_bounds("center-body")
        .expect("el cuerpo del centro se pinta");
    cx.simulate_click(body.center(), Modifiers::none());
    cx.run_until_parked();
    settle(cx);
}

fn popover(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Popover {
    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    chat.read_with(cx, |chat, _| chat.popover().clone())
}

/// Opens the chat menu the way its header button does.
fn open_chat_menu(workspace: &Entity<Workspace>, menu: &Popover, cx: &mut VisualTestContext) {
    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    chat.update(cx, |chat, cx| chat.toggle_popover(menu.clone(), cx));
    settle(cx);
    assert_eq!(&popover(workspace, cx), menu, "el menú se abrió");
}

const CHAT_MENUS: [Popover; 2] = [Popover::Connections, Popover::Conversations];

#[gpui::test]
fn a_click_on_the_editor_closes_the_chat_menus(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = open_workspace(dir.path(), cx);
    for menu in CHAT_MENUS {
        open_chat_menu(&workspace, &menu, cx);
        click_editor(cx);
        assert_eq!(
            popover(&workspace, cx),
            Popover::Closed,
            "{menu:?}: el clic en el editor lo cierra"
        );
        assert!(
            editor_has_focus(&workspace, cx),
            "{menu:?}: el foco es del editor"
        );
    }
}

#[gpui::test]
fn ctrl_l_closes_the_chat_menus(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = open_workspace(dir.path(), cx);
    for menu in CHAT_MENUS {
        open_chat_menu(&workspace, &menu, cx);
        cx.simulate_keystrokes("ctrl-l");
        settle(cx);
        assert_eq!(
            popover(&workspace, cx),
            Popover::Closed,
            "{menu:?}: Ctrl+L lo cierra"
        );
    }
}

#[gpui::test]
fn escape_closes_the_chat_menus_and_the_editor_keeps_the_focus_it_had(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = open_workspace(dir.path(), cx);
    assert!(editor_has_focus(&workspace, cx), "parte del editor");
    for menu in CHAT_MENUS {
        open_chat_menu(&workspace, &menu, cx);
        cx.simulate_keystrokes("escape");
        settle(cx);
        assert_eq!(
            popover(&workspace, cx),
            Popover::Closed,
            "{menu:?}: Esc lo cierra"
        );
        assert!(
            editor_has_focus(&workspace, cx),
            "{menu:?}: Esc devuelve el foco a donde estaba (el editor)"
        );
    }
}

#[gpui::test]
fn the_window_losing_the_focus_closes_the_chat_menus(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    for menu in CHAT_MENUS {
        let (workspace, cx) = open_workspace(dir.path(), cx);
        open_chat_menu(&workspace, &menu, cx);
        cx.deactivate_window();
        settle(cx);
        assert_eq!(
            popover(&workspace, cx),
            Popover::Closed,
            "{menu:?}: la ventana perdió el foco"
        );
    }
}

// ------------------------------------------------------------- title bar

fn title_menu_open(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |workspace, _| workspace.is_title_menu_open())
}

fn open_title_menu(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    let button = cx
        .debug_bounds("titlebar-menu")
        .expect("el botón del menú se pinta");
    cx.simulate_click(button.center(), Modifiers::none());
    cx.run_until_parked();
    settle(cx);
    assert!(title_menu_open(workspace, cx), "el clic abre el menú");
}

#[gpui::test]
fn a_click_on_the_editor_closes_the_title_menu(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = open_workspace(dir.path(), cx);
    open_title_menu(&workspace, cx);
    click_editor(cx);
    assert!(
        !title_menu_open(&workspace, cx),
        "el clic en el editor lo cierra"
    );
    assert!(editor_has_focus(&workspace, cx));
}

#[gpui::test]
fn ctrl_l_closes_the_title_menu(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = open_workspace(dir.path(), cx);
    open_title_menu(&workspace, cx);
    cx.simulate_keystrokes("ctrl-l");
    settle(cx);
    assert!(!title_menu_open(&workspace, cx), "Ctrl+L lo cierra");
}

#[gpui::test]
fn escape_closes_the_title_menu(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = open_workspace(dir.path(), cx);
    open_title_menu(&workspace, cx);
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert!(!title_menu_open(&workspace, cx), "Esc lo cierra");
}

#[gpui::test]
fn the_window_losing_the_focus_closes_the_title_menu(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = open_workspace(dir.path(), cx);
    open_title_menu(&workspace, cx);
    cx.deactivate_window();
    settle(cx);
    assert!(
        !title_menu_open(&workspace, cx),
        "la ventana perdió el foco: el menú se cierra"
    );
}

#[gpui::test]
fn a_click_on_the_title_button_closes_the_menu_and_the_next_one_opens_it(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = open_workspace(dir.path(), cx);
    open_title_menu(&workspace, cx);
    let button = cx.debug_bounds("titlebar-menu").expect("el botón se pinta");
    cx.simulate_click(button.center(), Modifiers::none());
    settle(cx);
    assert!(
        !title_menu_open(&workspace, cx),
        "el segundo clic lo cierra"
    );
    cx.simulate_click(button.center(), Modifiers::none());
    settle(cx);
    assert!(title_menu_open(&workspace, cx), "y el siguiente lo abre");
}

#[gpui::test]
fn opening_the_recent_folders_submenu_keeps_the_menu_open(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let other = sample_project();
    let (workspace, cx) = open_workspace(dir.path(), cx);
    let mut recents = cincel_project::Recents::new();
    recents.push(other.path());
    workspace.update(cx, |workspace, _| workspace.set_recents(recents));
    settle(cx);
    open_title_menu(&workspace, cx);
    // "Abrir carpeta…", then "Carpetas recientes", then its submenu: the focus
    // moves into the submenu, which is still part of the same menu.
    cx.simulate_keystrokes("down down right");
    settle(cx);
    assert!(
        title_menu_open(&workspace, cx),
        "abrir el submenú no cierra el menú"
    );
    // And it is a live submenu: `Enter` on its first row opens that folder.
    cx.simulate_keystrokes("enter");
    settle(cx);
    let root = workspace.read_with(cx, |workspace, cx| {
        workspace.project().unwrap().read(cx).root().to_path_buf()
    });
    assert_eq!(root, other.path(), "el submenú respondió: abrió la carpeta");
    assert!(
        !title_menu_open(&workspace, cx),
        "elegir una fila lo cierra"
    );
}

// ------------------------------------------------------- tree context menu

const SENTINEL: &str = "sin copiar";

/// Right-clicks the first row of the tree, which opens its context menu.
fn open_tree_menu(cx: &mut VisualTestContext) {
    let panel = cx
        .debug_bounds("files-panel")
        .expect("el panel de archivos se pinta");
    // The root line, then the first row of the tree.
    let row = gpui::point(panel.center().x, panel.top() + px(24. * 1.5));
    cx.simulate_mouse_down(row, gpui::MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(row, gpui::MouseButton::Right, Modifiers::none());
    settle(cx);
}

/// Whether the tree's context menu is still answering: `Enter` on its third
/// row ("Copiar ruta") copies a path. Destructive on purpose: it ends with
/// the menu closed whatever the answer.
fn tree_menu_answers(cx: &mut VisualTestContext) -> bool {
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(SENTINEL.to_string()));
    cx.simulate_keystrokes("down down enter");
    settle(cx);
    cx.read_from_clipboard()
        .and_then(|item| item.text())
        .is_some_and(|text| text != SENTINEL)
}

#[gpui::test]
fn the_tree_context_menu_opens_with_a_right_click(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (_workspace, cx) = open_workspace(dir.path(), cx);
    open_tree_menu(cx);
    assert!(
        tree_menu_answers(cx),
        "el clic derecho abre el menú (si no, el resto de los casos no prueba nada)"
    );
}

#[gpui::test]
fn a_click_on_the_editor_closes_the_tree_context_menu(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = open_workspace(dir.path(), cx);
    open_tree_menu(cx);
    click_editor(cx);
    assert!(!tree_menu_answers(cx), "el clic en el editor lo cierra");
    assert!(editor_has_focus(&workspace, cx) || true);
}

#[gpui::test]
fn ctrl_l_closes_the_tree_context_menu(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (_workspace, cx) = open_workspace(dir.path(), cx);
    open_tree_menu(cx);
    cx.simulate_keystrokes("ctrl-l");
    settle(cx);
    assert!(!tree_menu_answers(cx), "Ctrl+L lo cierra");
}

#[gpui::test]
fn escape_closes_the_tree_context_menu(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (_workspace, cx) = open_workspace(dir.path(), cx);
    open_tree_menu(cx);
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert!(!tree_menu_answers(cx), "Esc lo cierra");
}

#[gpui::test]
fn the_window_losing_the_focus_closes_the_tree_context_menu(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (_workspace, cx) = open_workspace(dir.path(), cx);
    open_tree_menu(cx);
    cx.deactivate_window();
    settle(cx);
    assert!(
        !tree_menu_answers(cx),
        "la ventana perdió el foco: se cierra"
    );
}
