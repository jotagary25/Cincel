//! Per-project layout, persisted between runs.
//!
//! `docs/specs/03-arquitectura.md` §6 puts it in
//! `~/.local/state/asteroid/workspaces/<hash de ruta>/layout.json`: the dock
//! sizes and their collapsed state, the open tabs and the scroll position of
//! each one. The window's own geometry is global and lives in
//! [`crate::window_state`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bumped when the shape of the file changes in a way older versions cannot
/// read; an unknown version is ignored and the defaults are used.
pub const LAYOUT_VERSION: u32 = 1;

/// One side dock.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DockLayout {
    /// Width in logical pixels.
    pub size: f32,
    /// Whether it was expanded.
    pub open: bool,
}

impl DockLayout {
    /// A dock of `size` pixels, open.
    pub const fn new(size: f32) -> Self {
        Self { size, open: true }
    }

    /// Whether the numbers could plausibly describe a dock.
    fn is_sane(&self) -> bool {
        self.size.is_finite() && self.size >= 0. && self.size <= 4_000.
    }
}

/// One open tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabLayout {
    /// Path relative to the project root.
    pub path: PathBuf,
    /// Whether it was a preview tab (italics) rather than a pinned one.
    #[serde(default)]
    pub preview: bool,
    /// Row the file should reopen on.
    #[serde(default)]
    pub scroll: f32,
    /// Cursor position as `[row, column]` in buffer coordinates, from 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<[u32; 2]>,
}

/// Everything remembered about one project's window layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceLayout {
    /// See [`LAYOUT_VERSION`].
    pub version: u32,
    /// The chat dock, on the left.
    pub chat: DockLayout,
    /// The file tree dock, on the right.
    pub tree: DockLayout,
    /// Open tabs, in tab bar order.
    #[serde(default)]
    pub tabs: Vec<TabLayout>,
    /// Index of the active tab in `tabs`.
    #[serde(default)]
    pub active: Option<usize>,
    /// The chat's autonomy mode, one of `review_after` / `ask_before` /
    /// `always_apply` (`01-producto.md` §F5, `docs/etapas/etapa-2.md`).
    #[serde(default = "default_autonomy_id")]
    pub autonomy: String,
}

fn default_autonomy_id() -> String {
    autonomy_to_id(asteroid_acp::AutonomyMode::default()).to_string()
}

/// The stable id an [`asteroid_acp::AutonomyMode`] is stored under.
#[must_use]
pub fn autonomy_to_id(mode: asteroid_acp::AutonomyMode) -> &'static str {
    match mode {
        asteroid_acp::AutonomyMode::ReviewAfter => "review_after",
        asteroid_acp::AutonomyMode::AskBefore => "ask_before",
        asteroid_acp::AutonomyMode::AlwaysApply => "always_apply",
    }
}

/// The [`asteroid_acp::AutonomyMode`] of a stored id, defaulting to
/// [`asteroid_acp::AutonomyMode::ReviewAfter`] for anything unrecognised
/// (an older layout file, a hand edit).
#[must_use]
pub fn autonomy_from_id(id: &str) -> asteroid_acp::AutonomyMode {
    match id {
        "ask_before" => asteroid_acp::AutonomyMode::AskBefore,
        "always_apply" => asteroid_acp::AutonomyMode::AlwaysApply,
        _ => asteroid_acp::AutonomyMode::ReviewAfter,
    }
}

impl Default for WorkspaceLayout {
    fn default() -> Self {
        Self {
            version: LAYOUT_VERSION,
            chat: DockLayout::new(crate::workspace::CHAT_WIDTH),
            tree: DockLayout::new(crate::workspace::TREE_WIDTH),
            tabs: Vec::new(),
            active: None,
            autonomy: default_autonomy_id(),
        }
    }
}

impl WorkspaceLayout {
    /// `~/.local/state/asteroid/workspaces/<hash>/layout.json` for `root`.
    pub fn path_for(root: &Path) -> Option<PathBuf> {
        Some(
            dirs::state_dir()?
                .join("asteroid")
                .join("workspaces")
                .join(project_hash(root))
                .join("layout.json"),
        )
    }

    /// Reads the layout of the project at `root`, or `None` when there is
    /// none, it is unreadable, or it was written by a newer Asteroid.
    pub fn load(root: &Path) -> Option<Self> {
        Self::load_from(&Self::path_for(root)?)
    }

    /// Reads a layout from an explicit file.
    pub fn load_from(path: &Path) -> Option<Self> {
        let raw = std::fs::read_to_string(path).ok()?;
        let layout: Self = serde_json::from_str(&raw)
            .inspect_err(|error| {
                tracing::warn!(path = %path.display(), %error, "no se pudo leer el layout");
            })
            .ok()?;
        if layout.version != LAYOUT_VERSION {
            tracing::info!(
                version = layout.version,
                "layout de otra versión, se usan los valores por defecto"
            );
            return None;
        }
        if !layout.chat.is_sane() || !layout.tree.is_sane() {
            tracing::warn!(?layout, "layout fuera de rango, se ignora");
            return None;
        }
        Some(layout.sanitized())
    }

    /// Writes the layout of the project at `root`.
    pub fn save(&self, root: &Path) -> anyhow::Result<()> {
        let path = Self::path_for(root)
            .ok_or_else(|| anyhow::anyhow!("no hay directorio de estado XDG"))?;
        self.save_to(&path)
    }

    /// Writes the layout to an explicit file, creating its directory.
    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        tracing::debug!(path = %path.display(), tabs = self.tabs.len(), "layout guardado");
        Ok(())
    }

    /// Drops what cannot be restored: an out-of-range active index, and tab
    /// paths that escape the project root.
    fn sanitized(mut self) -> Self {
        self.tabs
            .retain(|tab| tab.path.is_relative() && !tab.path.as_os_str().is_empty());
        self.active = self
            .active
            .filter(|index| *index < self.tabs.len())
            .or(if self.tabs.is_empty() { None } else { Some(0) });
        self
    }
}

/// The directory name of a project: the hex sha256 of its absolute path.
///
/// A hash rather than the path itself because a path can be arbitrarily long,
/// contain separators, and is not a valid directory name on its own.
pub fn project_hash(root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WorkspaceLayout {
        WorkspaceLayout {
            version: LAYOUT_VERSION,
            chat: DockLayout {
                size: 420.,
                open: false,
            },
            tree: DockLayout {
                size: 260.,
                open: true,
            },
            tabs: vec![
                TabLayout {
                    path: PathBuf::from("src/main.rs"),
                    preview: false,
                    scroll: 12.,
                    cursor: Some([11, 4]),
                },
                TabLayout {
                    path: PathBuf::from("Cargo.toml"),
                    preview: true,
                    scroll: 0.,
                    cursor: None,
                },
            ],
            active: Some(1),
            autonomy: "ask_before".to_string(),
        }
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspaces/abc/layout.json");
        let layout = sample();
        layout.save_to(&path).unwrap();
        assert_eq!(WorkspaceLayout::load_from(&path).unwrap(), layout);
    }

    #[test]
    fn a_missing_or_broken_file_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nada.json");
        assert!(WorkspaceLayout::load_from(&missing).is_none());

        let broken = dir.path().join("roto.json");
        std::fs::write(&broken, "{ esto no es json").unwrap();
        assert!(WorkspaceLayout::load_from(&broken).is_none());
    }

    #[test]
    fn a_future_version_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        let mut layout = sample();
        layout.version = LAYOUT_VERSION + 1;
        layout.save_to(&path).unwrap();
        assert!(WorkspaceLayout::load_from(&path).is_none());
    }

    #[test]
    fn absurd_sizes_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        let mut layout = sample();
        layout.chat.size = f32::NAN;
        layout.save_to(&path).unwrap();
        assert!(WorkspaceLayout::load_from(&path).is_none());
    }

    #[test]
    fn an_out_of_range_active_tab_is_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        let mut layout = sample();
        layout.active = Some(9);
        layout.save_to(&path).unwrap();
        assert_eq!(WorkspaceLayout::load_from(&path).unwrap().active, Some(0));
    }

    #[test]
    fn absolute_tab_paths_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        let mut layout = sample();
        layout.tabs.push(TabLayout {
            path: PathBuf::from("/etc/passwd"),
            preview: false,
            scroll: 0.,
            cursor: None,
        });
        layout.save_to(&path).unwrap();
        let restored = WorkspaceLayout::load_from(&path).unwrap();
        assert_eq!(restored.tabs.len(), 2);
    }

    #[test]
    fn the_hash_is_stable_and_path_dependent() {
        let one = project_hash(Path::new("/home/alguien/proyecto"));
        let two = project_hash(Path::new("/home/alguien/proyecto"));
        let other = project_hash(Path::new("/home/alguien/otro"));
        assert_eq!(one, two);
        assert_ne!(one, other);
        assert_eq!(one.len(), 64);
    }
}
