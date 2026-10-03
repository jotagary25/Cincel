//! cincel-review: the review model for agent edits. See
//! `docs/specs/modulos/review.md`.
//!
//! Pure Rust, no GPUI, no async. The algorithm is the one Zed and VS Code
//! converge on: per file a **base** (what the user already accepted) and the
//! live buffer; hunks are the diff between them.
//!
//! - **Accept** writes the buffer text of a hunk into the base. Never touches
//!   the buffer or the disk.
//! - **Reject** returns [`BufferEdit`]s that put the base text back into the
//!   buffer (or a [`FileOp`] for whole-file cases); the host applies them
//!   with [`EditSource::Review`](cincel_text::EditSource::Review) and saves.
//! - **User edits** are rebased: away from pending hunks they go into the
//!   base too, so they never show up as agent changes; touching a hunk they
//!   become part of its new side ([`rebase::rebase`]).
//!
//! # Public API
//!
//! Types: [`ReviewStore`], [`FileReview`], [`Hunk`], [`HunkId`], [`HunkKind`],
//! [`LinePair`], [`WordDiffs`], [`FileStatus`], [`TurnId`], [`BufferEdit`],
//! [`FileOp`], [`Revert`], [`ReviewLocation`], [`Tracked`],
//! [`RecomputeJob`], [`RecomputeResult`], [`RecomputeOutcome`], [`DiffData`],
//! [`AgentReport`], [`LoadReport`], [`DropReason`], [`ReviewError`];
//! comments: [`CommentId`], [`CommentState`], [`CommentView`],
//! [`SentComment`], [`SentRange`], [`CommentDropReason`].
//!
//! [`ReviewStore`] operations:
//!
//! | Group | Methods |
//! |---|---|
//! | turns | `begin_turn`, `end_turn`, `turn_active(path)`, `is_turn_active` |
//! | tracking | `capture_base(path, &BufferSnapshot)`, `capture_base_text`, `capture_base_bytes`, `file_written(path, &BufferSnapshot, EditSource)`, `file_created(path, text, previous)`, `file_deleted(path, previous)`, `buffer_edited(path, &BufferEvent, &BufferSnapshot, EditSource)` |
//! | recompute | `needs_recompute`, `recompute_job`, `apply_recompute`, `recompute(path, &BufferSnapshot)`; pure [`compute_diff`], [`diff::diff_texts`] |
//! | decisions | `accept_hunk`, `reject_hunk`, `accept_line`, `reject_line`, `accept_file`, `reject_file`, `accept_turn`, `reject_turn`, `accept_all`, `reject_all`, `undo_last_reject`, `can_undo_reject`, `undo_depth` |
//! | queries | `file`, `files`, `hunks`, `hunk`, `stats`, `pending_count`, `pending_files`, `locations`, `next_hunk`, `prev_hunk` |
//! | report | `report_for_agent`, `report_patches`, `set_formatted_text`, `forget_turn`, `set_workspace_root` |
//! | comments | `add_comment`, `edit_comment`, `remove_comment`, `drop_comments_in`, `comment_buffer_event`, `comments`, `comment_count`, `comment_count_in`, `commented_paths`, `comment_state`, `take_comments_for_prompt`, `restore_comments`; pure [`format_feedback`] |
//! | persistence | `save(dir)`, `load(dir, read_current)` |
//!
//! # Host protocol
//!
//! 1. **Base.** On the first contact of a turn with a file (agent read,
//!    first `diff` of an edit tool call, first `edit` tool call naming it)
//!    call `capture_base(path, &buffer.snapshot())` (or `capture_base_text`
//!    for a file that is not open). A no-op if the file is already in review.
//!    New files: `file_created`; deletions: `file_deleted`.
//! 2. **Buffer events.** For every [`BufferEvent`](cincel_text::BufferEvent)
//!    of a buffer whose path is in review, call
//!    `buffer_edited(path, &event, &buffer.snapshot(), event_source)`. For the
//!    fast path the snapshot must be taken right after that event (push
//!    subscription, or draining after each transaction); with a later
//!    snapshot the store diffs its own copy against it, which is correct
//!    but attributes the whole batch to one source. The store transforms
//!    its anchors itself (hunk bounds, line anchors, undo regions): the host
//!    never touches them.
//! 3. **Other changes.** When the text changed without events (agent write
//!    reread from disk, reload, a new buffer instance opened for a tracked
//!    path) call `file_written(path, &snapshot, source)`. Opening a new
//!    buffer instance for a tracked path **must** go through it, so versions
//!    are resynced.
//! 4. **Recompute.** Every call returns [`Tracked`]; when `needs_recompute`
//!    is set, debounce ~50 ms, then `store.recompute_job(path)` on the main
//!    thread, `job.run()` on the background executor, and
//!    `store.apply_recompute(result)` back on the main thread. Results for an
//!    outdated state are discarded ([`RecomputeOutcome::Stale`]); a running
//!    job stops early when the file changes ([`RecomputeOutcome::Cancelled`]).
//!    Hunks are exact without the recompute (see the invariants in
//!    `file.rs`); the recompute only improves their shape.
//! 5. **Decisions.** Accepts need nothing else. For a [`Revert`], apply the
//!    edits in one `EditSource::Review` transaction (`Buffer::edit_many`) or
//!    perform the [`FileOp`], then save. The resulting events flow back
//!    through step 2 and close the loop (the rejected hunk disappears; an
//!    undone reject reappears).
//! 6. **Turns.** `begin_turn`/`end_turn` around each prompt; buttons are
//!    disabled while `turn_active(path)`. Before the next prompt,
//!    `take_comments_for_prompt` takes the unsent comments,
//!    `report_for_agent(turn)` gives the patches, then `forget_turn`; and
//!    [`format_feedback`] joins both into the block for the agent. If the
//!    prompt does not go out, `restore_comments` puts the comments back.
//!    Comments follow the buffer through `comment_buffer_event`, fed with
//!    the same events as step 2 (for every watched path, tracked or not).
//! 7. **Persistence.** `save(dir)` after decisions and saves; `load(dir,
//!    read_current)` at startup (never writes files).
//!
//! Limits: files over 2 MB or 50 000 lines are `too_large`, files with a NUL
//! in their first 8 KB are `binary`; both only support whole-file decisions.

mod comments;
pub mod diff;
mod feedback;
mod file;
pub mod patch;
mod persist;
pub mod rebase;
mod recompute;
mod store;
mod text;
mod types;

pub use comments::{
    COMMENT_MAX_CHARS, COMMENT_SNIPPET_MAX_BYTES, COMMENT_SNIPPET_MAX_LINES, CommentDropReason,
    CommentId, CommentState, CommentView, SentComment, SentRange,
};
pub use feedback::{fence_for, format_feedback};
pub use file::FileReview;
pub use persist::{DropReason, LoadReport, STATE_VERSION};
pub use recompute::{DiffData, RecomputeJob, RecomputeResult, compute_diff};
pub use store::{AgentReport, FORMATTING_HEADER, REPORT_HEADER, ReviewStore, UNDO_LIMIT};
pub use types::{
    BufferEdit, FileOp, FileStatus, Hunk, HunkId, HunkKind, LinePair, RecomputeOutcome, Revert,
    ReviewError, ReviewLocation, Tracked, TurnId, WordDiffs,
};
