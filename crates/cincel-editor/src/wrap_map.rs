//! `WrapMap`: the soft-wrap layer of the display pipeline
//! (`docs/specs/03-arquitectura.md` §5).
//!
//! ```text
//! Buffer -> DiffTransformMap -> WrapMap -> EditorElement
//!           display rows         wrap rows
//! ```
//!
//! A **display row** is a logical line (a buffer row or a phantom row). A
//! **wrap row** is one visual row on screen: with soft wrap off the two spaces
//! are the same, with soft wrap on a display row occupies one wrap row per
//! segment.
//!
//! The map measures in *columns*, not pixels: the editor font is monospace, so
//! a column is one character advance and a tab is [`EditorSettings::tab_size`]
//! columns. That keeps the layer free of GPUI and unit-testable. A source that
//! paints a proportional font (the chat composer's prose rows) reports the
//! advance of every character itself through [`WrapSource::char_widths`], in
//! whatever unit the wrap width was given in (the view uses
//! [`WRAP_UNITS_PER_PX`]), and the same algorithm wraps by those widths.
//!
//! [`EditorSettings::tab_size`]: crate::EditorSettings::tab_size
//!
//! Mapping in both directions is O(log n) through a Fenwick tree over the
//! per-row segment counts, and only rows that actually wrap keep a `Vec` of
//! segment offsets, so a file of long-but-not-wrapping lines costs one `u32`
//! per row.

use std::collections::HashMap;
use std::ops::Range;

use crate::display_map::DisplayRow;

/// A visual row index: display rows with their soft-wrap segments expanded.
pub type WrapRow = u32;

/// Units per pixel of a measured wrap (see [`WrapSource::char_widths`]): an
/// eighth of a pixel keeps the rounding of glyph advances invisible.
pub const WRAP_UNITS_PER_PX: f32 = 8.;

/// Where the wrap map gets the text it measures.
///
/// Implemented by the view over the display map; the unit tests implement it
/// over a `Vec<String>`.
pub trait WrapSource {
    /// Number of display rows.
    fn row_count(&self) -> u32;
    /// Byte length of a row. Used as a cheap upper bound: a row shorter than
    /// the wrap width in bytes can never be wider than it in columns, so
    /// [`WrapSource::row_text`] is never called for it.
    fn row_len(&self, row: DisplayRow) -> u32;
    /// The full text of a row, without its newline.
    fn row_text(&self, row: DisplayRow) -> String;
    /// The advance of every character of the row, in the unit of the wrap
    /// width, when the row is not painted in a monospace font. `None` (the
    /// default) measures in columns. It
    /// only takes effect when [`WrapSource::is_measured`] is `true`, which
    /// opts every row out of the [`WrapSource::row_len`] shortcut.
    fn char_widths(&self, _row: DisplayRow, _text: &str) -> Option<Vec<u32>> {
        None
    }
    /// Whether [`WrapSource::char_widths`] measures the rows (a proportional
    /// font). `false` by default.
    fn is_measured(&self) -> bool {
        false
    }
}

/// The segments of a row that does not fit in one wrap row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrappedRow {
    /// Byte offset of the start of each segment. Always starts with `0` and has
    /// at least two entries.
    pub starts: Vec<u32>,
    /// Columns the continuation segments are indented by, so wrapped code stays
    /// aligned with the line it belongs to.
    pub indent: u32,
}

impl WrappedRow {
    /// Byte range of a segment inside the row text.
    pub fn segment(&self, ix: usize, row_len: u32) -> Range<u32> {
        let start = self.starts.get(ix).copied().unwrap_or(row_len);
        let end = self.starts.get(ix + 1).copied().unwrap_or(row_len);
        start..end.max(start)
    }
}

/// The soft-wrap layer.
#[derive(Clone, Debug, Default)]
pub struct WrapMap {
    columns: Option<u32>,
    tab_size: u32,
    rows: u32,
    /// Segments per display row; empty while soft wrap is off.
    counts: Vec<u32>,
    /// Fenwick tree over `counts`, 1-indexed.
    tree: Vec<u32>,
    /// Only the rows that wrap.
    wrapped: HashMap<DisplayRow, WrappedRow>,
    total: u32,
}

impl WrapMap {
    /// An empty map with soft wrap off.
    pub fn new() -> Self {
        Self {
            columns: None,
            tab_size: 4,
            rows: 0,
            counts: Vec::new(),
            tree: Vec::new(),
            wrapped: HashMap::new(),
            total: 0,
        }
    }

    /// The wrap width in columns, or `None` when soft wrap is off.
    pub fn columns(&self) -> Option<u32> {
        self.columns
    }

    /// Whether soft wrap is on.
    pub fn is_enabled(&self) -> bool {
        self.columns.is_some()
    }

    /// Number of display rows the map was built for.
    pub fn row_count(&self) -> u32 {
        self.rows
    }

    /// Number of visual rows.
    pub fn wrap_row_count(&self) -> WrapRow {
        if self.is_enabled() {
            self.total
        } else {
            self.rows
        }
    }

    /// Recomputes the whole map. `columns` of `None` turns soft wrap off, which
    /// costs nothing.
    pub fn rebuild(&mut self, columns: Option<u32>, tab_size: u32, source: &dyn WrapSource) {
        self.columns = columns.filter(|columns| *columns > 0);
        self.tab_size = tab_size.max(1);
        self.rows = source.row_count();
        self.wrapped.clear();
        let Some(columns) = self.columns else {
            self.counts = Vec::new();
            self.tree = Vec::new();
            self.total = self.rows;
            return;
        };
        self.counts = Vec::with_capacity(self.rows as usize);
        for row in 0..self.rows {
            let count = self.measure(row, columns, source);
            self.counts.push(count);
        }
        self.build_tree();
    }

    /// Recomputes a single row, for the common edit that neither adds nor
    /// removes rows. O(log n).
    pub fn update_row(&mut self, row: DisplayRow, source: &dyn WrapSource) {
        let Some(columns) = self.columns else {
            return;
        };
        if row >= self.rows {
            return;
        }
        let count = self.measure(row, columns, source);
        let old = self.counts[row as usize];
        if old == count {
            return;
        }
        self.counts[row as usize] = count;
        self.total = self.total + count - old;
        // Fenwick point update with the signed delta.
        let mut ix = row as usize + 1;
        while ix < self.tree.len() {
            self.tree[ix] = self.tree[ix].wrapping_add(count.wrapping_sub(old));
            ix += ix & ix.wrapping_neg();
        }
    }

    fn measure(&mut self, row: DisplayRow, columns: u32, source: &dyn WrapSource) -> u32 {
        // The byte-length shortcut only holds for column widths (every
        // character is at least one column wide).
        if !source.is_measured() && source.row_len(row) <= columns {
            self.wrapped.remove(&row);
            return 1;
        }
        let text = source.row_text(row);
        let wrapped = match source.char_widths(row, &text) {
            Some(widths) => wrap_row_by_widths(&text, columns, &widths),
            None => wrap_row(&text, columns, self.tab_size),
        };
        let count = wrapped.starts.len() as u32;
        if count > 1 {
            self.wrapped.insert(row, wrapped);
        } else {
            self.wrapped.remove(&row);
        }
        count
    }

    fn build_tree(&mut self) {
        let n = self.counts.len();
        self.tree = vec![0; n + 1];
        self.total = 0;
        for (ix, count) in self.counts.iter().enumerate() {
            self.tree[ix + 1] = *count;
            self.total += count;
        }
        for ix in 1..=n {
            let parent = ix + (ix & ix.wrapping_neg());
            if parent <= n {
                self.tree[parent] += self.tree[ix];
            }
        }
    }

    /// Segments of a row, or `None` when the row fits in one wrap row.
    pub fn wrapped_row(&self, row: DisplayRow) -> Option<&WrappedRow> {
        self.wrapped.get(&row)
    }

    /// How many wrap rows a display row occupies.
    pub fn segment_count(&self, row: DisplayRow) -> u32 {
        if !self.is_enabled() {
            return 1;
        }
        self.counts.get(row as usize).copied().unwrap_or(1)
    }

    /// The first wrap row of a display row.
    pub fn first_wrap_row(&self, row: DisplayRow) -> WrapRow {
        if !self.is_enabled() {
            return row.min(self.rows);
        }
        let row = row.min(self.rows) as usize;
        let mut sum = 0;
        let mut ix = row;
        while ix > 0 {
            sum += self.tree[ix];
            ix -= ix & ix.wrapping_neg();
        }
        sum
    }

    /// The wrap row of a given segment of a display row.
    pub fn to_wrap_row(&self, row: DisplayRow, segment: u32) -> WrapRow {
        self.first_wrap_row(row) + segment.min(self.segment_count(row).saturating_sub(1))
    }

    /// The display row and segment a wrap row belongs to.
    pub fn to_display(&self, wrap_row: WrapRow) -> (DisplayRow, u32) {
        if !self.is_enabled() {
            return (wrap_row.min(self.rows.saturating_sub(1)), 0);
        }
        if self.rows == 0 {
            return (0, 0);
        }
        let target = wrap_row.min(self.total.saturating_sub(1));
        // Fenwick binary lifting: the largest prefix whose sum is <= target.
        let mut ix = 0usize;
        let mut remaining = target;
        let mut step = (self.tree.len() - 1).next_power_of_two();
        while step > 0 {
            let next = ix + step;
            if next < self.tree.len() && self.tree[next] <= remaining {
                remaining -= self.tree[next];
                ix = next;
            }
            step /= 2;
        }
        let row = (ix as u32).min(self.rows - 1);
        (
            row,
            remaining.min(self.segment_count(row).saturating_sub(1)),
        )
    }

    /// Byte range of a segment inside its row text.
    pub fn segment_range(&self, row: DisplayRow, segment: u32, row_len: u32) -> Range<u32> {
        match self.wrapped.get(&row) {
            Some(wrapped) => wrapped.segment(segment as usize, row_len),
            None => 0..row_len,
        }
    }

    /// Columns the continuation segments of a row are indented by.
    pub fn indent(&self, row: DisplayRow) -> u32 {
        self.wrapped.get(&row).map_or(0, |wrapped| wrapped.indent)
    }

    /// The segment of `row` that contains `byte`.
    pub fn segment_for_byte(&self, row: DisplayRow, byte: u32) -> u32 {
        let Some(wrapped) = self.wrapped.get(&row) else {
            return 0;
        };
        match wrapped.starts.binary_search(&byte) {
            Ok(ix) => ix as u32,
            Err(ix) => (ix as u32).saturating_sub(1),
        }
    }
}

/// Columns a character advances.
fn char_width(ch: char, tab_size: u32) -> u32 {
    if ch == '\t' { tab_size.max(1) } else { 1 }
}

/// Splits one row into soft-wrap segments, breaking after whitespace when
/// possible and mid-word when a single word is wider than the viewport.
pub fn wrap_row(text: &str, columns: u32, tab_size: u32) -> WrappedRow {
    let widths: Vec<u32> = text.chars().map(|ch| char_width(ch, tab_size)).collect();
    wrap_row_by_widths(text, columns, &widths)
}

/// [`wrap_row`] with the advance of every character given (one entry per
/// `char` of `text`, in the unit of `columns`); a missing entry counts as 1.
pub fn wrap_row_by_widths(text: &str, columns: u32, widths: &[u32]) -> WrappedRow {
    let columns = columns.max(1);
    let chars: Vec<(u32, u32, bool)> = text
        .char_indices()
        .enumerate()
        .map(|(ix, (byte, ch))| {
            (
                byte as u32,
                widths.get(ix).copied().unwrap_or(1),
                ch.is_whitespace(),
            )
        })
        .collect();

    // Continuation rows keep the indentation of the line, capped at half the
    // viewport so a deeply indented line still shows something.
    let mut indent = 0;
    for (ix, ch) in text.chars().enumerate() {
        if ch == ' ' || ch == '\t' {
            indent += widths.get(ix).copied().unwrap_or(1);
        } else {
            break;
        }
    }
    let indent = indent.min(columns / 2);

    let mut starts = vec![0u32];
    let mut segment_start = 0usize;
    let mut column = 0u32;
    let mut limit = columns;
    let mut last_break: Option<usize> = None;
    let mut ix = 0usize;
    while ix < chars.len() {
        let (byte, width, is_space) = chars[ix];
        if column + width > limit && ix > segment_start {
            let break_ix = last_break
                .filter(|candidate| *candidate > segment_start && *candidate < chars.len())
                .unwrap_or(ix);
            starts.push(chars[break_ix].0);
            segment_start = break_ix;
            ix = break_ix;
            column = 0;
            last_break = None;
            limit = columns.saturating_sub(indent).max(1);
            continue;
        }
        let _ = byte;
        column += width;
        if is_space {
            last_break = Some(ix + 1);
        }
        ix += 1;
    }
    WrappedRow { starts, indent }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rows(Vec<String>);

    impl WrapSource for Rows {
        fn row_count(&self) -> u32 {
            self.0.len() as u32
        }
        fn row_len(&self, row: u32) -> u32 {
            self.0[row as usize].len() as u32
        }
        fn row_text(&self, row: u32) -> String {
            self.0[row as usize].clone()
        }
    }

    fn rows(lines: &[&str]) -> Rows {
        Rows(lines.iter().map(|line| line.to_string()).collect())
    }

    #[test]
    fn disabled_map_is_the_identity() {
        let mut map = WrapMap::new();
        map.rebuild(None, 4, &rows(&["una linea muy larga", "otra"]));
        assert!(!map.is_enabled());
        assert_eq!(map.wrap_row_count(), 2);
        assert_eq!(map.first_wrap_row(1), 1);
        assert_eq!(map.to_display(1), (1, 0));
        assert_eq!(map.segment_count(0), 1);
    }

    #[test]
    fn short_rows_do_not_wrap() {
        let mut map = WrapMap::new();
        map.rebuild(Some(20), 4, &rows(&["hola", "chau"]));
        assert_eq!(map.wrap_row_count(), 2);
        assert_eq!(map.segment_count(0), 1);
        assert!(map.wrapped_row(0).is_none());
    }

    #[test]
    fn a_long_row_wraps_at_a_space() {
        // 10 columns: "uno dos " | "tres"
        let wrapped = wrap_row("uno dos tres", 10, 4);
        assert_eq!(wrapped.starts, vec![0, 8]);
        assert_eq!(wrapped.indent, 0);
    }

    #[test]
    fn a_word_longer_than_the_viewport_breaks_mid_word() {
        let wrapped = wrap_row("aaaaaaaaaaaaaaa", 5, 4);
        assert_eq!(wrapped.starts, vec![0, 5, 10]);
    }

    #[test]
    fn continuation_rows_keep_the_indentation() {
        let wrapped = wrap_row("    let x = uno dos tres cuatro cinco;", 20, 4);
        assert_eq!(wrapped.indent, 4);
        assert!(wrapped.starts.len() > 1);
        assert_eq!(wrapped.starts[0], 0);
    }

    #[test]
    fn tabs_count_as_tab_size_columns() {
        let wrapped = wrap_row("\t\tabc", 4, 4);
        // Each tab already fills the width, so each one gets its own row; the
        // continuation rows are indented by 2 (half the viewport), which leaves
        // room for two characters.
        assert_eq!(wrapped.starts, vec![0, 1, 2, 4]);
        assert_eq!(wrapped.indent, 2);
    }

    #[test]
    fn mapping_round_trips_through_the_fenwick_tree() {
        let mut map = WrapMap::new();
        map.rebuild(
            Some(10),
            4,
            &rows(&[
                "corta",                 // 1 wrap row
                "uno dos tres cuatro",   // wraps
                "otra corta",            // 1
                "aaaaaaaaaaaaaaaaaaaaa", // wraps 3 times
            ]),
        );
        let mut expected = 0;
        for row in 0..map.row_count() {
            assert_eq!(map.first_wrap_row(row), expected, "row {row}");
            for segment in 0..map.segment_count(row) {
                assert_eq!(
                    map.to_display(expected + segment),
                    (row, segment),
                    "row {row} segment {segment}"
                );
            }
            expected += map.segment_count(row);
        }
        assert_eq!(map.wrap_row_count(), expected);
    }

    #[test]
    fn segment_ranges_cover_the_row() {
        let mut map = WrapMap::new();
        let source = rows(&["uno dos tres cuatro cinco"]);
        map.rebuild(Some(10), 4, &source);
        let len = source.row_len(0);
        let mut previous = 0;
        for segment in 0..map.segment_count(0) {
            let range = map.segment_range(0, segment, len);
            assert_eq!(range.start, previous);
            previous = range.end;
        }
        assert_eq!(previous, len);
    }

    #[test]
    fn segment_for_byte_finds_the_right_segment() {
        let mut map = WrapMap::new();
        map.rebuild(Some(10), 4, &rows(&["uno dos tres cuatro cinco"]));
        let wrapped = map.wrapped_row(0).expect("wrapped").clone();
        assert_eq!(map.segment_for_byte(0, 0), 0);
        assert_eq!(map.segment_for_byte(0, wrapped.starts[1]), 1);
        assert_eq!(map.segment_for_byte(0, wrapped.starts[1] - 1), 0);
    }

    #[test]
    fn updating_one_row_keeps_the_tree_consistent() {
        let mut lines = vec!["corta".to_string(), "otra".to_string(), "mas".to_string()];
        let mut map = WrapMap::new();
        map.rebuild(Some(10), 4, &Rows(lines.clone()));
        assert_eq!(map.wrap_row_count(), 3);

        lines[1] = "uno dos tres cuatro cinco".to_string();
        map.update_row(1, &Rows(lines.clone()));
        assert_eq!(map.wrap_row_count(), 3 - 1 + map.segment_count(1));
        assert_eq!(map.first_wrap_row(2), 1 + map.segment_count(1));
        assert_eq!(map.to_display(1), (1, 0));

        lines[1] = "otra".to_string();
        map.update_row(1, &Rows(lines));
        assert_eq!(map.wrap_row_count(), 3);
        assert_eq!(map.first_wrap_row(2), 2);
    }

    #[test]
    fn an_empty_map_answers_without_panicking() {
        let map = WrapMap::new();
        assert_eq!(map.wrap_row_count(), 0);
        assert_eq!(map.to_display(5), (0, 0));
    }
}
