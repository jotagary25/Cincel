//! Tests of E2 (`docs/specs/10-etapa7-ronda2.md` §7.2): `Ctrl+C` / `Ctrl+X`
//! without a selection copy or cut the cursor's row, and `Ctrl+V` of such a
//! line goes above the cursor's row. The clipboard is the test platform's.

use std::sync::Arc;

use cincel_syntax::LanguageRegistry;
use cincel_text::{Buffer, Point};
use gpui::{
    AnyWindowHandle, ClipboardEntry, ClipboardItem, Entity, TestAppContext, VisualTestContext,
};

use crate::actions::bind_default_keys;
use crate::display_map::DisplayPoint;
use crate::review::{ReviewHunkKind, ReviewHunkView, ReviewLineView, ReviewView};
use crate::settings::{EditorChrome, EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::{EditorView, LineClipboard, LineCopyMetadata};

fn open_with(
    cx: &mut TestAppContext,
    text: &str,
    settings: EditorSettings,
    review: Option<ReviewView>,
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
        .update(cx, |view, window, cx| {
            window.focus(&view.focus_handle, cx);
            if let Some(review) = review {
                view.set_review(review, cx);
            }
        })
        .unwrap();
    visual.run_until_parked();
    (view, handle, visual)
}

fn open(
    cx: &mut TestAppContext,
    text: &str,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    open_with(cx, text, EditorSettings::default(), None)
}

fn open_minimal(
    cx: &mut TestAppContext,
    text: &str,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    let settings = EditorSettings {
        chrome: EditorChrome::Minimal,
        ..EditorSettings::default()
    };
    open_with(cx, text, settings, None)
}

/// A file whose first buffer row replaced the base line `old`: display row 0
/// is the ghost row, display row 1 the buffer row 0.
fn open_with_ghost_row(
    cx: &mut TestAppContext,
    text: &str,
    old: &str,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    let hunk = ReviewHunkView {
        id: 1,
        base_rows: 0..1,
        deleted_lines: vec![old.to_string()],
        buffer_rows: 0..1,
        kind: ReviewHunkKind::Modified,
        word_diffs: None,
        lines: vec![ReviewLineView {
            base_line: Some(0),
            buffer_row: Some(0),
        }],
        from_previous_turn: false,
    };
    let review = ReviewView {
        pending_in_file: 1,
        hunks: vec![hunk],
        ..Default::default()
    };
    open_with(cx, text, EditorSettings::default(), Some(review))
}

fn place(view: &Entity<EditorView>, cx: &mut VisualTestContext, row: u32, column: u32) {
    select(view, cx, (row, column), (row, column));
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

fn keys(handle: AnyWindowHandle, cx: &mut TestAppContext, visual: &mut VisualTestContext, k: &str) {
    cx.simulate_keystrokes(handle, k);
    visual.run_until_parked();
}

fn text(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
    cx.update(|_window, cx| view.read(cx).text())
}

fn cursor(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Point {
    cx.update(|_window, cx| view.read(cx).cursor_point())
}

fn display_cursor(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> DisplayPoint {
    cx.update(|_window, cx| view.read(cx).cursor)
}

fn p(row: u32, column: u32) -> Point {
    Point::new(row, column)
}

fn clipboard(cx: &mut VisualTestContext) -> Option<ClipboardItem> {
    cx.update(|_window, cx| cx.read_from_clipboard())
}

fn clipboard_text(cx: &mut VisualTestContext) -> Option<String> {
    clipboard(cx).and_then(|item| item.text())
}

fn write_clipboard(cx: &mut VisualTestContext, item: ClipboardItem) {
    cx.update(|_window, cx| cx.write_to_clipboard(item));
}

/// The remembered whole line, if any.
fn remembered(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_window, cx| {
        cx.try_global::<LineClipboard>()
            .and_then(|line| line.0.clone())
    })
}

/// Whether the clipboard item carries the whole-line mark.
fn is_marked(item: &ClipboardItem) -> bool {
    item.entries().iter().any(|entry| match entry {
        ClipboardEntry::String(string) => string
            .metadata_json::<LineCopyMetadata>()
            .is_some_and(|metadata| metadata.whole_line),
        _ => false,
    })
}

/// Dispatches the action of the context-menu entry labelled `label`, as a
/// click on it does.
fn click_menu_entry(view: &Entity<EditorView>, cx: &mut VisualTestContext, label: &str) {
    let action = cx.update(|_window, cx| {
        view.read(cx)
            .context_menu_entries()
            .into_iter()
            .flatten()
            .find(|(entry, _)| *entry == label)
            .map(|(_, action)| action)
            .expect("the context menu has the entry")
    });
    cx.update(|window, cx| window.dispatch_action(action, cx));
    cx.run_until_parked();
}

// -- E2 criteria ---------------------------------------------------------------------

#[gpui::test]
fn ctrl_c_without_a_selection_copies_the_row_and_ctrl_v_puts_it_above(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\nc\n");
    place(&view, &mut visual, 1, 1);
    keys(handle, cx, &mut visual, "ctrl-c");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("b\n"));
    assert_eq!(text(&view, &mut visual), "a\nb\nc\n", "copy edits nothing");
    assert_eq!(cursor(&view, &mut visual), p(1, 1), "the cursor stays");

    place(&view, &mut visual, 0, 1);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(text(&view, &mut visual), "b\na\nb\nc\n");
    assert_eq!(
        cursor(&view, &mut visual),
        p(1, 1),
        "same text, same column, one row down"
    );
}

#[gpui::test]
fn the_line_goes_in_as_is_without_adapting_its_indentation(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "fn a() {\n        deep();\n}\nx\n");
    place(&view, &mut visual, 1, 3);
    keys(handle, cx, &mut visual, "ctrl-c");
    assert_eq!(
        clipboard_text(&mut visual).as_deref(),
        Some("        deep();\n")
    );
    place(&view, &mut visual, 3, 1);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(
        text(&view, &mut visual),
        "fn a() {\n        deep();\n}\n        deep();\nx\n"
    );
    assert_eq!(cursor(&view, &mut visual), p(4, 1));
}

#[gpui::test]
fn ctrl_x_without_a_selection_deletes_the_row_and_ctrl_z_brings_it_back(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\nc\n");
    place(&view, &mut visual, 1, 1);
    keys(handle, cx, &mut visual, "ctrl-x");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("b\n"));
    assert_eq!(text(&view, &mut visual), "a\nc\n");
    assert_eq!(
        cursor(&view, &mut visual),
        p(1, 1),
        "as editor::delete_line: the row below moves up"
    );
    keys(handle, cx, &mut visual, "ctrl-z");
    assert_eq!(
        text(&view, &mut visual),
        "a\nb\nc\n",
        "one undo step brings the row back"
    );
    // And the cut line pastes as a line.
    place(&view, &mut visual, 2, 0);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(text(&view, &mut visual), "a\nb\nb\nc\n");
}

#[gpui::test]
fn ctrl_x_on_the_last_row_without_a_final_newline(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb");
    place(&view, &mut visual, 1, 1);
    keys(handle, cx, &mut visual, "ctrl-x");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("b\n"));
    assert_eq!(text(&view, &mut visual), "a");
    keys(handle, cx, &mut visual, "ctrl-z");
    assert_eq!(text(&view, &mut visual), "a\nb");
}

#[gpui::test]
fn copy_and_paste_with_a_selection_are_unchanged(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "fn a() {\n    \n}\nif x {\n    y();\n}\n");
    // Copy a multi-line selection, paste it at an indented cursor: the
    // multi-line paste takes the indentation of where it lands.
    select(&view, &mut visual, (3, 0), (5, 1));
    keys(handle, cx, &mut visual, "ctrl-c");
    assert_eq!(
        clipboard_text(&mut visual).as_deref(),
        Some("if x {\n    y();\n}")
    );
    assert!(!is_marked(&clipboard(&mut visual).unwrap()));
    assert_eq!(remembered(&mut visual), None);
    place(&view, &mut visual, 1, 4);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(
        text(&view, &mut visual),
        "fn a() {\n    if x {\n        y();\n    }\n}\nif x {\n    y();\n}\n"
    );
}

#[gpui::test]
fn pasting_a_line_over_a_selection_replaces_the_selection(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\nc\n");
    place(&view, &mut visual, 1, 0);
    keys(handle, cx, &mut visual, "ctrl-c");
    select(&view, &mut visual, (0, 0), (0, 1));
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(
        text(&view, &mut visual),
        "b\n\nb\nc\n",
        "the selected `a` is replaced by the text, as always"
    );
}

#[gpui::test]
fn a_last_row_without_a_final_newline_is_copied_with_one_and_pastes_as_a_line(
    cx: &mut TestAppContext,
) {
    let (view, handle, mut visual) = open(cx, "a\nb");
    place(&view, &mut visual, 1, 0);
    keys(handle, cx, &mut visual, "ctrl-c");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("b\n"));
    place(&view, &mut visual, 0, 1);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(text(&view, &mut visual), "b\na\nb");
    assert_eq!(cursor(&view, &mut visual), p(1, 1));
}

#[gpui::test]
fn copying_or_cutting_with_a_selection_forgets_the_whole_line(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\nc\n");
    place(&view, &mut visual, 1, 0);
    keys(handle, cx, &mut visual, "ctrl-c");
    assert_eq!(remembered(&mut visual).as_deref(), Some("b\n"));

    // Copy the very same text, but with a selection: it is no longer a line.
    select(&view, &mut visual, (1, 0), (2, 0));
    keys(handle, cx, &mut visual, "ctrl-c");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("b\n"));
    assert_eq!(remembered(&mut visual), None);
    place(&view, &mut visual, 0, 1);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(
        text(&view, &mut visual),
        "ab\n\nb\nc\n",
        "a plain paste at the cursor"
    );

    // The same with a cut.
    place(&view, &mut visual, 0, 0);
    keys(handle, cx, &mut visual, "ctrl-c");
    assert_eq!(remembered(&mut visual).as_deref(), Some("ab\n"));
    select(&view, &mut visual, (0, 0), (0, 1));
    keys(handle, cx, &mut visual, "ctrl-x");
    assert_eq!(remembered(&mut visual), None);
    assert_eq!(text(&view, &mut visual), "b\n\nb\nc\n");
}

// -- the mark and the memory ------------------------------------------------------------

#[gpui::test]
fn the_copied_line_carries_the_json_mark(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\n");
    place(&view, &mut visual, 0, 0);
    keys(handle, cx, &mut visual, "ctrl-c");
    let item = clipboard(&mut visual).expect("something was copied");
    assert!(is_marked(&item), "{:?}", item.entries());
    assert_eq!(remembered(&mut visual).as_deref(), Some("a\n"));
    let ClipboardEntry::String(string) = &item.entries()[0] else {
        panic!("a string entry");
    };
    assert_eq!(string.metadata.as_deref(), Some(r#"{"whole_line":true}"#));
}

#[gpui::test]
fn a_clipboard_with_the_mark_pastes_as_a_line_even_without_the_memory(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\n");
    visual.update(|_window, cx| cx.set_global(LineClipboard(None)));
    write_clipboard(
        &mut visual,
        ClipboardItem::new_string_with_json_metadata(
            "z\n".to_string(),
            LineCopyMetadata { whole_line: true },
        ),
    );
    place(&view, &mut visual, 1, 0);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(text(&view, &mut visual), "a\nz\nb\n");
}

#[gpui::test]
fn the_remembered_text_alone_makes_a_line_and_other_text_does_not(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a\nb\n");
    // The system clipboard dropped the metadata but kept the text.
    visual.update(|_window, cx| cx.set_global(LineClipboard(Some("z\n".to_string()))));
    write_clipboard(&mut visual, ClipboardItem::new_string("z\n".to_string()));
    place(&view, &mut visual, 1, 1);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(text(&view, &mut visual), "a\nz\nb\n");
    assert_eq!(cursor(&view, &mut visual), p(2, 1));

    // Other text, even another line, is an ordinary paste at the cursor.
    write_clipboard(&mut visual, ClipboardItem::new_string("y\n".to_string()));
    place(&view, &mut visual, 0, 1);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(text(&view, &mut visual), "ay\n\nz\nb\n");
}

// -- ghost rows -----------------------------------------------------------------------------

#[gpui::test]
fn on_a_ghost_row_copy_takes_its_text_and_cut_only_copies(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_with_ghost_row(cx, "new\nrest\n", "old");
    place(&view, &mut visual, 0, 1);
    assert!(
        cx_row_is_ghost(&view, &mut visual, 0),
        "display row 0 is the ghost row"
    );
    keys(handle, cx, &mut visual, "ctrl-c");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("old\n"));
    assert!(is_marked(&clipboard(&mut visual).unwrap()));

    write_clipboard(&mut visual, ClipboardItem::new_string("other".to_string()));
    keys(handle, cx, &mut visual, "ctrl-x");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("old\n"));
    assert_eq!(
        text(&view, &mut visual),
        "new\nrest\n",
        "the ghost row is read-only: nothing is deleted"
    );
    assert_eq!(
        display_cursor(&view, &mut visual),
        DisplayPoint::new(0, 1),
        "and the cursor stays"
    );
}

#[gpui::test]
fn pasting_a_line_on_a_ghost_row_goes_above_the_next_real_row(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_with_ghost_row(cx, "new\nrest\n", "old");
    place(&view, &mut visual, 2, 1);
    keys(handle, cx, &mut visual, "ctrl-c");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("rest\n"));
    place(&view, &mut visual, 0, 2);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(
        text(&view, &mut visual),
        "rest\nnew\nrest\n",
        "the ghost text is not edited; the line lands on the real row it stands before"
    );
}

fn cx_row_is_ghost(view: &Entity<EditorView>, cx: &mut VisualTestContext, row: u32) -> bool {
    cx.update(|_window, cx| {
        matches!(
            view.read(cx).display_map.to_buffer(row),
            crate::display_map::DisplayCell::Phantom { .. }
        )
    })
}

// -- the context menu ---------------------------------------------------------------------------

#[gpui::test]
fn the_context_menu_entries_do_what_the_keys_do(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "a\nb\nc\n");
    place(&view, &mut visual, 1, 1);
    click_menu_entry(&view, &mut visual, "Copiar");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("b\n"));
    assert!(is_marked(&clipboard(&mut visual).unwrap()));

    place(&view, &mut visual, 0, 1);
    click_menu_entry(&view, &mut visual, "Pegar");
    assert_eq!(text(&view, &mut visual), "b\na\nb\nc\n");
    assert_eq!(cursor(&view, &mut visual), p(1, 1));

    place(&view, &mut visual, 3, 0);
    click_menu_entry(&view, &mut visual, "Cortar");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("c\n"));
    assert_eq!(text(&view, &mut visual), "b\na\nb\n");
}

// -- Minimal chrome (R13) --------------------------------------------------------------------------

#[gpui::test]
fn in_a_minimal_editor_copy_and_cut_without_a_selection_do_nothing(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_minimal(cx, "a\nb\nc\n");
    write_clipboard(&mut visual, ClipboardItem::new_string("keep".to_string()));
    visual.update(|_window, cx| cx.set_global(LineClipboard(None)));
    place(&view, &mut visual, 1, 1);
    keys(handle, cx, &mut visual, "ctrl-c");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("keep"));
    keys(handle, cx, &mut visual, "ctrl-x");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("keep"));
    assert_eq!(text(&view, &mut visual), "a\nb\nc\n");
    assert_eq!(remembered(&mut visual), None);
}

#[gpui::test]
fn in_a_minimal_editor_a_copied_line_pastes_at_the_cursor(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_minimal(cx, "a\nb\n");
    write_clipboard(
        &mut visual,
        ClipboardItem::new_string_with_json_metadata(
            "z\n".to_string(),
            LineCopyMetadata { whole_line: true },
        ),
    );
    visual.update(|_window, cx| cx.set_global(LineClipboard(Some("z\n".to_string()))));
    place(&view, &mut visual, 1, 0);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(text(&view, &mut visual), "a\nz\nb\n");
    assert_eq!(cursor(&view, &mut visual), p(2, 0), "an ordinary paste");
    place(&view, &mut visual, 0, 1);
    keys(handle, cx, &mut visual, "ctrl-v");
    assert_eq!(
        text(&view, &mut visual),
        "az\n\nz\nb\n",
        "inserted at the cursor, not above the row"
    );
}

#[gpui::test]
fn in_a_minimal_editor_copy_with_a_selection_still_works(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_minimal(cx, "hola\n");
    select(&view, &mut visual, (0, 0), (0, 2));
    keys(handle, cx, &mut visual, "ctrl-c");
    assert_eq!(clipboard_text(&mut visual).as_deref(), Some("ho"));
    keys(handle, cx, &mut visual, "ctrl-x");
    assert_eq!(text(&view, &mut visual), "la\n");
}
