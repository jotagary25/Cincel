//! Demo of the E0 editor: a buffer with two simulated review hunks, one that
//! deletes 2 lines and adds 3, and one that only deletes a line.
//!
//! Usage:
//! ```text
//! cargo run -p asteroid-editor --example phantom
//! cargo run -p asteroid-editor --example phantom -- --smoke-test
//! cargo run -p asteroid-editor --example phantom -- --rows 50000 --smoke-test
//! ```

use std::time::Duration;

use asteroid_editor::{EditorView, PhantomHunk, bind_default_keys};
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
}

fn parse_args() -> Args {
    let mut args = Args {
        smoke_test: false,
        rows: None,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--smoke-test" => args.smoke_test = true,
            "--rows" => {
                args.rows = iter.next().and_then(|value| value.parse().ok());
            }
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

fn document(args: &Args) -> (String, Vec<PhantomHunk>) {
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
        return (text, hunks);
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
    (text, hunks)
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = parse_args();
    let (text, hunks) = document(&args);
    let smoke_test = args.smoke_test;

    application().run(move |cx: &mut App| {
        bind_default_keys(cx);
        cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(1100.), px(760.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_window, cx| cx.new(|cx| EditorView::new(&text, hunks, cx)),
            )
            .expect("no se pudo abrir la ventana");

        window
            .update(cx, |view, window, cx| {
                window.focus(&view.focus_handle(cx), cx);
                cx.activate(true);
            })
            .expect("no se pudo enfocar el editor");

        if smoke_test {
            // Render ~60 frames while scrolling, then exit.
            cx.spawn(async move |cx| {
                for step in 0..60 {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let scrolled = window.update(cx, |view, _window, cx| {
                        view.scroll_rows(if step % 20 == 19 { -19. } else { 1. }, cx);
                    });
                    if scrolled.is_err() {
                        break;
                    }
                }
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
