//! Colors of the editor (One Dark), see `docs/specs/02-visual.md`.

use gpui::{Hsla, rgb};

/// Editor background.
pub const BG: u32 = 0x282c34;
/// Elevated surfaces: current line, pill.
pub const BG_ELEVATED: u32 = 0x2c313a;
/// Regular text.
pub const TEXT: u32 = 0xabb2bf;
/// Muted text: line numbers.
pub const TEXT_MUTED: u32 = 0x5c6370;
/// Cursor.
pub const CURSOR: u32 = 0x528bff;
/// Selection background.
pub const SELECTION: u32 = 0x3e4451;
/// Deleted (phantom) rows.
pub const DIFF_DELETED: u32 = 0xe06c75;
/// Added rows.
pub const DIFF_ADDED: u32 = 0x98c379;

/// Alpha of the diff row backgrounds.
pub const DIFF_BG_ALPHA: f32 = 0.18;
/// Opacity of the text of the phantom rows.
pub const PHANTOM_TEXT_ALPHA: f32 = 0.7;

/// An opaque color from a `0xRRGGBB` literal.
pub fn color(value: u32) -> Hsla {
    rgb(value).into()
}

/// A color from a `0xRRGGBB` literal with an explicit alpha.
pub fn color_alpha(value: u32, alpha: f32) -> Hsla {
    let mut hsla: Hsla = rgb(value).into();
    hsla.a = alpha;
    hsla
}
