//! Up on the first visual row and Down on the last one go to the edge of that
//! row (`docs/specs/13-etapa7-ronda5.md` §3.5, R9/R10): column 0 or the end of
//! the row, Shift extends the selection, and with several rows the goal
//! column is still kept. The chat composer is an `EditorView` too, so the same
//! tests cover it (a minimal-chrome view is opened as well).

use std::sync::Arc;

use cincel_syntax::LanguageRegistry;
use cincel_text::{Buffer, Point};
use gpui::{AnyWindowHandle, Entity, TestAppContext, VisualTestContext};

use crate::actions::bind_default_keys;
use crate::display_map::DisplayPoint;
use crate::settings::{EditorChrome, EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::EditorView;

fn open(
    cx: &mut TestAppContext,
    text: &str,
    settings: EditorSettings,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    cx.update(bind_default_keys);
    let buffer = shared(Buffer::new(text));
    let registry = Arc::new(LanguageRegistry::new());
    let window = cx.add_window(move |window, cx| {
        EditorView::new(
            buffer,
            None,
            registry,
            settings,
            EditorTheme::default(),
            window,
            cx,
        )
    });
    let view = window.update(cx, |_, _, cx| cx.entity()).unwrap();
    let handle: AnyWindowHandle = window.into();
    let visual = VisualTestContext::from_window(handle, cx);
    window
        .update(cx, |view, window, cx| window.focus(&view.focus_handle, cx))
        .unwrap();
    visual.run_until_parked();
    (view, handle, visual)
}

fn minimal() -> EditorSettings {
    EditorSettings {
        chrome: EditorChrome::Minimal,
        ..EditorSettings::default()
    }
}

fn place(view: &Entity<EditorView>, cx: &mut VisualTestContext, at: (u32, u32)) {
    cx.update(|_window, cx| {
        view.update(cx, |view, cx| {
            let point = view.clip_point(DisplayPoint::new(at.0, at.1));
            view.selection_anchor = point;
            view.cursor = point;
            view.goal_column = None;
            cx.notify();
        })
    });
    cx.run_until_parked();
}

fn keys(handle: AnyWindowHandle, cx: &mut TestAppContext, visual: &mut VisualTestContext, k: &str) {
    cx.simulate_keystrokes(handle, k);
    visual.run_until_parked();
}

fn selection(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> (Point, Point) {
    cx.update(|_window, cx| view.read(cx).buffer_selection())
}

fn p(row: u32, column: u32) -> Point {
    Point::new(row, column)
}

fn goal(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Option<u32> {
    cx.update(|_window, cx| view.read(cx).goal_column)
}

#[gpui::test]
fn single_row_up_goes_to_column_zero_and_down_to_the_end(cx: &mut TestAppContext) {
    for settings in [EditorSettings::default(), minimal()] {
        let (view, handle, mut visual) = open(cx, "hello world", settings);
        place(&view, &mut visual, (0, 5));
        keys(handle, cx, &mut visual, "up");
        assert_eq!(selection(&view, &mut visual), (p(0, 0), p(0, 0)));
        assert_eq!(goal(&view, &mut visual), Some(0));
        place(&view, &mut visual, (0, 5));
        keys(handle, cx, &mut visual, "down");
        assert_eq!(selection(&view, &mut visual), (p(0, 11), p(0, 11)));
        assert_eq!(goal(&view, &mut visual), Some(11));
    }
}

#[gpui::test]
fn shift_up_and_shift_down_extend_to_the_edge(cx: &mut TestAppContext) {
    for settings in [EditorSettings::default(), minimal()] {
        let (view, handle, mut visual) = open(cx, "hello world", settings);
        place(&view, &mut visual, (0, 5));
        keys(handle, cx, &mut visual, "shift-up");
        assert_eq!(selection(&view, &mut visual), (p(0, 5), p(0, 0)));
        place(&view, &mut visual, (0, 5));
        keys(handle, cx, &mut visual, "shift-down");
        assert_eq!(selection(&view, &mut visual), (p(0, 5), p(0, 11)));
    }
}

#[gpui::test]
fn first_row_up_and_last_row_down_of_a_multi_row_buffer(cx: &mut TestAppContext) {
    for settings in [EditorSettings::default(), minimal()] {
        let (view, handle, mut visual) = open(cx, "first line\nmiddle\nlast one here", settings);
        place(&view, &mut visual, (0, 4));
        keys(handle, cx, &mut visual, "up");
        assert_eq!(selection(&view, &mut visual), (p(0, 0), p(0, 0)));
        place(&view, &mut visual, (2, 4));
        keys(handle, cx, &mut visual, "down");
        assert_eq!(selection(&view, &mut visual), (p(2, 13), p(2, 13)));
        place(&view, &mut visual, (0, 4));
        keys(handle, cx, &mut visual, "shift-up");
        assert_eq!(selection(&view, &mut visual), (p(0, 4), p(0, 0)));
        place(&view, &mut visual, (2, 4));
        keys(handle, cx, &mut visual, "shift-down");
        assert_eq!(selection(&view, &mut visual), (p(2, 4), p(2, 13)));
    }
}

#[gpui::test]
fn middle_rows_keep_the_goal_column(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "abcdefgh\nab\nabcdefgh", EditorSettings::default());
    place(&view, &mut visual, (0, 6));
    keys(handle, cx, &mut visual, "down");
    assert_eq!(selection(&view, &mut visual), (p(1, 2), p(1, 2)));
    assert_eq!(goal(&view, &mut visual), Some(6));
    keys(handle, cx, &mut visual, "down");
    assert_eq!(selection(&view, &mut visual), (p(2, 6), p(2, 6)));
    keys(handle, cx, &mut visual, "up");
    keys(handle, cx, &mut visual, "up");
    assert_eq!(selection(&view, &mut visual), (p(0, 6), p(0, 6)));
}

#[gpui::test]
fn a_wrapped_line_uses_the_visual_row(cx: &mut TestAppContext) {
    let line = "word ".repeat(600);
    let (view, handle, mut visual) = open(cx, &line, EditorSettings::default());
    let rows = cx.update(|view_cx| view.read(view_cx).wrap_row_count());
    assert!(rows > 2, "the line must wrap, got {rows} rows");
    let end = line.len() as u32;

    // Down from the first wrap row is a normal move, not the end of the line.
    place(&view, &mut visual, (0, 3));
    keys(handle, cx, &mut visual, "down");
    let (_, cursor) = selection(&view, &mut visual);
    assert!(cursor.column > 3 && cursor.column < end, "{cursor:?}");

    // Up on the first wrap row goes to column 0 of the line.
    place(&view, &mut visual, (0, 3));
    keys(handle, cx, &mut visual, "up");
    assert_eq!(selection(&view, &mut visual), (p(0, 0), p(0, 0)));

    // Down on the last wrap row goes to the end of the whole line.
    place(&view, &mut visual, (0, end - 2));
    keys(handle, cx, &mut visual, "down");
    assert_eq!(selection(&view, &mut visual), (p(0, end), p(0, end)));
}
