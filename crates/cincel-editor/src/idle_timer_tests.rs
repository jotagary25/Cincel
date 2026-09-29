//! Regression tests of M8 (`docs/rendimiento.md`, E6-G): an editor at rest
//! owns no timer. The blink task used to wake the main thread every 500 ms
//! forever, even after the cursor stopped blinking.

use std::sync::Arc;
use std::time::Duration;

use cincel_syntax::LanguageRegistry;
use cincel_text::Buffer;
use gpui::{AnyWindowHandle, Entity, TestAppContext, VisualTestContext};

use crate::actions::bind_default_keys;
use crate::settings::{EditorSettings, shared};
use crate::theme::EditorTheme;
use crate::view::{BLINK_IDLE, EditorView};

fn open(
    cx: &mut TestAppContext,
    settings: EditorSettings,
) -> (Entity<EditorView>, VisualTestContext) {
    cx.update(bind_default_keys);
    let buffer = shared(Buffer::new("hola\n"));
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
    window
        .update(cx, |view, window, cx| window.focus(&view.focus_handle, cx))
        .unwrap();
    let handle: AnyWindowHandle = window.into();
    let visual = VisualTestContext::from_window(handle, cx);
    visual.run_until_parked();
    (view, visual)
}

fn blinking(view: &Entity<EditorView>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_window, cx| view.read(cx).is_blinking())
}

fn wait(cx: &mut VisualTestContext, duration: Duration) {
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
}

#[gpui::test]
fn the_blink_task_ends_at_rest_and_input_restarts_it(cx: &mut TestAppContext) {
    let (view, mut visual) = open(cx, EditorSettings::default());
    assert!(blinking(&view, &mut visual), "a new editor blinks");

    wait(&mut visual, BLINK_IDLE + Duration::from_secs(2));
    assert!(
        !blinking(&view, &mut visual),
        "after the idle time the blink task is gone"
    );
    assert!(visual.update(|_window, cx| view.read(cx).blink_visible));

    visual.simulate_input("x");
    visual.run_until_parked();
    assert!(blinking(&view, &mut visual), "typing starts it again");

    wait(&mut visual, BLINK_IDLE + Duration::from_secs(2));
    assert!(!blinking(&view, &mut visual), "and it ends again at rest");
}

#[gpui::test]
fn no_blink_task_with_blinking_off(cx: &mut TestAppContext) {
    let settings = EditorSettings {
        cursor_blink: false,
        ..EditorSettings::default()
    };
    let (view, mut visual) = open(cx, settings);
    // The first tick finds the blink off and ends the task.
    wait(&mut visual, Duration::from_secs(1));
    assert!(!blinking(&view, &mut visual));

    visual.simulate_input("x");
    wait(&mut visual, Duration::from_secs(1));
    assert!(
        !blinking(&view, &mut visual),
        "with the blink off, input leaves no timer running"
    );
}
