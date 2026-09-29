//! Review UI tests over the GPUI test harness (`02-visual.md` §6): the rows
//! a `ReviewView` produces, word diffs, pill, `+`/`−` icons, floating bar,
//! keys, the turn-active state and what `set_review` keeps in its caches.

// One-range vectors are exactly what a hunk with one word diff looks like.
#![allow(clippy::single_range_in_vec_init)]

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use cincel_syntax::LanguageRegistry;
use cincel_text::Buffer;
use gpui::{
    AnyWindowHandle, CursorStyle, Entity, Modifiers, TestAppContext, VisualTestContext, point, px,
};

use crate::actions::bind_default_keys;
use crate::display_map::{DisplayPoint, RowKind};
use crate::element::BAR_MARGIN;
use crate::review::{
    ReviewAction, ReviewHunkKind, ReviewHunkView, ReviewLineView, ReviewView, ReviewWordDiffs,
};
use crate::settings::{EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::{EditorEvent, EditorView, FrameRender};

/// Collects every `EditorEvent::Review` the view emits.
pub(crate) fn record(
    view: &Entity<EditorView>,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<ReviewAction>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_window, cx| {
        cx.subscribe(view, move |_, event: &EditorEvent, _| {
            if let EditorEvent::Review(action) = event {
                sink.borrow_mut().push(*action);
            }
        })
        .detach();
    });
    events
}

/// A hunk: `deleted` lines spliced before `rows.start`, paired line by line
/// with the rows, extras after the pairs (the engine's layout).
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
    let kind = match (deleted.is_empty(), rows.is_empty()) {
        (false, false) => ReviewHunkKind::Modified,
        (false, true) => ReviewHunkKind::Deleted,
        _ => ReviewHunkKind::Added,
    };
    ReviewHunkView {
        id,
        base_rows: 0..deleted.len() as u32,
        deleted_lines: deleted.iter().map(|line| line.to_string()).collect(),
        buffer_rows: rows,
        kind,
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
    settings: EditorSettings,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    cx.update(bind_default_keys);
    let buffer = shared(Buffer::new(text));
    let registry = Arc::new(LanguageRegistry::new());
    let language = registry.language("rust");
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
            // The floating bar only shows in the active window.
            window.activate_window();
            window.focus(&view.focus_handle, cx);
            view.set_render_probe(true);
            view.set_review(review, cx);
        })
        .unwrap();
    visual.run_until_parked();
    (view, handle, visual)
}

fn open_default(
    cx: &mut TestAppContext,
    text: &str,
    review: ReviewView,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    open(cx, text, review, EditorSettings::default())
}

/// The settings with `jump_to_next_on_decide` on (it is off by default).
fn jumping() -> EditorSettings {
    EditorSettings {
        jump_to_next_on_decide: true,
        ..Default::default()
    }
}

fn set_review(view: &Entity<EditorView>, cx: &mut VisualTestContext, review: ReviewView) {
    cx.update(|_window, cx| view.update(cx, |view, cx| view.set_review(review, cx)));
    cx.run_until_parked();
}

fn set_cursor(view: &Entity<EditorView>, cx: &mut VisualTestContext, point: DisplayPoint) {
    cx.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.cursor = view.clip_point(point);
            view.selection_anchor = view.cursor;
            cx.notify();
        })
    });
    cx.run_until_parked();
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

fn kinds(frame: &FrameRender) -> Vec<RowKind> {
    frame.rows.iter().map(|row| row.kind).collect()
}

/// `(line height, text origin x, element bounds)` of the last layout.
fn geometry(
    view: &Entity<EditorView>,
    cx: &mut VisualTestContext,
) -> (f32, f32, gpui::Bounds<gpui::Pixels>) {
    cx.update(|_window, cx| {
        let view = view.read(cx);
        let layout = view.layout.as_ref().expect("painted");
        (
            f32::from(layout.line_height),
            f32::from(layout.text_origin_x),
            layout.bounds,
        )
    })
}

fn hover(cx: &mut VisualTestContext, x: f32, y: f32) {
    cx.simulate_mouse_move(point(px(x), px(y)), None, Modifiers::default());
    cx.run_until_parked();
}

fn click(cx: &mut VisualTestContext, x: f32, y: f32) {
    hover(cx, x, y);
    cx.simulate_click(point(px(x), px(y)), Modifiers::default());
    cx.run_until_parked();
}

/// Centre x of the `+` and `−` icons of the row at `y`, as the frame painted
/// them after hovering the row.
fn icon_centers(view: &Entity<EditorView>, cx: &mut VisualTestContext, y: f32) -> (f32, f32) {
    hover(cx, 300., y);
    let (plus, minus) = frame(view, cx)
        .review
        .line_icon_bounds
        .expect("the icons were painted");
    (f32::from(plus.center().x), f32::from(minus.center().x))
}

const TEXT: &str = "a\nb\nc\nd\ne\nf\ng\nh\n";

/// Two hunks: `x`,`y` replaced by buffer row 1 ("b"); `z` deleted before row 5.
fn two_hunks() -> ReviewView {
    review(vec![hunk(10, &["x", "y"], 1..2), hunk(20, &["z"], 5..5)])
}

// -- rows ---------------------------------------------------------------------

#[gpui::test]
fn set_review_splices_phantom_rows_and_marks_added_rows(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let frame = frame(&view, &mut visual);
    let kinds = kinds(&frame);
    // a, x, y, b(added), c, d, e, z, f, g, h, "".
    assert_eq!(
        &kinds[..9],
        &[
            RowKind::Normal,
            RowKind::Phantom(0),
            RowKind::Phantom(0),
            RowKind::Added(0),
            RowKind::Normal,
            RowKind::Normal,
            RowKind::Normal,
            RowKind::Phantom(1),
            RowKind::Normal,
        ]
    );
    let sources: Vec<String> = visual.update(|_window, cx| {
        let view = view.read(cx);
        (0..9).map(|row| view.display_row_source(row)).collect()
    });
    assert_eq!(sources, ["a", "x", "y", "b", "c", "d", "e", "z", "f"]);
}

#[gpui::test]
fn phantom_rows_have_no_line_number_and_real_rows_do(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let frame = frame(&view, &mut visual);
    for row in &frame.rows {
        let phantom = matches!(row.kind, RowKind::Phantom(_));
        assert_eq!(row.line_number, !phantom, "display row {}", row.display_row);
    }
}

#[gpui::test]
fn word_diffs_are_painted_on_both_sides(cx: &mut TestAppContext) {
    let mut modified = hunk(1, &["let x = 1;"], 1..2);
    modified.word_diffs = Some(ReviewWordDiffs {
        deleted: vec![8..9],
        added: vec![8..9],
    });
    let (view, _handle, mut visual) =
        open_default(cx, "fn a() {}\nlet x = 2;\n", review(vec![modified]));
    let frame = frame(&view, &mut visual);
    let words = |row: u32| frame.row(row).expect("painted").word_diffs;
    assert_eq!(words(1), 1, "the deleted `1` on the phantom row");
    assert_eq!(words(2), 1, "the added `2` on the buffer row");
    assert_eq!(words(0), 0);
}

#[gpui::test]
fn no_word_diffs_without_them_in_the_view(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(
        cx,
        "fn a() {}\nlet x = 2;\n",
        review(vec![hunk(1, &["let x = 1;"], 1..2)]),
    );
    let frame = frame(&view, &mut visual);
    assert!(frame.rows.iter().all(|row| row.word_diffs == 0));
}

#[gpui::test]
fn added_word_ranges_are_relative_to_the_first_row_of_the_hunk(cx: &mut TestAppContext) {
    // Buffer rows 1..3 are "uno" and "dos": the text of the hunk is
    // "uno\ndos", so 4..7 is the whole of "dos".
    let mut added = hunk(1, &["una"], 1..3);
    added.word_diffs = Some(ReviewWordDiffs {
        deleted: vec![],
        added: vec![4..7],
    });
    let (view, _handle, mut visual) =
        open_default(cx, "cero\nuno\ndos\ntres\n", review(vec![added]));
    let frame = frame(&view, &mut visual);
    // Display: cero, una (phantom), uno, dos, tres.
    assert_eq!(frame.row(2).unwrap().word_diffs, 0);
    assert_eq!(frame.row(3).unwrap().word_diffs, 1);
}

// -- pill -----------------------------------------------------------------------

#[gpui::test]
fn the_pill_shows_on_the_hunk_with_the_cursor_only(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    assert!(
        frame(&view, &mut visual).review.pills.is_empty(),
        "no pill while the cursor and the mouse are elsewhere"
    );
    set_cursor(&view, &mut visual, DisplayPoint::new(2, 0));
    let pills = frame(&view, &mut visual).review.pills;
    assert_eq!(pills.len(), 1);
    assert_eq!(pills[0].hunk, 10);
    assert!(pills[0].enabled);
}

#[gpui::test]
fn hovering_a_hunk_shows_its_pill_too(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    let (line_height, _, _) = geometry(&view, &mut visual);
    // Display row 7 is the phantom row of the second hunk.
    hover(&mut visual, 300., line_height * 7.5);
    let pills: Vec<u64> = frame(&view, &mut visual)
        .review
        .pills
        .iter()
        .map(|pill| pill.hunk)
        .collect();
    assert_eq!(pills, vec![10, 20]);
}

#[gpui::test]
fn a_previous_turn_hunk_shows_the_clock_and_its_tooltip(cx: &mut TestAppContext) {
    let mut old = hunk(5, &["viejo"], 1..2);
    old.from_previous_turn = true;
    let (view, _handle, mut visual) = open_default(cx, TEXT, review(vec![old]));
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    let frame_before = frame(&view, &mut visual);
    assert!(frame_before.review.pills[0].previous_turn);
    assert_eq!(frame_before.review.tooltip, None);

    visual.update(|_window, cx| view.update(cx, |view, _| view.hover_clock = Some(5)));
    let tooltip = frame(&view, &mut visual).review.tooltip;
    assert_eq!(tooltip.as_deref(), Some("Turno anterior"));
}

// -- line icons -----------------------------------------------------------------

#[gpui::test]
fn hovering_a_hunk_row_shows_the_line_icons(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let (line_height, _, _) = geometry(&view, &mut visual);
    hover(&mut visual, 300., line_height * 0.5);
    assert_eq!(frame(&view, &mut visual).review.line_icons, None);
    // Display row 2 is `y`, the second deleted line (index 1 in `lines`).
    hover(&mut visual, 300., line_height * 2.5);
    assert_eq!(frame(&view, &mut visual).review.line_icons, Some((10, 1)));
    // Display row 3 is buffer row 1, paired with `x` (index 0).
    hover(&mut visual, 300., line_height * 3.5);
    assert_eq!(frame(&view, &mut visual).review.line_icons, Some((10, 0)));
}

#[gpui::test]
fn clicking_plus_emits_accept_line_with_its_index(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let events = record(&view, &mut visual);
    let (line_height, _, _) = geometry(&view, &mut visual);
    let (plus, _) = icon_centers(&view, &mut visual, line_height * 2.5);
    click(&mut visual, plus, line_height * 2.5);
    assert_eq!(
        *events.borrow(),
        vec![ReviewAction::AcceptLine { hunk: 10, line: 1 }]
    );
    let (cursor, text) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.cursor, view.text())
    });
    assert_eq!(
        cursor,
        DisplayPoint::new(0, 0),
        "the icon does not move the cursor"
    );
    assert_eq!(text, TEXT);
}

#[gpui::test]
fn clicking_minus_emits_reject_line(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let events = record(&view, &mut visual);
    let (line_height, _, _) = geometry(&view, &mut visual);
    let (_, minus) = icon_centers(&view, &mut visual, line_height * 7.5);
    click(&mut visual, minus, line_height * 7.5);
    assert_eq!(
        *events.borrow(),
        vec![ReviewAction::RejectLine { hunk: 20, line: 0 }]
    );
}

#[gpui::test]
fn line_keys_act_on_the_cursor_row(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let events = record(&view, &mut visual);
    set_cursor(&view, &mut visual, DisplayPoint::new(3, 0));
    cx.simulate_keystrokes(handle, "alt-enter");
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    cx.simulate_keystrokes(handle, "alt-backspace");
    visual.run_until_parked();
    assert_eq!(
        *events.borrow(),
        vec![
            ReviewAction::AcceptLine { hunk: 10, line: 0 },
            ReviewAction::RejectLine { hunk: 10, line: 0 },
        ]
    );
}

// -- keys -----------------------------------------------------------------------

#[gpui::test]
fn ctrl_enter_accepts_in_a_hunk_and_breaks_the_line_outside(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let events = record(&view, &mut visual);
    // A phantom row counts as "in the hunk".
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    cx.simulate_keystrokes(handle, "ctrl-enter");
    visual.run_until_parked();
    assert_eq!(*events.borrow(), vec![ReviewAction::AcceptHunk(10)]);
    assert_eq!(visual.update(|_window, cx| view.read(cx).text()), TEXT);

    set_cursor(&view, &mut visual, DisplayPoint::new(0, 1));
    cx.simulate_keystrokes(handle, "ctrl-enter");
    visual.run_until_parked();
    assert_eq!(events.borrow().len(), 1);
    assert!(
        visual
            .update(|_window, cx| view.read(cx).text())
            .starts_with("a\n\nb")
    );
}

#[gpui::test]
fn ctrl_backspace_rejects_in_a_hunk_and_deletes_a_word_outside(cx: &mut TestAppContext) {
    let (view, handle, mut visual) =
        open_default(cx, "hola mundo\nb\n", review(vec![hunk(3, &["x"], 1..2)]));
    let events = record(&view, &mut visual);
    set_cursor(&view, &mut visual, DisplayPoint::new(2, 1));
    cx.simulate_keystrokes(handle, "ctrl-backspace");
    visual.run_until_parked();
    assert_eq!(*events.borrow(), vec![ReviewAction::RejectHunk(3)]);

    set_cursor(&view, &mut visual, DisplayPoint::new(0, 10));
    cx.simulate_keystrokes(handle, "ctrl-backspace");
    visual.run_until_parked();
    assert_eq!(events.borrow().len(), 1);
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).text()),
        "hola \nb\n"
    );
}

#[gpui::test]
fn navigation_keys_emit_even_without_a_hunk_under_the_cursor(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_default(cx, TEXT, ReviewView::default());
    let events = record(&view, &mut visual);
    cx.simulate_keystrokes(handle, "alt-j alt-k f7 shift-f7 alt-l alt-shift-u");
    visual.run_until_parked();
    assert_eq!(
        *events.borrow(),
        vec![
            ReviewAction::NextHunk,
            ReviewAction::PrevHunk,
            ReviewAction::NextHunk,
            ReviewAction::PrevHunk,
            ReviewAction::NextFile,
            ReviewAction::UndoLastReject,
        ]
    );
}

#[gpui::test]
fn alt_j_moves_to_the_next_hunk_and_tells_the_host(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let events = record(&view, &mut visual);
    cx.simulate_keystrokes(handle, "alt-j");
    visual.run_until_parked();
    assert_eq!(*events.borrow(), vec![ReviewAction::NextHunk]);
    assert_eq!(visual.update(|_window, cx| view.read(cx).cursor.row), 1);
}

#[gpui::test]
fn the_review_context_covers_phantom_and_added_rows(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    for (row, inside) in [
        (0, false),
        (1, true),
        (2, true),
        (3, true),
        (4, false),
        (7, true),
    ] {
        set_cursor(&view, &mut visual, DisplayPoint::new(row, 0));
        let context = visual.update(|_window, cx| view.read(cx).key_context());
        assert_eq!(
            context.contains("review_hunk_under_cursor"),
            inside,
            "row {row}: {context}"
        );
    }
}

// -- turn active ----------------------------------------------------------------

#[gpui::test]
fn turn_active_disables_every_control_but_not_typing(cx: &mut TestAppContext) {
    let mut active = two_hunks();
    active.turn_active = true;
    let (view, handle, mut visual) = open_default(cx, TEXT, active);
    let events = record(&view, &mut visual);
    set_cursor(&view, &mut visual, DisplayPoint::new(3, 0));
    let (line_height, _, bounds) = geometry(&view, &mut visual);
    hover(&mut visual, 300., line_height * 3.5);

    let painted = frame(&view, &mut visual);
    assert_eq!(painted.review.pills.len(), 1);
    assert!(
        !painted.review.pills[0].enabled,
        "the pill shows the spinner"
    );
    assert_eq!(painted.review.line_icons, None);
    assert_eq!(
        painted.review.bar.as_deref(),
        Some("El agente está editando…")
    );
    assert!(!painted.review.bar_enabled);

    // Keys and clicks decide nothing.
    cx.simulate_keystrokes(
        handle,
        "ctrl-enter ctrl-backspace alt-enter ctrl-shift-enter",
    );
    let pill_accept = f32::from(bounds.right()) - 8. - 12. - 190. + 40.;
    click(&mut visual, pill_accept, line_height * 1.5);
    visual.run_until_parked();
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());

    // Typing still works.
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 1));
    cx.simulate_input(handle, "Z");
    visual.run_until_parked();
    assert!(
        visual
            .update(|_window, cx| view.read(cx).text())
            .starts_with("aZ\n")
    );
}

#[gpui::test]
fn the_spinner_ticks_only_while_the_turn_is_active(cx: &mut TestAppContext) {
    let mut active = two_hunks();
    active.turn_active = true;
    let (view, _handle, mut visual) = open_default(cx, TEXT, active);
    let phase =
        |visual: &mut VisualTestContext| visual.update(|_w, cx| view.read(cx).spinner_phase);
    let start = phase(&mut visual);
    visual.executor().advance_clock(Duration::from_millis(400));
    visual.run_until_parked();
    let ticked = phase(&mut visual);
    assert!(ticked > start, "{start} -> {ticked}");

    set_review(&view, &mut visual, two_hunks());
    visual.executor().advance_clock(Duration::from_millis(400));
    visual.run_until_parked();
    assert_eq!(phase(&mut visual), ticked, "stopped with the turn");
}

// -- floating bar ---------------------------------------------------------------

#[gpui::test]
fn the_bar_lists_the_actions_and_the_position(cx: &mut TestAppContext) {
    let mut three = two_hunks();
    three.hunks.push(hunk(30, &[], 7..8));
    three.pending_in_file = 3;
    let (view, _handle, mut visual) = open_default(cx, TEXT, three);
    // The cursor is on the second hunk (display row 7 is its phantom row).
    set_cursor(&view, &mut visual, DisplayPoint::new(7, 0));
    let painted = frame(&view, &mut visual);
    assert_eq!(
        painted.review.bar.as_deref(),
        Some(
            "✓ Aceptar todo Ctrl+Alt+↵ · ✗ Rechazar todo Ctrl+Alt+⌫ · ↑ Alt+K ↓ Alt+J · \
             cambio 2 de 3 · Revisar todo"
        )
    );
    assert!(painted.review.bar_enabled);
}

/// A file decided only as a whole (the read-only tab of an agent deletion:
/// one hunk of phantom rows over an empty buffer) gets the same bar as any
/// other file, without file buttons: its single hunk's pill decides it.
#[gpui::test]
fn the_bar_of_a_deleted_file_has_no_file_buttons(cx: &mut TestAppContext) {
    let whole = review(vec![hunk(40, &["x", "y"], 0..0)]);
    let (view, handle, mut visual) = open_default(cx, "", whole);
    let painted = frame(&view, &mut visual);
    assert_eq!(
        painted.review.bar.as_deref(),
        Some(
            "✓ Aceptar todo Ctrl+Alt+↵ · ✗ Rechazar todo Ctrl+Alt+⌫ · ↑ Alt+K ↓ Alt+J · \
             cambio 1 de 1 · Revisar todo"
        )
    );
    assert!(
        painted
            .review
            .bar_buttons
            .iter()
            .all(|button| !button.text.contains("archivo")),
        "{:?}",
        painted.review.bar_buttons
    );

    // The pill of the single hunk shows on hover and decides it.
    let events = record(&view, &mut visual);
    let (line_height, _, _) = geometry(&view, &mut visual);
    hover(&mut visual, 300., line_height * 0.5);
    let pill = frame(&view, &mut visual)
        .review
        .pills
        .first()
        .copied()
        .expect("the pill shows on hover");
    assert_eq!(pill.hunk, 40);
    let center = pill.bounds.center();
    click(
        &mut visual,
        f32::from(pill.bounds.left()) + 40.,
        f32::from(center.y),
    );
    assert_eq!(*events.borrow(), vec![ReviewAction::AcceptHunk(40)]);

    // The file decision still answers the keyboard (`review::accept_file`).
    cx.simulate_keystrokes(handle, "ctrl-shift-enter");
    visual.run_until_parked();
    assert_eq!(
        *events.borrow(),
        vec![ReviewAction::AcceptHunk(40), ReviewAction::AcceptFile]
    );
}

/// The pill sticks out of its hunk (here it sits on the row above, the
/// hunk's own row has no room): the mouse going from the hunk up to the
/// pill must not make it go away, and the click decides the hunk. The
/// hover zone is the hunk's rows plus the pill itself.
#[gpui::test]
fn the_pill_stays_while_the_mouse_reaches_it_from_above(cx: &mut TestAppContext) {
    let long = "x".repeat(400);
    let text = format!("arriba\ncorto\n{long}\nfin\n");
    // Buffer row 2 (the long one) was added by the agent.
    let (view, _handle, mut visual) = open_default(cx, &text, review(vec![hunk(10, &[], 2..3)]));
    let events = record(&view, &mut visual);
    let (line_height, _, _) = geometry(&view, &mut visual);

    // A row above everything: no pill (the cursor is on row 0, not in the
    // hunk).
    hover(&mut visual, 300., line_height * 0.5);
    assert!(frame(&view, &mut visual).review.pills.is_empty());

    // Into the hunk: its pill shows, on the row above it.
    hover(&mut visual, 300., line_height * 2.5);
    let pill = frame(&view, &mut visual)
        .review
        .pills
        .first()
        .copied()
        .expect("the pill shows on hover");
    assert_eq!(pill.hunk, 10);
    assert_eq!(pill.display_row, 1, "on the row above the hunk");
    assert!(!pill.compact);

    // Up onto the pill's "Aceptar", outside the hunk's rows: it stays.
    let accept_x = f32::from(pill.bounds.left()) + 40.;
    let y = f32::from(pill.bounds.center().y);
    assert!(
        y < line_height * 2.,
        "the button is outside the hunk's rows"
    );
    hover(&mut visual, accept_x, y);
    let painted = frame(&view, &mut visual);
    assert_eq!(painted.review.pills.len(), 1, "the pill is still there");
    assert_eq!(painted.review.pills[0].hovered, Some(true));

    // Its top edge too, coming down from further above.
    hover(&mut visual, accept_x, f32::from(pill.bounds.top()) + 1.);
    assert_eq!(frame(&view, &mut visual).review.pills.len(), 1);

    // And the click decides the hunk.
    click(&mut visual, accept_x, y);
    assert_eq!(*events.borrow(), vec![ReviewAction::AcceptHunk(10)]);

    // Off the pill and off the hunk, it goes away.
    hover(&mut visual, 300., line_height * 0.5);
    assert!(frame(&view, &mut visual).review.pills.is_empty());
}

#[gpui::test]
fn the_bar_counts_the_next_hunk_when_the_cursor_is_outside(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    set_cursor(&view, &mut visual, DisplayPoint::new(5, 0));
    let bar = frame(&view, &mut visual).review.bar.unwrap();
    assert!(bar.contains("cambio 2 de 2"), "{bar}");
}

#[gpui::test]
fn clicking_revisar_todo_opens_the_panel(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let events = record(&view, &mut visual);
    let (_, _, bounds) = geometry(&view, &mut visual);
    // "Revisar todo" is the last item: it ends at the bar padding.
    let x = f32::from(bounds.right()) - BAR_MARGIN - crate::element::BAR_PADDING - 10.;
    let y = f32::from(bounds.bottom()) - BAR_MARGIN - 15.;
    click(&mut visual, x, y);
    assert_eq!(*events.borrow(), vec![ReviewAction::OpenReviewPanel]);
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).cursor),
        DisplayPoint::new(0, 0),
        "a click on the bar does not move the cursor"
    );
}

#[gpui::test]
fn the_bar_says_nothing_is_left_here_for_three_seconds(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    set_review(
        &view,
        &mut visual,
        ReviewView {
            pending_in_other_files: 4,
            ..Default::default()
        },
    );
    assert_eq!(
        frame(&view, &mut visual).review.bar.as_deref(),
        Some(
            "Sin cambios pendientes en este archivo · 4 en otros archivos → siguiente archivo (Alt+L)"
        )
    );
    visual.executor().advance_clock(Duration::from_millis(3100));
    visual.run_until_parked();
    assert_eq!(frame(&view, &mut visual).review.bar, None);
}

#[gpui::test]
fn no_bar_without_a_review(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, ReviewView::default());
    let painted = frame(&view, &mut visual);
    assert_eq!(painted.review.bar, None);
    assert!(painted.review.pills.is_empty());
}

#[gpui::test]
fn the_bar_hides_when_the_window_is_not_active(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    assert!(frame(&view, &mut visual).review.bar.is_some());
    visual.deactivate_window();
    assert_eq!(frame(&view, &mut visual).review.bar, None);
}

// -- set_review -----------------------------------------------------------------

#[gpui::test]
fn no_frame_paints_a_stale_review(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    let _ = frame(&view, &mut visual);
    set_review(&view, &mut visual, review(vec![hunk(20, &["z"], 5..5)]));
    let frames = visual.update(|_window, cx| view.update(cx, |view, _| view.take_render_frames()));
    assert!(!frames.is_empty());
    for painted in frames {
        assert_eq!(painted.review.hunks, vec![20]);
        let phantoms = painted
            .rows
            .iter()
            .filter(|row| matches!(row.kind, RowKind::Phantom(_)))
            .count();
        assert_eq!(phantoms, 1);
        assert!(painted.review.pills.iter().all(|pill| pill.hunk == 20));
    }
}

#[gpui::test]
fn a_review_change_reshapes_no_row(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let _ = frame(&view, &mut visual);
    // A hunk goes away: every remaining row was already shaped.
    set_review(&view, &mut visual, review(vec![hunk(20, &["z"], 5..5)]));
    // Only a flag changes: not even the display map is rebuilt.
    let rebuilds = visual.update(|_window, cx| view.read(cx).review_rebuilds);
    let mut flagged = review(vec![hunk(20, &["z"], 5..5)]);
    flagged.hunks[0].from_previous_turn = true;
    flagged.pending_in_other_files = 2;
    set_review(&view, &mut visual, flagged);
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).review_rebuilds),
        rebuilds
    );
    let frames = visual.update(|_window, cx| view.update(cx, |view, _| view.take_render_frames()));
    assert!(!frames.is_empty());
    for painted in frames {
        for row in &painted.rows {
            assert!(!row.reshaped, "row {} was shaped again", row.display_row);
        }
    }
}

#[gpui::test]
fn the_cursor_keeps_its_line_when_a_hunk_above_goes_away(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    // Display row 9 is buffer row 6 ("g").
    set_cursor(&view, &mut visual, DisplayPoint::new(9, 1));
    set_review(&view, &mut visual, review(vec![hunk(20, &["z"], 5..5)]));
    let (row, point) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (
            view.display_row_source(view.cursor.row),
            view.cursor_point(),
        )
    });
    assert_eq!(row, "g");
    assert_eq!(point, cincel_text::Point::new(6, 1));
}

#[gpui::test]
fn after_a_decision_the_cursor_jumps_to_the_next_hunk(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, TEXT, two_hunks(), jumping());
    set_cursor(&view, &mut visual, DisplayPoint::new(3, 0));
    cx.simulate_keystrokes(handle, "ctrl-enter");
    visual.run_until_parked();
    // The host applied it: hunk 10 is gone.
    set_review(&view, &mut visual, review(vec![hunk(20, &["z"], 5..5)]));
    let (cursor, under) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.cursor, view.hunk_under_cursor())
    });
    assert_eq!(under, Some(0), "the cursor is in the next hunk");
    assert_eq!(cursor, DisplayPoint::new(5, 0));
}

#[gpui::test]
fn without_jump_to_next_the_cursor_stays(cx: &mut TestAppContext) {
    let settings = EditorSettings {
        jump_to_next_on_decide: false,
        ..Default::default()
    };
    let (view, handle, mut visual) = open(cx, TEXT, two_hunks(), settings);
    set_cursor(&view, &mut visual, DisplayPoint::new(3, 0));
    cx.simulate_keystrokes(handle, "ctrl-enter");
    visual.run_until_parked();
    set_review(&view, &mut visual, review(vec![hunk(20, &["z"], 5..5)]));
    let point = visual.update(|_window, cx| view.read(cx).cursor_point());
    assert_eq!(point, cincel_text::Point::new(1, 0), "still on `b`");
}

#[gpui::test]
fn a_new_current_index_moves_the_cursor_there(cx: &mut TestAppContext) {
    let text: String = (0..200).map(|row| format!("fila {row}\n")).collect();
    let far = review(vec![hunk(1, &["x"], 2..3), hunk(2, &["y"], 180..181)]);
    let (view, _handle, mut visual) = open_default(cx, &text, far.clone());
    let mut current = far;
    current.current_index = Some(1);
    set_review(&view, &mut visual, current);
    let (under, scroll_row) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (view.hunk_under_cursor(), view.scroll_row())
    });
    assert_eq!(under, Some(1));
    assert!(scroll_row > 100., "scrolled into view, at {scroll_row}");
}

#[gpui::test]
fn the_hunk_follows_rows_typed_above_it(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_default(cx, TEXT, two_hunks());
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 1));
    cx.simulate_keystrokes(handle, "enter");
    visual.run_until_parked();
    let (rows, start) = visual.update(|_window, cx| {
        let view = view.read(cx);
        (
            view.review().hunks[0].buffer_rows.clone(),
            view.diff().hunk_display_range(0).start,
        )
    });
    assert_eq!(rows, 2..3);
    assert_eq!(start, 2, "the phantom block moved down with it");
}

// Search over phantom rows (07-etapa5 §10.2) lives in `search_tests.rs`.

#[gpui::test]
fn legacy_set_hunks_still_feeds_the_review(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, ReviewView::default());
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            #[allow(deprecated)]
            view.set_hunks(
                vec![crate::display_map::PhantomHunk {
                    insert_before_buffer_row: 2,
                    deleted_text: vec!["viejo".into()],
                    added_rows: 2..3,
                }],
                cx,
            );
        })
    });
    visual.run_until_parked();
    let (review, bar) = (
        visual.update(|_window, cx| view.read(cx).review().clone()),
        frame(&view, &mut visual).review.bar,
    );
    assert_eq!(review.hunks.len(), 1);
    assert_eq!(review.hunks[0].kind, ReviewHunkKind::Modified);
    assert!(bar.unwrap().contains("cambio 1 de 1"));
}

// -- mouse cursors ----------------------------------------------------------------

fn cursor_at(view: &Entity<EditorView>, cx: &mut VisualTestContext, x: f32, y: f32) -> CursorStyle {
    hover(cx, x, y);
    cx.update(|_window, cx| view.read(cx).cursor_style_at(point(px(x), px(y))))
}

#[gpui::test]
fn an_enabled_pill_shows_the_hand_on_its_buttons_and_the_arrow_elsewhere(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    let (line_height, _, bounds) = geometry(&view, &mut visual);
    let pill_left = f32::from(bounds.right()) - 8. - 12. - 190.;
    let y = line_height * 1.5;
    assert_eq!(
        cursor_at(&view, &mut visual, pill_left + 40., y),
        CursorStyle::PointingHand,
        "accept"
    );
    assert_eq!(
        cursor_at(&view, &mut visual, pill_left + 190. - 20., y),
        CursorStyle::PointingHand,
        "reject"
    );
    assert_eq!(
        cursor_at(&view, &mut visual, pill_left + 2., y),
        CursorStyle::Arrow,
        "the pill's padding is not text"
    );
    assert_eq!(
        cursor_at(&view, &mut visual, pill_left - 40., y),
        CursorStyle::IBeam,
        "the text keeps the I-beam"
    );
}

#[gpui::test]
fn a_disabled_pill_shows_the_arrow_and_ignores_clicks(cx: &mut TestAppContext) {
    let mut active = two_hunks();
    active.turn_active = true;
    let (view, _handle, mut visual) = open_default(cx, TEXT, active);
    let events = record(&view, &mut visual);
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    let (line_height, _, bounds) = geometry(&view, &mut visual);
    let pill_left = f32::from(bounds.right()) - 8. - 12. - 190.;
    let y = line_height * 1.5;
    for x in [pill_left + 40., pill_left + 190. - 20., pill_left + 2.] {
        assert_eq!(
            cursor_at(&view, &mut visual, x, y),
            CursorStyle::Arrow,
            "{x}"
        );
        click(&mut visual, x, y);
    }
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).cursor),
        DisplayPoint::new(1, 0),
        "a click on a disabled pill does not move the cursor either"
    );
}

#[gpui::test]
fn the_gutter_icons_show_the_hand(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let (line_height, _, _) = geometry(&view, &mut visual);
    let (plus, minus) = icon_centers(&view, &mut visual, line_height * 2.5);
    assert_eq!(
        cursor_at(&view, &mut visual, plus, line_height * 2.5),
        CursorStyle::PointingHand,
        "+"
    );
    assert_eq!(
        cursor_at(&view, &mut visual, minus, line_height * 2.5),
        CursorStyle::PointingHand,
        "−"
    );
}

#[gpui::test]
fn the_bar_buttons_show_the_hand_and_its_text_the_arrow(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let (_, _, bounds) = geometry(&view, &mut visual);
    let y = f32::from(bounds.bottom()) - BAR_MARGIN - 15.;
    let revisar = f32::from(bounds.right()) - BAR_MARGIN - crate::element::BAR_PADDING - 10.;
    assert_eq!(
        cursor_at(&view, &mut visual, revisar, y),
        CursorStyle::PointingHand,
        "Revisar todo"
    );
    // The right padding of the bar is part of the bar, not a button.
    let padding = f32::from(bounds.right()) - BAR_MARGIN - 2.;
    assert_eq!(
        cursor_at(&view, &mut visual, padding, y),
        CursorStyle::Arrow
    );
}

#[gpui::test]
fn the_bar_of_an_active_turn_shows_the_arrow_everywhere(cx: &mut TestAppContext) {
    let mut active = two_hunks();
    active.turn_active = true;
    let (view, _handle, mut visual) = open_default(cx, TEXT, active);
    let (_, _, bounds) = geometry(&view, &mut visual);
    let y = f32::from(bounds.bottom()) - BAR_MARGIN - 15.;
    let x = f32::from(bounds.right()) - BAR_MARGIN - crate::element::BAR_PADDING - 10.;
    assert_eq!(cursor_at(&view, &mut visual, x, y), CursorStyle::Arrow);
}

// -- pill and bar colours (etapa-3 § correcciones) ------------------------------

#[gpui::test]
fn an_enabled_pill_reads_as_enabled_and_highlights_the_hovered_half(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    set_cursor(&view, &mut visual, DisplayPoint::new(2, 0));
    let theme = EditorTheme::default();
    let pill = frame(&view, &mut visual).review.pills[0];
    assert!(pill.enabled);
    assert_eq!(pill.check_color, crate::theme::color(theme.status_ok));
    assert_eq!(pill.cross_color, crate::theme::color(theme.status_error));
    assert_eq!(pill.label_color, crate::theme::color(theme.text));
    assert_eq!(pill.opacity, 1.);
    assert_eq!(pill.hovered, None);

    // The accept half, then the reject half, under the mouse.
    let y = f32::from(pill.bounds.center().y);
    hover(&mut visual, f32::from(pill.bounds.left()) + 40., y);
    assert_eq!(
        frame(&view, &mut visual).review.pills[0].hovered,
        Some(true)
    );
    hover(&mut visual, f32::from(pill.bounds.right()) - 20., y);
    assert_eq!(
        frame(&view, &mut visual).review.pills[0].hovered,
        Some(false)
    );
}

#[gpui::test]
fn a_disabled_pill_is_muted_and_never_highlighted(cx: &mut TestAppContext) {
    let mut active = two_hunks();
    active.turn_active = true;
    let (view, _handle, mut visual) = open_default(cx, TEXT, active);
    set_cursor(&view, &mut visual, DisplayPoint::new(2, 0));
    let muted = crate::theme::color(EditorTheme::default().text_muted);
    let pill = frame(&view, &mut visual).review.pills[0];
    assert!(!pill.enabled);
    assert_eq!(
        (pill.check_color, pill.cross_color, pill.label_color),
        (muted, muted, muted)
    );
    hover(
        &mut visual,
        f32::from(pill.bounds.left()) + 40.,
        f32::from(pill.bounds.center().y),
    );
    assert_eq!(frame(&view, &mut visual).review.pills[0].hovered, None);
}

#[gpui::test]
fn the_bar_buttons_read_as_enabled_and_highlight_on_hover(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let theme = EditorTheme::default();
    let buttons = frame(&view, &mut visual).review.bar_buttons;
    let accept = buttons
        .iter()
        .find(|button| button.text == "✓ Aceptar todo")
        .expect("Aceptar todo");
    let reject = buttons
        .iter()
        .find(|button| button.text == "✗ Rechazar todo")
        .expect("Rechazar todo");
    assert_eq!(accept.glyph_color, crate::theme::color(theme.status_ok));
    assert_eq!(accept.label_color, crate::theme::color(theme.text));
    assert_eq!(reject.glyph_color, crate::theme::color(theme.status_error));
    assert_eq!(reject.label_color, crate::theme::color(theme.text));
    assert!(buttons.iter().all(|button| !button.hovered));

    // "Revisar todo", the last button, sits right before the bar's padding.
    let (_, _, bounds) = geometry(&view, &mut visual);
    let y = f32::from(bounds.bottom()) - BAR_MARGIN - 15.;
    let x = f32::from(bounds.right()) - BAR_MARGIN - crate::element::BAR_PADDING - 10.;
    hover(&mut visual, x, y);
    let buttons = frame(&view, &mut visual).review.bar_buttons;
    let hovered: Vec<&str> = buttons
        .iter()
        .filter(|button| button.hovered)
        .map(|button| button.text.as_str())
        .collect();
    assert_eq!(hovered, vec!["Revisar todo"]);
}

/// The bar's two buttons decide the whole turn (`workspace::accept_turn` /
/// `reject_turn`), not the hunk under the cursor: that one has its pill and
/// `Ctrl+↵` / `Ctrl+⌫`.
#[gpui::test]
fn the_bar_buttons_decide_the_whole_turn(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let events = record(&view, &mut visual);
    let (_, _, bounds) = geometry(&view, &mut visual);
    let y = f32::from(bounds.bottom()) - BAR_MARGIN - 15.;
    // Walks the bar from its right end until the mouse is over `label`.
    let find = |visual: &mut VisualTestContext, label: &str| -> f32 {
        let mut x = f32::from(bounds.right()) - BAR_MARGIN - 4.;
        while x > f32::from(bounds.left()) {
            hover(visual, x, y);
            let buttons = frame(&view, visual).review.bar_buttons;
            if buttons
                .iter()
                .any(|button| button.hovered && button.text == label)
            {
                return x;
            }
            x -= 4.;
        }
        panic!("no se encontró «{label}» en la barra");
    };
    let accept = find(&mut visual, "✓ Aceptar todo");
    click(&mut visual, accept, y);
    let reject = find(&mut visual, "✗ Rechazar todo");
    click(&mut visual, reject, y);
    assert_eq!(
        *events.borrow(),
        vec![ReviewAction::AcceptTurn, ReviewAction::RejectTurn]
    );
}

// -- line icons over the numbers (etapa-3 § correcciones) -----------------------

#[gpui::test]
fn the_gutter_is_as_wide_with_and_without_a_review(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, ReviewView::default());
    let plain = frame(&view, &mut visual);
    set_review(&view, &mut visual, two_hunks());
    let reviewed = frame(&view, &mut visual);
    let mut active = two_hunks();
    active.turn_active = true;
    set_review(&view, &mut visual, active);
    let writing = frame(&view, &mut visual);
    set_review(&view, &mut visual, ReviewView::default());
    let after = frame(&view, &mut visual);
    for other in [&reviewed, &writing, &after] {
        assert_eq!(other.gutter_width, plain.gutter_width);
        assert_eq!(other.text_origin_x, plain.text_origin_x);
    }
}

#[gpui::test]
fn the_line_icons_sit_over_the_number_column_and_hide_its_number(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    let (line_height, _, _) = geometry(&view, &mut visual);
    // Display row 3 is buffer row 1, a real (added) row with a number.
    hover(&mut visual, 300., line_height * 3.5);
    let painted = frame(&view, &mut visual);
    let (left, right) = painted.review.number_column;
    let (plus, minus) = painted.review.line_icon_bounds.expect("icons");
    for icon in [plus, minus] {
        assert!(
            icon.left() >= left && icon.right() <= right,
            "{icon:?} fuera de {left:?}..{right:?}"
        );
        assert!(
            icon.size.width == px(crate::element::LINE_ICON_SIZE)
                || icon.size.width == px(crate::element::LINE_ICON_SIZE_SMALL),
            "{icon:?}"
        );
    }
    assert_eq!(
        minus.left() - plus.right(),
        px(crate::element::LINE_ICON_GAP)
    );
    assert!(!painted.row(3).unwrap().line_number, "the number gives way");
    assert!(painted.row(4).unwrap().line_number, "the next row keeps it");
    // A three-digit column always fits both icons.
    assert!(right - left >= px(2. * crate::element::LINE_ICON_SIZE_SMALL + 2.));
}

#[test]
fn the_line_icons_shrink_to_eleven_pixels_when_fourteen_do_not_fit() {
    use crate::element::line_icon_size;
    // Three JetBrains Mono digits at 14 px are about 25 px wide.
    assert_eq!(line_icon_size(25.2), 11.);
    assert_eq!(line_icon_size(30.), 14.);
    assert_eq!(line_icon_size(42.), 14.);
    assert_eq!(line_icon_size(20.), 9.);
}

#[gpui::test]
fn toggling_the_review_reshapes_no_text_row(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, ReviewView::default());
    let plain = frame(&view, &mut visual);
    // Buffer rows 2..4 added: no phantom rows, so every painted row is a row
    // that was already on screen.
    set_review(&view, &mut visual, review(vec![hunk(1, &[], 2..4)]));
    let reviewed = frame(&view, &mut visual);
    assert!(
        reviewed.rows.iter().all(|row| !row.reshaped),
        "{:?}",
        reviewed.rows
    );
    set_review(&view, &mut visual, ReviewView::default());
    let after = frame(&view, &mut visual);
    assert!(
        after.rows.iter().all(|row| !row.reshaped),
        "{:?}",
        after.rows
    );
    assert_eq!(reviewed.text_origin_x, plain.text_origin_x);
    assert_eq!(after.text_origin_x, plain.text_origin_x);
}

// -- pill placement (etapa-3 § correcciones) -------------------------------------

/// A line long enough to run under the pill at any test window width.
fn long_line() -> String {
    "x".repeat(400)
}

#[gpui::test]
fn the_pill_sits_on_a_short_first_row(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open_default(cx, TEXT, two_hunks());
    set_cursor(&view, &mut visual, DisplayPoint::new(3, 0));
    let painted = frame(&view, &mut visual);
    let pill = painted.review.pills[0];
    // Display rows 1 and 2 are the phantom `x` and `y`: the first one wins.
    assert_eq!(pill.display_row, 1);
    assert!(!pill.compact);
    assert_eq!(pill.bounds.size.height, px(crate::element::PILL_HEIGHT));
}

#[gpui::test]
fn a_long_first_row_sends_the_pill_to_the_next_short_one(cx: &mut TestAppContext) {
    let text = format!("a\n{}\nb\nc\n", long_line());
    let (view, _handle, mut visual) = open_default(cx, &text, review(vec![hunk(1, &[], 1..3)]));
    let plain_origin = frame(&view, &mut visual).text_origin_x;
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    let painted = frame(&view, &mut visual);
    let pill = painted.review.pills[0];
    assert_eq!(pill.display_row, 2, "the short second row");
    assert!(!pill.compact);
    assert_eq!(painted.text_origin_x, plain_origin, "nothing moved");
}

#[gpui::test]
fn the_row_above_is_the_last_candidate(cx: &mut TestAppContext) {
    let text = format!("a\n{}\n{}\nc\n", long_line(), long_line());
    let (view, _handle, mut visual) = open_default(cx, &text, review(vec![hunk(1, &[], 1..3)]));
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    let pill = frame(&view, &mut visual).review.pills[0];
    assert_eq!(pill.display_row, 0, "the short row just above the hunk");
    assert!(!pill.compact);
}

#[gpui::test]
fn with_no_room_anywhere_the_pill_turns_compact_and_translucent(cx: &mut TestAppContext) {
    let text = format!("{}\n{}\n{}\nc\n", long_line(), long_line(), long_line());
    let (view, _handle, mut visual) = open_default(cx, &text, review(vec![hunk(7, &[], 1..3)]));
    let plain = frame(&view, &mut visual);
    let events = record(&view, &mut visual);
    set_cursor(&view, &mut visual, DisplayPoint::new(2, 0));
    let painted = frame(&view, &mut visual);
    let pill = painted.review.pills[0];
    assert!(pill.compact);
    assert_eq!(pill.display_row, 1, "the first row of the hunk");
    assert_eq!(pill.opacity, crate::element::COMPACT_PILL_OPACITY);
    assert_eq!(
        pill.bounds.size.width,
        px(2. * crate::element::COMPACT_PILL_BUTTON + crate::element::COMPACT_PILL_GAP)
    );
    assert_eq!(painted.gutter_width, plain.gutter_width);
    assert_eq!(painted.text_origin_x, plain.text_origin_x);

    // Its hitboxes are where it was painted: left button accepts, right one
    // rejects.
    let y = f32::from(pill.bounds.center().y);
    let accept_x = f32::from(pill.bounds.left()) + crate::element::COMPACT_PILL_BUTTON / 2.;
    let reject_x = f32::from(pill.bounds.right()) - crate::element::COMPACT_PILL_BUTTON / 2.;
    click(&mut visual, accept_x, y);
    click(&mut visual, reject_x, y);
    assert_eq!(
        *events.borrow(),
        vec![ReviewAction::AcceptHunk(7), ReviewAction::RejectHunk(7)]
    );
}

#[gpui::test]
fn a_pure_deletion_places_the_pill_on_its_phantom_rows(cx: &mut TestAppContext) {
    let long = long_line();
    let (view, _handle, mut visual) = open_default(
        cx,
        &format!("{long}\nb\nc\n"),
        review(vec![hunk(3, &[long.as_str(), "corto"], 1..1)]),
    );
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 0));
    let pill = frame(&view, &mut visual).review.pills[0];
    // Display rows 1 and 2 are the phantom rows; the second is short.
    assert_eq!(pill.display_row, 2);
    assert!(!pill.compact);
}

// -- centring the target of a jump -----------------------------------------

/// `(cursor wrap row, first visible wrap row, visible rows)` after the last
/// frame.
fn jump_geometry(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> (f32, f32, f32) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|_window, cx| {
        let view = view.read(cx);
        let visible = view.layout.as_ref().expect("laid out").visible_row_count;
        (view.cursor_wrap_row() as f32, view.scroll_row(), visible)
    })
}

/// The cursor row sits in the middle of the viewport (within one row).
fn assert_centred((cursor, scroll, visible): (f32, f32, f32)) {
    let offset = cursor + 0.5 - scroll;
    assert!(
        (offset - visible / 2.).abs() <= 1.,
        "row {cursor} at {offset} of {visible} visible rows (scroll {scroll})"
    );
}

fn long_text() -> String {
    (0..300).map(|row| format!("fila {row}\n")).collect()
}

#[gpui::test]
fn a_host_jump_centres_the_target_row(cx: &mut TestAppContext) {
    let far = review(vec![hunk(1, &["x"], 2..3), hunk(2, &["y"], 200..201)]);
    let (view, _handle, mut visual) = open_default(cx, &long_text(), far.clone());
    let mut current = far;
    current.current_index = Some(1);
    set_review(&view, &mut visual, current);
    let geometry = jump_geometry(&view, &mut visual);
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).hunk_under_cursor()),
        Some(1)
    );
    assert_centred(geometry);
}

#[gpui::test]
fn alt_j_and_alt_k_centre_the_hunk(cx: &mut TestAppContext) {
    let far = review(vec![hunk(1, &["x"], 100..101), hunk(2, &["y"], 200..201)]);
    let (view, handle, mut visual) = open_default(cx, &long_text(), far);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    cx.simulate_keystrokes(handle, "alt-j");
    visual.run_until_parked();
    assert_centred(jump_geometry(&view, &mut visual));
    cx.simulate_keystrokes(handle, "alt-j");
    visual.run_until_parked();
    assert_centred(jump_geometry(&view, &mut visual));
    cx.simulate_keystrokes(handle, "alt-k");
    visual.run_until_parked();
    let geometry = jump_geometry(&view, &mut visual);
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).hunk_under_cursor()),
        Some(0)
    );
    assert_centred(geometry);
}

#[gpui::test]
fn deciding_centres_the_next_hunk(cx: &mut TestAppContext) {
    let two = review(vec![hunk(1, &["x"], 3..4), hunk(2, &["y"], 200..201)]);
    let (view, handle, mut visual) = open(cx, &long_text(), two, jumping());
    set_cursor(&view, &mut visual, DisplayPoint::new(4, 0));
    cx.simulate_keystrokes(handle, "ctrl-enter");
    visual.run_until_parked();
    // The host applied it: hunk 1 is gone.
    set_review(&view, &mut visual, review(vec![hunk(2, &["y"], 199..200)]));
    let geometry = jump_geometry(&view, &mut visual);
    assert_eq!(
        visual.update(|_window, cx| view.read(cx).hunk_under_cursor()),
        Some(0)
    );
    assert_centred(geometry);
}

#[gpui::test]
fn a_hunk_at_the_end_scrolls_as_far_as_the_document_allows(cx: &mut TestAppContext) {
    let text = long_text();
    let last = review(vec![hunk(1, &["x"], 299..300)]);
    let (view, handle, mut visual) = open_default(cx, &text, last);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    cx.simulate_keystrokes(handle, "alt-j");
    visual.run_until_parked();
    let (cursor, scroll, visible) = jump_geometry(&view, &mut visual);
    let rows = visual.update(|_window, cx| view.read(cx).wrap_row_count()) as f32;
    // Clamped at the bottom of the document, with the row on screen.
    assert!(
        (scroll - (rows - visible)).abs() < 0.01,
        "scroll {scroll}, {rows} rows, {visible} visible"
    );
    assert!(cursor >= scroll && cursor + 1. <= scroll + visible);
}

#[gpui::test]
fn a_hunk_already_in_the_middle_third_does_not_scroll(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open_default(cx, &long_text(), ReviewView::default());
    let (_, _, visible) = jump_geometry(&view, &mut visual);
    // A hunk whose phantom row lands in the middle of a viewport that starts
    // at row 0.
    let middle = (visible / 2.) as u32;
    set_review(
        &view,
        &mut visual,
        review(vec![hunk(1, &["x"], middle..middle + 1)]),
    );
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    cx.simulate_keystrokes(handle, "alt-j");
    visual.run_until_parked();
    let (cursor, scroll, visible) = jump_geometry(&view, &mut visual);
    assert_eq!(cursor, middle as f32);
    assert!(cursor >= visible / 3. && cursor + 1. <= visible * 2. / 3.);
    assert_eq!(scroll, 0., "already in the middle third: no scroll");
}

#[gpui::test]
fn the_gutter_and_the_scrollbar_show_the_arrow(cx: &mut TestAppContext) {
    let text = long_text();
    let (view, _handle, mut visual) = open_default(cx, &text, review(vec![hunk(1, &["x"], 2..3)]));
    let (line_height, text_x, bounds) = geometry(&view, &mut visual);
    let right = f32::from(bounds.right());
    let left = f32::from(bounds.left());
    let middle = f32::from(bounds.center().y);
    // The document scrolls, so the bar is there once the mouse moves.
    hover(&mut visual, right - 4., middle);
    assert!(
        frame(&view, &mut visual).rows.len() > 1,
        "a long document is painted"
    );
    assert_eq!(
        cursor_at(&view, &mut visual, right - 4., middle),
        CursorStyle::Arrow,
        "the scrollbar track"
    );
    assert_eq!(
        cursor_at(&view, &mut visual, right - 4., f32::from(bounds.top()) + 4.),
        CursorStyle::Arrow,
        "the thumb (at the top while the scroll is 0)"
    );
    assert_eq!(
        cursor_at(&view, &mut visual, left + 4., middle),
        CursorStyle::Arrow,
        "the gutter"
    );
    assert_eq!(
        cursor_at(&view, &mut visual, text_x + 40., middle),
        CursorStyle::IBeam,
        "the text keeps the I-beam"
    );
    // The review icons of the gutter keep their hand on top of the arrow
    // (display row 2 is the hunk's phantom row).
    let y = line_height * 2.5;
    let (plus, _) = icon_centers(&view, &mut visual, y);
    assert_eq!(
        cursor_at(&view, &mut visual, plus, y),
        CursorStyle::PointingHand
    );
}
