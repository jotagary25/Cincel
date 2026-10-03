//! Occurrences of the word under the cursor (`docs/specs/10-etapa7-ronda2.md`
//! §7.1, E1): the pure search ([`find_occurrences`]) and, over the GPUI test
//! harness, what the element lays out frame by frame: whole words, one-row
//! selections, the 150 ms delay, the search bar winning, `Minimal` and no
//! timer left at rest (M8).

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use cincel_syntax::LanguageRegistry;
use cincel_text::{Buffer, Point};
use gpui::{AnyWindowHandle, Entity, TestAppContext, VisualTestContext};

use crate::actions::bind_default_keys;
use crate::search::{MAX_OCCURRENCE_BYTES, OccurrenceQuery, find_occurrences};
use crate::settings::{EditorChrome, EditorSettings, shared};
use crate::theme::{EditorTheme, OCCURRENCE_ALPHA};
use crate::view::{BLINK_IDLE, EditorView, OCCURRENCE_DELAY};

const SAMPLE: &str = "let total = a + total_b; total\n";

// -- find_occurrences ----------------------------------------------------------

fn word(text: &str, own: Range<usize>) -> OccurrenceQuery {
    OccurrenceQuery {
        text: text.to_string(),
        whole_word_start: true,
        whole_word_end: true,
        own,
    }
}

#[test]
fn whole_words_only_and_the_own_occurrence_is_left_out() {
    let query = word("total", 4..9);
    assert_eq!(find_occurrences(SAMPLE, 0, &query), vec![25..30]);
    // Without its own range every whole word shows, `total_b` never does.
    let query = word("total", 100..105);
    assert_eq!(find_occurrences(SAMPLE, 0, &query), vec![4..9, 25..30]);
}

#[test]
fn ranges_are_offset_by_the_base() {
    let query = word("total", 0..0);
    assert_eq!(
        find_occurrences(SAMPLE, 1000, &query),
        vec![1004..1009, 1025..1030]
    );
}

#[test]
fn occurrences_are_case_sensitive() {
    let query = word("Total", 0..0);
    assert!(find_occurrences(SAMPLE, 0, &query).is_empty());
    let query = word("total", 0..0);
    assert_eq!(
        find_occurrences("Total total TOTAL", 0, &query),
        vec![6..11]
    );
}

#[test]
fn unicode_letters_are_word_characters() {
    // `añoñ` contains `año` but is another word; `ñ` and `é` are letters.
    let text = "año añoñ año\nmamá mamáé";
    let query = word("año", 0..4);
    assert_eq!(find_occurrences(text, 0, &query), vec![12..16]);
    let query = word("mamá", 0..0);
    assert_eq!(find_occurrences(text, 0, &query), vec![17..22]);
}

#[test]
fn a_selection_asks_for_the_boundary_only_on_its_word_sides() {
    // `a + ` starts with a word character and ends with a space.
    let query = OccurrenceQuery::for_selection("a + ", 0..4).unwrap();
    assert!(query.whole_word_start && !query.whole_word_end);
    let text = "a + b; ba + c; a + d";
    assert_eq!(find_occurrences(text, 0, &query), vec![15..19]);

    // ` + ` has no word character on either side: plain substring.
    let query = OccurrenceQuery::for_selection(" + ", 1..4).unwrap();
    assert_eq!(find_occurrences(text, 0, &query), vec![9..12, 16..19]);
}

#[test]
fn overlapping_candidates_are_not_lost() {
    // The first candidate (1..4) is rejected (`x` before it); the one that
    // overlaps it (3..6) is still found.
    let query = OccurrenceQuery::for_selection("a a", 100..103).unwrap();
    assert_eq!(find_occurrences("xa a a", 0, &query), vec![3..6]);
    // Accepted occurrences do not overlap.
    let query = OccurrenceQuery::for_selection("--", 100..102).unwrap();
    assert_eq!(find_occurrences("-----", 0, &query), vec![0..2, 2..4]);
}

#[test]
fn the_query_for_the_cursor() {
    let line = "let total = a + total_b; total";
    // Inside, at the start and right at the end of `total`.
    for column in [4, 6, 9] {
        let query = OccurrenceQuery::for_cursor(line, 10, column).unwrap();
        assert_eq!(query.text, "total");
        assert_eq!(query.own, 14..19);
        assert!(query.whole_word_start && query.whole_word_end);
    }
    // Between `=` and a space: no word.
    assert!(OccurrenceQuery::for_cursor(line, 0, 11).is_none());
    assert!(OccurrenceQuery::for_cursor("", 0, 0).is_none());
    // A multibyte word.
    let query = OccurrenceQuery::for_cursor("x mamá", 0, 7).unwrap();
    assert_eq!(query.text, "mamá");
}

#[test]
fn what_a_selection_cannot_ask_for() {
    assert!(OccurrenceQuery::for_selection("", 0..0).is_none());
    assert!(OccurrenceQuery::for_selection("   ", 0..3).is_none());
    assert!(OccurrenceQuery::for_selection("a\nb", 0..3).is_none());
    let long = "x".repeat(MAX_OCCURRENCE_BYTES + 1);
    assert!(OccurrenceQuery::for_selection(&long, 0..long.len()).is_none());
    let longest = "x".repeat(MAX_OCCURRENCE_BYTES);
    assert!(OccurrenceQuery::for_selection(&longest, 0..longest.len()).is_some());
}

#[test]
fn the_alpha_is_half_the_selection() {
    assert_eq!(OCCURRENCE_ALPHA, 0.5);
    assert_eq!(OCCURRENCE_DELAY, Duration::from_millis(150));
}

// -- the editor ----------------------------------------------------------------

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
        .update(cx, |view, window, cx| {
            window.activate_window();
            window.focus(&view.focus_handle, cx);
        })
        .unwrap();
    visual.run_until_parked();
    (view, handle, visual)
}

/// Repaints and returns the occurrences the element laid out on that frame.
fn painted(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Vec<Range<usize>> {
    cx.update(|_window, cx| view.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    cx.update(|_window, cx| view.read(cx).occurrence_ranges_for_test())
}

fn wait(cx: &mut VisualTestContext, duration: Duration) {
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
}

fn cursor_at(view: &Entity<EditorView>, cx: &mut VisualTestContext, row: u32, column: u32) {
    cx.update(|_window, cx| {
        view.update(cx, |view, cx| view.set_cursor(Point::new(row, column), cx))
    });
    cx.run_until_parked();
}

fn keys(handle: AnyWindowHandle, cx: &mut TestAppContext, visual: &VisualTestContext, keys: &str) {
    cx.simulate_keystrokes(handle, keys);
    visual.run_until_parked();
}

#[gpui::test]
fn the_word_under_the_cursor_marks_its_other_whole_occurrences(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, SAMPLE, EditorSettings::default());
    cursor_at(&view, &mut visual, 0, 6);
    wait(&mut visual, OCCURRENCE_DELAY);
    // `total` at 25..30, not `total_b` (16..23) and not its own (4..9).
    assert_eq!(painted(&view, &mut visual), vec![25..30]);

    // Right at the end of the last `total`: the first one shows.
    cursor_at(&view, &mut visual, 0, 30);
    wait(&mut visual, OCCURRENCE_DELAY);
    assert_eq!(painted(&view, &mut visual), vec![4..9]);

    // On `total_b`, a word of its own: nothing else matches it.
    cursor_at(&view, &mut visual, 0, 18);
    wait(&mut visual, OCCURRENCE_DELAY);
    assert!(painted(&view, &mut visual).is_empty());
}

#[gpui::test]
fn a_one_row_selection_marks_its_exact_occurrences(cx: &mut TestAppContext) {
    let text = "x = a + b;\ny = a + c;\nz = ba + d;\n";
    let (view, handle, mut visual) = open(cx, text, EditorSettings::default());
    // Select `a + ` on the first row.
    cursor_at(&view, &mut visual, 0, 4);
    keys(
        handle,
        cx,
        &visual,
        "shift-right shift-right shift-right shift-right",
    );
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).selected_text()),
        "a + "
    );
    wait(&mut visual, OCCURRENCE_DELAY);
    // Row 1's `a + ` (15..19); `ba + ` fails the word boundary at its start.
    assert_eq!(painted(&view, &mut visual), vec![15..19]);

    // A selection over two rows marks nothing.
    keys(handle, cx, &visual, "shift-down");
    wait(&mut visual, OCCURRENCE_DELAY);
    assert!(painted(&view, &mut visual).is_empty());
}

#[gpui::test]
fn the_marks_wait_150_ms_and_a_move_clears_them_in_the_same_frame(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, SAMPLE, EditorSettings::default());
    cursor_at(&view, &mut visual, 0, 6);
    wait(&mut visual, OCCURRENCE_DELAY - Duration::from_millis(1));
    assert!(
        painted(&view, &mut visual).is_empty(),
        "nothing before 150 ms"
    );
    wait(&mut visual, Duration::from_millis(1));
    assert_eq!(painted(&view, &mut visual), vec![25..30], "shown at 150 ms");

    // The frame right after the move has none, and the wait starts again.
    cx.simulate_keystrokes(handle, "right");
    assert!(painted(&view, &mut visual).is_empty());
    wait(&mut visual, OCCURRENCE_DELAY - Duration::from_millis(1));
    assert!(painted(&view, &mut visual).is_empty());
    wait(&mut visual, Duration::from_millis(1));
    assert_eq!(painted(&view, &mut visual), vec![25..30]);

    // Typing clears them too (the text changed under them).
    cx.simulate_input(handle, "x");
    assert!(painted(&view, &mut visual).is_empty());
}

#[gpui::test]
fn the_search_bar_with_a_query_hides_them(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, SAMPLE, EditorSettings::default());
    cursor_at(&view, &mut visual, 0, 6);
    wait(&mut visual, OCCURRENCE_DELAY);
    assert_eq!(painted(&view, &mut visual), vec![25..30]);

    keys(handle, cx, &visual, "ctrl-f");
    cx.simulate_input(handle, "a");
    visual.run_until_parked();
    wait(&mut visual, OCCURRENCE_DELAY);
    assert!(
        !visual.update(|_window, cx| view.read(cx).search.query.is_empty()),
        "the bar has a query"
    );
    assert!(painted(&view, &mut visual).is_empty(), "the search wins");
}

#[gpui::test]
fn no_timer_is_left_at_rest(cx: &mut TestAppContext) {
    let settings = EditorSettings {
        cursor_blink: false,
        ..EditorSettings::default()
    };
    let (view, handle, mut visual) = open(cx, SAMPLE, settings);
    cursor_at(&view, &mut visual, 0, 6);
    cx.simulate_input(handle, "x");
    visual.run_until_parked();
    assert!(visual.update(|_window, cx| view.read(cx).is_occurrence_timer_pending()));

    wait(&mut visual, Duration::from_secs(1));
    let (occurrence, blink) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.is_occurrence_timer_pending(), view.is_blinking())
    });
    assert!(!occurrence, "the occurrence timer fired once and ended");
    assert!(!blink, "and no blink task runs");
}

#[gpui::test]
fn no_timer_is_left_at_rest_with_the_blink_on(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, SAMPLE, EditorSettings::default());
    cursor_at(&view, &mut visual, 0, 6);
    cx.simulate_input(handle, "x");
    visual.run_until_parked();
    wait(&mut visual, Duration::from_secs(1));
    assert!(!visual.update(|_window, cx| view.read(cx).is_occurrence_timer_pending()));
    wait(&mut visual, BLINK_IDLE + Duration::from_secs(2));
    let (occurrence, blink) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.is_occurrence_timer_pending(), view.is_blinking())
    });
    assert!(!occurrence && !blink, "an editor at rest owns no timer");
}

#[gpui::test]
fn minimal_never_marks(cx: &mut TestAppContext) {
    let settings = EditorSettings {
        chrome: EditorChrome::Minimal,
        ..EditorSettings::default()
    };
    let (view, _handle, mut visual) = open(cx, SAMPLE, settings);
    cursor_at(&view, &mut visual, 0, 6);
    assert!(
        !visual.update(|_window, cx| view.read(cx).is_occurrence_timer_pending()),
        "Minimal does not even start the timer"
    );
    wait(&mut visual, OCCURRENCE_DELAY * 2);
    assert!(painted(&view, &mut visual).is_empty());
}

#[gpui::test]
fn a_read_only_buffer_never_marks(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, SAMPLE, EditorSettings::default());
    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_read_only(true, cx)));
    cursor_at(&view, &mut visual, 0, 6);
    wait(&mut visual, OCCURRENCE_DELAY * 2);
    assert!(painted(&view, &mut visual).is_empty());
}

#[gpui::test]
fn only_visible_rows_are_marked(cx: &mut TestAppContext) {
    // 400 rows of `total`: only the laid-out ones (plus the overscan) are
    // searched, never the whole file.
    let text = "total\n".repeat(400);
    let (view, _handle, mut visual) = open(cx, &text, EditorSettings::default());
    cursor_at(&view, &mut visual, 0, 2);
    wait(&mut visual, OCCURRENCE_DELAY);
    let marks = painted(&view, &mut visual);
    assert!(!marks.is_empty());
    assert!(marks.len() < 399, "only the visible rows: {}", marks.len());
    assert!(!marks.contains(&(0..5)), "not its own occurrence");
    assert_eq!(marks[0], 6..11);
}
