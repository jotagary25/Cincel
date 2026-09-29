//! The git column of the gutter (`docs/specs/07-etapa5-productividad.md` §6).
//!
//! The editor knows nothing about git: the host hands it rows with
//! [`EditorView::set_git_diff`] — added, modified, or the position of deleted
//! lines — computed against `HEAD` from the file **on disk**. Between two
//! computations the user keeps typing, so the rows are turned into anchors
//! tracked by the buffer itself ([`cincel_text::Buffer::track_anchor`]):
//! every edit, undo, redo, agent write or reload transforms them, and the
//! bars follow the text (three lines typed above a change push its bar three
//! rows down) until the host sends the next diff.
//!
//! What is typed and not saved is not *diffed* until the next save; it only
//! moves the existing bars.
//!
//! # Geometry
//!
//! The git column is 3 px wide, 2 px from the left edge of the gutter and
//! 2 px before the agent's diff bar (see `element.rs`). The gutter's total
//! width did not change to make room for it: the padding and the gap before
//! the numbers gave up the 5 px, so the text never moves.
//!
//! Phantom rows (the agent's deleted lines) never get a git bar: those lines
//! do not exist on disk.

use std::ops::Range;

use cincel_text::{AnchorId, Bias, Buffer, EditSource};
use gpui::{Context, Rgba, rgb};

use crate::settings::SharedBuffer;
use crate::view::EditorView;

/// What a [`GitGutterHunk`] marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GitGutterKind {
    /// Rows that do not exist in `HEAD` (`git.added`).
    Added,
    /// Rows that replace lines of `HEAD` (`git.modified`).
    Modified,
    /// Lines of `HEAD` removed right before [`GitGutterHunk::rows`]`.start`
    /// (`git.deleted`). The range is empty.
    Deleted,
}

/// One hunk of the git column, in buffer rows (zero based).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GitGutterHunk {
    /// What the hunk marks.
    pub kind: GitGutterKind,
    /// Rows covered. For [`GitGutterKind::Deleted`] an empty range whose
    /// `start` is the row that follows the deleted lines (0 when they were
    /// at the top of the file; the row count when they were at the end).
    pub rows: Range<u32>,
}

/// The three colours of the git column, from the theme's `git.*` tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GitGutterColors {
    /// `git.added`.
    pub added: Rgba,
    /// `git.modified`.
    pub modified: Rgba,
    /// `git.deleted`.
    pub deleted: Rgba,
}

impl Default for GitGutterColors {
    /// The values of the bundled dark theme.
    fn default() -> Self {
        Self {
            added: rgb(0x98c379),
            modified: rgb(0x61afef),
            deleted: rgb(0xe06c75),
        }
    }
}

impl GitGutterColors {
    /// The colour of `kind`.
    pub fn color(&self, kind: GitGutterKind) -> Rgba {
        match kind {
            GitGutterKind::Added => self.added,
            GitGutterKind::Modified => self.modified,
            GitGutterKind::Deleted => self.deleted,
        }
    }
}

/// A hunk anchored to the buffer.
#[derive(Clone, Copy, Debug)]
struct AnchoredHunk {
    kind: GitGutterKind,
    /// Start of the first row (or of the row after the gap, for a deletion).
    /// `After` bias: text inserted right at the row start (a newline typed
    /// at column 0) pushes the mark down with the row it belongs to.
    first: AnchorId,
    /// Start of the last row covered; `None` for a deletion.
    last: Option<AnchorId>,
    /// A deletion after the last row of the file: the anchor sits at the end
    /// of the buffer and the mark goes below that row.
    at_end: bool,
}

/// The git column of one view: anchors tracked by the shared buffer.
///
/// Owns a handle on the buffer so it can untrack its anchors when it is
/// replaced or dropped (the buffer outlives the view: the project's store
/// and the agent keep it).
pub(crate) struct GitGutterState {
    buffer: SharedBuffer,
    hunks: Vec<AnchoredHunk>,
    pub(crate) colors: GitGutterColors,
    /// Whether the host ever sent a diff (`None` and "no hunks" both paint
    /// nothing; the distinction is for [`EditorView::git_diff`]).
    present: bool,
}

impl GitGutterState {
    pub(crate) fn new(buffer: SharedBuffer) -> Self {
        Self {
            buffer,
            hunks: Vec::new(),
            colors: GitGutterColors::default(),
            present: false,
        }
    }

    /// Replaces the hunks. `hunks` are in rows of the buffer's **saved**
    /// text (what git diffed); when the buffer has unsaved edits they are
    /// first carried over to the current text, then anchored.
    fn set(&mut self, hunks: Option<Vec<GitGutterHunk>>) {
        let mut buffer = self.buffer.lock();
        for hunk in self.hunks.drain(..) {
            buffer.untrack_anchor(hunk.first);
            if let Some(last) = hunk.last {
                buffer.untrack_anchor(last);
            }
        }
        self.present = hunks.is_some();
        let Some(hunks) = hunks else {
            return;
        };
        let hunks = if buffer.is_dirty() {
            carry_over_unsaved_edits(&buffer, hunks)
        } else {
            hunks
        };
        self.hunks = anchor_hunks(&mut buffer, hunks);
    }

    /// The hunks at the buffer's current version, sorted by row.
    pub(crate) fn resolve(&self) -> Vec<GitGutterHunk> {
        if self.hunks.is_empty() {
            return Vec::new();
        }
        let buffer = self.buffer.lock();
        resolve_hunks(&buffer, &self.hunks)
    }

    pub(crate) fn is_present(&self) -> bool {
        self.present
    }
}

impl Drop for GitGutterState {
    fn drop(&mut self) {
        if !self.hunks.is_empty() {
            self.set(None);
        }
    }
}

/// Tracks `hunks` (rows of `buffer` as it is now) as anchors of `buffer`.
fn anchor_hunks(buffer: &mut Buffer, mut hunks: Vec<GitGutterHunk>) -> Vec<AnchoredHunk> {
    hunks.sort_by_key(|hunk| (hunk.rows.start, hunk.rows.end));
    let rows = buffer.line_count();
    let mut anchored = Vec::with_capacity(hunks.len());
    for hunk in hunks {
        let entry = match hunk.kind {
            GitGutterKind::Deleted => {
                let at_end = hunk.rows.start >= rows;
                let offset = if at_end {
                    buffer.len_bytes()
                } else {
                    buffer.line_start_offset(hunk.rows.start)
                };
                AnchoredHunk {
                    kind: hunk.kind,
                    first: buffer.track_anchor(offset, Bias::After),
                    last: None,
                    at_end,
                }
            }
            GitGutterKind::Added | GitGutterKind::Modified => {
                if hunk.rows.is_empty() || hunk.rows.start >= rows {
                    continue;
                }
                let last_row = (hunk.rows.end - 1).min(rows - 1);
                let first =
                    buffer.track_anchor(buffer.line_start_offset(hunk.rows.start), Bias::After);
                let last = buffer.track_anchor(buffer.line_start_offset(last_row), Bias::After);
                AnchoredHunk {
                    kind: hunk.kind,
                    first,
                    last: Some(last),
                    at_end: false,
                }
            }
        };
        anchored.push(entry);
    }
    anchored
}

/// Moves hunks given in rows of the saved text onto the current text of a
/// buffer with unsaved edits: they are anchored in a scratch copy of the
/// saved text, which is then turned into the current text with the same
/// minimal edits an agent write or a reload uses.
fn carry_over_unsaved_edits(buffer: &Buffer, hunks: Vec<GitGutterHunk>) -> Vec<GitGutterHunk> {
    let mut scratch = Buffer::new(&buffer.saved_snapshot().text());
    let anchored = anchor_hunks(&mut scratch, hunks);
    scratch.set_text_minimal(&buffer.text(), EditSource::User);
    resolve_hunks(&scratch, &anchored)
}

fn resolve_hunks(buffer: &Buffer, hunks: &[AnchoredHunk]) -> Vec<GitGutterHunk> {
    let row_of = |id: AnchorId| {
        buffer
            .resolve_tracked(id)
            .map(|offset| buffer.offset_to_point(offset).row)
    };
    let rows = buffer.line_count();
    let mut resolved: Vec<GitGutterHunk> = hunks
        .iter()
        .filter_map(|hunk| {
            let first = row_of(hunk.first)?;
            match hunk.last {
                None => {
                    // A deletion at the end stays below the last row, however
                    // the file grew.
                    let row = if hunk.at_end && first + 1 >= rows {
                        rows
                    } else {
                        first
                    };
                    Some(GitGutterHunk {
                        kind: hunk.kind,
                        rows: row..row,
                    })
                }
                Some(last) => {
                    let last = row_of(last)?.max(first);
                    Some(GitGutterHunk {
                        kind: hunk.kind,
                        rows: first..last + 1,
                    })
                }
            }
        })
        .collect();
    resolved.sort_by_key(|hunk| (hunk.rows.start, hunk.rows.end));
    resolved
}

impl EditorView {
    /// Shows the file's changes against `HEAD` in the git column of the
    /// gutter, or clears them with `None` (a file git does not track, a
    /// folder without git).
    ///
    /// `hunks` are in rows of the file as last saved — what git diffed —
    /// which are the buffer's rows unless it has unsaved edits, in which
    /// case the editor carries them over to the current text. It then
    /// anchors them, so they follow every edit until the next call. Nothing here can move the text: the git column has its own
    /// 3 px inside the gutter's unchanged width.
    pub fn set_git_diff(&mut self, hunks: Option<Vec<GitGutterHunk>>, cx: &mut Context<Self>) {
        // Anchors are only valid at the version they are made in: pick up
        // anything written through the shared handle first.
        self.sync_buffer(cx);
        self.git_gutter.set(hunks);
        cx.notify();
    }

    /// The git hunks, at the buffer's current version (anchors resolved), or
    /// `None` when the host never sent a diff or cleared it.
    pub fn git_diff(&self) -> Option<Vec<GitGutterHunk>> {
        self.git_gutter
            .is_present()
            .then(|| self.git_gutter.resolve())
    }

    /// The colours of the git column (`git.added`, `git.modified`,
    /// `git.deleted` of the theme in force).
    pub fn set_git_colors(&mut self, colors: GitGutterColors, cx: &mut Context<Self>) {
        if self.git_gutter.colors != colors {
            self.git_gutter.colors = colors;
            cx.notify();
        }
    }

    /// The colours the git column paints with.
    pub fn git_colors(&self) -> GitGutterColors {
        self.git_gutter.colors
    }
}

#[cfg(test)]
// One-range slices are exactly what a single edit looks like.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;
    use crate::settings::shared;

    fn state(text: &str) -> GitGutterState {
        GitGutterState::new(shared(Buffer::new(text)))
    }

    fn hunk(kind: GitGutterKind, rows: Range<u32>) -> GitGutterHunk {
        GitGutterHunk { kind, rows }
    }

    #[test]
    fn anchors_follow_edits_above_inside_and_below() {
        let mut state = state("a\nb\nc\nd\ne\n");
        state.set(Some(vec![
            hunk(GitGutterKind::Modified, 2..3),
            hunk(GitGutterKind::Added, 3..5),
            hunk(GitGutterKind::Deleted, 1..1),
        ]));
        assert_eq!(
            state.resolve(),
            vec![
                hunk(GitGutterKind::Deleted, 1..1),
                hunk(GitGutterKind::Modified, 2..3),
                hunk(GitGutterKind::Added, 3..5),
            ]
        );

        // Three lines typed at the end of row 0 push everything below down.
        {
            let mut buffer = state.buffer.lock();
            let end_of_first = buffer.line_len(0) as usize;
            buffer.edit(&[end_of_first..end_of_first], "\nx\ny\nz");
        }
        assert_eq!(
            state.resolve(),
            vec![
                hunk(GitGutterKind::Deleted, 4..4),
                hunk(GitGutterKind::Modified, 5..6),
                hunk(GitGutterKind::Added, 6..8),
            ]
        );

        // A newline typed at column 0 of the modified row moves its mark
        // down with it.
        {
            let mut buffer = state.buffer.lock();
            let start = buffer.line_start_offset(5);
            buffer.edit(&[start..start], "\n");
        }
        assert_eq!(state.resolve()[1], hunk(GitGutterKind::Modified, 6..7));

        // A line typed inside the added block grows it.
        {
            let mut buffer = state.buffer.lock();
            let start = buffer.line_start_offset(8);
            buffer.edit(&[start..start], "nueva\n");
        }
        assert_eq!(state.resolve()[2], hunk(GitGutterKind::Added, 7..10));
    }

    #[test]
    fn rows_of_the_saved_text_are_carried_over_to_unsaved_edits() {
        let buffer = shared(Buffer::new("a\nb\nc\nd\n"));
        buffer.lock().mark_saved();
        // Two lines typed at the top, not saved yet.
        buffer.lock().edit(&[0..0], "x\ny\n");
        let mut state = GitGutterState::new(buffer.clone());
        // Git diffed the file on disk: row 2 (`c`) modified, a deletion
        // before row 3 (`d`).
        state.set(Some(vec![
            hunk(GitGutterKind::Modified, 2..3),
            hunk(GitGutterKind::Deleted, 3..3),
        ]));
        assert_eq!(
            state.resolve(),
            vec![
                hunk(GitGutterKind::Modified, 4..5),
                hunk(GitGutterKind::Deleted, 5..5),
            ]
        );
    }

    #[test]
    fn a_deletion_at_the_end_stays_below_the_last_row() {
        let mut state = state("a\nb");
        state.set(Some(vec![hunk(GitGutterKind::Deleted, 2..2)]));
        assert_eq!(state.resolve(), vec![hunk(GitGutterKind::Deleted, 2..2)]);
        // Rows past the end of the file are clipped, not panics.
        state.set(Some(vec![
            hunk(GitGutterKind::Added, 5..9),
            hunk(GitGutterKind::Modified, 1..7),
        ]));
        assert_eq!(state.resolve(), vec![hunk(GitGutterKind::Modified, 1..2)]);
    }

    #[test]
    fn replacing_or_dropping_untracks_every_anchor() {
        let buffer = shared(Buffer::new("a\nb\nc\n"));
        let mut state = GitGutterState::new(buffer.clone());
        state.set(Some(vec![
            hunk(GitGutterKind::Added, 0..2),
            hunk(GitGutterKind::Deleted, 2..2),
        ]));
        assert_eq!(buffer.lock().anchors().len(), 3);
        state.set(Some(vec![hunk(GitGutterKind::Modified, 1..2)]));
        assert_eq!(buffer.lock().anchors().len(), 2);
        state.set(None);
        assert_eq!(buffer.lock().anchors().len(), 0);
        assert!(!state.is_present());
        state.set(Some(vec![hunk(GitGutterKind::Modified, 1..2)]));
        drop(state);
        assert_eq!(buffer.lock().anchors().len(), 0);
    }
}
