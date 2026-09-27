//! Host-computed decorations: highlights, full-width row backgrounds and
//! monospace rows that the host derives from the text itself.
//!
//! The code editor gets its colours from tree-sitter. A text field built on
//! the editor (the chat composer, which highlights the *structure* of the
//! Markdown it holds rather than a grammar's captures) hands the editor a
//! [`Decorator`] instead: a pure function of the buffer text, called by the
//! view once per text version, inside the same frame as the edit, so the
//! keystroke is painted with its final colours and backgrounds.

use std::ops::Range;
use std::sync::Arc;

use cincel_syntax::HighlightId;
use gpui::Rgba;

/// Everything a [`Decorator`] returns for one version of the text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextDecorations {
    /// Highlight spans in buffer byte offsets. When a decorator is installed
    /// these **replace** the tree-sitter highlights.
    pub highlights: Vec<(Range<usize>, HighlightId)>,
    /// Full-width backgrounds, per range of buffer rows, painted under the
    /// text and above the element background.
    pub row_backgrounds: Vec<(Range<u32>, Rgba)>,
    /// Buffer rows painted with the code font when
    /// [`crate::EditorSettings::prose_font_family`] is set.
    pub monospace_rows: Vec<Range<u32>>,
}

impl TextDecorations {
    /// Whether `row` falls in one of [`Self::monospace_rows`].
    pub fn is_monospace(&self, row: u32) -> bool {
        self.monospace_rows.iter().any(|rows| rows.contains(&row))
    }

    /// The background of `row`, if one of [`Self::row_backgrounds`] covers it
    /// (the last one wins).
    pub fn background(&self, row: u32) -> Option<Rgba> {
        self.row_backgrounds
            .iter()
            .rev()
            .find(|(rows, _)| rows.contains(&row))
            .map(|(_, color)| *color)
    }
}

/// Computes the [`TextDecorations`] of a text. Installed with
/// [`crate::EditorView::set_decorator`].
pub type Decorator = Arc<dyn Fn(&str) -> TextDecorations + Send + Sync>;
