//! Comments for the agent, as the editor shows them
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §6.2, §6.3,
//! §6.5).
//!
//! Like [`crate::ReviewView`], this is plain data between the editor and its
//! host: the host (the workspace, over `cincel-review`'s comment store) hands
//! the editor the unsent comments of the file with
//! [`crate::EditorView::set_comments`], and the editor reports what the user
//! did in a comment box as a [`CommentAction`] event. The editor never keeps a
//! comment of its own: it closes the box, and the comment shows (folded, with
//! its mark in the margin) once the host answers with the new list.
//!
//! Subscribe with `cx.subscribe(&editor, |_, _, action: &CommentAction, cx| …)`
//! (it is a second event type of [`crate::EditorView`], next to
//! [`crate::EditorEvent`]).

use std::ops::Range;

/// Most characters a comment may have (§6.2.3).
pub const COMMENT_MAX_CHARS: usize = 8_000;
/// From this many characters on, the box says how many are left.
pub const COMMENT_WARN_CHARS: usize = 7_000;
/// Block id of the box of a comment that does not exist yet.
pub const NEW_COMMENT_BLOCK: u64 = u64::MAX;
/// Characters of a comment its margin mark's tooltip shows.
pub const MARK_TOOLTIP_CHARS: usize = 200;

/// Interface texts (Spanish, §7.3).
pub mod texts {
    /// The third button of a hunk's pill.
    pub const COMMENT: &str = "Comentar";
    /// Context menu entry with a selection.
    pub const COMMENT_SELECTION: &str = "Comentar selección";
    /// Context menu entry without a selection.
    pub const COMMENT_LINE: &str = "Comentar línea";
    /// Context menu: cut.
    pub const CUT: &str = "Cortar";
    /// Context menu: copy.
    pub const COPY: &str = "Copiar";
    /// Context menu: paste.
    pub const PASTE: &str = "Pegar";
    /// Title of the open box, before " · calc.py:24-28".
    pub const BOX_TITLE: &str = "Comentario para el agente";
    /// Placeholder of the field.
    pub const PLACEHOLDER: &str = "Escribí qué querés que el agente haga con estas líneas…";
    /// Help under the field.
    pub const HELP: &str = "Ctrl+Enter guarda · Esc cancela";
    /// Save button.
    pub const SAVE: &str = "Guardar";
    /// Cancel button.
    pub const CANCEL: &str = "Cancelar";
    /// Edit button of a saved comment.
    pub const EDIT: &str = "Editar";
    /// Delete button of a saved comment.
    pub const DELETE: &str = "Borrar";
}

/// An unsent comment of the file, as the host hands it to the editor.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ReviewCommentView {
    /// Stable identity, echoed back in [`CommentAction::Edit`] and
    /// [`CommentAction::Delete`].
    pub id: u64,
    /// Buffer rows it talks about (live coordinates, like
    /// [`crate::ReviewHunkView::buffer_rows`]), end exclusive; an empty range
    /// sits right before `rows.start` (a pure deletion). Between two
    /// refreshes of the host the editor shifts them itself when the user
    /// edits above them.
    pub rows: Range<u32>,
    /// What the user wrote.
    pub text: String,
    /// The hunk whose "Comentar" button created it, if any: its box then goes
    /// below that hunk (below its red rows for a pure deletion), and the
    /// button opens it again for editing.
    pub from_hunk: Option<u64>,
}

/// What the user did in a comment box, emitted by [`crate::EditorView`] as an
/// event of its own. The host applies it to its store and answers with
/// [`crate::EditorView::set_comments`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CommentAction {
    /// "Guardar" (or `Ctrl+Enter`) on a new comment.
    Create {
        /// Buffer rows, end exclusive (empty = before `rows.start`).
        rows: Range<u32>,
        /// The hunk whose "Comentar" button opened the box.
        from_hunk: Option<u64>,
        /// The text, never empty, at most [`COMMENT_MAX_CHARS`] characters.
        text: String,
    },
    /// "Guardar" on an existing comment, with a new text.
    Edit {
        /// The comment.
        id: u64,
        /// Its new text.
        text: String,
    },
    /// "Borrar", or "Guardar" with the text emptied.
    Delete {
        /// The comment.
        id: u64,
    },
}

/// What an open comment box is writing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftTarget {
    /// A comment that does not exist yet.
    New {
        /// Buffer rows (end exclusive; empty = before `rows.start`).
        rows: Range<u32>,
        /// The hunk whose "Comentar" opened it.
        from_hunk: Option<u64>,
    },
    /// An existing comment.
    Edit {
        /// The comment.
        id: u64,
    },
}

/// How a comment block is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommentBlockKind {
    /// The box with the field (a new comment, or one being edited).
    Open,
    /// One row: the icon and the text cut with "…".
    Folded,
    /// The whole text, read only.
    Expanded,
}

/// `calc.py:24-28` (or `calc.py:24` for one line or a pure deletion), from a
/// file name and buffer rows (end exclusive). Without a name, `líneas 24-28`
/// / `línea 24`.
pub fn range_label(name: Option<&str>, rows: &Range<u32>) -> String {
    let first = rows.start + 1;
    let last = rows.end.max(rows.start + 1);
    match name {
        Some(name) if first == last => format!("{name}:{first}"),
        Some(name) => format!("{name}:{first}-{last}"),
        None if first == last => format!("línea {first}"),
        None => format!("líneas {first}-{last}"),
    }
}

/// The help line under the field: past [`COMMENT_WARN_CHARS`] it says how
/// many characters are left.
pub fn help_text(chars: usize) -> String {
    if chars > COMMENT_WARN_CHARS {
        let left = COMMENT_MAX_CHARS.saturating_sub(chars);
        let unit = if left == 1 { "carácter" } else { "caracteres" };
        format!("{} · quedan {left} {unit}", texts::HELP)
    } else {
        texts::HELP.to_string()
    }
}

/// The first [`MARK_TOOLTIP_CHARS`] characters of a comment, with "…" when it
/// was longer.
pub fn tooltip_text(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(MARK_TOOLTIP_CHARS).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// A comment on one line, for the folded box (line breaks become spaces; the
/// element cuts it with "…").
pub fn one_line(text: &str) -> String {
    text.split(['\n', '\r'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `text` cut to [`COMMENT_MAX_CHARS`] characters.
pub fn clamp_text(text: &str) -> Option<String> {
    let mut indices = text.char_indices();
    indices
        .nth(COMMENT_MAX_CHARS)
        .map(|(byte, _)| text[..byte].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_the_chat_rule() {
        assert_eq!(range_label(Some("calc.py"), &(23..28)), "calc.py:24-28");
        assert_eq!(range_label(Some("calc.py"), &(23..24)), "calc.py:24");
        // A pure deletion is one line.
        assert_eq!(range_label(Some("calc.py"), &(23..23)), "calc.py:24");
        assert_eq!(range_label(None, &(0..2)), "líneas 1-2");
        assert_eq!(range_label(None, &(4..5)), "línea 5");
    }

    #[test]
    fn the_help_counts_down_past_seven_thousand() {
        assert_eq!(help_text(10), "Ctrl+Enter guarda · Esc cancela");
        assert_eq!(help_text(7_000), "Ctrl+Enter guarda · Esc cancela");
        assert_eq!(
            help_text(7_001),
            "Ctrl+Enter guarda · Esc cancela · quedan 999 caracteres"
        );
        assert_eq!(
            help_text(7_999),
            "Ctrl+Enter guarda · Esc cancela · quedan 1 carácter"
        );
    }

    #[test]
    fn tooltips_and_one_liners() {
        let long = "á".repeat(250);
        let tip = tooltip_text(&long);
        assert_eq!(tip.chars().count(), 201);
        assert!(tip.ends_with('…'));
        assert_eq!(tooltip_text("corto"), "corto");
        assert_eq!(one_line("uno\ndos\r\n\ntres"), "uno dos tres");
    }

    #[test]
    fn texts_are_clamped_by_characters() {
        assert_eq!(clamp_text("hola"), None);
        let long = "ñ".repeat(COMMENT_MAX_CHARS + 5);
        let clamped = clamp_text(&long).unwrap();
        assert_eq!(clamped.chars().count(), COMMENT_MAX_CHARS);
    }
}
