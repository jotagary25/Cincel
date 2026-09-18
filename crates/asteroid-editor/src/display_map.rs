//! Display pipeline of the editor, E0 subset.
//!
//! ```text
//! Buffer (rope) -> DiffTransformMap -> [WrapMap, BlockMap: v1] -> EditorElement
//! ```
//!
//! [`DiffTransformMap`] is the "text splice" layer described in
//! `docs/research/04-repos-zed-lapce.md` (adenda 04-c): the deleted text of a
//! hunk lives *below* the display map as real rows, so it can be shaped,
//! selected and copied like any other row, but it is never editable.
//!
//! Row spaces:
//! - **buffer row**: a line index inside [`asteroid_text::Buffer`],
//! - **display row**: a visual row index, i.e. buffer rows with the phantom
//!   (deleted) rows of every hunk spliced in.

use std::ops::Range;

/// A line index inside the buffer.
pub type BufferRow = u32;
/// A visual row index, phantom rows included.
pub type DisplayRow = u32;

/// A simulated review hunk: `deleted_text` is shown as read-only phantom rows
/// right before `insert_before_buffer_row`, and `added_rows` are the buffer
/// rows the agent added in their place (may be empty for a pure deletion).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhantomHunk {
    /// Buffer row the phantom rows are spliced in front of.
    pub insert_before_buffer_row: BufferRow,
    /// The deleted lines, without trailing newlines.
    pub deleted_text: Vec<String>,
    /// Buffer rows added by the agent (painted with the "added" background).
    pub added_rows: Range<BufferRow>,
}

impl PhantomHunk {
    /// Number of phantom rows this hunk contributes.
    pub fn phantom_len(&self) -> u32 {
        self.deleted_text.len() as u32
    }
}

/// What a display row maps back to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayCell {
    /// A real, editable buffer row.
    Buffer(BufferRow),
    /// A read-only phantom row: `line_ix` inside `hunks[hunk_ix].deleted_text`.
    Phantom {
        /// Index of the hunk in [`DiffTransformMap::hunks`].
        hunk_ix: usize,
        /// Index of the line inside `deleted_text`.
        line_ix: usize,
    },
}

/// How a display row must be painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// Plain row, outside of any hunk.
    Normal,
    /// A read-only deleted row.
    Phantom(usize),
    /// A real row added by the agent.
    Added(usize),
}

/// A position in display space. `column` is a byte offset inside the row text
/// (phantom rows included), always on a `char` boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DisplayPoint {
    /// Display row.
    pub row: DisplayRow,
    /// Byte offset inside the row.
    pub column: u32,
}

impl DisplayPoint {
    /// Builds a new display point.
    pub fn new(row: DisplayRow, column: u32) -> Self {
        Self { row, column }
    }
}

/// The splice layer: maps buffer rows to display rows and back.
#[derive(Clone, Debug, Default)]
pub struct DiffTransformMap {
    hunks: Vec<PhantomHunk>,
    /// `starts[i]` = display row where hunk `i`'s phantom block begins.
    starts: Vec<DisplayRow>,
    /// `cumulative[i]` = phantom rows contributed by hunks `0..i`.
    cumulative: Vec<u32>,
}

impl DiffTransformMap {
    /// Builds the map from a set of hunks (sorted internally by row).
    pub fn new(mut hunks: Vec<PhantomHunk>) -> Self {
        hunks.sort_by_key(|hunk| hunk.insert_before_buffer_row);
        let mut starts = Vec::with_capacity(hunks.len());
        let mut cumulative = Vec::with_capacity(hunks.len() + 1);
        let mut total = 0;
        for hunk in &hunks {
            cumulative.push(total);
            starts.push(hunk.insert_before_buffer_row + total);
            total += hunk.phantom_len();
        }
        cumulative.push(total);
        Self {
            hunks,
            starts,
            cumulative,
        }
    }

    /// The hunks, sorted by `insert_before_buffer_row`.
    pub fn hunks(&self) -> &[PhantomHunk] {
        &self.hunks
    }

    /// Total number of phantom rows.
    pub fn phantom_row_count(&self) -> u32 {
        *self.cumulative.last().unwrap_or(&0)
    }

    /// First display row of the phantom block of `hunk_ix`.
    pub fn phantom_start(&self, hunk_ix: usize) -> DisplayRow {
        self.starts[hunk_ix]
    }

    /// Display row of a buffer row.
    pub fn to_display_row(&self, buffer_row: BufferRow) -> DisplayRow {
        // Every hunk spliced in at or before `buffer_row` pushes it down.
        let count = self
            .hunks
            .partition_point(|hunk| hunk.insert_before_buffer_row <= buffer_row);
        buffer_row + self.cumulative[count]
    }

    /// What a display row maps back to.
    pub fn to_buffer(&self, display_row: DisplayRow) -> DisplayCell {
        let after = self.starts.partition_point(|start| *start <= display_row);
        if after > 0 {
            let ix = after - 1;
            let offset = display_row - self.starts[ix];
            if offset < self.hunks[ix].phantom_len() {
                return DisplayCell::Phantom {
                    hunk_ix: ix,
                    line_ix: offset as usize,
                };
            }
        }
        DisplayCell::Buffer(display_row - self.cumulative[after])
    }

    /// How a display row must be painted.
    pub fn row_kind(&self, display_row: DisplayRow) -> RowKind {
        match self.to_buffer(display_row) {
            DisplayCell::Phantom { hunk_ix, .. } => RowKind::Phantom(hunk_ix),
            DisplayCell::Buffer(buffer_row) => {
                for (ix, hunk) in self.hunks.iter().enumerate() {
                    if hunk.added_rows.contains(&buffer_row) {
                        return RowKind::Added(ix);
                    }
                }
                RowKind::Normal
            }
        }
    }

    /// Display rows covered by a hunk: its phantom block plus its added rows.
    pub fn hunk_display_range(&self, hunk_ix: usize) -> Range<DisplayRow> {
        let hunk = &self.hunks[hunk_ix];
        let start = self.starts[hunk_ix];
        let end = if hunk.added_rows.is_empty() {
            start + hunk.phantom_len()
        } else {
            self.to_display_row(hunk.added_rows.end - 1) + 1
        };
        start..end
    }

    /// Index of the hunk covering `display_row`, if any.
    pub fn hunk_at_display_row(&self, display_row: DisplayRow) -> Option<usize> {
        (0..self.hunks.len()).find(|ix| self.hunk_display_range(*ix).contains(&display_row))
    }

    /// The buffer row an edit made on `display_row` must be redirected to:
    /// phantom rows are read-only, so edits land on the first real row below.
    pub fn edit_target_row(&self, display_row: DisplayRow) -> BufferRow {
        match self.to_buffer(display_row) {
            DisplayCell::Buffer(row) => row,
            DisplayCell::Phantom { hunk_ix, .. } => self.hunks[hunk_ix].insert_before_buffer_row,
        }
    }
}

/// `DiffTransformMap` + the buffer row count: the full display space of E0.
/// `WrapMap` and `BlockMap` are not implemented yet (no soft wrap in E0; the
/// accept/reject pill is painted by the element instead of being a block).
#[derive(Clone, Debug, Default)]
pub struct DisplayMap {
    buffer_rows: u32,
    diff: DiffTransformMap,
}

impl DisplayMap {
    /// Builds a display map for a buffer with `buffer_rows` lines.
    pub fn new(buffer_rows: u32, hunks: Vec<PhantomHunk>) -> Self {
        Self {
            buffer_rows,
            diff: DiffTransformMap::new(hunks),
        }
    }

    /// The splice layer.
    pub fn diff(&self) -> &DiffTransformMap {
        &self.diff
    }

    /// Number of buffer rows.
    pub fn buffer_row_count(&self) -> u32 {
        self.buffer_rows
    }

    /// Number of display rows (buffer rows + phantom rows).
    pub fn display_row_count(&self) -> u32 {
        self.buffer_rows + self.diff.phantom_row_count()
    }

    /// Display row of a buffer row.
    pub fn to_display_row(&self, buffer_row: BufferRow) -> DisplayRow {
        self.diff.to_display_row(buffer_row)
    }

    /// What a display row maps back to.
    pub fn to_buffer(&self, display_row: DisplayRow) -> DisplayCell {
        self.diff.to_buffer(display_row)
    }

    /// Clamps a display row into the valid range.
    pub fn clip_row(&self, display_row: DisplayRow) -> DisplayRow {
        display_row.min(self.display_row_count().saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(insert_before: u32, deleted: &[&str], added: Range<u32>) -> PhantomHunk {
        PhantomHunk {
            insert_before_buffer_row: insert_before,
            deleted_text: deleted.iter().map(|line| line.to_string()).collect(),
            added_rows: added,
        }
    }

    fn roundtrip(map: &DisplayMap) {
        // Every buffer row maps to a display row that maps back to it.
        for buffer_row in 0..map.buffer_row_count() {
            let display_row = map.to_display_row(buffer_row);
            assert_eq!(
                map.to_buffer(display_row),
                DisplayCell::Buffer(buffer_row),
                "buffer row {buffer_row} -> display row {display_row}"
            );
        }
        // Display rows are a partition: monotonic and without holes.
        let mut expected_buffer_row = 0;
        for display_row in 0..map.display_row_count() {
            match map.to_buffer(display_row) {
                DisplayCell::Buffer(row) => {
                    assert_eq!(row, expected_buffer_row, "at display row {display_row}");
                    expected_buffer_row += 1;
                }
                DisplayCell::Phantom { hunk_ix, line_ix } => {
                    let hunk = &map.diff().hunks()[hunk_ix];
                    assert!(line_ix < hunk.deleted_text.len());
                }
            }
        }
        assert_eq!(expected_buffer_row, map.buffer_row_count());
    }

    #[test]
    fn no_hunks_is_identity() {
        let map = DisplayMap::new(10, vec![]);
        assert_eq!(map.display_row_count(), 10);
        for row in 0..10 {
            assert_eq!(map.to_display_row(row), row);
            assert_eq!(map.to_buffer(row), DisplayCell::Buffer(row));
            assert_eq!(map.diff().row_kind(row), RowKind::Normal);
        }
        roundtrip(&map);
    }

    #[test]
    fn single_hunk_in_the_middle() {
        // 2 deleted rows spliced before buffer row 3, 3 added rows 3..6.
        let map = DisplayMap::new(10, vec![hunk(3, &["viejo a", "viejo b"], 3..6)]);
        assert_eq!(map.display_row_count(), 12);

        assert_eq!(map.to_display_row(0), 0);
        assert_eq!(map.to_display_row(2), 2);
        assert_eq!(map.to_display_row(3), 5);
        assert_eq!(map.to_display_row(9), 11);

        assert_eq!(map.to_buffer(2), DisplayCell::Buffer(2));
        assert_eq!(
            map.to_buffer(3),
            DisplayCell::Phantom {
                hunk_ix: 0,
                line_ix: 0
            }
        );
        assert_eq!(
            map.to_buffer(4),
            DisplayCell::Phantom {
                hunk_ix: 0,
                line_ix: 1
            }
        );
        assert_eq!(map.to_buffer(5), DisplayCell::Buffer(3));

        assert_eq!(map.diff().row_kind(2), RowKind::Normal);
        assert_eq!(map.diff().row_kind(3), RowKind::Phantom(0));
        assert_eq!(map.diff().row_kind(5), RowKind::Added(0));
        assert_eq!(map.diff().row_kind(7), RowKind::Added(0));
        assert_eq!(map.diff().row_kind(8), RowKind::Normal);
        assert_eq!(map.diff().hunk_display_range(0), 3..8);
        assert_eq!(map.diff().hunk_at_display_row(4), Some(0));
        assert_eq!(map.diff().hunk_at_display_row(8), None);
        assert_eq!(map.diff().edit_target_row(3), 3);
        assert_eq!(map.diff().edit_target_row(4), 3);
        roundtrip(&map);
    }

    #[test]
    fn hunk_at_row_zero() {
        let map = DisplayMap::new(5, vec![hunk(0, &["borrada"], 0..1)]);
        assert_eq!(map.display_row_count(), 6);
        assert_eq!(
            map.to_buffer(0),
            DisplayCell::Phantom {
                hunk_ix: 0,
                line_ix: 0
            }
        );
        assert_eq!(map.to_buffer(1), DisplayCell::Buffer(0));
        assert_eq!(map.to_display_row(0), 1);
        assert_eq!(map.diff().hunk_display_range(0), 0..2);
        assert_eq!(map.diff().edit_target_row(0), 0);
        roundtrip(&map);
    }

    #[test]
    fn hunk_at_last_row() {
        let map = DisplayMap::new(5, vec![hunk(4, &["ultima vieja"], 4..5)]);
        assert_eq!(map.display_row_count(), 6);
        assert_eq!(map.to_display_row(3), 3);
        assert_eq!(
            map.to_buffer(4),
            DisplayCell::Phantom {
                hunk_ix: 0,
                line_ix: 0
            }
        );
        assert_eq!(map.to_buffer(5), DisplayCell::Buffer(4));
        assert_eq!(map.diff().hunk_display_range(0), 4..6);
        roundtrip(&map);
    }

    #[test]
    fn pure_deletion_has_no_added_rows() {
        let map = DisplayMap::new(5, vec![hunk(2, &["se fue"], 2..2)]);
        assert_eq!(map.display_row_count(), 6);
        assert_eq!(map.diff().row_kind(2), RowKind::Phantom(0));
        assert_eq!(map.diff().row_kind(3), RowKind::Normal);
        assert_eq!(map.diff().hunk_display_range(0), 2..3);
        assert_eq!(map.diff().edit_target_row(2), 2);
        roundtrip(&map);
    }

    #[test]
    fn many_hunks() {
        let map = DisplayMap::new(
            100,
            vec![
                hunk(20, &["a", "b"], 20..23),
                hunk(60, &["c"], 60..60),
                hunk(5, &["d", "e", "f"], 5..6),
            ],
        );
        // Hunks are sorted on construction.
        assert_eq!(map.diff().hunks()[0].insert_before_buffer_row, 5);
        assert_eq!(map.diff().hunks()[1].insert_before_buffer_row, 20);
        assert_eq!(map.diff().hunks()[2].insert_before_buffer_row, 60);
        assert_eq!(map.display_row_count(), 100 + 3 + 2 + 1);

        assert_eq!(map.to_display_row(4), 4);
        assert_eq!(map.to_display_row(5), 8);
        assert_eq!(map.to_display_row(19), 22);
        assert_eq!(map.to_display_row(20), 25);
        assert_eq!(map.to_display_row(59), 64);
        assert_eq!(map.to_display_row(60), 66);

        assert_eq!(map.diff().phantom_start(0), 5);
        assert_eq!(map.diff().phantom_start(1), 23);
        assert_eq!(map.diff().phantom_start(2), 65);
        assert_eq!(map.diff().row_kind(65), RowKind::Phantom(2));
        assert_eq!(map.diff().row_kind(66), RowKind::Normal);
        roundtrip(&map);
    }

    #[test]
    fn adjacent_hunks_on_the_same_row() {
        let map = DisplayMap::new(10, vec![hunk(4, &["x"], 4..5), hunk(4, &["y"], 4..5)]);
        assert_eq!(map.display_row_count(), 12);
        assert_eq!(
            map.to_buffer(4),
            DisplayCell::Phantom {
                hunk_ix: 0,
                line_ix: 0
            }
        );
        assert_eq!(
            map.to_buffer(5),
            DisplayCell::Phantom {
                hunk_ix: 1,
                line_ix: 0
            }
        );
        assert_eq!(map.to_buffer(6), DisplayCell::Buffer(4));
        roundtrip(&map);
    }

    #[test]
    fn empty_buffer() {
        let map = DisplayMap::new(1, vec![]);
        assert_eq!(map.display_row_count(), 1);
        assert_eq!(map.clip_row(50), 0);
        roundtrip(&map);
    }
}
