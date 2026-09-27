//! The rendered Markdown preview of a tab (`workspace::toggle_markdown_preview`,
//! `Ctrl+Shift+V`, `docs/specs/02-visual.md` and `docs/specs/modulos/workspace.md`).
//!
//! It reuses the chat's own renderer (`cincel_chat::markdown`, already
//! public) over the buffer's *live* text, rather than copying any of the
//! chat's code: [`markdown_state`]/[`set_text`] parse the Markdown and
//! [`CodeHighlighter`] highlights fenced code blocks with `cincel-syntax`,
//! exactly like the chat does. Only the *visual* numbers differ from the
//! chat's own type scale, because the spec asks for a full-page reading
//! layout rather than a transcript entry: 32 px side padding, an 820 px
//! centered column, headings of 20/17/15 px and a 14 px body at 1.6 line
//! height. [`preview_style`] builds on the chat's
//! [`cincel_chat::markdown::text_view_style`] and overrides exactly those
//! two things (heading sizes, code block text size) rather than restating the
//! rest.
//!
//! One limitation of reusing `gpui-base`'s `TextView` this way: its heading
//! weight is hard-coded per level (h1 bold, h2-h5 semibold, h6 medium) and
//! `TextViewStyle` has no override for it, so "weight 500" only holds for the
//! sizes, not the weight — a real constraint of the vendored renderer, not a
//! shortcut taken here (the chat's own headings have the same limitation).

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use cincel_chat::ChatTheme;
use cincel_chat::markdown::{
    CodeHighlighter, CopyHandler, code_block_style_scaled, markdown_element, markdown_state,
    set_text, text_view_style_scaled,
};
use cincel_project::BufferHandle;
use cincel_syntax::LanguageRegistry;
use gpui::{
    App, ClipboardItem, Context, Entity, FocusHandle, Focusable, ScrollHandle, Styled as _, Task,
    Window,
};
use gpui_kit::base::text::TextViewState;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative};
use regex::{Captures, Regex};

/// How long to wait after the last buffer change before re-parsing the
/// preview (`docs/etapas/etapa-3.md`: "150 ms").
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(150);

/// Max width of the centered content column.
const CONTENT_MAX_WIDTH: f32 = 820.;
/// Side padding of the page.
const PAGE_PADDING: f32 = 32.;
/// Body text size.
const BODY_SIZE: f32 = 14.;
/// Body line height, as a multiple of the font size.
const BODY_LINE_HEIGHT: f32 = 1.6;
/// Code block text size (the chat's own is 12.5 px; the preview is a reading
/// surface, one notch bigger).
const CODE_SIZE: f32 = 13.;

/// A live rendering of a buffer's Markdown, reusing the chat's parser and
/// code-block highlighter.
///
/// Kept alive across toggles (cached on the [`crate::center::Tab`] that owns
/// it) so its [`ScrollHandle`] preserves the scroll position when the tab
/// flips back to the code editor and forward again.
pub struct MarkdownPreviewView {
    buffer: BufferHandle,
    registry: Arc<LanguageRegistry>,
    state: Entity<TextViewState>,
    /// The text last handed to `state`, with images already redacted
    /// (`gpui-base`'s `TextViewState` does not expose a getter of its own);
    /// read back by the tests to check the preview is showing the latest
    /// buffer text rather than what it was built with.
    rendered_text: String,
    theme: ChatTheme,
    scroll_handle: ScrollHandle,
    focus_handle: FocusHandle,
    _refresh_task: Option<Task<()>>,
}

impl MarkdownPreviewView {
    /// Builds a preview of `buffer`'s current text.
    pub fn new(
        buffer: BufferHandle,
        registry: Arc<LanguageRegistry>,
        theme: ChatTheme,
        cx: &mut App,
    ) -> Entity<Self> {
        let text = redact_images(&buffer.lock().text());
        cx.new(|cx| Self {
            buffer,
            registry,
            state: markdown_state(&text, cx),
            rendered_text: text,
            theme,
            scroll_handle: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _refresh_task: None,
        })
    }

    /// The Markdown text currently shown (images already redacted), for the
    /// tests.
    pub fn text(&self) -> &str {
        &self.rendered_text
    }

    /// Re-parses the buffer's current text right now.
    pub fn refresh_now(&mut self, cx: &mut Context<Self>) {
        self._refresh_task = None;
        let text = redact_images(&self.buffer.lock().text());
        set_text(&self.state, &text, cx);
        self.rendered_text = text;
    }

    /// Schedules [`Self::refresh_now`] 150 ms from now, replacing any refresh
    /// already pending (`docs/etapas/etapa-3.md`).
    pub fn schedule_refresh(&mut self, cx: &mut Context<Self>) {
        self._refresh_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| this.refresh_now(cx));
        }));
    }

    /// Hot-reloads the theme (settings reload, zoom).
    pub fn set_theme(&mut self, theme: ChatTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }
}

impl Focusable for MarkdownPreviewView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MarkdownPreviewView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        // Every pixel size follows the zoom (`workspace::zoom_*`).
        let scale = crate::settings::ui_scale(cx);
        let highlighter = CodeHighlighter::new(self.registry.clone(), &theme);
        let on_copy: CopyHandler = Arc::new(|code: String, cx: &mut App| {
            cx.write_to_clipboard(ClipboardItem::new_string(code));
        });

        // Links open through `cx.open_url` by default (gpui-base's
        // `TextView`, no handler installed); images are already redacted to a
        // muted blockquote placeholder before they ever reach the parser.
        let text_view = markdown_element(&self.state, &theme, highlighter, on_copy)
            .style(preview_style(&theme, scale))
            .text_size(px(BODY_SIZE * scale))
            .line_height(relative(BODY_LINE_HEIGHT));

        div()
            .id("markdown-preview")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll_handle)
            .bg(theme.bg_editor)
            .child(
                div()
                    .flex()
                    .justify_center()
                    .w_full()
                    .px(px(PAGE_PADDING * scale))
                    .py(px(PAGE_PADDING * scale))
                    .child(
                        div()
                            .id("markdown-preview-content")
                            .w_full()
                            .max_w(px(CONTENT_MAX_WIDTH * scale))
                            .text_color(theme.text)
                            .child(text_view),
                    ),
            )
    }
}

/// The [`cincel_chat::markdown::TextViewStyle`]-equivalent for the preview:
/// the chat's own colors and code-block look, with the two numbers the spec
/// gives its own values for (heading sizes, code text size).
fn preview_style(theme: &ChatTheme, scale: f32) -> gpui_kit::base::text::TextViewStyle {
    text_view_style_scaled(theme, scale)
        .with_heading_font_size(move |level, _base| {
            px(scale
                * match level {
                    1 => 20.,
                    2 => 17.,
                    _ => 15.,
                })
        })
        .with_code_block(code_block_style_scaled(theme, scale).text_size(px(CODE_SIZE * scale)))
}

/// Matches a Markdown image, `![alt](url "title")`. Reference-style images
/// (`![alt][ref]`) and inline `<img>` HTML are not covered: a preview of the
/// buffer as typed, not a full Markdown/HTML resolver.
fn image_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"!\[([^\]]*)\]\([^)]*\)"#).expect("el patrón de imagen no compila")
    })
}

/// Replaces every Markdown image with a muted blockquote placeholder instead
/// of loading it — the preview never fetches remote images or reads local
/// ones off the agent's own typing (`docs/etapas/etapa-3.md`).
///
/// A blockquote is a block-level element, so an inline `texto ![img](url) más
/// texto` becomes three paragraphs; that is the trade-off for a placeholder
/// that reads as muted text with `gpui-base`'s own styling instead of a new
/// one invented here.
fn redact_images(text: &str) -> String {
    image_pattern()
        .replace_all(text, |caps: &Captures<'_>| {
            let alt = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            let label = if alt.is_empty() {
                "imagen".to_owned()
            } else {
                format!("imagen: {alt}")
            };
            format!("\n\n> {label}\n\n")
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_become_a_muted_blockquote_placeholder() {
        assert_eq!(
            redact_images("mirá ![un gráfico](https://x/y.png) acá"),
            "mirá \n\n> imagen: un gráfico\n\n acá"
        );
        assert_eq!(redact_images("![](sin-alt.png)"), "\n\n> imagen\n\n");
    }

    #[test]
    fn text_without_images_is_untouched() {
        let text = "# Título\n\nUn párrafo con **negrita** y `código`.\n";
        assert_eq!(redact_images(text), text);
    }
}
