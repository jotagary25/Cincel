//! The images and the review comments of [`ChatPanel`]: the state half
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §5.3.2, §6.6).
//!
//! The three ways in — `Ctrl+V`, a drop from the system file manager, the
//! paperclip button — end in [`ChatPanel::attach_images`], which checks the
//! connection's capability **before** reading anything (D6), caps the draft at
//! [`MAX_IMAGES_PER_MESSAGE`] and prepares each file on the background
//! executor ([`crate::attachments::prepare_image`]). The comments are only
//! shown here: the workspace owns them (`cincel-review`) and the panel emits
//! what the user did with them. Painting lives in [`crate::render_media`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use cincel_acp::acp::schema::v1::ContentBlock;
use gpui::{ClipboardEntry, Context, Window};

use crate::attachments::{
    self, ImageError, MAX_IMAGES_PER_MESSAGE, NO_CONNECTION_FOR_IMAGES, PASTED_IMAGE_NAME,
    PreparedImage, TOO_MANY_IMAGES,
};
use crate::comments::{PendingComment, SentCommentCard};
use crate::events::ChatEvent;
use crate::model::{ChatImageSource, Entry, ImageRef, MessageBlock, NoticeLevel};
use crate::panel::ChatPanel;

/// One image of the draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAttachment {
    /// Unique within the panel; [`ChatPanel::remove_attachment`] takes it.
    pub id: u64,
    /// File name, or "Imagen pegada".
    pub name: String,
    /// Still being prepared, or ready to travel.
    pub state: AttachmentState,
}

/// Where a [`PendingAttachment`] is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachmentState {
    /// Being read and prepared on the background executor: the thumbnail
    /// shows a spinner and `Enter` does not send.
    Processing,
    /// Ready: this is what travels.
    Ready(PreparedImage),
}

/// What to attach: a file, or bytes that came without one (the clipboard).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachmentSource {
    /// A file on disk (dropped, picked, copied in the file manager).
    Path(PathBuf),
    /// Bytes with the name to show.
    Bytes {
        /// "Imagen pegada" for the clipboard.
        name: String,
        /// The encoded image.
        bytes: Vec<u8>,
    },
}

impl AttachmentSource {
    fn name(&self) -> String {
        match self {
            AttachmentSource::Path(path) => attachments::display_name(path),
            AttachmentSource::Bytes { name, .. } => name.clone(),
        }
    }
}

/// Thumbnails decoded once and the answers to "is the stored file there",
/// kept across frames and dropped with the conversation.
#[derive(Default)]
pub(crate) struct MediaCache {
    images: HashMap<String, Arc<gpui::Image>>,
    files: HashMap<PathBuf, bool>,
}

impl MediaCache {
    pub(crate) fn clear(&mut self) {
        self.images.clear();
        self.files.clear();
    }

    /// The GPUI image of `bytes`, built once per `key`.
    pub(crate) fn image(
        &mut self,
        key: &str,
        mime_type: &str,
        bytes: &Arc<[u8]>,
    ) -> Arc<gpui::Image> {
        self.images
            .entry(key.to_string())
            .or_insert_with(|| {
                let format =
                    gpui::ImageFormat::from_mime_type(mime_type).unwrap_or(gpui::ImageFormat::Png);
                Arc::new(gpui::Image::from_bytes(format, bytes.to_vec()))
            })
            .clone()
    }

    /// Whether `path` is a file, asked once.
    pub(crate) fn exists(&mut self, path: &Path) -> bool {
        *self
            .files
            .entry(path.to_path_buf())
            .or_insert_with(|| path.is_file())
    }
}

/// The image of a replayed `user_message_chunk` (`session/load`), shown from
/// its data and never stored apart (§5.1).
pub(crate) fn replayed_image(block: &ContentBlock) -> Option<ImageRef> {
    let ContentBlock::Image(image) = block else {
        return None;
    };
    let data = base64::engine::general_purpose::STANDARD
        .decode(image.data.as_bytes())
        .ok()?;
    let (sniffed_mime, width, height) =
        attachments::sniff_image(&data).unwrap_or(("image/png", 0, 0));
    let mime_type = if image.mime_type.is_empty() {
        sniffed_mime.to_string()
    } else {
        image.mime_type.clone()
    };
    Some(ImageRef {
        file: String::new(),
        name: "Imagen".to_string(),
        mime_type,
        width,
        height,
        bytes_len: data.len() as u64,
        data: Some(Arc::from(data)),
    })
}

/// What tells two images apart: the SHA-256 of the file name, or of the
/// bytes for an image that has no file.
pub(crate) fn image_identity(image: &ImageRef) -> Option<String> {
    if !image.file.is_empty() {
        return Some(image.sha256().to_string());
    }
    image.data.as_deref().map(attachments::sha256_hex)
}

impl ChatPanel {
    // ------------------------------------------------------------ images

    /// The images of the draft, in the order they were attached.
    #[must_use]
    pub fn attachments(&self) -> &[PendingAttachment] {
        &self.attachments
    }

    /// Whether the active connection accepts images (`None`: no connection,
    /// or it has not said yet).
    #[must_use]
    pub fn image_support(&self) -> Option<bool> {
        self.image_support
    }

    /// Tells the panel whether the active connection accepts images: the
    /// workspace calls it on `Connected` and when the connection changes
    /// (`None` = no connection). [`ChatPanel::handle_event`] already sets it
    /// from `AgentEvent::Connected` (`cincel_acp::agent_supports_images`).
    pub fn set_image_support(&mut self, support: Option<bool>, cx: &mut Context<Self>) {
        if self.image_support != support {
            self.image_support = support;
            cx.notify();
        }
    }

    /// The folder of the conversation on screen
    /// (`<state>/conversations/<id>/`): sent images whose bytes are not in
    /// memory are painted from its `images/` folder (D7).
    pub fn set_conversation_dir(&mut self, dir: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.conversation_dir != dir {
            self.conversation_dir = dir;
            self.media_cache.borrow_mut().clear();
            cx.notify();
        }
    }

    /// The folder [`ChatPanel::set_conversation_dir`] set.
    #[must_use]
    pub fn conversation_dir(&self) -> Option<&Path> {
        self.conversation_dir.as_deref()
    }

    /// Whether some image of the draft is still being prepared ("Preparando
    /// N imágenes…"; `Enter` waits).
    #[must_use]
    pub fn is_preparing_images(&self) -> bool {
        self.preparing_count() > 0
    }

    pub(crate) fn preparing_count(&self) -> usize {
        self.attachments
            .iter()
            .filter(|attachment| attachment.state == AttachmentState::Processing)
            .count()
    }

    /// Whether the draft has images the active connection does not take: the
    /// message waits until they are removed (§5.1).
    #[must_use]
    pub fn images_block_send(&self) -> bool {
        self.image_support == Some(false) && !self.attachments.is_empty()
    }

    /// The images of the draft that are ready, in order.
    pub(crate) fn ready_images(&self) -> impl Iterator<Item = &PreparedImage> {
        self.attachments
            .iter()
            .filter_map(|attachment| match &attachment.state {
                AttachmentState::Ready(image) => Some(image),
                AttachmentState::Processing => None,
            })
    }

    /// The label the image notices name the agent by.
    pub(crate) fn connection_name(&self) -> String {
        self.active_connection().map_or_else(
            || "El agente".to_string(),
            |connection| connection.label.clone(),
        )
    }

    /// Emits a pop-up notice for the workspace.
    pub(crate) fn notify_user(
        &mut self,
        level: NoticeLevel,
        text: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        cx.emit(ChatEvent::Notify {
            level,
            text: text.into(),
        });
    }

    /// D6: without a connection, or with one that does not take images,
    /// nothing is attached and the user is told why.
    fn may_attach(&mut self, cx: &mut Context<Self>) -> bool {
        match self.image_support {
            Some(true) => true,
            Some(false) => {
                let text = attachments::no_images_text(&self.connection_name());
                self.notify_user(NoticeLevel::Warning, text, cx);
                false
            }
            None => {
                self.notify_user(NoticeLevel::Warning, NO_CONNECTION_FOR_IMAGES, cx);
                false
            }
        }
    }

    /// The paperclip button and `chat::attach_image`: asks the workspace for
    /// the file dialog ([`ChatEvent::PickImages`]) when images can be
    /// attached; the files it picks come back through
    /// [`ChatPanel::attach_paths`].
    pub fn request_pick_images(&mut self, cx: &mut Context<Self>) {
        if self.may_attach(cx) {
            cx.emit(ChatEvent::PickImages);
        }
    }

    /// Attaches files (a drop on the box, the file dialog, the tree). Each one
    /// is checked by content: a `.txt` among them is refused with its notice.
    pub fn attach_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let sources = paths.iter().cloned().map(AttachmentSource::Path).collect();
        self.attach_images(sources, cx);
    }

    /// The one place every way of attaching ends in (§5.1).
    pub fn attach_images(&mut self, sources: Vec<AttachmentSource>, cx: &mut Context<Self>) {
        if sources.is_empty() || !self.may_attach(cx) {
            return;
        }
        let free = MAX_IMAGES_PER_MESSAGE.saturating_sub(self.attachments.len());
        if sources.len() > free {
            self.notify_user(NoticeLevel::Warning, TOO_MANY_IMAGES, cx);
        }
        for source in sources.into_iter().take(free) {
            let id = self.next_attachment_id;
            self.next_attachment_id += 1;
            self.attachments.push(PendingAttachment {
                id,
                name: source.name(),
                state: AttachmentState::Processing,
            });
            let work = cx.background_executor().spawn(async move {
                match source {
                    AttachmentSource::Path(path) => attachments::read_and_prepare(&path),
                    AttachmentSource::Bytes { name, bytes } => {
                        attachments::prepare_image(&bytes, &name)
                    }
                }
            });
            cx.spawn(async move |this, cx| {
                let result = work.await;
                this.update(cx, |this, cx| this.finish_attachment(id, result, cx))
                    .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// A prepared image comes back: it becomes ready, or leaves the draft
    /// with the reason.
    fn finish_attachment(
        &mut self,
        id: u64,
        result: Result<PreparedImage, ImageError>,
        cx: &mut Context<Self>,
    ) {
        // Removed with its `×` while it was being prepared.
        let Some(index) = self
            .attachments
            .iter()
            .position(|attachment| attachment.id == id)
        else {
            return;
        };
        match result {
            Ok(image) => {
                if image.animation_dropped {
                    let text = attachments::animation_dropped_text(&image.name);
                    self.notify_user(NoticeLevel::Info, text, cx);
                }
                self.attachments[index].state = AttachmentState::Ready(image);
            }
            Err(error) => {
                let attachment = self.attachments.remove(index);
                self.notify_user(NoticeLevel::Warning, error.message(&attachment.name), cx);
            }
        }
        cx.notify();
    }

    /// The `×` of a thumbnail: the image leaves the draft and does not travel.
    pub fn remove_attachment(&mut self, id: u64, cx: &mut Context<Self>) {
        let before = self.attachments.len();
        self.attachments.retain(|attachment| attachment.id != id);
        if self.attachments.len() != before {
            cx.notify();
        }
    }

    /// A file "dragged" from Cincel's tree (`Mencionar en el chat`): an image
    /// is attached as an image, anything else is still an `@` mention (the
    /// author's answer to §10).
    pub fn mention_or_attach(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if attachments::has_image_extension(&path) {
            self.attach_paths(&[path], cx);
        } else {
            self.insert_mention(path, window, cx);
        }
    }

    /// `Ctrl+V` in the composer, before the editor pastes: an image in the
    /// clipboard, or image files copied in the file manager (as
    /// `ExternalPaths`, or as `file://` lines, which is how Wayland hands them
    /// over), are attached and nothing is pasted; anything else falls through
    /// to the editor, which pastes the text as always.
    pub(crate) fn on_composer_paste(
        &mut self,
        _: &cincel_editor::Paste,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let mut sources: Vec<AttachmentSource> = Vec::new();
        let mut text = String::new();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => sources.push(AttachmentSource::Bytes {
                    name: PASTED_IMAGE_NAME.to_string(),
                    bytes: image.bytes.clone(),
                }),
                ClipboardEntry::ExternalPaths(paths) => sources.extend(
                    paths
                        .paths()
                        .iter()
                        .filter(|path| attachments::has_image_extension(path))
                        .cloned()
                        .map(AttachmentSource::Path),
                ),
                ClipboardEntry::String(string) => text.push_str(string.text()),
            }
        }
        if sources.is_empty()
            && let Some(paths) = attachments::file_uri_paths(&text)
        {
            sources.extend(
                paths
                    .into_iter()
                    .filter(|path| attachments::has_image_extension(path))
                    .map(AttachmentSource::Path),
            );
        }
        if sources.is_empty() {
            return;
        }
        cx.stop_propagation();
        self.attach_images(sources, cx);
    }

    /// A thumbnail of a sent message was clicked: the workspace opens the
    /// viewer ([`ChatEvent::OpenImage`]).
    pub fn open_image(&mut self, entry: usize, block: usize, cx: &mut Context<Self>) {
        let Some(Entry::UserMessage(message)) = self.entries.get(entry) else {
            return;
        };
        let Some(MessageBlock::Image(image)) = message.blocks.get(block) else {
            return;
        };
        let source = match (&image.data, self.image_path(image)) {
            (Some(data), _) => ChatImageSource::Bytes {
                mime_type: image.mime_type.clone(),
                data: data.clone(),
            },
            (None, Some(path)) => ChatImageSource::File(path),
            (None, None) => return,
        };
        cx.emit(ChatEvent::OpenImage {
            source,
            name: image.name.clone(),
            width: image.width,
            height: image.height,
            bytes_len: image.bytes_len,
        });
    }

    /// Where a stored image lives, when the panel knows the conversation's
    /// folder.
    pub(crate) fn image_path(&self, image: &ImageRef) -> Option<PathBuf> {
        if image.file.is_empty() {
            return None;
        }
        Some(
            self.conversation_dir
                .as_ref()?
                .join("images")
                .join(&image.file),
        )
    }

    // ---------------------------------------------------------- comments

    /// The unsent comments, as tags inside the composer box (§6.6), ordered
    /// by file and line.
    pub fn set_pending_comments(
        &mut self,
        mut comments: Vec<PendingComment>,
        cx: &mut Context<Self>,
    ) {
        comments.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        if self.pending_comments != comments {
            self.pending_comments = comments;
            cx.notify();
        }
    }

    /// The tags of the composer box.
    #[must_use]
    pub fn pending_comments(&self) -> &[PendingComment] {
        &self.pending_comments
    }

    /// The `×` of a tag: the tag goes now and the workspace deletes the
    /// comment ([`ChatEvent::RemoveComment`]), margin included.
    pub fn remove_comment(&mut self, id: u64, cx: &mut Context<Self>) {
        self.pending_comments.retain(|comment| comment.id != id);
        cx.emit(ChatEvent::RemoveComment { id });
        cx.notify();
    }

    /// A tag or a card's header: open the file at that line.
    pub fn open_location(&mut self, path: PathBuf, line: u32, cx: &mut Context<Self>) {
        cx.emit(ChatEvent::OpenLocation { path, line });
    }

    /// Adds the comments that left with the last message to it, as cards
    /// ([`MessageBlock::Comment`], stored with the conversation).
    pub fn attach_sent_comments(&mut self, cards: Vec<SentCommentCard>, cx: &mut Context<Self>) {
        if cards.is_empty() {
            return;
        }
        let Some(message) = self.entries.iter_mut().rev().find_map(|entry| match entry {
            Entry::UserMessage(message) => Some(message),
            _ => None,
        }) else {
            tracing::warn!("llegaron comentarios enviados sin un mensaje del usuario");
            return;
        };
        message
            .blocks
            .extend(cards.into_iter().map(MessageBlock::Comment));
        cx.notify();
    }

    /// D16: the last message with comment cards never left; its cards say
    /// "No se envió: el comentario volvió al margen".
    pub fn mark_comments_not_sent(&mut self, cx: &mut Context<Self>) {
        let message = self.entries.iter_mut().rev().find_map(|entry| match entry {
            Entry::UserMessage(message)
                if message
                    .blocks
                    .iter()
                    .any(|block| matches!(block, MessageBlock::Comment(_))) =>
            {
                Some(message)
            }
            _ => None,
        });
        let Some(message) = message else {
            return;
        };
        for block in &mut message.blocks {
            if let MessageBlock::Comment(card) = block {
                card.not_sent = true;
            }
        }
        cx.notify();
    }

    /// Unfolds (or folds back) the code of a comment card ("… N líneas más").
    pub fn toggle_comment_card(&mut self, entry: usize, block: usize, cx: &mut Context<Self>) {
        if !self.expanded_cards.remove(&(entry, block)) {
            self.expanded_cards.insert((entry, block));
        }
        cx.notify();
    }
}
