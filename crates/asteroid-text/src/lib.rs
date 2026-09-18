//! asteroid-text: ver `docs/specs/modulos/text.md`.
//!
//! E0 subset: a rope-backed [`Buffer`] with byte-offset edits, a monotonic
//! version counter, line access and `Point` <-> offset conversions.
//!
//! Scope limits for E0 (see the module spec for the full contract):
//! - line endings are LF only; `\r` is stripped on load,
//! - no anchors, no transactions, no undo/redo, no events yet.

use std::ops::Range;

use ropey::{Rope, RopeSlice};

/// A position in the buffer: `row` is a 0-based line index, `column` is a
/// 0-based **byte** offset inside that line (always on a `char` boundary).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    /// 0-based line index.
    pub row: u32,
    /// 0-based byte offset inside the line.
    pub column: u32,
}

impl Point {
    /// Builds a new point.
    pub fn new(row: u32, column: u32) -> Self {
        Self { row, column }
    }
}

/// A text buffer: a rope plus a monotonic version counter.
#[derive(Clone, Debug)]
pub struct Buffer {
    rope: Rope,
    version: u64,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new("")
    }
}

impl Buffer {
    /// Creates a buffer from `text`. CRLF and lone CR are normalized to LF.
    pub fn new(text: &str) -> Self {
        let normalized = if text.contains('\r') {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text.to_owned()
        };
        Self {
            rope: Rope::from_str(&normalized),
            version: 0,
        }
    }

    /// Monotonic version, bumped on every edit.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Total length in bytes.
    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Number of lines. A buffer ending in `\n` has a final empty line.
    pub fn line_count(&self) -> u32 {
        self.rope.len_lines() as u32
    }

    /// The whole text.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// The text of a byte range.
    pub fn text_in(&self, range: Range<usize>) -> String {
        let start = self.rope.byte_to_char(range.start);
        let end = self.rope.byte_to_char(range.end);
        self.rope.slice(start..end).to_string()
    }

    /// The raw slice of `row`, trailing newline included.
    pub fn line(&self, row: u32) -> RopeSlice<'_> {
        if row >= self.line_count() {
            return RopeSlice::from("");
        }
        self.rope.line(row as usize)
    }

    /// The text of `row` without its trailing newline.
    pub fn line_text(&self, row: u32) -> String {
        let line = self.line(row);
        let mut text = line.to_string();
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }

    /// Length of `row` in bytes, excluding the trailing newline.
    pub fn line_len(&self, row: u32) -> u32 {
        if row >= self.line_count() {
            return 0;
        }
        let line = self.rope.line(row as usize);
        let mut len = line.len_bytes();
        if line.len_chars() > 0 && line.char(line.len_chars() - 1) == '\n' {
            len -= 1;
        }
        len as u32
    }

    /// Byte offset of the first byte of `row`.
    pub fn line_start_offset(&self, row: u32) -> usize {
        let row = row.min(self.line_count().saturating_sub(1));
        self.rope.line_to_byte(row as usize)
    }

    /// Converts a point into a byte offset, clipping it first.
    pub fn point_to_offset(&self, point: Point) -> usize {
        let point = self.clip_point(point);
        self.line_start_offset(point.row) + point.column as usize
    }

    /// Converts a byte offset into a point.
    pub fn offset_to_point(&self, offset: usize) -> Point {
        let offset = offset.min(self.len_bytes());
        let row = self.rope.byte_to_line(offset) as u32;
        let column = (offset - self.line_start_offset(row)) as u32;
        Point { row, column }
    }

    /// Clamps `point` to a valid position, snapping the column down to a
    /// `char` boundary.
    pub fn clip_point(&self, point: Point) -> Point {
        let row = point.row.min(self.line_count().saturating_sub(1));
        let line = self.line_text(row);
        let mut column = point.column.min(line.len() as u32) as usize;
        while column > 0 && !line.is_char_boundary(column) {
            column -= 1;
        }
        Point {
            row,
            column: column as u32,
        }
    }

    /// Replaces the byte range `range` with `new_text`.
    pub fn edit(&mut self, range: Range<usize>, new_text: &str) {
        let start = range.start.min(self.len_bytes());
        let end = range.end.clamp(start, self.len_bytes());
        let char_start = self.rope.byte_to_char(start);
        let char_end = self.rope.byte_to_char(end);
        if char_start != char_end {
            self.rope.remove(char_start..char_end);
        }
        if !new_text.is_empty() {
            let normalized;
            let text = if new_text.contains('\r') {
                normalized = new_text.replace("\r\n", "\n").replace('\r', "\n");
                normalized.as_str()
            } else {
                new_text
            };
            self.rope.insert(char_start, text);
        }
        self.version += 1;
    }

    /// Inserts `text` at `offset`.
    pub fn insert(&mut self, offset: usize, text: &str) {
        self.edit(offset..offset, text);
    }

    /// Deletes the byte range `range`.
    pub fn delete(&mut self, range: Range<usize>) {
        self.edit(range, "");
    }

    /// Replaces the whole lines `rows` with `lines`, keeping the trailing
    /// newline structure intact. Used by the review "reject" operation.
    pub fn replace_rows(&mut self, rows: Range<u32>, lines: &[String]) {
        let line_count = self.line_count();
        let start_row = rows.start.min(line_count);
        let end_row = rows.end.min(line_count);
        let start = self.line_start_offset(start_row);
        let end = if end_row >= line_count {
            self.len_bytes()
        } else {
            self.line_start_offset(end_row)
        };
        let mut text = String::new();
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
        if end == self.len_bytes() && !lines.is_empty() {
            // The buffer did not end with a newline; do not introduce one.
            let ends_with_newline = self.len_bytes() > 0
                && self.rope.char(self.rope.len_chars().saturating_sub(1)) == '\n';
            if !ends_with_newline {
                text.pop();
            }
        }
        self.edit(start..end, &text);
    }

    /// Converts a byte offset into a UTF-16 code-unit offset.
    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        let offset = offset.min(self.len_bytes());
        self.rope.char_to_utf16_cu(self.rope.byte_to_char(offset))
    }

    /// Converts a UTF-16 code-unit offset into a byte offset.
    pub fn offset_from_utf16(&self, offset_utf16: usize) -> usize {
        let clamped = offset_utf16.min(self.rope.len_utf16_cu());
        self.rope.char_to_byte(self.rope.utf16_cu_to_char(clamped))
    }

    /// Converts a byte range into a UTF-16 range.
    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    /// Converts a UTF-16 range into a byte range.
    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    /// Previous `char` boundary before `offset`.
    pub fn previous_char_boundary(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }
        let char_ix = self.rope.byte_to_char(offset);
        let char_ix = if self.rope.char_to_byte(char_ix) < offset {
            char_ix
        } else {
            char_ix.saturating_sub(1)
        };
        self.rope.char_to_byte(char_ix)
    }

    /// Next `char` boundary after `offset`.
    pub fn next_char_boundary(&self, offset: usize) -> usize {
        if offset >= self.len_bytes() {
            return self.len_bytes();
        }
        let char_ix = self.rope.byte_to_char(offset);
        self.rope
            .char_to_byte((char_ix + 1).min(self.rope.len_chars()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_normalizes_crlf() {
        let buffer = Buffer::new("a\r\nb\rc");
        assert_eq!(buffer.text(), "a\nb\nc");
        assert_eq!(buffer.line_count(), 3);
    }

    #[test]
    fn lines_and_offsets() {
        let buffer = Buffer::new("uno\ndos\ntres\n");
        assert_eq!(buffer.line_count(), 4);
        assert_eq!(buffer.line_text(0), "uno");
        assert_eq!(buffer.line_text(3), "");
        assert_eq!(buffer.line_len(1), 3);
        assert_eq!(buffer.line_start_offset(2), 8);
        assert_eq!(buffer.point_to_offset(Point::new(1, 2)), 6);
        assert_eq!(buffer.offset_to_point(6), Point::new(1, 2));
        assert_eq!(buffer.text_in(4..7), "dos");
    }

    #[test]
    fn clip_point_snaps_to_char_boundary() {
        let buffer = Buffer::new("aá\nb");
        // "á" occupies bytes 1..3.
        assert_eq!(buffer.clip_point(Point::new(0, 2)), Point::new(0, 1));
        assert_eq!(buffer.clip_point(Point::new(0, 9)), Point::new(0, 3));
        assert_eq!(buffer.clip_point(Point::new(99, 0)), Point::new(1, 0));
    }

    #[test]
    fn edits_bump_version() {
        let mut buffer = Buffer::new("hola");
        assert_eq!(buffer.version(), 0);
        buffer.insert(4, " mundo");
        assert_eq!(buffer.text(), "hola mundo");
        assert_eq!(buffer.version(), 1);
        buffer.delete(0..5);
        assert_eq!(buffer.text(), "mundo");
        assert_eq!(buffer.version(), 2);
        buffer.edit(0..5, "adiós");
        assert_eq!(buffer.text(), "adiós");
        assert_eq!(buffer.version(), 3);
    }

    #[test]
    fn edit_across_lines() {
        let mut buffer = Buffer::new("uno\ndos\ntres\n");
        let start = buffer.point_to_offset(Point::new(0, 3));
        let end = buffer.point_to_offset(Point::new(1, 0));
        buffer.delete(start..end);
        assert_eq!(buffer.text(), "unodos\ntres\n");
        assert_eq!(buffer.line_count(), 3);
    }

    #[test]
    fn replace_rows_swaps_whole_lines() {
        let mut buffer = Buffer::new("a\nb\nc\nd\n");
        buffer.replace_rows(1..3, &["X".to_string(), "Y".to_string()]);
        assert_eq!(buffer.text(), "a\nX\nY\nd\n");

        let mut buffer = Buffer::new("a\nb\nc\n");
        buffer.replace_rows(1..2, &[]);
        assert_eq!(buffer.text(), "a\nc\n");

        let mut buffer = Buffer::new("a\nb");
        buffer.replace_rows(1..2, &["z".to_string()]);
        assert_eq!(buffer.text(), "a\nz");
    }

    #[test]
    fn utf16_conversions() {
        let buffer = Buffer::new("aá€\n😀");
        // a=1 byte, á=2, €=3, \n=1, 😀=4
        assert_eq!(buffer.offset_to_utf16(0), 0);
        assert_eq!(buffer.offset_to_utf16(3), 2);
        assert_eq!(buffer.offset_to_utf16(6), 3);
        assert_eq!(buffer.offset_from_utf16(3), 6);
        // The emoji is a surrogate pair in UTF-16.
        assert_eq!(buffer.offset_to_utf16(11), 6);
        assert_eq!(buffer.offset_from_utf16(6), 11);
        assert_eq!(buffer.range_to_utf16(&(0..3)), 0..2);
        assert_eq!(buffer.range_from_utf16(&(0..2)), 0..3);
    }

    #[test]
    fn char_boundaries() {
        let buffer = Buffer::new("aáb");
        assert_eq!(buffer.next_char_boundary(0), 1);
        assert_eq!(buffer.next_char_boundary(1), 3);
        assert_eq!(buffer.previous_char_boundary(3), 1);
        assert_eq!(buffer.previous_char_boundary(1), 0);
        assert_eq!(buffer.previous_char_boundary(0), 0);
        assert_eq!(buffer.next_char_boundary(4), 4);
    }
}
