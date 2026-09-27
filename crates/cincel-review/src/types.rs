//! Public value types of the review model.

use std::ops::Range;
use std::path::PathBuf;

use cincel_text::{Anchor, Rope};

/// Identifier of an agent turn. Same number as
/// [`EditSource::Agent { turn_id }`](cincel_text::EditSource::Agent).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TurnId(pub u64);

/// Identifier of a hunk, unique inside one [`ReviewStore`](crate::ReviewStore).
///
/// Ids survive recomputes whenever the new hunk overlaps the old one in the
/// buffer, so a UI can keep a hunk selected while the diff is refreshed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HunkId(pub u64);

/// What a hunk does to the base.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HunkKind {
    /// Only buffer lines, nothing removed from the base.
    Added,
    /// Only base lines, nothing in the buffer.
    Deleted,
    /// Base lines replaced by buffer lines.
    Modified,
}

/// Changed words inside a short hunk.
///
/// Byte ranges are **relative** to the start of the hunk on each side
/// (`base` to [`Hunk::base_byte_range`]`.start`, `buffer` to
/// [`Hunk::buffer_range`]`.start`), so they stay valid while edits elsewhere
/// shift the hunk. Whitespace changes are included.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WordDiffs {
    /// Removed words, relative to the base side of the hunk.
    pub base: Vec<Range<usize>>,
    /// Inserted words, relative to the buffer side of the hunk.
    pub buffer: Vec<Range<usize>>,
}

/// One line of a hunk as seen by the per-line accept/reject buttons.
///
/// `base_row` is a row of [`FileReview::base`](crate::FileReview::base);
/// `buffer_row` is an anchor at the start of a buffer line. A `None` side
/// means the line only exists on the other side (a pure insertion or
/// deletion).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LinePair {
    /// Row in the base, if the line exists there.
    pub base_row: Option<u32>,
    /// Start of the line in the live buffer, if the line exists there.
    pub buffer_row: Option<Anchor>,
}

/// A contiguous change between the base and the buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// Stable id.
    pub id: HunkId,
    /// Rows of the base covered by the hunk (empty for a pure insertion).
    pub base_rows: Range<u32>,
    /// Byte range of the base covered by the hunk. Same region as
    /// `base_rows`, handy for a display layer that splices base text.
    pub base_byte_range: Range<usize>,
    /// Region of the live buffer (empty for a pure deletion).
    pub buffer_range: Range<Anchor>,
    /// Added, deleted or modified.
    pub kind: HunkKind,
    /// Word-level changes, only when both sides have at most 5 lines.
    pub word_diffs: Option<WordDiffs>,
    /// Per-line decomposition. Empty while the hunk is *stale* (an edit
    /// reshaped it and the next recompute has not landed yet); line
    /// operations recompute first in that case.
    pub lines: Vec<LinePair>,
    /// `false` once a reject for this hunk has been handed to the host and
    /// its [`EditSource::Review`](cincel_text::EditSource::Review) edit has
    /// not come back yet. Such hunks are hidden from every query.
    pub(crate) pending: bool,
    /// Whether `lines`/`word_diffs`/`kind` describe the current texts.
    pub(crate) fresh: bool,
}

impl Hunk {
    /// Whether the hunk is still waiting for a decision.
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Whether `lines` and `word_diffs` are up to date (see [`Hunk::lines`]).
    pub fn is_fresh(&self) -> bool {
        self.fresh
    }

    /// Byte range of the hunk in the buffer.
    pub fn buffer_byte_range(&self) -> Range<usize> {
        self.buffer_range.start.offset..self.buffer_range.end.offset
    }
}

/// How the file relates to what existed before the agent touched it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileStatus {
    /// The file existed and was changed.
    Modified,
    /// The agent created the file. `previous` is what was there before, if
    /// anything (the agent recreated an existing path).
    Created {
        /// Content before the agent created the file.
        previous: Option<Rope>,
    },
    /// The agent deleted the file; `previous` is the text to restore.
    Deleted {
        /// Content to restore on reject.
        previous: Rope,
    },
}

/// One replacement the host applies to the buffer, in the buffer's current
/// coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferEdit {
    /// Byte range to replace.
    pub range: Range<usize>,
    /// Replacement text.
    pub text: String,
}

/// A file-level operation the host performs on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileOp {
    /// Delete the file (rejecting a file the agent created).
    DeleteFile(PathBuf),
    /// Write the file with this text (restoring a file the agent deleted, or
    /// re-creating one when a reject is undone).
    WriteFile(PathBuf, String),
}

/// What the host must do to carry out a reject (or an undo of one).
///
/// Edits come **bottom-up** (descending offsets): apply them in one
/// [`EditSource::Review`](cincel_text::EditSource::Review) transaction with
/// `Buffer::edit_many`, or one by one in the given order; either way offsets
/// stay valid. Then save the buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Revert {
    /// Edit the buffer of `path`.
    Edits {
        /// File to edit.
        path: PathBuf,
        /// Replacements, bottom-up.
        edits: Vec<BufferEdit>,
    },
    /// Perform a file operation.
    File(FileOp),
}

impl Revert {
    /// The file this revert is about.
    pub fn path(&self) -> &std::path::Path {
        match self {
            Revert::Edits { path, .. } => path,
            Revert::File(FileOp::DeleteFile(path)) | Revert::File(FileOp::WriteFile(path, _)) => {
                path
            }
        }
    }
}

/// A place the "next/previous change" navigation can land on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewLocation {
    /// File.
    pub path: PathBuf,
    /// Hunk, or `None` for a file that is only reviewable as a whole
    /// (deleted, too large or binary, or created empty).
    pub hunk: Option<HunkId>,
    /// First buffer row of the hunk (0 for file-level entries).
    pub row: u32,
}

/// What [`ReviewStore::buffer_edited`](crate::ReviewStore::buffer_edited)
/// and friends did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tracked {
    /// The file is in review (otherwise the call was ignored).
    pub tracked: bool,
    /// Hunks or base changed: the UI should refresh.
    pub changed: bool,
    /// Hunks are stale: schedule a (debounced) recompute.
    pub needs_recompute: bool,
}

/// Result of applying a recompute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecomputeOutcome {
    /// New hunks were installed.
    Applied {
        /// Number of pending hunks after the recompute.
        hunks: usize,
    },
    /// The result was computed for an older state and was discarded.
    Stale,
    /// The job noticed it was outdated and stopped early.
    Cancelled,
    /// The file is not in review.
    Untracked,
    /// Base and buffer are equal: the file left the review.
    Resolved,
}

/// Errors of review operations and persistence.
#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    /// The path is not in review.
    #[error("file not in review: {0}")]
    UnknownFile(PathBuf),
    /// No pending hunk with this id.
    #[error("unknown hunk {0:?}")]
    UnknownHunk(HunkId),
    /// The line pair is not part of any pending hunk (stale value).
    #[error("line pair not found in {0}")]
    UnknownLine(PathBuf),
    /// The file only supports whole-file decisions.
    #[error("file only supports whole-file review: {0}")]
    FileLevelOnly(PathBuf),
    /// Base bytes are not UTF-8.
    #[error("file is not valid UTF-8: {0}")]
    NotUtf8(PathBuf),
    /// Persistence I/O.
    #[error("review state I/O: {0}")]
    Io(#[from] std::io::Error),
    /// Persistence format.
    #[error("review state format: {0}")]
    Format(#[from] serde_json::Error),
}
