//! `cincel-editor`: the code editor element of Cincel, on GPUI.
//! See `docs/specs/modulos/editor.md` and `docs/specs/02-visual.md` §3–§5.
//!
//! # Public API (stable for the workspace)
//!
//! ```no_run
//! use std::sync::Arc;
//! use cincel_editor::{EditorSettings, EditorTheme, EditorView, SharedBuffer, shared};
//! use cincel_syntax::LanguageRegistry;
//! use cincel_text::Buffer;
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
//!   [`cincel_text::BufferSnapshot`] for reads and only locks to edit, and it
//!   picks up edits made by anyone else on the next frame.
//! - **Settings and theme**: [`EditorSettings`] and [`EditorTheme`] are plain
//!   `Default` structs, so the workspace maps its own types onto them.
//!   [`EditorView::set_settings`], [`EditorView::set_theme`] and
//!   [`EditorView::set_language`] hot-reload them.
//! - **Events**: the view is an `EventEmitter<`[`EditorEvent`]`>`, emitting
//!   `DirtyChanged`, `CursorMoved`, `ScrollChanged`, `SaveRequested` and
//!   `Review(`[`ReviewAction`]`)`. The host answers
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
//! - **Key contexts**: `Editor`, `Editor && searching`,
//!   `Editor && searching && replacing` and
//!   `Editor && review_hunk_under_cursor` (see [`EditorView::key_context`]).
//!   A host that loads its own keymap after the defaults re-binds
//!   [`search_bar_bindings`] on top of it, so `Tab`, `Enter` and `Ctrl+Enter`
//!   keep their search-bar meaning while the bar has the keyboard.
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
//! only the wrap rows in the viewport and caches the shaped lines by their
//! *content* (painted text, font size, run colours), so a row that did not
//! change is never shaped again — not even when an edit moved it up or down.
//!
//! # Threading
//!
//! Tree-sitter parsing follows `03-arquitectura.md` §3: `SyntaxState::apply_event`
//! runs on the UI thread for every buffer event, and the reparse is then tried
//! on the UI thread with a [`view::SYNC_PARSE_BUDGET`] deadline — one keystroke
//! is far below it, so the frame that paints the character already has its
//! final colours. Only a parse that blows the budget goes to
//! `cx.background_executor()` with a `CancelFlag` that a newer edit raises, and
//! while it runs the highlights of the previous frame are carried over, shifted
//! by the edit, so no frame is ever painted with uncoloured text. Highlights
//! are queried for the visible byte range only.
//!
//! # Review (`02-visual.md` §6)
//!
//! The host (the workspace, over `cincel-review`) owns the review. It hands
//! the editor a [`ReviewView`] with [`EditorView::set_review`] on every change
//! of its store; the editor paints it — phantom rows with the file's grammar at
//! 70% opacity, `diff.*.bg` rows, word diffs, the 3 px row bar and the 2 px hunk
//! border in the gutter, the `+`/`−` icons of the hovered line (over its line
//! number, in a number column at least three digits wide, so the text never
//! moves), the pill on the hunk with the cursor or the mouse (with a clock for
//! a previous turn) on the first of its rows whose text leaves it room — or a
//! compact two-icon pill at 70 % when none does (see [`element`]) — and the
//! floating bar, whose "✓ Aceptar todo" / "✗ Rechazar todo" (the whole turn)
//! read as enabled (`status.ok`
//! / `status.error` glyphs, `text` words, `bg.surface` on hover) — and reports
//! every click and key as
//! [`EditorEvent::Review`]. It **never applies a decision itself**: the host
//! does and calls `set_review` again. `set_review` keeps the display map when
//! no hunk changed rows, keeps phantom highlights by content, and keeps the
//! cursor on its line; after a decision it jumps to the next hunk when
//! [`EditorSettings::jump_to_next_on_decide`] is on. While
//! [`ReviewView::turn_active`] every decision is disabled (spinner on the pill
//! and the bar) and editing keeps working. [`PhantomHunk`] and the deprecated
//! [`EditorView::set_hunks`] remain as the E0 path, mapped onto a `ReviewView`.
//!
//! - **No `BlockMap`** (`03-arquitectura.md` §5): the pill is an overlay
//!   anchored to a wrap row of its hunk (or the row just above it; see
//!   [`element`]). It takes no vertical space, so the cursor never skips a UI
//!   row, and it scrolls and follows the hunk exactly like the text. `FoldMap`
//!   is v2.
//! - **Toasts** ("Segmento rechazado · Deshacer") belong to the host, which
//!   knows when a rejection was applied; `Alt+Shift+U` emits
//!   [`ReviewAction::UndoLastReject`].
//! - **Saving**: `editor::save` only emits [`EditorEvent::SaveRequested`]; the
//!   host writes the file and calls [`EditorView::mark_saved`].
//! - **Soft wrap** measures in monospace columns (see [`wrap_map`]), and tabs
//!   are painted as a fixed `tab_size` spaces rather than to the next tab stop.
//! - **Search and replace** (`Ctrl+F`, `Ctrl+H`;
//!   `docs/specs/07-etapa5-productividad.md` §10.2) covers the buffer *and*
//!   the phantom rows, in screen order: phantom matches are highlighted,
//!   counted (`3/12 (2 en líneas quitadas)`) and visited with `Enter` /
//!   `Shift+Enter`, but never replaced — the phantom text is what
//!   "Rechazar" restores, not the file. "Reemplazar" on a phantom match jumps
//!   to the next real one with a notice; "Reemplazar todo" is one
//!   `EditSource::User` transaction (one `Ctrl+Z`) over the real matches and
//!   reports the phantom ones it left alone. Replace mode splits the same
//!   28 px strip into two fields side by side, so the text never moves more
//!   than with the plain search bar ([`view::SEARCH_BAR_HEIGHT`]).
//! - `Ctrl+click` does nothing, as `editor.md` asks for v1 (no multi-cursor).
//!
//! # Git column (`docs/specs/07-etapa5-productividad.md` §6)
//!
//! [`EditorView::set_git_diff`] takes the file's changes against `HEAD` as
//! [`GitGutterHunk`]s in buffer rows (the host runs git; the editor never
//! does) and anchors them to the buffer, so the bars follow every edit until
//! the host sends the next diff; [`EditorView::git_diff`] reads them back
//! resolved. They are painted in a 3 px column of their own — green added,
//! blue modified, a red 3 × 6 px mark where lines were deleted — with the
//! colours of [`EditorView::set_git_colors`]. The gutter is exactly as wide
//! as before the column existed (padding 2 + git 3 + gap 2 + agent bar 3 +
//! gap 3 = the old 4 + 3 + 6), phantom rows never get a git bar, and the
//! minimal chrome has none.
//!
//! # The editor as a text field
//!
//! The chat composer is an `EditorView` too. Everything it needs is additive
//! and off by default:
//!
//! - [`EditorSettings::chrome`] = [`EditorChrome::Minimal`]: no gutter, no
//!   current-line background, no bracket boxes, and `editor::find` /
//!   `editor::go_to_line` open nothing (they propagate).
//! - [`EditorSettings::auto_height`]: the element measures its own height —
//!   the wrap rows of the text at the width it is given, clamped to
//!   `min_rows..=max_rows` — and scrolls past it.
//! - [`EditorSettings::prose_font_family`]: rows that are not code are painted
//!   in a proportional font, and soft wrap then measures real glyph advances
//!   ([`wrap_map::WrapSource::char_widths`]).
//! - [`EditorView::set_decorator`]: a [`Decorator`] computes
//!   [`TextDecorations`] from the text once per version — highlights that
//!   replace the tree-sitter ones, full-width row backgrounds and the rows
//!   that keep the code font — inside the frame of the edit.
//!   [`EditorView::set_row_backgrounds`] sets backgrounds directly.
//! - [`EditorView::set_placeholder`], [`EditorView::set_read_only`],
//!   [`EditorView::set_text`], [`EditorView::cursor_offset`] and
//!   [`EditorView::is_empty`].
//!
//! # Editing commands
//!
//! Besides movement, selection, clipboard and search, the editor has the usual
//! line and bracket commands, each a named action rebindable in `keymap.json`
//! (default Linux keys in brackets; none collides with `02-visual.md` §8):
//!
//! | Action | Keys |
//! |---|---|
//! | `editor::move_line_up` / `editor::move_line_down` | `Alt+↑` / `Alt+↓` |
//! | `editor::duplicate_line_down` / `editor::duplicate_line_up` | `Shift+Alt+↓` / `Shift+Alt+↑` |
//! | `editor::delete_line` | `Ctrl+Shift+K` |
//! | `editor::toggle_comments` | `Ctrl+/` |
//! | `editor::join_lines` | `Ctrl+J` |
//! | `editor::select_line` (repeat extends one line) | `Ctrl+Shift+L` |
//! | `editor::select_next` (word under the cursor, then next occurrence) | `Ctrl+D` |
//! | `editor::move_to_matching_bracket` | `Ctrl+Shift+\` |
//! | `editor::move_to_line_start` (first non-blank ↔ column 0) | `Home` (`Shift+Home` selects) |
//! | `editor::uppercase` / `editor::lowercase` / `editor::sort_lines` | — (commands only) |
//!
//! - The line commands act on the whole *buffer* rows the selection covers (a
//!   selection ending at column 0 does not take that row), keep the selection
//!   on the same text and are one undo step each. Phantom rows are never
//!   edited: a cursor on one acts on the row its edits land on.
//! - **Comments** use [`ops::comment_syntax`]: `//`, `#` or `--` per language,
//!   `<!-- -->` (HTML, Markdown) and `/* */` (CSS) around each line, nothing for
//!   JSON or plain text. Mixed commented and uncommented lines are all
//!   commented; only an all-commented block is uncommented.
//! - **Auto-closed pairs**: `(`, `[`, `{`, `"`, `'` and `` ` `` insert their
//!   closer when the next character is whitespace, a closer or the end of the
//!   line (quotes also not right after a word character; `'` never in Rust);
//!   typing a closer the editor inserted steps over it; `Backspace` between an
//!   empty pair deletes both; an opener with a selection wraps it.
//!   [`EditorSettings::auto_close_pairs`] turns it off (`editor.auto_close_pairs`
//!   in `settings.json`); [`EditorView::set_auto_close_pairs`] does the same at
//!   runtime.
//! - **Auto-indent**: `Enter` between an empty pair puts the closer on its own
//!   line; a multi-line paste takes the cursor's indentation
//!   ([`ops::reindent_paste`]); `Tab` / `Shift+Tab` indent every selected row.
//! - **Matching bracket**: the bracket at or before the cursor and its match
//!   get a subtle `border` box ([`EditorView::matching_brackets`]).
//! - **Mouse cursor**: the pointing hand over every enabled review control,
//!   the arrow over a disabled one, over the rest of a pill or of the bar and
//!   over the margins that are not text (gutter, vertical scrollbar), the
//!   I-beam over the text ([`EditorView::cursor_style_at`]).

pub mod actions;
pub mod decorations;
pub mod display_map;
pub mod element;
pub mod git_gutter;
pub mod input;
pub mod ops;
pub mod review;
pub mod search;
pub mod settings;
pub mod symbol;
pub mod theme;
pub mod view;
pub mod wrap_map;

#[cfg(all(test, feature = "test-support"))]
mod editing_tests;
#[cfg(all(test, feature = "test-support"))]
mod git_gutter_tests;
#[cfg(all(test, feature = "test-support"))]
mod review_tests;
#[cfg(all(test, feature = "test-support"))]
mod search_tests;
#[cfg(all(test, feature = "test-support"))]
mod tests;
#[cfg(all(test, feature = "test-support"))]
mod text_field_tests;

pub use actions::{
    AcceptFile, AcceptHunk, AcceptLine, Backspace, Backtab, Cancel, Confirm, Copy, Cut, Delete,
    DeleteLine, DeleteWordLeft, DeleteWordRight, DuplicateLineDown, DuplicateLineUp, Find,
    FindNext, FindPrev, FindReplace, GoToLine, InsertNewline, JoinLines, Lowercase, MoveDown,
    MoveLeft, MoveLineDown, MoveLineUp, MovePageDown, MovePageUp, MoveRight, MoveToDocumentEnd,
    MoveToDocumentStart, MoveToLineEnd, MoveToLineStart, MoveToMatchingBracket, MoveUp,
    MoveWordLeft, MoveWordRight, NextFile, NextHunk, OpenReviewPanel, Paste, PrevHunk, Redo,
    RejectFile, RejectHunk, RejectLine, ReplaceAll, ReplaceNext, Save, SearchNextField,
    SearchPrevField, SelectAll, SelectDown, SelectLeft, SelectLine, SelectNext, SelectPageDown,
    SelectPageUp, SelectRight, SelectToDocumentEnd, SelectToDocumentStart, SelectToLineEnd,
    SelectToLineStart, SelectUp, SelectWordLeft, SelectWordRight, SortLines, Tab, ToggleComments,
    ToggleSearchCase, ToggleSearchRegex, ToggleSoftWrap, ToggleWhitespace, Undo, UndoLastReject,
    Uppercase, bind_default_keys, default_key_bindings, search_bar_bindings,
};
pub use decorations::{Decorator, TextDecorations};
pub use display_map::{
    BufferRow, DiffTransformMap, DisplayCell, DisplayMap, DisplayPoint, DisplayRow, PhantomHunk,
    RowKind, RowText,
};
pub use element::EditorElement;
pub use git_gutter::{GitGutterColors, GitGutterHunk, GitGutterKind};
pub use input::WeakInputHandler;
pub use review::{
    ReviewAction, ReviewHunkKind, ReviewHunkView, ReviewLineView, ReviewView, ReviewWordDiffs,
};
pub use search::{
    MatchLocation, PhantomLines, SearchField, SearchState, find_matches, replace_all_message,
};
pub use settings::{
    AutoHeight, EditorChrome, EditorSettings, Indentation, SharedBuffer, detect_indentation, shared,
};
pub use symbol::{definition_kinds, symbol_at};
pub use theme::EditorTheme;
pub use view::{
    BarButtonRender, EditorEvent, EditorStyle, EditorView, FrameRender, FrameStats, PillRender,
    ReviewFrame, RowRender, editor,
};
pub use wrap_map::{WrapMap, WrapRow, WrapSource, WrappedRow};
