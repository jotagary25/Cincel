//! Comments for the agent inside the editor
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §6.2–§6.5).
//!
//! - **Where they come from**: the host's list ([`EditorView::set_comments`]),
//!   plus the one box being written (`CommentDraft`, one per view).
//! - **How they show**: each comment is a block of the [`BlockMap`] below the
//!   rows it talks about (below the hunk whose "Comentar" made it, below its
//!   red rows for a pure deletion): open (the box with its field), folded
//!   (one row, the text cut with "…") or expanded (the whole text, read
//!   only). Its first row gets a mark in the margin. Blocks take whole rows,
//!   so the gutter and the text column never move (D16).
//! - **The field** is an `EditorView` of its own ([`EditorChrome::Minimal`],
//!   soft wrap, 2 to 8 rows), so accents, IME, undo and paste work there as in
//!   the chat composer. Its key context adds `comment_box`, where `Ctrl+Enter`
//!   saves and `Esc` cancels; both bubble from the field up to this view.
//! - **What the host hears**: [`CommentAction`]s, a second event type of the
//!   view. Nothing is stored here: after "Guardar" the box closes and the
//!   comment shows once the host sends the new list.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext, ClickEvent, Context, DismissEvent, Entity, Focusable as _,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point,
    SharedString, StatefulInteractiveElement, Styled, Subscription, TextRun, Window, div, px, svg,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::PopupMenu;

use super::{EditorEvent, EditorView};
use crate::actions::{CancelComment, CommentHunk, CommentSelection, SaveComment};
use crate::block_map::{BlockMap, BlockPlacement};
use crate::comments::{
    CommentAction, CommentBlockKind, DraftTarget, NEW_COMMENT_BLOCK, ReviewCommentView, clamp_text,
    help_text, one_line, range_label, texts,
};
use crate::display_map::DisplayRow;
use crate::settings::{AutoHeight, EditorChrome, EditorSettings};
use crate::theme;
use crate::wrap_map::WrapRow;

/// Font size of the text of a comment (02-visual §3: interface, 13 px).
pub const COMMENT_FONT_SIZE: f32 = 13.;
/// Font size of the title and the help of the open box.
pub const COMMENT_SMALL_FONT_SIZE: f32 = 11.;
/// Line height of the comment text, as a multiple of its size.
pub const COMMENT_LINE_HEIGHT: f32 = 1.5;
/// Padding of the open box.
pub const COMMENT_BOX_PADDING: f32 = 8.;
/// Vertical padding of a folded or expanded box (§6.3: 4 px).
pub const COMMENT_FOLDED_PADDING_Y: f32 = 4.;
/// Horizontal padding of a folded or expanded box.
pub const COMMENT_FOLDED_PADDING_X: f32 = 8.;
/// Corner radius of every box (§6.3: 6).
pub const COMMENT_BOX_RADIUS: f32 = 6.;
/// Room between a box and the scrollbar (§6.3: 12 px).
pub const COMMENT_BOX_RIGHT_MARGIN: f32 = 12.;
/// Height of the title row of the open box.
pub const COMMENT_TITLE_HEIGHT: f32 = 16.;
/// Gap between the rows of the open box.
pub const COMMENT_BOX_GAP: f32 = 6.;
/// Height of the footer of the open box (help and buttons).
pub const COMMENT_FOOTER_HEIGHT: f32 = 22.;
/// Padding inside the field's frame.
pub const COMMENT_FIELD_PADDING: f32 = 4.;
/// Side of the icon of a folded box (§6.3: 12 px).
pub const COMMENT_ICON_SIZE: f32 = 12.;
/// Width kept at the right of a folded or expanded box for "Editar" and
/// "Borrar", shown or not, so the text never re-wraps on hover.
pub const COMMENT_BUTTONS_WIDTH: f32 = 120.;
/// Rows of the field (§6.3: from 2 to 8, then it scrolls).
pub const COMMENT_FIELD_ROWS: (u32, u32) = (2, 8);

/// The box being written: what it writes and its field.
pub(crate) struct CommentDraft {
    pub target: DraftTarget,
    pub editor: Entity<EditorView>,
    _events: Subscription,
}

/// The open context menu: gpui-kit's `PopupMenu`, owned by the view and
/// dropped as soon as it is dismissed (an item chosen, `Esc`, a click out).
pub(crate) struct EditorContextMenu {
    pub menu: Entity<PopupMenu>,
    /// Where the right click was, in window coordinates.
    pub position: Point<Pixels>,
    _dismiss: Subscription,
}

/// One comment block as the last frame laid it out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BlockLayout {
    pub id: u64,
    pub kind: CommentBlockKind,
    /// Index into `comments`, `None` for the box of a new comment.
    pub comment: Option<usize>,
    /// Height of the box itself.
    pub height: Pixels,
    /// Rows the block takes.
    pub rows: u32,
    /// Rows of the field (open box) or lines of the text (expanded box).
    pub lines: u32,
    /// Display row of the margin mark (`None` for a new comment).
    pub mark_row: Option<DisplayRow>,
}

/// Line height of a comment's text at `scale`.
pub fn comment_text_line_height(scale: f32) -> f32 {
    COMMENT_FONT_SIZE * COMMENT_LINE_HEIGHT * scale
}

/// Height of a folded box: a row of text plus 4 px above and below, and the
/// border.
pub fn folded_box_height(scale: f32) -> f32 {
    comment_text_line_height(scale) + 2. * COMMENT_FOLDED_PADDING_Y * scale + 2.
}

/// Height of an expanded box of `lines` lines.
pub fn expanded_box_height(lines: u32, scale: f32) -> f32 {
    lines.max(1) as f32 * comment_text_line_height(scale)
        + 2. * COMMENT_FOLDED_PADDING_Y * scale
        + 2.
}

/// Height of the field's frame for `rows` rows.
pub fn field_frame_height(rows: u32, scale: f32) -> f32 {
    rows as f32 * comment_text_line_height(scale) + 2. * COMMENT_FIELD_PADDING * scale + 2.
}

/// Height of the open box with a field of `rows` rows.
pub fn open_box_height(rows: u32, scale: f32) -> f32 {
    2. + 2. * COMMENT_BOX_PADDING * scale
        + COMMENT_TITLE_HEIGHT * scale
        + COMMENT_BOX_GAP * scale
        + field_frame_height(rows, scale)
        + COMMENT_BOX_GAP * scale
        + COMMENT_FOOTER_HEIGHT * scale
}

/// Whole rows a box `height` px tall takes (D10).
pub fn block_rows(height: f32, line_height: f32) -> u32 {
    ((height / line_height.max(1.)).ceil() as u32).max(1)
}

// -- host API ------------------------------------------------------------------

impl EditorView {
    /// Replaces the unsent comments of the file (the host calls it on every
    /// change of its store, like [`EditorView::set_review`]). The editor shows
    /// each one folded below its rows with a mark in the margin, keeps which
    /// ones are expanded, and closes the box of a comment that went away.
    /// A text field ([`EditorChrome::Minimal`]) keeps them but shows none.
    pub fn set_comments(&mut self, mut comments: Vec<ReviewCommentView>, cx: &mut Context<Self>) {
        comments.sort_by_key(|comment| (comment.rows.start, comment.id));
        if self.comments == comments {
            return;
        }
        self.comment_expanded
            .retain(|id, _| comments.iter().any(|comment| comment.id == *id));
        if let Some(DraftTarget::Edit { id }) = self.comment_draft.as_ref().map(|d| &d.target)
            && !comments.iter().any(|comment| comment.id == *id)
        {
            self.comment_draft = None;
        }
        self.comments = comments;
        cx.notify();
    }

    /// The comments being shown, by row.
    pub fn comments(&self) -> &[ReviewCommentView] {
        &self.comments
    }

    /// The file name the box titles use ("Comentario para el agente ·
    /// calc.py:24-28"); without one they say "líneas 24-28".
    pub fn set_display_name(&mut self, name: Option<SharedString>, cx: &mut Context<Self>) {
        if self.display_name != name {
            self.display_name = name;
            cx.notify();
        }
    }

    /// What the open box writes, if one is open.
    pub fn comment_draft(&self) -> Option<&DraftTarget> {
        self.comment_draft.as_ref().map(|draft| &draft.target)
    }

    /// The field of the open box, if one is open.
    pub fn comment_draft_editor(&self) -> Option<Entity<EditorView>> {
        self.comment_draft
            .as_ref()
            .map(|draft| draft.editor.clone())
    }

    /// Whether a saved comment shows its whole text.
    pub fn is_comment_expanded(&self, id: u64) -> bool {
        self.comment_expanded.get(&id).copied().unwrap_or(false)
    }

    /// Folds or unfolds a saved comment (a click on its text or its mark).
    pub fn toggle_comment(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.comments.iter().any(|comment| comment.id == id) {
            return;
        }
        let expanded = self.comment_expanded.entry(id).or_insert(false);
        *expanded = !*expanded;
        cx.notify();
    }

    /// The buffer rows `editor::comment_selection` comments with a selection:
    /// the rows it covers (one ending at column 0 does not take that row),
    /// phantom rows counting as the real row they sit on.
    pub fn comment_rows_for_selection(&self) -> Range<u32> {
        self.selected_buffer_rows()
    }

    /// The pill's "Comentar" on a hunk: opens its box below it, or the box of
    /// the comment its button already made, for editing. Works while the
    /// agent writes (it is not a decision).
    pub fn comment_hunk(&mut self, hunk: u64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.comments_enabled() {
            return;
        }
        let Some(ix) = self.review.index_of(hunk) else {
            return;
        };
        if let Some(existing) = self
            .comments
            .iter()
            .find(|comment| comment.from_hunk == Some(hunk))
            .map(|comment| comment.id)
        {
            self.edit_comment(existing, window, cx);
            return;
        }
        let rows = self.review.hunks[ix].buffer_rows.clone();
        self.open_comment_box(
            DraftTarget::New {
                rows,
                from_hunk: Some(hunk),
            },
            String::new(),
            window,
            cx,
        );
    }

    /// "Editar" on a saved comment: its box opens with its text.
    pub fn edit_comment(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self
            .comments
            .iter()
            .find(|comment| comment.id == id)
            .map(|comment| comment.text.clone())
        else {
            return;
        };
        self.open_comment_box(DraftTarget::Edit { id }, text, window, cx);
    }

    /// "Borrar" on a saved comment: asks the host to delete it.
    pub fn delete_comment(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.comments.iter().any(|comment| comment.id == id) {
            return;
        }
        if matches!(self.comment_draft(), Some(DraftTarget::Edit { id: editing }) if *editing == id)
        {
            self.comment_draft = None;
        }
        window.focus(&self.focus_handle, cx);
        cx.emit(CommentAction::Delete { id });
        cx.notify();
    }

    /// `Ctrl+Enter` or "Guardar": a new comment is created, an edited one
    /// changes; an empty text cancels a new one and deletes an edited one.
    pub fn save_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.comment_draft.take() else {
            return;
        };
        let typed = draft.editor.read(cx).text();
        let text = typed.trim();
        let text = clamp_text(text).unwrap_or_else(|| text.to_string());
        match draft.target {
            DraftTarget::New { rows, from_hunk } => {
                if !text.is_empty() {
                    cx.emit(CommentAction::Create {
                        rows,
                        from_hunk,
                        text,
                    });
                }
            }
            DraftTarget::Edit { id } => {
                let before = self
                    .comments
                    .iter()
                    .find(|comment| comment.id == id)
                    .map(|comment| comment.text.clone());
                if text.is_empty() {
                    cx.emit(CommentAction::Delete { id });
                } else if before.as_deref() != Some(text.as_str()) {
                    cx.emit(CommentAction::Edit { id, text });
                }
            }
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// `Esc` or "Cancelar": what was typed is dropped (an edited comment
    /// stays as it was).
    pub fn cancel_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.comment_draft.take().is_some() {
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    /// Whether this view shows comments at all: never in a text field.
    pub(crate) fn comments_enabled(&self) -> bool {
        self.settings.chrome == EditorChrome::Full && !self.comment_box
    }

    /// Opens the context menu at a window position (D12) with
    /// [`EditorView::context_menu_entries`]. Its actions go to the editor's
    /// focus, so they run the same handlers, and show the same keys, as the
    /// keyboard.
    pub(crate) fn open_context_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = self.context_menu_entries();
        if entries.is_empty() {
            return;
        }
        let focus = self.focus_handle.clone();
        let menu = PopupMenu::build(window, cx, move |menu, _, _| {
            let mut menu = menu.action_context(focus);
            for entry in entries {
                menu = match entry {
                    Some((label, action)) => menu.menu(label, action),
                    None => menu.separator(),
                };
            }
            menu
        });
        let dismiss = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.context_menu = None;
            cx.notify();
        });
        let menu_focus = menu.read(cx).focus_handle(cx);
        window.focus(&menu_focus, cx);
        self.context_menu = Some(EditorContextMenu {
            menu,
            position,
            _dismiss: dismiss,
        });
        cx.notify();
    }

    /// Whether the context menu is open.
    pub fn is_context_menu_open(&self) -> bool {
        self.context_menu.is_some()
    }

    /// Opens the box (replacing any other open one) and gives its field the
    /// keyboard.
    fn open_comment_box(
        &mut self,
        target: DraftTarget,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.comment_field_settings();
        let theme = self.theme;
        let registry = self.registry.clone();
        let buffer = crate::settings::shared(cincel_text::Buffer::new(&text));
        let editor =
            cx.new(|cx| EditorView::new(buffer, None, registry, settings, theme, window, cx));
        editor.update(cx, |field, cx| {
            field.comment_box = true;
            field.set_placeholder(Some(SharedString::from(texts::PLACEHOLDER)), cx);
            let end = field.text().len();
            field.set_text(&text, end, cx);
        });
        let events = cx.subscribe(&editor, |_, field, event: &EditorEvent, cx| {
            if matches!(
                event,
                EditorEvent::CursorMoved { .. } | EditorEvent::DirtyChanged(_)
            ) {
                // §6.2.3: at most 8 000 characters.
                let text = field.read(cx).text();
                if let Some(clamped) = clamp_text(&text) {
                    field.update(cx, |field, cx| {
                        let end = clamped.len();
                        field.set_text(&clamped, end, cx);
                    });
                }
                cx.notify();
            }
        });
        let focus = editor.read(cx).focus_handle.clone();
        let block = match &target {
            DraftTarget::New { .. } => NEW_COMMENT_BLOCK,
            DraftTarget::Edit { id } => *id,
        };
        self.comment_draft = Some(CommentDraft {
            target,
            editor,
            _events: events,
        });
        self.autoscroll_block = Some(block);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// The settings of a box's field: the interface font at 13 px (scaled),
    /// soft wrap, 2 to 8 rows, no gutter.
    fn comment_field_settings(&self) -> EditorSettings {
        let scale = self.settings.scale();
        let ui = self.style.ui_font.family.to_string();
        EditorSettings {
            soft_wrap: true,
            auto_close_pairs: false,
            cursor_blink: self.settings.cursor_blink,
            font_family: self.settings.font_family.clone(),
            font_size: COMMENT_FONT_SIZE * scale,
            line_height: COMMENT_LINE_HEIGHT,
            chrome: EditorChrome::Minimal,
            auto_height: Some(AutoHeight {
                min_rows: COMMENT_FIELD_ROWS.0,
                max_rows: COMMENT_FIELD_ROWS.1,
            }),
            prose_font_family: Some(vec![ui]),
            ui_scale: self.settings.ui_scale,
            ..EditorSettings::default()
        }
    }

    /// Keeps the comment rows (and the rows of a new box) aligned after an
    /// edit, until the host sends the refreshed list: the same rule as the
    /// hunks.
    pub(crate) fn adjust_comments(&mut self, edit_row: u32, delta: i64) {
        let shift = |row: u32| -> u32 { (row as i64 + delta).max(0) as u32 };
        let adjust = |rows: &mut Range<u32>| {
            if rows.start > edit_row {
                *rows = shift(rows.start)..shift(rows.end);
            } else if rows.contains(&edit_row) {
                *rows = rows.start..shift(rows.end).max(rows.start);
            }
        };
        for comment in &mut self.comments {
            adjust(&mut comment.rows);
        }
        if let Some(CommentDraft {
            target: DraftTarget::New { rows, .. },
            ..
        }) = self.comment_draft.as_mut()
        {
            adjust(rows);
        }
    }
}

// -- actions -------------------------------------------------------------------

impl EditorView {
    /// `editor::comment_selection` (`Ctrl+Shift+M`, "Comentar selección" /
    /// "Comentar línea"): the rows of the selection, or the cursor's row;
    /// without a selection inside a pending hunk, that hunk's "Comentar". In
    /// the read-only tab of a deleted file it comments its single hunk.
    pub(crate) fn on_comment_selection(
        &mut self,
        _: &CommentSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.comments_enabled() {
            cx.propagate();
            return;
        }
        if self.read_only && self.review.hunks.len() == 1 {
            let hunk = self.review.hunks[0].id;
            self.comment_hunk(hunk, window, cx);
            return;
        }
        if !self.has_selection()
            && let Some(ix) = self.hunk_under_cursor()
        {
            let hunk = self.review.hunks[ix].id;
            self.comment_hunk(hunk, window, cx);
            return;
        }
        let rows = self.comment_rows_for_selection();
        self.open_comment_box(
            DraftTarget::New {
                rows,
                from_hunk: None,
            },
            String::new(),
            window,
            cx,
        );
    }

    /// `review::comment_hunk`: the "Comentar" of the hunk under the cursor.
    pub(crate) fn on_comment_hunk(
        &mut self,
        _: &CommentHunk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.comments_enabled() {
            cx.propagate();
            return;
        }
        if let Some(ix) = self.hunk_under_cursor() {
            let hunk = self.review.hunks[ix].id;
            self.comment_hunk(hunk, window, cx);
        }
    }

    /// `editor::save_comment`: bubbles up from the field to the view that
    /// owns the box.
    pub(crate) fn on_save_comment(
        &mut self,
        _: &SaveComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.comment_draft.is_none() {
            cx.propagate();
            return;
        }
        self.save_comment(window, cx);
    }

    /// `editor::cancel_comment`: same path as saving.
    pub(crate) fn on_cancel_comment(
        &mut self,
        _: &CancelComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.comment_draft.is_none() {
            cx.propagate();
            return;
        }
        self.cancel_comment(window, cx);
    }
}

// -- layout --------------------------------------------------------------------

impl EditorView {
    /// The wrap row a comment's block hangs from: the last row of the hunk
    /// whose "Comentar" made it (its red rows for a pure deletion), else the
    /// last row of its range; an empty range hangs from the row above
    /// `rows.start` (the last red row of a deletion there).
    fn comment_anchor_row(&self, rows: &Range<u32>, from_hunk: Option<u64>) -> WrapRow {
        let diff = self.display_map.diff();
        let hunk_range = from_hunk
            .and_then(|id| self.review.index_of(id))
            .filter(|ix| *ix < diff.hunks().len())
            .map(|ix| diff.hunk_display_range(ix))
            .filter(|range| !range.is_empty());
        let display = match hunk_range {
            Some(range) => range.end - 1,
            None => {
                let line_count = self.snapshot.line_count().max(1);
                if rows.is_empty() {
                    let below = rows.start.min(line_count);
                    let display = if below >= line_count {
                        self.display_map.display_row_count()
                    } else {
                        self.display_map.to_display_row(below)
                    };
                    display.saturating_sub(1)
                } else {
                    let last = (rows.end - 1).min(line_count - 1);
                    self.display_map.to_display_row(last)
                }
            }
        };
        let display = self.display_map.clip_row(display);
        self.wrap.to_wrap_row(display, u32::MAX)
    }

    /// The display row of a comment's margin mark: the first row of its
    /// hunk, else the first row of its range.
    fn comment_mark_row(&self, comment: &ReviewCommentView) -> DisplayRow {
        let diff = self.display_map.diff();
        if let Some(range) = comment
            .from_hunk
            .and_then(|id| self.review.index_of(id))
            .filter(|ix| *ix < diff.hunks().len())
            .map(|ix| diff.hunk_display_range(ix))
            .filter(|range| !range.is_empty())
        {
            return range.start;
        }
        let row = comment
            .rows
            .start
            .min(self.snapshot.line_count().saturating_sub(1));
        self.display_map.to_display_row(row)
    }

    /// Width of a comment box: from the text area's left edge to 12 px before
    /// the scrollbar.
    pub(crate) fn comment_box_width(&self, text_width: Pixels) -> Pixels {
        (text_width - px(COMMENT_BOX_RIGHT_MARGIN * self.settings.scale())).max(px(40.))
    }

    /// Lays out the comment blocks for this frame and returns their
    /// placements (the element builds the [`BlockMap`] from them).
    pub(crate) fn layout_comment_blocks(
        &mut self,
        box_width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<BlockPlacement> {
        self.block_layouts.clear();
        if !self.comments_enabled() {
            self.comment_draft = None;
            return Vec::new();
        }
        let scale = self.settings.scale();
        let line_height = f32::from(self.style.line_height);
        let draft_target = self
            .comment_draft
            .as_ref()
            .map(|draft| draft.target.clone());
        let field_rows = self.comment_field_rows(box_width, window, cx);
        let mut placements = Vec::new();
        let mut layouts = Vec::new();
        for (ix, comment) in self.comments.iter().enumerate() {
            let editing =
                matches!(&draft_target, Some(DraftTarget::Edit { id }) if *id == comment.id);
            let (kind, height, lines) = if editing {
                (
                    CommentBlockKind::Open,
                    open_box_height(field_rows, scale),
                    field_rows,
                )
            } else if self.is_comment_expanded(comment.id) {
                let lines = self.comment_text_lines(&comment.text, box_width, window);
                (
                    CommentBlockKind::Expanded,
                    expanded_box_height(lines, scale),
                    lines,
                )
            } else {
                (CommentBlockKind::Folded, folded_box_height(scale), 1)
            };
            let rows = block_rows(height, line_height);
            placements.push(BlockPlacement {
                id: comment.id,
                after_wrap_row: self.comment_anchor_row(&comment.rows, comment.from_hunk),
                rows,
            });
            layouts.push(BlockLayout {
                id: comment.id,
                kind,
                comment: Some(ix),
                height: px(height),
                rows,
                lines,
                mark_row: Some(self.comment_mark_row(comment)),
            });
        }
        if let Some(DraftTarget::New { rows, from_hunk }) = &draft_target {
            let height = open_box_height(field_rows, scale);
            let block_rows = block_rows(height, line_height);
            placements.push(BlockPlacement {
                id: NEW_COMMENT_BLOCK,
                after_wrap_row: self.comment_anchor_row(rows, *from_hunk),
                rows: block_rows,
            });
            layouts.push(BlockLayout {
                id: NEW_COMMENT_BLOCK,
                kind: CommentBlockKind::Open,
                comment: None,
                height: px(height),
                rows: block_rows,
                lines: field_rows,
                mark_row: None,
            });
        }
        self.block_layouts = layouts;
        placements
    }

    /// Rows of the open box's field at the width it will be laid out at,
    /// measured now (the field wraps before the block map is built, so the
    /// box and its rows agree on the frame that paints them).
    fn comment_field_rows(
        &mut self,
        box_width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u32 {
        let Some(editor) = self
            .comment_draft
            .as_ref()
            .map(|draft| draft.editor.clone())
        else {
            return COMMENT_FIELD_ROWS.0;
        };
        let scale = self.settings.scale();
        let field_width = box_width
            - px(2. + 2. * COMMENT_BOX_PADDING * scale)
            - px(2. + 2. * COMMENT_FIELD_PADDING * scale);
        let rows = editor.update(cx, |field, cx| {
            field.sync_buffer(cx);
            let metrics = crate::element::gutter_metrics(field, window);
            let text_width =
                (field_width - metrics.gutter_width - px(crate::element::SCROLLBAR_WIDTH))
                    .max(px(1.));
            field.ensure_wrap(text_width, metrics.char_width, window);
            field.wrap_row_count()
        });
        rows.clamp(COMMENT_FIELD_ROWS.0, COMMENT_FIELD_ROWS.1)
    }

    /// Width of the text of a folded or expanded box.
    fn comment_text_width(&self, box_width: Pixels) -> Pixels {
        let scale = self.settings.scale();
        (box_width
            - px(2. + 2. * COMMENT_FOLDED_PADDING_X * scale)
            - px((COMMENT_ICON_SIZE + 2. * COMMENT_BOX_GAP + COMMENT_BUTTONS_WIDTH) * scale))
        .max(px(20.))
    }

    /// Lines the whole text of a comment wraps into in an expanded box.
    fn comment_text_lines(&self, text: &str, box_width: Pixels, window: &Window) -> u32 {
        if text.is_empty() {
            return 1;
        }
        let scale = self.settings.scale();
        let run = TextRun {
            len: text.len(),
            font: self.style.ui_font.clone(),
            color: gpui::Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_text(
                SharedString::from(text.to_string()),
                px(COMMENT_FONT_SIZE * scale),
                &[run],
                Some(self.comment_text_width(box_width)),
                None,
            )
            .map(|lines| {
                lines
                    .iter()
                    .map(|line| line.wrap_boundaries().len() as u32 + 1)
                    .sum::<u32>()
            })
            .unwrap_or(1)
            .max(1)
    }

    /// Replaces the block map with one over `wrap_rows` rows and `blocks`,
    /// moving the scroll so the row on top of the viewport does not move when
    /// a block above it appeared, went away or changed its height (D10).
    pub(crate) fn update_block_map(&mut self, wrap_rows: u32, blocks: Vec<BlockPlacement>) {
        let line_height = f32::from(self.style.line_height).max(1.);
        let map = BlockMap::new(wrap_rows, blocks);
        let old = std::mem::replace(&mut self.blocks, map);
        if old.signature() != self.blocks.signature() && self.pending_scroll_row.is_none() {
            let old_top = (self.scroll_top / line_height).floor().max(0.) as u32;
            let delta = self.blocks.compensation(&old, old_top);
            if delta != 0 {
                self.scroll_top = (self.scroll_top + delta as f32 * line_height).max(0.);
            }
        }
    }

    /// The layouts of the last frame (for the element and the probe).
    pub(crate) fn block_layout(&self, id: u64) -> Option<&BlockLayout> {
        self.block_layouts.iter().find(|layout| layout.id == id)
    }

    /// Whether "Editar" and "Borrar" show on a saved comment: with the mouse
    /// over its box, or with the cursor inside its rows.
    pub(crate) fn comment_buttons_visible(&self, comment: &ReviewCommentView) -> bool {
        if self.hover_comment == Some(comment.id) {
            return true;
        }
        let row = self.cursor_point().row;
        if comment.rows.is_empty() {
            row == comment.rows.start
        } else {
            comment.rows.contains(&row)
        }
    }

    /// The element of one comment block, `width` wide and `rows × line
    /// height` tall, the box centred in it.
    pub(crate) fn render_comment_block(
        &mut self,
        layout: &BlockLayout,
        width: Pixels,
        block_height: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let body = match layout.kind {
            CommentBlockKind::Open => self.render_open_box(layout, cx),
            CommentBlockKind::Folded | CommentBlockKind::Expanded => {
                self.render_saved_box(layout, width, cx)
            }
        };
        div()
            .w(width)
            .h(block_height)
            .flex()
            .flex_col()
            .justify_center()
            .child(body)
            .into_any_element()
    }

    fn render_open_box(&mut self, layout: &BlockLayout, cx: &mut Context<Self>) -> AnyElement {
        let Some(draft) = self.comment_draft.as_ref() else {
            return div().into_any_element();
        };
        let editor = draft.editor.clone();
        let rows = match &draft.target {
            DraftTarget::New { rows, .. } => rows.clone(),
            DraftTarget::Edit { id } => self
                .comments
                .iter()
                .find(|comment| comment.id == *id)
                .map(|comment| comment.rows.clone())
                .unwrap_or(0..1),
        };
        let theme = self.theme;
        let scale = self.settings.scale();
        let title = format!(
            "{} · {}",
            texts::BOX_TITLE,
            range_label(self.display_name.as_deref(), &rows)
        );
        let chars = editor.read(cx).text().chars().count();
        let help = help_text(chars);
        let muted = theme::color(theme.text_muted);
        let field_focus = editor.read(cx).focus_handle.clone();
        div()
            .id("comment-box")
            .debug_selector(|| "comment-box".to_string())
            .block_mouse_except_scroll()
            .w_full()
            .h(layout.height)
            .flex()
            .flex_col()
            .gap(px(COMMENT_BOX_GAP * scale))
            .p(px(COMMENT_BOX_PADDING * scale))
            .bg(theme::color(theme.surface))
            .border_1()
            .border_color(theme::color(theme.border_focus))
            .rounded(px(COMMENT_BOX_RADIUS * scale))
            .font_family(self.style.ui_font.family.clone())
            .cursor_default()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _: &MouseDownEvent, window, cx| {
                    window.focus(&field_focus, cx);
                }),
            )
            .child(
                div()
                    .debug_selector(|| "comment-box-title".to_string())
                    .h(px(COMMENT_TITLE_HEIGHT * scale))
                    .flex()
                    .items_center()
                    .text_size(px(COMMENT_SMALL_FONT_SIZE * scale))
                    .text_color(muted)
                    .truncate()
                    .child(SharedString::from(title)),
            )
            .child(
                div()
                    .debug_selector(|| "comment-box-field".to_string())
                    .w_full()
                    .h(px(field_frame_height(layout.lines, scale)))
                    .p(px(COMMENT_FIELD_PADDING * scale))
                    .border_1()
                    .border_color(theme::color(theme.border))
                    .rounded(px(4. * scale))
                    .bg(theme::color(theme.background))
                    .overflow_hidden()
                    .child(editor),
            )
            .child(
                div()
                    .h(px(COMMENT_FOOTER_HEIGHT * scale))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8. * scale))
                    .child(
                        div()
                            .debug_selector(|| "comment-box-help".to_string())
                            .min_w_0()
                            .truncate()
                            .text_size(px(COMMENT_SMALL_FONT_SIZE * scale))
                            .text_color(muted)
                            .child(SharedString::from(help)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap(px(4. * scale))
                            .child(
                                div()
                                    .debug_selector(|| "comment-box-cancel".to_string())
                                    .child(
                                        Button::new("comment-box-cancel")
                                            .label(texts::CANCEL)
                                            .ghost()
                                            .xsmall()
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, window, cx| {
                                                    this.cancel_comment(window, cx)
                                                },
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "comment-box-save".to_string())
                                    .child(
                                        Button::new("comment-box-save")
                                            .label(texts::SAVE)
                                            .primary()
                                            .xsmall()
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, window, cx| {
                                                    this.save_comment(window, cx)
                                                },
                                            )),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_saved_box(
        &mut self,
        layout: &BlockLayout,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(comment) = layout.comment.and_then(|ix| self.comments.get(ix)).cloned() else {
            return div().into_any_element();
        };
        let id = comment.id;
        let theme = self.theme;
        let scale = self.settings.scale();
        let expanded = layout.kind == CommentBlockKind::Expanded;
        let show_buttons = self.comment_buttons_visible(&comment);
        let text_width = self.comment_text_width(width);
        let line_height = px(comment_text_line_height(scale));
        let text = if expanded {
            comment.text.clone()
        } else {
            one_line(&comment.text)
        };
        let text_element = div()
            .id(("comment-text", id))
            .debug_selector(move || format!("comment-text-{id}"))
            .text_size(px(COMMENT_FONT_SIZE * scale))
            .line_height(line_height)
            .text_color(theme::color(theme.text))
            .cursor_pointer()
            .when(expanded, |text| text.w(text_width))
            .when(!expanded, |text| text.flex_1().min_w_0().truncate())
            .child(SharedString::from(text))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.toggle_comment(id, cx);
            }));
        let button = |label: &'static str,
                      selector: String,
                      delete: bool,
                      cx: &mut Context<Self>|
         -> AnyElement {
            div()
                .debug_selector(move || selector)
                .child(
                    Button::new((
                        if delete {
                            "comment-delete"
                        } else {
                            "comment-edit"
                        },
                        id,
                    ))
                    .label(label)
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            if delete {
                                this.delete_comment(id, window, cx);
                            } else {
                                this.edit_comment(id, window, cx);
                            }
                        },
                    )),
                )
                .into_any_element()
        };
        let buttons = div()
            .w(px(COMMENT_BUTTONS_WIDTH * scale))
            .flex_none()
            .flex()
            .justify_end()
            .gap(px(4. * scale))
            .when(show_buttons, |buttons| {
                buttons
                    .child(button(texts::EDIT, format!("comment-edit-{id}"), false, cx))
                    .child(button(
                        texts::DELETE,
                        format!("comment-delete-{id}"),
                        true,
                        cx,
                    ))
            });
        div()
            .id(("comment-block", id))
            .debug_selector(move || format!("comment-block-{id}"))
            .block_mouse_except_scroll()
            .w_full()
            .h(layout.height)
            .flex()
            .when(expanded, |row| row.items_start())
            .when(!expanded, |row| row.items_center())
            .gap(px(COMMENT_BOX_GAP * scale))
            .px(px(COMMENT_FOLDED_PADDING_X * scale))
            .py(px(COMMENT_FOLDED_PADDING_Y * scale))
            .bg(theme::color(theme.surface))
            .border_1()
            .border_color(theme::color(theme.border))
            .rounded(px(COMMENT_BOX_RADIUS * scale))
            .font_family(self.style.ui_font.family.clone())
            .cursor_default()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let next = if *hovered {
                    Some(id)
                } else if this.hover_comment == Some(id) {
                    None
                } else {
                    this.hover_comment
                };
                if next != this.hover_comment {
                    this.hover_comment = next;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .h(line_height)
                    .flex()
                    .flex_none()
                    .items_center()
                    .child(
                        svg()
                            .path(IconName::MessageSquareText.path())
                            .size(px(COMMENT_ICON_SIZE * scale))
                            .text_color(theme::color(theme.text_accent)),
                    ),
            )
            .child(text_element)
            .child(buttons)
            .into_any_element()
    }
}
