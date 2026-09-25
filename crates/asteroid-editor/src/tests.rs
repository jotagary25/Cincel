//! Interaction tests over the GPUI test harness: typing, selection across
//! phantom rows, undo/redo, search, soft wrap, indentation and the mouse.

use std::sync::Arc;

use asteroid_syntax::LanguageRegistry;
use asteroid_text::Buffer;
use gpui::{AnyWindowHandle, Entity, TestAppContext, VisualTestContext};

use crate::actions::bind_default_keys;
use crate::display_map::{DisplayPoint, PhantomHunk};
use crate::settings::{EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::EditorView;

fn hunk(insert_before: u32, deleted: &[&str], added: std::ops::Range<u32>) -> PhantomHunk {
    PhantomHunk {
        insert_before_buffer_row: insert_before,
        deleted_text: deleted.iter().map(|line| line.to_string()).collect(),
        added_rows: added,
    }
}

/// Opens a window with an editor and focuses it.
fn open_with(
    cx: &mut TestAppContext,
    text: &str,
    hunks: Vec<PhantomHunk>,
    settings: EditorSettings,
    language: Option<&str>,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    cx.update(bind_default_keys);
    let buffer = shared(Buffer::new(text));
    let registry = Arc::new(LanguageRegistry::new());
    let language = language.and_then(|name| registry.language(name));
    let window = cx.add_window(move |window, cx| {
        EditorView::new(
            buffer,
            language,
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
        .update(cx, |view, window, cx| {
            window.focus(&view.focus_handle, cx);
        })
        .unwrap();
    if !hunks.is_empty() {
        window
            .update(cx, |view, _window, cx| view.set_hunks(hunks, cx))
            .unwrap();
    }
    visual.run_until_parked();
    (view, handle, visual)
}

fn open(
    cx: &mut TestAppContext,
    text: &str,
    hunks: Vec<PhantomHunk>,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    open_with(cx, text, hunks, EditorSettings::default(), None)
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

fn text_of(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
    cx.update(|_window, cx| view.read(cx).text())
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

    assert_eq!(text_of(&view, &mut visual), "a\nbzá\n");
}

#[gpui::test]
fn selection_spans_phantom_and_real_rows(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\nc\n", vec![hunk(1, &["viejo"], 1..2)]);
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
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
    assert_eq!(text_of(&view, &mut visual), "a\nb\nc\n");
}

#[gpui::test]
fn backspace_and_enter_edit_the_buffer(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "hola\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 4));
    cx.simulate_keystrokes(handle, "backspace enter");
    visual.run_until_parked();

    assert_eq!(text_of(&view, &mut visual), "hol\n\n");
}

#[gpui::test]
fn insert_newline_inherits_the_indentation(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "fn main() {\n    let x = 1;\n}\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 14));
    cx.simulate_keystrokes(handle, "enter");
    visual.run_until_parked();

    assert_eq!(
        text_of(&view, &mut visual),
        "fn main() {\n    let x = 1;\n    \n}\n"
    );
}

#[gpui::test]
fn insert_newline_deepens_after_an_open_brace(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "fn main() {\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 11));
    cx.simulate_keystrokes(handle, "enter");
    visual.run_until_parked();

    assert_eq!(text_of(&view, &mut visual), "fn main() {\n    \n");
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
fn the_review_context_is_only_active_inside_a_hunk(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "a\nb\nc\n", vec![hunk(1, &["viejo"], 1..2)]);
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    let inside = visual.update(|_window, cx| view.read(cx).key_context());
    assert!(inside.contains("review_hunk_under_cursor"), "{inside}");

    set_cursor(&view, &mut visual, DisplayPoint::new(4, 0));
    let outside = visual.update(|_window, cx| view.read(cx).key_context());
    assert!(!outside.contains("review_hunk_under_cursor"), "{outside}");
    assert!(outside.contains("Editor"));
}

#[gpui::test]
fn ctrl_enter_accepts_inside_a_hunk_and_breaks_the_line_outside(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\nc\n", vec![hunk(1, &["viejo"], 1..2)]);
    // Outside the hunk: Ctrl+Enter is a plain newline (02-visual §8).
    set_cursor(&view, &mut visual, DisplayPoint::new(4, 1));
    cx.simulate_keystrokes(handle, "ctrl-enter");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "a\nb\nc\n\n");

    // Inside the hunk: it accepts.
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    visual.run_until_parked();
    cx.simulate_keystrokes(handle, "ctrl-enter");
    visual.run_until_parked();
    let hunks = visual.update(|_window, cx| view.read(cx).hunks().len());
    assert_eq!(hunks, 0, "Ctrl+Enter inside a hunk accepts it");
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
    // The pill sits on the first display row of each hunk: 1 and 4. It is
    // inset by the scrollbar (8 px) plus its own margin.
    let accept_x = right - 8. - 12. - 190. + 40.;
    let reject_x = right - 8. - 12. - 190. + 140.;

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

#[gpui::test]
fn undo_and_redo_group_a_burst_of_typing(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    cx.simulate_input(handle, "hola");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "hola");

    cx.simulate_keystrokes(handle, "ctrl-z");
    visual.run_until_parked();
    assert_eq!(
        text_of(&view, &mut visual),
        "",
        "typing within the group interval is one undo unit"
    );

    cx.simulate_keystrokes(handle, "ctrl-shift-z");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "hola");
}

#[gpui::test]
fn undo_puts_the_cursor_back_where_the_text_came_back(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "uno\ndos\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 3));
    cx.simulate_input(handle, "X");
    visual.run_until_parked();
    cx.simulate_keystrokes(handle, "ctrl-z");
    visual.run_until_parked();

    let (text, cursor) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.text(), view.cursor)
    });
    assert_eq!(text, "uno\ndos\n");
    assert_eq!(cursor.row, 1);
}

#[gpui::test]
fn the_search_bar_finds_and_walks_the_matches(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "uno dos uno tres uno\n", vec![]);
    cx.simulate_keystrokes(handle, "ctrl-f");
    visual.run_until_parked();
    assert!(visual.update(|_window, cx| view.read(cx).search_open));

    cx.simulate_input(handle, "uno");
    visual.run_until_parked();
    let (count, counter, context) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (
            view.search.matches().len(),
            view.search.counter(),
            view.key_context(),
        )
    });
    assert_eq!(count, 3);
    assert_eq!(counter.as_deref(), Some("1/3"));
    assert!(context.contains("searching"), "{context}");

    // Enter walks forward, Shift+Enter backwards, both wrapping around.
    cx.simulate_keystrokes(handle, "enter");
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).search.counter()),
        Some("2/3".to_string())
    );
    cx.simulate_keystrokes(handle, "shift-enter shift-enter");
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).search.counter()),
        Some("3/3".to_string())
    );

    // The selection follows the current match.
    let selected = visual.update(|_window, cx| view.read(cx).selected_text());
    assert_eq!(selected, "uno");

    cx.simulate_keystrokes(handle, "escape");
    visual.run_until_parked();
    let (open, matches) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.search_open, view.search.matches().len())
    });
    assert!(!open, "Esc closes the bar");
    assert_eq!(matches, 0);
}

#[gpui::test]
fn the_search_is_case_insensitive_by_default(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "Uno uno UNO\n", vec![]);
    cx.simulate_keystrokes(handle, "ctrl-f");
    cx.simulate_input(handle, "uno");
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).search.matches().len()),
        3
    );

    cx.simulate_keystrokes(handle, "alt-c");
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).search.matches().len()),
        1,
        "Alt+C makes the search case sensitive"
    );
}

#[gpui::test]
fn the_search_bar_takes_the_keyboard_from_the_buffer(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "hola\n", vec![]);
    cx.simulate_keystrokes(handle, "ctrl-f");
    cx.simulate_input(handle, "ho");
    visual.run_until_parked();
    assert_eq!(
        text_of(&view, &mut visual),
        "hola\n",
        "the buffer is intact"
    );
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).search.query.clone()),
        "ho"
    );

    cx.simulate_keystrokes(handle, "backspace");
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).search.query.clone()),
        "h"
    );
    assert_eq!(text_of(&view, &mut visual), "hola\n");
}

#[gpui::test]
fn soft_wrap_splits_a_long_row_into_several_wrap_rows(cx: &mut TestAppContext) {
    let long: String = std::iter::repeat_n("palabra ", 120).collect();
    let settings = EditorSettings {
        soft_wrap: true,
        ..Default::default()
    };
    let (view, _handle, mut visual) = open_with(
        cx,
        &format!("corta\n{long}\notra\n"),
        vec![],
        settings,
        None,
    );
    visual.run_until_parked();

    let (display_rows, wrap_rows, segments) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (
            view.display_row_count(),
            view.wrap_row_count(),
            view.wrap.segment_count(1),
        )
    });
    assert!(segments > 1, "the long row must wrap, got {segments}");
    assert_eq!(wrap_rows, display_rows - 1 + segments);

    // The row after the wrapped one starts where its segments end.
    let first = visual.update(|_window, cx| view.read(cx).wrap.first_wrap_row(2));
    assert_eq!(first, 1 + segments);
}

#[gpui::test]
fn moving_down_with_soft_wrap_stays_inside_the_row(cx: &mut TestAppContext) {
    let long: String = std::iter::repeat_n("palabra ", 120).collect();
    let settings = EditorSettings {
        soft_wrap: true,
        ..Default::default()
    };
    let (view, handle, mut visual) =
        open_with(cx, &format!("{long}\nfin\n"), vec![], settings, None);
    visual.run_until_parked();
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    visual.run_until_parked();

    cx.simulate_keystrokes(handle, "down");
    visual.run_until_parked();
    let cursor = visual.update(|_window, cx| view.read(cx).cursor);
    assert_eq!(cursor.row, 0, "still on the same logical row");
    assert!(cursor.column > 0, "but further into it");
}

#[gpui::test]
fn typing_inside_a_wrapped_row_remeasures_only_that_row(cx: &mut TestAppContext) {
    let long: String = std::iter::repeat_n("palabra ", 120).collect();
    let settings = EditorSettings {
        soft_wrap: true,
        ..Default::default()
    };
    let (view, handle, mut visual) =
        open_with(cx, &format!("{long}\ncorta\n"), vec![], settings, None);
    visual.run_until_parked();
    let before = visual.update(|_window, cx| view.read(cx).wrap_row_count());

    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    cx.simulate_input(handle, "x");
    // No layout in between: the edit itself must have brought the wrap map up
    // to date through the single-row fast path.
    let (epoch, state) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.wrap_epoch, view.wrap_state())
    });
    assert_eq!(epoch, state, "the single-row fast path must have run");

    visual.run_until_parked();
    let after = visual.update(|_window, cx| view.read(cx).wrap_row_count());
    assert!(
        after >= before,
        "one more character cannot remove wrap rows: {before} -> {after}"
    );
    // And the mapping is still a partition of the wrap rows.
    visual.update(|_window, cx| {
        let view = view.read(cx);
        let mut expected = 0;
        for row in 0..view.display_row_count() {
            assert_eq!(view.wrap.first_wrap_row(row), expected, "row {row}");
            expected += view.wrap.segment_count(row);
        }
        assert_eq!(expected, view.wrap_row_count());
    });
}

#[gpui::test]
fn toggling_soft_wrap_changes_the_row_count(cx: &mut TestAppContext) {
    let long: String = std::iter::repeat_n("palabra ", 120).collect();
    let (view, handle, mut visual) = open(cx, &format!("{long}\n"), vec![]);
    visual.run_until_parked();
    let before = visual.update(|_window, cx| view.read(cx).wrap_row_count());

    cx.simulate_keystrokes(handle, "alt-z");
    visual.run_until_parked();
    let after = visual.update(|_window, cx| view.read(cx).wrap_row_count());
    assert!(
        after > before,
        "soft wrap must add rows: {before} -> {after}"
    );
    assert!(visual.update(|_window, cx| view.read(cx).soft_wrap()));
}

#[gpui::test]
fn the_goal_column_survives_a_short_row(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "0123456789\nab\n0123456789\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 8));
    visual.run_until_parked();

    cx.simulate_keystrokes(handle, "down");
    visual.run_until_parked();
    let middle = visual.update(|_window, cx| view.read(cx).cursor);
    assert_eq!(middle, DisplayPoint::new(1, 2), "clamped to the short row");

    cx.simulate_keystrokes(handle, "down");
    visual.run_until_parked();
    let bottom = visual.update(|_window, cx| view.read(cx).cursor);
    assert_eq!(bottom, DisplayPoint::new(2, 8), "the goal column came back");
}

#[gpui::test]
fn tab_and_backtab_indent_the_selected_rows(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "uno\ndos\ntres\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    cx.simulate_keystrokes(handle, "shift-down shift-down tab");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "    uno\n    dos\ntres\n");

    cx.simulate_keystrokes(handle, "shift-tab");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "uno\ndos\ntres\n");
}

#[gpui::test]
fn tab_inserts_one_level_without_a_selection(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "uno\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    cx.simulate_keystrokes(handle, "tab");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "    uno\n");
}

#[gpui::test]
fn tab_follows_the_indentation_detected_in_the_file(cx: &mut TestAppContext) {
    // A file indented with tabs makes `tab` insert a tab.
    let (view, handle, mut visual) = open(cx, "fn main() {\n\tlet x = 1;\n}\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(2, 0));
    cx.simulate_keystrokes(handle, "tab");
    visual.run_until_parked();
    assert_eq!(
        text_of(&view, &mut visual),
        "fn main() {\n\tlet x = 1;\n\t}\n"
    );
}

#[gpui::test]
fn delete_word_left_and_right(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "uno dos tres\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 7));
    cx.simulate_keystrokes(handle, "ctrl-backspace");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "uno  tres\n");

    // Forward deletion eats the whitespace run first, then the word.
    cx.simulate_keystrokes(handle, "ctrl-delete");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "uno tres\n");
    cx.simulate_keystrokes(handle, "ctrl-delete");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "uno \n");
}

#[gpui::test]
fn select_all_then_copy_takes_the_whole_document(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "uno\ndos\ntres", vec![]);
    cx.simulate_keystrokes(handle, "ctrl-a ctrl-c");
    visual.run_until_parked();

    let clipboard = visual.update(|_window, cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    });
    assert_eq!(clipboard, "uno\ndos\ntres");
    assert_eq!(text_of(&view, &mut visual), "uno\ndos\ntres");
}

#[gpui::test]
fn cut_and_paste_move_the_text(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "uno dos\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    cx.simulate_keystrokes(handle, "ctrl-shift-right ctrl-x");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), " dos\n");

    cx.simulate_keystrokes(handle, "end ctrl-v");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), " dosuno\n");
}

#[gpui::test]
fn go_to_line_moves_the_cursor(cx: &mut TestAppContext) {
    let text: String = (0..20).map(|row| format!("linea {row}\n")).collect();
    let (view, handle, mut visual) = open(cx, &text, vec![]);
    cx.simulate_keystrokes(handle, "ctrl-g");
    visual.run_until_parked();
    cx.simulate_input(handle, "12");
    visual.run_until_parked();
    cx.simulate_keystrokes(handle, "enter");
    visual.run_until_parked();

    let cursor = visual.update(|_window, cx| view.read(cx).cursor_point());
    assert_eq!(cursor.row, 11, "line 12 is row 11");
    assert_eq!(text_of(&view, &mut visual), text, "the buffer is intact");
}

#[gpui::test]
fn the_dirty_flag_follows_the_buffer(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "hola\n", vec![]);
    assert!(!visual.update(|_window, cx| view.read(cx).is_dirty()));

    cx.simulate_input(handle, "X");
    visual.run_until_parked();
    assert!(visual.update(|_window, cx| view.read(cx).is_dirty()));

    visual.update(|_window, cx| view.update(cx, |view, cx| view.mark_saved(cx)));
    assert!(!visual.update(|_window, cx| view.read(cx).is_dirty()));
}

#[gpui::test]
fn undoing_back_to_the_saved_text_cleans_the_tab(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "hola\n", vec![]);
    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    visual.update(|_window, cx| {
        let sink = events.clone();
        cx.subscribe(
            &view,
            move |_entity, event: &crate::view::EditorEvent, _cx| {
                if let crate::view::EditorEvent::DirtyChanged(dirty) = event {
                    sink.lock().push(*dirty);
                }
            },
        )
        .detach();
    });

    set_cursor(&view, &mut visual, DisplayPoint::new(0, 4));
    cx.simulate_input(handle, " mundo");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "hola mundo\n");
    assert!(visual.update(|_window, cx| view.read(cx).is_dirty()));
    assert_eq!(events.lock().clone(), vec![true]);

    // Ctrl+Z back to the original text: the version moved, the content did not.
    cx.simulate_keystrokes(handle, "ctrl-z");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "hola\n");
    assert!(
        !visual.update(|_window, cx| view.read(cx).is_dirty()),
        "undoing to the text on disk leaves the tab clean"
    );
    assert_eq!(
        events.lock().clone(),
        vec![true, false],
        "exactly one transition each way"
    );

    // Ctrl+Shift+Z: dirty again.
    cx.simulate_keystrokes(handle, "ctrl-shift-z");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "hola mundo\n");
    assert!(visual.update(|_window, cx| view.read(cx).is_dirty()));
    assert_eq!(events.lock().clone(), vec![true, false, true]);
}

#[gpui::test]
fn retyping_the_saved_text_cleans_the_tab(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "hola\n", vec![]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 4));

    // Delete the word and type it back: a different edit path, same content.
    cx.simulate_keystrokes(handle, "ctrl-backspace");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "\n");
    assert!(visual.update(|_window, cx| view.read(cx).is_dirty()));

    cx.simulate_input(handle, "hola");
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "hola\n");
    assert!(
        !visual.update(|_window, cx| view.read(cx).is_dirty()),
        "the content is the one on disk again"
    );
}

#[gpui::test]
fn save_emits_the_event(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "hola\n", vec![]);
    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    visual.update(|_window, cx| {
        let sink = events.clone();
        cx.subscribe(
            &view,
            move |_entity, event: &crate::view::EditorEvent, _cx| {
                sink.lock().push(event.clone());
            },
        )
        .detach();
    });

    cx.simulate_keystrokes(handle, "ctrl-s");
    visual.run_until_parked();
    assert!(
        events
            .lock()
            .contains(&crate::view::EditorEvent::SaveRequested),
        "ctrl-s must emit SaveRequested, got {:?}",
        events.lock()
    );
}

#[gpui::test]
fn an_edit_through_the_shared_handle_reaches_the_view(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "hola\n", vec![]);
    visual.update(|_window, cx| {
        let buffer = view.read(cx).buffer().clone();
        let mut buffer = buffer.lock();
        buffer.transact(asteroid_text::EditSource::Agent { turn_id: 1 }, |buffer| {
            buffer.insert(0, "agente: ");
        });
    });
    // The host tells the view its buffer moved (the element also picks it up on
    // the next layout).
    visual.update(|_window, cx| view.update(cx, |view, cx| view.buffer_changed(cx)));
    visual.run_until_parked();

    assert_eq!(text_of(&view, &mut visual), "agente: hola\n");
    assert!(
        visual.update(|_window, cx| view.read(cx).is_dirty()),
        "someone else's edit also makes the tab dirty"
    );
}

#[gpui::test]
fn syntax_highlighting_lands_after_the_background_parse(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_with(
        cx,
        "fn main() {\n    let x = 1;\n}\n",
        vec![],
        EditorSettings::default(),
        Some("rust"),
    );
    visual.run_until_parked();

    let spans = visual.update(|_window, cx| {
        view.update(cx, |view, _cx| {
            let len = view.text().len();
            view.highlights(0..len).to_vec()
        })
    });
    assert!(!spans.is_empty(), "the Rust grammar must produce spans");
    assert!(
        visual.update(|_window, cx| view.read(cx).highlight_version() > 0),
        "the highlight version must move when a parse lands"
    );
}

#[gpui::test]
fn double_and_triple_click_select_a_word_and_a_row(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "uno dos tres\n", vec![]);
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.begin_selection(DisplayPoint::new(0, 5), false, 2, cx);
        })
    });
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).selected_text()),
        "dos"
    );

    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.begin_selection(DisplayPoint::new(0, 5), false, 3, cx);
        })
    });
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).selected_text()),
        "uno dos tres"
    );
}

#[gpui::test]
fn the_cursor_stops_blinking_after_the_idle_time(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "hola\n", vec![]);
    // The harness clock is virtual: advancing past the idle time must leave the
    // cursor solid instead of toggling.
    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(7));
    visual.run_until_parked();
    assert!(
        visual.update(|_window, cx| view.read(cx).blink_visible),
        "after 5 s of idle the cursor stays on"
    );
}

#[gpui::test]
fn the_scroll_row_round_trips_before_and_after_the_first_layout(cx: &mut TestAppContext) {
    cx.update(bind_default_keys);
    let text: String = (0..500).map(|row| format!("linea {row}\n")).collect();
    let buffer = shared(Buffer::new(&text));
    let registry = Arc::new(LanguageRegistry::new());
    let window = cx.add_window(move |window, cx| {
        EditorView::new(
            buffer,
            None,
            registry,
            EditorSettings::default(),
            EditorTheme::default(),
            window,
            cx,
        )
    });
    let view = window.update(cx, |_, _, cx| cx.entity()).unwrap();
    let handle: AnyWindowHandle = window.into();
    let mut visual = VisualTestContext::from_window(handle, cx);

    // Before any layout: the value is kept as-is and reads back.
    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_scroll_row(120., cx)));
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).scroll_row()),
        120.,
        "a scroll set before the first frame reads back unclamped"
    );

    visual.run_until_parked();
    let after = visual.update(|_window, cx| view.read(cx).scroll_row());
    assert!(
        (after - 120.).abs() < 1.,
        "the first layout applies it, got {after}"
    );

    // After a layout it is clamped to the document.
    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_scroll_row(10_000., cx)));
    visual.run_until_parked();
    let clamped = visual.update(|_window, cx| view.read(cx).scroll_row());
    assert!(
        clamped < 500. && clamped > 0.,
        "clamped into the document, got {clamped}"
    );

    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_scroll_row(0., cx)));
    visual.run_until_parked();
    assert_eq!(visual.update(|_window, cx| view.read(cx).scroll_row()), 0.);
}

#[gpui::test]
fn scrolling_emits_scroll_changed_once_per_frame(cx: &mut TestAppContext) {
    let text: String = (0..500).map(|row| format!("linea {row}\n")).collect();
    let (view, _handle, mut visual) = open(cx, &text, vec![]);
    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    visual.update(|_window, cx| {
        let sink = events.clone();
        cx.subscribe(
            &view,
            move |_entity, event: &crate::view::EditorEvent, _cx| {
                if let crate::view::EditorEvent::ScrollChanged { row } = event {
                    sink.lock().push(*row);
                }
            },
        )
        .detach();
    });

    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_scroll_row(42., cx)));
    visual.run_until_parked();
    let rows = events.lock().clone();
    assert_eq!(rows.len(), 1, "one event per painted frame, got {rows:?}");
    assert!((rows[0] - 42.).abs() < 1., "got {rows:?}");

    // Painting again without moving must not emit anything else.
    visual.update(|_window, cx| view.update(cx, |_view, cx| cx.notify()));
    visual.run_until_parked();
    assert_eq!(events.lock().len(), 1, "a still editor stays quiet");
}

#[gpui::test]
fn set_cursor_clips_the_point_and_reports_it(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "uno\ndos\ntres\n", vec![]);
    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    visual.update(|_window, cx| {
        let sink = events.clone();
        cx.subscribe(
            &view,
            move |_entity, event: &crate::view::EditorEvent, _cx| {
                if let crate::view::EditorEvent::CursorMoved { point } = event {
                    sink.lock().push(*point);
                }
            },
        )
        .detach();
    });

    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.set_cursor(asteroid_text::Point::new(1, 2), cx)
        })
    });
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).cursor_point()),
        asteroid_text::Point::new(1, 2)
    );
    assert!(
        !visual.update(|_window, cx| view.read(cx).has_selection()),
        "the selection collapses"
    );
    assert_eq!(events.lock().last(), Some(&asteroid_text::Point::new(1, 2)));

    // Out of range in both axes: clipped to the end of the document.
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.set_cursor(asteroid_text::Point::new(999, 999), cx)
        })
    });
    visual.run_until_parked();
    let clipped = visual.update(|_window, cx| view.read(cx).cursor_point());
    assert!(clipped.row <= 3, "clipped to the last row, got {clipped:?}");
}

#[gpui::test]
fn set_cursor_scrolls_the_target_into_view(cx: &mut TestAppContext) {
    let text: String = (0..500).map(|row| format!("linea {row}\n")).collect();
    let (view, _handle, mut visual) = open(cx, &text, vec![]);
    assert_eq!(visual.update(|_window, cx| view.read(cx).scroll_row()), 0.);

    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.set_cursor(asteroid_text::Point::new(300, 0), cx)
        })
    });
    visual.run_until_parked();
    let row = visual.update(|_window, cx| view.read(cx).scroll_row());
    assert!(row > 200., "the cursor row must be on screen, got {row}");
}

#[gpui::test]
fn symbol_at_cursor_walks_up_the_rust_tree(cx: &mut TestAppContext) {
    let source = "\
struct Factura {
    total: u32,
}

impl Factura {
    fn suma(&self, otro: u32) -> u32 {
        self.total + otro
    }
}

const LIBRE: u32 = 0;
";
    let (view, _handle, mut visual) =
        open_with(cx, source, vec![], EditorSettings::default(), Some("rust"));
    visual.run_until_parked();

    let symbol_at = |visual: &mut VisualTestContext, row: u32, column: u32| {
        visual.update(|_window, cx| {
            view.update(cx, |view, cx| {
                view.set_cursor(asteroid_text::Point::new(row, column), cx);
                view.symbol_at_cursor()
            })
        })
    };

    // Inside the body of the method: the innermost definition is the method.
    let (name, range) = symbol_at(&mut visual, 6, 12).expect("a symbol inside the method");
    assert_eq!(name, "suma");
    let text = visual.update(|_window, cx| view.read(cx).text());
    assert!(
        text[range.clone()].starts_with("fn suma"),
        "the range covers the whole definition: {:?}",
        &text[range]
    );

    // Inside the struct body: the struct.
    let (name, _) = symbol_at(&mut visual, 1, 6).expect("a symbol inside the struct");
    assert_eq!(name, "Factura");

    // On the `impl` line but outside the method: the impl, named by its type.
    let (name, _) = symbol_at(&mut visual, 4, 5).expect("a symbol inside the impl");
    assert_eq!(name, "Factura");

    // Top level: nothing encloses the cursor.
    assert_eq!(symbol_at(&mut visual, 10, 0), None);
}

#[gpui::test]
fn symbol_at_cursor_works_for_python(cx: &mut TestAppContext) {
    let source = "\
class Factura:
    def suma(self, otro):
        return otro

LIBRE = 0
";
    let (view, _handle, mut visual) = open_with(
        cx,
        source,
        vec![],
        EditorSettings::default(),
        Some("python"),
    );
    visual.run_until_parked();

    let symbol_at = |visual: &mut VisualTestContext, row: u32, column: u32| {
        visual.update(|_window, cx| {
            view.update(cx, |view, cx| {
                view.set_cursor(asteroid_text::Point::new(row, column), cx);
                view.symbol_at_cursor()
            })
        })
    };

    let (name, _) = symbol_at(&mut visual, 2, 8).expect("a symbol inside the method");
    assert_eq!(name, "suma");
    let (name, _) = symbol_at(&mut visual, 0, 6).expect("a symbol on the class line");
    assert_eq!(name, "Factura");
    assert_eq!(symbol_at(&mut visual, 4, 0), None);
}

#[gpui::test]
fn symbol_at_cursor_is_none_without_a_grammar(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "fn main() {}\n", vec![]);
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).symbol_at_cursor()),
        None,
        "plain text has no tree"
    );
}

#[gpui::test]
fn ime_composition_marks_and_commits_the_text(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;

    let (view, _handle, mut visual) = open(cx, "", vec![]);
    // A dead-key / IME sequence: mark "n", then commit "ñ".
    visual.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
        })
    });
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "n");
    assert!(
        visual.update(|_window, cx| view.read(cx).marked_range.is_some()),
        "the composition must be marked so it can be underlined"
    );

    visual.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.replace_text_in_range(None, "ñ", window, cx);
        })
    });
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "ñ");
    assert!(visual.update(|_window, cx| view.read(cx).marked_range.is_none()));
}

#[gpui::test]
fn settings_theme_and_language_hot_reload(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "fn main() {}\n", vec![]);
    let registry = Arc::new(LanguageRegistry::new());
    let rust = registry.language("rust");
    assert!(rust.is_some(), "the rust grammar must be compiled in");

    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.set_settings(
                EditorSettings {
                    tab_size: 2,
                    soft_wrap: true,
                    show_whitespace: true,
                    ruler: Some(80),
                    ..Default::default()
                },
                cx,
            );
            view.set_theme(EditorTheme::default(), cx);
            view.set_language(rust.clone(), cx);
        })
    });
    visual.run_until_parked();

    let (tab_size, wrap, language) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (
            view.settings().tab_size,
            view.soft_wrap(),
            view.language().map(|language| language.name()),
        )
    });
    assert_eq!(tab_size, 2);
    assert!(wrap);
    assert_eq!(language, Some("rust"));

    // A `tab` now inserts two spaces.
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.cursor = crate::DisplayPoint::new(0, 0);
            view.selection_anchor = view.cursor;
            view.insert_text(&view.settings().indent_unit(), cx);
        })
    });
    visual.run_until_parked();
    assert_eq!(text_of(&view, &mut visual), "  fn main() {}\n");
}

#[gpui::test]
fn next_and_prev_hunk_jump_and_wrap_around(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(
        cx,
        "a\nb\nc\nd\ne\n",
        vec![hunk(1, &["viejo"], 1..2), hunk(4, &["otro"], 4..5)],
    );
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));

    cx.simulate_keystrokes(handle, "alt-j");
    visual.run_until_parked();
    let first = visual.update(|_window, cx| view.read(cx).cursor.row);
    assert_eq!(first, 1, "the first hunk starts at its phantom row");

    cx.simulate_keystrokes(handle, "alt-j");
    visual.run_until_parked();
    let second = visual.update(|_window, cx| view.read(cx).cursor.row);
    assert!(second > first, "{first} -> {second}");

    cx.simulate_keystrokes(handle, "alt-k");
    visual.run_until_parked();
    assert_eq!(visual.update(|_window, cx| view.read(cx).cursor.row), first);
}

#[gpui::test]
fn accept_file_drops_every_hunk_without_touching_the_text(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(
        cx,
        "a\nb\nc\nd\ne\n",
        vec![hunk(1, &["viejo"], 1..2), hunk(4, &["otro"], 4..5)],
    );
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    visual.run_until_parked();
    cx.simulate_keystrokes(handle, "ctrl-shift-enter");
    visual.run_until_parked();

    let (hunks, text) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.hunks().len(), view.text())
    });
    assert_eq!(hunks, 0);
    assert_eq!(text, "a\nb\nc\nd\ne\n");
}

#[gpui::test]
fn the_scrollbar_appears_on_mouse_move_and_fades(cx: &mut TestAppContext) {
    let text: String = (0..500).map(|row| format!("linea {row}\n")).collect();
    let (view, _handle, mut visual) = open(cx, &text, vec![]);
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).scrollbar_alpha()),
        0.,
        "hidden until the mouse moves"
    );

    visual.simulate_mouse_move(
        gpui::point(gpui::px(200.), gpui::px(200.)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).scrollbar_alpha()),
        1.,
        "fully visible right after the move"
    );

    // ~1 s of hold plus the 200 ms fade.
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(1400));
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).scrollbar_alpha()),
        0.,
        "faded out again"
    );
}

#[gpui::test]
fn horizontal_scrolling_only_exists_without_soft_wrap(cx: &mut TestAppContext) {
    let long: String = std::iter::repeat_n("palabra ", 200).collect();
    let (view, handle, mut visual) = open(cx, &format!("{long}\n"), vec![]);
    visual.run_until_parked();

    visual.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(gpui::px(200.), gpui::px(200.)),
        delta: gpui::ScrollDelta::Lines(gpui::point(-3., 0.)),
        modifiers: gpui::Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    visual.run_until_parked();
    let scrolled = visual.update(|_window, cx| view.read(cx).scroll_left);
    assert!(scrolled > 0., "a long row scrolls sideways, got {scrolled}");

    cx.simulate_keystrokes(handle, "alt-z");
    visual.run_until_parked();
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).scroll_left),
        0.,
        "turning soft wrap on resets the horizontal scroll"
    );
}

/// Highlight spans of one buffer row, in row-local byte offsets, so they can be
/// compared across an edit that moved every offset after it.
fn row_spans(
    view: &Entity<EditorView>,
    cx: &mut VisualTestContext,
    row: u32,
) -> Vec<(std::ops::Range<usize>, asteroid_syntax::HighlightId)> {
    cx.update(|_window, cx| {
        view.update(cx, |view, _cx| {
            let start = view.snapshot_line_start(row);
            let end = start + view.display_row_source(row).len();
            view.highlights(start..end)
                .iter()
                .filter(|(span, _)| span.end > start && span.start < end)
                .map(|(span, id)| {
                    (
                        span.start.max(start) - start..span.end.min(end) - start,
                        *id,
                    )
                })
                .collect()
        })
    })
}

#[gpui::test]
fn typing_never_paints_a_frame_without_the_highlights_it_already_had(cx: &mut TestAppContext) {
    // The bug this guards: every keystroke moved the tree-sitter state to the
    // background executor, `highlights()` came back empty while it was away and
    // the whole viewport was painted in the plain text colour for a frame or
    // two — every character in the editor blinking on each keystroke.
    const TEXT: &str = "\
// una nota
fn main() {
    let alpha = 1;
    let beta = 2;
    let gamma = alpha + beta;
}
";
    // The comment on row 0 is where the character is typed.
    const EDITED_ROW: u32 = 0;
    let rows = TEXT.lines().count() as u32;

    let (view, handle, mut visual) =
        open_with(cx, TEXT, vec![], EditorSettings::default(), Some("rust"));
    visual.run_until_parked();

    let before: Vec<_> = (0..rows)
        .map(|row| row_spans(&view, &mut visual, row))
        .collect();
    assert!(
        before
            .iter()
            .enumerate()
            .filter(|(row, _)| *row as u32 != EDITED_ROW)
            .all(|(_, spans)| !spans.is_empty()),
        "every row must start out highlighted, got {before:?}"
    );

    // One painted frame with the probe on: the baseline every later frame is
    // compared against.
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.set_render_probe(true);
            cx.notify();
        })
    });
    visual.run_until_parked();
    let baseline = visual
        .update(|_window, cx| view.update(cx, |view, _cx| view.take_render_frames()))
        .pop()
        .expect("the probe must have recorded the frame before the keystroke");
    assert!(
        baseline.rows.iter().any(|row| row.highlight_spans > 0),
        "the baseline frame must have painted highlights"
    );

    // Type at the end of the comment.
    set_cursor(
        &view,
        &mut visual,
        DisplayPoint::new(EDITED_ROW, TEXT.lines().next().unwrap().len() as u32),
    );
    cx.simulate_keystrokes(handle, "s");
    visual.run_until_parked();

    let frames =
        visual.update(|_window, cx| view.update(cx, |view, _cx| view.take_render_frames()));
    assert!(
        !frames.is_empty(),
        "the keystroke must have painted at least one frame"
    );

    // (1) No frame in between dropped the highlights of a row outside the edit,
    // and (2) none of those rows was shaped again.
    for (ix, frame) in frames.iter().enumerate() {
        for painted in &frame.rows {
            if painted.display_row == EDITED_ROW {
                continue;
            }
            let expected = baseline
                .row(painted.display_row)
                .expect("the baseline painted the same rows")
                .highlight_spans;
            assert_eq!(
                painted.highlight_spans, expected,
                "frame {ix} painted row {} with {} spans instead of {expected}",
                painted.display_row, painted.highlight_spans
            );
            assert!(
                !painted.reshaped,
                "frame {ix} re-shaped row {}, which the keystroke did not change",
                painted.display_row
            );
        }
    }

    // And the spans themselves are the ones the row had before the keystroke.
    let after: Vec<_> = (0..rows)
        .map(|row| row_spans(&view, &mut visual, row))
        .collect();
    for row in 0..rows {
        if row == EDITED_ROW {
            continue;
        }
        assert_eq!(
            after[row as usize], before[row as usize],
            "row {row} lost or changed its highlight spans"
        );
    }
}

#[gpui::test]
fn one_keystroke_reparses_without_leaving_the_ui_thread(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_with(
        cx,
        "fn main() {\n    let alpha = 1;\n}\n",
        vec![],
        EditorSettings::default(),
        Some("rust"),
    );
    visual.run_until_parked();
    let version = visual.update(|_window, cx| view.read(cx).highlight_version());

    set_cursor(&view, &mut visual, DisplayPoint::new(1, 18));
    cx.simulate_keystrokes(handle, "s");
    // No `run_until_parked`: an incremental reparse of one character fits in
    // `SYNC_PARSE_BUDGET`, so the highlights are already current here and the
    // frame that paints the character paints it with its final colours.
    assert!(
        visual.update(|_window, cx| view.read(cx).highlight_version()) > version,
        "the keystroke must have been reparsed on the UI thread"
    );
}
