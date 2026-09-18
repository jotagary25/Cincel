//! IME plumbing: the same thing GPUI's `ElementInputHandler` does, but holding
//! the view weakly.
//!
//! # Why this exists
//!
//! `Window::handle_input` parks the handler on the *platform* window
//! (`gpui-pre` 0.3.5, `window.rs:3210`), and that slot is not cleared by
//! `Window::remove_window` nor before the `App` is dropped. Because
//! `ElementInputHandler` owns a strong `Entity<V>`, the focused editor's handle
//! outlives the app and GPUI's leak detector aborts the process with
//! "Exited with leaked handles" — the leak `docs/etapas/etapa-0.md` (finding 7)
//! left open for E1.
//!
//! Holding a [`WeakEntity`] instead costs one upgrade per IME call and makes
//! the leak go away, with no change in behaviour: when the entity is gone the
//! handler answers like an empty document.

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, EntityInputHandler, InputHandler, Pixels, Point, UTF16Selection,
    WeakEntity, Window,
};

/// `ElementInputHandler` with a weak view handle.
pub struct WeakInputHandler<V: 'static> {
    view: WeakEntity<V>,
    element_bounds: Bounds<Pixels>,
}

impl<V: 'static> WeakInputHandler<V> {
    /// Builds the handler for an element's bounds. Pass it to
    /// `window.handle_input` during `paint`, exactly like
    /// `ElementInputHandler::new`.
    pub fn new(element_bounds: Bounds<Pixels>, view: WeakEntity<V>) -> Self {
        Self {
            view,
            element_bounds,
        }
    }
}

impl<V: EntityInputHandler> InputHandler for WeakInputHandler<V> {
    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection> {
        self.view
            .update(cx, |view, cx| {
                view.selected_text_range(ignore_disabled_input, window, cx)
            })
            .ok()
            .flatten()
    }

    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.view
            .update(cx, |view, cx| view.marked_text_range(window, cx))
            .ok()
            .flatten()
    }

    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        self.view
            .update(cx, |view, cx| {
                view.text_for_range(range_utf16, adjusted_range, window, cx)
            })
            .ok()
            .flatten()
    }

    fn replace_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.view
            .update(cx, |view, cx| {
                view.replace_text_in_range(replacement_range, text, window, cx)
            })
            .ok();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.view
            .update(cx, |view, cx| {
                view.replace_and_mark_text_in_range(
                    range_utf16,
                    new_text,
                    new_selected_range,
                    window,
                    cx,
                )
            })
            .ok();
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        self.view
            .update(cx, |view, cx| view.unmark_text(window, cx))
            .ok();
    }

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut App) {
        self.view
            .update(cx, |view, cx| view.paste(item, window, cx))
            .ok();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        let element_bounds = self.element_bounds;
        self.view
            .update(cx, |view, cx| {
                view.bounds_for_range(range_utf16, element_bounds, window, cx)
            })
            .ok()
            .flatten()
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<usize> {
        self.view
            .update(cx, |view, cx| {
                view.character_index_for_point(point, window, cx)
            })
            .ok()
            .flatten()
    }
}
