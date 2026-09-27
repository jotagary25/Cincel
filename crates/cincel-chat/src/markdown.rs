//! Streaming markdown for the agent's answers (`02-visual.md` §7).
//!
//! `gpui-kit` owns the parsing and the layout: a [`TextViewState`] entity holds
//! the document and `push_str` appends a chunk without re-parsing the whole
//! text, which is what "el texto llega en streaming y se agrega sin saltos"
//! asks for. What this module adds is Cincel's own look: the colours of
//! [`ChatTheme`] and, for every fenced code block, the tree-sitter highlighting
//! of `cincel-syntax`, the `bg.editor` background with a 1 px `border` and a
//! small header with the language and a "Copiar" button, so a block of code
//! never looks like a user bubble (`docs/etapas/etapa-3.md` § correcciones).

use std::ops::Range;
use std::sync::Arc;

use cincel_syntax::{CancelFlag, HighlightId, LanguageRegistry, SyntaxState};
use cincel_text::Buffer;
use gpui::{App, AppContext as _, Entity, HighlightStyle, SharedString, StyleRefinement, px};
use gpui_kit::base::TextView;
use gpui_kit::base::text::{CodeBlock, TextViewState, TextViewStyle};

use crate::settings::{CODE_BLOCK_HEADER, CODE_BLOCK_RADIUS, TEXT_CODE, TEXT_LABEL, heading_size};
use crate::theme::{ChatTheme, alpha};

/// Longest code block that is highlighted inline.
///
/// Above this the answer is still shown, just unhighlighted: parsing a huge
/// fence on the UI thread every frame would cost more than it is worth, and the
/// real place to read a long file is the editor.
pub const MAX_HIGHLIGHTED_CODE_BYTES: usize = 64 * 1024;

/// Highlights the fenced code blocks of an answer with `cincel-syntax`.
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

/// The refinement `gpui-kit` applies to the box of a fenced code block: the
/// code size of the type scale, a 1 px `border`, rounded corners and room at
/// the top for the language / "Copiar" header.
#[must_use]
pub fn code_block_style(theme: &ChatTheme) -> StyleRefinement {
    code_block_style_scaled(theme, 1.)
}

/// [`code_block_style`] at the interface zoom `scale`.
#[must_use]
pub fn code_block_style_scaled(theme: &ChatTheme, scale: f32) -> StyleRefinement {
    use gpui::Styled as _;
    StyleRefinement::default()
        .text_size(px(TEXT_CODE * scale))
        .border_1()
        .border_color(theme.border)
        .rounded(px(CODE_BLOCK_RADIUS * scale))
        .pt(px((CODE_BLOCK_HEADER + 6.) * scale))
        .px(px(10. * scale))
        .pb(px(8. * scale))
}

/// The markdown style of the chat, derived from [`ChatTheme`].
///
/// The selection is `text.accent` at 35 % ([`ChatTheme::text_selection`]),
/// which reads on the panel, on the accent-tinted user bubble and on the
/// `bg.editor` of a code block alike. `gpui-kit` paints every selection of a
/// `TextView` (paragraphs, inline code, fenced blocks) with this field; the
/// kit theme's own `selection` token, its fallback, is mapped to the same
/// colour by the workspace.
#[must_use]
pub fn text_view_style(theme: &ChatTheme) -> TextViewStyle {
    text_view_style_scaled(theme, 1.)
}

/// [`text_view_style`] at the interface zoom `scale`.
#[must_use]
pub fn text_view_style_scaled(theme: &ChatTheme, scale: f32) -> TextViewStyle {
    TextViewStyle::default()
        .with_foreground(theme.text)
        .with_muted_foreground(theme.text_muted)
        .with_link(theme.text_accent)
        .with_selection(theme.text_selection())
        .with_code_background(theme.bg_editor)
        .with_code_block(code_block_style_scaled(theme, scale))
        .with_border(theme.border)
        .with_heading_base_font_size(px(TEXT_BODY_FOR_HEADINGS * scale))
        .with_heading_font_size(move |level, _| px(heading_size(level) * scale))
        .with_inline_code(HighlightStyle {
            background_color: Some(alpha(theme.bg_elevated, 0.8)),
            ..Default::default()
        })
        .with_dark(true)
}

/// The base `gpui-kit` scales headings from; irrelevant once
/// `with_heading_font_size` pins every level, kept equal to the body size.
const TEXT_BODY_FOR_HEADINGS: f32 = crate::settings::TEXT_BODY;

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
    markdown_element_scaled(state, theme, 1., highlighter, on_copy)
}

/// [`markdown_element`] at the interface zoom `scale`: the answers and the
/// user bubbles of the chat use this one.
pub fn markdown_element_scaled(
    state: &Entity<TextViewState>,
    theme: &ChatTheme,
    scale: f32,
    highlighter: CodeHighlighter,
    on_copy: CopyHandler,
) -> TextView {
    let button_text = theme.text_muted;
    let button_hover = theme.text;
    TextView::new(state)
        .selectable(true)
        .style(text_view_style_scaled(theme, scale))
        .code_block_highlighter(move |block| highlighter.highlight(block))
        .code_block_actions(move |block, _window, _cx| {
            let code = block.code().to_string();
            let lang = block.lang().map(|lang| lang.to_string());
            let on_copy = on_copy.clone();
            code_block_header(code, lang, button_text, button_hover, on_copy, scale)
        })
}

/// The label of a code block's header: its language, or "código".
#[must_use]
pub fn code_block_label(lang: Option<&str>) -> String {
    match lang.map(str::trim) {
        Some(lang) if !lang.is_empty() => lang.to_lowercase(),
        _ => "código".to_string(),
    }
}

/// The header `gpui-kit` paints in the top-right corner of a code block: the
/// language and a "Copiar" button, in the label size of the type scale.
fn code_block_header(
    code: String,
    lang: Option<String>,
    text: gpui::Hsla,
    hover: gpui::Hsla,
    on_copy: CopyHandler,
    scale: f32,
) -> impl gpui::IntoElement {
    use gpui::prelude::*;
    use gpui::{ClickEvent, div};

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px((CODE_BLOCK_HEADER - 6.) * scale))
        .text_size(px(TEXT_LABEL * scale))
        .text_color(text)
        .child(div().child(SharedString::from(code_block_label(lang.as_deref()))))
        .child(
            div()
                .id(SharedString::from(format!(
                    "copy-{:x}",
                    fnv1a(code.as_bytes())
                )))
                .px_1()
                .cursor_pointer()
                .hover(move |style| style.text_color(hover))
                .child("Copiar")
                .on_click(move |_: &ClickEvent, _window, cx| on_copy(code.clone(), cx)),
        )
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
    fn a_rust_fence_gets_spans_from_cincel_syntax() {
        let highlighter =
            CodeHighlighter::new(Arc::new(LanguageRegistry::new()), &ChatTheme::default());
        let block = CodeBlock::from_code("fn main() { let x = 1; }", Some("rust"));
        let spans = highlighter.highlight(&block);
        assert!(!spans.is_empty(), "el resaltado de Rust no devolvió nada");
        assert!(spans.iter().all(|(range, _)| range.end <= 24));
    }

    #[test]
    fn a_code_block_is_painted_like_the_editor_not_like_a_bubble() {
        let theme = ChatTheme::default();
        let style = code_block_style(&theme);
        assert_eq!(
            style.text.font_size,
            Some(gpui::AbsoluteLength::Pixels(px(TEXT_CODE)))
        );
        assert_eq!(style.border_color, Some(theme.border));
        assert_ne!(theme.bg_editor, theme.bg_app);
        assert_eq!(code_block_label(Some("Rust")), "rust");
        assert_eq!(code_block_label(None), "código");
        assert_eq!(code_block_label(Some("  ")), "código");
    }

    #[test]
    fn the_text_views_select_with_the_accent_at_35_percent() {
        let theme = ChatTheme::default();
        let style = text_view_style(&theme);
        assert_eq!(style.selection(), alpha(theme.text_accent, 0.35));
        assert_ne!(style.selection(), theme.selection, "not the editor's");
        let zoomed = text_view_style_scaled(&theme, 1.5);
        assert_eq!(zoomed.selection(), style.selection());
        assert_eq!(zoomed.heading_font_size(1), Some(px(22.5)));
    }

    #[test]
    fn an_unknown_language_is_not_highlighted() {
        let highlighter =
            CodeHighlighter::new(Arc::new(LanguageRegistry::new()), &ChatTheme::default());
        let block = CodeBlock::from_code("hello", Some("brainfuck"));
        assert!(highlighter.highlight(&block).is_empty());
    }
}
