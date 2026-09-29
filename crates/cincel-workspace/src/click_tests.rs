//! Real coordinate clicks on the title bar's and the status bar's buttons
//! (`docs/specs/08-etapa6-cierre-1-0.md` §5.6.2, D16): `simulate_click` over
//! the center of each button's `debug_bounds`, then a check of the visible
//! effect — never a direct call to the handler, which is what every other
//! test module already does and is exactly the gap this file closes.
//!
//! Kept out of `tests.rs` (a new file, per `08-etapa6-cierre-1-0.md` §10.1).
//! Needs `gpui-kit/test-support` (through `cincel-workspace`'s own
//! `test-support` feature, D16) for the click on a gpui-kit `Button` (the
//! title menu) to actually register instead of only being paintable.
//!
//! What is *not* here, and why: the window's own `×` has no `debug_selector`
//! to click — gpui-kit only paints client-side-decoration window controls,
//! and GPUI's test window always reports server-side decorations, so the
//! button is never painted in a test at all (the spec itself anticipates
//! this, §5.6.2). It stays covered the way it already was, by calling
//! `Workspace::close_from_title_bar` directly (`tests.rs`). The status bar's
//! "atajos" and "pendientes" buttons (`crate::workspace::render_status_bar`)
//! have no `debug_selector` either, and adding one means editing
//! `workspace.rs`, which this wave's rules reserve for E6-A — see the
//! deviation noted in this sub-stage's final report.

use std::path::Path;

use cincel_chat::Popover;
use cincel_settings::Config;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use gpui_kit::component::dock::DockPlacement;

use crate::project::ProjectOptions;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

fn sample_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
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

/// Clicks the center of `selector`'s painted bounds.
fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("«{selector}» no se pintó"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
}

/// `Workspace::toggle_focus` (`focus.rs`) only *closes* an already open dock
/// when its zone already has the keyboard; otherwise it just moves the
/// keyboard there, open or not. Pressing the keyboard toggle twice reaches
/// "closed" from either starting point: closed → open+focused → closed;
/// open+unfocused → open+focused → closed.
fn ensure_closed(
    keys: &str,
    placement: DockPlacement,
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) {
    press(keys, cx);
    press(keys, cx);
    assert!(
        !workspace.read_with(cx, |workspace, cx| workspace.is_dock_open(placement, cx)),
        "dos togglés seguidos tienen que dejarlo cerrado"
    );
}

/// §5.6.2: a real click on "titlebar-menu" opens the title bar's menu, the
/// same `Workspace::is_title_menu_open` the `Esc`/outside-click tests read.
#[gpui::test]
fn clicking_the_title_menu_button_opens_it(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    assert!(!workspace.read_with(cx, |workspace, _| workspace.is_title_menu_open()));
    click("titlebar-menu", cx);
    assert!(
        workspace.read_with(cx, |workspace, _| workspace.is_title_menu_open()),
        "el clic tiene que abrir el menú"
    );
}

/// §5.6.2: a real click on "titlebar-toggle-chat" toggles the left dock,
/// exactly like `Ctrl+Shift+A`.
#[gpui::test]
fn clicking_the_chat_toggle_button_flips_the_left_dock(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    ensure_closed("ctrl-shift-a", DockPlacement::Left, &workspace, cx);

    click("titlebar-toggle-chat", cx);

    assert!(
        workspace.read_with(cx, |workspace, cx| workspace
            .is_dock_open(DockPlacement::Left, cx)),
        "el clic tiene que abrir el chat"
    );
}

/// §5.6.2: a real click on "titlebar-toggle-files" toggles the right dock.
#[gpui::test]
fn clicking_the_files_toggle_button_flips_the_right_dock(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);
    ensure_closed("ctrl-shift-e", DockPlacement::Right, &workspace, cx);

    click("titlebar-toggle-files", cx);

    assert!(
        workspace.read_with(cx, |workspace, cx| workspace
            .is_dock_open(DockPlacement::Right, cx)),
        "el clic tiene que abrir los archivos"
    );
}

/// §5.6.2: a real click on the status bar's connection chip
/// ("status-connection") opens the chat's "Conectar" popover, expanding the
/// chat dock first if it was collapsed.
#[gpui::test]
fn clicking_the_status_bar_connection_chip_opens_the_connect_popover(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    // Collapse the chat dock first, so the click also has to reopen it.
    ensure_closed("ctrl-shift-a", DockPlacement::Left, &workspace, cx);

    click("status-connection", cx);

    assert!(workspace.read_with(cx, |workspace, cx| {
        workspace.is_dock_open(DockPlacement::Left, cx)
    }));
    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    assert_eq!(
        chat.read_with(cx, |chat, _| chat.popover().clone()),
        Popover::Connections
    );
}
