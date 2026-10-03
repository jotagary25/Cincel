//! `BlockMap`: whole interface rows spliced between the text rows
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §6.5, D10).
//!
//! ```text
//! Buffer -> DiffTransformMap -> WrapMap -> BlockMap -> EditorElement
//!           display rows         wrap rows  visual rows
//! ```
//!
//! A **block** is a run of `rows` visual rows that holds no text — the
//! comment box of a review hunk or of a selection — inserted right *after*
//! a wrap row. The element paints and scrolls in **visual rows**
//! (`y = visual_row × line_height`, the scroll offset and the total height),
//! while the cursor, the selection, the mouse drag and the search keep
//! working in wrap rows and simply never land on a block: the arrow keys go
//! from the row above a box to the row below it.
//!
//! Blocks take whole rows (their pixel height rounded up to a multiple of the
//! line height), so `row × line_height` stays the only geometry the element
//! knows, and the gutter and the text column never move (D16): a block is
//! painted from the left edge of the text area, with nothing in the gutter.
//!
//! Without blocks (every editor that has no comment) the map is the identity
//! and costs one comparison per call.

use std::ops::Range;

use crate::wrap_map::WrapRow;

/// A visual row index: wrap rows with the block rows spliced in.
pub type VisualRow = u32;

/// Where a block goes and how tall it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockPlacement {
    /// Identity of the block (a comment id, or [`crate::comments::NEW_COMMENT_BLOCK`]).
    pub id: u64,
    /// The block sits right below this wrap row.
    pub after_wrap_row: WrapRow,
    /// Visual rows it takes (a block of 0 rows is dropped).
    pub rows: u32,
}

/// What a visual row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualCell {
    /// A text row (wrap row).
    Wrap(WrapRow),
    /// Row `row_in_block` of block `id`.
    Block {
        /// The block.
        id: u64,
        /// Row inside it, from 0.
        row_in_block: u32,
    },
}

/// The block layer of the display pipeline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockMap {
    wrap_rows: u32,
    /// Sorted by `after_wrap_row`, stable (two blocks below the same row
    /// keep the order they were given in).
    blocks: Vec<BlockPlacement>,
    /// Visual row where block `i` starts.
    starts: Vec<VisualRow>,
    /// `before[i]` = block rows of blocks `0..i` (`blocks.len() + 1` long).
    before: Vec<u32>,
}

impl BlockMap {
    /// Builds the map over `wrap_rows` text rows. Blocks below a row past the
    /// end go below the last row, and empty blocks are dropped.
    pub fn new(wrap_rows: u32, blocks: Vec<BlockPlacement>) -> Self {
        let last = wrap_rows.saturating_sub(1);
        let mut blocks: Vec<BlockPlacement> = blocks
            .into_iter()
            .filter(|block| block.rows > 0)
            .map(|block| BlockPlacement {
                after_wrap_row: block.after_wrap_row.min(last),
                ..block
            })
            .collect();
        if blocks.is_empty() {
            // The common case (no comment in the file): nothing allocated,
            // every lookup is the identity.
            return Self {
                wrap_rows,
                ..Self::default()
            };
        }
        blocks.sort_by_key(|block| block.after_wrap_row);
        let mut starts = Vec::with_capacity(blocks.len());
        let mut before = Vec::with_capacity(blocks.len() + 1);
        let mut total = 0;
        for block in &blocks {
            before.push(total);
            starts.push(block.after_wrap_row + 1 + total);
            total += block.rows;
        }
        before.push(total);
        Self {
            wrap_rows,
            blocks,
            starts,
            before,
        }
    }

    /// The identity map over `wrap_rows` rows.
    pub fn identity(wrap_rows: u32) -> Self {
        Self::new(wrap_rows, Vec::new())
    }

    /// Whether there is no block.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The blocks, in visual order.
    pub fn blocks(&self) -> &[BlockPlacement] {
        &self.blocks
    }

    /// Text rows the map was built over.
    pub fn wrap_row_count(&self) -> u32 {
        self.wrap_rows
    }

    /// Rows taken by every block together.
    pub fn block_row_count(&self) -> u32 {
        *self.before.last().unwrap_or(&0)
    }

    /// Visual rows: text rows plus block rows.
    pub fn total_rows(&self) -> u32 {
        self.wrap_rows + self.block_row_count()
    }

    /// The visual row of a wrap row (a row past the end maps past the end).
    pub fn to_visual_row(&self, wrap_row: WrapRow) -> VisualRow {
        if self.blocks.is_empty() {
            return wrap_row;
        }
        let count = self
            .blocks
            .partition_point(|block| block.after_wrap_row < wrap_row);
        wrap_row + self.before[count]
    }

    /// What a visual row is. A row past the end is a wrap row past the end.
    pub fn from_visual_row(&self, visual_row: VisualRow) -> VisualCell {
        if self.blocks.is_empty() {
            return VisualCell::Wrap(visual_row);
        }
        let count = self.starts.partition_point(|start| *start <= visual_row);
        if count > 0 {
            let ix = count - 1;
            let offset = visual_row - self.starts[ix];
            if offset < self.blocks[ix].rows {
                return VisualCell::Block {
                    id: self.blocks[ix].id,
                    row_in_block: offset,
                };
            }
        }
        VisualCell::Wrap(visual_row - self.before[count])
    }

    /// The wrap row a visual row belongs to: itself for a text row, the row
    /// a block hangs from for a block row.
    pub fn nearest_wrap_row(&self, visual_row: VisualRow) -> WrapRow {
        match self.from_visual_row(visual_row) {
            VisualCell::Wrap(row) => row,
            VisualCell::Block { id, .. } => self
                .placement(id)
                .map(|block| block.after_wrap_row)
                .unwrap_or(0),
        }
    }

    /// The placement of a block.
    pub fn placement(&self, id: u64) -> Option<BlockPlacement> {
        self.blocks.iter().find(|block| block.id == id).copied()
    }

    /// Visual rows of a block.
    pub fn block_range(&self, id: u64) -> Option<Range<VisualRow>> {
        let ix = self.blocks.iter().position(|block| block.id == id)?;
        let start = self.starts[ix];
        Some(start..start + self.blocks[ix].rows)
    }

    /// Block rows above a wrap row (the difference between its visual row
    /// and itself).
    pub fn rows_above(&self, wrap_row: WrapRow) -> u32 {
        self.to_visual_row(wrap_row) - wrap_row
    }

    /// The wrap rows whose visual row falls in `visual` (end exclusive).
    pub fn wrap_rows_in(&self, visual: Range<VisualRow>) -> Range<WrapRow> {
        if visual.start >= visual.end {
            let row = self.first_wrap_at_or_after(visual.start);
            return row..row;
        }
        let first = self.first_wrap_at_or_after(visual.start);
        let last = match self.from_visual_row(visual.end - 1) {
            VisualCell::Wrap(row) => row + 1,
            VisualCell::Block { id, .. } => self
                .placement(id)
                .map(|block| block.after_wrap_row + 1)
                .unwrap_or(0),
        };
        first..last.max(first).min(self.wrap_rows.max(first))
    }

    fn first_wrap_at_or_after(&self, visual_row: VisualRow) -> WrapRow {
        match self.from_visual_row(visual_row) {
            VisualCell::Wrap(row) => row,
            VisualCell::Block { id, .. } => self
                .placement(id)
                .map(|block| block.after_wrap_row + 1)
                .unwrap_or(0),
        }
    }

    /// `(id, rows)` of every block, the part of the map a scroll
    /// compensation compares between two frames.
    pub fn signature(&self) -> Vec<(u64, u32)> {
        let mut signature: Vec<(u64, u32)> = self
            .blocks
            .iter()
            .map(|block| (block.id, block.rows))
            .collect();
        signature.sort_unstable();
        signature
    }

    /// How many visual rows the view must scroll so the text row on top of
    /// the viewport stays where it is when the blocks change from `old` to
    /// `self` (D10): `old_top` is the first visible visual row of `old`. A
    /// block above it that appears, goes away or changes its height moves
    /// everything below by its rows; one below it moves nothing on screen.
    pub fn compensation(&self, old: &BlockMap, old_top: VisualRow) -> i64 {
        let anchor = old.nearest_wrap_row(old_top);
        // A viewport that starts inside a block keeps that block's own rows
        // out of the count: it is the one being looked at.
        let anchor_visual_old = old.to_visual_row(anchor) as i64;
        let anchor_visual_new = self.to_visual_row(anchor) as i64;
        anchor_visual_new - anchor_visual_old
    }
}
