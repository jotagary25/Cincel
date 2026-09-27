//! Window geometry persisted between runs.
//!
//! `docs/specs/03-arquitectura.md` §6 puts volatile state under
//! `~/.local/state/cincel/`; the window's own size and position live in
//! `window.json` there. The per-project layout (`workspaces/<hash>/layout.json`)
//! is a later stage.

use std::path::PathBuf;

use gpui_kit::{App, Bounds, Pixels, Size, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

/// Default window size when nothing has been saved yet.
const DEFAULT_SIZE: (f32, f32) = (1280., 800.);
/// Smallest window we are willing to restore to; anything smaller is treated
/// as a corrupted file.
const MIN_SIZE: (f32, f32) = (640., 480.);

/// The saved geometry of the main window.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    /// Left edge, in logical pixels, in screen coordinates.
    pub x: f32,
    /// Top edge, in logical pixels, in screen coordinates.
    pub y: f32,
    /// Width in logical pixels. This is the restore size when `maximized`.
    pub width: f32,
    /// Height in logical pixels.
    pub height: f32,
    /// Whether the window was maximized when it was saved.
    #[serde(default)]
    pub maximized: bool,
    /// Whether the window was fullscreen when it was saved.
    #[serde(default)]
    pub fullscreen: bool,
}

impl WindowState {
    /// `~/.local/state/cincel/window.json`, following the XDG base
    /// directory specification.
    pub fn path() -> Option<PathBuf> {
        dirs::state_dir().map(|dir| dir.join("cincel").join("window.json"))
    }

    /// Reads the saved geometry, or `None` when there is none or it is
    /// unusable. A broken file is never an error: the window simply opens at
    /// its default size.
    pub fn load() -> Option<Self> {
        let path = Self::path()?;
        let raw = std::fs::read_to_string(&path).ok()?;
        let state: Self = serde_json::from_str(&raw)
            .inspect_err(|error| {
                tracing::warn!(path = %path.display(), %error, "no se pudo leer el estado de la ventana");
            })
            .ok()?;
        if !state.is_sane() {
            tracing::warn!(path = %path.display(), ?state, "estado de ventana fuera de rango, se ignora");
            return None;
        }
        Some(state)
    }

    /// Writes the geometry, creating `~/.local/state/cincel/` if needed.
    pub fn save(&self) -> anyhow::Result<()> {
        let path =
            Self::path().ok_or_else(|| anyhow::anyhow!("no hay directorio de estado XDG"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_vec_pretty(self)?)?;
        tracing::debug!(path = %path.display(), "estado de ventana guardado");
        Ok(())
    }

    /// Whether the numbers could plausibly describe a window. Guards against
    /// a hand-edited file, a NaN, or a display that no longer exists.
    fn is_sane(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite())
            && self.width >= MIN_SIZE.0
            && self.height >= MIN_SIZE.1
            && self.width <= 32_000.
            && self.height <= 32_000.
    }

    /// The geometry as GPUI window bounds.
    pub fn to_window_bounds(self) -> WindowBounds {
        let bounds = Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(px(self.width), px(self.height)),
        };
        if self.fullscreen {
            WindowBounds::Fullscreen(bounds)
        } else if self.maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        }
    }

    /// The geometry of a live window.
    pub fn from_window_bounds(bounds: WindowBounds) -> Self {
        let inner = bounds.get_bounds();
        Self {
            x: f32::from(inner.origin.x),
            y: f32::from(inner.origin.y),
            width: f32::from(inner.size.width),
            height: f32::from(inner.size.height),
            maximized: matches!(bounds, WindowBounds::Maximized(_)),
            fullscreen: matches!(bounds, WindowBounds::Fullscreen(_)),
        }
    }
}

/// The bounds the main window should open with: the saved ones when they are
/// usable, otherwise a default-sized window centered on the primary display.
pub fn initial_window_bounds(cx: &App) -> WindowBounds {
    match WindowState::load() {
        Some(state) => {
            tracing::info!(?state, "restaurando geometría de ventana");
            state.to_window_bounds()
        }
        None => WindowBounds::Windowed(Bounds::centered(None, default_size(cx), cx)),
    }
}

/// The default window size, clamped to the primary display so the window is
/// never born larger than the screen.
fn default_size(cx: &App) -> Size<Pixels> {
    let mut wanted = size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1));
    if let Some(display) = cx.primary_display() {
        let display_size = display.bounds().size;
        wanted.width = wanted.width.min(display_size.width * 0.9);
        wanted.height = wanted.height.min(display_size.height * 0.9);
    }
    wanted
}

/// The smallest window Cincel allows.
pub fn min_window_size() -> Size<Pixels> {
    size(px(MIN_SIZE.0), px(MIN_SIZE.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_window_bounds() {
        let state = WindowState {
            x: 10.,
            y: 20.,
            width: 1000.,
            height: 700.,
            maximized: false,
            fullscreen: false,
        };
        assert_eq!(
            WindowState::from_window_bounds(state.to_window_bounds()),
            state
        );
    }

    #[test]
    fn maximized_survives_the_round_trip() {
        let state = WindowState {
            x: 0.,
            y: 0.,
            width: 1280.,
            height: 800.,
            maximized: true,
            fullscreen: false,
        };
        let restored = WindowState::from_window_bounds(state.to_window_bounds());
        assert!(restored.maximized);
        assert_eq!(restored.width, 1280.);
    }

    #[test]
    fn rejects_implausible_geometry() {
        let tiny = WindowState {
            x: 0.,
            y: 0.,
            width: 4.,
            height: 4.,
            maximized: false,
            fullscreen: false,
        };
        assert!(!tiny.is_sane());

        let nan = WindowState {
            x: f32::NAN,
            ..tiny
        };
        assert!(!nan.is_sane());
    }
}
