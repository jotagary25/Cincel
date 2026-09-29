//! Embedded Inter and JetBrains Mono fonts (`docs/specs/08-etapa6-cierre-1-0.md` §4).
//!
//! Cincel looks the same on every machine, whether or not the user has these
//! families installed: ten static TTF files (six Inter weights/styles, four
//! JetBrains Mono weights/styles, none of them subset — the OFL treats
//! subsetting as a modification) travel inside the binary and are handed to
//! GPUI's text system once, at startup. Sources, exact versions and SHA-256
//! hashes: `assets/fonts/SOURCES.md`. Both families ship under the SIL Open
//! Font License 1.1; its text travels next to each family
//! (`assets/fonts/inter/OFL.txt`, `assets/fonts/jetbrains-mono/OFL.txt`) and,
//! packaged, under `share/doc/cincel/`.
//!
//! [`register_embedded`] must be called from `main.rs`, before
//! `cincel_workspace::init` and before any window opens, never from `init`
//! itself: the existing test suite measures text against the system fonts it
//! already relies on, and moving font registration into `init` would change
//! what every one of those tests sees. If registration fails, the caller logs
//! it and Cincel falls back to the system fonts the rest of the codebase
//! already tries first ([`crate::theme::Typography`], `cincel_chat`'s
//! `PROSE_FALLBACKS`, `cincel_editor`'s `EditorSettings`/`UI_FONT_FAMILIES`).

use std::borrow::Cow;
use std::time::Instant;

use gpui::App;

/// The UI family the embedded fonts provide (`docs/specs/02-visual.md` §3).
pub const UI_FAMILY: &str = "Inter";
/// The code family the embedded fonts provide (`docs/specs/02-visual.md` §3).
pub const BUFFER_FAMILY: &str = "JetBrains Mono";

/// Reads one embedded font file at compile time.
macro_rules! embedded_font {
    ($path:literal) => {
        Cow::Borrowed(include_bytes!($path).as_slice())
    };
}

/// The ten embedded TTF files, unmodified.
///
/// Static (not variable) weights: each one the interface uses (400, 500, 600,
/// 700 and their italics) exists as its own file, so it does not depend on
/// how `cosmic-text` resolves a variable font's weight axis.
pub fn embedded() -> Vec<Cow<'static, [u8]>> {
    vec![
        embedded_font!("../assets/fonts/inter/Inter-Regular.ttf"),
        embedded_font!("../assets/fonts/inter/Inter-Italic.ttf"),
        embedded_font!("../assets/fonts/inter/Inter-Medium.ttf"),
        embedded_font!("../assets/fonts/inter/Inter-SemiBold.ttf"),
        embedded_font!("../assets/fonts/inter/Inter-Bold.ttf"),
        embedded_font!("../assets/fonts/inter/Inter-BoldItalic.ttf"),
        embedded_font!("../assets/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf"),
        embedded_font!("../assets/fonts/jetbrains-mono/JetBrainsMono-Italic.ttf"),
        embedded_font!("../assets/fonts/jetbrains-mono/JetBrainsMono-Bold.ttf"),
        embedded_font!("../assets/fonts/jetbrains-mono/JetBrainsMono-BoldItalic.ttf"),
    ]
}

/// Registers the embedded fonts with GPUI's text system.
///
/// Call once, before opening any window. On success, [`UI_FAMILY`] and
/// [`BUFFER_FAMILY`] resolve through `cx.text_system().all_font_names()` even
/// on a machine that never installed either family; on failure, the caller is
/// expected to log the error and continue (the system fallbacks still work).
pub fn register_embedded(cx: &mut App) -> anyhow::Result<()> {
    let start = Instant::now();
    cx.text_system().add_fonts(embedded())?;
    tracing::info!(
        ui_family = UI_FAMILY,
        buffer_family = BUFFER_FAMILY,
        elapsed_ms = start.elapsed().as_millis(),
        "fuentes embebidas registradas"
    );
    Ok(())
}
