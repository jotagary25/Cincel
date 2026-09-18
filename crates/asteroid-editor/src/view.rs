//! `EditorView`: the GPUI entity that owns the buffer, the display map, the
//! selection and the scroll state, and implements `EntityInputHandler` (IME).

use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use asteroid_text::{Buffer, Point};
use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, CursorStyle, Entity, EntityInputHandler,
    FocusHandle, Focusable, Font, FontFallbacks, FontFeatures, FontStyle, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Pixels, Render, ShapedLine,
    SharedString, Styled, Task, UTF16Selection, Window, actions, div, px,
};

use crate::display_map::{DiffTransformMap, DisplayCell, DisplayMap, DisplayPoint, PhantomHunk};
use crate::element::EditorElement;
use crate::theme;

actions!(
    asteroid_editor,
    [
        /// Moves the cursor one character to the left.
        MoveLeft,
        /// Moves the cursor one character to the right.
        MoveRight,
        /// Moves the cursor one row up.
        MoveUp,
        /// Moves the cursor one row down.
        MoveDown,
        /// Extends the selection one character to the left.
        SelectLeft,
        /// Extends the selection one character to the right.
        SelectRight,
        /// Extends the selection one row up.
        SelectUp,
        /// Extends the selection one row down.
        SelectDown,
        /// Moves the cursor to the start of the row.
        MoveToLineStart,
        /// Moves the cursor to the end of the row.
        MoveToLineEnd,
        /// Extends the selection to the start of the row.
        SelectToLineStart,
        /// Extends the selection to the end of the row.
        SelectToLineEnd,
        /// Moves the cursor one page up.
        MovePageUp,
        /// Moves the cursor one page down.
        MovePageDown,
        /// Deletes the character before the cursor.
        Backspace,
        /// Deletes the character after the cursor.
        Delete,
        /// Inserts a line break.
        Newline,
        /// Selects the whole document.
        SelectAll,
        /// Copies the selection, phantom rows included.
        Copy,
        /// Cuts the selection.
        Cut,
        /// Pastes the clipboard.
        Paste,
    ]
);

/// Registers the default key bindings of the editor in the `AsteroidEditor`
/// key context.
pub fn bind_default_keys(cx: &mut App) {
    let context = Some("AsteroidEditor");
    cx.bind_keys([
        KeyBinding::new("left", MoveLeft, context),
        KeyBinding::new("right", MoveRight, context),
        KeyBinding::new("up", MoveUp, context),
        KeyBinding::new("down", MoveDown, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("home", MoveToLineStart, context),
        KeyBinding::new("end", MoveToLineEnd, context),
        KeyBinding::new("shift-home", SelectToLineStart, context),
        KeyBinding::new("shift-end", SelectToLineEnd, context),
        KeyBinding::new("pageup", MovePageUp, context),
        KeyBinding::new("pagedown", MovePageDown, context),
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("enter", Newline, context),
        KeyBinding::new("ctrl-a", SelectAll, context),
        KeyBinding::new("ctrl-c", Copy, context),
        KeyBinding::new("ctrl-x", Cut, context),
        KeyBinding::new("ctrl-v", Paste, context),
    ]);
}

/// Font and metrics of the editor.
#[derive(Clone, Debug)]
pub struct EditorStyle {
    /// Monospace font, with fallbacks.
    pub font: Font,
    /// Font size in pixels.
    pub font_size: Pixels,
    /// Row height in pixels (1.5x the font size).
    pub line_height: Pixels,
}

impl EditorStyle {
    /// Picks the first available monospace family of the preferred list.
    pub fn new(cx: &App) -> Self {
        const PREFERRED: [&str; 5] = [
            "JetBrains Mono",
            "JetBrainsMono Nerd Font Mono",
            "JetBrainsMono Nerd Font",
            "DejaVu Sans Mono",
            "monospace",
        ];
        let available = cx.text_system().all_font_names();
        let family: SharedString = PREFERRED
            .iter()
            .find(|name| available.iter().any(|found| found == *name))
            .map(|name| SharedString::from(*name))
            .unwrap_or_else(|| SharedString::from("monospace"));
        let fallbacks = FontFallbacks::from_fonts(
            PREFERRED
                .iter()
                .filter(|name| **name != family.as_ref())
                .map(|name| name.to_string())
                .collect(),
        );
        let font_size = px(14.);
        Self {
            font: Font {
                family,
                features: FontFeatures::default(),
                fallbacks: Some(fallbacks),
                weight: FontWeight::NORMAL,
                style: FontStyle::Normal,
            },
            font_size,
            line_height: font_size * 1.5,
        }
    }
}

/// Key of the shaped-line cache: the buffer version plus the row identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LineCacheKey {
    Buffer(u32),
    Phantom(usize, usize),
    Number(u32),
}

/// What the element measured on the last frame, so mouse events can be mapped
/// back to display positions.
#[derive(Clone)]
pub(crate) struct LayoutSnapshot {
    pub bounds: Bounds<Pixels>,
    pub text_origin_x: Pixels,
    pub line_height: Pixels,
    pub first_row: u32,
    pub lines: Vec<ShapedLine>,
    pub visible_row_count: f32,
}

/// The editor entity.
pub struct EditorView {
    pub(crate) buffer: Buffer,
    pub(crate) hunks: Vec<PhantomHunk>,
    pub(crate) display_map: DisplayMap,
    pub(crate) cursor: DisplayPoint,
    pub(crate) selection_anchor: DisplayPoint,
    pub(crate) scroll_top: f32,
    pub(crate) style: EditorStyle,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) marked_range: Option<Range<usize>>,
    pub(crate) layout: Option<LayoutSnapshot>,
    pub(crate) line_cache: HashMap<LineCacheKey, ShapedLine>,
    pub(crate) cache_version: u64,
    pub(crate) blink_visible: bool,
    pub(crate) autoscroll: bool,
    pub(crate) selecting: bool,
    _blink_task: Option<Task<()>>,
}

impl EditorView {
    /// Builds an editor over `text` with a set of simulated review hunks.
    pub fn new(text: &str, hunks: Vec<PhantomHunk>, cx: &mut Context<Self>) -> Self {
        let buffer = Buffer::new(text);
        let display_map = DisplayMap::new(buffer.line_count(), hunks.clone());
        let hunks = display_map.diff().hunks().to_vec();
        let mut this = Self {
            buffer,
            hunks,
            display_map,
            cursor: DisplayPoint::default(),
            selection_anchor: DisplayPoint::default(),
            scroll_top: 0.,
            style: EditorStyle::new(cx),
            focus_handle: cx.focus_handle(),
            marked_range: None,
            layout: None,
            line_cache: HashMap::new(),
            cache_version: 0,
            blink_visible: true,
            autoscroll: false,
            selecting: false,
            _blink_task: None,
        };
        this.start_blinking(cx);
        this
    }

    /// The hunks currently displayed.
    pub fn hunks(&self) -> &[PhantomHunk] {
        &self.hunks
    }

    /// The whole text of the buffer.
    pub fn text(&self) -> String {
        self.buffer.text()
    }

    /// Number of display rows.
    pub fn display_row_count(&self) -> u32 {
        self.display_map.display_row_count()
    }

    fn start_blinking(&mut self, cx: &mut Context<Self>) {
        self._blink_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let updated = this.update(cx, |this, cx| {
                    this.blink_visible = !this.blink_visible;
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        }));
    }

    fn reset_blink(&mut self) {
        self.blink_visible = true;
    }

    // -- display helpers ---------------------------------------------------

    /// The text of a display row (buffer row or phantom row).
    pub fn display_row_text(&self, row: u32) -> String {
        match self.display_map.to_buffer(row) {
            DisplayCell::Buffer(buffer_row) => self.buffer.line_text(buffer_row),
            DisplayCell::Phantom { hunk_ix, line_ix } => self.hunks[hunk_ix].deleted_text[line_ix]
                .clone()
                .replace('\t', "    "),
        }
    }

    /// Clamps a display point to a valid position on a `char` boundary.
    pub fn clip_point(&self, point: DisplayPoint) -> DisplayPoint {
        let row = self.display_map.clip_row(point.row);
        let text = self.display_row_text(row);
        let mut column = (point.column as usize).min(text.len());
        while column > 0 && !text.is_char_boundary(column) {
            column -= 1;
        }
        DisplayPoint::new(row, column as u32)
    }

    /// The ordered selection in display space.
    pub fn selection_range(&self) -> Range<DisplayPoint> {
        if self.selection_anchor <= self.cursor {
            self.selection_anchor..self.cursor
        } else {
            self.cursor..self.selection_anchor
        }
    }

    /// The selected text, phantom rows included.
    pub fn selected_text(&self) -> String {
        let range = self.selection_range();
        if range.start == range.end {
            return String::new();
        }
        let mut text = String::new();
        for row in range.start.row..=range.end.row {
            let line = self.display_row_text(row);
            let start = if row == range.start.row {
                (range.start.column as usize).min(line.len())
            } else {
                0
            };
            let end = if row == range.end.row {
                (range.end.column as usize).min(line.len())
            } else {
                line.len()
            };
            text.push_str(&line[start..end]);
            if row != range.end.row {
                text.push('\n');
            }
        }
        text
    }

    /// Buffer offset where an edit at `point` must be applied. Phantom rows are
    /// read-only: edits are redirected to the start of the next real row.
    fn edit_offset(&self, point: DisplayPoint) -> usize {
        match self.display_map.to_buffer(point.row) {
            DisplayCell::Buffer(buffer_row) => self
                .buffer
                .point_to_offset(Point::new(buffer_row, point.column)),
            DisplayCell::Phantom { hunk_ix, .. } => {
                let row = self.hunks[hunk_ix].insert_before_buffer_row;
                self.buffer.point_to_offset(Point::new(row, 0))
            }
        }
    }

    /// The buffer range an edit must replace (the selection, redirected).
    fn edit_range(&self) -> Range<usize> {
        let range = self.selection_range();
        let start = self.edit_offset(range.start);
        let end = self.edit_offset(range.end);
        start.min(end)..start.max(end)
    }

    /// Display position of a buffer offset.
    fn display_point_for_offset(&self, offset: usize) -> DisplayPoint {
        let point = self.buffer.offset_to_point(offset);
        DisplayPoint::new(self.display_map.to_display_row(point.row), point.column)
    }

    fn rebuild_display_map(&mut self) {
        self.display_map = DisplayMap::new(self.buffer.line_count(), self.hunks.clone());
        self.hunks = self.display_map.diff().hunks().to_vec();
    }

    /// Keeps hunk rows aligned after an edit that changed the line count.
    fn adjust_hunks(&mut self, edit_row: u32, delta: i64) {
        if delta == 0 {
            return;
        }
        let shift = |row: u32| -> u32 { (row as i64 + delta).max(0) as u32 };
        for hunk in &mut self.hunks {
            if hunk.insert_before_buffer_row > edit_row {
                hunk.insert_before_buffer_row = shift(hunk.insert_before_buffer_row);
                hunk.added_rows = shift(hunk.added_rows.start)..shift(hunk.added_rows.end);
            }
        }
    }

    // -- editing -----------------------------------------------------------

    /// Replaces the selection with `text` (the path taken by typing and IME).
    pub fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let range = self.edit_range();
        self.replace_buffer_range(range, text, cx);
    }

    fn replace_buffer_range(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let rows_before = self.buffer.line_count();
        let edit_row = self.buffer.offset_to_point(range.start).row;
        self.buffer.edit(range.clone(), text);
        let delta = self.buffer.line_count() as i64 - rows_before as i64;
        self.adjust_hunks(edit_row, delta);
        self.rebuild_display_map();
        let offset = range.start + text.len();
        self.cursor = self.display_point_for_offset(offset);
        self.selection_anchor = self.cursor;
        self.marked_range = None;
        self.reset_blink();
        self.autoscroll = true;
        cx.notify();
    }

    // -- actions -----------------------------------------------------------

    fn move_cursor(&mut self, point: DisplayPoint, extend: bool, cx: &mut Context<Self>) {
        self.cursor = self.clip_point(point);
        if !extend {
            self.selection_anchor = self.cursor;
        }
        self.reset_blink();
        self.autoscroll = true;
        cx.notify();
    }

    fn point_left(&self, point: DisplayPoint) -> DisplayPoint {
        if point.column > 0 {
            let text = self.display_row_text(point.row);
            let mut column = (point.column as usize - 1).min(text.len());
            while column > 0 && !text.is_char_boundary(column) {
                column -= 1;
            }
            DisplayPoint::new(point.row, column as u32)
        } else if point.row > 0 {
            let row = point.row - 1;
            DisplayPoint::new(row, self.display_row_text(row).len() as u32)
        } else {
            point
        }
    }

    fn point_right(&self, point: DisplayPoint) -> DisplayPoint {
        let text = self.display_row_text(point.row);
        if (point.column as usize) < text.len() {
            let mut column = point.column as usize + 1;
            while column < text.len() && !text.is_char_boundary(column) {
                column += 1;
            }
            DisplayPoint::new(point.row, column as u32)
        } else if point.row + 1 < self.display_map.display_row_count() {
            DisplayPoint::new(point.row + 1, 0)
        } else {
            point
        }
    }

    fn page_rows(&self) -> u32 {
        self.layout
            .as_ref()
            .map(|layout| layout.visible_row_count.max(1.) as u32)
            .unwrap_or(20)
    }

    fn on_move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        let point = if self.selection_range().start != self.selection_range().end {
            self.selection_range().start
        } else {
            self.point_left(self.cursor)
        };
        self.move_cursor(point, false, cx);
    }

    fn on_move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        let point = if self.selection_range().start != self.selection_range().end {
            self.selection_range().end
        } else {
            self.point_right(self.cursor)
        };
        self.move_cursor(point, false, cx);
    }

    fn on_move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        let point = DisplayPoint::new(self.cursor.row.saturating_sub(1), self.cursor.column);
        self.move_cursor(point, false, cx);
    }

    fn on_move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        let point = DisplayPoint::new(self.cursor.row + 1, self.cursor.column);
        self.move_cursor(point, false, cx);
    }

    fn on_select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let point = self.point_left(self.cursor);
        self.move_cursor(point, true, cx);
    }

    fn on_select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let point = self.point_right(self.cursor);
        self.move_cursor(point, true, cx);
    }

    fn on_select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let point = DisplayPoint::new(self.cursor.row.saturating_sub(1), self.cursor.column);
        self.move_cursor(point, true, cx);
    }

    fn on_select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let point = DisplayPoint::new(self.cursor.row + 1, self.cursor.column);
        self.move_cursor(point, true, cx);
    }

    fn on_move_to_line_start(
        &mut self,
        _: &MoveToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_cursor(DisplayPoint::new(self.cursor.row, 0), false, cx);
    }

    fn on_move_to_line_end(&mut self, _: &MoveToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(DisplayPoint::new(self.cursor.row, u32::MAX), false, cx);
    }

    fn on_select_to_line_start(
        &mut self,
        _: &SelectToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_cursor(DisplayPoint::new(self.cursor.row, 0), true, cx);
    }

    fn on_select_to_line_end(
        &mut self,
        _: &SelectToLineEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_cursor(DisplayPoint::new(self.cursor.row, u32::MAX), true, cx);
    }

    fn on_move_page_up(&mut self, _: &MovePageUp, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.page_rows();
        let point = DisplayPoint::new(self.cursor.row.saturating_sub(rows), self.cursor.column);
        self.move_cursor(point, false, cx);
    }

    fn on_move_page_down(&mut self, _: &MovePageDown, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.page_rows();
        let point = DisplayPoint::new(self.cursor.row + rows, self.cursor.column);
        self.move_cursor(point, false, cx);
    }

    fn on_backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.edit_range();
        let range = if range.start == range.end {
            self.buffer.previous_char_boundary(range.start)..range.end
        } else {
            range
        };
        if range.start == range.end {
            return;
        }
        self.replace_buffer_range(range, "", cx);
    }

    fn on_delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.edit_range();
        let range = if range.start == range.end {
            range.start..self.buffer.next_char_boundary(range.end)
        } else {
            range
        };
        if range.start == range.end {
            return;
        }
        self.replace_buffer_range(range, "", cx);
    }

    fn on_newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.insert_text("\n", cx);
    }

    fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let last_row = self.display_map.display_row_count().saturating_sub(1);
        self.selection_anchor = DisplayPoint::new(0, 0);
        self.cursor = self.clip_point(DisplayPoint::new(last_row, u32::MAX));
        cx.notify();
    }

    fn on_copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn on_cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let range = self.edit_range();
        if range.start != range.end {
            self.replace_buffer_range(range, "", cx);
        }
    }

    fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert_text(&text, cx);
        }
    }

    // -- review ------------------------------------------------------------

    /// Accepts a hunk: the phantom rows and the added background go away.
    pub fn accept_hunk(&mut self, hunk_ix: usize, cx: &mut Context<Self>) {
        if hunk_ix >= self.hunks.len() {
            return;
        }
        self.hunks.remove(hunk_ix);
        self.rebuild_display_map();
        self.cursor = self.clip_point(self.cursor);
        self.selection_anchor = self.clip_point(self.selection_anchor);
        self.line_cache.clear();
        cx.notify();
    }

    /// Rejects a hunk: the added rows are replaced by the deleted text.
    pub fn reject_hunk(&mut self, hunk_ix: usize, cx: &mut Context<Self>) {
        if hunk_ix >= self.hunks.len() {
            return;
        }
        let hunk = self.hunks.remove(hunk_ix);
        let added = hunk.added_rows.clone();
        self.buffer.replace_rows(added.clone(), &hunk.deleted_text);
        let delta = hunk.deleted_text.len() as i64 - (added.end - added.start) as i64;
        let shift = |row: u32| -> u32 { (row as i64 + delta).max(0) as u32 };
        for other in &mut self.hunks {
            if other.insert_before_buffer_row >= added.end {
                other.insert_before_buffer_row = shift(other.insert_before_buffer_row);
                other.added_rows = shift(other.added_rows.start)..shift(other.added_rows.end);
            }
        }
        self.rebuild_display_map();
        self.cursor = self.clip_point(self.cursor);
        self.selection_anchor = self.clip_point(self.selection_anchor);
        self.line_cache.clear();
        cx.notify();
    }

    // -- mouse -------------------------------------------------------------

    /// Maps a window position to a display point using the last layout.
    pub(crate) fn point_for_position(
        &self,
        position: gpui::Point<Pixels>,
        window: &Window,
    ) -> DisplayPoint {
        let Some(layout) = self.layout.as_ref() else {
            return DisplayPoint::default();
        };
        let relative_y = position.y - layout.bounds.top() + px(self.scroll_top);
        let row = (f32::from(relative_y) / f32::from(layout.line_height)).floor();
        let row = row.max(0.) as u32;
        let row = self.display_map.clip_row(row);
        let x = position.x - layout.text_origin_x;
        let column = if let Some(line) = layout
            .lines
            .get(row.saturating_sub(layout.first_row) as usize)
            .filter(|_| row >= layout.first_row)
        {
            line.closest_index_for_x(x.max(px(0.)))
        } else {
            let text: SharedString = self.display_row_text(row).into();
            let run = gpui::TextRun {
                len: text.len(),
                font: self.style.font.clone(),
                color: theme::color(theme::TEXT),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window
                .text_system()
                .shape_line(text, self.style.font_size, &[run], None);
            line.closest_index_for_x(x.max(px(0.)))
        };
        self.clip_point(DisplayPoint::new(row, column as u32))
    }

    pub(crate) fn begin_selection(
        &mut self,
        point: DisplayPoint,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        self.selecting = true;
        self.move_cursor(point, extend, cx);
    }

    pub(crate) fn update_selection(&mut self, point: DisplayPoint, cx: &mut Context<Self>) {
        if self.selecting {
            self.move_cursor(point, true, cx);
        }
    }

    pub(crate) fn end_selection(&mut self) {
        self.selecting = false;
    }

    /// Scrolls by `delta_rows` rows (negative scrolls up). Used by the demo to
    /// exercise the virtualization without a mouse.
    pub fn scroll_rows(&mut self, delta_rows: f32, cx: &mut Context<Self>) {
        let line_height = f32::from(self.style.line_height);
        let viewport = self
            .layout
            .as_ref()
            .map(|layout| f32::from(layout.bounds.size.height))
            .unwrap_or(0.);
        let max_scroll =
            (self.display_map.display_row_count() as f32 * line_height - viewport).max(0.);
        self.scroll_top = (self.scroll_top + delta_rows * line_height).clamp(0., max_scroll);
        cx.notify();
    }

    pub(crate) fn scroll_by(&mut self, delta_y: Pixels, max_scroll: f32, cx: &mut Context<Self>) {
        self.scroll_top = (self.scroll_top - f32::from(delta_y)).clamp(0., max_scroll.max(0.));
        cx.notify();
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("AsteroidEditor")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .size_full()
            .bg(theme::color(theme::BG))
            .on_action(cx.listener(Self::on_move_left))
            .on_action(cx.listener(Self::on_move_right))
            .on_action(cx.listener(Self::on_move_up))
            .on_action(cx.listener(Self::on_move_down))
            .on_action(cx.listener(Self::on_select_left))
            .on_action(cx.listener(Self::on_select_right))
            .on_action(cx.listener(Self::on_select_up))
            .on_action(cx.listener(Self::on_select_down))
            .on_action(cx.listener(Self::on_move_to_line_start))
            .on_action(cx.listener(Self::on_move_to_line_end))
            .on_action(cx.listener(Self::on_select_to_line_start))
            .on_action(cx.listener(Self::on_select_to_line_end))
            .on_action(cx.listener(Self::on_move_page_up))
            .on_action(cx.listener(Self::on_move_page_down))
            .on_action(cx.listener(Self::on_backspace))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_newline))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_paste))
            .child(EditorElement::new(cx.entity()))
    }
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.buffer.range_from_utf16(&range_utf16);
        actual_range.replace(self.buffer.range_to_utf16(&range));
        Some(self.buffer.text_in(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = self.edit_range();
        Some(UTF16Selection {
            range: self.buffer.range_to_utf16(&range),
            reversed: self.cursor < self.selection_anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.buffer.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.buffer.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.edit_range());
        self.replace_buffer_range(range, new_text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.buffer.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.edit_range());
        let start = range.start;
        self.replace_buffer_range(range, new_text, cx);
        self.marked_range = if new_text.is_empty() {
            None
        } else {
            Some(start..start + new_text.len())
        };
        if let Some(selected) = new_selected_range_utf16 {
            // The IME reports the selection relative to the marked text.
            let marked = self.buffer.text_in(start..start + new_text.len());
            let offset = |units: usize| -> usize {
                let mut utf16 = 0;
                let mut bytes = 0;
                for ch in marked.chars() {
                    if utf16 >= units {
                        break;
                    }
                    utf16 += ch.len_utf16();
                    bytes += ch.len_utf8();
                }
                bytes
            };
            let cursor_offset = start + offset(selected.end);
            self.cursor = self.display_point_for_offset(cursor_offset);
            self.selection_anchor = self.display_point_for_offset(start + offset(selected.start));
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.buffer.range_from_utf16(&range_utf16);
        let point = self.display_point_for_offset(range.start);
        let line = layout
            .lines
            .get(point.row.checked_sub(layout.first_row)? as usize)?;
        let x = layout.text_origin_x + line.x_for_index(point.column as usize);
        let y = element_bounds.top() + layout.line_height * (point.row - layout.first_row) as f32
            - px(self.scroll_top % f32::from(layout.line_height));
        Some(Bounds::new(
            gpui::point(x, y),
            gpui::size(px(2.), layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let display_point = self.point_for_position(point, window);
        let offset = self.edit_offset(display_point);
        Some(self.buffer.offset_to_utf16(offset))
    }
}

/// Convenience constructor used by the example and by tests.
pub fn editor(text: &str, hunks: Vec<PhantomHunk>, cx: &mut App) -> Entity<EditorView> {
    cx.new(|cx| EditorView::new(text, hunks, cx))
}

/// Access to the diff layer, for tests and for the element.
pub fn diff_of(view: &EditorView) -> &DiffTransformMap {
    view.display_map.diff()
}
