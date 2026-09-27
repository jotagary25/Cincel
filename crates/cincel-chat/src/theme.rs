//! Colours the chat panel paints, named after the tokens of
//! `docs/specs/02-visual.md` §2.
//!
//! [`ChatTheme`] is a plain struct with a `Default` ("Cincel Dark"), so the
//! workspace projects its own `ThemeColors` onto it without this crate
//! depending on `cincel-settings`, exactly like `cincel_editor::EditorTheme`.

use cincel_syntax::{HighlightId, HighlightTheme};
use gpui::{Hsla, rgb};

/// Every colour the chat panel paints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChatTheme {
    /// `bg.app`: the panel background.
    pub bg_app: Hsla,
    /// `bg.editor`: the fenced code blocks of an answer, so code reads as a
    /// piece of the editor and never as a user bubble.
    pub bg_editor: Hsla,
    /// `bg.surface`: tool cards, inputs.
    pub bg_surface: Hsla,
    /// `bg.elevated`: popovers, hovered rows, chips.
    pub bg_elevated: Hsla,
    /// `border`.
    pub border: Hsla,
    /// `border.focus`: the permission card and the focus ring.
    pub border_focus: Hsla,
    /// `text`.
    pub text: Hsla,
    /// `text.muted`.
    pub text_muted: Hsla,
    /// `text.accent`.
    pub text_accent: Hsla,
    /// `selection`.
    pub selection: Hsla,
    /// `diff.gutter.added` (solid): the `+N` of an edit tool card.
    pub diff_added: Hsla,
    /// `diff.gutter.deleted` (solid): the `−M` of an edit tool card.
    pub diff_deleted: Hsla,
    /// `status.error`.
    pub status_error: Hsla,
    /// `status.warning`.
    pub status_warning: Hsla,
    /// `status.ok`.
    pub status_ok: Hsla,
    /// The 12 tree-sitter captures, indexed by [`HighlightId::index`], used to
    /// paint the fenced code blocks of the agent's markdown.
    pub syntax: [Hsla; 12],
}

impl Default for ChatTheme {
    fn default() -> Self {
        let highlights = HighlightTheme::one_dark();
        let mut syntax = [rgb(0xabb2bf).into(); 12];
        for id in HighlightId::ALL {
            let [r, g, b] = highlights.color(id);
            syntax[id.index()] = rgb(u32::from_be_bytes([0, r, g, b])).into();
        }
        Self {
            bg_app: rgb(0x1e2127).into(),
            bg_editor: rgb(0x282c34).into(),
            bg_surface: rgb(0x21252b).into(),
            bg_elevated: rgb(0x2c313a).into(),
            border: rgb(0x181a1f).into(),
            border_focus: rgb(0x528bff).into(),
            text: rgb(0xabb2bf).into(),
            text_muted: rgb(0x5c6370).into(),
            text_accent: rgb(0x61afef).into(),
            selection: rgb(0x3e4451).into(),
            diff_added: rgb(0x98c379).into(),
            diff_deleted: rgb(0xe06c75).into(),
            status_error: rgb(0xe06c75).into(),
            status_warning: rgb(0xe5c07b).into(),
            status_ok: rgb(0x98c379).into(),
            syntax,
        }
    }
}

/// Alpha of the text selection in the chat's text views: `text.accent` at
/// 35 %, visible on the user bubble (accent at 14 %), on the panel and inside
/// code blocks, in both themes. The code editor keeps its own `selection`.
pub const SELECTION_ALPHA: f32 = 0.35;

impl ChatTheme {
    /// The colour of a syntax capture.
    #[must_use]
    pub fn syntax_color(&self, id: HighlightId) -> Hsla {
        self.syntax[id.index()]
    }

    /// The selection of the chat's text views (answers, bubbles, code blocks
    /// and the composer): `text.accent` at [`SELECTION_ALPHA`].
    #[must_use]
    pub fn text_selection(&self) -> Hsla {
        alpha(self.text_accent, SELECTION_ALPHA)
    }

    /// The composer's editor theme: the input box's `bg.surface` behind the
    /// text, the chat's selection, `border.focus` for the caret (the spec's
    /// `cursor` has the same value) and the twelve syntax colours the
    /// Markdown structure is painted with.
    #[must_use]
    pub fn composer_theme(&self) -> cincel_editor::EditorTheme {
        let rgba = |color: Hsla| -> gpui::Rgba { color.into() };
        cincel_editor::EditorTheme {
            background: rgba(self.bg_surface),
            surface: rgba(self.bg_surface),
            elevated: rgba(self.bg_elevated),
            border: rgba(self.border),
            border_focus: rgba(self.border_focus),
            text: rgba(self.text),
            text_muted: rgba(self.text_muted),
            text_accent: rgba(self.text_accent),
            cursor: rgba(self.border_focus),
            selection: rgba(self.text_selection()),
            diff_deleted: rgba(self.diff_deleted),
            diff_added: rgba(self.diff_added),
            diff_modified: rgba(self.status_warning),
            search_match: rgba(self.status_warning),
            status_error: rgba(self.status_error),
            status_warning: rgba(self.status_warning),
            status_ok: rgba(self.status_ok),
            syntax: self.syntax.map(rgba),
        }
    }
}

/// A theme colour with an explicit alpha.
#[must_use]
pub fn alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..color }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_theme_is_cincel_dark() {
        let theme = ChatTheme::default();
        assert_eq!(theme.bg_surface, rgb(0x21252b).into());
        assert_eq!(theme.border_focus, rgb(0x528bff).into());
        // `keyword` is One Dark's purple.
        assert_eq!(
            theme.syntax_color(HighlightId::Keyword),
            rgb(0xc678dd).into()
        );
    }

    #[test]
    fn the_selection_is_the_accent_at_35_percent() {
        let theme = ChatTheme::default();
        let selection = theme.text_selection();
        assert_eq!(selection, alpha(theme.text_accent, 0.35));
        let composer = theme.composer_theme();
        let painted: Hsla = composer.selection.into();
        assert!((painted.a - 0.35).abs() < 0.01);
        assert!((painted.h - theme.text_accent.h).abs() < 0.01);
    }

    #[test]
    fn alpha_overrides_only_the_alpha_channel() {
        let theme = ChatTheme::default();
        let faded = alpha(theme.border_focus, 0.25);
        assert_eq!(faded.h, theme.border_focus.h);
        assert_eq!(faded.a, 0.25);
    }
}
