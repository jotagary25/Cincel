//! Tests of E4, E5 and E6 (`docs/specs/10-etapa7-ronda2.md` §7.4–§7.6): a
//! closer typed on an indentation-only row takes one level off, `Enter` on
//! such a row leaves it empty, and `Backspace` inside the indentation goes
//! back to the previous indent stop. All of it only in `EditorChrome::Full`.

use std::sync::Arc;

use cincel_syntax::LanguageRegistry;
use cincel_text::{Buffer, Point};
use gpui::{AnyWindowHandle, Entity, TestAppContext, VisualTestContext};

use crate::actions::{InsertNewline, bind_default_keys};
use crate::display_map::DisplayPoint;
use crate::settings::{EditorChrome, EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::EditorView;

fn open_with(
    cx: &mut TestAppContext,
    text: &str,
    language: Option<&str>,
    settings: EditorSettings,
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
        .update(cx, |view, window, cx| window.focus(&view.focus_handle, cx))
        .unwrap();
    visual.run_until_parked();
    (view, handle, visual)
}

fn open(
    cx: &mut TestAppContext,
    text: &str,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    open_with(cx, text, None, EditorSettings::default())
}

fn open_minimal(
    cx: &mut TestAppContext,
    text: &str,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    let settings = EditorSettings {
        chrome: EditorChrome::Minimal,
        ..EditorSettings::default()
    };
    open_with(cx, text, None, settings)
}

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

fn place(view: &Entity<EditorView>, cx: &mut VisualTestContext, row: u32, column: u32) {
    select(view, cx, (row, column), (row, column));
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

fn cursor(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Point {
    cx.update(|_window, cx| view.read(cx).cursor_point())
}

fn p(row: u32, column: u32) -> Point {
    Point::new(row, column)
}

// -- E4: a closer takes one level off -------------------------------------------------

#[gpui::test]
fn a_closer_on_an_indentation_only_row_takes_one_level_off(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "fn a() {\n        ");
    place(&view, &mut visual, 1, 8);
    typed(handle, cx, &mut visual, "}");
    assert_eq!(text(&view, &mut visual), "fn a() {\n    }");
    assert_eq!(cursor(&view, &mut visual), p(1, 5), "after the `}}`");
}

#[gpui::test]
fn the_three_closers_do_it_with_a_language_too(cx: &mut TestAppContext) {
    for closer in ["}", "]", ")"] {
        let (view, handle, mut visual) = open_with(
            cx,
            "fn a() {\n        ",
            Some("rust"),
            EditorSettings::default(),
        );
        place(&view, &mut visual, 1, 8);
        typed(handle, cx, &mut visual, closer);
        assert_eq!(
            text(&view, &mut visual),
            format!("fn a() {{\n    {closer}"),
            "closer {closer}"
        );
    }
}

#[gpui::test]
fn with_tabs_one_tab_goes(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "\t\t");
    place(&view, &mut visual, 0, 2);
    typed(handle, cx, &mut visual, ")");
    assert_eq!(text(&view, &mut visual), "\t)");
    assert_eq!(cursor(&view, &mut visual), p(0, 2));
}

#[gpui::test]
fn a_closer_after_code_keeps_the_indentation(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "    foo");
    place(&view, &mut visual, 0, 7);
    typed(handle, cx, &mut visual, "}");
    assert_eq!(text(&view, &mut visual), "    foo}");
    // Code after the cursor counts too.
    let (view, handle, mut visual) = open(cx, "    x");
    place(&view, &mut visual, 0, 4);
    typed(handle, cx, &mut visual, ")");
    assert_eq!(text(&view, &mut visual), "    )x");
    // And so does an empty indentation: there is nothing to take off.
    let (view, handle, mut visual) = open(cx, "");
    typed(handle, cx, &mut visual, "}");
    assert_eq!(text(&view, &mut visual), "}");
}

#[gpui::test]
fn blanks_after_the_cursor_stay(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "          ");
    place(&view, &mut visual, 0, 8);
    typed(handle, cx, &mut visual, "]");
    assert_eq!(text(&view, &mut visual), "    ]  ");
    assert_eq!(cursor(&view, &mut visual), p(0, 5));
}

#[gpui::test]
fn ctrl_z_leaves_the_row_as_it_was_before_the_closer(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "fn a() {\n        ");
    place(&view, &mut visual, 1, 8);
    typed(handle, cx, &mut visual, "}");
    assert_eq!(text(&view, &mut visual), "fn a() {\n    }");
    keys(handle, cx, &mut visual, "ctrl-z");
    assert_eq!(
        text(&view, &mut visual),
        "fn a() {\n        ",
        "one step: the indentation comes back with the closer gone"
    );
    keys(handle, cx, &mut visual, "ctrl-shift-z");
    assert_eq!(text(&view, &mut visual), "fn a() {\n    }");
}

#[gpui::test]
fn the_level_taken_off_is_the_one_shift_tab_takes(cx: &mut TestAppContext) {
    for indent in ["        ", "      ", "  ", " ", "\t\t", "\t", "  \t"] {
        let (view, handle, mut visual) = open(cx, indent);
        place(&view, &mut visual, 0, indent.len() as u32);
        keys(handle, cx, &mut visual, "shift-tab");
        let by_shift_tab = text(&view, &mut visual);

        let (view, handle, mut visual) = open(cx, indent);
        place(&view, &mut visual, 0, indent.len() as u32);
        typed(handle, cx, &mut visual, "}");
        assert_eq!(
            text(&view, &mut visual),
            format!("{by_shift_tab}}}"),
            "indent {indent:?}"
        );
    }
}

#[gpui::test]
fn a_two_space_file_loses_two_spaces(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\n  b\n    c\n    ");
    place(&view, &mut visual, 3, 4);
    typed(handle, cx, &mut visual, "}");
    assert_eq!(text(&view, &mut visual), "a\n  b\n    c\n  }");
}

#[gpui::test]
fn the_step_over_an_auto_closed_closer_wins(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "    ");
    place(&view, &mut visual, 0, 4);
    typed(handle, cx, &mut visual, "(");
    assert_eq!(text(&view, &mut visual), "    ()");
    typed(handle, cx, &mut visual, ")");
    assert_eq!(
        text(&view, &mut visual),
        "    ()",
        "stepped over, indentation untouched"
    );
    assert_eq!(cursor(&view, &mut visual), p(0, 6));
}

#[gpui::test]
fn a_selection_is_replaced_by_the_closer_as_always(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "        ");
    select(&view, &mut visual, (0, 4), (0, 8));
    typed(handle, cx, &mut visual, "}");
    assert_eq!(text(&view, &mut visual), "    }");
}

#[gpui::test]
fn an_ime_composition_does_not_outdent(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "    ");
    place(&view, &mut visual, 0, 4);
    visual.update(|_window, cx| view.update(cx, |view, _| view.marked_range = Some(4..4)));
    typed(handle, cx, &mut visual, "}");
    assert_eq!(text(&view, &mut visual), "    }");
}

#[gpui::test]
fn a_minimal_editor_does_not_outdent_a_closer(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_minimal(cx, "        ");
    place(&view, &mut visual, 0, 8);
    typed(handle, cx, &mut visual, "}");
    assert_eq!(text(&view, &mut visual), "        }");
}

// -- E5: Enter on an indentation-only row --------------------------------------------------

#[gpui::test]
fn enter_on_an_indentation_only_row_leaves_it_empty(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "x\n        ");
    place(&view, &mut visual, 1, 8);
    keys(handle, cx, &mut visual, "enter");
    assert_eq!(text(&view, &mut visual), "x\n\n        ");
    assert_eq!(cursor(&view, &mut visual), p(2, 8));
}

#[gpui::test]
fn enter_in_the_middle_of_the_indentation_splits_it_at_the_cursor(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "        ");
    place(&view, &mut visual, 0, 4);
    keys(handle, cx, &mut visual, "enter");
    assert_eq!(
        text(&view, &mut visual),
        "\n    ",
        "empty row, then 4 spaces"
    );
    assert_eq!(cursor(&view, &mut visual), p(1, 4));
}

#[gpui::test]
fn enter_on_tabs_and_on_an_empty_row(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "\t\t");
    place(&view, &mut visual, 0, 2);
    keys(handle, cx, &mut visual, "enter");
    assert_eq!(text(&view, &mut visual), "\n\t\t");
    assert_eq!(cursor(&view, &mut visual), p(1, 2));

    let (view, handle, mut visual) = open(cx, "x\n");
    place(&view, &mut visual, 1, 0);
    keys(handle, cx, &mut visual, "enter");
    assert_eq!(text(&view, &mut visual), "x\n\n");
    assert_eq!(cursor(&view, &mut visual), p(2, 0));
}

#[gpui::test]
fn enter_is_one_undo_step(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "x\n        ");
    place(&view, &mut visual, 1, 8);
    keys(handle, cx, &mut visual, "enter");
    keys(handle, cx, &mut visual, "ctrl-z");
    assert_eq!(text(&view, &mut visual), "x\n        ");
}

#[gpui::test]
fn enter_after_code_keeps_the_indentation_as_before(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "    foo");
    place(&view, &mut visual, 0, 7);
    keys(handle, cx, &mut visual, "enter");
    assert_eq!(text(&view, &mut visual), "    foo\n    ");
    assert_eq!(cursor(&view, &mut visual), p(1, 4));
}

#[gpui::test]
fn the_other_enter_rules_do_not_change(cx: &mut TestAppContext) {
    // After an opener the next row is one level deeper.
    let (view, handle, mut visual) = open(cx, "fn a() {");
    place(&view, &mut visual, 0, 8);
    keys(handle, cx, &mut visual, "enter");
    assert_eq!(text(&view, &mut visual), "fn a() {\n    ");
    // Between a pair the closer goes on its own row.
    let (view, handle, mut visual) = open(cx, "    if x {}");
    place(&view, &mut visual, 0, 10);
    keys(handle, cx, &mut visual, "enter");
    assert_eq!(text(&view, &mut visual), "    if x {\n        \n    }");
    assert_eq!(cursor(&view, &mut visual), p(1, 8));
}

#[gpui::test]
fn enter_with_a_selection_replaces_it_as_before(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "      ");
    select(&view, &mut visual, (0, 2), (0, 4));
    keys(handle, cx, &mut visual, "enter");
    assert_eq!(text(&view, &mut visual), "  \n        ");
}

#[gpui::test]
fn a_minimal_editor_keeps_the_old_enter(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_minimal(cx, "    ");
    place(&view, &mut visual, 0, 4);
    visual.dispatch_action(InsertNewline);
    visual.run_until_parked();
    assert_eq!(text(&view, &mut visual), "    \n    ");
}

// -- E6: Backspace inside the indentation --------------------------------------------------

#[gpui::test]
fn backspace_goes_back_to_the_previous_indent_stop(cx: &mut TestAppContext) {
    for (spaces, left) in [(8, 4), (6, 4), (1, 0), (4, 0), (5, 4), (2, 0), (3, 0)] {
        let indent = " ".repeat(spaces);
        let (view, handle, mut visual) = open(cx, &indent);
        place(&view, &mut visual, 0, spaces as u32);
        keys(handle, cx, &mut visual, "backspace");
        assert_eq!(
            text(&view, &mut visual),
            " ".repeat(left),
            "{spaces} spaces"
        );
        assert_eq!(cursor(&view, &mut visual), p(0, left as u32));
    }
}

#[gpui::test]
fn backspace_repeats_down_to_the_margin(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "x\n          ");
    place(&view, &mut visual, 1, 10);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "x\n        ");
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "x\n    ");
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "x\n");
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(
        text(&view, &mut visual),
        "x",
        "column 0 joins the rows as before"
    );
}

#[gpui::test]
fn backspace_in_the_middle_of_the_indentation_keeps_what_is_after(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\n    b\n        x");
    place(&view, &mut visual, 2, 6);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "a\n    b\n      x");
    assert_eq!(cursor(&view, &mut visual), p(2, 4));
}

#[gpui::test]
fn backspace_after_code_deletes_one_character(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "    a");
    place(&view, &mut visual, 0, 5);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "    ");
}

#[gpui::test]
fn backspace_with_tabs_or_a_mix_deletes_one_character(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "\t\t");
    place(&view, &mut visual, 0, 2);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "\t");

    let (view, handle, mut visual) = open(cx, "\t    ");
    place(&view, &mut visual, 0, 5);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(
        text(&view, &mut visual),
        "\t   ",
        "a tab before: one character"
    );

    let (view, handle, mut visual) = open(cx, "  \t  ");
    place(&view, &mut visual, 0, 5);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "  \t ");
}

#[gpui::test]
fn backspace_with_a_selection_deletes_the_selection(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "        ");
    select(&view, &mut visual, (0, 2), (0, 6));
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "    ");
}

#[gpui::test]
fn the_pair_deleter_still_goes_first(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "    ()");
    place(&view, &mut visual, 0, 5);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "    ");
    assert_eq!(cursor(&view, &mut visual), p(0, 4));
}

#[gpui::test]
fn the_stop_follows_the_indent_unit_of_the_file(cx: &mut TestAppContext) {
    // A two-space file.
    let (view, handle, mut visual) = open(cx, "a\n  b\n    c\n      ");
    place(&view, &mut visual, 3, 6);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "a\n  b\n    c\n    ");
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "a\n  b\n    c\n  ");

    // A tab file: spaces typed in it go one at a time.
    let (view, handle, mut visual) = open(cx, "\tx\n    ");
    place(&view, &mut visual, 1, 4);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "\tx\n   ");

    // No indentation in the file: `editor.tab_size`.
    let settings = EditorSettings {
        tab_size: 2,
        ..EditorSettings::default()
    };
    let (view, handle, mut visual) = open_with(cx, "      ", None, settings);
    place(&view, &mut visual, 0, 6);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "    ");
}

#[gpui::test]
fn backspace_in_the_indentation_is_one_undo_step(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "        ");
    place(&view, &mut visual, 0, 8);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "    ");
    keys(handle, cx, &mut visual, "ctrl-z");
    assert_eq!(text(&view, &mut visual), "        ");
}

#[gpui::test]
fn a_minimal_editor_deletes_one_space(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_minimal(cx, "        ");
    place(&view, &mut visual, 0, 8);
    keys(handle, cx, &mut visual, "backspace");
    assert_eq!(text(&view, &mut visual), "       ");
}
