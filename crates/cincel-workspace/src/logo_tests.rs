//! Tests of Cincel's own logo (`crate::logo`): the embedded bytes are a
//! valid SVG, and both places that show it — the empty state
//! (`crate::workspace::Workspace::render_empty_state`) and the title bar
//! (`crate::title_menu::Workspace::render_title_bar`) — actually paint it
//! (`debug_selector`/`debug_bounds`, the same mechanism `title_bar_tests.rs`
//! and `click_tests.rs` use).

use cincel_settings::Config;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::project::ProjectOptions;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

/// The first real element of an SVG document, skipping the `<?xml ...?>`
/// declaration and any leading comments (`packaging/icons/dev.cincel.Cincel.svg`
/// carries both, like every icon under `packaging/icons/`) — what "empieza
/// por `<svg`" means for a real, well-formed file rather than a bare
/// fragment.
fn first_element(document: &str) -> &str {
    let mut rest = document.trim_start();
    if let Some(after_decl) = rest.strip_prefix("<?xml")
        && let Some(end) = after_decl.find("?>")
    {
        rest = after_decl[end + 2..].trim_start();
    }
    while let Some(after_comment) = rest.strip_prefix("<!--") {
        let Some(end) = after_comment.find("-->") else {
            break;
        };
        rest = after_comment[end + 3..].trim_start();
    }
    rest
}

/// The embedded bytes are valid UTF-8 and a real SVG document (its root
/// element is `<svg`, not something else or nothing at all).
#[test]
fn the_embedded_logo_is_a_valid_svg() {
    let text = str::from_utf8(crate::logo::SVG).expect("UTF-8");
    assert!(
        first_element(text).starts_with("<svg"),
        "el documento no empieza por «<svg»: {text}"
    );
    assert!(text.contains("</svg>"), "el documento nunca cierra <svg>");
}

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

fn sample_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
    dir
}

fn workspace_window(
    options: WorkspaceOptions,
    cx: &mut TestAppContext,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    (workspace, cx)
}

/// A window with no project shows the empty state, chisel included, at the
/// `debug_selector` `render_empty_state` sets on it.
#[gpui::test]
fn the_empty_state_paints_the_logo(cx: &mut TestAppContext) {
    init_test(cx);
    let options = WorkspaceOptions {
        project: None,
        open_last_project: false,
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (_workspace, cx) = workspace_window(options, cx);
    assert!(
        cx.debug_bounds("empty-state-logo").is_some(),
        "la pantalla vacía tiene que pintar el logo"
    );
}

/// The title bar paints the same logo next to the menu button, with or
/// without a project open.
#[gpui::test]
fn the_title_bar_paints_the_logo(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let options = WorkspaceOptions {
        project: Some(dir.path().to_path_buf()),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (_workspace, cx) = workspace_window(options, cx);
    assert!(
        cx.debug_bounds("titlebar-logo").is_some(),
        "la barra de título tiene que pintar el logo"
    );
    // The existing menu button is still there, untouched, next to it.
    assert!(cx.debug_bounds("titlebar-menu").is_some());
}
