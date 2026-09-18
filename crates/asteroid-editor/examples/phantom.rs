//! Demo of the editor: opens a real file (or a synthetic buffer) with syntax
//! highlighting and two simulated review hunks, one that deletes 2 lines and
//! adds 3, and one that only deletes a line.
//!
//! Usage:
//! ```text
//! cargo run -p asteroid-editor --example phantom
//! cargo run -p asteroid-editor --example phantom -- --file src/main.rs
//! cargo run -p asteroid-editor --example phantom -- --smoke-test
//! cargo run -p asteroid-editor --example phantom -- --rows 50000 --soft-wrap --smoke-test
//! ```
//!
//! `Ctrl+S` prints "save requested" on stdout: the real save belongs to the
//! workspace, the editor only emits the event.

use std::sync::Arc;
use std::time::Duration;

use asteroid_editor::{
    EditorEvent, EditorSettings, EditorTheme, EditorView, PhantomHunk, SharedBuffer,
    bind_default_keys, shared,
};
use asteroid_syntax::LanguageRegistry;
use asteroid_text::Buffer;
use gpui::{
    App, AppContext, Bounds, Focusable, KeyBinding, WindowBounds, WindowOptions, actions, px, size,
};
use gpui_platform::application;

actions!(
    phantom_example,
    [
        /// Closes the demo.
        Quit
    ]
);

/// The file shown by default.
const SAMPLE: &str = include_str!("sample.rs");

struct Args {
    smoke_test: bool,
    rows: Option<usize>,
    file: Option<String>,
    soft_wrap: bool,
    show_whitespace: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        smoke_test: false,
        rows: None,
        file: None,
        soft_wrap: false,
        show_whitespace: false,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--smoke-test" => args.smoke_test = true,
            "--soft-wrap" => args.soft_wrap = true,
            "--show-whitespace" => args.show_whitespace = true,
            "--rows" => args.rows = iter.next().and_then(|value| value.parse().ok()),
            "--file" => args.file = iter.next(),
            other => eprintln!("argumento desconocido: {other}"),
        }
    }
    args
}

fn row_of(text: &str, needle: &str) -> u32 {
    text.lines()
        .position(|line| line.contains(needle))
        .unwrap_or(0) as u32
}

/// Builds a synthetic buffer of `rows` lines for the scroll benchmark.
fn synthetic_text(rows: usize) -> String {
    let mut text = String::with_capacity(rows * 48);
    for row in 0..rows {
        text.push_str(&format!(
            "fn generated_{row}(value: u32) -> u32 {{ value * {} + {row} }}\n",
            row % 7 + 1
        ));
    }
    text
}

/// The document to show: `(text, hunks, path for language detection)`.
fn document(args: &Args) -> (String, Vec<PhantomHunk>, String) {
    if let Some(path) = &args.file {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("no se pudo leer {path}: {error}"));
        return (text, Vec::new(), path.clone());
    }

    if let Some(rows) = args.rows {
        let text = synthetic_text(rows);
        let hunks = vec![
            PhantomHunk {
                insert_before_buffer_row: 20,
                deleted_text: vec![
                    "fn generated_20(value: u32) -> u32 { value * 6 }".to_string(),
                    "// old helper".to_string(),
                ],
                added_rows: 20..23,
            },
            PhantomHunk {
                insert_before_buffer_row: 60,
                deleted_text: vec!["// removed line".to_string()],
                added_rows: 60..60,
            },
        ];
        return (text, hunks, "generado.rs".to_string());
    }

    let text = SAMPLE.to_string();
    // Hunk 1: the agent replaced 2 lines with 3 inside `Item::subtotal`.
    let added_start = row_of(&text, "let gross = quantity * self.unit_price;");
    // Hunk 2: the agent deleted a single line (no replacement).
    let deleted_only = row_of(&text, "/// Creates an empty invoice.");
    let hunks = vec![
        PhantomHunk {
            insert_before_buffer_row: added_start,
            deleted_text: vec![
                "        let gross = self.quantity as f64 * self.unit_price;".to_string(),
                "        gross".to_string(),
            ],
            added_rows: added_start..added_start + 3,
        },
        PhantomHunk {
            insert_before_buffer_row: deleted_only,
            deleted_text: vec![
                "    // TODO: remove this constructor once the builder lands".to_string(),
            ],
            added_rows: deleted_only..deleted_only,
        },
    ];
    (text, hunks, "sample.rs".to_string())
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = parse_args();
    let (text, hunks, path) = document(&args);
    let smoke_test = args.smoke_test;
    let mut settings = EditorSettings {
        soft_wrap: args.soft_wrap,
        show_whitespace: args.show_whitespace,
        ..Default::default()
    };
    settings.ruler = Some(100);

    application().run(move |cx: &mut App| {
        bind_default_keys(cx);
        cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let registry = Arc::new(LanguageRegistry::new());
        let language = registry.language_for_path(&path);
        println!(
            "archivo: {path} · lenguaje: {}",
            language
                .as_ref()
                .map(|language| language.name())
                .unwrap_or("texto plano")
        );
        let buffer: SharedBuffer = shared(Buffer::new(&text));

        let bounds = Bounds::centered(None, size(px(1100.), px(760.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        EditorView::new(
                            buffer,
                            language,
                            registry,
                            settings,
                            EditorTheme::default(),
                            window,
                            cx,
                        )
                    })
                },
            )
            .expect("no se pudo abrir la ventana");

        window
            .update(cx, |view, window, cx| {
                window.focus(&view.focus_handle(cx), cx);
                cx.activate(true);
                let entity = cx.entity();
                cx.subscribe(
                    &entity,
                    |_view, _entity, event: &EditorEvent, _cx| match event {
                        EditorEvent::SaveRequested => println!("save requested"),
                        EditorEvent::DirtyChanged(dirty) => println!("dirty: {dirty}"),
                        EditorEvent::CursorMoved { .. } | EditorEvent::ScrollChanged { .. } => {}
                    },
                )
                .detach();
                if !hunks.is_empty() {
                    view.set_hunks(hunks, cx);
                }
            })
            .expect("no se pudo enfocar el editor");

        if smoke_test {
            // Scroll for ~3 s so the frame statistics have something to say,
            // then exit.
            cx.spawn(async move |cx| {
                for step in 0..180 {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let scrolled = window.update(cx, |view, _window, cx| {
                        view.scroll_rows(if step % 40 == 39 { -39. } else { 1. }, cx);
                    });
                    if scrolled.is_err() {
                        break;
                    }
                }
                window
                    .update(cx, |view, _window, _cx| {
                        let stats = view.frame_stats();
                        println!(
                            "frames: {} · p50 {:.2} ms · p95 {:.2} ms · max {:.2} ms",
                            stats.count,
                            stats.p50_us as f64 / 1000.,
                            stats.p95_us as f64 / 1000.,
                            stats.max_us as f64 / 1000.,
                        );
                    })
                    .ok();
                // Close the window before quitting so no entity handle
                // outlives the app (gpui's leak detection is strict).
                window
                    .update(cx, |_view, window, _cx| window.remove_window())
                    .ok();
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}
