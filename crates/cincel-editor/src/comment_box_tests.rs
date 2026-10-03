//! The comment box of the editor over the GPUI test harness
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §6.5, §6.11):
//! the pill's "Comentar" (normal and compact, in its hover zone and coming
//! from above), `Ctrl+Shift+M` on a selection, the context menu, `Ctrl+Enter`
//! / `Esc` in the box (and `Ctrl+Enter` never accepting the hunk), folding,
//! "Editar" / "Borrar", the margin mark, nothing moving the text (D16) and
//! nothing at all in a text field.
//!
//! A fake host answers the editor's [`CommentAction`]s the way the workspace
//! will (E7-F): it keeps the list and calls `set_comments` back.

// One-range vectors are what a one-row comment looks like.
#![allow(clippy::single_range_in_vec_init)]

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use cincel_syntax::LanguageRegistry;
use cincel_text::Buffer;
use gpui::{
    AnyWindowHandle, Bounds, Entity, Modifiers, MouseButton, Pixels, TestAppContext,
    VisualTestContext, point, px,
};

use crate::actions::bind_default_keys;
use crate::comments::{
    CommentAction, CommentBlockKind, DraftTarget, NEW_COMMENT_BLOCK, ReviewCommentView,
};
use crate::display_map::DisplayPoint;
use crate::element::{COMPACT_PILL_BUTTON, COMPACT_PILL_GAP, GUTTER_TEXT_GAP};
use crate::review::{ReviewAction, ReviewHunkKind, ReviewHunkView, ReviewLineView, ReviewView};
use crate::settings::{EditorChrome, EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::{EditorEvent, EditorView, FrameRender};

/// What the fake host saw and holds.
#[derive(Default)]
pub(crate) struct Host {
    pub actions: Vec<CommentAction>,
    pub review: Vec<ReviewAction>,
    pub comments: Vec<ReviewCommentView>,
    next_id: u64,
}

pub(crate) type SharedHost = Rc<RefCell<Host>>;

/// A hunk: `deleted` lines spliced before `rows.start`, paired line by line.
pub(crate) fn hunk(id: u64, deleted: &[&str], rows: Range<u32>) -> ReviewHunkView {
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

pub(crate) fn review(hunks: Vec<ReviewHunkView>) -> ReviewView {
    ReviewView {
        pending_in_file: hunks.len(),
        hunks,
        ..Default::default()
    }
}

pub(crate) fn comment(id: u64, rows: Range<u32>, text: &str) -> ReviewCommentView {
    ReviewCommentView {
        id,
        rows,
        text: text.to_string(),
        from_hunk: None,
    }
}

/// Opens a focused editor with the render probe on, `review` applied and a
/// fake host answering its comment actions.
pub(crate) fn open(
    cx: &mut TestAppContext,
    text: &str,
    review: ReviewView,
    settings: EditorSettings,
) -> (
    Entity<EditorView>,
    AnyWindowHandle,
    VisualTestContext,
    SharedHost,
) {
    cx.update(|cx| {
        // The boxes' buttons and the context menu are gpui-kit's.
        gpui_kit::init(cx);
        bind_default_keys(cx);
    });
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
    let host: SharedHost = Rc::new(RefCell::new(Host {
        next_id: 100,
        ..Default::default()
    }));
    let sink = host.clone();
    let review_sink = host.clone();
    cx.update(|cx| {
        cx.subscribe(&view, move |view, action: &CommentAction, cx| {
            let comments = {
                let mut host = sink.borrow_mut();
                host.actions.push(action.clone());
                match action.clone() {
                    CommentAction::Create {
                        rows,
                        from_hunk,
                        text,
                    } => {
                        let id = host.next_id;
                        host.next_id += 1;
                        host.comments.push(ReviewCommentView {
                            id,
                            rows,
                            text,
                            from_hunk,
                        });
                    }
                    CommentAction::Edit { id, text } => {
                        if let Some(comment) = host.comments.iter_mut().find(|c| c.id == id) {
                            comment.text = text;
                        }
                    }
                    CommentAction::Delete { id } => host.comments.retain(|c| c.id != id),
                }
                host.comments.clone()
            };
            view.update(cx, |view, cx| view.set_comments(comments, cx));
        })
        .detach();
        cx.subscribe(&view, move |_, event: &EditorEvent, _| {
            if let EditorEvent::Review(action) = event {
                review_sink.borrow_mut().review.push(*action);
            }
        })
        .detach();
    });
    window
        .update(cx, |view, window, cx| {
            window.activate_window();
            window.focus(&view.focus_handle, cx);
            view.set_render_probe(true);
            view.set_display_name(Some("calc.py".into()), cx);
            view.set_review(review, cx);
        })
        .unwrap();
    visual.run_until_parked();
    (view, handle, visual, host)
}

/// Hands the host (and the editor) a list of comments.
pub(crate) fn set_comments(
    view: &Entity<EditorView>,
    host: &SharedHost,
    cx: &mut VisualTestContext,
    comments: Vec<ReviewCommentView>,
) {
    host.borrow_mut().comments = comments.clone();
    cx.update(|_window, cx| view.update(cx, |view, cx| view.set_comments(comments, cx)));
    cx.run_until_parked();
}

pub(crate) fn set_cursor(
    view: &Entity<EditorView>,
    cx: &mut VisualTestContext,
    point: DisplayPoint,
) {
    cx.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.cursor = view.clip_point(point);
            view.selection_anchor = view.cursor;
            cx.notify();
        })
    });
    cx.run_until_parked();
}

pub(crate) fn select(
    view: &Entity<EditorView>,
    cx: &mut VisualTestContext,
    from: DisplayPoint,
    to: DisplayPoint,
) {
    cx.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.selection_anchor = view.clip_point(from);
            view.cursor = view.clip_point(to);
            cx.notify();
        })
    });
    cx.run_until_parked();
}

/// Forces a repaint and returns the frame it painted.
pub(crate) fn frame(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> FrameRender {
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

/// `(line height, text origin x, element bounds)` of the last layout.
pub(crate) fn geometry(
    view: &Entity<EditorView>,
    cx: &mut VisualTestContext,
) -> (f32, f32, Bounds<Pixels>) {
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

pub(crate) fn hover(cx: &mut VisualTestContext, x: f32, y: f32) {
    cx.simulate_mouse_move(point(px(x), px(y)), None, Modifiers::default());
    cx.run_until_parked();
}

pub(crate) fn click(cx: &mut VisualTestContext, x: f32, y: f32) {
    hover(cx, x, y);
    cx.simulate_click(point(px(x), px(y)), Modifiers::default());
    cx.run_until_parked();
}

pub(crate) fn click_bounds(cx: &mut VisualTestContext, bounds: Bounds<Pixels>) {
    let center = bounds.center();
    click(cx, f32::from(center.x), f32::from(center.y));
}

/// Clicks the element tagged `selector` (a debug selector of the boxes).
pub(crate) fn click_selector(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} was painted"));
    click_bounds(cx, bounds);
}

pub(crate) fn draft(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Option<DraftTarget> {
    cx.update(|_window, cx| view.read(cx).comment_draft().cloned())
}

pub(crate) fn field_text(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_window, cx| {
        view.read(cx)
            .comment_draft_editor()
            .map(|editor| editor.read(cx).text())
    })
}

pub(crate) fn field_focused(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        view.read(cx)
            .comment_draft_editor()
            .is_some_and(|editor| editor.read(cx).focus_handle.is_focused(window))
    })
}

const TEXT: &str = "fn total(items) {\n    sum(items)\n}\n\nfn otra() {}\nfin\n";

/// One hunk: "viejo" replaced by buffer row 1.
fn one_hunk() -> ReviewView {
    review(vec![hunk(40, &["    viejo(items)"], 1..2)])
}

fn open_default(
    cx: &mut TestAppContext,
) -> (
    Entity<EditorView>,
    AnyWindowHandle,
    VisualTestContext,
    SharedHost,
) {
    open(cx, TEXT, one_hunk(), EditorSettings::default())
}

// -- the pill's "Comentar" -------------------------------------------------------

#[gpui::test]
fn the_pill_has_three_equal_parts_and_comentar_opens_the_box_below_the_hunk(
    cx: &mut TestAppContext,
) {
    let (view, _handle, mut visual, host) = open_default(cx);
    // Display rows: 0 fn, 1 viejo (red), 2 sum (green), 3 }, …
    set_cursor(&view, &mut visual, DisplayPoint::new(2, 0));
    let painted = frame(&view, &mut visual);
    let pill = painted.review.pills[0];
    assert!(!pill.compact);
    assert_eq!(pill.bounds.size.width, px(crate::element::PILL_WIDTH));
    let widths = [pill.accept_bounds, pill.reject_bounds, pill.comment_bounds]
        .map(|part| f32::from(part.size.width));
    assert!((widths[0] - widths[1]).abs() < 0.5 && (widths[1] - widths[2]).abs() < 0.5);
    assert!(widths[0] * 3. > f32::from(pill.bounds.size.width) - 10.);
    assert!(pill.accept_bounds.right() <= pill.reject_bounds.left() + px(0.5));
    assert!(pill.reject_bounds.right() <= pill.comment_bounds.left() + px(0.5));
    let theme = EditorTheme::default();
    assert_eq!(
        pill.comment_icon_color,
        crate::theme::color(theme.text_accent)
    );
    assert_eq!(pill.comment_label_color, crate::theme::color(theme.text));

    click_bounds(&mut visual, pill.comment_bounds);
    assert_eq!(
        draft(&view, &mut visual),
        Some(DraftTarget::New {
            rows: 1..2,
            from_hunk: Some(40),
        })
    );
    assert!(
        field_focused(&view, &mut visual),
        "the field has the keyboard"
    );
    assert!(
        host.borrow().review.is_empty(),
        "commenting decides nothing"
    );

    let painted = frame(&view, &mut visual);
    let block = painted.review.blocks[0];
    assert_eq!(block.id, NEW_COMMENT_BLOCK);
    assert_eq!(block.kind, CommentBlockKind::Open);
    // Right below the hunk's last row (display row 2, visual row 2).
    assert_eq!(block.visual_row, 3);
    let title = visual.debug_bounds("comment-box-title");
    assert!(title.is_some(), "the box has its title");
}

#[gpui::test]
fn the_compact_pill_has_a_third_button_that_comments(cx: &mut TestAppContext) {
    let long = "x".repeat(400);
    let text = format!("{long}\n{long}\n{long}\nc\n");
    let (view, _handle, mut visual, _host) = open(
        cx,
        &text,
        review(vec![hunk(7, &[], 1..3)]),
        EditorSettings::default(),
    );
    set_cursor(&view, &mut visual, DisplayPoint::new(2, 0));
    let pill = frame(&view, &mut visual).review.pills[0];
    assert!(pill.compact);
    assert_eq!(
        pill.bounds.size.width,
        px(3. * COMPACT_PILL_BUTTON + 2. * COMPACT_PILL_GAP)
    );
    assert_eq!(pill.comment_bounds.right(), pill.bounds.right());
    click_bounds(&mut visual, pill.comment_bounds);
    assert_eq!(
        draft(&view, &mut visual),
        Some(DraftTarget::New {
            rows: 1..3,
            from_hunk: Some(7),
        })
    );
}

/// The pill sits on the row above its hunk (no room on the hunk's own row):
/// the mouse going up from the hunk to "Comentar" keeps it up, and the
/// click opens the box.
#[gpui::test]
fn comentar_shows_in_the_hover_zone_and_when_reached_from_above(cx: &mut TestAppContext) {
    let long = "x".repeat(400);
    let text = format!("arriba\ncorto\n{long}\nfin\n");
    let (view, _handle, mut visual, _host) = open(
        cx,
        &text,
        review(vec![hunk(10, &[], 2..3)]),
        EditorSettings::default(),
    );
    let (line_height, _, _) = geometry(&view, &mut visual);

    hover(&mut visual, 300., line_height * 0.5);
    assert!(frame(&view, &mut visual).review.pills.is_empty());

    hover(&mut visual, 300., line_height * 2.5);
    let pill = frame(&view, &mut visual).review.pills[0];
    assert_eq!(pill.display_row, 1, "on the row above the hunk");
    let center = pill.comment_bounds.center();
    assert!(
        f32::from(center.y) < line_height * 2.,
        "outside the hunk's rows"
    );

    hover(&mut visual, f32::from(center.x), f32::from(center.y));
    let painted = frame(&view, &mut visual);
    assert_eq!(painted.review.pills.len(), 1, "the pill stays");
    assert!(painted.review.pills[0].comment_hovered);
    // Coming down from further above, onto its top edge.
    hover(
        &mut visual,
        f32::from(center.x),
        f32::from(pill.bounds.top()) + 1.,
    );
    assert_eq!(frame(&view, &mut visual).review.pills.len(), 1);

    click(&mut visual, f32::from(center.x), f32::from(center.y));
    assert!(matches!(
        draft(&view, &mut visual),
        Some(DraftTarget::New {
            from_hunk: Some(10),
            ..
        })
    ));
}

#[gpui::test]
fn comentar_works_while_the_agent_writes(cx: &mut TestAppContext) {
    let mut active = one_hunk();
    active.turn_active = true;
    let (view, _handle, mut visual, host) = open(cx, TEXT, active, EditorSettings::default());
    set_cursor(&view, &mut visual, DisplayPoint::new(2, 0));
    let pill = frame(&view, &mut visual).review.pills[0];
    assert!(!pill.enabled, "accept and reject are disabled");
    click_bounds(&mut visual, pill.accept_bounds);
    assert!(host.borrow().review.is_empty());
    click_bounds(&mut visual, pill.comment_bounds);
    assert!(draft(&view, &mut visual).is_some(), "Comentar still reacts");
}

// -- Ctrl+Shift+M and the box's keys ---------------------------------------------

#[gpui::test]
fn ctrl_shift_m_comments_the_rows_of_the_selection(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open_default(cx);
    // From display row 3 ("}") to the start of display row 5: rows 2..4 of
    // the buffer — the row where the selection ends at column 0 does not
    // count.
    select(
        &view,
        &mut visual,
        DisplayPoint::new(3, 0),
        DisplayPoint::new(5, 0),
    );
    visual.simulate_keystrokes("ctrl-shift-m");
    assert_eq!(
        draft(&view, &mut visual),
        Some(DraftTarget::New {
            rows: 2..4,
            from_hunk: None,
        })
    );
    let block = frame(&view, &mut visual).review.blocks[0];
    // Below buffer row 3 (display row 4).
    assert_eq!(block.visual_row, 5);
}

#[gpui::test]
fn ctrl_shift_m_without_a_selection_inside_a_hunk_is_its_comentar(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open_default(cx);
    // On the red row: the hunk's own comment.
    set_cursor(&view, &mut visual, DisplayPoint::new(1, 2));
    visual.simulate_keystrokes("ctrl-shift-m");
    assert_eq!(
        draft(&view, &mut visual),
        Some(DraftTarget::New {
            rows: 1..2,
            from_hunk: Some(40),
        })
    );
    // Outside any hunk: the cursor's row.
    visual.simulate_keystrokes("escape");
    set_cursor(&view, &mut visual, DisplayPoint::new(5, 1));
    visual.simulate_keystrokes("ctrl-shift-m");
    assert_eq!(
        draft(&view, &mut visual),
        Some(DraftTarget::New {
            rows: 4..5,
            from_hunk: None,
        })
    );
}

#[gpui::test]
fn ctrl_enter_saves_the_comment_and_never_accepts_the_hunk(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open_default(cx);
    set_cursor(&view, &mut visual, DisplayPoint::new(2, 0));
    visual.simulate_keystrokes("ctrl-shift-m");
    visual.simulate_input("usá un diccionario");
    // `Enter` is a line break in the box.
    visual.simulate_keystrokes("enter");
    visual.simulate_input("y probalo");
    assert_eq!(
        field_text(&view, &mut visual).as_deref(),
        Some("usá un diccionario\ny probalo")
    );
    visual.simulate_keystrokes("ctrl-enter");

    assert_eq!(
        host.borrow().actions,
        vec![CommentAction::Create {
            rows: 1..2,
            from_hunk: Some(40),
            text: "usá un diccionario\ny probalo".into(),
        }]
    );
    assert!(host.borrow().review.is_empty(), "the hunk was not accepted");
    assert_eq!(draft(&view, &mut visual), None);
    let focused = visual.update(|window, cx| view.read(cx).focus_handle.is_focused(window));
    assert!(focused, "the keyboard goes back to the code");

    // The host answered: the comment shows folded, with its mark.
    let painted = frame(&view, &mut visual);
    let block = painted.review.blocks[0];
    assert_eq!(block.id, 100);
    assert_eq!(block.kind, CommentBlockKind::Folded);
    assert_eq!(painted.review.comment_marks.len(), 1);
    // The mark is on the first row of the hunk (its red row).
    assert_eq!(painted.review.comment_marks[0].display_row, 1);
    // The cursor is still in the hunk: Ctrl+Enter accepts it now.
    visual.simulate_keystrokes("ctrl-enter");
    assert_eq!(host.borrow().review, vec![ReviewAction::AcceptHunk(40)]);
}

#[gpui::test]
fn the_buttons_save_and_cancel_too(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open_default(cx);
    set_cursor(&view, &mut visual, DisplayPoint::new(4, 0));
    visual.simulate_keystrokes("ctrl-shift-m");
    visual.simulate_input("algo");
    click_selector(&mut visual, "comment-box-cancel");
    assert_eq!(draft(&view, &mut visual), None);
    assert!(host.borrow().actions.is_empty(), "Cancelar sends nothing");

    visual.simulate_keystrokes("ctrl-shift-m");
    assert_eq!(
        field_text(&view, &mut visual).as_deref(),
        Some(""),
        "a new box starts empty"
    );
    visual.simulate_input("guardado");
    click_selector(&mut visual, "comment-box-save");
    assert_eq!(
        host.borrow().actions,
        vec![CommentAction::Create {
            rows: 3..4,
            from_hunk: None,
            text: "guardado".into(),
        }]
    );
}

#[gpui::test]
fn escape_cancels_and_saving_empty_creates_nothing(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open_default(cx);
    set_cursor(&view, &mut visual, DisplayPoint::new(4, 0));
    visual.simulate_keystrokes("ctrl-shift-m");
    visual.simulate_input("se descarta");
    visual.simulate_keystrokes("escape");
    assert_eq!(draft(&view, &mut visual), None);
    assert!(frame(&view, &mut visual).review.blocks.is_empty());

    visual.simulate_keystrokes("ctrl-shift-m");
    visual.simulate_input("   ");
    visual.simulate_keystrokes("ctrl-enter");
    assert_eq!(draft(&view, &mut visual), None);
    assert!(host.borrow().actions.is_empty(), "empty is cancel");
}

#[gpui::test]
fn a_hunk_with_a_comment_reopens_it_for_editing(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open_default(cx);
    set_comments(
        &view,
        &host,
        &mut visual,
        vec![ReviewCommentView {
            from_hunk: Some(40),
            ..comment(7, 1..2, "primero")
        }],
    );
    set_cursor(&view, &mut visual, DisplayPoint::new(2, 0));
    let pill = frame(&view, &mut visual).review.pills[0];
    click_bounds(&mut visual, pill.comment_bounds);
    assert_eq!(draft(&view, &mut visual), Some(DraftTarget::Edit { id: 7 }));
    assert_eq!(field_text(&view, &mut visual).as_deref(), Some("primero"));
    let block = frame(&view, &mut visual).review.blocks[0];
    assert_eq!((block.id, block.kind), (7, CommentBlockKind::Open));
}

// -- saved comments: fold, edit, delete, mark --------------------------------------

#[gpui::test]
fn editar_changes_the_text_escape_keeps_it_and_emptying_deletes(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open_default(cx);
    set_comments(
        &view,
        &host,
        &mut visual,
        vec![comment(7, 4..5, "hacelo así")],
    );
    // The buttons show with the cursor in the comment's rows.
    set_cursor(&view, &mut visual, DisplayPoint::new(5, 0));
    click_selector(&mut visual, "comment-edit-7");
    assert_eq!(draft(&view, &mut visual), Some(DraftTarget::Edit { id: 7 }));
    assert!(field_focused(&view, &mut visual));

    // Esc: the comment stays as it was.
    visual.simulate_input(" y más");
    visual.simulate_keystrokes("escape");
    assert!(host.borrow().actions.is_empty());
    assert_eq!(host.borrow().comments[0].text, "hacelo así");

    // Edited and saved.
    click_selector(&mut visual, "comment-edit-7");
    visual.simulate_keystrokes("ctrl-a");
    visual.simulate_input("mejor así");
    visual.simulate_keystrokes("ctrl-enter");
    assert_eq!(
        host.borrow().actions,
        vec![CommentAction::Edit {
            id: 7,
            text: "mejor así".into(),
        }]
    );

    // Emptied and saved: deleted.
    click_selector(&mut visual, "comment-edit-7");
    visual.simulate_keystrokes("ctrl-a backspace ctrl-enter");
    assert_eq!(
        host.borrow().actions.last(),
        Some(&CommentAction::Delete { id: 7 })
    );
    assert!(frame(&view, &mut visual).review.blocks.is_empty());
    assert!(frame(&view, &mut visual).review.comment_marks.is_empty());
}

#[gpui::test]
fn borrar_deletes_and_the_buttons_show_on_hover(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open_default(cx);
    set_comments(&view, &host, &mut visual, vec![comment(7, 4..5, "fuera")]);
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    frame(&view, &mut visual);
    assert!(
        visual.debug_bounds("comment-delete-7").is_none(),
        "no buttons with the mouse away and the cursor elsewhere"
    );
    let block = visual
        .debug_bounds("comment-block-7")
        .expect("the folded box");
    hover(
        &mut visual,
        f32::from(block.center().x),
        f32::from(block.center().y),
    );
    frame(&view, &mut visual);
    let delete = visual
        .debug_bounds("comment-delete-7")
        .expect("Borrar shows on hover");
    assert!(visual.debug_bounds("comment-edit-7").is_some());
    click_bounds(&mut visual, delete);
    assert_eq!(host.borrow().actions, vec![CommentAction::Delete { id: 7 }]);
    assert!(frame(&view, &mut visual).review.blocks.is_empty());
}

#[gpui::test]
fn the_text_and_the_mark_fold_and_unfold(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open_default(cx);
    let long = "palabra ".repeat(120);
    set_comments(&view, &host, &mut visual, vec![comment(7, 4..5, &long)]);
    let folded = frame(&view, &mut visual).review.blocks[0];
    assert_eq!(folded.kind, CommentBlockKind::Folded);

    click_selector(&mut visual, "comment-text-7");
    let expanded = frame(&view, &mut visual);
    let block = expanded.review.blocks[0];
    assert_eq!(block.kind, CommentBlockKind::Expanded);
    assert!(block.rows > folded.rows, "the whole text takes more rows");
    assert!(visual.update(|_, cx| view.read(cx).is_comment_expanded(7)));

    // The mark folds it again.
    let mark = expanded.review.comment_marks[0];
    click_bounds(&mut visual, mark.bounds);
    assert_eq!(
        frame(&view, &mut visual).review.blocks[0].kind,
        CommentBlockKind::Folded
    );
    // And unfolds it.
    click_bounds(&mut visual, mark.bounds);
    assert_eq!(
        frame(&view, &mut visual).review.blocks[0].kind,
        CommentBlockKind::Expanded
    );
}

#[gpui::test]
fn the_mark_sits_in_the_gap_and_shows_the_comment(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open_default(cx);
    let before = frame(&view, &mut visual);
    let text = format!("{}fin", "a".repeat(230));
    set_comments(&view, &host, &mut visual, vec![comment(7, 4..6, &text)]);
    let painted = frame(&view, &mut visual);
    assert_eq!(painted.gutter_width, before.gutter_width);
    assert_eq!(painted.text_origin_x, before.text_origin_x);
    let mark = painted.review.comment_marks[0];
    // Buffer row 4 is display row 5 (one red row above).
    assert_eq!(mark.display_row, 5);
    let (_, numbers_right) = painted.review.number_column;
    let center = f32::from(mark.bounds.center().x);
    let gap_center = f32::from(numbers_right) + GUTTER_TEXT_GAP / 2.;
    assert!((center - gap_center).abs() < 0.5, "centred in the gap");
    assert!(mark.bounds.left() >= numbers_right);
    assert!(mark.bounds.right() <= painted.text_origin_x);
    assert_eq!(
        mark.bounds.size.width,
        px(crate::element::COMMENT_MARK_SIZE)
    );

    hover(
        &mut visual,
        f32::from(mark.bounds.center().x),
        f32::from(mark.bounds.center().y),
    );
    let tooltip = frame(&view, &mut visual).review.tooltip.expect("a tooltip");
    assert_eq!(
        tooltip.chars().count(),
        201,
        "200 characters and the ellipsis"
    );
    assert!(tooltip.starts_with("aaaa"));
}

// -- D16: nothing moves ---------------------------------------------------------------

#[gpui::test]
fn opening_and_closing_a_box_never_moves_the_text(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, host) = open_default(cx);
    let (line_height, _, _) = geometry(&view, &mut visual);
    set_cursor(&view, &mut visual, DisplayPoint::new(3, 0));
    let before = frame(&view, &mut visual);
    let y_of = |frame: &FrameRender, row: u32| frame.row(row).expect("painted").y;

    visual.simulate_keystrokes("ctrl-shift-m");
    let open = frame(&view, &mut visual);
    assert_eq!(
        open.gutter_width, before.gutter_width,
        "the gutter keeps its width"
    );
    assert_eq!(
        open.text_origin_x, before.text_origin_x,
        "the text keeps its column"
    );
    let block = open.review.blocks[0];
    let box_bounds = visual.debug_bounds("comment-box").expect("the open box");
    // The layout snaps to whole pixels.
    assert!(
        (f32::from(box_bounds.left()) - f32::from(open.text_origin_x)).abs() < 1.,
        "{box_bounds:?} vs {:?}",
        open.text_origin_x
    );
    assert_eq!(block.bounds.left(), open.text_origin_x);
    // 12 px before the scrollbar (8 px).
    assert_eq!(
        block.bounds.right(),
        open.bounds.right() - px(8. + crate::view::COMMENT_BOX_RIGHT_MARGIN)
    );
    assert!(box_bounds.size.height <= block.bounds.size.height);
    assert_eq!(
        block.bounds.size.height,
        px(line_height * block.rows as f32)
    );
    // Whole rows: the rows below moved down exactly by them, the ones above
    // did not move.
    let rows_frame = |frame: &FrameRender| frame.visual_rows - frame.wrap_rows;
    assert_eq!(rows_frame(&open), block.rows);
    assert_eq!(block.visual_row, 4);
    for row in 0..=3 {
        assert_eq!(
            y_of(&open, row),
            y_of(&before, row),
            "row {row} above the box"
        );
    }
    for row in 4..=6 {
        assert_eq!(
            y_of(&open, row),
            y_of(&before, row) + px(line_height * block.rows as f32),
            "row {row} below the box"
        );
    }
    // Same x for every row: the text never moves sideways.
    for (a, b) in open.rows.iter().zip(&before.rows) {
        assert_eq!(a.display_row, b.display_row);
        assert_eq!(a.line_number, b.line_number);
    }

    visual.simulate_keystrokes("escape");
    let closed = frame(&view, &mut visual);
    assert_eq!(closed.gutter_width, before.gutter_width);
    assert_eq!(closed.text_origin_x, before.text_origin_x);
    assert_eq!(closed.visual_rows, before.visual_rows);
    for row in 0..=6 {
        assert_eq!(y_of(&closed, row), y_of(&before, row));
    }
    assert!(host.borrow().actions.is_empty());
}

// -- the context menu -----------------------------------------------------------------

#[gpui::test]
fn the_context_menu_has_the_minimum_entries(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open_default(cx);
    let labels = |view: &Entity<EditorView>, cx: &mut VisualTestContext| -> Vec<Option<String>> {
        cx.update(|_, cx| {
            view.read(cx)
                .context_menu_entries()
                .into_iter()
                .map(|entry| entry.map(|(label, action)| format!("{label} {}", action.name())))
                .collect()
        })
    };
    set_cursor(&view, &mut visual, DisplayPoint::new(0, 0));
    assert_eq!(
        labels(&view, &mut visual),
        vec![
            Some("Cortar editor::cut".into()),
            Some("Copiar editor::copy".into()),
            Some("Pegar editor::paste".into()),
            None,
            Some("Comentar línea editor::comment_selection".into()),
        ]
    );
    select(
        &view,
        &mut visual,
        DisplayPoint::new(0, 0),
        DisplayPoint::new(0, 2),
    );
    assert_eq!(
        labels(&view, &mut visual)
            .last()
            .cloned()
            .flatten()
            .as_deref(),
        Some("Comentar selección editor::comment_selection")
    );
    // A read-only tab (a deleted file): only Copiar and Comentar selección.
    visual.update(|_, cx| view.update(cx, |view, cx| view.set_read_only(true, cx)));
    assert_eq!(
        labels(&view, &mut visual),
        vec![
            Some("Copiar editor::copy".into()),
            Some("Comentar selección editor::comment_selection".into()),
        ]
    );
}

#[gpui::test]
fn a_right_click_in_the_selection_keeps_it_and_comentar_seleccion_opens_the_box(
    cx: &mut TestAppContext,
) {
    let (view, _handle, mut visual, _host) = open_default(cx);
    let (line_height, text_x, _) = geometry(&view, &mut visual);
    select(
        &view,
        &mut visual,
        DisplayPoint::new(3, 0),
        DisplayPoint::new(5, 3),
    );
    // Inside the selection (display row 4).
    let at = point(px(text_x + 2.), px(line_height * 4.5));
    visual.simulate_mouse_move(at, None, Modifiers::default());
    visual.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
    visual.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
    visual.run_until_parked();
    let selection = visual.update(|_, cx| view.read(cx).selection_range());
    assert_eq!(
        selection.start,
        DisplayPoint::new(3, 0),
        "the selection stays"
    );
    assert_eq!(selection.end, DisplayPoint::new(5, 3));
    // The last entry of the menu, from the keyboard.
    visual.simulate_keystrokes("up enter");
    assert_eq!(
        draft(&view, &mut visual),
        Some(DraftTarget::New {
            rows: 2..5,
            from_hunk: None,
        })
    );
    assert!(field_focused(&view, &mut visual));
}

#[gpui::test]
fn a_right_click_outside_the_selection_moves_the_cursor_first(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open_default(cx);
    let (line_height, text_x, _) = geometry(&view, &mut visual);
    select(
        &view,
        &mut visual,
        DisplayPoint::new(0, 0),
        DisplayPoint::new(0, 4),
    );
    let at = point(px(text_x + 2.), px(line_height * 5.5));
    visual.simulate_mouse_move(at, None, Modifiers::default());
    visual.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
    visual.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
    visual.run_until_parked();
    let (has_selection, cursor) =
        visual.update(|_, cx| (view.read(cx).has_selection(), view.read(cx).cursor));
    assert!(!has_selection);
    assert_eq!(cursor.row, 5);
    // "Comentar línea": the row clicked.
    visual.simulate_keystrokes("up enter");
    assert_eq!(
        draft(&view, &mut visual),
        Some(DraftTarget::New {
            rows: 4..5,
            from_hunk: None,
        })
    );
}

// -- limits, deleted files, text fields -----------------------------------------------

#[gpui::test]
fn a_comment_has_at_most_8000_characters(cx: &mut TestAppContext) {
    let (view, _handle, mut visual, _host) = open_default(cx);
    set_cursor(&view, &mut visual, DisplayPoint::new(4, 0));
    visual.simulate_keystrokes("ctrl-shift-m");
    // Short lines (one giant line would only measure the wrap, slowly).
    let line = format!("{}\n", "ñ".repeat(59));
    visual.simulate_input(&line.repeat(125));
    assert_eq!(
        field_text(&view, &mut visual).unwrap().chars().count(),
        7_500
    );
    frame(&view, &mut visual);
    assert!(visual.debug_bounds("comment-box-help").is_some());
    visual.simulate_input(&line.repeat(10));
    let text = field_text(&view, &mut visual).unwrap();
    assert_eq!(text.chars().count(), crate::comments::COMMENT_MAX_CHARS);
    // The field stops at 8 rows and scrolls.
    let block = frame(&view, &mut visual).review.blocks[0];
    let (line_height, _, _) = geometry(&view, &mut visual);
    let max = crate::view::open_box_height(8, 1.);
    assert_eq!(block.rows, crate::view::block_rows(max, line_height));
}

#[gpui::test]
fn the_deleted_file_tab_comments_its_single_hunk(cx: &mut TestAppContext) {
    let deleted = review(vec![hunk(99, &["uno", "dos", "tres"], 0..0)]);
    let (view, _handle, mut visual, _host) = open(cx, "", deleted, EditorSettings::default());
    visual.update(|_, cx| view.update(cx, |view, cx| view.set_read_only(true, cx)));
    select(
        &view,
        &mut visual,
        DisplayPoint::new(0, 0),
        DisplayPoint::new(1, 2),
    );
    visual.simulate_keystrokes("ctrl-shift-m");
    assert_eq!(
        draft(&view, &mut visual),
        Some(DraftTarget::New {
            rows: 0..0,
            from_hunk: Some(99),
        })
    );
    // Below its red rows.
    let block = frame(&view, &mut visual).review.blocks[0];
    assert_eq!(block.visual_row, 3);
}

#[gpui::test]
fn a_text_field_takes_no_comments(cx: &mut TestAppContext) {
    let settings = EditorSettings {
        chrome: EditorChrome::Minimal,
        ..EditorSettings::default()
    };
    let (view, _handle, mut visual, host) =
        open(cx, "hola\nmundo\n", ReviewView::default(), settings);
    select(
        &view,
        &mut visual,
        DisplayPoint::new(0, 0),
        DisplayPoint::new(1, 2),
    );
    visual.simulate_keystrokes("ctrl-shift-m");
    assert_eq!(draft(&view, &mut visual), None);
    set_comments(&view, &host, &mut visual, vec![comment(1, 0..1, "no")]);
    let painted = frame(&view, &mut visual);
    assert!(painted.review.blocks.is_empty());
    assert!(painted.review.comment_marks.is_empty());
    assert_eq!(painted.visual_rows, painted.wrap_rows);
    let entries = visual.update(|_, cx| view.read(cx).context_menu_entries().len());
    assert_eq!(entries, 0, "no context menu");
}
