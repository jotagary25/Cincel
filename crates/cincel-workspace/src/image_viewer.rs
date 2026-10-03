//! The image viewer: a sent image at full size, floating over the whole
//! window (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §5.1,
//! §5.2, §5.3.5, D8).
//!
//! The chat panel is narrow, so the viewer is a layer of the workspace and not
//! of the panel (D8): it floats, like the file finder, and moves nothing. A
//! click on a thumbnail of a sent message makes the chat emit
//! `ChatEvent::OpenImage`; [`Workspace::open_image_viewer`] shows it. `Esc`,
//! a click outside the image or the `×` close it and the keyboard goes back
//! where it was. While it is up [`Workspace::is_modal_open`] is `true`, so the
//! panel shortcuts and `Ctrl+L` do nothing.

use std::path::PathBuf;
use std::sync::Arc;

use cincel_chat::ChatImageSource;
use cincel_chat::attachments::image_summary;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, MouseButton, ObjectFit, Window, img,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{div, px};

use crate::theme::ThemeColors;
use crate::workspace::Workspace;

/// The GPUI key context of the layer; the default keymap binds `Esc` against
/// it (`workspace::close_image_viewer`, §7.1, §7.2).
pub const KEY_CONTEXT: &str = "ImageViewer";

/// Space kept free around the image, on every side (§5.2), before `ui_scale`.
pub const VIEWER_MARGIN: f32 = 32.;
/// Opacity of the `bg.app` veil that covers the window (§5.2: "al 85 %").
const VEIL_OPACITY: f32 = 0.85;
/// Height of the caption row under the image (12 px text plus its gap).
const CAPTION_HEIGHT: f32 = 28.;
/// Size of the caption text (§5.2).
const CAPTION_TEXT: f32 = 12.;

/// `workspace::close_image_viewer`: `Esc` while the viewer is up.
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = workspace, name = "close_image_viewer")]
pub struct CloseImageViewer;

/// Where the pixels of the image on screen come from.
enum ViewerSource {
    /// The file under `conversations/<id>/images/`.
    File(PathBuf),
    /// Bytes that are still in memory (not stored yet, or a replayed image).
    Bytes(Arc<gpui::Image>),
}

/// What `ChatEvent::OpenImage` carries: the pixels and the caption's data.
#[derive(Clone, Debug)]
pub struct ViewerImage {
    /// Where the bytes are (a stored file, or memory).
    pub source: ChatImageSource,
    /// File name it was attached with.
    pub name: String,
    /// Width in pixels (`0` when unknown).
    pub width: u32,
    /// Height in pixels (`0` when unknown).
    pub height: u32,
    /// Size in bytes.
    pub bytes_len: u64,
}

/// The image on screen and what the caption says about it.
struct Shown {
    source: ViewerSource,
    name: String,
    width: u32,
    height: u32,
    bytes_len: u64,
}

/// The size the image is painted at inside `available` (width, height in
/// pixels): its own size when it fits, scaled down on the tighter axis when it
/// does not (§5.3.5: `min(1, (ventana − 64 px) / imagen)` por eje, keeping the
/// proportions). An image of unknown size (`0 × 0`) takes the whole box.
#[must_use]
pub fn fitted_size(available: (f32, f32), image: (u32, u32)) -> (f32, f32) {
    let (available_w, available_h) = (available.0.max(1.), available.1.max(1.));
    if image.0 == 0 || image.1 == 0 {
        return (available_w, available_h);
    }
    let (width, height) = (image.0 as f32, image.1 as f32);
    let scale = (available_w / width).min(available_h / height).min(1.);
    (width * scale, height * scale)
}

/// The floating viewer. Built once per window; shows nothing until
/// [`ImageViewer::open`].
pub struct ImageViewer {
    shown: Option<Shown>,
    /// What had the keyboard when it opened, to give it back on close.
    previous_focus: Option<FocusHandle>,
    focus_handle: FocusHandle,
}

impl ImageViewer {
    /// A closed viewer.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            shown: None,
            previous_focus: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Whether an image is on screen.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.shown.is_some()
    }

    /// The caption of the image on screen ("nombre · ancho × alto · tamaño").
    #[must_use]
    pub fn caption(&self) -> Option<String> {
        self.shown
            .as_ref()
            .map(|shown| image_summary(&shown.name, shown.width, shown.height, shown.bytes_len))
    }

    /// Whether the image on screen is read from a stored file (and not from
    /// bytes in memory).
    #[must_use]
    pub fn is_showing_file(&self) -> bool {
        matches!(
            self.shown,
            Some(Shown {
                source: ViewerSource::File(_),
                ..
            })
        )
    }

    /// Shows an image full size and takes the keyboard. `previous_focus` is
    /// what to give the keyboard back to; opening a second image while one is
    /// already up keeps the first one's.
    pub fn open(
        &mut self,
        image: ViewerImage,
        previous_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ViewerImage {
            source,
            name,
            width,
            height,
            bytes_len,
        } = image;
        let source = match source {
            ChatImageSource::File(path) => ViewerSource::File(path),
            ChatImageSource::Bytes { mime_type, data } => {
                let format =
                    gpui::ImageFormat::from_mime_type(&mime_type).unwrap_or(gpui::ImageFormat::Png);
                ViewerSource::Bytes(Arc::new(gpui::Image::from_bytes(format, data.to_vec())))
            }
        };
        if self.shown.is_none() {
            self.previous_focus = previous_focus;
        }
        self.shown = Some(Shown {
            source,
            name,
            width,
            height,
            bytes_len,
        });
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Closes the viewer and gives the keyboard back to where it was.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shown.take().is_none() {
            return;
        }
        if let Some(previous) = self.previous_focus.take() {
            window.focus(&previous, cx);
        }
        cx.notify();
    }

    fn on_close(&mut self, _: &CloseImageViewer, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }
}

impl Focusable for ImageViewer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ImageViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(shown) = &self.shown else {
            return div().into_any_element();
        };
        let theme = ThemeColors::global(cx).clone();
        let scale = crate::settings::ui_scale(cx);
        let margin = VIEWER_MARGIN * scale;
        let caption_height = CAPTION_HEIGHT * scale;
        let viewport = window.viewport_size();
        let available = (
            f32::from(viewport.width) - 2. * margin,
            f32::from(viewport.height) - 2. * margin - caption_height,
        );
        let (width, height) = fitted_size(available, (shown.width, shown.height));
        let picture = match &shown.source {
            ViewerSource::File(path) => img(path.clone()),
            ViewerSource::Bytes(image) => img(image.clone()),
        }
        .size_full()
        .object_fit(ObjectFit::Contain);
        let caption = image_summary(&shown.name, shown.width, shown.height, shown.bytes_len);

        div()
            .id("image-viewer")
            .debug_selector(|| "image-viewer".to_string())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_close))
            .absolute()
            .inset_0()
            // Keeps the mouse and the wheel from reaching what is behind.
            .occlude()
            .bg(theme.bg_app.opacity(VEIL_OPACITY))
            // A click anywhere that is not the image closes it (§5.1): the
            // image, the caption and the `×` stop their own mouse-downs.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseDownEvent, window, cx| {
                    this.close(window, cx);
                }),
            )
            .child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap(px(8. * scale))
                    .child(
                        div()
                            .id("image-viewer-image")
                            .debug_selector(|| "image-viewer-image".to_string())
                            .flex_none()
                            .w(px(width))
                            .h(px(height))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(picture),
                    )
                    .child(
                        div()
                            .id("image-viewer-caption")
                            .debug_selector(|| "image-viewer-caption".to_string())
                            .flex_none()
                            .text_size(px(CAPTION_TEXT * scale))
                            .text_color(theme.text_muted)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(caption),
                    ),
            )
            .child(
                h_flex()
                    .absolute()
                    .top(px(12. * scale))
                    .right(px(12. * scale))
                    .child(
                        div()
                            .debug_selector(|| "image-viewer-close".to_string())
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                Button::new("image-viewer-close")
                                    .icon(IconName::X)
                                    .small()
                                    .ghost()
                                    .tooltip("Cerrar")
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.close(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Workspace {
    /// A thumbnail of a sent message was clicked (`ChatEvent::OpenImage`):
    /// shows the image full size over the whole window. What had the keyboard
    /// (the chat's box, usually) gets it back when the viewer closes.
    pub fn open_image_viewer(
        &mut self,
        image: ViewerImage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let viewer = self.image_viewer().clone();
        let previous = window.focused(cx);
        viewer.update(cx, |viewer, cx| viewer.open(image, previous, window, cx));
        cx.notify();
    }

    /// Whether the image viewer is on screen.
    #[must_use]
    pub fn is_image_viewer_open(&self, cx: &App) -> bool {
        self.image_viewer().read(cx).is_open()
    }
}

/// The viewer, as the workspace builds it.
pub(crate) fn build(cx: &mut Context<Workspace>) -> Entity<ImageViewer> {
    cx.new(ImageViewer::new)
}
