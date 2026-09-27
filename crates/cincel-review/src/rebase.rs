//! Rebasing user edits onto the base (Zed's `apply_non_conflicting_edits`).
//!
//! The store keeps an exact invariant between the buffer and the base: both
//! decompose into the same *equal segments* separated by hunks,
//!
//! ```text
//! buffer = E0 H1 E1 H2 E2 …      base = E0 B1 E1 B2 E2 …
//! ```
//!
//! so a position inside an equal segment of the buffer maps to exactly one
//! position of the base. A user edit that stays inside one equal segment is
//! replayed on the base at the mapped position ([`Rebased`]) and never shows
//! up as an agent change. An edit that touches a hunk is a [`Conflict`]: the
//! base is left alone and the hunk grows to include the user's text.
//!
//! "Touches" is decided on bytes:
//!
//! - a non-empty edit conflicts with a hunk when the ranges overlap
//!   (`edit.start < hunk.end && edit.end > hunk.start`); for a pure deletion
//!   hunk (empty buffer range at `p`) that means the edit strictly contains
//!   `p`;
//! - an insertion at `p` conflicts with a non-empty hunk when
//!   `hunk.start <= p < hunk.end`: typing at the start of a hunk line is
//!   typing inside the hunk, typing at the start of the next line is not.
//!
//! The function is pure and works on plain spans, so it can later be replaced
//! by real operational transformation (VS Code's `tryRebase`) without
//! touching the store.

use std::ops::Range;

/// A hunk seen by the rebase: its region in the buffer and in the base, in
/// the current coordinates of each. Spans must be sorted by buffer position
/// and disjoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HunkSpan {
    /// Region in the buffer.
    pub buffer: Range<usize>,
    /// Region in the base.
    pub base: Range<usize>,
}

/// The edit to replay on the base.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rebased {
    /// Base range to replace.
    pub base_range: Range<usize>,
}

/// The edit touches pending hunks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// Indices (into the span slice) of the hunks the edit touches.
    pub hunks: Range<usize>,
}

/// Rebases a user edit of `range` (buffer coordinates, before the edit) onto
/// the base.
pub fn rebase(range: &Range<usize>, hunks: &[HunkSpan]) -> Result<Rebased, Conflict> {
    let touched = touching(range, hunks);
    if !touched.is_empty() {
        return Err(Conflict { hunks: touched });
    }
    let start = map_right(range.start, hunks);
    let end = if range.is_empty() {
        start
    } else {
        map_left(range.end, hunks)
    };
    Ok(Rebased {
        base_range: start..end.max(start),
    })
}

/// Indices of the hunks `range` touches (contiguous because spans are sorted
/// and disjoint).
pub fn touching(range: &Range<usize>, hunks: &[HunkSpan]) -> Range<usize> {
    let first =
        hunks.partition_point(|h| !touches(range, &h.buffer) && h.buffer.end <= range.start);
    let mut last = first;
    while last < hunks.len() && touches(range, &hunks[last].buffer) {
        last += 1;
    }
    first..last
}

/// Whether an edit of `range` touches a hunk occupying `hunk`.
pub fn touches(range: &Range<usize>, hunk: &Range<usize>) -> bool {
    if range.is_empty() {
        hunk.start <= range.start && range.start < hunk.end
    } else {
        range.start < hunk.end && range.end > hunk.start
    }
}

/// Maps a buffer position that starts an edit to the base. A pure deletion
/// hunk sitting exactly at `offset` counts as *before* it (text inserted
/// there lands after the deleted block, like the hunk's `Before` anchors).
pub fn map_right(offset: usize, hunks: &[HunkSpan]) -> usize {
    let before = hunks.partition_point(|h| h.buffer.end <= offset);
    map_after(offset, before, hunks)
}

/// Maps a buffer position that ends an edit to the base. A pure deletion
/// hunk sitting exactly at `offset` counts as *after* it.
pub fn map_left(offset: usize, hunks: &[HunkSpan]) -> usize {
    let before = hunks.partition_point(|h| {
        h.buffer.end < offset || (h.buffer.end == offset && !h.buffer.is_empty())
    });
    map_after(offset, before, hunks)
}

/// Maps `offset`, knowing the first `before` hunks lie before it.
fn map_after(offset: usize, before: usize, hunks: &[HunkSpan]) -> usize {
    match before.checked_sub(1).map(|i| &hunks[i]) {
        Some(h) => h.base.end + (offset - h.buffer.end),
        None => offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(buffer: Range<usize>, base: Range<usize>) -> HunkSpan {
        HunkSpan { buffer, base }
    }

    #[test]
    fn outside_edits_map_through_hunks() {
        // buffer: "aa" H(4) "bb"; base: "aa" B(2) "bb".
        let hunks = [span(2..6, 2..4)];
        assert_eq!(rebase(&(0..1), &hunks).unwrap().base_range, 0..1);
        assert_eq!(rebase(&(6..8), &hunks).unwrap().base_range, 4..6);
        assert_eq!(rebase(&(6..6), &hunks).unwrap().base_range, 4..4);
        // Ends exactly at the hunk start: no conflict.
        assert_eq!(rebase(&(1..2), &hunks).unwrap().base_range, 1..2);
    }

    #[test]
    fn touching_edits_conflict() {
        let hunks = [span(2..6, 2..4), span(8..9, 6..6)];
        assert_eq!(rebase(&(2..2), &hunks).unwrap_err().hunks, 0..1);
        assert_eq!(rebase(&(5..7), &hunks).unwrap_err().hunks, 0..1);
        assert_eq!(rebase(&(1..9), &hunks).unwrap_err().hunks, 0..2);
        assert!(rebase(&(6..6), &hunks).is_ok());
    }

    #[test]
    fn deletion_hunks_have_a_side() {
        // Pure deletion at buffer offset 2; base block 2..5.
        let hunks = [span(2..2, 2..5)];
        // Insertion at the block lands after it.
        assert_eq!(rebase(&(2..2), &hunks).unwrap().base_range, 5..5);
        // Deleting up to the block stops before it.
        assert_eq!(rebase(&(0..2), &hunks).unwrap().base_range, 0..2);
        // Deleting from the block starts after it.
        assert_eq!(rebase(&(2..4), &hunks).unwrap().base_range, 5..7);
        // Spanning it conflicts.
        assert!(rebase(&(1..3), &hunks).is_err());
    }
}
