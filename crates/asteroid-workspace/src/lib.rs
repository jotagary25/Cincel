//! asteroid-workspace: ver `docs/specs/modulos/workspace.md`.
//!
//! The GPUI layer that owns the window: the integrated title bar, the
//! three-panel dock (chat, tabs, file tree), the status bar, the toasts, the
//! theme, the keymap and the global commands. The `asteroid` binary only
//! parses the command line and boots; everything visible lives here.
//!
//! ```no_run
//! use asteroid_workspace::{Workspace, WorkspaceOptions};
//!
//! # fn main() {
//! let config = asteroid_settings::Config::load();
//! gpui_kit::application().run(move |cx| {
//!     asteroid_workspace::init(config.value, cx);
//!     Workspace::open_window(WorkspaceOptions::default(), cx).detach();
//! });
//! # }
//! ```
//!
//! Comments and documentation are in English; everything the user reads is in
//! Spanish, like the rest of Asteroid.

pub mod actions;
pub mod agents;
pub mod center;
pub mod conversations;
pub mod keymap;
pub mod layout;
pub mod panels;
pub mod project;
pub mod settings;
pub mod theme;
pub mod toast;
pub mod tree_panel;
pub mod window_state;
pub mod workspace;

#[cfg(all(test, feature = "test-support"))]
mod tests;

pub use agents::Agents;
pub use center::{CenterPanel, Tab, TabContent};
pub use conversations::{ConversationStore, IndexEntry};
pub use layout::WorkspaceLayout;
pub use panels::ChatDock;
pub use project::{Project, ProjectEvent};
pub use settings::AppSettings;
pub use theme::{SyntaxColors, ThemeColors, Typography};
pub use toast::{ToastKind, Toasts};
pub use tree_panel::FilesPanel;
pub use window_state::WindowState;
pub use workspace::{FrameCounter, Workspace, WorkspaceEvent, WorkspaceOptions};

use asteroid_settings::Config;
use gpui_kit::App;

/// Initializes gpui-kit and the workspace: theme, typography and keymap from
/// `config`.
///
/// Call once, inside `Application::run`, before opening any window.
pub fn init(config: Config, cx: &mut App) {
    gpui_kit::init(cx);
    // Whatever gpui-kit just bound is the base a keymap reload rebuilds on.
    keymap::snapshot_base_bindings(cx);
    settings::install(config, cx);
}
