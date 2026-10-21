//! The floating bar shrinks by levels with the width of the editor (spec 11
//! §3): the full bar, then without the written shortcuts, then icons only —
//! always whole inside the element, 30 px high, bottom right.

use std::sync::Arc;

use cincel_syntax::LanguageRegistry;
use cincel_text::Buffer;
use gpui::{
    AnyWindowHandle, Bounds, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext, point,
    px, size,
};

use crate::actions::bind_default_keys;
use crate::element::{BAR_HEIGHT, BAR_MARGIN};
use crate::review::{ReviewAction, ReviewHunkKind, ReviewHunkView, ReviewLineView, ReviewView};
use crate::review_tests::record;
use crate::settings::{EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::{EditorView, FrameRender};

const TEXT: &str = "a\nb\nc\nd\ne\nf\ng\nh\n";

/// The widths of spec 11 §3.3, one per level.
const WIDE: f32 = 1200.;
/// Spec 11 says 560 px, measured with the interface font of the app. The
/// test harness shapes text with GPUI's `NoopTextSystem`, where every glyph
/// is 0.6 em wide (7.8 px at 13 px, wider than Inter): there the level-1 bar
/// measures 580 px and needs 628 px of editor, so the medium width is 700.
const MEDIUM: f32 = 700.;
const NARROW: f32 = 380.;

/// A hunk of rows added by the agent.
fn added(id: u64, row: u32) -> ReviewHunkView {
    ReviewHunkView {
        id,
        base_rows: 0..0,
        deleted_lines: Vec::new(),
        buffer_rows: row..row + 1,
        kind: ReviewHunkKind::Added,
        word_diffs: None,
        lines: vec![ReviewLineView {
            base_line: None,
            buffer_row: Some(row),
        }],
        from_previous_turn: false,
    }
}

/// Three pending hunks: the bar reads "cambio 1 de 3" with the cursor on
/// row 0.
fn three_hunks() -> ReviewView {
    ReviewView {
        hunks: vec![added(1, 1), added(2, 3), added(3, 5)],
        pending_in_file: 3,
        ..Default::default()
    }
}

/// Opens a focused, active editor `width` px wide with the render probe on
/// and `review` applied.
fn open(
    cx: &mut TestAppContext,
    review: ReviewView,
    width: f32,
) -> (Entity<EditorView>, VisualTestContext) {
    cx.update(bind_default_keys);
    let buffer = shared(Buffer::new(TEXT));
    let registry = Arc::new(LanguageRegistry::new());
    let language = registry.language("rust");
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
        .update(cx, |view, window, cx| {
            window.activate_window();
            window.focus(&view.focus_handle, cx);
            view.set_render_probe(true);
            view.set_review(review, cx);
        })
        .unwrap();
    visual.simulate_resize(size(px(width), px(600.)));
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
        .expect("a frame was painted")
}

/// Bounds of the element in the last layout.
fn element_bounds(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.update(|_window, cx| view.read(cx).layout.as_ref().expect("painted").bounds)
}

fn hover(cx: &mut VisualTestContext, x: f32, y: f32) {
    cx.simulate_mouse_move(point(px(x), px(y)), None, Modifiers::default());
    cx.run_until_parked();
}

/// Walks the bar from its right end until the button `label` is under the
/// mouse, and clicks it.
fn click_button(view: &Entity<EditorView>, cx: &mut VisualTestContext, label: &str) {
    let bar = frame(view, cx).review.bar_bounds.expect("the bar");
    let y = f32::from(bar.center().y);
    let mut x = f32::from(bar.right()) - 2.;
    while x > f32::from(bar.left()) {
        hover(cx, x, y);
        if frame(view, cx)
            .review
            .bar_buttons
            .iter()
            .any(|button| button.hovered && button.text == label)
        {
            cx.simulate_click(point(px(x), px(y)), Modifiers::default());
            cx.run_until_parked();
            return;
        }
        x -= 2.;
    }
    panic!("no se encontró «{label}» en la barra");
}

#[gpui::test]
fn a_wide_editor_shows_the_full_bar(cx: &mut TestAppContext) {
    let (view, mut visual) = open(cx, three_hunks(), WIDE);
    let painted = frame(&view, &mut visual);
    let bar = painted.review.bar.expect("the bar");
    assert_eq!(painted.review.bar_level, 0);
    assert!(bar.contains("Ctrl+Alt+↵"), "{bar}");
    assert_eq!(
        bar,
        "✓ Aceptar todo Ctrl+Alt+↵ · ✗ Rechazar todo Ctrl+Alt+⌫ · ↑ Alt+K ↓ Alt+J · \
         cambio 1 de 3 · Revisar todo"
    );
}

#[gpui::test]
fn a_medium_editor_drops_the_written_shortcuts(cx: &mut TestAppContext) {
    let (view, mut visual) = open(cx, three_hunks(), MEDIUM);
    let painted = frame(&view, &mut visual);
    let bar = painted.review.bar.expect("the bar");
    assert_eq!(painted.review.bar_level, 1, "{bar}");
    assert!(!bar.contains("Ctrl+Alt+↵"), "{bar}");
    assert!(!bar.contains("Alt+K"), "{bar}");
    assert!(bar.contains("Aceptar todo"), "{bar}");
    assert!(bar.contains('↑'), "{bar}");
    let buttons: Vec<&str> = painted
        .review
        .bar_buttons
        .iter()
        .map(|button| button.text.as_str())
        .collect();
    assert_eq!(
        buttons,
        [
            "✓ Aceptar todo",
            "✗ Rechazar todo",
            "↑",
            "↓",
            "Revisar todo"
        ],
        "the arrows are still buttons"
    );
}

#[gpui::test]
fn a_narrow_editor_shows_only_icons(cx: &mut TestAppContext) {
    let (view, mut visual) = open(cx, three_hunks(), NARROW);
    let painted = frame(&view, &mut visual);
    let bar = painted.review.bar.expect("the bar");
    assert_eq!(painted.review.bar_level, 2, "{bar}");
    assert_eq!(bar, "✓ ✗ ↑ ↓ 1/3 ≡");
    for icon in ["✓", "✗", "↑", "↓", "1/3", "≡"] {
        assert!(bar.contains(icon), "{icon} en {bar}");
    }
    assert!(
        !bar.chars().any(char::is_alphabetic),
        "ninguna palabra: {bar}"
    );
    // "≡" opens the review panel, in `text.accent`.
    let accent = crate::theme::color(EditorTheme::default().text_accent);
    let review_all = painted
        .review
        .bar_buttons
        .iter()
        .find(|button| button.text == "≡")
        .expect("≡");
    assert_eq!(review_all.glyph_color, accent);
}

#[gpui::test]
fn the_bar_stays_whole_inside_the_editor_at_every_width(cx: &mut TestAppContext) {
    for (width, level) in [(WIDE, 0), (MEDIUM, 1), (NARROW, 2)] {
        let (view, mut visual) = open(cx, three_hunks(), width);
        let painted = frame(&view, &mut visual);
        let bounds = element_bounds(&view, &mut visual);
        let bar = painted.review.bar_bounds.expect("the bar");
        assert_eq!(painted.review.bar_level, level, "{width}");
        assert!(
            bar.left() >= bounds.left() + px(BAR_MARGIN),
            "{width}: {bar:?} en {bounds:?}"
        );
        assert_eq!(bar.right(), bounds.right() - px(BAR_MARGIN), "{width}");
        assert_eq!(bar.bottom(), bounds.bottom() - px(BAR_MARGIN), "{width}");
        assert_eq!(bar.size.height, px(BAR_HEIGHT), "{width}");
    }
}

#[gpui::test]
fn the_icons_do_what_the_full_buttons_do(cx: &mut TestAppContext) {
    let pairs = [
        ("✓ Aceptar todo", "✓", ReviewAction::AcceptTurn),
        ("Revisar todo", "≡", ReviewAction::OpenReviewPanel),
        ("↓ Alt+J", "↓", ReviewAction::NextHunk),
    ];
    let mut actions = Vec::new();
    for (width, labels) in [
        (WIDE, pairs.map(|(full, _, _)| full)),
        (NARROW, pairs.map(|(_, icon, _)| icon)),
    ] {
        let (view, mut visual) = open(cx, three_hunks(), width);
        let events = record(&view, &mut visual);
        for label in labels {
            click_button(&view, &mut visual, label);
        }
        actions.push(events.borrow().clone());
    }
    let expected: Vec<ReviewAction> = pairs.iter().map(|(_, _, action)| *action).collect();
    assert_eq!(actions[0], expected, "level 0");
    assert_eq!(actions[1], expected, "level 2");
}

#[gpui::test]
fn the_other_states_shrink_too(cx: &mut TestAppContext) {
    // The agent writing: only the spinner at 380 px.
    let writing = ReviewView {
        turn_active: true,
        ..three_hunks()
    };
    let (view, mut visual) = open(cx, writing, NARROW);
    let painted = frame(&view, &mut visual);
    assert_eq!(painted.review.bar_level, 2);
    assert_eq!(painted.review.bar.as_deref(), Some(""), "no text");
    assert!(painted.review.bar_glyph, "the spinner");
    assert!(!painted.review.bar_enabled);

    // At the other widths it says so in words, shorter on the medium one.
    for (width, text) in [(WIDE, "El agente está editando…"), (MEDIUM, "Editando…")] {
        let writing = ReviewView {
            turn_active: true,
            ..three_hunks()
        };
        let (view, mut visual) = open(cx, writing, width);
        assert_eq!(
            frame(&view, &mut visual).review.bar.as_deref(),
            Some(text),
            "{width}"
        );
    }

    // "Sin cambios pendientes…": the button alone at 380 px, still a button.
    for (width, text) in [
        (
            WIDE,
            "Sin cambios pendientes en este archivo · 4 en otros archivos → siguiente archivo (Alt+L)",
        ),
        (MEDIUM, "4 en otros archivos → siguiente (Alt+L)"),
        (NARROW, "→ siguiente archivo"),
    ] {
        let (view, mut visual) = open(cx, three_hunks(), width);
        visual.update(|_window, cx| {
            view.update(cx, |view, cx| {
                view.set_review(
                    ReviewView {
                        pending_in_other_files: 4,
                        ..Default::default()
                    },
                    cx,
                )
            })
        });
        visual.run_until_parked();
        let painted = frame(&view, &mut visual);
        assert_eq!(painted.review.bar.as_deref(), Some(text), "{width}");
        let bar = painted.review.bar_bounds.expect("the bar");
        let bounds = element_bounds(&view, &mut visual);
        assert!(bar.left() >= bounds.left() + px(BAR_MARGIN), "{width}");
        assert_eq!(bar.right(), bounds.right() - px(BAR_MARGIN), "{width}");
        if width == NARROW {
            let events = record(&view, &mut visual);
            click_button(&view, &mut visual, "→ siguiente archivo");
            assert_eq!(*events.borrow(), vec![ReviewAction::NextFile]);
        }
    }
}
