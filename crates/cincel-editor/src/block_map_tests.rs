//! `BlockMap` (spec 09 §6.5.1, D10, §6.11): row conversions, the cursor
//! never landing on a block, the total height and the scroll compensation.

use crate::block_map::{BlockMap, BlockPlacement, VisualCell};

fn block(id: u64, after_wrap_row: u32, rows: u32) -> BlockPlacement {
    BlockPlacement {
        id,
        after_wrap_row,
        rows,
    }
}

/// Every wrap row maps to a visual row that maps back to it, and the visual
/// rows are a partition of text rows and block rows, in order.
fn roundtrip(map: &BlockMap) {
    for wrap in 0..map.wrap_row_count() {
        let visual = map.to_visual_row(wrap);
        assert_eq!(
            map.from_visual_row(visual),
            VisualCell::Wrap(wrap),
            "wrap {wrap}"
        );
    }
    let mut expected_wrap = 0;
    let mut block_rows = 0;
    for visual in 0..map.total_rows() {
        match map.from_visual_row(visual) {
            VisualCell::Wrap(row) => {
                assert_eq!(row, expected_wrap, "visual row {visual}");
                expected_wrap += 1;
            }
            VisualCell::Block { id, row_in_block } => {
                let placement = map.placement(id).expect("a known block");
                assert!(row_in_block < placement.rows);
                // A block sits right below the row it hangs from.
                assert_eq!(expected_wrap, placement.after_wrap_row + 1);
                block_rows += 1;
            }
        }
    }
    assert_eq!(expected_wrap, map.wrap_row_count());
    assert_eq!(block_rows, map.block_row_count());
}

#[test]
fn without_blocks_the_map_is_the_identity() {
    let map = BlockMap::identity(10);
    assert!(map.is_empty());
    assert_eq!(map.total_rows(), 10);
    for row in 0..10 {
        assert_eq!(map.to_visual_row(row), row);
        assert_eq!(map.from_visual_row(row), VisualCell::Wrap(row));
    }
    roundtrip(&map);
}

#[test]
fn a_block_pushes_the_rows_below_it() {
    // Two rows of box below wrap row 3.
    let map = BlockMap::new(10, vec![block(7, 3, 2)]);
    assert_eq!(map.total_rows(), 12);
    assert_eq!(map.to_visual_row(3), 3);
    assert_eq!(map.to_visual_row(4), 6);
    assert_eq!(map.to_visual_row(9), 11);
    assert_eq!(
        map.from_visual_row(4),
        VisualCell::Block {
            id: 7,
            row_in_block: 0
        }
    );
    assert_eq!(
        map.from_visual_row(5),
        VisualCell::Block {
            id: 7,
            row_in_block: 1
        }
    );
    assert_eq!(map.from_visual_row(6), VisualCell::Wrap(4));
    assert_eq!(map.block_range(7), Some(4..6));
    assert_eq!(map.rows_above(4), 2);
    assert_eq!(map.rows_above(3), 0);
    roundtrip(&map);
}

#[test]
fn blocks_are_sorted_and_keep_their_order_on_the_same_row() {
    let map = BlockMap::new(
        20,
        vec![
            block(1, 12, 3),
            block(2, 2, 1),
            block(3, 2, 2),
            block(4, 19, 1),
        ],
    );
    let order: Vec<u64> = map.blocks().iter().map(|block| block.id).collect();
    assert_eq!(order, vec![2, 3, 1, 4]);
    assert_eq!(map.block_range(2), Some(3..4));
    assert_eq!(map.block_range(3), Some(4..6));
    assert_eq!(map.to_visual_row(3), 6);
    // Below the last row: the very end of the document.
    assert_eq!(map.block_range(4), Some(26..27));
    assert_eq!(map.total_rows(), 27);
    roundtrip(&map);
}

#[test]
fn rows_past_the_end_and_empty_blocks() {
    let map = BlockMap::new(5, vec![block(1, 40, 2), block(2, 1, 0)]);
    // The empty block is dropped, the one past the end goes below the last row.
    assert_eq!(map.blocks().len(), 1);
    assert_eq!(map.placement(1).unwrap().after_wrap_row, 4);
    assert_eq!(map.total_rows(), 7);
    roundtrip(&map);
}

#[test]
fn the_cursor_never_lands_on_a_block() {
    // A click or a drag on a block row goes to the row it hangs from; the
    // arrow keys move in wrap rows, which never include the block, so going
    // down from row 3 lands on row 4, below the box.
    let map = BlockMap::new(10, vec![block(7, 3, 3)]);
    for visual in 4..7 {
        assert_eq!(map.nearest_wrap_row(visual), 3);
    }
    assert_eq!(map.nearest_wrap_row(7), 4);
    let down_from_3 = 3 + 1;
    assert_eq!(
        map.from_visual_row(map.to_visual_row(down_from_3)),
        VisualCell::Wrap(4)
    );
}

#[test]
fn the_visible_wrap_rows_skip_the_blocks() {
    let map = BlockMap::new(10, vec![block(7, 3, 3)]);
    // Visual 2..8 = wrap 2, 3, [box ×3], wrap 4.
    assert_eq!(map.wrap_rows_in(2..8), 2..5);
    // Starting inside the box: the first text row is the one below it.
    assert_eq!(map.wrap_rows_in(5..9), 4..6);
    // Only box rows: nothing to shape.
    let inside = map.wrap_rows_in(4..6);
    assert!(inside.is_empty());
}

#[test]
fn a_block_above_the_top_row_moves_the_scroll_with_it() {
    let old = BlockMap::identity(100);
    // The viewport starts at row 40; a 3-row box appears below row 10.
    let new = BlockMap::new(100, vec![block(1, 10, 3)]);
    assert_eq!(new.compensation(&old, 40), 3);
    // It goes away again.
    assert_eq!(old.compensation(&new, 43), -3);
    // It grows by one row.
    let taller = BlockMap::new(100, vec![block(1, 10, 4)]);
    assert_eq!(taller.compensation(&new, 43), 1);
}

#[test]
fn a_block_below_the_top_row_moves_nothing() {
    let old = BlockMap::identity(100);
    let new = BlockMap::new(100, vec![block(1, 50, 3)]);
    assert_eq!(new.compensation(&old, 40), 0);
    // Right below the top row: the top row stays the top row.
    let below_top = BlockMap::new(100, vec![block(1, 40, 3)]);
    assert_eq!(below_top.compensation(&old, 40), 0);
    // A viewport that starts inside a block keeps looking at it.
    let grown = BlockMap::new(100, vec![block(1, 40, 5)]);
    assert_eq!(grown.compensation(&below_top, 42), 0);
}

#[test]
fn the_signature_ignores_where_blocks_are() {
    let a = BlockMap::new(100, vec![block(1, 10, 3), block(2, 20, 2)]);
    let moved = BlockMap::new(101, vec![block(2, 21, 2), block(1, 11, 3)]);
    assert_eq!(a.signature(), moved.signature());
    let resized = BlockMap::new(100, vec![block(1, 10, 4), block(2, 20, 2)]);
    assert_ne!(a.signature(), resized.signature());
}
