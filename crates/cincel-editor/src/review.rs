//! The review state the editor paints, as plain data.
//!
//! This is the interface between the editor and whatever owns the review (the
//! workspace, on top of `cincel-review`). The editor never decides anything:
//! the host hands it a [`ReviewView`] with [`crate::EditorView::set_review`]
//! every time its store changes, and the editor reports every user decision as
//! [`crate::EditorEvent::Review`] with a [`ReviewAction`]. The host applies the
//! action to its engine (which edits the buffer on a rejection) and calls
//! `set_review` again.
//!
//! Every type here is `serde`-free plain data, so the workspace can map its
//! engine types onto them without this crate depending on the engine.

use std::ops::Range;

use crate::display_map::PhantomHunk;

/// What a hunk does to the file, which picks the colour of its gutter border
/// (`02-visual.md` §6.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ReviewHunkKind {
    /// Only adds rows (`diff.gutter.added`).
    Added,
    /// Only deletes rows (`diff.gutter.deleted`).
    Deleted,
    /// Deletes and adds rows (`diff.gutter.modified`).
    #[default]
    Modified,
}

/// Intra-line (word) differences of a hunk, painted over the row backgrounds
/// with the brighter `diff.*.word` tokens.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ReviewWordDiffs {
    /// Byte ranges inside `deleted_lines.join("\n")`.
    pub deleted: Vec<Range<usize>>,
    /// Byte ranges inside the buffer text of `buffer_rows`, joined by `'\n'`
    /// (that is, offsets from the first byte of `buffer_rows.start`).
    pub added: Vec<Range<usize>>,
}

/// One line of a hunk that can be decided on its own (`review::accept_line`).
///
/// A modified line pairs a deleted line with the buffer row that replaced it;
/// a pure deletion or addition has only one side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ReviewLineView {
    /// Index into [`ReviewHunkView::deleted_lines`].
    pub base_line: Option<u32>,
    /// Absolute buffer row (live coordinates, like `buffer_rows`).
    pub buffer_row: Option<u32>,
}

/// A pending hunk of the file, as the editor shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ReviewHunkView {
    /// Stable identity of the hunk, echoed back in every [`ReviewAction`].
    pub id: u64,
    /// Rows of the base text the hunk replaces (informational).
    pub base_rows: Range<u32>,
    /// The deleted lines, without newlines. They become read-only phantom rows
    /// spliced in right before `buffer_rows.start`.
    pub deleted_lines: Vec<String>,
    /// Live buffer rows the agent wrote (empty for a pure deletion). The host
    /// anchors them and refreshes them on every change; between two refreshes
    /// the editor shifts them itself when the user edits above them.
    pub buffer_rows: Range<u32>,
    /// Added, deleted or modified.
    pub kind: ReviewHunkKind,
    /// Word diffs, when the engine computed them (small hunks only).
    pub word_diffs: Option<ReviewWordDiffs>,
    /// The lines that can be decided one by one; `AcceptLine { line }` is an
    /// index into this vector.
    pub lines: Vec<ReviewLineView>,
    /// Still pending from an earlier turn: the pill shows a clock.
    pub from_previous_turn: bool,
}

/// Everything the editor needs to paint the review of its file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ReviewView {
    /// Pending hunks of this file, in any order (the editor sorts them by
    /// `buffer_rows.start`).
    pub hunks: Vec<ReviewHunkView>,
    /// The agent is writing this file: pill and bar show a spinner and every
    /// review control is disabled (editing the text is still allowed).
    pub turn_active: bool,
    /// Pending hunks in this file (the `M` of `cambio N de M`).
    pub pending_in_file: usize,
    /// Pending hunks in the other files of the review.
    pub pending_in_other_files: usize,
    /// The hunk the host considers current (index into `hunks` *as sent*).
    /// When it changes, or after a decision with `jump_to_next_on_decide`, the
    /// editor moves the cursor there and scrolls it into view.
    pub current_index: Option<usize>,
}

/// A decision or a navigation request, emitted as
/// [`crate::EditorEvent::Review`]. The editor never applies it itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReviewAction {
    /// Accept a whole hunk (pill, bar, `Ctrl+Enter`).
    AcceptHunk(u64),
    /// Reject a whole hunk (pill, bar, `Ctrl+Backspace`).
    RejectHunk(u64),
    /// Accept one line: `line` indexes [`ReviewHunkView::lines`].
    AcceptLine {
        /// The hunk id.
        hunk: u64,
        /// Index into the hunk's `lines`.
        line: usize,
    },
    /// Reject one line: `line` indexes [`ReviewHunkView::lines`].
    RejectLine {
        /// The hunk id.
        hunk: u64,
        /// Index into the hunk's `lines`.
        line: usize,
    },
    /// Accept every hunk of the file.
    AcceptFile,
    /// Reject every hunk of the file.
    RejectFile,
    /// Accept every pending change of the agent's turn, in every file (the
    /// bar's "Aceptar todo", same as `workspace::accept_turn`).
    AcceptTurn,
    /// Reject every pending change of the agent's turn, in every file (the
    /// bar's "Rechazar todo"; the host asks for confirmation first).
    RejectTurn,
    /// Go to the next pending hunk (`Alt+J`, `F7`).
    NextHunk,
    /// Go to the previous pending hunk (`Alt+K`, `Shift+F7`).
    PrevHunk,
    /// Go to the next file with pending changes (`Alt+L`).
    NextFile,
    /// Open the review panel ("Revisar todo").
    OpenReviewPanel,
    /// Undo the last rejection (`Alt+Shift+U`).
    UndoLastReject,
}

impl ReviewAction {
    /// Whether the action decides something (and is therefore disabled while
    /// the agent is writing the file).
    pub fn is_decision(&self) -> bool {
        matches!(
            self,
            Self::AcceptHunk(_)
                | Self::RejectHunk(_)
                | Self::AcceptLine { .. }
                | Self::RejectLine { .. }
                | Self::AcceptFile
                | Self::RejectFile
                | Self::AcceptTurn
                | Self::RejectTurn
                | Self::UndoLastReject
        )
    }
}

impl ReviewHunkView {
    /// Index into `lines` of the phantom row showing `deleted_lines[base_line]`.
    pub fn line_for_base(&self, base_line: u32) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| line.base_line == Some(base_line))
    }

    /// Index into `lines` of the line on a buffer row.
    pub fn line_for_buffer_row(&self, buffer_row: u32) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| line.buffer_row == Some(buffer_row))
    }

    /// The display-map model of this hunk.
    pub(crate) fn to_phantom(&self) -> PhantomHunk {
        PhantomHunk {
            insert_before_buffer_row: self.buffer_rows.start,
            deleted_text: self.deleted_lines.clone(),
            added_rows: self.buffer_rows.clone(),
        }
    }

    /// The hunk a legacy [`PhantomHunk`] stands for: every deleted line and
    /// every added row is a line of its own, and there are no word diffs.
    pub fn from_phantom(id: u64, hunk: &PhantomHunk) -> Self {
        let deleted = hunk.deleted_text.len() as u32;
        let added = hunk.added_rows.clone();
        let kind = match (deleted > 0, !added.is_empty()) {
            (true, true) => ReviewHunkKind::Modified,
            (true, false) => ReviewHunkKind::Deleted,
            _ => ReviewHunkKind::Added,
        };
        let lines = (0..deleted)
            .map(|base| ReviewLineView {
                base_line: Some(base),
                buffer_row: None,
            })
            .chain(added.clone().map(|row| ReviewLineView {
                base_line: None,
                buffer_row: Some(row),
            }))
            .collect();
        Self {
            id,
            base_rows: 0..deleted,
            deleted_lines: hunk.deleted_text.clone(),
            buffer_rows: added,
            kind,
            word_diffs: None,
            lines,
            from_previous_turn: false,
        }
    }

    /// Whether two hunks produce the same display rows (same phantom text in
    /// the same place): if so, the display map and the wrap map survive.
    pub(crate) fn same_rows(&self, other: &Self) -> bool {
        self.buffer_rows == other.buffer_rows && self.deleted_lines == other.deleted_lines
    }

    /// The deleted word ranges, split per deleted line, in line-local bytes.
    pub(crate) fn deleted_words_per_line(&self) -> Vec<Vec<Range<u32>>> {
        let mut per_line = vec![Vec::new(); self.deleted_lines.len()];
        let Some(words) = self.word_diffs.as_ref() else {
            return per_line;
        };
        let mut start = 0usize;
        for (ix, line) in self.deleted_lines.iter().enumerate() {
            let end = start + line.len();
            for range in &words.deleted {
                let from = range.start.max(start);
                let to = range.end.min(end);
                if from < to {
                    per_line[ix].push((from - start) as u32..(to - start) as u32);
                }
            }
            start = end + 1;
        }
        per_line
    }

    /// Whether any added word range falls on the buffer row starting `offset`
    /// bytes after the first byte of `buffer_rows.start`, `len` bytes long;
    /// returns them clipped, in row-local bytes.
    pub(crate) fn added_words_on_row(&self, offset: usize, len: usize) -> Vec<Range<u32>> {
        let Some(words) = self.word_diffs.as_ref() else {
            return Vec::new();
        };
        let end = offset + len;
        words
            .added
            .iter()
            .filter_map(|range| {
                let from = range.start.max(offset);
                let to = range.end.min(end);
                (from < to).then(|| (from - offset) as u32..(to - offset) as u32)
            })
            .collect()
    }
}

impl ReviewView {
    /// The review a legacy list of [`PhantomHunk`]s stands for (ids are the
    /// positions after sorting).
    pub fn from_phantoms(hunks: &[PhantomHunk]) -> Self {
        let mut sorted: Vec<&PhantomHunk> = hunks.iter().collect();
        sorted.sort_by_key(|hunk| hunk.insert_before_buffer_row);
        let hunks: Vec<ReviewHunkView> = sorted
            .into_iter()
            .enumerate()
            .map(|(ix, hunk)| ReviewHunkView::from_phantom(ix as u64, hunk))
            .collect();
        Self {
            pending_in_file: hunks.len(),
            hunks,
            ..Default::default()
        }
    }

    /// Index of a hunk by id.
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.hunks.iter().position(|hunk| hunk.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(deleted: &[&str], rows: Range<u32>, words: Option<ReviewWordDiffs>) -> ReviewHunkView {
        ReviewHunkView {
            id: 7,
            base_rows: 0..deleted.len() as u32,
            deleted_lines: deleted.iter().map(|line| line.to_string()).collect(),
            buffer_rows: rows,
            kind: ReviewHunkKind::Modified,
            word_diffs: words,
            lines: Vec::new(),
            from_previous_turn: false,
        }
    }

    #[test]
    fn deleted_words_are_split_per_line() {
        // "a - b" / "ccc": the ranges 2..3 ("-") and 6..9 ("ccc").
        let words = ReviewWordDiffs {
            deleted: vec![2..3, 6..9],
            added: vec![],
        };
        let hunk = hunk(&["a - b", "ccc"], 1..2, Some(words));
        assert_eq!(hunk.deleted_words_per_line(), vec![vec![2..3], vec![0..3]]);
    }

    #[test]
    fn a_word_range_across_lines_is_clipped_on_both() {
        let words = ReviewWordDiffs {
            deleted: std::iter::once(3..7).collect(),
            added: vec![],
        };
        let hunk = hunk(&["abcde", "fgh"], 1..2, Some(words));
        assert_eq!(hunk.deleted_words_per_line(), vec![vec![3..5], vec![0..1]]);
    }

    #[test]
    fn added_words_are_row_local() {
        let words = ReviewWordDiffs {
            deleted: vec![],
            added: vec![1..2, 8..10],
        };
        let hunk = hunk(&[], 3..5, Some(words));
        // Row 3 is bytes 0..5, row 4 starts at 6 ("\n" in between).
        assert_eq!(hunk.added_words_on_row(0, 5), vec![1..2]);
        assert_eq!(hunk.added_words_on_row(6, 5), vec![2..4]);
        assert!(hunk.added_words_on_row(20, 3).is_empty());
    }

    #[test]
    fn legacy_hunks_become_one_line_per_row() {
        let legacy = PhantomHunk {
            insert_before_buffer_row: 4,
            deleted_text: vec!["x".into(), "y".into()],
            added_rows: 4..5,
        };
        let view = ReviewView::from_phantoms(&[legacy]);
        let hunk = &view.hunks[0];
        assert_eq!(hunk.kind, ReviewHunkKind::Modified);
        assert_eq!(hunk.lines.len(), 3);
        assert_eq!(hunk.line_for_base(1), Some(1));
        assert_eq!(hunk.line_for_buffer_row(4), Some(2));
        assert_eq!(view.pending_in_file, 1);
        assert_eq!(hunk.to_phantom().added_rows, 4..5);
    }

    #[test]
    fn decisions_are_told_apart_from_navigation() {
        assert!(ReviewAction::AcceptHunk(1).is_decision());
        assert!(ReviewAction::RejectLine { hunk: 1, line: 0 }.is_decision());
        assert!(!ReviewAction::NextHunk.is_decision());
        assert!(!ReviewAction::OpenReviewPanel.is_decision());
    }
}
