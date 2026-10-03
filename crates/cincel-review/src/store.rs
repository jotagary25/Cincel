//! [`ReviewStore`]: every file in review, the reject undo stack, turns and
//! the per-turn record used to report back to the agent.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use cincel_text::{Anchor, Bias, Buffer, BufferEvent, BufferSnapshot, EditSource, Rope};

use crate::comments::{CommentId, CommentStore};
use crate::file::{FileReview, Touched};
use crate::recompute::{RecomputeJob, RecomputeResult, compute_diff};
use crate::text::{common_prefix, common_suffix, join_lines, normalize};
use crate::types::{
    BufferEdit, FileOp, FileStatus, Hunk, HunkId, LinePair, RecomputeOutcome, Revert, ReviewError,
    ReviewLocation, Tracked, TurnId,
};

/// How many rejects [`ReviewStore::undo_last_reject`] can take back.
pub const UNDO_LIMIT: usize = 32;

/// The review model of a workspace. See the crate docs for the protocol.
#[derive(Debug, Default)]
pub struct ReviewStore {
    pub(crate) files: BTreeMap<PathBuf, FileReview>,
    undo_stack: Vec<RejectUndo>,
    active_turns: Vec<TurnId>,
    records: BTreeMap<TurnId, BTreeMap<PathBuf, TurnRecord>>,
    /// Files with reject edits handed to the host and not seen back yet;
    /// they are never garbage-collected in between.
    expecting_review: BTreeSet<PathBuf>,
    pub(crate) next_id: u64,
    root: Option<PathBuf>,
    /// Comments for the agent (`comments.rs`).
    pub(crate) comments: CommentStore,
}

/// One undoable reject (possibly several files).
#[derive(Debug)]
struct RejectUndo {
    files: Vec<UndoFile>,
    /// Comments this reject marked as rejected (the undo takes it back).
    marks: Vec<CommentId>,
}

#[derive(Debug)]
enum UndoFile {
    /// Put the rejected text back into these regions.
    Edits {
        path: PathBuf,
        regions: Vec<UndoRegion>,
    },
    /// Re-track the file and perform `op` (a file-level reject).
    Restore { review: Box<FileReview>, op: FileOp },
}

/// A region of the buffer that now holds base text, and the text that was
/// rejected from it. The anchors follow every buffer edit.
#[derive(Debug)]
struct UndoRegion {
    range: Range<Anchor>,
    text: String,
    /// Until the reject edit lands, the end sticks to the right of the text
    /// that will be inserted; afterwards it stops growing.
    armed: bool,
}

/// What the agent left in a file during one turn.
#[derive(Clone, Debug, Default)]
struct TurnRecord {
    /// Text the agent left (`None`: it deleted the file).
    agent_text: Option<Rope>,
    /// The agent text after formatting, if the host formatted it.
    formatted: Option<String>,
    /// Text after the review, fixed when the file left the review.
    final_text: Option<Option<Rope>>,
}

/// The two patches of [`ReviewStore::report_for_agent`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentReport {
    /// From the agent's (formatted, if any) text to the text after review.
    pub user_patch: String,
    /// From the agent's text to its formatted version, if the host
    /// formatted it and that changed something.
    pub formatting_patch: Option<String>,
}

/// Header of [`ReviewStore::report_for_agent`].
pub const REPORT_HEADER: &str = "The user made the following updates to your changes:";
/// Header of the formatting section of [`ReviewStore::report_for_agent`].
pub const FORMATTING_HEADER: &str =
    "The following formatting-only changes were also applied to your edits:";

impl ReviewStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Workspace root: paths in agent reports are made relative to it.
    pub fn set_workspace_root(&mut self, root: Option<PathBuf>) {
        self.root = root;
    }

    // ------------------------------------------------------------------ turns

    /// An agent turn starts. Files touched from now on belong to it.
    pub fn begin_turn(&mut self, turn: TurnId) {
        if !self.active_turns.contains(&turn) {
            self.active_turns.push(turn);
        }
    }

    /// An agent turn ended. Records what the agent left in each file it
    /// touched (for [`ReviewStore::report_for_agent`]) and drops files that
    /// ended up unchanged.
    pub fn end_turn(&mut self, turn: TurnId) {
        self.active_turns.retain(|t| *t != turn);
        let touched: Vec<(PathBuf, Option<Rope>)> = self
            .files
            .values()
            .filter(|f| f.turn_id == turn)
            .map(|f| (f.path.clone(), live_text(f)))
            .collect();
        let records = self.records.entry(turn).or_default();
        for (path, text) in touched {
            records.entry(path).or_insert_with(|| TurnRecord {
                agent_text: text,
                ..TurnRecord::default()
            });
        }
        self.gc();
    }

    /// Whether `turn` is running.
    pub fn is_turn_active(&self, turn: TurnId) -> bool {
        self.active_turns.contains(&turn)
    }

    /// Whether the file is being written by a running turn (the UI disables
    /// accept/reject meanwhile).
    pub fn turn_active(&self, path: &Path) -> bool {
        self.files
            .get(path)
            .is_some_and(|f| self.active_turns.contains(&f.turn_id))
    }

    fn current_turn(&self) -> TurnId {
        self.active_turns.last().copied().unwrap_or_default()
    }

    fn turn_for(&self, source: EditSource) -> TurnId {
        match source {
            EditSource::Agent { turn_id } => TurnId(turn_id),
            _ => self.current_turn(),
        }
    }

    // --------------------------------------------------------------- tracking

    /// Fixes the base of `path` from the host's buffer before the agent
    /// touches it. Does nothing (returns `false`) if the file is already in
    /// review: the base persists across turns until everything is decided.
    pub fn capture_base(&mut self, path: &Path, snapshot: &BufferSnapshot) -> bool {
        if self.files.contains_key(path) {
            return false;
        }
        let turn = self.current_turn();
        let file = FileReview::new(
            path.to_owned(),
            snapshot.rope().clone(),
            snapshot.clone(),
            true,
            FileStatus::Modified,
            turn,
            &mut self.next_id,
        );
        self.files.insert(path.to_owned(), file);
        true
    }

    /// [`ReviewStore::capture_base`] from plain text (a file that is not open;
    /// CRLF and BOM are normalized away). The first buffer snapshot the host
    /// reports afterwards is diffed against it.
    pub fn capture_base_text(&mut self, path: &Path, text: &str) -> bool {
        if self.files.contains_key(path) {
            return false;
        }
        let snapshot = detached(text);
        let turn = self.current_turn();
        let file = FileReview::new(
            path.to_owned(),
            snapshot.rope().clone(),
            snapshot,
            false,
            FileStatus::Modified,
            turn,
            &mut self.next_id,
        );
        self.files.insert(path.to_owned(), file);
        true
    }

    /// [`ReviewStore::capture_base_text`] from raw file bytes.
    pub fn capture_base_bytes(&mut self, path: &Path, bytes: &[u8]) -> Result<bool, ReviewError> {
        let text = std::str::from_utf8(bytes).map_err(|_| ReviewError::NotUtf8(path.to_owned()))?;
        Ok(self.capture_base_text(path, text))
    }

    /// The content of `path` changed without the store seeing the individual
    /// buffer events (an agent write reread from disk, a reload, a newly
    /// opened buffer for a tracked path). The store diffs its copy against
    /// `snapshot` and treats the difference as coming from `source`.
    ///
    /// Also the way to (re)attach a buffer instance to a tracked path: after
    /// it, events of that buffer take the fast path.
    pub fn file_written(
        &mut self,
        path: &Path,
        snapshot: &BufferSnapshot,
        source: EditSource,
    ) -> Tracked {
        let turn = self.turn_for(source);
        let Some(file) = self.files.get_mut(path) else {
            return Tracked::default();
        };
        if matches!(file.status, FileStatus::Deleted { .. }) {
            // The file came back.
            file.status = FileStatus::Modified;
            file.current = snapshot.clone();
            file.synced = true;
            file.buffer_version = snapshot.version();
            file.reset_hunks(&mut self.next_id);
            file.bump();
        } else {
            sync_by_diff(
                file,
                snapshot,
                source,
                Some(&mut self.undo_stack),
                &mut self.next_id,
            );
            file.synced = true;
        }
        self.after_edit(path, source, turn)
    }

    /// The agent created `path` with `text`. `previous` is what existed there
    /// before, if anything.
    pub fn file_created(&mut self, path: &Path, text: &str, previous: Option<Rope>) -> Tracked {
        let turn = self.current_turn();
        let source = EditSource::Agent { turn_id: turn.0 };
        let snapshot = detached(text);
        match self.files.get_mut(path) {
            Some(file) if matches!(file.status, FileStatus::Deleted { .. }) => {
                file.status = FileStatus::Modified;
                file.current = snapshot;
                file.synced = false;
                file.reset_hunks(&mut self.next_id);
                file.bump();
            }
            Some(file) => {
                sync_by_diff(
                    file,
                    &snapshot,
                    source,
                    Some(&mut self.undo_stack),
                    &mut self.next_id,
                );
                file.synced = false;
            }
            None => {
                let base = previous
                    .as_ref()
                    .map(|p| Rope::from_str(&normalize(&p.to_string())))
                    .unwrap_or_default();
                let file = FileReview::new(
                    path.to_owned(),
                    base,
                    snapshot,
                    false,
                    FileStatus::Created { previous },
                    turn,
                    &mut self.next_id,
                );
                self.files.insert(path.to_owned(), file);
            }
        }
        self.after_edit(path, source, turn)
    }

    /// The agent deleted `path`, whose content was `previous`.
    pub fn file_deleted(&mut self, path: &Path, previous: Rope) -> Tracked {
        let turn = self.current_turn();
        let status = self.files.get(path).map(|f| f.status.clone());
        match status {
            Some(FileStatus::Created { previous: None }) => {
                // Created and deleted by the agent: nothing to review.
                self.files.remove(path);
                self.record_agent(turn, path, None);
                self.finalize(path, None);
                return Tracked {
                    tracked: false,
                    changed: true,
                    needs_recompute: false,
                };
            }
            Some(FileStatus::Created {
                previous: Some(prev),
            }) => {
                let file = self.files.get_mut(path).expect("tracked");
                file.base = prev.clone();
                file.status = FileStatus::Deleted { previous: prev };
            }
            Some(FileStatus::Modified) => {
                let file = self.files.get_mut(path).expect("tracked");
                file.status = FileStatus::Deleted {
                    previous: file.base.clone(),
                };
            }
            Some(FileStatus::Deleted { .. }) => {}
            None => {
                let base = Rope::from_str(&normalize(&previous.to_string()));
                let file = FileReview::new(
                    path.to_owned(),
                    base.clone(),
                    BufferSnapshot::default(),
                    false,
                    FileStatus::Deleted { previous: base },
                    turn,
                    &mut self.next_id,
                );
                self.files.insert(path.to_owned(), file);
            }
        }
        let file = self.files.get_mut(path).expect("tracked above");
        file.hunks.clear();
        file.current = BufferSnapshot::default();
        file.synced = false;
        file.turn_id = turn;
        file.base_version += 1;
        file.bump();
        self.record_agent(turn, path, None);
        Tracked {
            tracked: true,
            changed: true,
            needs_recompute: false,
        }
    }

    /// Feeds one [`BufferEvent`] of the buffer of `path` to the review.
    ///
    /// `snapshot` must be the buffer right after `event` for the fast path
    /// (its version equals the event's); when the host batches events and
    /// passes a later snapshot, the store falls back to diffing its copy
    /// against it (correct, attributes the whole batch to `source`). `source`
    /// is normally the event's own; it is a parameter so the host can
    /// reclassify an edit.
    ///
    /// - [`EditSource::User`]: [`rebase`](crate::rebase::rebase). Edits away
    ///   from pending hunks are replayed on the base; edits touching a hunk
    ///   grow it (the user's text is on the new side).
    /// - `Agent`, `Review`, `Load`: the change is not in the base; it grows or
    ///   creates hunks. A hunk whose sides became equal (a reject landed, an
    ///   agent edit was undone) disappears.
    pub fn buffer_edited(
        &mut self,
        path: &Path,
        event: &BufferEvent,
        snapshot: &BufferSnapshot,
        source: EditSource,
    ) -> Tracked {
        let BufferEvent::Edited {
            old_ranges,
            new_ranges,
            version,
            ..
        } = event;
        // Reject undo regions follow every edit of the path, tracked or not.
        for undo in &mut self.undo_stack {
            for file in &mut undo.files {
                if let UndoFile::Edits { path: p, regions } = file
                    && p == path
                {
                    for (old, new) in old_ranges.iter().zip(new_ranges) {
                        let range = new.start..new.start + old.len();
                        transform_regions(regions, &range, new.len(), source);
                    }
                }
            }
        }

        let turn = self.turn_for(source);
        let Some(file) = self.files.get_mut(path) else {
            return Tracked::default();
        };
        if matches!(file.status, FileStatus::Deleted { .. }) {
            return Tracked {
                tracked: true,
                ..Tracked::default()
            };
        }
        if file.synced && *version <= file.current.version() {
            // Already incorporated (a batched event after the fallback).
            return Tracked {
                tracked: true,
                needs_recompute: file.needs_recompute,
                ..Tracked::default()
            };
        }
        let fast =
            file.synced && file.current.version() + 1 == *version && snapshot.version() == *version;
        if fast {
            let user = source == EditSource::User;
            let mut touched = Touched::default();
            for (old, new) in old_ranges.iter().zip(new_ranges) {
                let range = new.start..new.start + old.len();
                let text = snapshot.text_in(new.clone());
                file.apply_step(range, &text, user, &mut self.next_id, &mut touched);
            }
            let rebuilt = !touched.dirty.is_empty();
            file.current = snapshot.clone();
            file.buffer_version = snapshot.version();
            file.finish(touched, &mut self.next_id);
            file.bump();
            if rebuilt || !user {
                file.needs_recompute = true;
            }
        } else {
            sync_by_diff(file, snapshot, source, None, &mut self.next_id);
            file.synced = true;
        }
        self.after_edit(path, source, turn)
    }

    /// Bookkeeping shared by every buffer change of a tracked file.
    fn after_edit(&mut self, path: &Path, source: EditSource, turn: TurnId) -> Tracked {
        let Some(file) = self.files.get_mut(path) else {
            return Tracked::default();
        };
        match source {
            EditSource::User => file.user_edited = true,
            EditSource::Agent { .. } => file.turn_id = turn,
            EditSource::Review => {
                self.expecting_review.remove(path);
            }
            EditSource::Load => {}
        }
        let needs_recompute = file.needs_recompute;
        if matches!(source, EditSource::Agent { .. }) {
            let text = live_text(file);
            self.record_agent(turn, path, text);
        }
        self.gc();
        Tracked {
            tracked: self.files.contains_key(path),
            changed: true,
            needs_recompute,
        }
    }

    // -------------------------------------------------------------- recompute

    /// Whether `path` has stale hunks waiting for a recompute.
    pub fn needs_recompute(&self, path: &Path) -> bool {
        self.files.get(path).is_some_and(|f| f.needs_recompute)
    }

    /// A job that recomputes the hunks of `path` off the main thread.
    pub fn recompute_job(&self, path: &Path) -> Option<RecomputeJob> {
        let file = self.files.get(path)?;
        Some(RecomputeJob {
            path: path.to_owned(),
            base: file.base.clone(),
            buffer: file.current.clone(),
            generation: file.generation(),
            token: Arc::clone(&file.token),
        })
    }

    /// Installs the result of a [`RecomputeJob`], unless the file changed
    /// since the job was built.
    pub fn apply_recompute(&mut self, result: RecomputeResult) -> RecomputeOutcome {
        let Some(file) = self.files.get_mut(&result.path) else {
            return RecomputeOutcome::Untracked;
        };
        let Some(data) = result.data else {
            return RecomputeOutcome::Cancelled;
        };
        if result.generation != file.generation() {
            return RecomputeOutcome::Stale;
        }
        file.too_large = data.too_large;
        file.binary = data.binary;
        file.whole_stats = data.stats;
        file.whole_differs = file.base != *file.current.rope();
        if file.file_level_only() {
            file.hunks.clear();
        } else {
            file.install(&data.hunks, &mut self.next_id);
        }
        file.needs_recompute = false;
        file.bump();
        let pending = file.pending_hunks().count();
        let has_work = file.has_pending_work();
        self.gc();
        if has_work {
            RecomputeOutcome::Applied { hunks: pending }
        } else {
            RecomputeOutcome::Resolved
        }
    }

    /// Synchronous recompute. If `snapshot` is not what the store last saw,
    /// the difference is first synced as an [`EditSource::Load`] change.
    pub fn recompute(&mut self, path: &Path, snapshot: &BufferSnapshot) -> RecomputeOutcome {
        let Some(file) = self.files.get(path) else {
            return RecomputeOutcome::Untracked;
        };
        let same = file.current.ptr_eq(snapshot)
            || (file.current.version() == snapshot.version()
                && file.current.same_text_as(snapshot));
        if !same && !matches!(file.status, FileStatus::Deleted { .. }) {
            self.file_written(path, snapshot, EditSource::Load);
        }
        match self.recompute_job(path) {
            Some(job) => {
                let result = job.run();
                self.apply_recompute(result)
            }
            None => RecomputeOutcome::Untracked,
        }
    }

    fn recompute_now(file: &mut FileReview, ids: &mut u64) {
        if let Some(data) = compute_diff(&file.base, file.current.rope(), &|| false) {
            file.too_large = data.too_large;
            file.binary = data.binary;
            file.whole_stats = data.stats;
            if file.file_level_only() {
                file.hunks.clear();
            } else {
                file.install(&data.hunks, ids);
            }
            file.needs_recompute = false;
            file.bump();
        }
    }

    // --------------------------------------------------------------- decisions

    fn find_hunk(&self, id: HunkId) -> Result<(PathBuf, usize), ReviewError> {
        self.files
            .iter()
            .find_map(|(path, file)| {
                file.hunks
                    .iter()
                    .position(|h| h.id == id && h.pending)
                    .map(|index| (path.clone(), index))
            })
            .ok_or(ReviewError::UnknownHunk(id))
    }

    /// Accepts a hunk: its buffer text becomes the base. No I/O.
    pub fn accept_hunk(&mut self, id: HunkId) -> Result<PathBuf, ReviewError> {
        let (path, index) = self.find_hunk(id)?;
        if let Some(span) = self.hunk_decision_span(&path, id) {
            self.mark_comments(&path, Some(&[span]), true);
        }
        let file = self.files.get_mut(&path).expect("found");
        let text = file.buffer_text(&file.hunks[index]);
        file.replace_base_region(index, &text);
        file.hunks.remove(index);
        file.clear_if_equal();
        file.bump();
        if file.hunks.is_empty() && matches!(file.status, FileStatus::Created { .. }) {
            file.status = FileStatus::Modified;
        }
        self.gc();
        Ok(path)
    }

    /// Rejects a hunk: returns the edit that puts the base text back. The
    /// host applies it with [`EditSource::Review`] and saves.
    pub fn reject_hunk(&mut self, id: HunkId) -> Result<Revert, ReviewError> {
        let (path, index) = self.find_hunk(id)?;
        let marks = match self.hunk_decision_span(&path, id) {
            Some(span) => self.mark_comments(&path, Some(&[span]), false),
            None => Vec::new(),
        };
        let file = self.files.get_mut(&path).expect("found");
        let hunk = &file.hunks[index];
        let (edit, region) = narrowed(
            hunk.buffer_byte_range(),
            &file.buffer_text(hunk),
            &file.base_text(hunk),
        );
        file.hunks[index].pending = false;
        file.bump();
        self.push_undo(
            vec![UndoFile::Edits {
                path: path.clone(),
                regions: vec![region],
            }],
            marks,
        );
        self.expecting_review.insert(path.clone());
        Ok(Revert::Edits {
            path,
            edits: vec![edit],
        })
    }

    /// Finds the hunk and line of `pair`, recomputing first when hunks are
    /// stale.
    fn locate_pair(&mut self, path: &Path, pair: &LinePair) -> Result<(usize, usize), ReviewError> {
        let file = self
            .files
            .get_mut(path)
            .ok_or_else(|| ReviewError::UnknownFile(path.to_owned()))?;
        if file.file_level_only() {
            return Err(ReviewError::FileLevelOnly(path.to_owned()));
        }
        if let Some(found) = file.find_pair(pair) {
            return Ok(found);
        }
        if file.has_stale_hunks() {
            Self::recompute_now(file, &mut self.next_id);
            if let Some(found) = file.find_pair(pair) {
                return Ok(found);
            }
        }
        Err(ReviewError::UnknownLine(path.to_owned()))
    }

    /// Accepts one line of a hunk (the base takes that buffer line; a
    /// base-only line is dropped from the base). The rest of the hunk stays
    /// pending, split around the accepted line.
    pub fn accept_line(&mut self, path: &Path, pair: &LinePair) -> Result<(), ReviewError> {
        let (index, line) = self.locate_pair(path, pair)?;
        if let Some(span) = self.line_decision_span(path, pair) {
            self.mark_comments(path, Some(&[span]), true);
        }
        let file = self.files.get_mut(path).expect("located");
        let parts: Vec<String> = file
            .pair_texts(index)
            .into_iter()
            .enumerate()
            .filter_map(|(k, (base, buffer))| if k == line { buffer } else { base })
            .collect();
        let new_base = join_lines(&parts);
        file.replace_base_region(index, &new_base);
        file.refresh_local(index, &mut self.next_id);
        file.clear_if_equal();
        file.bump();
        if file.hunks.is_empty() && matches!(file.status, FileStatus::Created { .. }) {
            file.status = FileStatus::Modified;
        }
        self.gc();
        Ok(())
    }

    /// Rejects one line of a hunk (the buffer takes that base line back; a
    /// buffer-only line is removed). Returns the edit for the host.
    pub fn reject_line(&mut self, path: &Path, pair: &LinePair) -> Result<Revert, ReviewError> {
        let (index, line) = self.locate_pair(path, pair)?;
        let marks = match self.line_decision_span(path, pair) {
            Some(span) => self.mark_comments(path, Some(&[span]), false),
            None => Vec::new(),
        };
        let file = self.files.get_mut(path).expect("located");
        let parts: Vec<String> = file
            .pair_texts(index)
            .into_iter()
            .enumerate()
            .filter_map(|(k, (base, buffer))| if k == line { base } else { buffer })
            .collect();
        let hunk = &file.hunks[index];
        let new_buffer = join_lines(&parts);
        let (edit, region) = narrowed(
            hunk.buffer_byte_range(),
            &file.buffer_text(hunk),
            &new_buffer,
        );
        file.bump();
        self.push_undo(
            vec![UndoFile::Edits {
                path: path.to_owned(),
                regions: vec![region],
            }],
            marks,
        );
        self.expecting_review.insert(path.to_owned());
        Ok(Revert::Edits {
            path: path.to_owned(),
            edits: vec![edit],
        })
    }

    /// Accepts the whole file: the base becomes the buffer (for a deleted
    /// file, the deletion stands). No I/O.
    pub fn accept_file(&mut self, path: &Path) -> Result<(), ReviewError> {
        if self.files.contains_key(path) {
            let spans = self.file_decision_spans(path);
            self.mark_comments(path, spans.as_deref(), true);
        }
        let file = self
            .files
            .get_mut(path)
            .ok_or_else(|| ReviewError::UnknownFile(path.to_owned()))?;
        if matches!(file.status, FileStatus::Deleted { .. }) {
            self.mark_deleted_file(path);
            self.files.remove(path);
            self.finalize(path, None);
            return Ok(());
        }
        file.base = file.current.rope().clone();
        file.base_version += 1;
        file.hunks.clear();
        file.status = FileStatus::Modified;
        file.whole_differs = false;
        file.needs_recompute = false;
        file.bump();
        self.gc();
        Ok(())
    }

    /// Rejects the whole file. A file the agent created (and the user did not
    /// edit) is deleted; a deleted or binary file is written back; anything
    /// else gets buffer edits that restore the base (the file stays).
    pub fn reject_file(&mut self, path: &Path) -> Result<Revert, ReviewError> {
        let (revert, undo, marks) = self.reject_file_inner(path)?;
        self.push_undo(undo.into_iter().collect(), marks);
        Ok(revert)
    }

    fn reject_file_inner(
        &mut self,
        path: &Path,
    ) -> Result<(Revert, Option<UndoFile>, Vec<CommentId>), ReviewError> {
        let marks = if self.files.contains_key(path) {
            let spans = self.file_decision_spans(path);
            self.mark_comments(path, spans.as_deref(), false)
        } else {
            Vec::new()
        };
        let file = self
            .files
            .get_mut(path)
            .ok_or_else(|| ReviewError::UnknownFile(path.to_owned()))?;
        let whole_file_op = match &file.status {
            FileStatus::Deleted { previous } => Some((
                FileOp::WriteFile(path.to_owned(), previous.to_string()),
                FileOp::DeleteFile(path.to_owned()),
                Some(previous.clone()),
            )),
            FileStatus::Created { previous: None } if !file.user_edited => Some((
                FileOp::DeleteFile(path.to_owned()),
                FileOp::WriteFile(path.to_owned(), file.current.text()),
                None,
            )),
            _ if file.binary => Some((
                FileOp::WriteFile(path.to_owned(), file.base.to_string()),
                FileOp::WriteFile(path.to_owned(), file.current.text()),
                Some(file.base.clone()),
            )),
            _ => None,
        };
        if let Some((op, undo_op, final_text)) = whole_file_op {
            let review = self.files.remove(path).expect("tracked");
            self.finalize(path, final_text);
            return Ok((
                Revert::File(op),
                Some(UndoFile::Restore {
                    review: Box::new(review),
                    op: undo_op,
                }),
                marks,
            ));
        }

        let mut edits = Vec::new();
        let mut regions = Vec::new();
        if file.too_large {
            let current = file.current.text();
            let base = file.base.to_string();
            for (range, text) in cincel_text::minimal_edits(&current, &base) {
                let old = current[range.clone()].to_owned();
                edits.push(BufferEdit {
                    range: range.clone(),
                    text,
                });
                regions.push(UndoRegion::new(range, old));
            }
        } else {
            let pending: Vec<usize> = (0..file.hunks.len())
                .filter(|&i| file.hunks[i].pending)
                .collect();
            for &index in &pending {
                file.hunks[index].pending = false;
                let hunk = &file.hunks[index];
                let (edit, region) = narrowed(
                    hunk.buffer_byte_range(),
                    &file.buffer_text(hunk),
                    &file.base_text(hunk),
                );
                edits.push(edit);
                regions.push(region);
            }
        }
        file.status = FileStatus::Modified;
        file.bump();
        edits.reverse();
        if !edits.is_empty() {
            self.expecting_review.insert(path.to_owned());
        }
        let undo = (!regions.is_empty()).then(|| UndoFile::Edits {
            path: path.to_owned(),
            regions,
        });
        Ok((
            Revert::Edits {
                path: path.to_owned(),
                edits,
            },
            undo,
            marks,
        ))
    }

    fn paths_where(&self, keep: impl Fn(&FileReview) -> bool) -> Vec<PathBuf> {
        self.files
            .values()
            .filter(|f| f.has_pending_work() && keep(f))
            .map(|f| f.path.clone())
            .collect()
    }

    /// Accepts every file last touched by `turn`. Returns the files.
    pub fn accept_turn(&mut self, turn: TurnId) -> Vec<PathBuf> {
        let paths = self.paths_where(|f| f.turn_id == turn);
        for path in &paths {
            let _ = self.accept_file(path);
        }
        paths
    }

    /// Rejects every file last touched by `turn`, as one undoable step.
    pub fn reject_turn(&mut self, turn: TurnId) -> Vec<Revert> {
        let paths = self.paths_where(|f| f.turn_id == turn);
        self.reject_paths(&paths)
    }

    /// Accepts everything. Returns the files.
    pub fn accept_all(&mut self) -> Vec<PathBuf> {
        let paths = self.paths_where(|_| true);
        for path in &paths {
            let _ = self.accept_file(path);
        }
        paths
    }

    /// Rejects everything, as one undoable step.
    pub fn reject_all(&mut self) -> Vec<Revert> {
        let paths = self.paths_where(|_| true);
        self.reject_paths(&paths)
    }

    fn reject_paths(&mut self, paths: &[PathBuf]) -> Vec<Revert> {
        let mut reverts = Vec::new();
        let mut undo = Vec::new();
        let mut marks = Vec::new();
        for path in paths {
            if let Ok((revert, file_undo, file_marks)) = self.reject_file_inner(path) {
                reverts.push(revert);
                undo.extend(file_undo);
                marks.extend(file_marks);
            }
        }
        self.push_undo(undo, marks);
        reverts
    }

    fn push_undo(&mut self, files: Vec<UndoFile>, marks: Vec<CommentId>) {
        if files.is_empty() {
            return;
        }
        self.undo_stack.push(RejectUndo { files, marks });
        if self.undo_stack.len() > UNDO_LIMIT {
            self.undo_stack.remove(0);
        }
    }

    /// Whether there is a reject to undo.
    pub fn can_undo_reject(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// How many rejects [`Self::undo_last_reject`] can take back right now
    /// (at most [`UNDO_LIMIT`]): comparing it before and after a reject tells
    /// whether the store stacked one.
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    /// Takes back the last reject: returns what the host must do to put the
    /// rejected text back (the hunks reappear when those edits come back as
    /// [`EditSource::Review`] events).
    pub fn undo_last_reject(&mut self) -> Vec<Revert> {
        let Some(undo) = self.undo_stack.pop() else {
            return Vec::new();
        };
        self.unmark_rejected(&undo.marks);
        let mut reverts = Vec::new();
        for file in undo.files {
            match file {
                UndoFile::Edits { path, regions } => {
                    let Some(review) = self.files.get(&path) else {
                        continue;
                    };
                    let len = review.current.len();
                    let mut edits: Vec<BufferEdit> = regions
                        .into_iter()
                        .map(|region| {
                            let start = region.range.start.offset.min(len);
                            let end = region.range.end.offset.clamp(start, len);
                            BufferEdit {
                                range: start..end,
                                text: region.text,
                            }
                        })
                        .collect();
                    edits.sort_by_key(|e| std::cmp::Reverse(e.range.start));
                    self.expecting_review.insert(path.clone());
                    reverts.push(Revert::Edits { path, edits });
                }
                UndoFile::Restore { review, op } => {
                    let path = review.path.clone();
                    if !self.files.contains_key(&path) {
                        let mut review = *review;
                        review.synced = false;
                        review.token = Arc::new(AtomicU64::new(0));
                        self.files.insert(path.clone(), review);
                        for record in self.records.values_mut() {
                            if let Some(record) = record.get_mut(&path) {
                                record.final_text = None;
                            }
                        }
                    }
                    reverts.push(Revert::File(op));
                }
            }
        }
        reverts
    }

    // ----------------------------------------------------------------- queries

    /// The review state of `path`.
    pub fn file(&self, path: &Path) -> Option<&FileReview> {
        self.files.get(path)
    }

    /// Every file in review, by path.
    pub fn files(&self) -> impl Iterator<Item = &FileReview> {
        self.files.values()
    }

    /// Pending hunks of `path`, in order.
    pub fn hunks(&self, path: &Path) -> Vec<&Hunk> {
        self.files
            .get(path)
            .map(|f| f.pending_hunks().collect())
            .unwrap_or_default()
    }

    /// A pending hunk by id.
    pub fn hunk(&self, id: HunkId) -> Option<(&Path, &Hunk)> {
        self.files.values().find_map(|f| {
            f.pending_hunks()
                .find(|h| h.id == id)
                .map(|h| (f.path.as_path(), h))
        })
    }

    /// `(added, removed)` pending lines of `path`.
    pub fn stats(&self, path: &Path) -> (u32, u32) {
        self.files.get(path).map_or((0, 0), FileReview::stats)
    }

    /// Number of pending decisions: hunks, plus one per file that is only
    /// reviewable as a whole.
    pub fn pending_count(&self) -> usize {
        self.files
            .values()
            .map(|f| {
                if !f.has_pending_work() {
                    0
                } else {
                    f.pending_hunks().count().max(1)
                }
            })
            .sum()
    }

    /// Files with pending decisions, by path.
    pub fn pending_files(&self) -> Vec<PathBuf> {
        self.paths_where(|_| true)
    }

    /// Every navigation stop, ordered by path then row.
    pub fn locations(&self) -> Vec<ReviewLocation> {
        let mut out = Vec::new();
        for file in self.files.values().filter(|f| f.has_pending_work()) {
            let before = out.len();
            if !file.file_level_only() {
                for hunk in file.pending_hunks() {
                    out.push(ReviewLocation {
                        path: file.path.clone(),
                        hunk: Some(hunk.id),
                        row: file
                            .current
                            .offset_to_point(hunk.buffer_range.start.offset)
                            .row,
                    });
                }
            }
            if out.len() == before {
                out.push(ReviewLocation {
                    path: file.path.clone(),
                    hunk: None,
                    row: 0,
                });
            }
        }
        out
    }

    /// The first stop strictly after `(path, row)`, wrapping to the first
    /// one.
    pub fn next_hunk(&self, after: (&Path, u32)) -> Option<ReviewLocation> {
        let locations = self.locations();
        locations
            .iter()
            .find(|l| (l.path.as_path(), l.row) > after)
            .or_else(|| locations.first())
            .cloned()
    }

    /// The last stop strictly before `(path, row)`, wrapping to the last one.
    pub fn prev_hunk(&self, before: (&Path, u32)) -> Option<ReviewLocation> {
        let locations = self.locations();
        locations
            .iter()
            .rev()
            .find(|l| (l.path.as_path(), l.row) < before)
            .or_else(|| locations.last())
            .cloned()
    }

    // ----------------------------------------------------------------- report

    fn record_agent(&mut self, turn: TurnId, path: &Path, text: Option<Rope>) {
        let record = self
            .records
            .entry(turn)
            .or_default()
            .entry(path.to_owned())
            .or_default();
        record.agent_text = text;
        record.final_text = None;
    }

    fn finalize(&mut self, path: &Path, text: Option<Rope>) {
        for records in self.records.values_mut() {
            if let Some(record) = records.get_mut(path)
                && record.final_text.is_none()
            {
                record.final_text = Some(text.clone());
            }
        }
    }

    /// The host formatted what the agent wrote in `path` during `turn`: the
    /// report separates formatting from the user's changes.
    pub fn set_formatted_text(&mut self, turn: TurnId, path: &Path, text: &str) {
        if let Some(record) = self.records.get_mut(&turn).and_then(|r| r.get_mut(path)) {
            record.formatted = Some(normalize(text));
        }
    }

    /// Forgets what the agent left during `turn` (after the report was sent).
    pub fn forget_turn(&mut self, turn: TurnId) {
        self.records.remove(&turn);
    }

    pub(crate) fn display_path(&self, path: &Path) -> String {
        let relative = self
            .root
            .as_ref()
            .and_then(|root| path.strip_prefix(root).ok())
            .unwrap_or(path);
        let text = relative.to_string_lossy().replace('\\', "/");
        text.trim_start_matches('/').to_owned()
    }

    /// The patches for [`ReviewStore::report_for_agent`], or `None` when the
    /// user neither rejected nor edited anything the agent wrote in `turn`.
    pub fn report_patches(&self, turn: TurnId) -> Option<AgentReport> {
        let records = self.records.get(&turn)?;
        let mut user_patch = String::new();
        let mut formatting_patch = String::new();
        for (path, record) in records {
            let after: Option<String> = match &record.final_text {
                Some(text) => text.as_ref().map(Rope::to_string),
                None => match self.files.get(path) {
                    Some(file) => live_text(file).map(|r| r.to_string()),
                    None => continue,
                },
            };
            let agent = record.agent_text.as_ref().map(Rope::to_string);
            let name = self.display_path(path);
            match &record.formatted {
                Some(formatted) => {
                    formatting_patch.push_str(&crate::patch::unified_patch(
                        &name,
                        agent.as_deref(),
                        Some(formatted),
                    ));
                    user_patch.push_str(&crate::patch::unified_patch(
                        &name,
                        Some(formatted),
                        after.as_deref(),
                    ));
                }
                None => user_patch.push_str(&crate::patch::unified_patch(
                    &name,
                    agent.as_deref(),
                    after.as_deref(),
                )),
            }
        }
        if user_patch.is_empty() && formatting_patch.is_empty() {
            return None;
        }
        Some(AgentReport {
            user_patch,
            formatting_patch: (!formatting_patch.is_empty()).then_some(formatting_patch),
        })
    }

    /// A note for the agent's next prompt: for each file it touched in
    /// `turn`, a unified patch from what it left to what is there after the
    /// review, under [`REPORT_HEADER`]; formatting-only changes (see
    /// [`ReviewStore::set_formatted_text`]) go in a separate section. `None`
    /// if there were no rejects nor edits.
    pub fn report_for_agent(&self, turn: TurnId) -> Option<String> {
        let report = self.report_patches(turn)?;
        let mut out = String::new();
        out.push_str(REPORT_HEADER);
        out.push_str("\n\n");
        if report.user_patch.is_empty() {
            out.push_str("(no changes besides formatting)\n");
        } else {
            out.push_str(&report.user_patch);
        }
        if let Some(formatting) = &report.formatting_patch {
            out.push('\n');
            out.push_str(FORMATTING_HEADER);
            out.push_str("\n\n");
            out.push_str(formatting);
        }
        Some(out)
    }

    // --------------------------------------------------------------------- gc

    /// Drops files with nothing left to decide, unless a running turn, the
    /// undo stack or an in-flight reject still needs them.
    fn gc(&mut self) {
        let referenced: BTreeSet<&Path> = self
            .undo_stack
            .iter()
            .flat_map(|u| u.files.iter())
            .filter_map(|f| match f {
                UndoFile::Edits { path, .. } => Some(path.as_path()),
                UndoFile::Restore { .. } => None,
            })
            .collect();
        let removable: Vec<PathBuf> = self
            .files
            .values()
            .filter(|f| {
                f.is_idle()
                    && !self.active_turns.contains(&f.turn_id)
                    && !referenced.contains(f.path.as_path())
                    && !self.expecting_review.contains(&f.path)
            })
            .map(|f| f.path.clone())
            .collect();
        for path in removable {
            if let Some(file) = self.files.remove(&path) {
                self.finalize(&path, Some(file.current.rope().clone()));
            }
        }
    }
}

impl UndoRegion {
    fn new(range: Range<usize>, text: String) -> Self {
        Self {
            range: Anchor::before(range.start)..Anchor::after(range.end),
            text,
            armed: false,
        }
    }
}

/// The edit replacing `old` (at `range`) with `new`, trimmed to what really
/// changes, and the undo region for it.
fn narrowed(range: Range<usize>, old: &str, new: &str) -> (BufferEdit, UndoRegion) {
    let prefix = common_prefix(old, new);
    let suffix = common_suffix(&old[prefix..], &new[prefix..]);
    let start = range.start + prefix;
    let end = range.end - suffix;
    let text = new[prefix..new.len() - suffix].to_owned();
    let rejected = old[prefix..old.len() - suffix].to_owned();
    (
        BufferEdit {
            range: start..end,
            text,
        },
        UndoRegion::new(start..end, rejected),
    )
}

/// Moves undo regions across one edit. A reject edit landing on a region
/// arms it: from then on its end stops sticking to text typed after it.
fn transform_regions(
    regions: &mut [UndoRegion],
    range: &Range<usize>,
    new_len: usize,
    source: EditSource,
) {
    for region in regions.iter_mut() {
        let hit = range.start <= region.range.end.offset && range.end >= region.range.start.offset;
        region.range.start.transform(range, new_len);
        region.range.end.transform(range, new_len);
        if hit && source == EditSource::Review && !region.armed {
            region.armed = true;
            region.range.end.bias = Bias::Before;
        }
    }
}

/// The file's text as the report sees it (`None` for a deleted file).
fn live_text(file: &FileReview) -> Option<Rope> {
    match file.status {
        FileStatus::Deleted { .. } => None,
        _ => Some(file.current.rope().clone()),
    }
}

/// A snapshot not tied to any host buffer.
pub(crate) fn detached(text: &str) -> BufferSnapshot {
    Buffer::new(&normalize(text)).snapshot()
}

/// Brings `file.current` to `snapshot` by diffing, feeding the difference
/// through the edit machinery as coming from `source`. `undo` is given when
/// the undo regions must follow too (no events were seen for this change).
fn sync_by_diff(
    file: &mut FileReview,
    snapshot: &BufferSnapshot,
    source: EditSource,
    mut undo: Option<&mut Vec<RejectUndo>>,
    ids: &mut u64,
) {
    if file.current.same_text_as(snapshot) {
        file.current = snapshot.clone();
        file.buffer_version = snapshot.version();
        return;
    }
    let old = file.current.text();
    let new = snapshot.text();
    let edits = cincel_text::minimal_edits(&old, &new);
    let user = source == EditSource::User;
    let mut touched = Touched::default();
    let mut delta: isize = 0;
    for (range, text) in &edits {
        let start = (range.start as isize + delta) as usize;
        let shifted = start..start + range.len();
        if let Some(undo) = undo.as_deref_mut() {
            for entry in undo.iter_mut() {
                for undo_file in &mut entry.files {
                    if let UndoFile::Edits { path, regions } = undo_file
                        && *path == file.path
                    {
                        transform_regions(regions, &shifted, text.len(), source);
                    }
                }
            }
        }
        file.apply_step(shifted, text, user, ids, &mut touched);
        delta += text.len() as isize - range.len() as isize;
    }
    let rebuilt = !touched.dirty.is_empty();
    file.current = snapshot.clone();
    file.buffer_version = snapshot.version();
    file.finish(touched, ids);
    if file.file_level_only() {
        file.detect_limits();
    }
    file.bump();
    if rebuilt || !user {
        file.needs_recompute = true;
    }
}
