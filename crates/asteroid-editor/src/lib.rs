//! `asteroid-editor`: the code editor element of Asteroid, on GPUI.
//! See `docs/specs/modulos/editor.md` and `docs/specs/02-visual.md` §3–§5.
//!
//! # Public API (stable for the workspace)
//!
//! ```no_run
//! use std::sync::Arc;
//! use asteroid_editor::{EditorSettings, EditorTheme, EditorView, SharedBuffer, shared};
//! use asteroid_syntax::LanguageRegistry;
//! use asteroid_text::Buffer;
//! # use gpui::{App, AppContext, Window};
//! # fn demo(window: &mut Window, cx: &mut App) {
//! let registry = Arc::new(LanguageRegistry::new());
//! let language = registry.language_for_path("src/main.rs");
//! let buffer: SharedBuffer = shared(Buffer::new("fn main() {}\n"));
//!
//! let editor = cx.new(|cx| {
//!     EditorView::new(
//!         buffer,
//!         language,
//!         registry,
//!         EditorSettings::default(),
//!         EditorTheme::default(),
//!         window,
//!         cx,
//!     )
//! });
//! # let _ = editor;
//! # }
//! ```
//!
//! - **Buffer sharing**: [`SharedBuffer`] is `Arc<Mutex<Buffer>>`. The editor
//!   never owns the buffer; the project's `BufferStore` hands the same handle
//!   to every view of the file and to background work. The view keeps a
//!   [`asteroid_text::BufferSnapshot`] for reads and only locks to edit, and it
//!   picks up edits made by anyone else on the next frame.
//! - **Settings and theme**: [`EditorSettings`] and [`EditorTheme`] are plain
//!   `Default` structs, so the workspace maps its own types onto them.
//!   [`EditorView::set_settings`], [`EditorView::set_theme`] and
//!   [`EditorView::set_language`] hot-reload them.
//! - **Events**: the view is an `EventEmitter<`[`EditorEvent`]`>`, emitting
//!   `DirtyChanged`, `CursorMoved` and `SaveRequested`. The host answers
//!   `SaveRequested` by writing the buffer and calling
//!   [`EditorView::mark_saved`]. [`EditorView::is_dirty`],
//!   [`EditorView::cursor_point`] and [`EditorView::buffer`] round out what a
//!   tab needs. `ScrollChanged` reports the vertical position at most once per
//!   painted frame, so a tab can persist it and restore it with
//!   [`EditorView::set_scroll_row`] — which works before the first layout.
//! - **What a tab asks the editor**: [`EditorView::scroll_row`] /
//!   [`EditorView::set_scroll_row`] for the scroll position,
//!   [`EditorView::set_cursor`] to jump to a buffer point (from the outline, a
//!   search result, a stack trace), and [`EditorView::symbol_at_cursor`] for
//!   the breadcrumb: the innermost enclosing definition and its byte range.
//! - **Key bindings**: [`default_key_bindings`] returns the Linux defaults of
//!   `02-visual.md` §8 for the editor contexts; install them with
//!   [`bind_default_keys`] before loading the user's keymap. Action names are
//!   the ones in the spec (`editor::insert_newline`, `review::accept_hunk`, …).
//! - **Key contexts**: `Editor`, `Editor && searching` and
//!   `Editor && review_hunk_under_cursor` (see [`EditorView::key_context`]).
//!
//! # Display pipeline
//!
//! ```text
//! Buffer (rope) -> DiffTransformMap -> WrapMap -> EditorElement
//!                  phantom rows        soft wrap   virtualized painting
//! ```
//!
//! [`DiffTransformMap`] splices the deleted lines of a review hunk in as
//! read-only *phantom rows* (still shaped, selectable, copyable); [`WrapMap`]
//! turns every display row into one or more *wrap rows*. The element shapes
//! only the wrap rows in the viewport and caches them by
//! `(buffer version, highlight version, row, wrap width)`.
//!
//! # Threading
//!
//! Tree-sitter parsing follows `03-arquitectura.md` §3: `SyntaxState::apply_event`
//! runs on the UI thread for every buffer event, and the expensive `reparse`
//! runs on `cx.background_executor()` with a `CancelFlag` that a newer edit
//! raises. Highlights are queried for the visible byte range only.
//!
//! # Not implemented yet (E1 workstream C scope)
//!
//! - **Review**: the hunks are the *simulated* [`PhantomHunk`]s of E0, handed
//!   over with [`EditorView::set_hunks`]; there is no `ReviewStore` yet, so the
//!   accept/reject pill, `review::*` and the `review_hunk_under_cursor` context
//!   work against that model. Word diffs, the per-line `+`/`−` gutter icons,
//!   the floating review bar, the turn spinner and `jump_to_next_on_decide`
//!   wait for `asteroid-review`.
//! - **BlockMap** (`03-arquitectura.md` §5): the pill is painted by the element
//!   instead of living in a block layer. `FoldMap` is v2.
//! - **Saving**: `editor::save` only emits [`EditorEvent::SaveRequested`]; the
//!   host writes the file and calls [`EditorView::mark_saved`].
//! - **Soft wrap** measures in monospace columns (see [`wrap_map`]), and tabs
//!   are painted as a fixed `tab_size` spaces rather than to the next tab stop.
//! - **Search** covers the buffer, not the phantom rows, and has no replace.
//! - `Ctrl+click` does nothing, as `editor.md` asks for v1 (no multi-cursor).

pub mod actions;
pub mod display_map;
pub mod element;
pub mod input;
pub mod search;
pub mod settings;
pub mod symbol;
pub mod theme;
pub mod view;
pub mod wrap_map;

#[cfg(all(test, feature = "test-support"))]
mod tests;

pub use actions::{
    AcceptFile, AcceptHunk, AcceptLine, Backspace, Backtab, Cancel, Confirm, Copy, Cut, Delete,
    DeleteWordLeft, DeleteWordRight, Find, FindNext, FindPrev, GoToLine, InsertNewline, MoveDown,
    MoveLeft, MovePageDown, MovePageUp, MoveRight, MoveToDocumentEnd, MoveToDocumentStart,
    MoveToLineEnd, MoveToLineStart, MoveUp, MoveWordLeft, MoveWordRight, NextHunk, Paste, PrevHunk,
    Redo, RejectFile, RejectHunk, RejectLine, Save, SelectAll, SelectDown, SelectLeft,
    SelectPageDown, SelectPageUp, SelectRight, SelectToDocumentEnd, SelectToDocumentStart,
    SelectToLineEnd, SelectToLineStart, SelectUp, SelectWordLeft, SelectWordRight, Tab,
    ToggleSearchCase, ToggleSearchRegex, ToggleSoftWrap, ToggleWhitespace, Undo, bind_default_keys,
    default_key_bindings,
};
pub use display_map::{
    BufferRow, DiffTransformMap, DisplayCell, DisplayMap, DisplayPoint, DisplayRow, PhantomHunk,
    RowKind, RowText,
};
pub use element::EditorElement;
pub use input::WeakInputHandler;
pub use search::{SearchState, find_matches};
pub use settings::{EditorSettings, Indentation, SharedBuffer, detect_indentation, shared};
pub use symbol::{definition_kinds, symbol_at};
pub use theme::EditorTheme;
pub use view::{EditorEvent, EditorStyle, EditorView, FrameStats, editor};
pub use wrap_map::{WrapMap, WrapRow, WrapSource, WrappedRow};
