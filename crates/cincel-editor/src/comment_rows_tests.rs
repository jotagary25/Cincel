//! Comment blocks among the rows (spec 09 §6.5.1, D10, §6.10): the arrow
//! keys skip them, a block above the top row keeps the view still, they go
//! below every segment of a soft-wrapped line, below a hunk's red rows, and
//! they follow rows typed above them.

#![allow(clippy::single_range_in_vec_init)]

use gpui::{TestAppContext, px};

use crate::comment_box_tests::{
    comment, frame, geometry, hunk, open, review, select, set_comments, set_cursor,
};
use crate::comments::{CommentBlockKind, DraftTarget, ReviewCommentView};
use crate::display_map::DisplayPoint;
use crate::review::ReviewView;
use crate::settings::EditorSettings;

fn lines(count: usize) -> String {
    (0..count).map(|ix| format!("línea {ix}\n")).collect()
}

#[gpui::test]
fn the_arrow_keys_go_from_the_row_above_a_box_to_the_row_below(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open(
        cx,
        &lines(10),
        ReviewView::default(),
        EditorSettings::default(),
    );
    set_comments(
        &view,
        &host,
        &mut visual,
        vec![comment(7, 2..3, "abajo de la 3")],
    );
    let block = frame(&view, &mut visual).review.blocks[0];
    assert_eq!(block.visual_row, 3);

    set_cursor(&view, &mut visual, DisplayPoint::new(2, 3));
    visual.simulate_keystrokes("down");
    let cursor = visual.update(|_, cx| view.read(cx).cursor);
    assert_eq!(cursor, DisplayPoint::new(3, 3), "never on the box");
    visual.simulate_keystrokes("up");
    let cursor = visual.update(|_, cx| view.read(cx).cursor);
    assert_eq!(cursor, DisplayPoint::new(2, 3));
    // Selecting across it too.
    visual.simulate_keystrokes("shift-down shift-down");
    let selection = visual.update(|_, cx| view.read(cx).selection_range());
    assert_eq!(selection.end, DisplayPoint::new(4, 3));
}

#[gpui::test]
fn a_click_on_the_row_below_a_box_lands_on_that_row(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open(
        cx,
        &lines(10),
        ReviewView::default(),
        EditorSettings::default(),
    );
    set_comments(&view, &host, &mut visual, vec![comment(7, 2..3, "x")]);
    let painted = frame(&view, &mut visual);
    let block = painted.review.blocks[0];
    let (line_height, text_x, _) = geometry(&view, &mut visual);
    let row_below = painted.row(3).expect("painted").y;
    assert_eq!(row_below, block.bounds.bottom());
    crate::comment_box_tests::click(
        &mut visual,
        text_x + 1.,
        f32::from(row_below) + line_height / 2.,
    );
    let cursor = visual.update(|_, cx| view.read(cx).cursor);
    assert_eq!(cursor.row, 3);
}

#[gpui::test]
fn a_block_above_the_top_row_keeps_the_view_still(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open(
        cx,
        &lines(300),
        ReviewView::default(),
        EditorSettings::default(),
    );
    let (line_height, _, _) = geometry(&view, &mut visual);
    visual.update(|_, cx| view.update(cx, |view, cx| view.set_scroll_row(100., cx)));
    let before = frame(&view, &mut visual);
    let top_before = before
        .rows
        .iter()
        .find(|row| row.y >= px(0.))
        .copied()
        .unwrap();

    // A comment far above the viewport.
    set_comments(
        &view,
        &host,
        &mut visual,
        vec![comment(7, 10..11, "arriba")],
    );
    let after = frame(&view, &mut visual);
    let block = after.review.blocks.first().copied();
    assert!(
        block.is_none(),
        "the block is off screen and is never built"
    );
    let rows = after.visual_rows - after.wrap_rows;
    assert!(rows > 0);
    assert_eq!(
        after.scroll_top,
        before.scroll_top + rows as f32 * line_height
    );
    let top_after = after
        .rows
        .iter()
        .find(|row| row.display_row == top_before.display_row)
        .copied()
        .unwrap();
    assert_eq!(
        top_after.y, top_before.y,
        "the row being looked at stays put"
    );

    // It goes away again: the same row stays on top.
    set_comments(&view, &host, &mut visual, Vec::new());
    let gone = frame(&view, &mut visual);
    assert_eq!(gone.scroll_top, before.scroll_top);

    // One below the top row moves nothing.
    set_comments(
        &view,
        &host,
        &mut visual,
        vec![comment(8, 110..111, "abajo")],
    );
    let below = frame(&view, &mut visual);
    assert_eq!(below.scroll_top, before.scroll_top);
}

#[gpui::test]
fn a_block_goes_below_every_segment_of_a_wrapped_line(cx: &mut TestAppContext) {
    let long = "palabra ".repeat(200);
    let text = format!("corta\n{long}\nfin\n");
    let (view, _handle, mut visual, host) =
        open(cx, &text, ReviewView::default(), EditorSettings::default());
    let plain = frame(&view, &mut visual);
    let segments = plain.rows.iter().filter(|row| row.display_row == 1).count() as u32;
    assert!(segments > 1, "the long line wraps");

    set_comments(
        &view,
        &host,
        &mut visual,
        vec![comment(7, 1..2, "sobre la larga")],
    );
    let painted = frame(&view, &mut visual);
    let block = painted.review.blocks[0];
    // Wrap rows: 0 corta, 1..=segments the long one; the block right after.
    assert_eq!(block.visual_row, 1 + segments);
    let last_segment = painted
        .rows
        .iter()
        .filter(|row| row.display_row == 1)
        .map(|row| row.y)
        .fold(px(0.), |a, b| a.max(b));
    let (line_height, _, _) = geometry(&view, &mut visual);
    assert_eq!(last_segment + px(line_height), block.bounds.top());
    assert_eq!(painted.row(2).unwrap().y, block.bounds.bottom());
    // Its mark is on the first segment only.
    assert_eq!(painted.review.comment_marks.len(), 1);
    assert_eq!(painted.review.comment_marks[0].display_row, 1);

    // The open box wraps its own text and grows with it, 2 to 8 rows.
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    visual.simulate_keystrokes("ctrl-shift-m");
    let small = frame(&view, &mut visual)
        .review
        .blocks
        .into_iter()
        .find(|block| block.kind == CommentBlockKind::Open)
        .unwrap();
    visual.simulate_input(&"texto largo ".repeat(120));
    let grown = frame(&view, &mut visual)
        .review
        .blocks
        .into_iter()
        .find(|block| block.kind == CommentBlockKind::Open)
        .unwrap();
    assert!(grown.rows > small.rows, "{} > {}", grown.rows, small.rows);
}

#[gpui::test]
fn a_comment_on_a_hunk_with_red_rows_goes_below_them(cx: &mut TestAppContext) {
    let text = "a\nb\nc\nd\ne\nf\n";
    // Hunk 10: "x", "y" replaced by row 1 ("b"); hunk 20: "z" deleted before
    // row 4 (a pure deletion).
    let hunks = review(vec![hunk(10, &["x", "y"], 1..2), hunk(20, &["z"], 4..4)]);
    let (view, _handle, mut visual, host) = open(cx, text, hunks, EditorSettings::default());
    // Display: 0 a, 1 x, 2 y, 3 b, 4 c, 5 d, 6 z, 7 e, 8 f.
    set_comments(
        &view,
        &host,
        &mut visual,
        vec![
            ReviewCommentView {
                from_hunk: Some(10),
                ..comment(1, 1..2, "sobre el cambio")
            },
            ReviewCommentView {
                from_hunk: Some(20),
                ..comment(2, 4..4, "sobre lo borrado")
            },
        ],
    );
    let painted = frame(&view, &mut visual);
    let blocks = &painted.review.blocks;
    assert_eq!(blocks.len(), 2);
    // Below "b" (the hunk's last row, display 3).
    assert_eq!(blocks[0].id, 1);
    assert_eq!(blocks[0].visual_row, 4);
    // Below the red "z" (display 6), the second block after the first one's
    // rows.
    assert_eq!(blocks[1].id, 2);
    assert_eq!(blocks[1].visual_row, 7 + blocks[0].rows);
    // The marks are on each hunk's first row: its first red row.
    let marks: Vec<(u64, u32)> = painted
        .review
        .comment_marks
        .iter()
        .map(|mark| (mark.id, mark.display_row))
        .collect();
    assert_eq!(marks, vec![(1, 1), (2, 6)]);
    // The red rows keep their colours and the rows below their numbers.
    use crate::display_map::RowKind;
    assert_eq!(painted.row(1).unwrap().kind, RowKind::Phantom(0));
    assert_eq!(painted.row(6).unwrap().kind, RowKind::Phantom(1));
    assert!(painted.row(7).unwrap().line_number);

    // A selection on the red rows comments the real row they sit on.
    select(
        &view,
        &mut visual,
        DisplayPoint::new(1, 0),
        DisplayPoint::new(2, 1),
    );
    visual.simulate_keystrokes("ctrl-shift-m");
    let draft = visual.update(|_, cx| view.read(cx).comment_draft().cloned());
    assert_eq!(
        draft,
        Some(DraftTarget::New {
            rows: 1..2,
            from_hunk: None,
        })
    );
}

#[gpui::test]
fn comments_follow_rows_typed_above_them(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open(
        cx,
        &lines(10),
        ReviewView::default(),
        EditorSettings::default(),
    );
    set_comments(&view, &host, &mut visual, vec![comment(7, 5..7, "sigue")]);
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    visual.simulate_keystrokes("enter enter");
    let rows = visual.update(|_, cx| view.read(cx).comments()[0].rows.clone());
    assert_eq!(rows, 7..9);
    let painted = frame(&view, &mut visual);
    assert_eq!(painted.review.comment_marks[0].display_row, 7);
    assert_eq!(painted.review.blocks[0].visual_row, 9);
    // Typing inside the range grows it.
    set_cursor(&view, &mut visual, DisplayPoint::new(7, 2));
    visual.simulate_keystrokes("enter");
    let rows = visual.update(|_, cx| view.read(cx).comments()[0].rows.clone());
    assert_eq!(rows, 7..10);
}

#[gpui::test]
fn the_scroll_reaches_the_last_row_past_the_blocks(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open(
        cx,
        &lines(200),
        ReviewView::default(),
        EditorSettings::default(),
    );
    set_comments(
        &view,
        &host,
        &mut visual,
        vec![comment(1, 10..11, "uno"), comment(2, 190..191, "dos")],
    );
    let painted = frame(&view, &mut visual);
    assert!(painted.visual_rows > painted.wrap_rows);
    visual.simulate_keystrokes("ctrl-end");
    let painted = frame(&view, &mut visual);
    // The last row (display 200, the empty one) is painted inside the view.
    let last = painted.row(200).expect("the last row is on screen");
    let (line_height, _, bounds) = geometry(&view, &mut visual);
    assert!(last.y + px(line_height) <= bounds.bottom() + px(0.5));
}
