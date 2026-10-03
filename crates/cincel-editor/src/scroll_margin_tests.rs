//! The margin past the last row of a code editor and the one-row scroll keys
//! over the GPUI test harness (`docs/specs/10-etapa7-ronda2.md` §6 and §7.7):
//! the maximum scroll adds half a viewport, the wheel, `set_scroll_row` and
//! `Ctrl+↑` / `Ctrl+↓` stop there, a text field has none, the buffer and the
//! text column do not change and a click in the empty space lands on the last
//! line.

use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::comment_box_tests::{
    click, comment, frame, geometry, hunk, open, review, set_comments, set_cursor,
};
use crate::display_map::DisplayPoint;
use crate::element::{BAR_HEIGHT, BAR_MARGIN};
use crate::review::ReviewView;
use crate::settings::{EditorChrome, EditorSettings};
use crate::view::EditorView;

/// 200 short lines, no trailing newline: "linea 0" … "linea 199".
fn two_hundred_lines() -> String {
    (0..200)
        .map(|row| format!("linea {row}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The margin the spec asks for, computed independently of the editor.
fn expected_margin(viewport: f32, line_height: f32) -> f32 {
    (viewport / line_height / 2.).floor() * line_height
}

fn scroll_top(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> f32 {
    cx.update(|_window, cx| view.read(cx).scroll_top)
}

fn scroll_row(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> f32 {
    cx.update(|_window, cx| view.read(cx).scroll_row())
}

fn max_scroll_top(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> f32 {
    let (_, _, bounds) = geometry(view, cx);
    cx.update(|_window, cx| view.read(cx).max_scroll_top(f32::from(bounds.size.height)))
}

fn scroll_to_the_end(view: &Entity<EditorView>, cx: &mut VisualTestContext) {
    cx.update(|_window, cx| view.update(cx, |view, cx| view.set_scroll_row(10_000., cx)));
    cx.run_until_parked();
}

fn minimal() -> EditorSettings {
    EditorSettings {
        chrome: EditorChrome::Minimal,
        ..EditorSettings::default()
    }
}

fn close(left: f32, right: f32) -> bool {
    (left - right).abs() < 0.5
}

// -- §6.3: the maximum scroll -------------------------------------------------------

#[gpui::test]
fn the_maximum_scroll_adds_half_a_viewport_in_a_code_editor(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open(
        cx,
        &two_hundred_lines(),
        ReviewView::default(),
        EditorSettings::default(),
    );
    let painted = frame(&view, &mut visual);
    assert_eq!(painted.visual_rows, 200);
    let (line_height, _, bounds) = geometry(&view, &mut visual);
    let viewport = f32::from(bounds.size.height);
    let visible = (viewport / line_height).floor();
    assert!(
        visible > 10.,
        "the window shows a screen of rows, got {visible}"
    );

    let margin = expected_margin(viewport, line_height);
    assert!(margin >= line_height, "half a screen is at least a row");
    let expected = 200. * line_height + margin - viewport;
    let max = max_scroll_top(&view, &mut visual);
    assert!(close(max, expected), "{max} vs {expected}");
    // "(200 + visibles/2 - visibles) rows", the spec's formula.
    let in_rows = 200. + (viewport / line_height / 2.).floor() - viewport / line_height;
    assert!(close(max, in_rows * line_height), "{max} vs {in_rows} rows");

    // The painted frame reaches exactly that and not a pixel more.
    scroll_to_the_end(&view, &mut visual);
    assert!(close(scroll_top(&view, &mut visual), expected));
    let at_end = frame(&view, &mut visual);
    assert!(close(at_end.scroll_top, expected), "{}", at_end.scroll_top);
}

#[gpui::test]
fn a_text_field_keeps_the_maximum_scroll_it_had(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) =
        open(cx, &two_hundred_lines(), ReviewView::default(), minimal());
    let painted = frame(&view, &mut visual);
    assert_eq!(painted.visual_rows, 200);
    let (line_height, _, bounds) = geometry(&view, &mut visual);
    let expected = 200. * line_height - f32::from(bounds.size.height);
    assert!(expected > 0.);
    assert!(close(max_scroll_top(&view, &mut visual), expected));

    scroll_to_the_end(&view, &mut visual);
    assert!(close(scroll_top(&view, &mut visual), expected));
    // The last row ends at the bottom of the field, no empty space after it.
    let at_end = frame(&view, &mut visual);
    let last_bottom = 200. * line_height - at_end.scroll_top;
    assert!(close(last_bottom, f32::from(bounds.size.height)));
}

#[gpui::test]
fn the_last_row_stops_at_mid_screen_and_the_bar_covers_no_text(cx: &mut TestAppContext) {
    // One pending hunk on the last rows, so the floating review bar is up.
    let pending = review(vec![hunk(1, &[], 197..199)]);
    let (view, _handle, mut visual, _host) =
        open(cx, &two_hundred_lines(), pending, EditorSettings::default());
    scroll_to_the_end(&view, &mut visual);
    let painted = frame(&view, &mut visual);
    assert!(painted.review.bar.is_some(), "the review bar is painted");

    let (line_height, _, bounds) = geometry(&view, &mut visual);
    let viewport = f32::from(bounds.size.height);
    let margin = expected_margin(viewport, line_height);
    let last_row_bottom = painted.visual_rows as f32 * line_height - painted.scroll_top;
    assert!(
        close(last_row_bottom, viewport - margin),
        "the last row ends {last_row_bottom} px under the top, half screen is {}",
        viewport - margin
    );
    // Half a screen up from the bottom: `visible - floor(visible / 2)` rows.
    let visible = (viewport / line_height).floor();
    let rows_shown = visible - (visible / 2.).floor();
    assert!(last_row_bottom <= (rows_shown + 1.) * line_height + 0.5);

    // The bar sits BAR_MARGIN above the bottom edge: no text row reaches it.
    let bar_top = viewport - BAR_MARGIN - BAR_HEIGHT;
    assert!(
        last_row_bottom <= bar_top,
        "the last row ({last_row_bottom}) stays above the bar ({bar_top})"
    );
}

#[gpui::test]
fn scrolling_to_the_end_moves_neither_the_text_column_nor_the_gutter(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open(
        cx,
        &two_hundred_lines(),
        ReviewView::default(),
        EditorSettings::default(),
    );
    let before = frame(&view, &mut visual);
    scroll_to_the_end(&view, &mut visual);
    let after = frame(&view, &mut visual);
    assert!(after.scroll_top > before.scroll_top);
    assert_eq!(before.text_origin_x, after.text_origin_x);
    assert_eq!(before.gutter_width, after.gutter_width);
    assert_eq!(before.text_width, after.text_width);
    assert_eq!(before.wrap_rows, after.wrap_rows);
    assert_eq!(before.bounds, after.bounds);
}

#[gpui::test]
fn scrolling_to_the_end_leaves_the_buffer_alone(cx: &mut TestAppContext) {
    let text = two_hundred_lines();
    let (view, _handle, mut visual, _host) =
        open(cx, &text, ReviewView::default(), EditorSettings::default());
    let dirty_before = visual.update(|_window, cx| view.read(cx).is_dirty());
    let lines_before = visual.update(|_window, cx| view.read(cx).display_row_count());
    scroll_to_the_end(&view, &mut visual);
    frame(&view, &mut visual);
    assert_eq!(visual.update(|_window, cx| view.read(cx).text()), text);
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).display_row_count()),
        lines_before,
        "no line was added for the margin"
    );
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).is_dirty()),
        dirty_before
    );
    assert!(!dirty_before, "an untouched buffer is not modified");
}

// -- the ways to scroll --------------------------------------------------------------

#[gpui::test]
fn the_wheel_scrolls_down_to_the_margin_and_no_further(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open(
        cx,
        &two_hundred_lines(),
        ReviewView::default(),
        EditorSettings::default(),
    );
    frame(&view, &mut visual);
    let max = max_scroll_top(&view, &mut visual);
    for _ in 0..3 {
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(gpui::px(200.), gpui::px(200.)),
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-100_000.))),
            modifiers: gpui::Modifiers::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        visual.run_until_parked();
    }
    assert!(close(scroll_top(&view, &mut visual), max));
    let (line_height, ..) = geometry(&view, &mut visual);
    // More than the old maximum (no margin): the extra rows are reachable.
    let (_, _, bounds) = geometry(&view, &mut visual);
    let old_max = 200. * line_height - f32::from(bounds.size.height);
    assert!(max > old_max + line_height - 0.5, "{max} vs {old_max}");
    // And the other way back to the top.
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(gpui::px(200.), gpui::px(200.)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(100_000.))),
        modifiers: gpui::Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    visual.run_until_parked();
    assert_eq!(scroll_top(&view, &mut visual), 0.);
}

#[gpui::test]
fn set_scroll_row_is_clamped_to_the_new_maximum(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open(
        cx,
        &two_hundred_lines(),
        ReviewView::default(),
        EditorSettings::default(),
    );
    frame(&view, &mut visual);
    let (line_height, ..) = geometry(&view, &mut visual);
    let max = max_scroll_top(&view, &mut visual);

    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_scroll_row(10_000., cx)));
    visual.run_until_parked();
    let row = scroll_row(&view, &mut visual);
    assert!(close(row * line_height, max), "{row} rows vs {max} px");

    // A row between the old and the new maximum is honoured as asked.
    let (_, _, bounds) = geometry(&view, &mut visual);
    let old_max_row = (200. * line_height - f32::from(bounds.size.height)) / line_height;
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.set_scroll_row(old_max_row.ceil() + 2., cx)
        })
    });
    visual.run_until_parked();
    assert!(close(
        scroll_row(&view, &mut visual),
        old_max_row.ceil() + 2.
    ));
}

#[gpui::test]
fn a_file_that_fits_on_the_screen_can_still_scroll(cx: &mut TestAppContext) {
    // Three quarters of a screen of rows: more than the half the margin is
    // made of, so the file scrolls until its last line is at mid-screen.
    let probe = two_hundred_lines();
    let (view, _handle, mut visual, _host) =
        open(cx, &probe, ReviewView::default(), EditorSettings::default());
    frame(&view, &mut visual);
    let (line_height, _, bounds) = geometry(&view, &mut visual);
    let viewport = f32::from(bounds.size.height);
    let rows = ((viewport / line_height) * 0.75).floor() as usize;
    let short: String = (0..rows)
        .map(|row| format!("linea {row}"))
        .collect::<Vec<_>>()
        .join("\n");
    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_text(&short, 0, cx)));
    visual.run_until_parked();
    let painted = frame(&view, &mut visual);
    assert_eq!(painted.visual_rows as usize, rows);
    assert!(rows as f32 * line_height < viewport, "the file fits");

    let expected = rows as f32 * line_height + expected_margin(viewport, line_height) - viewport;
    assert!(expected > 0., "{expected}");
    assert!(close(max_scroll_top(&view, &mut visual), expected));
    scroll_to_the_end(&view, &mut visual);
    let at_end = frame(&view, &mut visual);
    let last_bottom = rows as f32 * line_height - at_end.scroll_top;
    assert!(close(
        last_bottom,
        viewport - expected_margin(viewport, line_height)
    ));
}

#[gpui::test]
fn a_click_in_the_empty_margin_puts_the_cursor_at_the_end_of_the_last_line(
    cx: &mut TestAppContext,
) {
    let (view, _handle, mut visual, _host) = open(
        cx,
        &two_hundred_lines(),
        ReviewView::default(),
        EditorSettings::default(),
    );
    scroll_to_the_end(&view, &mut visual);
    set_cursor(&view, &mut visual, DisplayPoint::new(10, 0));
    let painted = frame(&view, &mut visual);
    let (line_height, text_x, bounds) = geometry(&view, &mut visual);
    let last_row_bottom = painted.visual_rows as f32 * line_height - painted.scroll_top;
    let y = f32::from(bounds.top()) + last_row_bottom + line_height * 2.;
    assert!(
        y < f32::from(bounds.bottom()) - BAR_MARGIN - BAR_HEIGHT,
        "the click lands in the empty space under the text"
    );
    // Right of the end of the line: the end of the last line, as a click
    // under the text did before the margin existed.
    click(&mut visual, text_x + 300., y);
    let cursor = visual.update(|_window, cx| view.read(cx).cursor_point());
    assert_eq!((cursor.row, cursor.column), (199, "linea 199".len() as u32));

    // Under the text and at its left edge: still the last line (never a row
    // that does not exist), at the clicked column.
    set_cursor(&view, &mut visual, DisplayPoint::new(10, 0));
    click(&mut visual, text_x + 1., y);
    let cursor = visual.update(|_window, cx| view.read(cx).cursor_point());
    assert_eq!((cursor.row, cursor.column), (199, 0));
}

#[gpui::test]
fn soft_wrap_rows_count_towards_the_margin(cx: &mut TestAppContext) {
    let long: String = std::iter::repeat_n("palabra ", 120).collect();
    let text: String = (0..60)
        .map(|row| format!("{row} {long}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (view, _handle, mut visual, _host) =
        open(cx, &text, ReviewView::default(), EditorSettings::default());
    let painted = frame(&view, &mut visual);
    assert!(painted.wrap_rows > 60, "the long lines wrap");
    assert_eq!(painted.visual_rows, painted.wrap_rows);
    let (line_height, _, bounds) = geometry(&view, &mut visual);
    let viewport = f32::from(bounds.size.height);
    let expected = painted.visual_rows as f32 * line_height
        + expected_margin(viewport, line_height)
        - viewport;
    assert!(close(max_scroll_top(&view, &mut visual), expected));
    scroll_to_the_end(&view, &mut visual);
    assert!(close(scroll_top(&view, &mut visual), expected));
}

#[gpui::test]
fn an_open_comment_box_rows_count_towards_the_margin(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open(
        cx,
        &two_hundred_lines(),
        ReviewView::default(),
        EditorSettings::default(),
    );
    let plain = frame(&view, &mut visual);
    assert_eq!(plain.visual_rows, plain.wrap_rows);
    set_comments(
        &view,
        &host,
        &mut visual,
        vec![comment(1, 0..1, "revisá esto")],
    );
    let painted = frame(&view, &mut visual);
    assert!(
        painted.visual_rows > painted.wrap_rows,
        "the comment block adds rows"
    );
    let (line_height, _, bounds) = geometry(&view, &mut visual);
    let viewport = f32::from(bounds.size.height);
    let expected = painted.visual_rows as f32 * line_height
        + expected_margin(viewport, line_height)
        - viewport;
    assert!(close(max_scroll_top(&view, &mut visual), expected));
    scroll_to_the_end(&view, &mut visual);
    assert!(close(scroll_top(&view, &mut visual), expected));
}

// -- §7.7: Ctrl+↑ / Ctrl+↓ ----------------------------------------------------------

#[gpui::test]
fn ctrl_down_scrolls_one_row_each_time_and_leaves_the_cursor_alone(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open(
        cx,
        &two_hundred_lines(),
        ReviewView::default(),
        EditorSettings::default(),
    );
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 3));
    frame(&view, &mut visual);
    assert_eq!(scroll_row(&view, &mut visual), 0.);
    let cursor_before = visual.update(|_window, cx| view.read(cx).cursor_point());

    visual.simulate_keystrokes("ctrl-down ctrl-down ctrl-down");
    assert!(close(scroll_row(&view, &mut visual), 3.));
    visual.update(|_window, cx| {
        let view = view.read(cx);
        assert_eq!(view.cursor_point(), cursor_before);
        assert!(!view.has_selection());
    });

    // The cursor row (1) is now above the viewport and stays there: the view
    // does not run back to it.
    frame(&view, &mut visual);
    assert!(close(scroll_row(&view, &mut visual), 3.));

    // Writing or moving brings it back, as ever.
    visual.simulate_keystrokes("right");
    assert!(scroll_row(&view, &mut visual) <= 1.);
}

#[gpui::test]
fn ctrl_down_keeps_a_selection(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open(
        cx,
        &two_hundred_lines(),
        ReviewView::default(),
        EditorSettings::default(),
    );
    crate::comment_box_tests::select(
        &view,
        &mut visual,
        DisplayPoint::new(0, 0),
        DisplayPoint::new(2, 4),
    );
    let before = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.cursor_point(), view.selection_anchor)
    });
    visual.simulate_keystrokes("ctrl-down ctrl-down");
    assert!(close(scroll_row(&view, &mut visual), 2.));
    let after = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.cursor_point(), view.selection_anchor)
    });
    assert_eq!(before, after);
    assert!(visual.update(|_window, cx| view.read(cx).has_selection()));
}

#[gpui::test]
fn ctrl_up_at_the_top_does_nothing(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open(
        cx,
        &two_hundred_lines(),
        ReviewView::default(),
        EditorSettings::default(),
    );
    set_cursor(&view, &mut visual, DisplayPoint::new(4, 2));
    frame(&view, &mut visual);
    visual.simulate_keystrokes("ctrl-up ctrl-up");
    assert_eq!(scroll_top(&view, &mut visual), 0.);
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).cursor_point()),
        cincel_text::Point::new(4, 2)
    );
}

#[gpui::test]
fn ctrl_up_walks_back_up_one_row_at_a_time(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open(
        cx,
        &two_hundred_lines(),
        ReviewView::default(),
        EditorSettings::default(),
    );
    frame(&view, &mut visual);
    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_scroll_row(10., cx)));
    visual.run_until_parked();
    visual.simulate_keystrokes("ctrl-up ctrl-up");
    assert!(close(scroll_row(&view, &mut visual), 8.));
}

#[gpui::test]
fn ctrl_down_stops_at_the_maximum_with_margin(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open(
        cx,
        &two_hundred_lines(),
        ReviewView::default(),
        EditorSettings::default(),
    );
    frame(&view, &mut visual);
    let (line_height, ..) = geometry(&view, &mut visual);
    let max = max_scroll_top(&view, &mut visual);
    let max_row = max / line_height;
    // Two rows short of the end, then more presses than rows are left.
    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_scroll_row(max_row - 2., cx)));
    visual.run_until_parked();
    visual.simulate_keystrokes("ctrl-down ctrl-down ctrl-down ctrl-down ctrl-down");
    assert!(close(scroll_top(&view, &mut visual), max));
    assert!(
        max > 200. * line_height - f32::from(geometry(&view, &mut visual).2.size.height),
        "the stop is past the old maximum: the margin is reachable"
    );
}

#[gpui::test]
fn the_scroll_keys_work_in_a_text_field_too(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) =
        open(cx, &two_hundred_lines(), ReviewView::default(), minimal());
    frame(&view, &mut visual);
    visual.simulate_keystrokes("ctrl-down ctrl-down");
    assert!(close(scroll_row(&view, &mut visual), 2.));
    visual.simulate_keystrokes("ctrl-up");
    assert!(close(scroll_row(&view, &mut visual), 1.));
}
