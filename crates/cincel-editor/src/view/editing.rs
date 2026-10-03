//! The line, bracket and case commands of the editor, plus auto-closed pairs
//! and auto-indent (`editor::move_line_up`, `editor::toggle_comments`, …).
//!
//! Every command works on *buffer* rows: the selection is first mapped through
//! [`EditorView::edit_range`], so a cursor on a phantom row acts on the real
//! row its edits would land on, and phantom text is never edited. Each command
//! is a single `EditSource::User` transaction, so one `Ctrl+Z` undoes it.

use std::ops::Range;

use gpui::{App, ClipboardEntry, ClipboardItem, Context, Global, Window};
use serde::{Deserialize, Serialize};

use super::EditorView;
use crate::actions::*;
use crate::display_map::DisplayCell;
use crate::ops::{self, Edit};
use crate::settings::EditorChrome;

/// How far the bracket matcher walks, in characters, before giving up.
pub const MAX_BRACKET_SCAN: usize = 200_000;

/// The text `Ctrl+C` / `Ctrl+X` last put on the clipboard as a whole line
/// (no selection, R12), or `None` when the last copy was a selection. On
/// Linux the JSON mark of the clipboard item does not always survive the
/// system clipboard, so `Ctrl+V` also treats a clipboard that holds exactly
/// this text as a line.
#[derive(Default)]
pub(crate) struct LineClipboard(pub(crate) Option<String>);

impl Global for LineClipboard {}

/// The JSON mark on the clipboard item of a copied line.
#[derive(Serialize, Deserialize)]
pub(crate) struct LineCopyMetadata {
    pub(crate) whole_line: bool,
}

/// Whether `item` holds a whole line copied by [`EditorView::copy_or_cut`]:
/// its metadata says so, or its text is the remembered [`LineClipboard`].
fn is_line_copy(item: &ClipboardItem, text: &str, cx: &App) -> bool {
    let marked = item.entries().iter().any(|entry| match entry {
        ClipboardEntry::String(string) => string
            .metadata_json::<LineCopyMetadata>()
            .is_some_and(|metadata| metadata.whole_line),
        _ => false,
    });
    marked
        || cx
            .try_global::<LineClipboard>()
            .and_then(|line| line.0.as_deref())
            == Some(text)
}

impl EditorView {
    // -- helpers -------------------------------------------------------------

    /// Buffer rows covered by the selection, end exclusive. A selection that
    /// ends at column 0 of a row does not include that row.
    pub(crate) fn selected_buffer_rows(&self) -> Range<u32> {
        let range = self.edit_range();
        let start = self.snapshot.offset_to_point(range.start);
        let end = self.snapshot.offset_to_point(range.end);
        let end_row = if end.column == 0 && end.row > start.row {
            end.row
        } else {
            end.row + 1
        };
        start.row..end_row.min(self.snapshot.line_count()).max(start.row + 1)
    }

    fn line_end(&self, row: u32) -> usize {
        self.snapshot.line_start_offset(row) + self.snapshot.line_len(row) as usize
    }

    /// `(anchor, cursor)` as buffer offsets.
    pub(super) fn selection_offsets(&self) -> (usize, usize) {
        (
            self.edit_offset(self.selection_anchor),
            self.edit_offset(self.cursor),
        )
    }

    /// Where the selection goes after `edits`: its start does not follow an
    /// insertion made exactly there (so a selection from column 0 still
    /// starts at column 0), its end does.
    pub(super) fn map_selection(&self, edits: &[Edit]) -> (usize, usize) {
        let (anchor, cursor) = self.selection_offsets();
        if anchor == cursor {
            let at = ops::map_offset(cursor, edits);
            return (at, at);
        }
        let (start, end) = (anchor.min(cursor), anchor.max(cursor));
        let start = ops::map_offset_left(start, edits);
        let end = ops::map_offset(end, edits);
        if anchor <= cursor {
            (start, end)
        } else {
            (end, start)
        }
    }

    /// Applies `edits` (sorted, non-overlapping, in current buffer offsets) as
    /// one transaction and puts the selection on `anchor..cursor`, which are
    /// offsets in the text *after* the edits.
    pub(crate) fn apply_edits(
        &mut self,
        edits: Vec<Edit>,
        anchor: usize,
        cursor: usize,
        cx: &mut Context<Self>,
    ) {
        if edits.is_empty() || self.is_read_only() {
            return;
        }
        // Row deltas for the hunks, computed on the text before the edits.
        let shifts: Vec<(u32, i64)> = edits
            .iter()
            .map(|(range, text)| {
                let row = self.snapshot.offset_to_point(range.start).row;
                let removed = self.snapshot.text_in(range.clone()).matches('\n').count() as i64;
                (row, text.matches('\n').count() as i64 - removed)
            })
            .collect();
        self.shift_auto_closers(&edits);
        let borrowed: Vec<(Range<usize>, &str)> = edits
            .iter()
            .map(|(range, text)| (range.clone(), text.as_str()))
            .collect();
        self.edit_buffer(&borrowed, cx);
        // Bottom first, so each row is still in the coordinates it was
        // computed in.
        for (row, delta) in shifts.into_iter().rev() {
            self.adjust_hunks(row, delta);
        }
        self.rebuild_display_map();
        let len = self.snapshot.len();
        self.selection_anchor =
            self.display_point_for_offset(self.snapshot.clip_offset(anchor.min(len)));
        self.cursor = self.display_point_for_offset(self.snapshot.clip_offset(cursor.min(len)));
        self.marked_range = None;
        self.goal_column = None;
        self.after_input(cx);
    }

    /// Selects `anchor..cursor` (buffer offsets) without editing.
    fn select_offsets(&mut self, anchor: usize, cursor: usize, cx: &mut Context<Self>) {
        self.selection_anchor = self.display_point_for_offset(anchor);
        self.cursor = self.display_point_for_offset(cursor);
        self.goal_column = None;
        self.auto_closers.clear();
        self.after_input(cx);
    }

    /// Keeps the auto-inserted closers on their characters across `edits`,
    /// forgetting the ones an edit removed.
    pub(crate) fn shift_auto_closers(&mut self, edits: &[Edit]) {
        if self.auto_closers.is_empty() {
            return;
        }
        self.auto_closers.retain(|offset| {
            !edits
                .iter()
                .any(|(range, _)| range.start <= *offset && *offset < range.end)
        });
        for offset in &mut self.auto_closers {
            *offset = ops::map_offset(*offset, edits);
        }
    }

    /// The character right before and right after a buffer offset, on the same
    /// line (`None` at the start or the end of the line).
    fn chars_around(&self, offset: usize) -> (Option<char>, Option<char>) {
        let point = self.snapshot.offset_to_point(offset);
        let line = self.snapshot.line_text(point.row);
        let column = (point.column as usize).min(line.len());
        let before = line[..column].chars().next_back();
        let after = line[column..].chars().next();
        (before, after)
    }

    /// The language name, for the per-language tables of [`crate::ops`].
    fn language_name(&self) -> Option<&'static str> {
        self.language.as_ref().map(|language| language.name())
    }

    /// Whether typed brackets and quotes auto-close (on by default).
    pub fn auto_close_pairs(&self) -> bool {
        self.auto_close_pairs
    }

    /// Turns auto-closed brackets and quotes on or off.
    ///
    /// [`EditorView::set_settings`](crate::EditorView::set_settings) calls
    /// this with [`EditorSettings::auto_close_pairs`](crate::EditorSettings::auto_close_pairs)
    /// on every hot reload; a host can also call it directly (the tests do).
    pub fn set_auto_close_pairs(&mut self, on: bool) {
        self.auto_close_pairs = on;
        if !on {
            self.auto_closers.clear();
        }
    }

    // -- typing: auto-closed pairs -------------------------------------------

    /// Handles one typed character for the auto-closed pairs. Returns `true`
    /// when it did the edit (or the skip) itself.
    pub(crate) fn type_with_pairs(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if !self.auto_close_pairs {
            return false;
        }
        let mut chars = text.chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else {
            return false;
        };
        let range = self.edit_range();
        if range.start != range.end {
            // An opener wraps the selection and keeps it on the inner text.
            let Some(close) = ops::closer_for(ch) else {
                return false;
            };
            let selected = self.snapshot.text_in(range.clone());
            let inner_start = range.start + ch.len_utf8();
            let inner_end = inner_start + selected.len();
            let reversed = self.cursor < self.selection_anchor;
            let wrapped = format!("{ch}{selected}{close}");
            let (anchor, cursor) = if reversed {
                (inner_end, inner_start)
            } else {
                (inner_start, inner_end)
            };
            self.apply_edits(vec![(range, wrapped)], anchor, cursor, cx);
            return true;
        }
        let offset = range.start;
        let (before, after) = self.chars_around(offset);
        // Typing the closer over one we inserted steps over it.
        if ops::is_closer(ch) && after == Some(ch) && self.auto_closers.contains(&offset) {
            self.auto_closers.retain(|at| *at != offset);
            let point = self.display_point_for_offset(offset + ch.len_utf8());
            self.cursor = point;
            self.selection_anchor = point;
            self.goal_column = None;
            self.after_input(cx);
            return true;
        }
        let Some(close) = ops::closer_for(ch) else {
            return false;
        };
        if !ops::should_auto_close(ch, before, after, self.language_name()) {
            return false;
        }
        let inside = offset + ch.len_utf8();
        self.apply_edits(
            vec![(offset..offset, format!("{ch}{close}"))],
            inside,
            inside,
            cx,
        );
        self.auto_closers.push(inside);
        true
    }

    /// Backspace between an empty pair deletes both halves. Returns `true`
    /// when it did.
    pub(crate) fn backspace_pair(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.auto_close_pairs || self.has_selection() {
            return false;
        }
        if !matches!(
            self.display_map.to_buffer(self.cursor.row),
            DisplayCell::Buffer(_)
        ) {
            return false;
        }
        let offset = self.edit_offset(self.cursor);
        let (Some(before), Some(after)) = self.chars_around(offset) else {
            return false;
        };
        if ops::closer_for(before) != Some(after) {
            return false;
        }
        let start = offset - before.len_utf8();
        let end = offset + after.len_utf8();
        self.apply_edits(vec![(start..end, String::new())], start, start, cx);
        true
    }

    /// `Enter` right between an opener and its closer puts the closer on its
    /// own line at the row's indentation and the cursor on an indented line
    /// in between. Returns `true` when it did.
    pub(crate) fn newline_in_pair(&mut self, cx: &mut Context<Self>) -> bool {
        if self.has_selection()
            || !matches!(
                self.display_map.to_buffer(self.cursor.row),
                DisplayCell::Buffer(_)
            )
        {
            return false;
        }
        let offset = self.edit_offset(self.cursor);
        let (Some(before), Some(after)) = self.chars_around(offset) else {
            return false;
        };
        let Some((_, close, true)) = ops::bracket_info(before) else {
            return false;
        };
        if after != close {
            return false;
        }
        let indent = self.row_indent(self.cursor.row);
        let unit = self.indent_unit();
        let text = format!("\n{indent}{unit}\n{indent}");
        let cursor = offset + 1 + indent.len() + unit.len();
        self.apply_edits(vec![(offset..offset, text)], cursor, cursor, cx);
        true
    }

    /// The clipboard text re-indented for where it is pasted.
    pub(crate) fn paste_text(&self, text: &str) -> String {
        let start = self.edit_range().start;
        let point = self.snapshot.offset_to_point(start);
        let line = self.snapshot.line_text(point.row);
        let column = (point.column as usize).min(line.len());
        let indent = &line[..ops::indent_len(&line)];
        ops::reindent_paste(text, &line[..column], indent)
    }

    // -- whole-line clipboard (E2) -----------------------------------------------

    /// Whether this view has the code-editor chrome: the typing helpers of
    /// E2, E4, E5 and E6 are off in the chat composer and the comment box
    /// (R13).
    fn is_full_chrome(&self) -> bool {
        self.settings.chrome == EditorChrome::Full
    }

    /// `Ctrl+C` / `Ctrl+X`. With a selection: the selected text goes to the
    /// clipboard (and `cut` deletes it) and the whole-line memory is
    /// forgotten. Without one, in `Full` chrome: the cursor's row (a ghost
    /// row's base text too) goes to the clipboard with its `\n`, marked as a
    /// whole line, and `cut` deletes the buffer row as `editor::delete_line`
    /// does (one undo step); a ghost row is read-only, so it is only copied.
    pub(crate) fn copy_or_cut(&mut self, cut: bool, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.set_global(LineClipboard(None));
            if cut {
                let range = self.edit_range();
                if range.start != range.end {
                    self.replace_buffer_range(range, "", cx);
                }
            }
            return;
        }
        if !self.is_full_chrome() {
            return;
        }
        let mut line = self.display_row_source(self.cursor.row);
        line.push('\n');
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            line.clone(),
            LineCopyMetadata { whole_line: true },
        ));
        cx.set_global(LineClipboard(Some(line)));
        if cut
            && matches!(
                self.display_map.to_buffer(self.cursor.row),
                DisplayCell::Buffer(_)
            )
        {
            self.delete_lines(cx);
        }
    }

    /// `Ctrl+V` of a copied whole line with no selection: inserts it as is
    /// at the start of the cursor's row, so the text the cursor was on moves
    /// one row down and the cursor stays on it, in the same column. Returns
    /// `false` (and does nothing) when the paste is not that case.
    pub(crate) fn paste_whole_line(
        &mut self,
        item: &ClipboardItem,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_full_chrome() || self.has_selection() {
            return false;
        }
        let Some(text) = item.text() else {
            return false;
        };
        if !text.ends_with('\n') || !is_line_copy(item, &text, cx) {
            return false;
        }
        // On a ghost row `edit_offset` is already the start of the next real
        // row, which is where the line goes.
        let cursor = self.edit_offset(self.cursor);
        let row_start = match self.display_map.to_buffer(self.cursor.row) {
            DisplayCell::Buffer(row) => self.snapshot.line_start_offset(row),
            DisplayCell::Phantom { .. } => cursor,
        };
        let after = cursor + text.len();
        self.apply_edits(vec![(row_start..row_start, text)], after, after, cx);
        true
    }

    // -- indentation while typing (E4, E5, E6) ----------------------------------------

    /// Typing `}`, `]` or `)` on a row that only has indentation before the
    /// cursor (and nothing but blanks after it) takes one level off that
    /// indentation, by the rule of `Shift+Tab`, and types the character: one
    /// undo step. Returns `true` when it did.
    ///
    /// Not with a selection, an IME composition or in `Minimal` chrome. The
    /// "step over the auto-closed closer" of `type_with_pairs` always wins:
    /// it needs the closer right after the cursor, which this never accepts.
    pub(crate) fn outdent_for_closer(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if !self.is_full_chrome() || self.has_selection() || self.marked_range.is_some() {
            return false;
        }
        let mut chars = text.chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else {
            return false;
        };
        if !matches!(ch, '}' | ']' | ')') {
            return false;
        }
        let DisplayCell::Buffer(row) = self.display_map.to_buffer(self.cursor.row) else {
            return false;
        };
        let offset = self.edit_offset(self.cursor);
        let line_start = self.snapshot.line_start_offset(row);
        let line = self.snapshot.line_text(row);
        let column = offset.saturating_sub(line_start).min(line.len());
        let (before, after) = line.split_at(column);
        let blank = |c: char| c == ' ' || c == '\t';
        if before.is_empty() || !before.chars().all(blank) || !after.chars().all(blank) {
            return false;
        }
        let unit = self.indent_unit();
        let mut replacement = ops::outdent_once(before, &unit).to_string();
        replacement.push(ch);
        let cursor = line_start + replacement.len();
        self.apply_edits(vec![(line_start..offset, replacement)], cursor, cursor, cx);
        true
    }

    /// `Enter` on a row that is only blanks (or empty): the row becomes empty
    /// and the new row below gets the indentation that was before the cursor,
    /// with the cursor at its end: one undo step. Returns `true` when it did.
    pub(crate) fn newline_on_indent_row(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.is_full_chrome() || self.has_selection() {
            return false;
        }
        let DisplayCell::Buffer(row) = self.display_map.to_buffer(self.cursor.row) else {
            return false;
        };
        let line = self.snapshot.line_text(row);
        if !line.chars().all(|c| c == ' ' || c == '\t') {
            return false;
        }
        let line_start = self.snapshot.line_start_offset(row);
        let column = self
            .edit_offset(self.cursor)
            .saturating_sub(line_start)
            .min(line.len());
        let text = format!("\n{}", &line[..column]);
        self.replace_buffer_range(line_start..line_start + line.len(), &text, cx);
        true
    }

    /// `Backspace` with only spaces before the cursor deletes back to the
    /// previous indent stop (`((column - 1) % size) + 1` spaces, `size` being
    /// the width of [`EditorView::indent_unit`]; a tab unit counts 1). A tab
    /// before the cursor, a selection or a composition leave it to the
    /// one-character delete. Returns `true` when it did.
    pub(crate) fn backspace_in_indent(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.is_full_chrome() || self.has_selection() || self.marked_range.is_some() {
            return false;
        }
        let DisplayCell::Buffer(row) = self.display_map.to_buffer(self.cursor.row) else {
            return false;
        };
        let offset = self.edit_offset(self.cursor);
        let line_start = self.snapshot.line_start_offset(row);
        let column = offset.saturating_sub(line_start);
        let line = self.snapshot.line_text(row);
        if column == 0 || column > line.len() || !line[..column].bytes().all(|b| b == b' ') {
            return false;
        }
        let size = self.indent_unit().len();
        let count = ops::spaces_to_previous_stop(column, size);
        self.replace_buffer_range(offset - count..offset, "", cx);
        true
    }

    // -- line commands ---------------------------------------------------------

    fn move_lines(&mut self, up: bool, cx: &mut Context<Self>) {
        let rows = self.selected_buffer_rows();
        let count = self.snapshot.line_count();
        let (anchor, cursor) = self.selection_offsets();
        let block_start = self.snapshot.line_start_offset(rows.start);
        let block_end = self.line_end(rows.end - 1);
        let block = self.snapshot.text_in(block_start..block_end);
        if up {
            if rows.start == 0 {
                return;
            }
            let previous = rows.start - 1;
            let previous_text = self.snapshot.line_text(previous);
            let start = self.snapshot.line_start_offset(previous);
            let shift = previous_text.len() + 1;
            self.apply_edits(
                vec![(start..block_end, format!("{block}\n{previous_text}"))],
                anchor.saturating_sub(shift),
                cursor.saturating_sub(shift),
                cx,
            );
        } else {
            if rows.end >= count {
                return;
            }
            let next = rows.end;
            // The empty row after the final newline is not a line to swap
            // with: moving onto it would drop the file's last newline.
            if next + 1 == count && self.snapshot.line_len(next) == 0 {
                return;
            }
            let next_text = self.snapshot.line_text(next);
            let end = self.line_end(next);
            let shift = next_text.len() + 1;
            self.apply_edits(
                vec![(block_start..end, format!("{next_text}\n{block}"))],
                anchor + shift,
                cursor + shift,
                cx,
            );
        }
    }

    fn duplicate_lines(&mut self, down: bool, cx: &mut Context<Self>) {
        let rows = self.selected_buffer_rows();
        let (anchor, cursor) = self.selection_offsets();
        let block_start = self.snapshot.line_start_offset(rows.start);
        let block_end = self.line_end(rows.end - 1);
        let block = self.snapshot.text_in(block_start..block_end);
        if down {
            let shift = block.len() + 1;
            self.apply_edits(
                vec![(block_end..block_end, format!("\n{block}"))],
                anchor + shift,
                cursor + shift,
                cx,
            );
        } else {
            self.apply_edits(
                vec![(block_start..block_start, format!("{block}\n"))],
                anchor,
                cursor,
                cx,
            );
        }
    }

    fn delete_lines(&mut self, cx: &mut Context<Self>) {
        let rows = self.selected_buffer_rows();
        let count = self.snapshot.line_count();
        let column = self
            .snapshot
            .offset_to_point(self.edit_offset(self.cursor))
            .column;
        let start = self.snapshot.line_start_offset(rows.start);
        let (range, cursor) = if rows.end < count {
            let range = start..self.snapshot.line_start_offset(rows.end);
            // The row below moves up into the deleted rows' place.
            let len = self.snapshot.line_len(rows.end);
            (range, start + column.min(len) as usize)
        } else if rows.start > 0 {
            let previous = rows.start - 1;
            let range = self.line_end(previous)..self.line_end(rows.end - 1);
            let len = self.snapshot.line_len(previous);
            let at = self.snapshot.line_start_offset(previous) + column.min(len) as usize;
            (range, at)
        } else {
            (0..self.snapshot.len(), 0)
        };
        if range.is_empty() {
            return;
        }
        self.apply_edits(vec![(range, String::new())], cursor, cursor, cx);
    }

    fn toggle_comments(&mut self, cx: &mut Context<Self>) {
        let Some(syntax) = self.language_name().and_then(ops::comment_syntax) else {
            return;
        };
        let rows = self.selected_buffer_rows();
        let lines: Vec<(usize, String)> = rows
            .map(|row| {
                (
                    self.snapshot.line_start_offset(row),
                    self.snapshot.line_text(row),
                )
            })
            .collect();
        let edits = ops::toggle_comment_edits(&lines, syntax);
        let (anchor, cursor) = self.map_selection(&edits);
        self.apply_edits(edits, anchor, cursor, cx);
    }

    fn join(&mut self, cx: &mut Context<Self>) {
        let rows = self.selected_buffer_rows();
        let count = self.snapshot.line_count();
        let rows = if rows.end - rows.start >= 2 {
            rows
        } else if rows.start + 1 < count {
            rows.start..rows.start + 2
        } else {
            return;
        };
        let lines: Vec<String> = rows
            .clone()
            .map(|row| self.snapshot.line_text(row))
            .collect();
        let (joined, points) = ops::join_lines(&lines);
        let start = self.snapshot.line_start_offset(rows.start);
        let end = self.line_end(rows.end - 1);
        let at = start + points.last().copied().unwrap_or(joined.len());
        self.apply_edits(vec![(start..end, joined)], at, at, cx);
    }

    fn select_line(&mut self, cx: &mut Context<Self>) {
        let range = self.edit_range();
        let start_row = self.snapshot.offset_to_point(range.start).row;
        let end_row = self.snapshot.offset_to_point(range.end).row;
        let next = end_row + 1;
        let cursor = if next < self.snapshot.line_count() {
            self.snapshot.line_start_offset(next)
        } else {
            self.snapshot.len()
        };
        let anchor = self.snapshot.line_start_offset(start_row);
        self.select_offsets(anchor, cursor, cx);
    }

    /// Buffer range of the word (letters, digits, `_`) at or right before a
    /// buffer offset.
    fn word_around(&self, offset: usize) -> Option<Range<usize>> {
        let point = self.snapshot.offset_to_point(offset);
        let line = self.snapshot.line_text(point.row);
        let column = (point.column as usize).min(line.len());
        let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
        let at = if line[column..].chars().next().is_some_and(is_word) {
            column
        } else {
            let (index, ch) = line[..column].char_indices().next_back()?;
            if !is_word(ch) {
                return None;
            }
            index
        };
        let (start, end) = super::word_at(&line, at);
        let base = self.snapshot.line_start_offset(point.row);
        Some(base + start..base + end)
    }

    fn select_next(&mut self, cx: &mut Context<Self>) {
        let range = self.edit_range();
        if range.is_empty() {
            if let Some(word) = self.word_around(range.start) {
                self.select_offsets(word.start, word.end, cx);
            }
            return;
        }
        let needle = self.snapshot.text_in(range.clone());
        let text = self.snapshot.text();
        let found = text[range.end..]
            .find(&needle)
            .map(|at| range.end + at)
            .or_else(|| text[..range.end].find(&needle));
        if let Some(start) = found
            && start != range.start
        {
            self.select_offsets(start, start + needle.len(), cx);
        }
    }

    // -- brackets ---------------------------------------------------------------

    /// The bracket at or right before the cursor and the one matching it, as
    /// buffer byte offsets of the two characters. `None` on a phantom row, when
    /// there is no bracket there, or when it is unbalanced.
    ///
    /// The scan counts brackets of the same kind and does not know about
    /// strings or comments; it gives up after [`MAX_BRACKET_SCAN`] characters.
    pub fn matching_brackets(&self) -> Option<(usize, usize)> {
        if !matches!(
            self.display_map.to_buffer(self.cursor.row),
            DisplayCell::Buffer(_)
        ) {
            return None;
        }
        let offset = self.edit_offset(self.cursor);
        let rope = self.snapshot.rope();
        let char_ix = rope.byte_to_char(offset);
        let total = rope.len_chars();
        let at = (char_ix < total)
            .then(|| rope.char(char_ix))
            .and_then(ops::bracket_info)
            .map(|info| (char_ix, info));
        let before = (char_ix > 0)
            .then(|| rope.char(char_ix - 1))
            .and_then(ops::bracket_info)
            .map(|info| (char_ix - 1, info));
        let (position, (open, close, is_open)) = at.or(before)?;
        let mut depth = 0usize;
        let found = if is_open {
            let mut found = None;
            for (step, ch) in rope
                .chars_at(position + 1)
                .take(MAX_BRACKET_SCAN)
                .enumerate()
            {
                if ch == open {
                    depth += 1;
                } else if ch == close {
                    if depth == 0 {
                        found = Some(position + 1 + step);
                        break;
                    }
                    depth -= 1;
                }
            }
            found
        } else {
            let mut chars = rope.chars_at(position);
            let mut index = position;
            let mut found = None;
            for _ in 0..MAX_BRACKET_SCAN {
                let Some(ch) = chars.prev() else { break };
                index -= 1;
                if ch == close {
                    depth += 1;
                } else if ch == open {
                    if depth == 0 {
                        found = Some(index);
                        break;
                    }
                    depth -= 1;
                }
            }
            found
        }?;
        Some((rope.char_to_byte(position), rope.char_to_byte(found)))
    }

    /// [`EditorView::matching_brackets`], memoised per text version and cursor
    /// offset for the element, which asks every frame.
    pub(crate) fn matching_brackets_cached(&mut self) -> Option<(usize, usize)> {
        let key = (self.snapshot.version(), self.cursor);
        if let Some((cached_key, value)) = self.bracket_cache
            && cached_key == key
        {
            return value;
        }
        let value = self.matching_brackets();
        self.bracket_cache = Some((key, value));
        value
    }

    // -- case and sorting -------------------------------------------------------

    fn transform_case(&mut self, upper: bool, cx: &mut Context<Self>) {
        let selected = self.edit_range();
        let had_selection = !selected.is_empty();
        let range = if had_selection {
            selected
        } else {
            match self.word_around(selected.start) {
                Some(word) => word,
                None => return,
            }
        };
        let text = self.snapshot.text_in(range.clone());
        let changed = if upper {
            text.to_uppercase()
        } else {
            text.to_lowercase()
        };
        if changed == text {
            return;
        }
        let new_end = range.start + changed.len();
        let (anchor, cursor) = if had_selection {
            if self.cursor < self.selection_anchor {
                (new_end, range.start)
            } else {
                (range.start, new_end)
            }
        } else {
            let at = self.edit_offset(self.cursor).min(new_end);
            (at, at)
        };
        self.apply_edits(vec![(range, changed)], anchor, cursor, cx);
    }

    fn sort_selected_lines(&mut self, cx: &mut Context<Self>) {
        let rows = self.selected_buffer_rows();
        if rows.end - rows.start < 2 {
            return;
        }
        let mut lines: Vec<String> = rows
            .clone()
            .map(|row| self.snapshot.line_text(row))
            .collect();
        ops::sort_lines(&mut lines);
        let sorted = lines.join("\n");
        let start = self.snapshot.line_start_offset(rows.start);
        let end = self.line_end(rows.end - 1);
        if self.snapshot.text_in(start..end) == sorted {
            return;
        }
        let new_end = start + sorted.len();
        self.apply_edits(vec![(start..end, sorted)], start, new_end, cx);
    }

    /// The selection as a range of buffer points, for the tests.
    #[cfg(test)]
    pub(crate) fn buffer_selection(&self) -> (cincel_text::Point, cincel_text::Point) {
        let (anchor, cursor) = self.selection_offsets();
        (
            self.snapshot.offset_to_point(anchor),
            self.snapshot.offset_to_point(cursor),
        )
    }

    // -- action handlers --------------------------------------------------------

    pub(super) fn on_move_line_up(
        &mut self,
        _: &MoveLineUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_lines(true, cx);
    }

    pub(super) fn on_move_line_down(
        &mut self,
        _: &MoveLineDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_lines(false, cx);
    }

    pub(super) fn on_duplicate_line_down(
        &mut self,
        _: &DuplicateLineDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.duplicate_lines(true, cx);
    }

    pub(super) fn on_duplicate_line_up(
        &mut self,
        _: &DuplicateLineUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.duplicate_lines(false, cx);
    }

    pub(super) fn on_delete_line(
        &mut self,
        _: &DeleteLine,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_lines(cx);
    }

    pub(super) fn on_toggle_comments(
        &mut self,
        _: &ToggleComments,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_comments(cx);
    }

    pub(super) fn on_join_lines(&mut self, _: &JoinLines, _: &mut Window, cx: &mut Context<Self>) {
        self.join(cx);
    }

    pub(super) fn on_select_line(
        &mut self,
        _: &SelectLine,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_line(cx);
    }

    pub(super) fn on_select_next(
        &mut self,
        _: &SelectNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_next(cx);
    }

    pub(super) fn on_move_to_matching_bracket(
        &mut self,
        _: &MoveToMatchingBracket,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, other)) = self.matching_brackets() else {
            return;
        };
        // The cursor lands right before the matching bracket, so pressing
        // again comes back (the bracket under the cursor wins over the one
        // before it).
        let point = self.display_point_for_offset(other);
        self.move_cursor(point, false, cx);
    }

    pub(super) fn on_uppercase(&mut self, _: &Uppercase, _: &mut Window, cx: &mut Context<Self>) {
        self.transform_case(true, cx);
    }

    pub(super) fn on_lowercase(&mut self, _: &Lowercase, _: &mut Window, cx: &mut Context<Self>) {
        self.transform_case(false, cx);
    }

    pub(super) fn on_sort_lines(&mut self, _: &SortLines, _: &mut Window, cx: &mut Context<Self>) {
        self.sort_selected_lines(cx);
    }
}
