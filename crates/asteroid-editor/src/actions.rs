//! Named actions and the default Linux key bindings.
//!
//! Every command of `docs/specs/modulos/editor.md` is a GPUI action whose
//! registered name is exactly the one the spec uses (`editor::insert_newline`,
//! `review::accept_hunk`, …), so `keymap.json` can rebind it. GPUI derives the
//! name from the type name, so the snake-case names are set explicitly with
//! `#[action(name = "…")]`.
//!
//! Key contexts (`docs/specs/modulos/editor.md`):
//! - `Editor`: always,
//! - `Editor && searching`: while the search bar is open,
//! - `Editor && review_hunk_under_cursor`: while the cursor is inside a hunk.
//!
//! `Ctrl+Enter` and `Ctrl+Backspace` are bound to the review actions **only**
//! in the third context; in plain `Editor` the same keys insert a newline and
//! delete a word, which is the fallthrough `02-visual.md` §8 asks for.

use gpui::KeyBinding;

/// Key-binding context predicate of the editor element.
pub const CONTEXT_EDITOR: &str = "Editor";
/// Key-binding context predicate while the search bar is open.
pub const CONTEXT_SEARCHING: &str = "Editor && searching";
/// Key-binding context predicate while the cursor is inside a review hunk.
pub const CONTEXT_REVIEW: &str = "Editor && review_hunk_under_cursor";

/// Declares unit-struct actions with explicit registered names.
macro_rules! named_actions {
    ($namespace:ident, [ $( $(#[$attr:meta])* $type:ident => $name:literal ),* $(,)? ]) => {
        $(
            $(#[$attr])*
            #[derive(Clone, Debug, Default, PartialEq, gpui::Action)]
            #[action(namespace = $namespace, name = $name)]
            pub struct $type;
        )*
    };
}

named_actions!(editor, [
    /// Moves the cursor one character left.
    MoveLeft => "move_left",
    /// Moves the cursor one character right.
    MoveRight => "move_right",
    /// Moves the cursor one row up.
    MoveUp => "move_up",
    /// Moves the cursor one row down.
    MoveDown => "move_down",
    /// Moves the cursor to the previous word boundary.
    MoveWordLeft => "move_word_left",
    /// Moves the cursor to the next word boundary.
    MoveWordRight => "move_word_right",
    /// Moves the cursor to the first non-blank column, then to column 0.
    MoveToLineStart => "move_to_line_start",
    /// Moves the cursor to the end of the row.
    MoveToLineEnd => "move_to_line_end",
    /// Moves the cursor one screen up.
    MovePageUp => "move_page_up",
    /// Moves the cursor one screen down.
    MovePageDown => "move_page_down",
    /// Moves the cursor to the start of the document.
    MoveToDocumentStart => "move_to_document_start",
    /// Moves the cursor to the end of the document.
    MoveToDocumentEnd => "move_to_document_end",
    /// Extends the selection one character left.
    SelectLeft => "select_left",
    /// Extends the selection one character right.
    SelectRight => "select_right",
    /// Extends the selection one row up.
    SelectUp => "select_up",
    /// Extends the selection one row down.
    SelectDown => "select_down",
    /// Extends the selection to the previous word boundary.
    SelectWordLeft => "select_word_left",
    /// Extends the selection to the next word boundary.
    SelectWordRight => "select_word_right",
    /// Extends the selection to the start of the row.
    SelectToLineStart => "select_to_line_start",
    /// Extends the selection to the end of the row.
    SelectToLineEnd => "select_to_line_end",
    /// Extends the selection one screen up.
    SelectPageUp => "select_page_up",
    /// Extends the selection one screen down.
    SelectPageDown => "select_page_down",
    /// Extends the selection to the start of the document.
    SelectToDocumentStart => "select_to_document_start",
    /// Extends the selection to the end of the document.
    SelectToDocumentEnd => "select_to_document_end",
    /// Selects the whole document.
    SelectAll => "select_all",
    /// Deletes the character before the cursor.
    Backspace => "backspace",
    /// Deletes the character after the cursor.
    Delete => "delete",
    /// Deletes to the previous word boundary.
    DeleteWordLeft => "delete_word_left",
    /// Deletes to the next word boundary.
    DeleteWordRight => "delete_word_right",
    /// Inserts a line break, inheriting the indentation of the current row.
    InsertNewline => "insert_newline",
    /// Indents the selection, or inserts one indent level.
    Tab => "tab",
    /// Removes one indent level.
    Backtab => "backtab",
    /// Undoes the last transaction.
    Undo => "undo",
    /// Redoes the last undone transaction.
    Redo => "redo",
    /// Copies the selection.
    Copy => "copy",
    /// Cuts the selection.
    Cut => "cut",
    /// Pastes the clipboard.
    Paste => "paste",
    /// Opens the search bar.
    Find => "find",
    /// Goes to the next search match.
    FindNext => "find_next",
    /// Goes to the previous search match.
    FindPrev => "find_prev",
    /// Toggles the regular-expression mode of the search bar.
    ToggleSearchRegex => "toggle_search_regex",
    /// Toggles case sensitivity of the search bar.
    ToggleSearchCase => "toggle_search_case",
    /// Opens the "go to line" prompt.
    GoToLine => "go_to_line",
    /// Turns soft wrap on or off for this editor.
    ToggleSoftWrap => "toggle_soft_wrap",
    /// Toggles the whitespace indicators.
    ToggleWhitespace => "toggle_whitespace",
    /// Asks the host to save the buffer (`EditorEvent::SaveRequested`).
    Save => "save",
    /// Closes the search bar or the prompt, or collapses the selection.
    Cancel => "cancel",
    /// Confirms the open prompt.
    Confirm => "confirm",
]);

named_actions!(review, [
    /// Accepts the hunk under the cursor.
    AcceptHunk => "accept_hunk",
    /// Rejects the hunk under the cursor.
    RejectHunk => "reject_hunk",
    /// Accepts the line under the cursor inside a hunk.
    AcceptLine => "accept_line",
    /// Rejects the line under the cursor inside a hunk.
    RejectLine => "reject_line",
    /// Accepts every hunk of the file.
    AcceptFile => "accept_file",
    /// Rejects every hunk of the file.
    RejectFile => "reject_file",
    /// Moves the cursor to the next hunk.
    NextHunk => "next_hunk",
    /// Moves the cursor to the previous hunk.
    PrevHunk => "prev_hunk",
]);

/// The default Linux key bindings of the editor (`docs/specs/02-visual.md` §8
/// for the editor-scoped rows, plus the usual text-editing keys).
///
/// The workspace installs them with `cx.bind_keys(default_key_bindings())`
/// before loading the user's `keymap.json`, which can then override any of
/// them. Order matters inside the vector: when two bindings match the same key
/// on the same node, GPUI keeps the last one, so the review bindings come after
/// the editor ones and win whenever their context is active.
pub fn default_key_bindings() -> Vec<KeyBinding> {
    let editor = Some(CONTEXT_EDITOR);
    let searching = Some(CONTEXT_SEARCHING);
    let review = Some(CONTEXT_REVIEW);
    vec![
        // Movement.
        KeyBinding::new("left", MoveLeft, editor),
        KeyBinding::new("right", MoveRight, editor),
        KeyBinding::new("up", MoveUp, editor),
        KeyBinding::new("down", MoveDown, editor),
        KeyBinding::new("ctrl-left", MoveWordLeft, editor),
        KeyBinding::new("ctrl-right", MoveWordRight, editor),
        KeyBinding::new("home", MoveToLineStart, editor),
        KeyBinding::new("end", MoveToLineEnd, editor),
        KeyBinding::new("pageup", MovePageUp, editor),
        KeyBinding::new("pagedown", MovePageDown, editor),
        KeyBinding::new("ctrl-home", MoveToDocumentStart, editor),
        KeyBinding::new("ctrl-end", MoveToDocumentEnd, editor),
        // Selection.
        KeyBinding::new("shift-left", SelectLeft, editor),
        KeyBinding::new("shift-right", SelectRight, editor),
        KeyBinding::new("shift-up", SelectUp, editor),
        KeyBinding::new("shift-down", SelectDown, editor),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, editor),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, editor),
        KeyBinding::new("shift-home", SelectToLineStart, editor),
        KeyBinding::new("shift-end", SelectToLineEnd, editor),
        KeyBinding::new("shift-pageup", SelectPageUp, editor),
        KeyBinding::new("shift-pagedown", SelectPageDown, editor),
        KeyBinding::new("ctrl-shift-home", SelectToDocumentStart, editor),
        KeyBinding::new("ctrl-shift-end", SelectToDocumentEnd, editor),
        KeyBinding::new("ctrl-a", SelectAll, editor),
        // Editing.
        KeyBinding::new("backspace", Backspace, editor),
        KeyBinding::new("delete", Delete, editor),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, editor),
        KeyBinding::new("ctrl-delete", DeleteWordRight, editor),
        KeyBinding::new("enter", InsertNewline, editor),
        // 02-visual §8: Ctrl+Enter falls through to a plain newline outside a hunk.
        KeyBinding::new("ctrl-enter", InsertNewline, editor),
        KeyBinding::new("tab", Tab, editor),
        KeyBinding::new("shift-tab", Backtab, editor),
        KeyBinding::new("ctrl-z", Undo, editor),
        KeyBinding::new("ctrl-shift-z", Redo, editor),
        KeyBinding::new("ctrl-y", Redo, editor),
        KeyBinding::new("ctrl-c", Copy, editor),
        KeyBinding::new("ctrl-x", Cut, editor),
        KeyBinding::new("ctrl-v", Paste, editor),
        // Search, go to line, view.
        KeyBinding::new("ctrl-f", Find, editor),
        KeyBinding::new("f3", FindNext, editor),
        KeyBinding::new("shift-f3", FindPrev, editor),
        KeyBinding::new("ctrl-g", GoToLine, editor),
        KeyBinding::new("alt-z", ToggleSoftWrap, editor),
        KeyBinding::new("ctrl-s", Save, editor),
        KeyBinding::new("escape", Cancel, editor),
        // Search bar.
        KeyBinding::new("enter", FindNext, searching),
        KeyBinding::new("shift-enter", FindPrev, searching),
        KeyBinding::new("escape", Cancel, searching),
        KeyBinding::new("alt-r", ToggleSearchRegex, searching),
        KeyBinding::new("alt-c", ToggleSearchCase, searching),
        KeyBinding::new("backspace", Backspace, searching),
        // Review: only bound while the cursor is inside a hunk.
        KeyBinding::new("ctrl-enter", AcceptHunk, review),
        KeyBinding::new("ctrl-backspace", RejectHunk, review),
        KeyBinding::new("ctrl-shift-enter", AcceptFile, review),
        KeyBinding::new("ctrl-shift-backspace", RejectFile, review),
        KeyBinding::new("alt-j", NextHunk, editor),
        KeyBinding::new("alt-k", PrevHunk, editor),
        KeyBinding::new("f7", NextHunk, editor),
        KeyBinding::new("shift-f7", PrevHunk, editor),
    ]
}

/// Installs [`default_key_bindings`] into the app.
pub fn bind_default_keys(cx: &mut gpui::App) {
    cx.bind_keys(default_key_bindings());
}
