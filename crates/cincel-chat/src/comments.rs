//! Review comments as the chat shows them
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §6.6).
//!
//! The chat never depends on `cincel-review`: the workspace translates its
//! `CommentView`s into [`PendingComment`]s (the tags inside the composer box,
//! [`crate::ChatPanel::set_pending_comments`]) and the `SentComment`s of a
//! prompt into [`SentCommentCard`]s (the cards of the sent message,
//! [`crate::ChatPanel::attach_sent_comments`]). [`comment_tag_labels`] and
//! [`comment_tag_tooltip`] build the tag texts the way §6.6 words them, so the
//! workspace does not have to.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// "pendiente": a pending hunk meets the commented lines.
pub const COMMENT_STATE_PENDING: &str = "pendiente";
/// "aceptado": only accepts touched the lines.
pub const COMMENT_STATE_ACCEPTED: &str = "aceptado";
/// "rechazado": only rejects touched the lines.
pub const COMMENT_STATE_REJECTED: &str = "rechazado";
/// "mixto": accepts and rejects (line by line).
pub const COMMENT_STATE_MIXED: &str = "mixto";
/// "sin cambios del agente": a plain selection.
pub const COMMENT_STATE_NO_AGENT_CHANGE: &str = "sin cambios del agente";
/// What a card of a message that never left says (D16).
pub const COMMENT_NOT_SENT: &str = "No se envió: el comentario volvió al margen";
/// Lines of code a card shows before "… N líneas más".
pub const CARD_CODE_LINES: usize = 6;
/// Characters of the comment's text in a tag's tooltip.
pub const TAG_TOOLTIP_TEXT_CHARS: usize = 200;

/// An unsent comment: one tag of the composer box, with its `×`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingComment {
    /// The store's comment id; the `×` emits
    /// [`crate::ChatEvent::RemoveComment`] with it.
    pub id: u64,
    /// "calc.py:24-28" ([`comment_tag_labels`]).
    pub label: String,
    /// "src/calc.py, líneas 24 a 28" and the text below
    /// ([`comment_tag_tooltip`]).
    pub tooltip: String,
    /// The file, as the workspace keys it; a click on the tag emits
    /// [`crate::ChatEvent::OpenLocation`] with it.
    pub path: PathBuf,
    /// First line of the comment, 1-based.
    pub line: u32,
}

/// What the commented lines are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SentCommentKind {
    /// Lines of the file as it is now.
    Lines,
    /// Where a pure deletion removed lines, before `first_line`.
    RemovedBefore,
    /// A file the agent deleted.
    DeletedFile,
}

/// A comment that left with a prompt: a card of the sent message, stored with
/// the conversation ([`crate::MessageBlock::Comment`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentCommentCard {
    /// Path relative to the project root, `/` separated.
    pub display_path: String,
    /// The file, as the workspace keys it (the header opens it).
    pub path: PathBuf,
    /// First line, 1-based.
    pub first_line: u32,
    /// Last line, 1-based, inclusive.
    pub last_line: u32,
    /// What the lines are.
    pub kind: SentCommentKind,
    /// One of the `COMMENT_STATE_*` texts.
    pub state_label: String,
    /// The code as it was sent (`None` for a pure deletion or a deleted file).
    pub code: Option<String>,
    /// The removed lines of a pending pure deletion or deleted file.
    pub removed: Option<String>,
    /// Lines the size limits left out of what was sent.
    pub truncated_lines: u32,
    /// Fence language (the extension), if any.
    pub lang: Option<String>,
    /// The user's text.
    pub text: String,
    /// The message never left: the comment went back to the margin (D16).
    #[serde(default)]
    pub not_sent: bool,
}

impl SentCommentCard {
    /// The file name the header shows ("calc.py").
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.display_path
            .rsplit('/')
            .next()
            .unwrap_or(&self.display_path)
    }

    /// " · líneas 24-28 · rechazado", the muted part of the header.
    #[must_use]
    pub fn header_detail(&self) -> String {
        let lines = match self.kind {
            SentCommentKind::DeletedFile => "archivo borrado".to_string(),
            SentCommentKind::RemovedBefore => {
                format!("líneas quitadas antes de la {}", self.first_line)
            }
            SentCommentKind::Lines if self.first_line == self.last_line => {
                format!("línea {}", self.first_line)
            }
            SentCommentKind::Lines => format!("líneas {}-{}", self.first_line, self.last_line),
        };
        format!(" · {lines} · {}", self.state_label)
    }

    /// The code the card shows: the current code, or the removed lines.
    #[must_use]
    pub fn shown_code(&self) -> Option<&str> {
        self.code
            .as_deref()
            .or(self.removed.as_deref())
            .filter(|code| !code.is_empty())
    }
}

/// One comment as [`comment_tag_labels`] needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TagSource<'a> {
    /// Path relative to the project root, `/` separated.
    pub display_path: &'a str,
    /// First line, 1-based.
    pub first_line: u32,
    /// Last line, 1-based, inclusive.
    pub last_line: u32,
    /// What the lines are.
    pub kind: SentCommentKind,
}

/// The tags of §6.6: `calc.py:24-28`, `calc.py:24` (one line or a pure
/// deletion), `old.py (borrado)`; two comments on files with the same name in
/// different folders use the relative path instead of the name (like the
/// mentions).
#[must_use]
pub fn comment_tag_labels(sources: &[TagSource<'_>]) -> Vec<String> {
    let name_of = |path: &str| path.rsplit('/').next().unwrap_or(path).to_string();
    sources
        .iter()
        .map(|source| {
            let name = name_of(source.display_path);
            let clashes = sources.iter().any(|other| {
                other.display_path != source.display_path && name_of(other.display_path) == name
            });
            let shown = if clashes {
                source.display_path.to_string()
            } else {
                name
            };
            match source.kind {
                SentCommentKind::DeletedFile => format!("{shown} (borrado)"),
                SentCommentKind::RemovedBefore => format!("{shown}:{}", source.first_line),
                SentCommentKind::Lines if source.first_line == source.last_line => {
                    format!("{shown}:{}", source.first_line)
                }
                SentCommentKind::Lines => {
                    format!("{shown}:{}-{}", source.first_line, source.last_line)
                }
            }
        })
        .collect()
}

/// A tag's tooltip: "src/calc.py, líneas 24 a 28" and, below, the first
/// [`TAG_TOOLTIP_TEXT_CHARS`] characters of the text.
#[must_use]
pub fn comment_tag_tooltip(source: TagSource<'_>, text: &str) -> String {
    let lines = match source.kind {
        SentCommentKind::DeletedFile => "archivo borrado".to_string(),
        SentCommentKind::RemovedBefore => {
            format!("líneas quitadas antes de la {}", source.first_line)
        }
        SentCommentKind::Lines if source.first_line == source.last_line => {
            format!("línea {}", source.first_line)
        }
        SentCommentKind::Lines => {
            format!("líneas {} a {}", source.first_line, source.last_line)
        }
    };
    let mut excerpt: String = text.chars().take(TAG_TOOLTIP_TEXT_CHARS).collect();
    if text.chars().count() > TAG_TOOLTIP_TEXT_CHARS {
        excerpt.push('…');
    }
    format!("{}, {lines}\n{excerpt}", source.display_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(path: &str, first: u32, last: u32, kind: SentCommentKind) -> TagSource<'_> {
        TagSource {
            display_path: path,
            first_line: first,
            last_line: last,
            kind,
        }
    }

    #[test]
    fn tags_read_like_the_spec() {
        let labels = comment_tag_labels(&[
            source("src/calc.py", 24, 28, SentCommentKind::Lines),
            source("src/calc.py", 7, 7, SentCommentKind::Lines),
            source("util.py", 3, 3, SentCommentKind::RemovedBefore),
            source("old.py", 1, 9, SentCommentKind::DeletedFile),
        ]);
        assert_eq!(
            labels,
            [
                "calc.py:24-28",
                "calc.py:7",
                "util.py:3",
                "old.py (borrado)"
            ]
        );
    }

    #[test]
    fn same_name_in_two_folders_uses_the_relative_path() {
        let labels = comment_tag_labels(&[
            source("src/main.rs", 1, 2, SentCommentKind::Lines),
            source("tests/main.rs", 5, 5, SentCommentKind::Lines),
            source("lib.rs", 5, 6, SentCommentKind::Lines),
        ]);
        assert_eq!(labels, ["src/main.rs:1-2", "tests/main.rs:5", "lib.rs:5-6"]);
    }

    #[test]
    fn the_tooltip_names_the_lines_and_clips_the_text() {
        let long = "a".repeat(250);
        let tooltip =
            comment_tag_tooltip(source("src/calc.py", 24, 28, SentCommentKind::Lines), &long);
        let (first, second) = tooltip.split_once('\n').expect("dos líneas");
        assert_eq!(first, "src/calc.py, líneas 24 a 28");
        assert_eq!(second.chars().count(), TAG_TOOLTIP_TEXT_CHARS + 1);
        assert_eq!(
            comment_tag_tooltip(source("a.py", 3, 3, SentCommentKind::Lines), "hola"),
            "a.py, línea 3\nhola"
        );
    }

    #[test]
    fn card_headers_name_file_lines_and_state() {
        let mut card = SentCommentCard {
            display_path: "src/calc.py".into(),
            path: PathBuf::from("/p/src/calc.py"),
            first_line: 24,
            last_line: 28,
            kind: SentCommentKind::Lines,
            state_label: COMMENT_STATE_REJECTED.into(),
            code: Some("x = 1".into()),
            removed: None,
            truncated_lines: 0,
            lang: Some("py".into()),
            text: "usá un diccionario".into(),
            not_sent: false,
        };
        assert_eq!(card.file_name(), "calc.py");
        assert_eq!(card.header_detail(), " · líneas 24-28 · rechazado");
        card.last_line = 24;
        card.state_label = COMMENT_STATE_NO_AGENT_CHANGE.into();
        assert_eq!(card.header_detail(), " · línea 24 · sin cambios del agente");
        card.kind = SentCommentKind::RemovedBefore;
        card.code = None;
        card.removed = Some("viejo".into());
        assert_eq!(card.shown_code(), Some("viejo"));
    }
}
