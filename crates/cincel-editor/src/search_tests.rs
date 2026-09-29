//! Search and replace over the GPUI test harness
//! (`docs/specs/07-etapa5-productividad.md` §10.2): `Ctrl+H`, the two fields,
//! replace one / all as one undo step, regex groups, phantom rows (searched,
//! highlighted, counted, visited, never replaced) and the bar's height.

use std::ops::Range;
use std::sync::Arc;

use cincel_syntax::LanguageRegistry;
use cincel_text::Buffer;
use gpui::{AnyWindowHandle, ClipboardItem, Entity, TestAppContext, VisualTestContext};

use crate::actions::bind_default_keys;
use crate::display_map::DisplayPoint;
use crate::review::{ReviewAction, ReviewHunkKind, ReviewHunkView, ReviewLineView, ReviewView};
use crate::search::{MatchLocation, PHANTOM_REPLACE_NOTICE, SearchField};
use crate::settings::{EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::{EditorView, FrameRender, SEARCH_BAR_HEIGHT};

/// A hunk whose `deleted` lines sit before `rows.start`, paired line by line
/// with the rows (the engine's layout).
fn hunk(id: u64, deleted: &[&str], rows: Range<u32>) -> ReviewHunkView {
    let paired = deleted.len().min(rows.len());
    let mut lines: Vec<ReviewLineView> = (0..paired)
        .map(|ix| ReviewLineView {
            base_line: Some(ix as u32),
            buffer_row: Some(rows.start + ix as u32),
        })
        .collect();
    lines.extend((paired..deleted.len()).map(|ix| ReviewLineView {
        base_line: Some(ix as u32),
        buffer_row: None,
    }));
    lines.extend((paired..rows.len()).map(|ix| ReviewLineView {
        base_line: None,
        buffer_row: Some(rows.start + ix as u32),
    }));
    ReviewHunkView {
        id,
        base_rows: 0..deleted.len() as u32,
        deleted_lines: deleted.iter().map(|line| line.to_string()).collect(),
        buffer_rows: rows,
        kind: ReviewHunkKind::Modified,
        word_diffs: None,
        lines,
        from_previous_turn: false,
    }
}

fn review(hunks: Vec<ReviewHunkView>) -> ReviewView {
    ReviewView {
        pending_in_file: hunks.len(),
        hunks,
        ..Default::default()
    }
}

/// Opens a focused editor with the render probe on and `review` applied.
fn open(
    cx: &mut TestAppContext,
    text: &str,
    review: ReviewView,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    cx.update(bind_default_keys);
    let buffer = shared(Buffer::new(text));
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
    let visual = VisualTestContext::from_window(handle, cx);
    window
        .update(cx, |view, window, cx| {
            window.activate_window();
            window.focus(&view.focus_handle, cx);
            view.set_render_probe(true);
            view.set_review(review, cx);
        })
        .unwrap();
    visual.run_until_parked();
    (view, handle, visual)
}

/// Forces a repaint and returns the frame it painted.
fn frame(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> FrameRender {
    cx.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.take_render_frames();
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|_window, cx| view.update(cx, |view, _| view.take_render_frames()))
        .pop()
        .expect("a frame was painted")
}

fn text_of(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
    cx.update(|_window, cx| view.read(cx).text())
}

fn counter(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_window, cx| view.read(cx).search.status_or_counter())
}

fn current(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Option<MatchLocation> {
    cx.update(|_window, cx| view.read(cx).search.current())
}

fn keys(handle: AnyWindowHandle, cx: &mut TestAppContext, visual: &VisualTestContext, keys: &str) {
    cx.simulate_keystrokes(handle, keys);
    visual.run_until_parked();
}

fn input(handle: AnyWindowHandle, cx: &mut TestAppContext, visual: &VisualTestContext, text: &str) {
    cx.simulate_input(handle, text);
    visual.run_until_parked();
}

// -- the bar -----------------------------------------------------------------

#[gpui::test]
fn ctrl_h_opens_replace_mode_and_tab_switches_fields(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "foo\n", ReviewView::default());
    keys(handle, cx, &visual, "ctrl-h");
    let (open, replace, field, context) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (
            view.search_open,
            view.search.replace_open,
            view.search.field,
            view.key_context(),
        )
    });
    assert!(open && replace);
    assert_eq!(field, SearchField::Find, "the keyboard starts in «Buscar…»");
    assert!(context.contains("searching") && !context.contains("replacing"));

    // Tab goes to «Reemplazar…» and never reaches the file.
    keys(handle, cx, &visual, "tab");
    let (field, context) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.search.field, view.key_context())
    });
    assert_eq!(field, SearchField::Replace);
    assert!(context.contains("replacing"), "{context}");
    assert_eq!(text_of(&view, &mut visual), "foo\n");

    keys(handle, cx, &visual, "shift-tab");
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).search.field),
        SearchField::Find
    );

    // With the bar already open, Ctrl+H moves to «Reemplazar…».
    keys(handle, cx, &visual, "ctrl-h");
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).search.field),
        SearchField::Replace
    );

    // Ctrl+F goes back to the plain search bar.
    keys(handle, cx, &visual, "ctrl-f");
    let (replace, field) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.search.replace_open, view.search.field)
    });
    assert!(!replace);
    assert_eq!(field, SearchField::Find);
}

#[gpui::test]
fn tab_in_the_plain_search_bar_does_not_indent(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "foo\n", ReviewView::default());
    keys(handle, cx, &visual, "ctrl-f tab shift-tab");
    assert_eq!(text_of(&view, &mut visual), "foo\n");
}

#[gpui::test]
fn typing_backspace_and_paste_go_to_the_active_field(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "hola mundo\n", ReviewView::default());
    keys(handle, cx, &visual, "ctrl-h");
    input(handle, cx, &visual, "mun");
    keys(handle, cx, &visual, "tab");
    input(handle, cx, &visual, "tierrx");
    keys(handle, cx, &visual, "backspace");
    visual.update(|_window, cx| cx.write_to_clipboard(ClipboardItem::new_string("a!".into())));
    keys(handle, cx, &visual, "ctrl-v");
    let (query, replacement) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.search.query.clone(), view.search.replacement.clone())
    });
    assert_eq!(query, "mun");
    assert_eq!(replacement, "tierra!");

    // Pasting into «Buscar…» searches.
    keys(handle, cx, &visual, "shift-tab");
    visual.update(|_window, cx| cx.write_to_clipboard(ClipboardItem::new_string("do".into())));
    keys(handle, cx, &visual, "ctrl-v");
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).search.query.clone()),
        "mundo"
    );
    assert_eq!(counter(&view, &mut visual).as_deref(), Some("1/1"));
    assert_eq!(
        text_of(&view, &mut visual),
        "hola mundo\n",
        "the file is intact"
    );
}

/// D16: the bar takes the same 28 px strip in both modes, so the first row of
/// text sits at the same height with the plain search and with replace.
#[gpui::test]
fn the_bar_is_one_28_px_strip_in_both_modes(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "uno\ndos\n", ReviewView::default());
    let top = |visual: &mut VisualTestContext| f32::from(frame(&view, visual).bounds.origin.y);
    let closed = top(&mut visual);

    keys(handle, cx, &visual, "ctrl-f");
    let search = top(&mut visual);
    keys(handle, cx, &visual, "ctrl-h");
    let replace = top(&mut visual);
    keys(handle, cx, &visual, "ctrl-f");
    let back = top(&mut visual);
    keys(handle, cx, &visual, "escape");
    let closed_again = top(&mut visual);

    assert_eq!(
        search - closed,
        SEARCH_BAR_HEIGHT,
        "the search bar is 28 px"
    );
    assert_eq!(replace, search, "replace mode adds no row");
    assert_eq!(back, search, "toggling replace never changes the height");
    assert_eq!(closed_again, closed);
}

// -- replacing ---------------------------------------------------------------

#[gpui::test]
fn enter_replaces_the_current_match_and_ctrl_enter_all_in_one_undo(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "foo foo foo\n", ReviewView::default());
    keys(handle, cx, &visual, "ctrl-h");
    input(handle, cx, &visual, "foo");
    keys(handle, cx, &visual, "tab");
    input(handle, cx, &visual, "bar");

    keys(handle, cx, &visual, "enter");
    assert_eq!(
        text_of(&view, &mut visual),
        "bar foo foo\n",
        "only the current one"
    );
    assert_eq!(
        current(&view, &mut visual),
        Some(MatchLocation::Buffer(4..7))
    );
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).selected_text()),
        "foo",
        "the next match is selected"
    );
    assert_eq!(counter(&view, &mut visual).as_deref(), Some("1/2"));

    // Undo that one first, so the next step starts from the original text.
    keys(handle, cx, &visual, "ctrl-z");
    assert_eq!(text_of(&view, &mut visual), "foo foo foo\n");

    keys(handle, cx, &visual, "ctrl-enter");
    assert_eq!(text_of(&view, &mut visual), "bar bar bar\n");
    assert_eq!(
        counter(&view, &mut visual).as_deref(),
        Some("3 reemplazadas")
    );
    keys(handle, cx, &visual, "ctrl-z");
    assert_eq!(
        text_of(&view, &mut visual),
        "foo foo foo\n",
        "one Ctrl+Z undoes the whole replace-all"
    );
}

#[gpui::test]
fn replacing_follows_the_case_toggle(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "Foo foo FOO\n", ReviewView::default());
    keys(handle, cx, &visual, "ctrl-h");
    input(handle, cx, &visual, "foo");
    keys(handle, cx, &visual, "alt-c tab");
    input(handle, cx, &visual, "x");
    keys(handle, cx, &visual, "ctrl-enter");
    assert_eq!(text_of(&view, &mut visual), "Foo x FOO\n");
    keys(handle, cx, &visual, "ctrl-z alt-c ctrl-enter");
    assert_eq!(text_of(&view, &mut visual), "x x x\n");
}

#[gpui::test]
fn a_regex_replacement_expands_groups(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "ana@ casa, luis@ campo\n", ReviewView::default());
    keys(handle, cx, &visual, "ctrl-h alt-r");
    input(handle, cx, &visual, r"(\w+)@");
    keys(handle, cx, &visual, "tab");
    input(handle, cx, &visual, "$1 en");
    keys(handle, cx, &visual, "enter");
    assert_eq!(text_of(&view, &mut visual), "ana en casa, luis@ campo\n");
    keys(handle, cx, &visual, "ctrl-enter");
    assert_eq!(text_of(&view, &mut visual), "ana en casa, luis en campo\n");
}

#[gpui::test]
fn the_replace_buttons_do_the_same_as_the_keys(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "a a a\n", ReviewView::default());
    keys(handle, cx, &visual, "ctrl-h");
    input(handle, cx, &visual, "a");
    keys(handle, cx, &visual, "tab");
    input(handle, cx, &visual, "b");
    visual.update(|_window, cx| view.update(cx, |view, cx| view.replace_next(cx)));
    assert_eq!(text_of(&view, &mut visual), "b a a\n");
    visual.update(|_window, cx| view.update(cx, |view, cx| view.replace_all(cx)));
    assert_eq!(text_of(&view, &mut visual), "b b b\n");
}

// -- phantom rows --------------------------------------------------------------

/// Row 0 of the buffer replaced `let viejo = 1;`; `viejo real` is a real row.
/// Display rows: 0 = phantom `let viejo = 1;`, 1 = `let nuevo = 1;`,
/// 2 = `viejo real`, 3 = empty.
const MIXED: &str = "let nuevo = 1;\nviejo real\n";

#[gpui::test]
fn search_finds_highlights_and_counts_phantom_rows(cx: &mut TestAppContext) {
    let (view, handle, mut visual) =
        open(cx, MIXED, review(vec![hunk(1, &["let viejo = 1;"], 0..1)]));
    keys(handle, cx, &visual, "ctrl-f");
    input(handle, cx, &visual, "viejo");

    let matches = visual.update(|_window, cx| view.read(cx).search.matches().to_vec());
    assert_eq!(
        matches,
        vec![
            MatchLocation::Phantom {
                hunk_ix: 0,
                line_ix: 0,
                range: 4..9,
            },
            MatchLocation::Buffer(15..20),
        ]
    );
    assert_eq!(
        counter(&view, &mut visual).as_deref(),
        Some("1/2 (1 en líneas quitadas)")
    );
    // The phantom match is selected in display coordinates.
    let (anchor, cursor, selected) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.selection_anchor, view.cursor, view.selected_text())
    });
    assert_eq!(
        (anchor, cursor),
        (DisplayPoint::new(0, 4), DisplayPoint::new(0, 9))
    );
    assert_eq!(selected, "viejo");

    // Both rows are highlighted; the phantom one is the current match.
    let frame = frame(&view, &mut visual);
    let row = |display_row: u32| {
        frame
            .rows
            .iter()
            .find(|row| row.display_row == display_row)
            .expect("row painted")
    };
    assert_eq!(row(0).search_matches, 1);
    assert!(row(0).current_search_match);
    assert_eq!(row(2).search_matches, 1);
    assert!(!row(2).current_search_match);
    assert_eq!(row(1).search_matches, 0);
}

#[gpui::test]
fn next_and_previous_walk_phantom_rows_in_screen_order(cx: &mut TestAppContext) {
    // Display rows: 0 `foo a`, 1 phantom `foo viejo`, 2 phantom `otro foo`,
    // 3 `foo b`, 4 `foo c`, 5 phantom `foo final`, 6 `fin`, 7 empty.
    let (view, handle, mut visual) = open(
        cx,
        "foo a\nfoo b\nfoo c\nfin\n",
        review(vec![
            hunk(1, &["foo viejo", "otro foo"], 1..2),
            hunk(2, &["foo final"], 3..4),
        ]),
    );
    keys(handle, cx, &visual, "ctrl-f");
    input(handle, cx, &visual, "foo");
    let mut rows = Vec::new();
    for _ in 0..6 {
        rows.push(visual.update(|_window, cx| view.read(cx).cursor.row));
        keys(handle, cx, &visual, "enter");
    }
    assert_eq!(rows, vec![0, 1, 2, 3, 4, 5], "Enter follows the screen");
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).cursor.row),
        0,
        "and wraps around"
    );
    keys(handle, cx, &visual, "shift-enter");
    assert_eq!(visual.update(|_window, cx| view.read(cx).cursor.row), 5);
    keys(handle, cx, &visual, "shift-enter");
    assert_eq!(visual.update(|_window, cx| view.read(cx).cursor.row), 4);
    assert_eq!(
        counter(&view, &mut visual).as_deref(),
        Some("5/6 (3 en líneas quitadas)")
    );
}

#[gpui::test]
fn replace_on_a_phantom_match_skips_to_the_next_real_one(cx: &mut TestAppContext) {
    let (view, handle, mut visual) =
        open(cx, MIXED, review(vec![hunk(1, &["let viejo = 1;"], 0..1)]));
    keys(handle, cx, &visual, "ctrl-h");
    input(handle, cx, &visual, "viejo");
    keys(handle, cx, &visual, "tab");
    input(handle, cx, &visual, "nuevo");
    assert!(current(&view, &mut visual).unwrap().is_phantom());

    keys(handle, cx, &visual, "enter");
    assert_eq!(text_of(&view, &mut visual), MIXED, "nothing was replaced");
    assert_eq!(
        counter(&view, &mut visual).as_deref(),
        Some(PHANTOM_REPLACE_NOTICE)
    );
    assert_eq!(
        current(&view, &mut visual),
        Some(MatchLocation::Buffer(15..20))
    );

    keys(handle, cx, &visual, "enter");
    assert_eq!(text_of(&view, &mut visual), "let nuevo = 1;\nnuevo real\n");
    // Only the phantom match is left, and replacing it still does nothing.
    keys(handle, cx, &visual, "enter");
    assert_eq!(text_of(&view, &mut visual), "let nuevo = 1;\nnuevo real\n");
    assert!(current(&view, &mut visual).unwrap().is_phantom());
    let deleted = visual.update(|_window, cx| view.read(cx).diff().hunks()[0].deleted_text.clone());
    assert_eq!(deleted, vec!["let viejo = 1;".to_string()]);
}

#[gpui::test]
fn replace_all_leaves_phantom_rows_alone_and_says_so(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(
        cx,
        "viejo 1\nlet nuevo = 1;\nviejo 2\n",
        review(vec![hunk(7, &["let viejo = 1;", "viejo viejo"], 1..2)]),
    );
    keys(handle, cx, &visual, "ctrl-h");
    input(handle, cx, &visual, "viejo");
    assert_eq!(
        counter(&view, &mut visual).as_deref(),
        Some("1/5 (3 en líneas quitadas)")
    );
    keys(handle, cx, &visual, "tab");
    input(handle, cx, &visual, "nuevo");
    keys(handle, cx, &visual, "ctrl-enter");

    assert_eq!(
        text_of(&view, &mut visual),
        "nuevo 1\nlet nuevo = 1;\nnuevo 2\n"
    );
    assert_eq!(
        counter(&view, &mut visual).as_deref(),
        Some("2 reemplazadas · 3 en líneas quitadas por el agente sin tocar")
    );
    // What "Rechazar" restores is untouched: the review's base text and the
    // phantom rows the editor paints.
    let (base, phantom, matches) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (
            view.review().hunks[0].deleted_lines.clone(),
            view.diff().hunks()[0].deleted_text.clone(),
            view.search.matches().len(),
        )
    });
    let expected = vec!["let viejo = 1;".to_string(), "viejo viejo".to_string()];
    assert_eq!(base, expected);
    assert_eq!(phantom, expected);
    assert_eq!(matches, 3, "the phantom matches are still found");
}

#[gpui::test]
fn replacing_inside_a_pending_hunk_keeps_it_pending_and_reject_restores_the_base(
    cx: &mut TestAppContext,
) {
    let (view, handle, mut visual) = open(
        cx,
        "uno\nlet nuevo = 1;\ntres\n",
        review(vec![hunk(4, &["let viejo = 1;"], 1..2)]),
    );
    let events = crate::review_tests::record(&view, &mut visual);
    keys(handle, cx, &visual, "ctrl-h");
    input(handle, cx, &visual, "nuevo");
    keys(handle, cx, &visual, "tab");
    // A replacement that adds a row: the hunk grows with it, like any edit.
    input(handle, cx, &visual, "a");
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.search.replacement.push('\n');
            view.search.replacement.push('b');
            cx.notify();
        })
    });
    keys(handle, cx, &visual, "ctrl-enter");
    assert_eq!(text_of(&view, &mut visual), "uno\nlet a\nb = 1;\ntres\n");
    let (rows, pending, base) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (
            view.review().hunks[0].buffer_rows.clone(),
            view.review().hunks.len(),
            view.review().hunks[0].deleted_lines.clone(),
        )
    });
    assert_eq!(pending, 1, "the hunk is still pending");
    assert_eq!(rows, 1..3, "it grew with the replacement");
    assert_eq!(base, vec!["let viejo = 1;".to_string()]);

    // Rejecting it asks the host to restore the base text, which replacing
    // never touched.
    keys(handle, cx, &visual, "escape");
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.cursor = DisplayPoint::new(2, 0);
            view.selection_anchor = view.cursor;
            cx.notify();
        })
    });
    visual.run_until_parked();
    keys(handle, cx, &visual, "ctrl-backspace");
    assert_eq!(events.borrow().as_slice(), &[ReviewAction::RejectHunk(4)]);
    // The host's rejection: the hunk's rows go back to its base lines.
    let restored = visual.update(|_window, cx| {
        let view = view.read(cx);
        let hunk = &view.review().hunks[0];
        let text = view.text();
        let lines: Vec<&str> = text.split('\n').collect();
        let mut restored: Vec<String> = lines[..hunk.buffer_rows.start as usize]
            .iter()
            .map(|line| line.to_string())
            .collect();
        restored.extend(hunk.deleted_lines.iter().cloned());
        restored.extend(
            lines[hunk.buffer_rows.end as usize..]
                .iter()
                .map(|line| line.to_string()),
        );
        restored.join("\n")
    });
    assert_eq!(restored, "uno\nlet viejo = 1;\ntres\n");
}
