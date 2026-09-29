//! cincel-workspace: ver `docs/specs/modulos/workspace.md`.
//!
//! The GPUI layer that owns the window: the integrated title bar, the
//! three-panel dock (chat, tabs, file tree), the status bar, the toasts, the
//! theme, the keymap and the global commands. The `cincel` binary only
//! parses the command line and boots; everything visible lives here.
//!
//! ```no_run
//! use cincel_workspace::{Workspace, WorkspaceOptions};
//!
//! # fn main() {
//! let config = cincel_settings::Config::load();
//! gpui_kit::application().run(move |cx| {
//!     cincel_workspace::init(config.value, cx);
//!     Workspace::open_window(WorkspaceOptions::default(), cx).detach();
//! });
//! # }
//! ```
//!
//! Comments and documentation are in English; everything the user reads is in
//! Spanish, like the rest of Cincel.

pub mod actions;
pub mod agents;
pub mod center;
pub mod connection_modal;
pub mod conversations;
pub mod file_finder;
pub mod focus;
pub mod keymap;
pub mod layout;
pub mod markdown_preview;
pub mod new_file;
pub mod panels;
pub mod project;
pub mod review;
pub mod review_close;
pub mod settings;
pub mod settings_view;
pub mod shortcuts_modal;
pub mod theme;
pub mod title_menu;
pub mod toast;
pub mod tree_panel;
pub mod window_state;
pub mod workspace;

#[cfg(all(test, feature = "test-support"))]
mod appearance_tests;
#[cfg(all(test, feature = "test-support"))]
mod autosave_tests;
#[cfg(all(test, feature = "test-support"))]
mod connection_cancel_tests;
#[cfg(all(test, feature = "test-support"))]
mod deleted_file_review_tests;
#[cfg(all(test, feature = "test-support"))]
mod file_finder_tests;
#[cfg(all(test, feature = "test-support"))]
mod first_open_tests;
#[cfg(all(test, feature = "test-support"))]
mod git_gutter_tests;
#[cfg(all(test, feature = "test-support"))]
mod modal_scroll_tests;
#[cfg(all(test, feature = "test-support"))]
mod pending_review_close_tests;
#[cfg(all(test, feature = "test-support"))]
mod review_counter_tests;
#[cfg(all(test, feature = "test-support"))]
mod review_tests;
#[cfg(all(test, feature = "test-support"))]
mod settings_view_tests;
#[cfg(all(test, feature = "test-support"))]
mod shortcuts_modal_tests;
#[cfg(all(test, feature = "test-support"))]
mod snapshot_review_tests;
#[cfg(all(test, feature = "test-support"))]
mod test_support;
#[cfg(all(test, feature = "test-support"))]
mod tests;
#[cfg(all(test, feature = "test-support"))]
mod title_bar_tests;

pub use agents::Agents;
pub use center::{CenterItem, CenterPanel, Tab, TabContent};
pub use connection_modal::{ConnectionsModal, ModalEvent};
pub use conversations::{ConversationStore, IndexEntry};
pub use focus::{FocusZone, VisibleZones, next_zone};
pub use layout::WorkspaceLayout;
pub use markdown_preview::MarkdownPreviewView;
pub use panels::ChatDock;
pub use project::{Project, ProjectEvent};
pub use review::{DirtyChoice, Review, ReviewSummary};
pub use settings::AppSettings;
pub use settings_view::{SettingsSection, SettingsView, SettingsViewEvent};
pub use theme::{SyntaxColors, ThemeColors, Typography};
pub use toast::{ToastKind, Toasts};
pub use tree_panel::FilesPanel;
pub use window_state::WindowState;
pub use workspace::{FrameCounter, Workspace, WorkspaceEvent, WorkspaceOptions};

use cincel_settings::Config;
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
