//! The editor as a text field (the chat composer): minimal chrome, automatic
//! height, host decorations, placeholder, read-only mode and whole-text
//! replacement.

// One-range vectors are exactly what a single fenced block looks like.
#![allow(clippy::single_range_in_vec_init)]

use std::sync::Arc;

use cincel_syntax::{HighlightId, LanguageRegistry};
use cincel_text::Buffer;
use gpui::{AnyWindowHandle, Entity, TestAppContext, VisualTestContext, rgb};

use crate::actions::bind_default_keys;
use crate::decorations::{Decorator, TextDecorations};
use crate::settings::{AutoHeight, EditorChrome, EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::{EditorView, FrameRender};

fn field_settings() -> EditorSettings {
    EditorSettings {
        chrome: EditorChrome::Minimal,
        soft_wrap: true,
        auto_height: Some(AutoHeight {
            min_rows: 1,
            max_rows: 8,
        }),
        ..Default::default()
    }
}

fn open(
    cx: &mut TestAppContext,
    text: &str,
    settings: EditorSettings,
) -> (Entity<EditorView>, AnyWindowHandle, VisualTestContext) {
    cx.update(bind_default_keys);
    let buffer = shared(Buffer::new(text));
    let registry = Arc::new(LanguageRegistry::new());
    let language = registry.language("markdown");
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
            view.set_render_probe(true);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    (view, handle, visual)
}

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

/// `(height of the element, line height)` of the last layout.
fn height(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> (f32, f32) {
    cx.update(|_window, cx| {
        let layout = view.read(cx).layout.clone().expect("painted");
        (
            f32::from(layout.bounds.size.height),
            f32::from(layout.line_height),
        )
    })
}

#[gpui::test]
fn the_minimal_chrome_paints_no_gutter_and_no_numbers(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "uno\ndos\n", field_settings());
    let painted = frame(&view, &mut visual);
    assert_eq!(painted.gutter_width, gpui::px(0.));
    assert!(painted.rows.iter().all(|row| !row.line_number));
}

#[gpui::test]
fn the_height_follows_the_rows_between_one_and_eight(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "", field_settings());
    let (empty, line_height) = height(&view, &mut visual);
    assert_eq!(empty, line_height, "one row when empty");

    visual.update(|_window, cx| {
        view.update(cx, |view, cx| view.set_text("a\nb\nc", 5, cx));
    });
    visual.run_until_parked();
    assert_eq!(height(&view, &mut visual).0, line_height * 3.);

    let long: String = (0..20).map(|n| format!("línea {n}\n")).collect();
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| view.set_text(&long, 0, cx));
    });
    visual.run_until_parked();
    assert_eq!(
        height(&view, &mut visual).0,
        line_height * 8.,
        "then it scrolls"
    );
}

#[gpui::test]
fn a_decorator_replaces_the_highlights_and_paints_backgrounds(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "# Título\n```\ncode\n```\nfin", field_settings());
    let decorator: Decorator = Arc::new(|text: &str| TextDecorations {
        highlights: vec![
            (0..1, HighlightId::Keyword),
            (2..text.len().min(8), HighlightId::Keyword),
        ],
        row_backgrounds: vec![(1..4, rgb(0x282c34))],
        monospace_rows: vec![1..4],
    });
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| view.set_decorator(Some(decorator), cx));
    });
    let painted = frame(&view, &mut visual);
    let row = |n: u32| painted.row(n).expect("painted");
    assert_eq!(row(0).highlight_spans, 2, "the decorator's two spans");
    assert_eq!(row(2).highlight_spans, 0, "and nothing from tree-sitter");
    assert!(!row(0).background);
    assert!(row(1).background && row(2).background && row(3).background);
    assert!(!row(4).background);
}

#[gpui::test]
fn prose_rows_use_the_prose_font_and_code_rows_the_code_font(cx: &mut TestAppContext) {
    let settings = EditorSettings {
        prose_font_family: Some(vec!["Inter".into(), "DejaVu Sans".into()]),
        ..field_settings()
    };
    let (view, _handle, mut visual) = open(cx, "texto\n```\ncode\n```\n", settings);
    let decorator: Decorator = Arc::new(|_: &str| TextDecorations {
        monospace_rows: vec![1..4],
        ..Default::default()
    });
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| view.set_decorator(Some(decorator), cx));
    });
    let painted = frame(&view, &mut visual);
    let prose_differs = visual.update(|_window, cx| {
        let style = &view.read(cx).style;
        style.prose_font.as_ref() != Some(&style.font)
    });
    if prose_differs {
        assert!(!painted.row(0).unwrap().monospace, "prose row");
    }
    assert!(painted.row(2).unwrap().monospace, "code row");
}

#[gpui::test]
fn the_placeholder_shows_only_while_empty(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "", field_settings());
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.set_placeholder(Some("Escribí un mensaje…".into()), cx)
        });
    });
    assert!(frame(&view, &mut visual).placeholder);
    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_text("hola", 4, cx)));
    assert!(!frame(&view, &mut visual).placeholder);
}

#[gpui::test]
fn a_read_only_field_refuses_every_edit(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "hola", field_settings());
    visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.set_read_only(true, cx);
            view.set_text("otra cosa", 0, cx);
        })
    });
    cx.simulate_input(handle, "x");
    cx.simulate_keystrokes(handle, "backspace enter");
    visual.run_until_parked();
    assert_eq!(visual.update(|_window, cx| view.read(cx).text()), "hola");

    visual.update(|_window, cx| view.update(cx, |view, cx| view.set_read_only(false, cx)));
    cx.simulate_input(handle, "!");
    visual.run_until_parked();
    assert_eq!(visual.update(|_window, cx| view.read(cx).text()), "!hola");
}

#[gpui::test]
fn set_text_replaces_everything_and_places_the_cursor(cx: &mut TestAppContext) {
    let (view, _handle, mut visual) = open(cx, "viejo", field_settings());
    let (text, cursor) = visual.update(|_window, cx| {
        view.update(cx, |view, cx| {
            view.set_text("mirá @main.rs ", 6, cx);
            (view.text(), view.cursor_offset())
        })
    });
    assert_eq!(text, "mirá @main.rs ");
    assert_eq!(cursor, 6);
}

#[gpui::test]
fn find_and_go_to_line_open_nothing_in_a_minimal_field(cx: &mut TestAppContext) {
    let (view, handle, mut visual) = open(cx, "uno", field_settings());
    cx.simulate_keystrokes(handle, "ctrl-f ctrl-g");
    visual.run_until_parked();
    let (search, prompt) =
        visual.update(|_window, cx| (view.read(cx).search_open, view.read(cx).prompt.is_some()));
    assert!(!search && !prompt);
}
