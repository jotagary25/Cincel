//! Colours the editor element needs, named after the tokens of
//! `docs/specs/02-visual.md` §2.
//!
//! [`EditorTheme`] is a plain struct with a `Default` ("Asteroid Dark"), so the
//! workspace can project its own theme type onto it without this crate
//! depending on `asteroid-settings`.

use asteroid_syntax::{HighlightId, HighlightTheme};
use gpui::{Hsla, Rgba, rgb};

/// Every colour the element paints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorTheme {
    /// `bg.editor`.
    pub background: Rgba,
    /// `bg.surface`: the search bar and the go-to-line prompt.
    pub surface: Rgba,
    /// `bg.elevated`: current line (at 60%), pill, popovers.
    pub elevated: Rgba,
    /// `border`.
    pub border: Rgba,
    /// `border.focus`.
    pub border_focus: Rgba,
    /// `text`.
    pub text: Rgba,
    /// `text.muted`: line numbers, whitespace marks, ruler.
    pub text_muted: Rgba,
    /// `text.accent`.
    pub text_accent: Rgba,
    /// `cursor`.
    pub cursor: Rgba,
    /// `selection`.
    pub selection: Rgba,
    /// `diff.deleted.bg` (painted at [`DIFF_BG_ALPHA`]).
    pub diff_deleted: Rgba,
    /// `diff.added.bg` (painted at [`DIFF_BG_ALPHA`]).
    pub diff_added: Rgba,
    /// `diff.gutter.modified`.
    pub diff_modified: Rgba,
    /// Background of a search match (painted at [`SEARCH_MATCH_ALPHA`]).
    pub search_match: Rgba,
    /// `status.error`.
    pub status_error: Rgba,
    /// `status.warning`.
    pub status_warning: Rgba,
    /// `status.ok`.
    pub status_ok: Rgba,
    /// The 12 tree-sitter captures, indexed by [`HighlightId::index`].
    pub syntax: [Rgba; 12],
}

/// Alpha of the diff row backgrounds (02-visual §2: 18%).
pub const DIFF_BG_ALPHA: f32 = 0.18;
/// Alpha of the word-diff backgrounds (02-visual §2: 38%).
pub const DIFF_WORD_ALPHA: f32 = 0.38;
/// Opacity of the text of the phantom rows (02-visual §6.1: 70%).
pub const PHANTOM_TEXT_ALPHA: f32 = 0.7;
/// Alpha of the current-line background (02-visual §5: 60%).
pub const CURRENT_LINE_ALPHA: f32 = 0.6;
/// Alpha of a search match background (02-visual §5: 30%).
pub const SEARCH_MATCH_ALPHA: f32 = 0.3;
/// Alpha of the *current* search match background (02-visual §5: 55%).
pub const SEARCH_CURRENT_ALPHA: f32 = 0.55;

impl Default for EditorTheme {
    fn default() -> Self {
        let highlights = HighlightTheme::one_dark();
        let mut syntax = [rgb(0xabb2bf); 12];
        for id in HighlightId::ALL {
            let [r, g, b] = highlights.color(id);
            syntax[id.index()] = rgb(u32::from_be_bytes([0, r, g, b]));
        }
        Self {
            background: rgb(0x282c34),
            surface: rgb(0x21252b),
            elevated: rgb(0x2c313a),
            border: rgb(0x181a1f),
            border_focus: rgb(0x528bff),
            text: rgb(0xabb2bf),
            text_muted: rgb(0x5c6370),
            text_accent: rgb(0x61afef),
            cursor: rgb(0x528bff),
            selection: rgb(0x3e4451),
            diff_deleted: rgb(0xe06c75),
            diff_added: rgb(0x98c379),
            diff_modified: rgb(0xe5c07b),
            search_match: rgb(0xe5c07b),
            status_error: rgb(0xe06c75),
            status_warning: rgb(0xe5c07b),
            status_ok: rgb(0x98c379),
            syntax,
        }
    }
}

impl EditorTheme {
    /// The colour of a syntax capture.
    pub fn syntax_color(&self, id: HighlightId) -> Hsla {
        color(self.syntax[id.index()])
    }
}

/// An opaque [`Hsla`] from a theme colour.
pub fn color(value: Rgba) -> Hsla {
    value.into()
}

/// A theme colour with an explicit alpha.
pub fn alpha(value: Rgba, alpha: f32) -> Hsla {
    let mut hsla: Hsla = value.into();
    hsla.a = alpha;
    hsla
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_theme_is_one_dark() {
        let theme = EditorTheme::default();
        assert_eq!(theme.background, rgb(0x282c34));
        // `keyword` is One Dark's purple.
        assert_eq!(theme.syntax[HighlightId::Keyword.index()], rgb(0xc678dd));
        assert_eq!(theme.syntax[HighlightId::String.index()], rgb(0x98c379));
    }

    #[test]
    fn alpha_overrides_only_the_alpha_channel() {
        let theme = EditorTheme::default();
        let opaque = color(theme.cursor);
        let faded = alpha(theme.cursor, 0.5);
        assert_eq!(faded.h, opaque.h);
        assert_eq!(faded.a, 0.5);
    }
}
