//! Comments for the agent, wired into the workspace
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §6.7).
//!
//! The comments live in the [`cincel_review::ReviewStore`] next to the hunks
//! (E7-B); the editor draws them and reports what the user does with a
//! [`CommentAction`] (E7-C); the chat shows the unsent ones as tags and the
//! sent ones as cards (E7-E). This module is the glue, a child of
//! `review.rs` like `review_snapshot.rs`:
//!
//! - **Editor → store.** Every editor of a tab is subscribed to its
//!   [`CommentAction`]s ([`Review::sync_comment_editors`], run by every
//!   refresh); `Create`/`Edit`/`Delete` become `add_comment`/`edit_comment`/
//!   `remove_comment`, each followed by a refresh (the editors get the new
//!   list through `EditorView::set_comments`, the chat its tags through
//!   `ChatPanel::set_pending_comments`, the tree and the status bar their
//!   counters) and a debounced save.
//! - **Chat → store.** `ChatEvent::RemoveComment` deletes; `OpenLocation`
//!   opens the file at that line ([`Review::attach_chat`]).
//! - **Anchors.** A commented file stays watched (`ensure_buffer`) even out
//!   of the review; every buffer event also goes to
//!   `store.comment_buffer_event` (`Review::flush`).
//! - **Sending.** [`Review::begin_prompt`] takes the comments
//!   (`take_comments_for_prompt`) and joins them with the patches of the
//!   previous turns (`format_feedback`) into the same
//!   `<user_review_feedback>` block as always; `Agents::prepare_prompt`
//!   attaches their cards to the message ([`sent_cards`]). A prompt that
//!   never leaves (cancelled while it waits for the photo, or the agent went
//!   away meanwhile) gives them back ([`Review::prompt_not_sent`], D16).
//! - **Binary and missing files.** No comments on a binary (the store
//!   refuses them, and the center keeps the binary tab's menu and shortcut
//!   from opening a box); a commented file that turns binary during a turn,
//!   or disappears outside one, loses its comments with a notice.
//! - **Persistence.** With the review (`state.json` v2): `load` reopens the
//!   commented buffers and warns about the comments it dropped.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use cincel_chat::{
    COMMENT_STATE_ACCEPTED, COMMENT_STATE_MIXED, COMMENT_STATE_NO_AGENT_CHANGE,
    COMMENT_STATE_PENDING, COMMENT_STATE_REJECTED, ChatEvent, ChatPanel, PendingComment,
    SentCommentCard, SentCommentKind, TagSource, comment_tag_labels, comment_tag_tooltip,
};
use cincel_editor::{CommentAction, EditorView, ReviewCommentView};
use cincel_review::{
    CommentDropReason, CommentId, CommentState, CommentView, FileStatus, HunkId, SentComment,
    SentRange,
};
use cincel_text::{BufferSnapshot, Point};
use gpui::{
    Context, Entity, EntityId, Focusable as _, SharedString, Subscription, WeakEntity, Window,
};

use super::{DELETED_FILE_HUNK, Review, file_name};

/// What [`Review::begin_prompt`] hands the prompt that is going out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptFeedback {
    /// The review turn the prompt starts (`EditSource::Agent { turn_id }`).
    pub turn: u64,
    /// The inside of `<user_review_feedback>` (`cincel-acp` wraps it):
    /// the patches of the previous turns, then the comments (§6.8).
    pub feedback: Option<String>,
    /// The comments that leave with it, in the order the agent reads them
    /// (their cards, [`sent_cards`]).
    pub sent: Vec<SentComment>,
}

/// The comment side of a [`Review`].
#[derive(Default)]
pub(super) struct CommentGlue {
    /// The chat whose composer shows the tags.
    chat: Option<WeakEntity<ChatPanel>>,
    _chat_subscription: Option<Subscription>,
    /// The editors of the tabs, each subscribed to its comment actions.
    editors: HashMap<EntityId, Subscription>,
    /// What the last prompt took, in case it never leaves (D16).
    last_sent: Vec<SentComment>,
}

impl CommentGlue {
    /// A project closed or opened: nothing taken belongs to the new one.
    pub(super) fn reset(&mut self) {
        self.last_sent.clear();
    }

    /// Paths of the comments the last prompt took: their buffers stay
    /// watched until the next prompt, so a restore finds them where they are.
    pub(super) fn in_flight(&self, path: &Path) -> bool {
        self.last_sent.iter().any(|sent| sent.path == path)
    }
}

/// "1 comentario" / "N comentarios".
pub fn comments_label(count: usize) -> String {
    super::count_label(count, "comentario", "comentarios")
}

/// The status bar's review counter (§6.2.9): "3 cambios pendientes · 2
/// comentarios", "2 comentarios" alone, or the changes alone (also with
/// none, as before the comments).
pub fn status_label(pending: usize, comments: usize) -> String {
    let changes = if pending == 1 {
        "1 cambio pendiente".to_string()
    } else {
        format!("{pending} cambios pendientes")
    };
    match (pending, comments) {
        (_, 0) => changes,
        (0, _) => comments_label(comments),
        _ => format!("{changes} · {}", comments_label(comments)),
    }
}

/// The close dialog's extra line (D17), when there are unsent comments.
pub fn close_comments_line(comments: usize) -> Option<String> {
    match comments {
        0 => None,
        1 => Some(
            "También hay 1 comentario sin enviar: se guarda y vuelve al abrir la carpeta."
                .to_string(),
        ),
        n => Some(format!(
            "También hay {n} comentarios sin enviar: se guardan y vuelven al abrir la carpeta."
        )),
    }
}

/// The notice of a comment that was dropped (§6.2.11, §6.2.12).
pub fn comment_dropped_message(path: &Path, reason: CommentDropReason) -> String {
    let why = match reason {
        CommentDropReason::Missing => "el archivo ya no existe",
        CommentDropReason::NotText => "el archivo ya no es de texto",
    };
    format!(
        "El comentario sobre «{}» se descartó: {why}",
        file_name(path)
    )
}

/// The chat's word for the state of a sent comment.
pub fn state_label(state: CommentState) -> &'static str {
    match state {
        CommentState::Pending => COMMENT_STATE_PENDING,
        CommentState::Accepted => COMMENT_STATE_ACCEPTED,
        CommentState::Rejected => COMMENT_STATE_REJECTED,
        CommentState::Mixed => COMMENT_STATE_MIXED,
        CommentState::NoAgentChange => COMMENT_STATE_NO_AGENT_CHANGE,
    }
}

fn card_kind(kind: SentRange) -> SentCommentKind {
    match kind {
        SentRange::Lines => SentCommentKind::Lines,
        SentRange::RemovedBefore => SentCommentKind::RemovedBefore,
        SentRange::DeletedFile => SentCommentKind::DeletedFile,
    }
}

/// The cards of the message the comments left with (`MessageBlock::Comment`).
pub fn sent_cards(sent: &[SentComment]) -> Vec<SentCommentCard> {
    sent.iter()
        .map(|comment| SentCommentCard {
            display_path: comment.display_path.clone(),
            path: comment.path.clone(),
            first_line: comment.first_line,
            last_line: comment.last_line,
            kind: card_kind(comment.kind),
            state_label: state_label(comment.state).to_string(),
            code: comment.code.clone(),
            removed: comment.removed.clone(),
            truncated_lines: comment.truncated_lines,
            lang: comment.lang.clone(),
            text: comment.text.clone(),
            not_sent: false,
        })
        .collect()
}

impl Review {
    // ------------------------------------------------------------ wiring

    /// Shows the unsent comments as tags in `chat`'s composer and follows
    /// its `×` (`RemoveComment`) and its clicks (`OpenLocation`).
    pub fn attach_chat(
        &mut self,
        chat: &Entity<ChatPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscription = cx.subscribe_in(
            chat,
            window,
            |this, _, event: &ChatEvent, window, cx| match event {
                ChatEvent::RemoveComment { id } => {
                    this.remove_comment(CommentId(*id), cx);
                }
                ChatEvent::OpenLocation { path, line } => {
                    this.open_location(path, *line, window, cx);
                }
                _ => {}
            },
        );
        self.comments.chat = Some(chat.downgrade());
        self.comments._chat_subscription = Some(subscription);
        self.push_pending_comments(cx);
    }

    /// Subscribes to the comment actions of every editor of a tab (new tabs,
    /// and the new editor of a tab that was rebuilt), forgets closed ones and
    /// gives each its file name (the box's title).
    pub(super) fn sync_comment_editors(&mut self, cx: &mut Context<Self>) {
        let Some(center) = self.center.upgrade() else {
            return;
        };
        let editors: Vec<(PathBuf, Entity<EditorView>)> = center
            .read(cx)
            .tabs()
            .iter()
            .map(|tab| (tab.path.clone(), tab.editor().clone()))
            .collect();
        self.comments
            .editors
            .retain(|id, _| editors.iter().any(|(_, editor)| editor.entity_id() == *id));
        for (path, editor) in editors {
            if self.comments.editors.contains_key(&editor.entity_id()) {
                continue;
            }
            let name = SharedString::from(file_name(&path));
            editor.update(cx, |editor, cx| editor.set_display_name(Some(name), cx));
            let subscription = cx.subscribe(&editor, move |this, _, action: &CommentAction, cx| {
                this.on_comment_action(&path, action, cx);
            });
            self.comments
                .editors
                .insert(editor.entity_id(), subscription);
        }
    }

    fn on_comment_action(&mut self, path: &Path, action: &CommentAction, cx: &mut Context<Self>) {
        match action {
            CommentAction::Create {
                rows,
                from_hunk,
                text,
            } => {
                self.add_comment(path, rows.clone(), *from_hunk, text.clone(), cx);
            }
            CommentAction::Edit { id, text } => {
                self.edit_comment(CommentId(*id), text.clone(), cx);
            }
            CommentAction::Delete { id } => {
                self.remove_comment(CommentId(*id), cx);
            }
        }
    }

    // ------------------------------------------------------------ editing

    /// A new comment on `rows` (buffer rows, end exclusive; empty = before
    /// `rows.start`) of `path`. `from_hunk`: the hunk whose "Comentar" made it
    /// ([`DELETED_FILE_HUNK`] for the tab of a file the agent deleted: the
    /// comment covers the whole deleted text). `None` for blank text, a
    /// binary file or a file Cincel cannot read.
    pub fn add_comment(
        &mut self,
        path: &Path,
        rows: Range<u32>,
        from_hunk: Option<u64>,
        text: String,
        cx: &mut Context<Self>,
    ) -> Option<CommentId> {
        self.flush(None, cx);
        let deleted = self.store.file(path).and_then(|file| match &file.status {
            FileStatus::Deleted { previous } => Some(previous.to_string()),
            _ => None,
        });
        let (snapshot, rows, from_hunk) = match deleted {
            Some(previous) => {
                let snapshot = cincel_text::Buffer::new(&previous).snapshot();
                let lines = snapshot.line_count().max(1);
                (snapshot, 0..lines, None)
            }
            None => {
                let handle = self.ensure_buffer(path, cx)?;
                let snapshot = handle.lock().snapshot();
                let from_hunk = from_hunk.filter(|id| *id != DELETED_FILE_HUNK).map(HunkId);
                (snapshot, rows, from_hunk)
            }
        };
        let id = self
            .store
            .add_comment(path, &snapshot, rows, from_hunk, text);
        if id.is_some() {
            tracing::debug!(path = %path.display(), "comentario agregado");
        }
        self.comments_changed(Some(path), cx);
        id
    }

    /// Replaces a comment's text (blank text deletes it).
    pub fn edit_comment(&mut self, id: CommentId, text: String, cx: &mut Context<Self>) -> bool {
        let path = self.comment_path(id);
        let changed = self.store.edit_comment(id, text);
        self.comments_changed(path.as_deref(), cx);
        changed
    }

    /// Deletes a comment (the `×` of its tag, "Borrar" in the margin).
    pub fn remove_comment(&mut self, id: CommentId, cx: &mut Context<Self>) -> bool {
        let path = self.comment_path(id);
        let removed = self.store.remove_comment(id);
        self.comments_changed(path.as_deref(), cx);
        removed
    }

    fn comment_path(&self, id: CommentId) -> Option<PathBuf> {
        self.store
            .comments(|_| None)
            .into_iter()
            .find(|comment| comment.id == id)
            .map(|comment| comment.path)
    }

    /// After a change of the comments: repaint them everywhere and save.
    fn comments_changed(&mut self, path: Option<&Path>, cx: &mut Context<Self>) {
        self.prune(cx);
        self.refresh(path, cx);
        self.schedule_persist(cx);
    }

    /// Drops every comment of `path` with its notice.
    pub(super) fn drop_comments(
        &mut self,
        path: &Path,
        reason: CommentDropReason,
        cx: &mut Context<Self>,
    ) {
        let dropped = self.store.drop_comments_in(path);
        if dropped == 0 {
            return;
        }
        tracing::info!(path = %path.display(), ?reason, dropped, "comentarios descartados");
        crate::toast::warn(comment_dropped_message(path, reason), cx);
        self.prune(cx);
        self.refresh(None, cx);
        self.schedule_persist(cx);
    }

    /// `path` changed on disk during a turn: a commented file that is not
    /// text any more loses its comments.
    pub(super) fn drop_comments_if_binary(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.store.comment_count_in(path) == 0 || !path.is_file() {
            return;
        }
        let binary = std::fs::read(path)
            .ok()
            .is_some_and(|bytes| cincel_project::looks_binary(&bytes));
        if binary {
            self.drop_comments(path, CommentDropReason::NotText, cx);
        }
    }

    // ------------------------------------------------------------ queries

    /// Every unsent comment, by path then row (the tests read it).
    pub fn comment_views(&self) -> Vec<CommentView> {
        self.store.comments(|path| self.comment_snapshot(path))
    }

    /// Number of unsent comments.
    pub fn comment_count(&self) -> usize {
        self.store.comment_count()
    }

    /// The live buffer of a watched path.
    fn comment_snapshot(&self, path: &Path) -> Option<BufferSnapshot> {
        self.buffers
            .get(path)
            .map(|watched| watched.handle.lock().snapshot())
    }

    /// The comments the editor of `path` draws. In the read-only tab of a
    /// file the agent deleted, every comment hangs from its single hunk.
    pub(super) fn editor_comments(
        &self,
        views: &[CommentView],
        path: &Path,
        deleted_review: bool,
    ) -> Vec<ReviewCommentView> {
        views
            .iter()
            .filter(|comment| comment.path == path)
            .map(|comment| ReviewCommentView {
                id: comment.id.0,
                rows: comment.rows.clone(),
                text: comment.text.clone(),
                from_hunk: if deleted_review {
                    Some(DELETED_FILE_HUNK)
                } else {
                    comment.from_hunk.map(|hunk| hunk.0)
                },
            })
            .collect()
    }

    /// The tags of the chat's composer, in the same order as the comments.
    pub(super) fn pending_tags(&self, views: &[CommentView]) -> Vec<PendingComment> {
        let kinds: Vec<SentCommentKind> = views
            .iter()
            .map(|comment| {
                if self.is_pending_deletion(&comment.path) {
                    SentCommentKind::DeletedFile
                } else if comment.rows.is_empty() {
                    SentCommentKind::RemovedBefore
                } else {
                    SentCommentKind::Lines
                }
            })
            .collect();
        let sources: Vec<TagSource<'_>> = views
            .iter()
            .zip(&kinds)
            .map(|(comment, kind)| {
                let first_line = comment.rows.start + 1;
                TagSource {
                    display_path: &comment.display_path,
                    first_line,
                    last_line: if comment.rows.is_empty() {
                        first_line
                    } else {
                        comment.rows.end
                    },
                    kind: *kind,
                }
            })
            .collect();
        let labels = comment_tag_labels(&sources);
        views
            .iter()
            .zip(sources.iter().zip(labels))
            .map(|(comment, (source, label))| PendingComment {
                id: comment.id.0,
                label,
                tooltip: comment_tag_tooltip(*source, &comment.text),
                path: comment.path.clone(),
                line: source.first_line,
            })
            .collect()
    }

    /// Pushes the comments to the editors of `path` (every tab with `None`)
    /// and the tags to the chat; called by every refresh.
    pub(super) fn push_comments(
        &mut self,
        editors: &[(PathBuf, Entity<EditorView>, bool)],
        cx: &mut Context<Self>,
    ) {
        let views = self.comment_views();
        for (path, editor, deleted_review) in editors {
            let comments = self.editor_comments(&views, path, *deleted_review);
            editor.update(cx, |editor, cx| editor.set_comments(comments, cx));
        }
        let tags = self.pending_tags(&views);
        self.set_chat_tags(tags, cx);
    }

    fn push_pending_comments(&mut self, cx: &mut Context<Self>) {
        let views = self.comment_views();
        let tags = self.pending_tags(&views);
        self.set_chat_tags(tags, cx);
    }

    fn set_chat_tags(&mut self, tags: Vec<PendingComment>, cx: &mut Context<Self>) {
        if let Some(chat) = self.comments.chat.as_ref().and_then(WeakEntity::upgrade) {
            chat.update(cx, |chat, cx| chat.set_pending_comments(tags, cx));
        }
    }

    // ------------------------------------------------------------ sending

    /// Takes every unsent comment for the prompt that is going out (§6.7),
    /// keeping them aside until the next prompt in case this one never
    /// leaves.
    pub(super) fn take_comments(&mut self) -> Vec<SentComment> {
        let buffers = &self.buffers;
        let sent = self.store.take_comments_for_prompt(|path| {
            buffers.get(path).map(|watched| {
                let buffer = watched.handle.lock();
                (buffer.snapshot(), buffer.is_dirty())
            })
        });
        self.comments.last_sent = sent.clone();
        sent
    }

    /// The prompt [`Review::begin_prompt`] prepared never left (D16): its
    /// comments go back to the margin and to the chat. Returns whether there
    /// were any (the chat then marks their cards "No se envió").
    pub fn prompt_not_sent(&mut self, cx: &mut Context<Self>) -> bool {
        let sent = std::mem::take(&mut self.comments.last_sent);
        if sent.is_empty() {
            return false;
        }
        self.flush(None, cx);
        let buffers = &self.buffers;
        self.store.restore_comments(&sent, |path| {
            buffers
                .get(path)
                .map(|watched| watched.handle.lock().snapshot())
        });
        tracing::info!(
            count = sent.len(),
            "el mensaje no salió: los comentarios vuelven"
        );
        self.refresh(None, cx);
        self.schedule_persist(cx);
        true
    }

    // ------------------------------------------------------------ chat

    /// A tag or a card's header: opens `path` at `line` (1-based).
    fn open_location(
        &mut self,
        path: &Path,
        line: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(center) = self.center.upgrade() else {
            return;
        };
        center.update(cx, |center, cx| center.open_file(path, true, window, cx));
        let editor = center
            .read(cx)
            .tabs()
            .iter()
            .find(|tab| tab.path == path)
            .map(|tab| tab.editor().clone());
        if let Some(editor) = editor {
            let row = line.saturating_sub(1);
            editor.update(cx, |editor, cx| editor.set_cursor(Point::new(row, 0), cx));
            let focus = editor.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
    }

    // ------------------------------------------------------------ loading

    /// After `store.load`: reopens the commented buffers (their anchors
    /// follow the user from the first keystroke) and warns about the
    /// comments that could not come back.
    pub(super) fn after_comments_loaded(
        &mut self,
        report: &cincel_review::LoadReport,
        cx: &mut Context<Self>,
    ) {
        for (path, reason) in &report.comments_dropped {
            crate::toast::warn(comment_dropped_message(path, *reason), cx);
        }
        if report.comments_relocated > 0 {
            tracing::info!(
                relocated = report.comments_relocated,
                "comentarios reubicados al abrir"
            );
        }
        for path in self.store.commented_paths() {
            if path.is_file() {
                self.ensure_buffer(&path, cx);
            }
        }
    }
}
