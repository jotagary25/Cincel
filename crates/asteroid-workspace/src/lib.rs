//! asteroid-workspace: ver `docs/specs/modulos/workspace.md`.
//!
//! The GPUI layer that owns the window: the integrated title bar, the
//! three-panel dock, the status bar, the theme and the global commands. The
//! `asteroid` binary only bootstraps; everything visible lives here.

pub mod panels;
pub mod theme;
pub mod window_state;
pub mod workspace;

pub use theme::{Theme, Typography};
pub use window_state::WindowState;
pub use workspace::{FrameCounter, ToggleChat, ToggleTree, Workspace};

use gpui_kit::App;

/// Initializes gpui-kit and the workspace (theme, actions, key bindings).
///
/// Call once, inside `Application::run`, before opening any window.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    workspace::init(cx);
}
