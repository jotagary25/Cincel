//! Tests of the line, bracket and case commands over the GPUI test harness:
//! move/duplicate/delete lines, comments, join, line and word selection,
//! smart Home, auto-closed pairs, matching brackets, case, sort, auto-indent
//! and indentation of a selection.

use std::sync::Arc;

use cincel_syntax::LanguageRegistry;
use cincel_text::{Buffer, Point};
use gpui::{AnyWindowHandle, ClipboardItem, Entity, TestAppContext, VisualTestContext};

use crate::actions::{Lowercase, SortLines, Uppercase, bind_default_keys};
use crate::display_map::DisplayPoint;
use crate::settings::{EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::EditorView;

/// Opens a focused editor over `text`, highlighted as `language`.
fn open(
    cx: &mut TestAppContext,
    text: &str,
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
            EditorSettings::default(),
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

/// Selects from `anchor` to `cursor` (`(row, column)`; no hunks, so display
/// and buffer rows are the same).
fn select(
    view: &Entity<EditorView>,
    cx: &mut VisualTestContext,
    anchor: (u32, u32),
    cursor: (u32, u32),
) {
    cx.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.selection_anchor = view.clip_point(DisplayPoint::new(anchor.0, anchor.1));
            view.cursor = view.clip_point(DisplayPoint::new(cursor.0, cursor.1));
            cx.notify();
        })
    });
    cx.run_until_parked();
}

fn place(view: &Entity<EditorView>, cx: &mut VisualTestContext, at: (u32, u32)) {
    select(view, cx, at, at);
}

fn keys(handle: AnyWindowHandle, cx: &mut TestAppContext, visual: &mut VisualTestContext, k: &str) {
    cx.simulate_keystrokes(handle, k);
    visual.run_until_parked();
}

fn typed(
    handle: AnyWindowHandle,
    cx: &mut TestAppContext,
    visual: &mut VisualTestContext,
    t: &str,
) {
    for ch in t.chars() {
        cx.simulate_input(handle, &ch.to_string());
    }
    visual.run_until_parked();
}

fn text(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
    cx.update(|_window, cx| view.read(cx).text())
}

/// `(anchor, cursor)` in buffer points.
fn selection(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> (Point, Point) {
    cx.update(|_window, cx| view.read(cx).buffer_selection())
}

fn cursor(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Point {
    cx.update(|_window, cx| view.read(cx).cursor_point())
}

fn p(row: u32, column: u32) -> Point {
    Point::new(row, column)
}

// -- move lines -------------------------------------------------------------------

#[gpui::test]
fn alt_up_moves_the_line_and_the_cursor_with_it(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nbb\ncc\n", None);
    place(&view, &mut visual, (2, 1));
    keys(handle, cx, &mut visual, "alt-up");
    assert_eq!(text(&view, &mut visual), "a\ncc\nbb\n");
    assert_eq!(cursor(&view, &mut visual), p(1, 1));
}

#[gpui::test]
fn alt_up_on_the_first_line_does_nothing(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\n", None);
    place(&view, &mut visual, (0, 1));
    keys(handle, cx, &mut visual, "alt-up");
    assert_eq!(text(&view, &mut visual), "a\nb\n");
    assert_eq!(cursor(&view, &mut visual), p(0, 1));
}

#[gpui::test]
fn alt_down_on_the_last_line_does_nothing(cx: &mut TestAppContext) {
    // With a final newline the empty row after it is not a line to swap with.
    let (view, handle, mut visual) = open(cx, "a\nb\n", None);
    place(&view, &mut visual, (1, 0));
    keys(handle, cx, &mut visual, "alt-down");
    assert_eq!(text(&view, &mut visual), "a\nb\n");
    // Without one, the last row is simply the last.
    let (view, handle, mut visual) = open(cx, "a\nb", None);
    place(&view, &mut visual, (1, 0));
    keys(handle, cx, &mut visual, "alt-down");
    assert_eq!(text(&view, &mut visual), "a\nb");
}

#[gpui::test]
fn alt_down_moves_every_selected_line_and_keeps_the_selection(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "one\ntwo\nthree\nfour\n", None);
    select(&view, &mut visual, (0, 1), (1, 2));
    keys(handle, cx, &mut visual, "alt-down");
    assert_eq!(text(&view, &mut visual), "three\none\ntwo\nfour\n");
    assert_eq!(selection(&view, &mut visual), (p(1, 1), p(2, 2)));
    keys(handle, cx, &mut visual, "alt-up alt-up");
    assert_eq!(text(&view, &mut visual), "one\ntwo\nthree\nfour\n");
    assert_eq!(selection(&view, &mut visual), (p(0, 1), p(1, 2)));
}

#[gpui::test]
fn moving_a_line_is_one_undo_step(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\nc\n", None);
    place(&view, &mut visual, (1, 0));
    keys(handle, cx, &mut visual, "alt-down ctrl-z");
    assert_eq!(text(&view, &mut visual), "a\nb\nc\n");
}

// -- duplicate and delete ---------------------------------------------------------

#[gpui::test]
fn shift_alt_down_duplicates_below_and_follows_the_copy(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nbc\nd\n", None);
    place(&view, &mut visual, (1, 1));
    keys(handle, cx, &mut visual, "shift-alt-down");
    assert_eq!(text(&view, &mut visual), "a\nbc\nbc\nd\n");
    assert_eq!(cursor(&view, &mut visual), p(2, 1));
}

#[gpui::test]
fn shift_alt_up_duplicates_above_and_stays_on_the_upper_copy(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nbc\n", None);
    place(&view, &mut visual, (1, 2));
    keys(handle, cx, &mut visual, "shift-alt-up");
    assert_eq!(text(&view, &mut visual), "a\nbc\nbc\n");
    assert_eq!(cursor(&view, &mut visual), p(1, 2));
}

#[gpui::test]
fn duplicating_a_selection_copies_every_line_it_covers(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "x\ny\nz", None);
    select(&view, &mut visual, (0, 0), (1, 1));
    keys(handle, cx, &mut visual, "shift-alt-down");
    assert_eq!(text(&view, &mut visual), "x\ny\nx\ny\nz");
    assert_eq!(selection(&view, &mut visual), (p(2, 0), p(3, 1)));
}

#[gpui::test]
fn ctrl_shift_k_deletes_the_line_and_keeps_the_column(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "abc\ndef\nghi\n", None);
    place(&view, &mut visual, (1, 2));
    keys(handle, cx, &mut visual, "ctrl-shift-k");
    assert_eq!(text(&view, &mut visual), "abc\nghi\n");
    assert_eq!(cursor(&view, &mut visual), p(1, 2));
}

#[gpui::test]
fn deleting_the_last_line_takes_the_newline_before_it(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "abc\nde", None);
    place(&view, &mut visual, (1, 1));
    keys(handle, cx, &mut visual, "ctrl-shift-k");
    assert_eq!(text(&view, &mut visual), "abc");
    assert_eq!(cursor(&view, &mut visual), p(0, 1));
    keys(handle, cx, &mut visual, "ctrl-shift-k");
    assert_eq!(text(&view, &mut visual), "");
}

#[gpui::test]
fn deleting_a_selection_removes_every_row_it_touches(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "1\n2\n3\n4\n", None);
    // Ends at column 0 of row 3: that row is not part of it.
    select(&view, &mut visual, (1, 1), (3, 0));
    keys(handle, cx, &mut visual, "ctrl-shift-k");
    assert_eq!(text(&view, &mut visual), "1\n4\n");
}

// -- comments -------------------------------------------------------------------------

#[gpui::test]
fn ctrl_slash_comments_and_uncomments_rust(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "fn a() {\n    x();\n}\n", Some("rust"));
    place(&view, &mut visual, (1, 6));
    keys(handle, cx, &mut visual, "ctrl-/");
    assert_eq!(text(&view, &mut visual), "fn a() {\n    // x();\n}\n");
    assert_eq!(
        cursor(&view, &mut visual),
        p(1, 9),
        "the cursor stays on its char"
    );
    keys(handle, cx, &mut visual, "ctrl-/");
    assert_eq!(text(&view, &mut visual), "fn a() {\n    x();\n}\n");
    assert_eq!(cursor(&view, &mut visual), p(1, 6));
}

#[gpui::test]
fn mixed_commented_lines_are_all_commented(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "// a\nb\n\n// c\n", Some("rust"));
    select(&view, &mut visual, (0, 0), (3, 4));
    keys(handle, cx, &mut visual, "ctrl-/");
    assert_eq!(text(&view, &mut visual), "// // a\n// b\n\n// // c\n");
    // The selection still covers every line, from column 0.
    assert_eq!(selection(&view, &mut visual), (p(0, 0), p(3, 7)));
}

#[gpui::test]
fn comments_use_the_token_of_the_language(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "x = 1\n", Some("python"));
    keys(handle, cx, &mut visual, "ctrl-/");
    assert_eq!(text(&view, &mut visual), "# x = 1\n");

    let (view, handle, mut visual) = open(cx, "select 1;\n", Some("sql"));
    keys(handle, cx, &mut visual, "ctrl-/");
    assert_eq!(text(&view, &mut visual), "-- select 1;\n");

    let (view, handle, mut visual) = open(cx, "<p>hola</p>\n", Some("html"));
    keys(handle, cx, &mut visual, "ctrl-/");
    assert_eq!(text(&view, &mut visual), "<!-- <p>hola</p> -->\n");
}

#[gpui::test]
fn comments_do_nothing_in_json_or_plain_text(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "{\"a\": 1}\n", Some("json"));
    keys(handle, cx, &mut visual, "ctrl-/");
    assert_eq!(text(&view, &mut visual), "{\"a\": 1}\n");
    let (view, handle, mut visual) = open(cx, "hola\n", None);
    keys(handle, cx, &mut visual, "ctrl-/");
    assert_eq!(text(&view, &mut visual), "hola\n");
}

// -- join, select line, select next ---------------------------------------------------

#[gpui::test]
fn ctrl_j_joins_with_the_next_line(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "let x =  \n    1;\nz\n", None);
    place(&view, &mut visual, (0, 2));
    keys(handle, cx, &mut visual, "ctrl-j");
    assert_eq!(text(&view, &mut visual), "let x = 1;\nz\n");
    assert_eq!(cursor(&view, &mut visual), p(0, 8), "at the join");
}

#[gpui::test]
fn ctrl_j_joins_every_selected_line(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "f(\n  a,\n  b\n)\n", None);
    select(&view, &mut visual, (0, 0), (3, 1));
    keys(handle, cx, &mut visual, "ctrl-j");
    assert_eq!(text(&view, &mut visual), "f(a, b)\n");
}

#[gpui::test]
fn ctrl_j_on_the_last_line_does_nothing(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb", None);
    place(&view, &mut visual, (1, 1));
    keys(handle, cx, &mut visual, "ctrl-j");
    assert_eq!(text(&view, &mut visual), "a\nb");
}

#[gpui::test]
fn ctrl_shift_l_selects_the_line_and_extends_on_repeat(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nbb\nc", None);
    place(&view, &mut visual, (0, 1));
    keys(handle, cx, &mut visual, "ctrl-shift-l");
    assert_eq!(selection(&view, &mut visual), (p(0, 0), p(1, 0)));
    keys(handle, cx, &mut visual, "ctrl-shift-l");
    assert_eq!(selection(&view, &mut visual), (p(0, 0), p(2, 0)));
    keys(handle, cx, &mut visual, "ctrl-shift-l");
    assert_eq!(
        selection(&view, &mut visual),
        (p(0, 0), p(2, 1)),
        "the last line ends the document"
    );
}

#[gpui::test]
fn ctrl_d_selects_the_word_then_its_next_occurrence(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "foo bar\nfoo_x foo\n", None);
    place(&view, &mut visual, (0, 1));
    keys(handle, cx, &mut visual, "ctrl-d");
    assert_eq!(selection(&view, &mut visual), (p(0, 0), p(0, 3)));
    keys(handle, cx, &mut visual, "ctrl-d");
    assert_eq!(selection(&view, &mut visual), (p(1, 0), p(1, 3)));
    keys(handle, cx, &mut visual, "ctrl-d");
    assert_eq!(selection(&view, &mut visual), (p(1, 6), p(1, 9)));
    keys(handle, cx, &mut visual, "ctrl-d");
    assert_eq!(
        selection(&view, &mut visual),
        (p(0, 0), p(0, 3)),
        "wraps around"
    );
}

#[gpui::test]
fn ctrl_d_takes_the_word_before_the_cursor_and_ignores_blanks(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "abc  (", None);
    place(&view, &mut visual, (0, 3));
    keys(handle, cx, &mut visual, "ctrl-d");
    assert_eq!(selection(&view, &mut visual), (p(0, 0), p(0, 3)));
    place(&view, &mut visual, (0, 5));
    keys(handle, cx, &mut visual, "ctrl-d");
    assert_eq!(selection(&view, &mut visual), (p(0, 5), p(0, 5)));
}

// -- smart Home ---------------------------------------------------------------------------

#[gpui::test]
fn home_toggles_between_the_first_non_blank_and_column_zero(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "    let x;\n", None);
    place(&view, &mut visual, (0, 8));
    keys(handle, cx, &mut visual, "home");
    assert_eq!(cursor(&view, &mut visual), p(0, 4));
    keys(handle, cx, &mut visual, "home");
    assert_eq!(cursor(&view, &mut visual), p(0, 0));
    keys(handle, cx, &mut visual, "home");
    assert_eq!(cursor(&view, &mut visual), p(0, 4));
    // From inside the indentation it goes to the first non-blank too.
    place(&view, &mut visual, (0, 2));
    keys(handle, cx, &mut visual, "home");
    assert_eq!(cursor(&view, &mut visual), p(0, 4));
}

#[gpui::test]
fn shift_home_selects_to_the_first_non_blank_then_column_zero(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "  ab\n", None);
    place(&view, &mut visual, (0, 4));
    keys(handle, cx, &mut visual, "shift-home");
    assert_eq!(selection(&view, &mut visual), (p(0, 4), p(0, 2)));
    keys(handle, cx, &mut visual, "shift-home");
    assert_eq!(selection(&view, &mut visual), (p(0, 4), p(0, 0)));
}

// -- auto-closed pairs -----------------------------------------------------------------

#[gpui::test]
fn an_opener_inserts_its_closer_and_the_closer_steps_over_it(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "\n", None);
    typed(handle, cx, &mut visual, "f(");
    assert_eq!(text(&view, &mut visual), "f()\n");
    assert_eq!(cursor(&view, &mut visual), p(0, 2));
    typed(handle, cx, &mut visual, "a[1");
    assert_eq!(text(&view, &mut visual), "f(a[1])\n");
    typed(handle, cx, &mut visual, "])");
    assert_eq!(
        text(&view, &mut visual),
        "f(a[1])\n",
        "both closers stepped over"
    );
    assert_eq!(cursor(&view, &mut visual), p(0, 7));
}

#[gpui::test]
fn no_pair_before_a_word_and_no_skip_over_a_typed_closer(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "x\n", None);
    place(&view, &mut visual, (0, 0));
    typed(handle, cx, &mut visual, "(");
    assert_eq!(text(&view, &mut visual), "(x\n", "the next char is a word");

    // A `)` the user typed is not stepped over.
    let (view, handle, mut visual) = open(cx, "()\n", None);
    place(&view, &mut visual, (0, 1));
    typed(handle, cx, &mut visual, ")");
    assert_eq!(text(&view, &mut visual), "())\n");
}

#[gpui::test]
fn quotes_pair_except_after_a_word(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "\n", Some("python"));
    typed(handle, cx, &mut visual, "x = \"");
    assert_eq!(text(&view, &mut visual), "x = \"\"\n");
    typed(handle, cx, &mut visual, "a\"");
    assert_eq!(
        text(&view, &mut visual),
        "x = \"a\"\n",
        "the quote steps over"
    );
    typed(handle, cx, &mut visual, " don't");
    assert_eq!(text(&view, &mut visual), "x = \"a\" don't\n");
}

#[gpui::test]
fn backspace_between_an_empty_pair_deletes_both(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "\n", None);
    typed(handle, cx, &mut visual, "{");
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "\n");
    // Also for a pair that was already there.
    let (view, handle, mut visual) = open(cx, "a[]\n", None);
    place(&view, &mut visual, (0, 2));
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "a\n");
}

#[gpui::test]
fn an_opener_wraps_the_selection(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "let x = a + b;\n", None);
    select(&view, &mut visual, (0, 8), (0, 13));
    typed(handle, cx, &mut visual, "(");
    assert_eq!(text(&view, &mut visual), "let x = (a + b);\n");
    assert_eq!(selection(&view, &mut visual), (p(0, 9), p(0, 14)));
    typed(handle, cx, &mut visual, "\"");
    assert_eq!(text(&view, &mut visual), "let x = (\"a + b\");\n");
}

#[gpui::test]
fn auto_close_can_be_turned_off(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "\n", None);
    visual.update(|_window, cx| view.update(cx, |view, _| view.set_auto_close_pairs(false)));
    typed(handle, cx, &mut visual, "(\"");
    assert_eq!(text(&view, &mut visual), "(\"\n");
}

/// `EditorSettings::auto_close_pairs` reaches the editor at construction, with
/// no separate call to `set_auto_close_pairs` needed
/// (`cincel_workspace::theme::editor_settings` is what maps
/// `settings.json`'s `editor.auto_close_pairs` onto this field).
#[gpui::test]
fn auto_close_pairs_setting_applies_from_construction(cx: &mut TestAppContext) {
    cx.update(bind_default_keys);
    let buffer = shared(Buffer::new("\n"));
    let registry = Arc::new(LanguageRegistry::new());
    let settings = EditorSettings {
        auto_close_pairs: false,
        ..Default::default()
    };
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
    let mut visual = VisualTestContext::from_window(handle, cx);
    window
        .update(cx, |view, window, cx| window.focus(&view.focus_handle, cx))
        .unwrap();
    visual.run_until_parked();

    assert!(!cx.update(|cx| view.read(cx).auto_close_pairs()));
    typed(handle, cx, &mut visual, "(\"");
    assert_eq!(text(&view, &mut visual), "(\"\n");
}

/// `EditorView::set_settings` also hot-reloads it (a `settings.json` save
/// while the editor is open).
#[gpui::test]
fn set_settings_hot_reloads_auto_close_pairs(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "\n", None);
    assert!(visual.update(|_window, cx| view.read(cx).auto_close_pairs()));

    let off = EditorSettings {
        auto_close_pairs: false,
        ..Default::default()
    };
    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_settings(off, cx)));
    assert!(!visual.update(|_window, cx| view.read(cx).auto_close_pairs()));
    typed(handle, cx, &mut visual, "(\"");
    assert_eq!(text(&view, &mut visual), "(\"\n");
}

#[gpui::test]
fn moving_away_forgets_the_auto_inserted_closer(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "\n", None);
    typed(handle, cx, &mut visual, "(");
    keys(handle, cx, &mut visual, "left right");
    typed(handle, cx, &mut visual, ")");
    assert_eq!(text(&view, &mut visual), "())\n");
}

// -- brackets -------------------------------------------------------------------------------

#[gpui::test]
fn the_matching_bracket_is_found_across_lines_and_nesting(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "f(a, [b], {\n  c(d)\n})\n", None);
    let brackets = |visual: &mut VisualTestContext| {
        visual.update(|_window, cx| view.read(cx).matching_brackets())
    };
    // Right after `(`: the one before the cursor counts.
    place(&view, &mut visual, (0, 2));
    assert_eq!(brackets(&mut visual), Some((1, 20)));
    // On `{`.
    place(&view, &mut visual, (0, 10));
    assert_eq!(brackets(&mut visual), Some((10, 19)));
    // No bracket around.
    place(&view, &mut visual, (1, 1));
    assert_eq!(brackets(&mut visual), None);

    place(&view, &mut visual, (0, 1));
    keys(handle, cx, &mut visual, "ctrl-shift-\\");
    assert_eq!(cursor(&view, &mut visual), p(2, 1), "on the matching `)`");
    keys(handle, cx, &mut visual, "ctrl-shift-\\");
    assert_eq!(cursor(&view, &mut visual), p(0, 1), "and back");
}

#[gpui::test]
fn an_unbalanced_bracket_has_no_match(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "((a)\n", None);
    place(&view, &mut visual, (0, 0));
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).matching_brackets()),
        None
    );
}

// -- case and sort ----------------------------------------------------------------------

#[gpui::test]
fn uppercase_and_lowercase_keep_the_selection(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "hola Mundo\n", None);
    select(&view, &mut visual, (0, 0), (0, 10));
    visual.dispatch_action(Uppercase);
    visual.run_until_parked();
    assert_eq!(text(&view, &mut visual), "HOLA MUNDO\n");
    assert_eq!(selection(&view, &mut visual), (p(0, 0), p(0, 10)));
    visual.dispatch_action(Lowercase);
    visual.run_until_parked();
    assert_eq!(text(&view, &mut visual), "hola mundo\n");
}

#[gpui::test]
fn case_without_a_selection_changes_the_word_under_the_cursor(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "uno dos\n", None);
    place(&view, &mut visual, (0, 5));
    visual.dispatch_action(Uppercase);
    visual.run_until_parked();
    assert_eq!(text(&view, &mut visual), "uno DOS\n");
    assert_eq!(cursor(&view, &mut visual), p(0, 5));
}

#[gpui::test]
fn sort_lines_sorts_the_selected_rows_only(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "z\nc\na\nb\n", None);
    select(&view, &mut visual, (1, 0), (3, 1));
    visual.dispatch_action(SortLines);
    visual.run_until_parked();
    assert_eq!(text(&view, &mut visual), "z\na\nb\nc\n");
    assert_eq!(selection(&view, &mut visual), (p(1, 0), p(3, 1)));
    // One line is nothing to sort.
    place(&view, &mut visual, (0, 0));
    visual.dispatch_action(SortLines);
    visual.run_until_parked();
    assert_eq!(text(&view, &mut visual), "z\na\nb\nc\n");
}

// -- auto-indent ----------------------------------------------------------------------------

#[gpui::test]
fn enter_between_an_empty_pair_opens_an_indented_line(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "    if x {}\n", None);
    place(&view, &mut visual, (0, 10));
    keys(handle, cx, &mut visual, "enter");
    assert_eq!(text(&view, &mut visual), "    if x {\n        \n    }\n");
    assert_eq!(cursor(&view, &mut visual), p(1, 8));
    // The same with the pair typed right away.
    let (view, handle, mut visual) = open(cx, "\n", None);
    typed(handle, cx, &mut visual, "f(");
    keys(handle, cx, &mut visual, "enter");
    assert_eq!(text(&view, &mut visual), "f(\n    \n)\n");
}

#[gpui::test]
fn a_multi_line_paste_takes_the_cursor_indentation(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "fn a() {\n    \n}\n", None);
    place(&view, &mut visual, (1, 4));
    visual.update(|_window, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string("if x {\n    y();\n}".into()))
    });
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(
        text(&view, &mut visual),
        "fn a() {\n    if x {\n        y();\n    }\n}\n"
    );
}

#[gpui::test]
fn tab_indents_every_selected_line_and_keeps_the_selection(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\n\nb\nc\n", None);
    select(&view, &mut visual, (0, 0), (2, 1));
    keys(handle, cx, &mut visual, "tab");
    assert_eq!(
        text(&view, &mut visual),
        "    a\n\n    b\nc\n",
        "the empty row stays empty"
    );
    assert_eq!(selection(&view, &mut visual), (p(0, 0), p(2, 5)));
    keys(handle, cx, &mut visual, "shift-tab");
    assert_eq!(text(&view, &mut visual), "a\n\nb\nc\n");
    assert_eq!(selection(&view, &mut visual), (p(0, 0), p(2, 1)));
}
