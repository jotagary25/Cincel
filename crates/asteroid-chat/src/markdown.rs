//! Streaming markdown for the agent's answers (`02-visual.md` §7).
//!
//! `gpui-kit` owns the parsing and the layout: a [`TextViewState`] entity holds
//! the document and `push_str` appends a chunk without re-parsing the whole
//! text, which is what "el texto llega en streaming y se agrega sin saltos"
//! asks for. What this module adds is Asteroid's own look: the colours of
//! [`ChatTheme`] and, for every fenced code block, the tree-sitter highlighting
//! of `asteroid-syntax` plus a "Copiar" button.

use std::ops::Range;
use std::sync::Arc;

use asteroid_syntax::{CancelFlag, HighlightId, LanguageRegistry, SyntaxState};
use asteroid_text::Buffer;
use gpui::{App, AppContext as _, Entity, HighlightStyle, SharedString, px};
use gpui_kit::base::TextView;
use gpui_kit::base::text::{CodeBlock, TextViewState, TextViewStyle};

use crate::theme::{ChatTheme, alpha};

/// Longest code block that is highlighted inline.
///
/// Above this the answer is still shown, just unhighlighted: parsing a huge
/// fence on the UI thread every frame would cost more than it is worth, and the
/// real place to read a long file is the editor.
pub const MAX_HIGHLIGHTED_CODE_BYTES: usize = 64 * 1024;

/// Highlights the fenced code blocks of an answer with `asteroid-syntax`.
///
/// Cheap to clone (an `Arc` and twelve colours) and `Send + Sync`, which is what
/// `gpui-kit` asks of a code-block highlighter.
#[derive(Clone)]
pub struct CodeHighlighter {
    registry: Arc<LanguageRegistry>,
    colors: [gpui::Hsla; 12],
}

impl CodeHighlighter {
    /// Builds the highlighter for a theme.
    #[must_use]
    pub fn new(registry: Arc<LanguageRegistry>, theme: &ChatTheme) -> Self {
        Self {
            registry,
            colors: theme.syntax,
        }
    }

    /// The spans of one code block, relative to [`CodeBlock::code`].
    #[must_use]
    pub fn highlight(&self, block: &CodeBlock) -> Vec<(Range<usize>, HighlightStyle)> {
        let Some(lang) = block.lang() else {
            return Vec::new();
        };
        let Some(language) = self.registry.language(lang.as_ref()) else {
            return Vec::new();
        };
        let code = block.code();
        if code.len() > MAX_HIGHLIGHTED_CODE_BYTES {
            return Vec::new();
        }
        let buffer = Buffer::new(code.as_ref());
        let mut state = SyntaxState::new(self.registry.clone(), language, buffer.snapshot());
        state.reparse(&CancelFlag::new());
        state
            .highlights(0..code.len())
            .into_iter()
            .map(|(range, id)| (range, self.style(id)))
            .collect()
    }

    fn style(&self, id: HighlightId) -> HighlightStyle {
        HighlightStyle {
            color: Some(self.colors[id.index()]),
            ..Default::default()
        }
    }
}

/// Creates the state of a markdown entry.
pub fn markdown_state(text: &str, cx: &mut App) -> Entity<TextViewState> {
    cx.new(|cx| TextViewState::markdown(text, cx).selectable(true))
}

/// Appends a streamed chunk to an existing entry, without reparsing the rest.
pub fn append_chunk(state: &Entity<TextViewState>, chunk: &str, cx: &mut App) {
    state.update(cx, |state, cx| state.push_str(chunk, cx));
}

/// Replaces the whole text of an entry (used by `import_transcript`).
pub fn set_text(state: &Entity<TextViewState>, text: &str, cx: &mut App) {
    state.update(cx, |state, cx| state.set_text(text, cx));
}

/// The markdown style of the chat, derived from [`ChatTheme`].
#[must_use]
pub fn text_view_style(theme: &ChatTheme) -> TextViewStyle {
    TextViewStyle::default()
        .with_foreground(theme.text)
        .with_muted_foreground(theme.text_muted)
        .with_link(theme.text_accent)
        .with_selection(theme.selection)
        .with_code_background(theme.bg_surface)
        .with_border(theme.border)
        .with_heading_base_font_size(px(15.))
        .with_inline_code(HighlightStyle {
            background_color: Some(alpha(theme.bg_elevated, 0.8)),
            ..Default::default()
        })
        .with_dark(true)
}

/// What the "Copiar" button of a code block calls; the panel turns it into
/// [`crate::ChatEvent::CopyToClipboard`].
pub type CopyHandler = Arc<dyn Fn(String, &mut App) + Send + Sync>;

/// Builds the element of a markdown entry.
///
/// `on_copy` receives the source of the code block whose "Copiar" button was
/// clicked; the panel turns it into [`crate::ChatEvent::CopyToClipboard`],
/// because only the workspace may touch the clipboard.
pub fn markdown_element(
    state: &Entity<TextViewState>,
    theme: &ChatTheme,
    highlighter: CodeHighlighter,
    on_copy: CopyHandler,
) -> TextView {
    let button_text = theme.text_muted;
    let button_hover = theme.text;
    TextView::new(state)
        .selectable(true)
        .style(text_view_style(theme))
        .code_block_highlighter(move |block| highlighter.highlight(block))
        .code_block_actions(move |block, _window, _cx| {
            let code = block.code().to_string();
            let on_copy = on_copy.clone();
            copy_button(code, button_text, button_hover, on_copy)
        })
}

/// The "Copiar" button `gpui-kit` paints in the corner of a code block.
fn copy_button(
    code: String,
    text: gpui::Hsla,
    hover: gpui::Hsla,
    on_copy: CopyHandler,
) -> impl gpui::IntoElement {
    use gpui::prelude::*;
    use gpui::{ClickEvent, div};

    div()
        .id(SharedString::from(format!(
            "copy-{:x}",
            fnv1a(code.as_bytes())
        )))
        .px_1p5()
        .py_0p5()
        .text_size(px(11.))
        .text_color(text)
        .cursor_pointer()
        .hover(move |style| style.text_color(hover))
        .child("Copiar")
        .on_click(move |_: &ClickEvent, _window, cx| on_copy(code.clone(), cx))
}

/// A stable id per code block, so the button keeps its identity across frames.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fence_without_a_language_is_not_highlighted() {
        let highlighter =
            CodeHighlighter::new(Arc::new(LanguageRegistry::new()), &ChatTheme::default());
        let block = CodeBlock::from_code("fn main() {}", None::<&str>);
        assert!(highlighter.highlight(&block).is_empty());
    }

    #[test]
    fn a_rust_fence_gets_spans_from_asteroid_syntax() {
        let highlighter =
            CodeHighlighter::new(Arc::new(LanguageRegistry::new()), &ChatTheme::default());
        let block = CodeBlock::from_code("fn main() { let x = 1; }", Some("rust"));
        let spans = highlighter.highlight(&block);
        assert!(!spans.is_empty(), "el resaltado de Rust no devolvió nada");
        assert!(spans.iter().all(|(range, _)| range.end <= 24));
    }

    #[test]
    fn an_unknown_language_is_not_highlighted() {
        let highlighter =
            CodeHighlighter::new(Arc::new(LanguageRegistry::new()), &ChatTheme::default());
        let block = CodeBlock::from_code("hello", Some("brainfuck"));
        assert!(highlighter.highlight(&block).is_empty());
    }
}
