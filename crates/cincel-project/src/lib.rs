//! cincel-project: ver `docs/specs/modulos/project.md`.
//!
//! The open project, with no GPUI and no async runtime: a file tree, a file
//! watcher, the store of open buffers, the git status and the per-file diff
//! against `HEAD` the editor's gutter paints. Every long
//! operation is expressed as "start it, then drive it from wherever you
//! like", so the caller (`cincel-workspace`) can run it on GPUI's
//! background executor without this crate knowing what an executor is.
//!
//! ```no_run
//! use cincel_project::{Watcher, WatchOptions, Worktree, WorktreeConfig};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // The root directory is ready immediately; the rest arrives as events.
//! let (mut worktree, scan) = Worktree::scan("/proyecto", WorktreeConfig::default())?;
//! std::thread::spawn(move || {
//!     for event in scan {
//!         // send `event` to the UI thread, which calls `worktree.apply(event)`
//!         let _ = event;
//!     }
//! });
//!
//! // File system changes arrive in debounced batches.
//! let (_watcher, events) = Watcher::new("/proyecto", WatchOptions::default())?;
//! while let Ok(batch) = events.recv_blocking() {
//!     for event in &batch {
//!         worktree.apply_fs_event(event);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Comments and documentation are in English; user-facing strings are in
//! Spanish, like the rest of Cincel.

mod buffer_store;
mod diff;
mod git;
mod ignore_rules;
mod recents;
mod snapshot;
mod watcher;
mod worktree;

pub use buffer_store::{
    AgentWrite, BufferChange, BufferHandle, BufferState, BufferStore, ContentHash, DiskState,
    EditSource, OpenError, ReloadOutcome, SaveError,
};
pub use diff::{Edit, apply as apply_edits, minimal_edits};
pub use git::{
    GIT_DEBOUNCE, GIT_DIR_DEBOUNCE, GitDirEvent, GitDirWatcher, GitFileStatus, GitHunk,
    GitHunkKind, GitStatus, GitStatusWatcher, LineDiff, diff_against_head, git_dir,
};
pub use ignore_rules::{ExcludeSet, IgnoreRules};
pub use recents::{MAX_RECENTS, Recents};
pub use snapshot::{
    ProjectSnapshot, RACY_WINDOW, SnapshotChange, SnapshotContent, SnapshotEntry, SnapshotLimits,
    SnapshotSource,
};
pub use watcher::{FsEvent, WATCH_DEBOUNCE, WatchError, WatchOptions, Watcher};
pub use worktree::{Entries, Entry, EntryKind, ScanEvent, Worktree, WorktreeConfig, WorktreeScan};
