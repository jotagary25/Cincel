//! Interaction tests over the GPUI test harness: typing, selection across
//! phantom rows, copy, and accept/reject of a hunk.

use gpui::{AnyWindowHandle, Entity, TestAppContext, VisualTestContext};

use crate::display_map::{DisplayPoint, PhantomHunk};
use crate::view::{EditorView, bind_default_keys};

fn hunk(insert_before: u32, deleted: &[&str], added: std::ops::Range<u32>) -> PhantomHunk {
    PhantomHunk {
        insert_before_buffer_row: insert_before,
        deleted_text: deleted.iter().map(|line| line.to_string()).collect(),
        added_rows: added,
    }
}

/// Opens a window with an editor and focuses it.
fn open(
    cx: &mut TestAppContext,
    text: &str,
    hunks: Vec<PhantomHunk>,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    cx.update(bind_default_keys);
    let text = text.to_string();
    let window = cx.add_window(move |_window, cx| EditorView::new(&text, hunks, cx));
    let view = window.update(cx, |_, _, cx| cx.entity()).unwrap();
    let handle: AnyWindowHandle = window.into();
    let visual = VisualTestContext::from_window(handle, cx);
    window
        .update(cx, |view, window, cx| {
            window.focus(&view.focus_handle, cx);
        })
        .unwrap();
    visual.run_until_parked();
    (view, handle, visual)
}

fn set_cursor(view: &Entity<EditorView>, cx: &mut VisualTestContext, point: DisplayPoint) {
    cx.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.cursor = view.clip_point(point);
            view.selection_anchor = view.cursor;
            cx.notify();
        })
    });
}

#[gpui::test]
fn typing_on_a_phantom_row_edits_the_next_real_row(cx: &mut TestAppContext) {
    // Row 1 of the buffer is the added row; "viejo" is the deleted row.
    let (view, handle, mut visual) = open(cx, "a\nb\nc\n", vec![hunk(1, &["viejo"], 1..2)]);
    // Display rows: 0 = "a", 1 = phantom "viejo", 2 = "b", 3 = "c".
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 3));
    visual.run_until_parked();

    cx.simulate_input(handle, "X");
    visual.run_until_parked();

    let (text, cursor) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.text(), view.cursor)
    });
    // The edit landed at the start of the next real row, not in the phantom row.
    assert_eq!(text, "a\nXb\nc\n");
    assert_eq!(cursor, DisplayPoint::new(2, 1));
}

#[gpui::test]
fn typing_on_a_real_row_inserts_there(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 1));
    cx.simulate_input(handle, "zá");
    visual.run_until_parked();

    let text = visual.update(|_window, cx| view.read(cx).text());
    assert_eq!(text, "a\nbzá\n");
}

#[gpui::test]
fn selection_spans_phantom_and_real_rows(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\nc\n", vec![hunk(1, &["viejo"], 1..2)]);
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    // shift-down + shift-end selects the phantom row and the row below it.
    cx.simulate_keystrokes(handle, "shift-down shift-end");
    visual.run_until_parked();

    let selected = visual.update(|_window, cx| view.read(cx).selected_text());
    assert_eq!(selected, "viejo\nb");
}

#[gpui::test]
fn copy_includes_the_phantom_text(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\nc\n", vec![hunk(1, &["viejo"], 1..2)]);
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    cx.simulate_keystrokes(handle, "shift-down shift-end ctrl-c");
    visual.run_until_parked();

    let clipboard = visual.update(|_window, cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    });
    assert_eq!(clipboard, "viejo\nb");
    // The buffer was not modified by the copy.
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).text()),
        "a\nb\nc\n"
    );
}

#[gpui::test]
fn backspace_and_enter_edit_the_buffer(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "hola\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 4));
    cx.simulate_keystrokes(handle, "backspace enter");
    visual.run_until_parked();

    let text = visual.update(|_window, cx| view.read(cx).text());
    assert_eq!(text, "hol\n\n");
}

#[gpui::test]
fn accepting_a_hunk_removes_its_phantom_rows(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "a\nb\nc\n", vec![hunk(1, &["viejo"], 1..2)]);
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).display_row_count()),
        5
    );

    visual.update(|_window, cx| view.update(cx, |view, cx| view.accept_hunk(0, cx)));
    visual.run_until_parked();

    let (rows, hunks, text) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.display_row_count(), view.hunks().len(), view.text())
    });
    assert_eq!(rows, 4);
    assert_eq!(hunks, 0);
    assert_eq!(text, "a\nb\nc\n");
}

#[gpui::test]
fn rejecting_a_hunk_restores_the_deleted_text(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "a\nb\nc\n", vec![hunk(1, &["viejo"], 1..2)]);

    visual.update(|_window, cx| view.update(cx, |view, cx| view.reject_hunk(0, cx)));
    visual.run_until_parked();

    let (rows, hunks, text) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.display_row_count(), view.hunks().len(), view.text())
    });
    assert_eq!(text, "a\nviejo\nc\n");
    assert_eq!(hunks, 0);
    assert_eq!(rows, 4);
}

#[gpui::test]
fn clicking_positions_the_cursor_on_the_clicked_row(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "a\nb\nc\nd\n", vec![hunk(1, &["viejo"], 1..2)]);
    let line_height = visual.update(|_window, cx| f32::from(view.read(cx).style.line_height));
    // Click in the middle of display row 3 ("c", below the phantom row).
    visual.simulate_click(
        gpui::point(gpui::px(200.), gpui::px(line_height * 3.5)),
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();

    let cursor = visual.update(|_window, cx| view.read(cx).cursor);
    assert_eq!(cursor.row, 3);
}

#[gpui::test]
fn clicking_the_pill_accepts_and_rejects(cx: &mut TestAppContext) {
    // Two hunks: clicking "Aceptar" on the first and "Rechazar" on the second.
    let (view, _handle, mut visual) = open(
        cx,
        "a\nb\nc\nd\n",
        vec![hunk(1, &["viejo"], 1..2), hunk(3, &["otro"], 3..4)],
    );
    let (line_height, right) = visual.update(|_window, cx| {
        let view = view.read(cx);
        let layout = view.layout.as_ref().expect("the element already painted");
        (
            f32::from(view.style.line_height),
            f32::from(layout.bounds.right()),
        )
    });
    // The pill sits on the first display row of each hunk: 1 and 4.
    let accept_x = right - 12. - 190. + 40.;
    let reject_x = right - 12. - 190. + 140.;

    visual.simulate_click(
        gpui::point(gpui::px(accept_x), gpui::px(line_height * 1.5)),
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    let (hunks, text) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.hunks().len(), view.text())
    });
    assert_eq!(hunks, 1, "accepting removes the hunk");
    assert_eq!(text, "a\nb\nc\nd\n", "accepting does not touch the buffer");

    // The remaining hunk moved up one display row (the phantom row is gone).
    visual.simulate_click(
        gpui::point(gpui::px(reject_x), gpui::px(line_height * 3.5)),
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    let (hunks, text) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.hunks().len(), view.text())
    });
    assert_eq!(hunks, 0);
    assert_eq!(
        text, "a\nb\nc\notro\n",
        "rejecting restores the deleted text"
    );
}

#[gpui::test]
fn the_wheel_scrolls_the_editor(cx: &mut TestAppContext) {
    let text: String = (0..500).map(|row| format!("linea {row}\n")).collect();
    let (view, _handle, mut visual) = open(cx, &text, vec![]);
    assert_eq!(visual.update(|_window, cx| view.read(cx).scroll_top), 0.);

    visual.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(gpui::px(200.), gpui::px(200.)),
        delta: gpui::ScrollDelta::Lines(gpui::point(0., -3.)),
        modifiers: gpui::Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    visual.run_until_parked();

    let scroll_top = visual.update(|_window, cx| view.read(cx).scroll_top);
    assert!(
        scroll_top > 0.,
        "the wheel must scroll down, got {scroll_top}"
    );
}

#[gpui::test]
fn arrow_keys_walk_into_phantom_rows(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\n", vec![hunk(1, &["viejo"], 1..2)]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    cx.simulate_keystrokes(handle, "down end");
    visual.run_until_parked();

    let cursor = visual.update(|_window, cx| view.read(cx).cursor);
    // Display row 1 is the phantom row and the cursor can sit at its end.
    assert_eq!(cursor, DisplayPoint::new(1, 5));
}
