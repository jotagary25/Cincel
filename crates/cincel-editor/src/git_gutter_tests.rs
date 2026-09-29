//! The git column of the gutter over the GPUI test harness
//! (`docs/specs/07-etapa5-productividad.md` §6.3–§6.5): the gutter keeps its
//! width, the bars follow the text, phantom rows never get one, and the git
//! column, the agent's bar and the hunk border never overlap.

use std::sync::Arc;

use cincel_syntax::LanguageRegistry;
use cincel_text::{Buffer, Point};
use gpui::{AnyWindowHandle, Bounds, Entity, Pixels, TestAppContext, VisualTestContext, px};

use crate::element::{
    GIT_BAR_GAP, GIT_BAR_WIDTH, GIT_DELETED_MARK_HEIGHT, GUTTER_BAR_GAP, GUTTER_BAR_WIDTH,
    GUTTER_BEFORE_NUMBERS, GUTTER_PADDING_LEFT, GUTTER_TEXT_GAP, gutter_metrics,
};
use crate::git_gutter::{GitGutterColors, GitGutterHunk, GitGutterKind};
use crate::review::{ReviewHunkKind, ReviewHunkView, ReviewLineView, ReviewView};
use crate::settings::{EditorChrome, EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::{EditorView, FrameRender};

/// Thirty numbered lines.
fn text() -> String {
    (1..=30).map(|n| format!("línea {n}\n")).collect()
}

fn open(
    cx: &mut TestAppContext,
    settings: EditorSettings,
) -> (Entity<EditorView>, VisualTestContext) {
    let buffer = shared(Buffer::new(&text()));
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
            view.set_render_probe(true);
        })
        .unwrap();
    visual.run_until_parked();
    (view, visual)
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
        .expect("se pintó un cuadro")
}

fn hunk(kind: GitGutterKind, rows: std::ops::Range<u32>) -> GitGutterHunk {
    GitGutterHunk { kind, rows }
}

fn set_git(
    view: &Entity<EditorView>,
    cx: &mut VisualTestContext,
    hunks: Option<Vec<GitGutterHunk>>,
) {
    cx.update(|_window, cx| view.update(cx, |view, cx| view.set_git_diff(hunks, cx)));
    cx.run_until_parked();
}

/// A review hunk of the agent that replaced buffer row `row` (one phantom
/// row spliced in before it, the row itself painted as added).
fn review_on(row: u32) -> ReviewView {
    ReviewView {
        pending_in_file: 1,
        hunks: vec![ReviewHunkView {
            id: 7,
            base_rows: row..row + 1,
            deleted_lines: vec!["antes".to_owned()],
            buffer_rows: row..row + 1,
            kind: ReviewHunkKind::Modified,
            word_diffs: None,
            lines: vec![ReviewLineView {
                base_line: Some(row),
                buffer_row: Some(row),
            }],
            from_previous_turn: false,
        }],
        ..Default::default()
    }
}

fn set_review(view: &Entity<EditorView>, cx: &mut VisualTestContext, review: ReviewView) {
    cx.update(|_window, cx| view.update(cx, |view, cx| view.set_review(review, cx)));
    cx.run_until_parked();
}

/// The git marks of `kind`, top to bottom.
fn marks(frame: &FrameRender, kind: GitGutterKind) -> Vec<Bounds<Pixels>> {
    frame
        .git_marks
        .iter()
        .filter(|(mark, _)| *mark == kind)
        .map(|(_, bounds)| *bounds)
        .collect()
}

/// `(display row, git bar)` of the first segment of every painted row.
fn bars(frame: &FrameRender) -> Vec<(u32, Option<GitGutterKind>)> {
    frame
        .rows
        .iter()
        .filter(|row| row.segment == 0)
        .map(|row| (row.display_row, row.git_bar))
        .collect()
}

#[test]
fn the_gutter_geometry_adds_up_to_what_it_always_was() {
    // Before stage 5: padding 4 + agent bar 3 + gap 6.
    assert_eq!(GUTTER_BEFORE_NUMBERS, 4. + 3. + 6.);
    assert_eq!(
        GUTTER_PADDING_LEFT + GIT_BAR_WIDTH + GIT_BAR_GAP + GUTTER_BAR_WIDTH + GUTTER_BAR_GAP,
        13.
    );
    assert_eq!(
        (
            GUTTER_PADDING_LEFT,
            GIT_BAR_WIDTH,
            GIT_BAR_GAP,
            GUTTER_BAR_WIDTH,
            GUTTER_BAR_GAP
        ),
        (2., 3., 2., 3., 3.)
    );
    assert_eq!(GUTTER_TEXT_GAP, 12.);
}

/// §6.4: `gutter_metrics` and the text origin are exactly the values of
/// before this stage — 13 px + the number column + 12 px — with and without
/// git hunks and with and without a review.
#[gpui::test]
fn git_hunks_and_reviews_never_move_the_text(cx: &mut TestAppContext) {
    let (view, mut cx) = open(cx, EditorSettings::default());
    let metrics = cx.update(|window, cx| gutter_metrics(view.read(cx), window));
    // 30 rows: the number column is three digits wide (the minimum).
    let expected = px(13.) + metrics.char_width * 3. + px(12.);
    assert_eq!(metrics.number_width, metrics.char_width * 3.);
    assert_eq!(metrics.gutter_width, expected);

    let plain = frame(&view, &mut cx);
    assert_eq!(plain.gutter_width, expected);
    assert_eq!(plain.text_origin_x, plain.bounds.left() + expected);

    let mut states = Vec::new();
    set_git(
        &view,
        &mut cx,
        Some(vec![
            hunk(GitGutterKind::Modified, 2..3),
            hunk(GitGutterKind::Added, 10..12),
            hunk(GitGutterKind::Deleted, 20..20),
        ]),
    );
    states.push(("git", frame(&view, &mut cx)));
    set_review(&view, &mut cx, review_on(2));
    states.push(("git + revisión", frame(&view, &mut cx)));
    set_git(&view, &mut cx, None);
    states.push(("revisión", frame(&view, &mut cx)));
    set_review(&view, &mut cx, ReviewView::default());
    states.push(("nada", frame(&view, &mut cx)));

    for (label, frame) in states {
        assert_eq!(frame.gutter_width, plain.gutter_width, "{label}");
        assert_eq!(frame.text_origin_x, plain.text_origin_x, "{label}");
        let metrics = cx.update(|window, cx| gutter_metrics(view.read(cx), window));
        assert_eq!(metrics.gutter_width, expected, "{label}");
    }
}

#[gpui::test]
fn bars_paint_in_their_own_column_and_marks_sit_on_the_boundary(cx: &mut TestAppContext) {
    let (view, mut cx) = open(cx, EditorSettings::default());
    set_git(
        &view,
        &mut cx,
        Some(vec![
            hunk(GitGutterKind::Modified, 2..3),
            hunk(GitGutterKind::Added, 10..12),
            hunk(GitGutterKind::Deleted, 20..20),
        ]),
    );
    let frame = frame(&view, &mut cx);
    let left = frame.bounds.left();
    let line_height = cx.update(|_, cx| view.read(cx).style.line_height);

    let by_row = bars(&frame);
    for (row, bar) in &by_row {
        let expected = match row {
            2 => Some(GitGutterKind::Modified),
            10 | 11 => Some(GitGutterKind::Added),
            _ => None,
        };
        assert_eq!(*bar, expected, "fila {row}");
    }

    // Every bar and mark is 3 px wide at x = 2, clear of the agent's bar
    // (x = 7..10) and of the hunk border (centred in the 12 px gap).
    for (_, bounds) in &frame.git_marks {
        assert_eq!(bounds.left() - left, px(GUTTER_PADDING_LEFT));
        assert_eq!(bounds.size.width, px(GIT_BAR_WIDTH));
        assert!(bounds.right() - left <= px(GUTTER_PADDING_LEFT + GIT_BAR_WIDTH));
    }
    let agent_left = GUTTER_PADDING_LEFT + GIT_BAR_WIDTH + GIT_BAR_GAP;
    assert!(GUTTER_PADDING_LEFT + GIT_BAR_WIDTH < agent_left);

    // The deletion mark: 6 px high, centred on the boundary above row 20.
    let deleted = marks(&frame, GitGutterKind::Deleted);
    assert_eq!(deleted.len(), 1);
    let row_20 = frame
        .rows
        .iter()
        .position(|row| row.display_row == 20)
        .unwrap();
    let boundary = frame.bounds.top() + line_height * row_20 as f32;
    // (the viewport starts at the top of the file, row 0 at `bounds.top()`)
    assert_eq!(frame.rows[0].display_row, 0);
    assert_eq!(deleted[0].size.height, px(GIT_DELETED_MARK_HEIGHT));
    assert_eq!(
        deleted[0].top(),
        boundary - px(GIT_DELETED_MARK_HEIGHT / 2.)
    );

    // Deleted at the top of the file: inside row 1, from its top edge.
    set_git(
        &view,
        &mut cx,
        Some(vec![hunk(GitGutterKind::Deleted, 0..0)]),
    );
    let top = marks(&frame_of(&view, &mut cx), GitGutterKind::Deleted);
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].top(), frame.bounds.top());
}

fn frame_of(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> FrameRender {
    frame(view, cx)
}

/// §6.4: typing three lines above an unsaved change moves its bars three
/// rows down; the diff itself is only recomputed on the next save.
#[gpui::test]
fn bars_follow_the_text_between_saves(cx: &mut TestAppContext) {
    let (view, mut cx) = open(cx, EditorSettings::default());
    set_git(
        &view,
        &mut cx,
        Some(vec![
            hunk(GitGutterKind::Modified, 5..6),
            hunk(GitGutterKind::Deleted, 8..8),
        ]),
    );
    cx.update(|_window, cx| {
        view.update(cx, |view, cx| {
            // End of row 1 (zero based), then three new lines.
            view.set_cursor(Point::new(1, 7), cx);
            view.insert_text("\nuno\ndos\ntres", cx);
        })
    });
    cx.run_until_parked();

    let hunks = cx.update(|_, cx| view.read(cx).git_diff()).unwrap();
    assert_eq!(
        hunks,
        vec![
            hunk(GitGutterKind::Modified, 8..9),
            hunk(GitGutterKind::Deleted, 11..11),
        ]
    );
    let frame = frame(&view, &mut cx);
    for (row, bar) in bars(&frame) {
        assert_eq!(bar.is_some(), row == 8, "fila {row}");
    }

    // Undo brings them back.
    cx.update(|window, cx| {
        view.update(cx, |_, cx| {
            window.dispatch_action(Box::new(crate::actions::Undo), cx);
        })
    });
    cx.run_until_parked();
    let hunks = cx.update(|_, cx| view.read(cx).git_diff()).unwrap();
    assert_eq!(hunks[0], hunk(GitGutterKind::Modified, 5..6));

    // `None` clears the column.
    set_git(&view, &mut cx, None);
    assert_eq!(cx.update(|_, cx| view.read(cx).git_diff()), None);
    assert!(frame_of(&view, &mut cx).git_marks.is_empty());
}

/// §6.1: phantom rows never get a git bar; a git bar and the agent's bar
/// share a real row, each in its column.
#[gpui::test]
fn phantom_rows_have_no_git_bar_and_both_bars_coexist(cx: &mut TestAppContext) {
    let (view, mut cx) = open(cx, EditorSettings::default());
    // The agent replaced row 3; git sees rows 2..5 as modified.
    set_review(&view, &mut cx, review_on(3));
    set_git(
        &view,
        &mut cx,
        Some(vec![hunk(GitGutterKind::Modified, 2..5)]),
    );
    let frame = frame(&view, &mut cx);
    let phantom: Vec<_> = frame
        .rows
        .iter()
        .filter(|row| matches!(row.kind, crate::display_map::RowKind::Phantom(_)))
        .collect();
    assert_eq!(phantom.len(), 1);
    assert!(phantom.iter().all(|row| row.git_bar.is_none()));
    // Display rows: 0 1 2 [phantom 3] 4(=buffer 3, added) 5(=4) …
    let added: Vec<_> = frame
        .rows
        .iter()
        .filter(|row| matches!(row.kind, crate::display_map::RowKind::Added(_)))
        .collect();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].git_bar, Some(GitGutterKind::Modified));
    let with_bar: Vec<u32> = bars(&frame)
        .into_iter()
        .filter_map(|(row, bar)| bar.map(|_| row))
        .collect();
    assert_eq!(with_bar, vec![2, 4, 5]);
    // Three bars, none of them on the phantom row's y.
    assert_eq!(marks(&frame, GitGutterKind::Modified).len(), 3);

    // A deletion right below the phantom row stays inside the real row.
    set_git(
        &view,
        &mut cx,
        Some(vec![hunk(GitGutterKind::Deleted, 3..3)]),
    );
    let frame = frame_of(&view, &mut cx);
    let line_height = cx.update(|_, cx| view.read(cx).style.line_height);
    let deleted = marks(&frame, GitGutterKind::Deleted);
    assert_eq!(deleted.len(), 1);
    // Buffer row 3 is display row 4, below the phantom row 3.
    assert_eq!(deleted[0].top(), frame.bounds.top() + line_height * 4.);
}

#[gpui::test]
fn the_minimal_chrome_has_no_git_column(cx: &mut TestAppContext) {
    let settings = EditorSettings {
        chrome: EditorChrome::Minimal,
        ..EditorSettings::default()
    };
    let (view, mut cx) = open(cx, settings);
    set_git(&view, &mut cx, Some(vec![hunk(GitGutterKind::Added, 0..3)]));
    let frame = frame(&view, &mut cx);
    assert_eq!(frame.gutter_width, px(0.));
    assert!(frame.git_marks.is_empty());
}

#[gpui::test]
fn the_colors_come_from_the_host(cx: &mut TestAppContext) {
    let (view, mut cx) = open(cx, EditorSettings::default());
    assert_eq!(
        cx.update(|_, cx| view.read(cx).git_colors()),
        GitGutterColors::default()
    );
    let light = GitGutterColors {
        added: gpui::rgb(0x50a14f),
        modified: gpui::rgb(0x4078f2),
        deleted: gpui::rgb(0xe45649),
    };
    cx.update(|_, cx| view.update(cx, |view, cx| view.set_git_colors(light, cx)));
    assert_eq!(cx.update(|_, cx| view.read(cx).git_colors()), light);
    assert_eq!(light.color(GitGutterKind::Modified), gpui::rgb(0x4078f2));
}
