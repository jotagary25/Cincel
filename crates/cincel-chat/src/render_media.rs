//! Painting the images and the review comments of the chat
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §5.2, §6.3).
//!
//! Inside the composer box, above the text, two rows of their own: the tags
//! of the unsent comments, then the thumbnails of the attached images, each
//! with its `×`. In a sent message, under the text: the images in small, then
//! one card per comment that left with it. A third `impl ChatPanel` block, so
//! `render.rs` only calls into it.

use gpui::prelude::*;
use gpui::{AnyElement, App, ClickEvent, Context, ObjectFit, SharedString, div, img, px};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};

use crate::attachments::{
    ATTACH_TOOLTIP, IMAGE_UNAVAILABLE, REMOVE_COMMENT_TOOLTIP, REMOVE_IMAGE_TOOLTIP,
};
use crate::comments::{CARD_CODE_LINES, COMMENT_NOT_SENT, SentCommentCard};
use crate::model::{ImageRef, MessageBlock, UserMessage};
use crate::panel::{COMPOSER_HINT, ChatPanel};
use crate::panel_media::{AttachmentState, image_identity};
use crate::settings::{CARD_RADIUS, TEXT_BODY, TEXT_CODE, TEXT_LABEL, TEXT_SMALL};
use crate::theme::ChatTheme;

/// Side of a thumbnail inside the composer box.
pub const ATTACHMENT_THUMB_SIDE: f32 = 56.;
/// Radius of every thumbnail.
pub const THUMB_RADIUS: f32 = 6.;
/// Tallest a thumbnail of a sent message gets.
pub const SENT_THUMB_MAX_HEIGHT: f32 = 96.;
/// Widest a thumbnail of a sent message gets.
pub const SENT_THUMB_MAX_WIDTH: f32 = 160.;
/// Diameter of the `×` of a thumbnail.
const REMOVE_BADGE: f32 = 16.;

/// The size a sent thumbnail is painted at: at most
/// [`SENT_THUMB_MAX_HEIGHT`] high and [`SENT_THUMB_MAX_WIDTH`] wide, keeping
/// the image's proportions (unscaled pixels).
#[must_use]
pub fn sent_thumb_size(width: u32, height: u32) -> (f32, f32) {
    if width == 0 || height == 0 {
        return (SENT_THUMB_MAX_HEIGHT, SENT_THUMB_MAX_HEIGHT);
    }
    let (width, height) = (width as f32, height as f32);
    let mut h = height.min(SENT_THUMB_MAX_HEIGHT);
    let mut w = h * width / height;
    if w > SENT_THUMB_MAX_WIDTH {
        w = SENT_THUMB_MAX_WIDTH;
        h = w * height / width;
    }
    (w, h)
}

fn mono_family(cx: &App) -> SharedString {
    gpui_kit::base::Theme::global(cx)
        .tokens
        .typography
        .mono
        .clone()
}

impl ChatPanel {
    /// The line under the box: what the composer does, or "Preparando N
    /// imágenes…" while images are being prepared.
    pub(crate) fn composer_hint(&self) -> SharedString {
        match self.preparing_count() {
            0 => SharedString::from(COMPOSER_HINT),
            count => SharedString::from(crate::attachments::preparing_text(count)),
        }
    }

    /// "«Nombre» no acepta imágenes: quitá las imágenes para enviar", above
    /// the box while the draft has images the connection does not take.
    pub(crate) fn render_composer_warning(&self, theme: &ChatTheme) -> Option<AnyElement> {
        if !self.images_block_send() {
            return None;
        }
        let s = *self.settings();
        Some(
            div()
                .debug_selector(|| "chat-images-blocked".to_string())
                .text_size(s.px(TEXT_SMALL))
                .text_color(theme.status_warning)
                .child(SharedString::from(crate::attachments::blocked_send_text(
                    &self.connection_name(),
                )))
                .into_any_element(),
        )
    }

    /// The paperclip, at the bottom left of the box.
    pub(crate) fn render_attach_button(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .debug_selector(|| "chat-attach".to_string())
            .flex_shrink_0()
            .child(
                Button::new("chat-attach")
                    .icon(IconName::Paperclip)
                    .small()
                    .ghost()
                    .tooltip(ATTACH_TOOLTIP)
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.request_pick_images(cx);
                    })),
            )
            .into_any_element()
    }

    /// The tags of the unsent comments (D13): the look of the file chip of a
    /// sent message, a comment icon before and a `×` after.
    pub(crate) fn render_comment_tags(
        &self,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.pending_comments.is_empty() {
            return None;
        }
        let theme = *theme;
        let s = *self.settings();
        let tags: Vec<AnyElement> = self
            .pending_comments
            .iter()
            .enumerate()
            .map(|(index, comment)| {
                let id = comment.id;
                let path = comment.path.clone();
                let line = comment.line;
                let tooltip = SharedString::from(comment.tooltip.clone());
                h_flex()
                    .id(("comment-tag", index))
                    .debug_selector(move || format!("comment-tag-{index}"))
                    .flex_shrink_0()
                    .gap_1()
                    .items_center()
                    .px_1p5()
                    .py_0p5()
                    .rounded(s.px(CARD_RADIUS))
                    .bg(theme.bg_elevated)
                    .text_size(s.px(TEXT_SMALL))
                    .text_color(theme.text_accent)
                    .cursor_pointer()
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.open_location(path.clone(), line, cx);
                    }))
                    .child(Icon::new(IconName::MessageSquareText).size(s.px(11.)))
                    .child(SharedString::from(comment.label.clone()))
                    .child(
                        div()
                            .id(("comment-tag-remove", index))
                            .debug_selector(move || format!("comment-tag-remove-{index}"))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(s.px(CARD_RADIUS))
                            .text_color(theme.text_muted)
                            .hover(move |style| style.text_color(theme.status_error))
                            .tooltip(|window, cx| {
                                Tooltip::new(REMOVE_COMMENT_TOOLTIP).build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                cx.stop_propagation();
                                this.remove_comment(id, cx);
                            }))
                            .child(Icon::new(IconName::X).size(s.px(12.))),
                    )
                    .into_any_element()
            })
            .collect();
        Some(
            h_flex()
                .debug_selector(|| "chat-comment-tags".to_string())
                .w_full()
                .flex_wrap()
                .gap_1p5()
                .children(tags)
                .into_any_element(),
        )
    }

    /// The thumbnails of the draft: 56 × 56, cropped to the centre, a `×` in
    /// a circle at the top right (§5.2).
    pub(crate) fn render_attachment_row(
        &self,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let theme = *theme;
        let s = *self.settings();
        let side = s.px(ATTACHMENT_THUMB_SIDE);
        let thumbs: Vec<AnyElement> = self
            .attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| {
                let id = attachment.id;
                let (content, tooltip): (AnyElement, SharedString) = match &attachment.state {
                    AttachmentState::Processing => (
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Spinner::new().small().color(theme.text_accent))
                            .into_any_element(),
                        SharedString::from(attachment.name.clone()),
                    ),
                    AttachmentState::Ready(image) => {
                        let source = self.media_cache.borrow_mut().image(
                            &image.sha256,
                            image.mime_type,
                            &image.bytes,
                        );
                        (
                            img(source)
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                                .into_any_element(),
                            SharedString::from(image.summary()),
                        )
                    }
                };
                div()
                    .id(("attachment", index))
                    .debug_selector(move || format!("attachment-{index}"))
                    .relative()
                    .flex_shrink_0()
                    .size(side)
                    .child(
                        div()
                            .id(("attachment-thumb", index))
                            .size_full()
                            .overflow_hidden()
                            .rounded(s.px(THUMB_RADIUS))
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.bg_editor)
                            .tooltip(move |window, cx| {
                                Tooltip::new(tooltip.clone()).build(window, cx)
                            })
                            .child(content),
                    )
                    .child(
                        div()
                            .id(("attachment-remove", index))
                            .debug_selector(move || format!("attachment-remove-{index}"))
                            .absolute()
                            .top(s.px(2.))
                            .right(s.px(2.))
                            .size(s.px(REMOVE_BADGE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(theme.bg_elevated)
                            .text_color(theme.text)
                            .cursor_pointer()
                            .hover(move |style| style.text_color(theme.status_error))
                            .tooltip(|window, cx| {
                                Tooltip::new(REMOVE_IMAGE_TOOLTIP).build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                cx.stop_propagation();
                                this.remove_attachment(id, cx);
                            }))
                            .child(Icon::new(IconName::X).size(s.px(12.))),
                    )
                    .into_any_element()
            })
            .collect();
        Some(
            h_flex()
                .debug_selector(|| "chat-attachments".to_string())
                .w_full()
                .flex_wrap()
                .gap_1p5()
                .children(thumbs)
                .into_any_element(),
        )
    }

    /// The row of thumbnails a sent message carries above its text, or `None`
    /// when it has no image (`docs/specs/10-etapa7-ronda2.md` §5.1).
    pub(crate) fn render_message_images(
        &self,
        index: usize,
        message: &UserMessage,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let images: Vec<AnyElement> = message
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(block_index, block)| match block {
                MessageBlock::Image(image) => Some((block_index, image)),
                _ => None,
            })
            .enumerate()
            .map(|(ordinal, (block_index, image))| {
                self.render_sent_image(index, block_index, ordinal, image, theme, cx)
            })
            .collect();
        (!images.is_empty()).then(|| {
            h_flex()
                .debug_selector(move || format!("user-images-{index}"))
                .flex_wrap()
                .gap_1p5()
                .children(images)
                .into_any_element()
        })
    }

    /// The cards of the comments that left with a sent message, painted under
    /// its text (`docs/specs/09-etapa7-…` §6.3).
    pub(crate) fn render_message_cards(
        &self,
        index: usize,
        message: &UserMessage,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        message
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(block_index, block)| match block {
                MessageBlock::Comment(card) => Some((block_index, card)),
                _ => None,
            })
            .enumerate()
            .map(|(ordinal, (block_index, card))| {
                self.render_comment_card(index, block_index, ordinal, card, theme, cx)
            })
            .collect()
    }

    /// One thumbnail of a sent message: from its bytes while they are in
    /// memory, from `images/<file>` once they are not, or "Imagen no
    /// disponible" when the file is gone. A click opens the viewer.
    fn render_sent_image(
        &self,
        index: usize,
        block: usize,
        ordinal: usize,
        image: &ImageRef,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        let (width, height) = sent_thumb_size(image.width, image.height);
        let path = self.image_path(image);
        let content: Option<AnyElement> = match (&image.data, &path) {
            (Some(data), _) => {
                let key = image_identity(image).unwrap_or_default();
                let source = self
                    .media_cache
                    .borrow_mut()
                    .image(&key, &image.mime_type, data);
                Some(
                    img(source)
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .into_any_element(),
                )
            }
            (None, Some(path)) if self.media_cache.borrow_mut().exists(path) => Some(
                img(path.clone())
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .into_any_element(),
            ),
            _ => None,
        };
        let Some(content) = content else {
            return h_flex()
                .debug_selector(move || format!("sent-image-missing-{index}-{ordinal}"))
                .h(s.px(SENT_THUMB_MAX_HEIGHT / 2.))
                .px_2()
                .gap_1()
                .items_center()
                .rounded(s.px(THUMB_RADIUS))
                .border_1()
                .border_color(theme.border)
                .bg(theme.bg_surface)
                .text_size(s.px(TEXT_SMALL))
                .text_color(theme.text_muted)
                .child(Icon::new(IconName::ImageOff).size(s.px(14.)))
                .child(IMAGE_UNAVAILABLE)
                .into_any_element();
        };
        let tooltip = SharedString::from(image.summary());
        div()
            .id(("sent-image", index * 4096 + block))
            .debug_selector(move || format!("sent-image-{index}-{ordinal}"))
            .flex_shrink_0()
            .w(s.px(width))
            .h(s.px(height))
            .overflow_hidden()
            .rounded(s.px(THUMB_RADIUS))
            .border_1()
            .border_color(theme.border)
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.open_image(index, block, cx);
            }))
            .child(content)
            .into_any_element()
    }

    /// The card of a comment that left with the message: file, lines and
    /// state in the header (a click opens the file there), the code in mono
    /// on `bg.editor` (six lines, then "… N líneas más") and the user's text.
    fn render_comment_card(
        &self,
        index: usize,
        block: usize,
        ordinal: usize,
        card: &SentCommentCard,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        let path = card.path.clone();
        let line = card.first_line;
        let header = h_flex()
            .id(("comment-card-header", index * 4096 + block))
            .debug_selector(move || format!("comment-card-header-{index}-{ordinal}"))
            .gap_1()
            .items_center()
            .min_w(px(0.))
            .text_size(s.px(TEXT_LABEL))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.open_location(path.clone(), line, cx);
            }))
            .child(
                Icon::new(IconName::MessageSquareText)
                    .size(s.px(11.))
                    .text_color(theme.text_accent),
            )
            .child(
                div().min_w(px(0.)).truncate().child(
                    div()
                        .text_color(theme.text)
                        .child(SharedString::from(card.file_name().to_string())),
                ),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_color(theme.text_muted)
                    .child(SharedString::from(card.header_detail())),
            );

        let code = card.shown_code().map(|code| {
            let lines: Vec<&str> = code.lines().collect();
            let expanded = self.expanded_cards.contains(&(index, block));
            let hidden = lines.len().saturating_sub(CARD_CODE_LINES);
            let shown = if expanded || hidden == 0 {
                lines.join("\n")
            } else {
                lines[..CARD_CODE_LINES].join("\n")
            };
            let mut column = v_flex()
                .debug_selector(move || format!("comment-card-code-{index}-{ordinal}"))
                .w_full()
                .px_1p5()
                .py_0p5()
                .rounded(s.px(CARD_RADIUS))
                .bg(theme.bg_editor)
                .font_family(mono_family(cx))
                .text_size(s.px(TEXT_CODE))
                .text_color(theme.text)
                .child(div().whitespace_normal().child(SharedString::from(shown)));
            if hidden > 0 {
                let label = if expanded {
                    "Mostrar menos".to_string()
                } else {
                    format!("… {hidden} líneas más")
                };
                column = column.child(
                    div()
                        .id(("comment-card-more", index * 4096 + block))
                        .debug_selector(move || format!("comment-card-more-{index}-{ordinal}"))
                        .text_size(s.px(TEXT_LABEL))
                        .text_color(theme.text_muted)
                        .cursor_pointer()
                        .hover(move |style| style.text_color(theme.text))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.toggle_comment_card(index, block, cx);
                        }))
                        .child(SharedString::from(label)),
                );
            }
            if card.truncated_lines > 0 && (expanded || hidden == 0) {
                column = column.child(
                    div()
                        .text_size(s.px(TEXT_LABEL))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(format!(
                            "({} líneas más no se enviaron)",
                            card.truncated_lines
                        ))),
                );
            }
            column
        });

        v_flex()
            .id(("comment-card", index * 4096 + block))
            .debug_selector(move || format!("comment-card-{index}-{ordinal}"))
            .w_full()
            .min_w(px(0.))
            .p(s.px(8.))
            .gap_1()
            .rounded(s.px(CARD_RADIUS))
            .bg(theme.bg_surface)
            .border_1()
            .border_color(theme.border)
            .child(header)
            .children(code)
            .child(
                div()
                    .debug_selector(move || format!("comment-card-text-{index}-{ordinal}"))
                    .text_size(s.px(TEXT_BODY))
                    .text_color(theme.text)
                    .whitespace_normal()
                    .child(SharedString::from(card.text.clone())),
            )
            .when(card.not_sent, |this| {
                this.child(
                    div()
                        .debug_selector(move || format!("comment-card-not-sent-{index}-{ordinal}"))
                        .text_size(s.px(TEXT_SMALL))
                        .text_color(theme.status_warning)
                        .child(COMMENT_NOT_SENT),
                )
            })
            .into_any_element()
    }
}
