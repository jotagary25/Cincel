//! Background recompute: a `Send` job with everything it needs, and the pure
//! function it runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cincel_text::{BufferSnapshot, Rope};

use crate::diff::{self, Origin, RawHunk};
use crate::text::{rope_is_binary, rope_is_too_large};

/// Output of [`compute_diff`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffData {
    /// Hunks (empty for file-level files).
    pub hunks: Vec<RawHunk>,
    /// Over the inline limits.
    pub too_large: bool,
    /// Looks binary.
    pub binary: bool,
    /// `(added, removed)` lines of the whole diff (`(0, 0)` for binary).
    pub stats: (u32, u32),
}

/// Diffs a base against a buffer text. Pure; returns `None` if `cancelled`
/// turns true while it runs.
pub fn compute_diff(base: &Rope, buffer: &Rope, cancelled: &dyn Fn() -> bool) -> Option<DiffData> {
    let binary = rope_is_binary(base) || rope_is_binary(buffer);
    let too_large = rope_is_too_large(base) || rope_is_too_large(buffer);
    let base_text = base.to_string();
    let buffer_text = buffer.to_string();
    if cancelled() {
        return None;
    }
    if binary {
        return Some(DiffData {
            binary,
            too_large,
            ..DiffData::default()
        });
    }
    if too_large {
        let stats = diff::line_stats(&base_text, &buffer_text);
        return Some(DiffData {
            too_large,
            stats,
            ..DiffData::default()
        });
    }
    let hunks = diff::diff_region(&base_text, &buffer_text, Origin::default(), cancelled)?;
    let stats = hunks.iter().fold((0, 0), |(added, removed), h| {
        (
            added + (h.buffer_rows.end - h.buffer_rows.start),
            removed + (h.base_rows.end - h.base_rows.start),
        )
    });
    Some(DiffData {
        hunks,
        too_large,
        binary,
        stats,
    })
}

/// A recompute to run off the main thread.
///
/// Built by [`ReviewStore::recompute_job`](crate::ReviewStore::recompute_job)
/// from the store's current base and buffer snapshot. `Send` and cheap to
/// build (two rope clones). [`RecomputeJob::run`] stops early when the file
/// changed since the job was built, and
/// [`ReviewStore::apply_recompute`](crate::ReviewStore::apply_recompute)
/// discards results for an outdated state.
#[derive(Clone, Debug)]
pub struct RecomputeJob {
    pub(crate) path: PathBuf,
    pub(crate) base: Rope,
    pub(crate) buffer: BufferSnapshot,
    pub(crate) generation: u64,
    pub(crate) token: Arc<AtomicU64>,
}

/// What a [`RecomputeJob`] produced.
#[derive(Clone, Debug)]
pub struct RecomputeResult {
    pub(crate) path: PathBuf,
    pub(crate) generation: u64,
    /// `None` when the job was cancelled.
    pub data: Option<DiffData>,
}

impl RecomputeResult {
    /// File the result is for.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl RecomputeJob {
    /// File the job is for.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the file has not changed since the job was built.
    pub fn is_current(&self) -> bool {
        self.token.load(Ordering::SeqCst) == self.generation
    }

    /// Runs the diff. Synchronous and pure apart from reading the
    /// cancellation counter.
    pub fn run(&self) -> RecomputeResult {
        let cancelled = || !self.is_current();
        RecomputeResult {
            path: self.path.clone(),
            generation: self.generation,
            data: compute_diff(&self.base, self.buffer.rope(), &cancelled),
        }
    }
}
