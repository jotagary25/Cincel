//! One file in review: base, hunks and the edit machinery that keeps them
//! exact between recomputes.
//!
//! # Invariants
//!
//! After every public operation, for a file with inline hunks:
//!
//! 1. **Segments**: buffer and base decompose into the same equal segments
//!    separated by the hunks (see [`crate::rebase`]). Accept and reject are
//!    therefore exact even before the next recompute lands.
//! 2. **Lines**: every hunk covers whole lines on both sides. Edits that
//!    break this (a user deleting the newline right before a hunk, an agent
//!    changing one word) are repaired by growing the hunk to its lines,
//!    merging with a neighbour if the growth reaches it.
//! 3. **Order**: hunks are sorted and disjoint in the buffer and in the base.
//!
//! Anchors are transformed explicitly, not through a generic bias rule
//! (`AnchorMap`): which side of an edit a hunk boundary falls on is decided
//! by the rebase, and a bias transform would collapse a hunk start sitting at
//! the end of a replaced range onto the start of the replacement.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cincel_text::{Anchor, BufferSnapshot, Rope};

use crate::diff::{self, Origin, RawHunk};
use crate::rebase::{self, HunkSpan};
use crate::text::{
    at_line_end, at_line_start, count_newlines, next_newline, prev_newline, rope_is_binary,
    rope_is_too_large, rope_newlines, rope_replace, rope_text,
};
use crate::types::{FileStatus, Hunk, HunkId, HunkKind, LinePair, TurnId};

/// Regions larger than this (base + buffer bytes) are not re-diffed
/// synchronously after an edit; they wait for the background recompute.
pub(crate) const LOCAL_REFRESH_LIMIT: usize = 256 * 1024;

/// Review state of one file.
///
/// The public fields follow `docs/specs/modulos/review.md`. `hunks` also
/// holds hunks whose reject is in flight (see [`Hunk::is_pending`]); use
/// [`FileReview::pending_hunks`] for what the UI shows.
#[derive(Clone, Debug)]
pub struct FileReview {
    /// Path, as the host keys it.
    pub path: PathBuf,
    /// Accepted text: what existed before the agent, advanced by accepts.
    pub base: Rope,
    /// Modified, created or deleted.
    pub status: FileStatus,
    /// Last turn that touched the file.
    pub turn_id: TurnId,
    /// Hunks, sorted by position.
    pub hunks: Vec<Hunk>,
    /// Bumped on every change of `base`.
    pub base_version: u64,
    /// Version of the buffer snapshot the store last saw.
    pub buffer_version: u64,
    /// Over 2 MB or 50 000 lines: whole-file review only.
    pub too_large: bool,
    /// NUL byte in the first 8 KB: whole-file review only.
    pub binary: bool,
    /// The store's copy of the buffer.
    pub(crate) current: BufferSnapshot,
    /// Whether `current` is a snapshot of the host's buffer (so its version
    /// can be compared with events), rather than a detached copy.
    pub(crate) synced: bool,
    /// Generation counter shared with recompute jobs.
    pub(crate) token: Arc<AtomicU64>,
    pub(crate) needs_recompute: bool,
    pub(crate) user_edited: bool,
    pub(crate) created_at: u64,
    /// `(added, removed)` of the last whole-file diff (file-level files).
    pub(crate) whole_stats: (u32, u32),
    /// Whether base and buffer differ (file-level files).
    pub(crate) whole_differs: bool,
}

/// Ids of hunks an edit step reshaped (`dirty`) or may have misaligned
/// (`check`).
#[derive(Default)]
pub(crate) struct Touched {
    pub dirty: Vec<HunkId>,
    pub check: Vec<HunkId>,
}

impl FileReview {
    pub(crate) fn new(
        path: PathBuf,
        base: Rope,
        current: BufferSnapshot,
        synced: bool,
        status: FileStatus,
        turn_id: TurnId,
        ids: &mut u64,
    ) -> Self {
        let mut file = Self {
            path,
            base,
            status,
            turn_id,
            hunks: Vec::new(),
            base_version: 0,
            buffer_version: current.version(),
            too_large: false,
            binary: false,
            current,
            synced,
            token: Arc::new(AtomicU64::new(0)),
            needs_recompute: false,
            user_edited: false,
            created_at: now_secs(),
            whole_stats: (0, 0),
            whole_differs: false,
        };
        file.reset_hunks(ids);
        file
    }

    /// Replaces the hunks with one coarse hunk over the whole texts (or none
    /// if they are equal), then refines it when it is small enough.
    pub(crate) fn reset_hunks(&mut self, ids: &mut u64) {
        self.hunks.clear();
        self.detect_limits();
        self.needs_recompute = true;
        if self.file_level_only() {
            return;
        }
        if self.base == *self.current.rope() {
            return;
        }
        let id = next_id(ids);
        self.hunks.push(Hunk {
            id,
            base_rows: 0..0,
            base_byte_range: 0..self.base.len_bytes(),
            buffer_range: Anchor::before(0)..Anchor::before(self.current.len()),
            kind: HunkKind::Modified,
            word_diffs: None,
            lines: Vec::new(),
            pending: true,
            fresh: false,
        });
        self.fix_rows(0);
        self.refresh_local(0, ids);
    }

    /// Re-evaluates `too_large` / `binary` and the whole-file flags.
    pub(crate) fn detect_limits(&mut self) {
        let current = self.current.rope();
        self.binary = rope_is_binary(&self.base) || rope_is_binary(current);
        self.too_large = rope_is_too_large(&self.base) || rope_is_too_large(current);
        if self.file_level_only() {
            self.hunks.clear();
        }
        self.whole_differs = self.base != *current;
    }

    /// Whether only whole-file decisions are possible.
    pub fn file_level_only(&self) -> bool {
        self.too_large || self.binary || matches!(self.status, FileStatus::Deleted { .. })
    }

    /// The store's view of the buffer text.
    pub fn current(&self) -> &BufferSnapshot {
        &self.current
    }

    /// Seconds since the Unix epoch when the file entered review.
    pub fn created_at(&self) -> u64 {
        self.created_at
    }

    /// Whether the user edited the file while it was in review.
    pub fn user_edited(&self) -> bool {
        self.user_edited
    }

    /// Hunks waiting for a decision, in order.
    pub fn pending_hunks(&self) -> impl Iterator<Item = &Hunk> {
        self.hunks.iter().filter(|h| h.pending)
    }

    /// Whether the file still needs a decision.
    pub fn has_pending_work(&self) -> bool {
        match self.status {
            FileStatus::Deleted { .. } | FileStatus::Created { .. } => true,
            FileStatus::Modified => {
                if self.too_large || self.binary {
                    self.whole_differs
                } else {
                    self.hunks.iter().any(|h| h.pending)
                }
            }
        }
    }

    /// Nothing pending and nothing in flight.
    pub(crate) fn is_idle(&self) -> bool {
        !self.has_pending_work() && self.hunks.is_empty()
    }

    /// Buffer rows of a hunk, resolved against the store's snapshot (rows as
    /// the editor counts them).
    pub fn buffer_rows(&self, hunk: &Hunk) -> Range<u32> {
        let range = hunk.buffer_byte_range();
        let start = self.current.offset_to_point(range.start).row;
        if range.is_empty() {
            return start..start;
        }
        let lines = diff::split_lines(&self.current.text_in(range)).len() as u32;
        start..start + lines
    }

    /// `(added, removed)` lines still pending.
    pub fn stats(&self) -> (u32, u32) {
        match &self.status {
            FileStatus::Deleted { previous } => (0, crate::text::rope_rows(previous) as u32),
            _ if self.too_large || self.binary => self.whole_stats,
            _ => self.pending_hunks().fold((0, 0), |(added, removed), hunk| {
                let rows = self.buffer_rows(hunk);
                (
                    added + (rows.end - rows.start),
                    removed + (hunk.base_rows.end - hunk.base_rows.start),
                )
            }),
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.token.load(Ordering::SeqCst)
    }

    pub(crate) fn bump(&mut self) {
        self.token.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn index_of(&self, id: HunkId) -> Option<usize> {
        self.hunks.iter().position(|h| h.id == id)
    }

    pub(crate) fn spans(&self) -> Vec<HunkSpan> {
        self.hunks
            .iter()
            .map(|h| HunkSpan {
                buffer: h.buffer_byte_range(),
                base: h.base_byte_range.clone(),
            })
            .collect()
    }

    pub(crate) fn base_text(&self, hunk: &Hunk) -> String {
        rope_text(&self.base, hunk.base_byte_range.clone())
    }

    pub(crate) fn buffer_text(&self, hunk: &Hunk) -> String {
        self.current.text_in(hunk.buffer_byte_range())
    }

    // ------------------------------------------------------------ edit steps

    /// Applies one replacement of the buffer (`range` in current
    /// coordinates, replaced by `text`) to the review state. The buffer
    /// snapshot is updated by the caller once all steps of an event ran.
    pub(crate) fn apply_step(
        &mut self,
        range: Range<usize>,
        text: &str,
        user: bool,
        ids: &mut u64,
        touched: &mut Touched,
    ) {
        if self.file_level_only() {
            return;
        }
        let spans = self.spans();
        let delta = text.len() as isize - range.len() as isize;
        let before = spans.partition_point(|h| h.buffer.end <= range.start);

        if user && let Ok(rebased) = rebase::rebase(&range, &spans) {
            let removed_rows = rope_newlines(&self.base, rebased.base_range.clone()) as i64;
            let row_delta = count_newlines(text) as i64 - removed_rows;
            let base_delta = text.len() as isize - rebased.base_range.len() as isize;
            rope_replace(&mut self.base, rebased.base_range, text);
            self.base_version += 1;
            for hunk in &mut self.hunks[before..] {
                shift_buffer(hunk, delta);
                shift_base(hunk, base_delta, row_delta);
            }
            // The neighbours may have lost their line alignment (a newline
            // right before or after them was deleted).
            for index in [before.checked_sub(1), Some(before)].into_iter().flatten() {
                if let Some(hunk) = self.hunks.get(index) {
                    touched.check.push(hunk.id);
                }
            }
            return;
        }

        let hit = rebase::touching(&range, &spans);
        if hit.is_empty() {
            // A change the base does not have, away from every hunk: a new
            // hunk right where it happened.
            let base_start = rebase::map_right(range.start, &spans);
            let base_end = if range.is_empty() {
                base_start
            } else {
                rebase::map_left(range.end, &spans)
            };
            for hunk in &mut self.hunks[before..] {
                shift_buffer(hunk, delta);
            }
            let id = next_id(ids);
            self.hunks.insert(
                before,
                stale_hunk(
                    id,
                    base_start..base_end.max(base_start),
                    range.start..range.start + text.len(),
                    true,
                ),
            );
            touched.dirty.push(id);
            self.check_neighbours(before, touched);
            return;
        }

        // The edit touches hunks: they merge into one that also covers the
        // edit, the base is left alone.
        let first = &spans[hit.start];
        let last = &spans[hit.end - 1];
        let buffer_start = range.start.min(first.buffer.start);
        let buffer_end = range.end.max(last.buffer.end);
        let base_start = if range.start < first.buffer.start {
            rebase::map_right(range.start, &spans)
        } else {
            first.base.start
        };
        let base_end = if range.end > last.buffer.end {
            rebase::map_left(range.end, &spans)
        } else {
            last.base.end
        };
        let id = self.hunks[hit.start].id;
        let pending = self.hunks[hit.clone()].iter().any(|h| h.pending);
        self.hunks.drain(hit.clone());
        for hunk in &mut self.hunks[hit.start..] {
            shift_buffer(hunk, delta);
        }
        let new_end = (buffer_end as isize + delta) as usize;
        self.hunks.insert(
            hit.start,
            stale_hunk(id, base_start..base_end, buffer_start..new_end, pending),
        );
        touched.dirty.push(id);
        self.check_neighbours(hit.start, touched);
    }

    /// Queues the hunks around `index` for an alignment check.
    fn check_neighbours(&self, index: usize, touched: &mut Touched) {
        for i in [index.checked_sub(1), index.checked_add(1)]
            .into_iter()
            .flatten()
        {
            if let Some(hunk) = self.hunks.get(i) {
                touched.check.push(hunk.id);
            }
        }
    }

    /// Repairs the invariants after the steps of one event, with the buffer
    /// snapshot already installed in `current`: aligns reshaped hunks to
    /// lines, drops the ones that became equal, re-diffs the rest locally.
    pub(crate) fn finish(&mut self, touched: Touched, ids: &mut u64) {
        if self.file_level_only() {
            self.whole_differs = self.base != *self.current.rope();
            return;
        }
        let mut work: Vec<(HunkId, bool)> = touched
            .dirty
            .into_iter()
            .map(|id| (id, true))
            .chain(touched.check.into_iter().map(|id| (id, false)))
            .collect();
        // Process in position order so row bookkeeping can lean on the
        // (already repaired) previous hunk.
        work.sort_by_key(|(id, _)| self.index_of(*id).unwrap_or(usize::MAX));
        let mut done: Vec<HunkId> = Vec::new();
        for (id, dirty) in work {
            if done.contains(&id) {
                continue;
            }
            let Some(index) = self.index_of(id) else {
                continue;
            };
            let (index, grown) = self.align(index);
            let id = self.hunks[index].id;
            done.push(id);
            if !(dirty || grown) {
                continue;
            }
            self.fix_rows(index);
            let hunk = &self.hunks[index];
            if self.base_text(hunk) == self.buffer_text(hunk) {
                self.hunks.remove(index);
                continue;
            }
            self.hunks[index].pending = true;
            self.refresh_local(index, ids);
        }
        self.clear_if_equal();
    }

    /// Hunks can cancel out across an equal segment (a line moved down by
    /// one); when the texts are equal as a whole nothing is pending. O(1)
    /// unless both texts have the same length.
    pub(crate) fn clear_if_equal(&mut self) {
        if !self.hunks.is_empty()
            && self.base.len_bytes() == self.current.len()
            && self.base == *self.current.rope()
        {
            self.hunks.clear();
        }
    }

    /// Grows hunk `index` to whole lines on both sides, merging with the
    /// neighbours it reaches. Returns its (possibly new) index and whether it
    /// changed.
    fn align(&mut self, mut index: usize) -> (usize, bool) {
        let mut grown = false;
        // Each round grows the hunk or merges a neighbour, so this bound is
        // never reached unless the segment invariant was already broken.
        let mut budget = 2 * self.hunks.len() + 2 * self.current.len() + 8;
        loop {
            if budget == 0 {
                tracing::error!(path = %self.path.display(), "review: hunk alignment did not converge");
                self.needs_recompute = true;
                return (index, grown);
            }
            budget -= 1;
            let hunk = &self.hunks[index];
            let buffer = hunk.buffer_byte_range();
            let base = hunk.base_byte_range.clone();
            // Hunks with no equal text between them are one change (an
            // insertion next to a deletion can cancel out).
            if index > 0 && self.hunks[index - 1].buffer_byte_range().end == buffer.start {
                self.merge_with_next(index - 1);
                index -= 1;
                grown = true;
                continue;
            }
            if self
                .hunks
                .get(index + 1)
                .is_some_and(|next| next.buffer_byte_range().start == buffer.end)
            {
                self.merge_with_next(index);
                grown = true;
                continue;
            }
            let rope = self.current.rope();
            if !(at_line_start(rope, buffer.start) && at_line_start(&self.base, base.start)) {
                let floor = index
                    .checked_sub(1)
                    .map_or(0, |i| self.hunks[i].buffer_byte_range().end);
                // If only the base side is misaligned, look past the newline
                // the buffer side already starts after.
                let ceiling = if at_line_start(rope, buffer.start) {
                    buffer.start.saturating_sub(1).max(floor)
                } else {
                    buffer.start
                };
                match prev_newline(rope, floor, ceiling) {
                    Some(newline) => {
                        let k = buffer.start - (newline + 1);
                        self.grow(index, k, 0);
                    }
                    None if index > 0 => {
                        self.merge_with_next(index - 1);
                        index -= 1;
                    }
                    None => {
                        let k = buffer.start;
                        self.grow(index, k, 0);
                    }
                }
                grown = true;
                continue;
            }
            if !(at_line_end(rope, buffer.end) && at_line_end(&self.base, base.end)) {
                let ceiling = self
                    .hunks
                    .get(index + 1)
                    .map_or(rope.len_bytes(), |h| h.buffer_byte_range().start);
                match next_newline(rope, buffer.end, ceiling) {
                    Some(newline) => {
                        let k = newline + 1 - buffer.end;
                        self.grow(index, 0, k);
                    }
                    None if index + 1 < self.hunks.len() => {
                        self.merge_with_next(index);
                    }
                    None => {
                        let k = rope.len_bytes() - buffer.end;
                        self.grow(index, 0, k);
                    }
                }
                grown = true;
                continue;
            }
            return (index, grown);
        }
    }

    /// Extends hunk `index` by `left` bytes before and `right` bytes after,
    /// on both sides (the bytes come from equal segments).
    fn grow(&mut self, index: usize, left: usize, right: usize) {
        let hunk = &mut self.hunks[index];
        hunk.buffer_range.start.offset -= left;
        hunk.buffer_range.end.offset += right;
        hunk.base_byte_range.start -= left;
        hunk.base_byte_range.end += right;
        make_stale(hunk);
    }

    /// Merges hunk `index` with hunk `index + 1` (and the equal segment
    /// between them).
    fn merge_with_next(&mut self, index: usize) {
        let next = self.hunks.remove(index + 1);
        let hunk = &mut self.hunks[index];
        hunk.buffer_range.end = next.buffer_range.end;
        hunk.base_byte_range.end = next.base_byte_range.end;
        hunk.pending |= next.pending;
        make_stale(hunk);
    }

    /// Recomputes `base_rows` of hunk `index` from its byte range, relying on
    /// the previous hunk's rows.
    pub(crate) fn fix_rows(&mut self, index: usize) {
        let base = self.hunks[index].base_byte_range.clone();
        let start = match index.checked_sub(1).map(|i| &self.hunks[i]) {
            Some(prev) => {
                prev.base_rows.end
                    + rope_newlines(&self.base, prev.base_byte_range.end..base.start) as u32
            }
            None => rope_newlines(&self.base, 0..base.start) as u32,
        };
        let lines = diff::split_lines(&rope_text(&self.base, base)).len() as u32;
        self.hunks[index].base_rows = start..start + lines;
    }

    /// Re-diffs hunk `index` on its own (both sides are whole lines) and
    /// replaces it with the resulting fresh hunks. Big regions are left for
    /// the background recompute.
    pub(crate) fn refresh_local(&mut self, index: usize, ids: &mut u64) {
        let hunk = &self.hunks[index];
        let base_text = self.base_text(hunk);
        let buffer_text = self.buffer_text(hunk);
        if base_text.len() + buffer_text.len() > LOCAL_REFRESH_LIMIT {
            self.needs_recompute = true;
            return;
        }
        let origin = Origin {
            base_byte: hunk.base_byte_range.start,
            base_row: hunk.base_rows.start,
            buffer_byte: hunk.buffer_range.start.offset,
            buffer_row: 0,
        };
        let id = hunk.id;
        let pending = hunk.pending;
        let raw =
            diff::diff_region(&base_text, &buffer_text, origin, &|| false).unwrap_or_default();
        let fresh: Vec<Hunk> = raw
            .iter()
            .enumerate()
            .map(|(k, raw)| {
                let id = if k == 0 { id } else { next_id(ids) };
                let mut hunk = hunk_from_raw(id, raw);
                hunk.pending = pending;
                hunk
            })
            .collect();
        self.hunks.splice(index..index + 1, fresh);
    }

    /// Installs the result of a full recompute, keeping the ids of hunks that
    /// overlap their predecessors.
    pub(crate) fn install(&mut self, raw: &[RawHunk], ids: &mut u64) {
        let old = std::mem::take(&mut self.hunks);
        let mut used = vec![false; old.len()];
        let mut cursor = 0;
        let mut hunks = Vec::with_capacity(raw.len());
        for raw in raw {
            let mut matched = None;
            let mut k = cursor;
            while k < old.len() {
                let range = old[k].buffer_byte_range();
                if range.start > raw.buffer_bytes.end {
                    break;
                }
                if !used[k] && ranges_meet(&range, &raw.buffer_bytes) {
                    matched = Some(k);
                    break;
                }
                k += 1;
            }
            let (id, pending) = match matched {
                Some(k) => {
                    used[k] = true;
                    cursor = k + 1;
                    // A reject in flight stays hidden if it is still the
                    // very same change.
                    let same = old[k].buffer_byte_range() == raw.buffer_bytes
                        && old[k].base_byte_range == raw.base_bytes;
                    (old[k].id, old[k].pending || !same)
                }
                None => (next_id(ids), true),
            };
            let mut hunk = hunk_from_raw(id, raw);
            hunk.pending = pending;
            hunks.push(hunk);
        }
        self.hunks = hunks;
    }

    /// Replaces the base region of hunk `index` with `text`, shifting the
    /// hunks after it.
    pub(crate) fn replace_base_region(&mut self, index: usize, text: &str) {
        let range = self.hunks[index].base_byte_range.clone();
        let removed_rows = rope_newlines(&self.base, range.clone()) as i64;
        let row_delta = count_newlines(text) as i64 - removed_rows;
        let delta = text.len() as isize - range.len() as isize;
        rope_replace(&mut self.base, range.clone(), text);
        self.base_version += 1;
        let hunk = &mut self.hunks[index];
        hunk.base_byte_range = range.start..range.start + text.len();
        for hunk in &mut self.hunks[index + 1..] {
            shift_base(hunk, delta, row_delta);
        }
        self.fix_rows(index);
    }

    // ------------------------------------------------------------ line ops

    /// Text of each line pair side, in order: `(base line, buffer line)`.
    pub(crate) fn pair_texts(&self, index: usize) -> Vec<(Option<String>, Option<String>)> {
        let hunk = &self.hunks[index];
        let base_text = self.base_text(hunk);
        let buffer_text = self.buffer_text(hunk);
        let base_lines = diff::split_lines(&base_text);
        let buffer_lines = diff::split_lines(&buffer_text);
        let buffer_start = hunk.buffer_range.start.offset;
        hunk.lines
            .iter()
            .map(|pair| {
                let base = pair.base_row.and_then(|row| {
                    let k = row.checked_sub(hunk.base_rows.start)? as usize;
                    base_lines.get(k).map(|r| base_text[r.clone()].to_owned())
                });
                let buffer = pair.buffer_row.and_then(|anchor| {
                    let rel = anchor.offset.checked_sub(buffer_start)?;
                    buffer_lines
                        .iter()
                        .find(|r| r.start == rel)
                        .map(|r| buffer_text[r.clone()].to_owned())
                });
                (base, buffer)
            })
            .collect()
    }

    /// Finds the pending, fresh hunk holding `pair`.
    pub(crate) fn find_pair(&self, pair: &LinePair) -> Option<(usize, usize)> {
        self.hunks.iter().enumerate().find_map(|(index, hunk)| {
            if !hunk.pending || !hunk.fresh {
                return None;
            }
            hunk.lines
                .iter()
                .position(|p| p == pair)
                .map(|line| (index, line))
        })
    }

    /// Whether any hunk is waiting for a recompute to get its lines.
    pub(crate) fn has_stale_hunks(&self) -> bool {
        self.hunks.iter().any(|h| h.pending && !h.fresh)
    }
}

/// Whether two buffer ranges overlap, or touch where one of them is empty.
fn ranges_meet(a: &Range<usize>, b: &Range<usize>) -> bool {
    if a.is_empty() || b.is_empty() {
        a.start <= b.end && b.start <= a.end
    } else {
        a.start < b.end && b.start < a.end
    }
}

pub(crate) fn next_id(ids: &mut u64) -> HunkId {
    *ids += 1;
    HunkId(*ids)
}

fn stale_hunk(id: HunkId, base: Range<usize>, buffer: Range<usize>, pending: bool) -> Hunk {
    let mut hunk = Hunk {
        id,
        base_rows: 0..0,
        base_byte_range: base,
        buffer_range: Anchor::before(buffer.start)..Anchor::before(buffer.end),
        kind: HunkKind::Modified,
        word_diffs: None,
        lines: Vec::new(),
        pending,
        fresh: false,
    };
    make_stale(&mut hunk);
    hunk
}

fn make_stale(hunk: &mut Hunk) {
    hunk.fresh = false;
    hunk.lines.clear();
    hunk.word_diffs = None;
    hunk.kind = match (
        hunk.base_byte_range.is_empty(),
        hunk.buffer_range.start == hunk.buffer_range.end,
    ) {
        (true, _) => HunkKind::Added,
        (_, true) => HunkKind::Deleted,
        _ => HunkKind::Modified,
    };
}

pub(crate) fn hunk_from_raw(id: HunkId, raw: &RawHunk) -> Hunk {
    Hunk {
        id,
        base_rows: raw.base_rows.clone(),
        base_byte_range: raw.base_bytes.clone(),
        buffer_range: Anchor::before(raw.buffer_bytes.start)..Anchor::before(raw.buffer_bytes.end),
        kind: raw.kind,
        word_diffs: raw.word_diffs.clone(),
        lines: raw
            .lines
            .iter()
            .map(|pair| LinePair {
                base_row: pair.base_row,
                buffer_row: pair.buffer_offset.map(Anchor::before),
            })
            .collect(),
        pending: true,
        fresh: true,
    }
}

fn shift_buffer(hunk: &mut Hunk, delta: isize) {
    if delta == 0 {
        return;
    }
    let shift = |offset: &mut usize| *offset = (*offset as isize + delta) as usize;
    shift(&mut hunk.buffer_range.start.offset);
    shift(&mut hunk.buffer_range.end.offset);
    for pair in &mut hunk.lines {
        if let Some(anchor) = pair.buffer_row.as_mut() {
            shift(&mut anchor.offset);
        }
    }
}

fn shift_base(hunk: &mut Hunk, delta: isize, row_delta: i64) {
    let shift = |offset: usize| (offset as isize + delta) as usize;
    hunk.base_byte_range = shift(hunk.base_byte_range.start)..shift(hunk.base_byte_range.end);
    if row_delta != 0 {
        let rows = |row: u32| (row as i64 + row_delta) as u32;
        hunk.base_rows = rows(hunk.base_rows.start)..rows(hunk.base_rows.end);
        for pair in &mut hunk.lines {
            if let Some(row) = pair.base_row.as_mut() {
                *row = rows(*row);
            }
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
