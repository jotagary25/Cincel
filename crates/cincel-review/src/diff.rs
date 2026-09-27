//! The pure diff: base text against buffer text, no store, no anchors.
//!
//! Everything here is a plain function of two strings, so it runs on a
//! background executor from a [`RecomputeJob`](crate::RecomputeJob) and is
//! trivially testable.
//!
//! - Lines: [`imara_diff`] with [`Algorithm::Histogram`] followed by
//!   [`Diff::postprocess_lines`] (hunk merging plus git's indentation/slider
//!   heuristic; the workspace's `imara-diff 0.2` has it, no fork needed).
//!   Tokens keep their `\n`, so whitespace and a missing final newline are
//!   real changes (nothing is ignored) and a newline added or removed at EOF
//!   is a one-line hunk.
//! - Words: [`similar`] at Unicode word level, only when both sides have at
//!   most [`MAX_WORD_DIFF_LINE_COUNT`] lines.
//! - Lines of a hunk are paired through the equal words of the word diff
//!   (weighted monotone alignment); leftovers inside each gap, and hunks
//!   without word diff, pair by index; the rest are one-sided pairs.

use std::ops::Range;

use imara_diff::{Algorithm, Diff, InternedInput};

use crate::types::{HunkKind, WordDiffs};

/// Word diffs are only computed when both sides have at most this many lines
/// (the same limit Zed uses).
pub const MAX_WORD_DIFF_LINE_COUNT: usize = 5;
/// Files larger than this are reviewed as a whole (no inline hunks).
pub const MAX_INLINE_BYTES: usize = 2 * 1024 * 1024;
/// Files with more lines than this are reviewed as a whole.
pub const MAX_INLINE_LINES: usize = 50_000;
/// A NUL byte in this many leading bytes marks the file as binary.
pub const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// A line pair before it is anchored into a live buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawLinePair {
    /// Absolute base row.
    pub base_row: Option<u32>,
    /// Absolute byte offset of the start of the buffer line.
    pub buffer_offset: Option<usize>,
}

/// A hunk in absolute byte/row coordinates of the two texts it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawHunk {
    /// Byte range in the base.
    pub base_bytes: Range<usize>,
    /// Row range in the base.
    pub base_rows: Range<u32>,
    /// Byte range in the buffer.
    pub buffer_bytes: Range<usize>,
    /// Row range in the buffer.
    pub buffer_rows: Range<u32>,
    /// Added, deleted or modified.
    pub kind: HunkKind,
    /// Word-level changes for short hunks.
    pub word_diffs: Option<WordDiffs>,
    /// Per-line decomposition.
    pub lines: Vec<RawLinePair>,
}

/// Whether `text` looks binary: a NUL in its first [`BINARY_SNIFF_BYTES`].
pub fn is_binary(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0)
}

/// Whether `text` is over the inline review limits
/// ([`MAX_INLINE_BYTES`] or [`MAX_INLINE_LINES`]).
pub fn is_too_large(text: &str) -> bool {
    if text.len() > MAX_INLINE_BYTES {
        return true;
    }
    // Cheap early exit: fewer bytes than the line limit means fewer lines.
    if text.len() < MAX_INLINE_LINES {
        return false;
    }
    text.bytes().filter(|&b| b == b'\n').count() + 1 > MAX_INLINE_LINES
}

/// Line counts `(added, removed)` of a whole-text diff, without building
/// hunks. Used for files over the inline limits.
pub fn line_stats(base: &str, buffer: &str) -> (u32, u32) {
    let input = InternedInput::new(base, buffer);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    (diff.count_additions(), diff.count_removals())
}

/// Hunks between two whole texts.
pub fn diff_texts(base: &str, buffer: &str) -> Vec<RawHunk> {
    diff_region(base, buffer, Origin::default(), &|| false).unwrap_or_default()
}

/// Where a region of the texts starts, so hunks of a sub-diff come out in
/// absolute coordinates.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Origin {
    pub base_byte: usize,
    pub base_row: u32,
    pub buffer_byte: usize,
    pub buffer_row: u32,
}

/// Hunks between `base` and `buffer`, shifted by `origin`. Returns `None` when
/// `cancelled` says so between hunks.
pub(crate) fn diff_region(
    base: &str,
    buffer: &str,
    origin: Origin,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<RawHunk>> {
    if base == buffer {
        return Some(Vec::new());
    }
    let input = InternedInput::new(base, buffer);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    if cancelled() {
        return None;
    }

    let base_lines = line_starts(base);
    let buffer_lines = line_starts(buffer);
    let mut hunks = Vec::new();
    for (index, hunk) in diff.hunks().enumerate() {
        if index % 64 == 63 && cancelled() {
            return None;
        }
        let base_rows = hunk.before.clone();
        let buffer_rows = hunk.after.clone();
        let base_bytes = rows_to_bytes(&base_lines, base_rows.clone());
        let buffer_bytes = rows_to_bytes(&buffer_lines, buffer_rows.clone());
        let base_text = &base[base_bytes.clone()];
        let buffer_text = &buffer[buffer_bytes.clone()];
        hunks.push(build_hunk(
            base_text,
            buffer_text,
            Origin {
                base_byte: origin.base_byte + base_bytes.start,
                base_row: origin.base_row + base_rows.start,
                buffer_byte: origin.buffer_byte + buffer_bytes.start,
                buffer_row: origin.buffer_row + buffer_rows.start,
            },
        ));
    }
    Some(hunks)
}

/// Builds one hunk from the two texts it covers (whole lines each).
pub(crate) fn build_hunk(base_text: &str, buffer_text: &str, at: Origin) -> RawHunk {
    let base_lines = split_lines(base_text);
    let buffer_lines = split_lines(buffer_text);
    let kind = match (base_lines.is_empty(), buffer_lines.is_empty()) {
        (true, _) => HunkKind::Added,
        (_, true) => HunkKind::Deleted,
        _ => HunkKind::Modified,
    };
    let word = if kind == HunkKind::Modified
        && base_lines.len() <= MAX_WORD_DIFF_LINE_COUNT
        && buffer_lines.len() <= MAX_WORD_DIFF_LINE_COUNT
    {
        Some(word_diff(base_text, buffer_text))
    } else {
        None
    };
    let pairs = pair_lines(
        base_text,
        buffer_text,
        &base_lines,
        &buffer_lines,
        word.as_ref().map(|(_, equal)| equal.as_slice()),
    );
    let lines = pairs
        .into_iter()
        .map(|(b, c)| RawLinePair {
            base_row: b.map(|b| at.base_row + b as u32),
            buffer_offset: c.map(|c| at.buffer_byte + buffer_lines[c].start),
        })
        .collect();
    RawHunk {
        base_bytes: at.base_byte..at.base_byte + base_text.len(),
        base_rows: at.base_row..at.base_row + base_lines.len() as u32,
        buffer_bytes: at.buffer_byte..at.buffer_byte + buffer_text.len(),
        buffer_rows: at.buffer_row..at.buffer_row + buffer_lines.len() as u32,
        kind,
        word_diffs: word.map(|(diffs, _)| diffs),
        lines,
    }
}

/// Byte ranges of each line of `text` (terminator included).
pub fn split_lines(text: &str) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            lines.push(start..index + 1);
            start = index + 1;
        }
    }
    if start < text.len() {
        lines.push(start..text.len());
    }
    lines
}

/// Start offset of every line as `imara-diff` counts them (no trailing empty
/// line), plus the text length.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    if *starts.last().expect("non-empty") != text.len() {
        starts.push(text.len());
    }
    starts
}

fn rows_to_bytes(starts: &[usize], rows: Range<u32>) -> Range<usize> {
    let last = *starts.last().unwrap_or(&0);
    let start = starts.get(rows.start as usize).copied().unwrap_or(last);
    let end = starts.get(rows.end as usize).copied().unwrap_or(last);
    start..end.max(start)
}

/// An equal word shared by both sides: byte range in base, byte range in
/// buffer (both relative to the hunk).
type EqualSpan = (Range<usize>, Range<usize>);

/// Word diff of a short hunk: changed ranges plus the equal spans used to
/// pair lines.
fn word_diff(base: &str, buffer: &str) -> (WordDiffs, Vec<EqualSpan>) {
    let diff = similar::TextDiff::from_unicode_words(base, buffer);
    let base_offsets = token_offsets(diff.iter_old_slices());
    let buffer_offsets = token_offsets(diff.iter_new_slices());
    let mut diffs = WordDiffs::default();
    let mut equal = Vec::new();
    for op in diff.ops() {
        let old = op.old_range();
        let new = op.new_range();
        let old_bytes = base_offsets[old.start]..base_offsets[old.end];
        let new_bytes = buffer_offsets[new.start]..buffer_offsets[new.end];
        match op.tag() {
            similar::DiffTag::Equal => equal.push((old_bytes, new_bytes)),
            _ => {
                push_merged(&mut diffs.base, old_bytes);
                push_merged(&mut diffs.buffer, new_bytes);
            }
        }
    }
    (diffs, equal)
}

fn push_merged(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
    if range.is_empty() {
        return;
    }
    match ranges.last_mut() {
        Some(last) if last.end == range.start => last.end = range.end,
        _ => ranges.push(range),
    }
}

fn token_offsets<'a>(tokens: impl Iterator<Item = &'a str>) -> Vec<usize> {
    let mut offsets = vec![0];
    let mut at = 0;
    for token in tokens {
        at += token.len();
        offsets.push(at);
    }
    offsets
}

/// Pairs the lines of a hunk. Returns `(base line, buffer line)` indices in
/// display order.
///
/// Two rules keep every per-line decision meaningful (it always changes
/// something):
///
/// - a last line without `\n` (end of file) can only be last, so it pairs
///   with the other side's last line and the pair goes last;
/// - identical lines are never paired (deciding such a pair would be a
///   no-op); they become two one-sided pairs.
fn pair_lines(
    base_text: &str,
    buffer_text: &str,
    base: &[Range<usize>],
    buffer: &[Range<usize>],
    equal: Option<&[EqualSpan]>,
) -> Vec<(Option<usize>, Option<usize>)> {
    let (n, m) = (base.len(), buffer.len());
    if n == 0 || m == 0 {
        return (0..n)
            .map(|b| (Some(b), None))
            .chain((0..m).map(|c| (None, Some(c))))
            .collect();
    }

    // Anchor pairs from the equal words, if we have a word diff.
    let mut anchors: Vec<(usize, usize)> = match equal {
        Some(equal) => align_by_words(base, buffer, equal),
        None => Vec::new(),
    };
    let open_end = !base_text.ends_with('\n') || !buffer_text.ends_with('\n');
    if open_end {
        anchors.retain(|&(b, c)| b + 1 < n && c + 1 < m);
        anchors.push((n - 1, m - 1));
    }

    // Walk the gaps between anchors; inside a gap, pair by index and leave
    // the surplus one-sided (base lines first, like a unified diff).
    let mut out = Vec::with_capacity(n.max(m));
    let (mut b, mut c) = (0, 0);
    for &(ab, ac) in anchors.iter().chain(std::iter::once(&(n, m))) {
        let common = (ab - b).min(ac - c);
        for k in 0..common {
            out.push((Some(b + k), Some(c + k)));
        }
        for k in b + common..ab {
            out.push((Some(k), None));
        }
        for k in c + common..ac {
            out.push((None, Some(k)));
        }
        if ab < n && ac < m {
            out.push((Some(ab), Some(ac)));
        }
        b = ab + 1;
        c = ac + 1;
    }
    let mut split = Vec::with_capacity(out.len());
    for pair in out {
        match pair {
            (Some(b), Some(c)) if base_text[base[b].clone()] == buffer_text[buffer[c].clone()] => {
                split.push((Some(b), None));
                split.push((None, Some(c)));
            }
            pair => split.push(pair),
        }
    }
    split
}

/// Weighted monotone alignment of lines sharing equal (non-blank) words.
fn align_by_words(
    base: &[Range<usize>],
    buffer: &[Range<usize>],
    equal: &[EqualSpan],
) -> Vec<(usize, usize)> {
    let (n, m) = (base.len(), buffer.len());
    let mut weight = vec![vec![0usize; m]; n];
    let line_of = |lines: &[Range<usize>], offset: usize| {
        lines
            .iter()
            .position(|line| offset < line.end)
            .unwrap_or(lines.len() - 1)
    };
    for (old, new) in equal {
        // Walk the span character by character so a span crossing a newline
        // credits each pair of lines it touches; blank characters carry no
        // weight (they match everywhere).
        let len = old.len().min(new.len());
        let mut k = 0;
        while k < len {
            let b = line_of(base, old.start + k);
            let c = line_of(buffer, new.start + k);
            let step_b = base[b].end - (old.start + k);
            let step_c = buffer[c].end - (new.start + k);
            let step = step_b.min(step_c).min(len - k).max(1);
            weight[b][c] += step;
            k += step;
        }
    }
    // Discount pure whitespace: a pair must share at least one visible
    // character to count. Weight is a byte count, so a lone "\n" match is 1;
    // require more than that.
    for row in &mut weight {
        for w in row.iter_mut() {
            if *w <= 1 {
                *w = 0;
            }
        }
    }
    // Classic DP for the maximum weight common subsequence.
    let mut best = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let take = if weight[i][j] > 0 {
                weight[i][j] + best[i + 1][j + 1]
            } else {
                0
            };
            best[i][j] = take.max(best[i + 1][j]).max(best[i][j + 1]);
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if weight[i][j] > 0 && best[i][j] == weight[i][j] + best[i + 1][j + 1] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if best[i][j] == best[i + 1][j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newline_at_eof_is_one_line() {
        let hunks = diff_texts("a\nb\nc", "a\nb\nc\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].base_rows, 2..3);
        assert_eq!(hunks[0].buffer_rows, 2..3);
        assert_eq!(hunks[0].kind, HunkKind::Modified);
    }

    #[test]
    fn whitespace_is_a_change() {
        let hunks = diff_texts("a b\n", "a  b\n");
        assert_eq!(hunks.len(), 1);
        let words = hunks[0].word_diffs.as_ref().unwrap();
        assert_eq!(words.buffer.len(), 1);
    }

    #[test]
    fn pairs_follow_words() {
        // Base line 0 matches buffer line 1.
        let hunks = diff_texts("x\nfoo bar baz\ny\n", "x\nnew line\nfoo bar qux\ny\n");
        assert_eq!(hunks.len(), 1);
        let lines = &hunks[0].lines;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].base_row, None);
        assert_eq!(lines[1].base_row, Some(1));
        assert_eq!(lines[1].buffer_offset, Some(11));
    }

    #[test]
    fn index_pairing_for_long_hunks() {
        let base = "1\n2\n3\n4\n5\n6\n";
        let buffer = "a\nb\nc\nd\ne\nf\ng\n";
        let hunks = diff_texts(base, buffer);
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].word_diffs.is_none());
        let lines = &hunks[0].lines;
        assert_eq!(lines.len(), 7);
        assert!(lines[..6].iter().all(|p| p.base_row.is_some()));
        assert_eq!(lines[6].base_row, None);
    }

    #[test]
    fn binary_and_size_limits() {
        assert!(is_binary("ab\0c"));
        assert!(!is_binary("abc"));
        let mut late = "x".repeat(BINARY_SNIFF_BYTES);
        late.push('\0');
        assert!(!is_binary(&late));
        assert!(is_too_large(&"a\n".repeat(MAX_INLINE_LINES + 1)));
        assert!(!is_too_large(&"a\n".repeat(MAX_INLINE_LINES - 1)));
        assert!(is_too_large(&"a".repeat(MAX_INLINE_BYTES + 1)));
    }
}
