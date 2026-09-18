//! Minimal edits between two texts.
//!
//! When an agent writes a file it sends the **whole** new content. Replacing
//! the buffer wholesale would destroy anchors, selections, the undo history
//! and every review hunk, so the write is turned into the smallest set of
//! edits that produces the same text.
//!
//! Two passes, the same shape `asteroid-review` will use for hunks:
//!
//! 1. a line diff with [`imara_diff`]'s histogram algorithm plus its hunk
//!    post-processing (the indentation/slider heuristic git uses), which is
//!    what decides *where* a change is;
//! 2. a character refinement inside each hunk with [`similar`], so changing
//!    one identifier in a line produces one small edit instead of replacing
//!    the line.
//!
//! The result is a list of non-overlapping [`Edit`]s in ascending byte order.
//! Apply them **from the end backwards** so earlier offsets stay valid, which
//! is what [`apply`] does.

use std::ops::Range;

use imara_diff::{Algorithm, Diff, InternedInput};

/// A replacement of a byte range of the old text by new text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    /// Byte range in the *old* text.
    pub range: Range<usize>,
    /// What goes in its place.
    pub new_text: String,
}

/// Refinement is skipped for hunks larger than this, where a character diff
/// costs more than it saves.
const REFINE_LIMIT: usize = 4096;

/// Two character-level changes separated by fewer than this many equal
/// characters are merged into one edit.
///
/// Without it, `"hola"` to `"chau"` becomes three edits (the `h` and the `a`
/// happen to match) instead of one replacement, which is both noisier to
/// apply and unreadable as a word diff.
const MIN_EQUAL_RUN: usize = 3;

/// The smallest set of edits that turns `before` into `after`.
///
/// Returns an empty list when the texts are equal.
pub fn minimal_edits(before: &str, after: &str) -> Vec<Edit> {
    if before == after {
        return Vec::new();
    }
    if before.is_empty() || after.is_empty() {
        return vec![Edit {
            range: 0..before.len(),
            new_text: after.to_owned(),
        }];
    }

    let input = InternedInput::new(before, after);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);

    let before_lines = line_offsets(before);
    let after_lines = line_offsets(after);

    let mut edits = Vec::new();
    for hunk in diff.hunks() {
        let old = byte_range(
            &before_lines,
            hunk.before.start as usize,
            hunk.before.end as usize,
        );
        let new = byte_range(
            &after_lines,
            hunk.after.start as usize,
            hunk.after.end as usize,
        );
        refine(before, after, old, new, &mut edits);
    }
    edits
}

/// Applies `edits` (as produced by [`minimal_edits`]) to `text`.
///
/// Mostly for tests and for callers with a plain `String`; a real buffer
/// applies them itself so they go through its transaction and event machinery.
pub fn apply(text: &str, edits: &[Edit]) -> String {
    let mut out = text.to_owned();
    for edit in edits.iter().rev() {
        out.replace_range(edit.range.clone(), &edit.new_text);
    }
    out
}

/// Splits one line-level hunk into character-level edits.
fn refine(before: &str, after: &str, old: Range<usize>, new: Range<usize>, edits: &mut Vec<Edit>) {
    let old_text = &before[old.clone()];
    let new_text = &after[new.clone()];

    // Trim the common prefix and suffix first: it is cheap, it is what makes
    // "one word changed in a long line" a one-word edit, and it shrinks what
    // the character diff has to look at.
    let prefix = common_prefix(old_text, new_text);
    let suffix = common_suffix(&old_text[prefix..], &new_text[prefix..]);
    let old_core = old.start + prefix..old.end - suffix;
    let new_core = new.start + prefix..new.end - suffix;

    if old_core.is_empty() && new_core.is_empty() {
        return;
    }
    let old_core_text = &before[old_core.clone()];
    let new_core_text = &after[new_core.clone()];

    if old_core_text.is_empty()
        || new_core_text.is_empty()
        || old_core_text.len() > REFINE_LIMIT
        || new_core_text.len() > REFINE_LIMIT
    {
        edits.push(Edit {
            range: old_core,
            new_text: new_core_text.to_owned(),
        });
        return;
    }

    // Character diff of what is left. Anything that is not `Equal` becomes an
    // edit; consecutive ones are merged by construction because `similar`
    // already reports maximal runs.
    let diff = similar::TextDiff::from_chars(old_core_text, new_core_text);
    let old_chars: Vec<usize> = char_offsets(old_core_text);
    let new_chars: Vec<usize> = char_offsets(new_core_text);
    let mut spans: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    for op in diff.ops() {
        if matches!(op.tag(), similar::DiffTag::Equal) {
            continue;
        }
        let old_range = op.old_range();
        let new_range = op.new_range();
        match spans.last_mut() {
            Some(last) if old_range.start - last.0.end < MIN_EQUAL_RUN => {
                last.0.end = old_range.end;
                last.1.end = new_range.end;
            }
            _ => spans.push((old_range, new_range)),
        }
    }
    let produced = !spans.is_empty();
    for (old_range, new_range) in spans {
        let range =
            old_core.start + old_chars[old_range.start]..old_core.start + old_chars[old_range.end];
        let text = &new_core_text[new_chars[new_range.start]..new_chars[new_range.end]];
        edits.push(Edit {
            range,
            new_text: text.to_owned(),
        });
    }
    if !produced {
        // Should not happen (the cores differ), but never silently drop a
        // change.
        edits.push(Edit {
            range: old_core,
            new_text: new_core_text.to_owned(),
        });
    }
}

/// Byte offset of the start of every line, plus the end of the text.
fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            offsets.push(index + 1);
        }
    }
    // `imara-diff` counts lines without a trailing empty one, so the last
    // entry is always the end of the text.
    if *offsets.last().expect("siempre hay un 0") != text.len() {
        offsets.push(text.len());
    }
    offsets
}

/// The byte range covered by lines `start..end`.
fn byte_range(offsets: &[usize], start: usize, end: usize) -> Range<usize> {
    let last = *offsets.last().unwrap_or(&0);
    let start = offsets.get(start).copied().unwrap_or(last);
    let end = offsets.get(end).copied().unwrap_or(last);
    start..end.max(start)
}

/// Byte offset of every character, plus the length.
fn char_offsets(text: &str) -> Vec<usize> {
    let mut offsets: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
    offsets.push(text.len());
    offsets
}

/// Length in bytes of the common prefix, on a `char` boundary.
fn common_prefix(a: &str, b: &str) -> usize {
    let mut length = 0;
    for (x, y) in a.chars().zip(b.chars()) {
        if x != y {
            break;
        }
        length += x.len_utf8();
    }
    length
}

/// Length in bytes of the common suffix, on a `char` boundary.
fn common_suffix(a: &str, b: &str) -> usize {
    let mut length = 0;
    for (x, y) in a.chars().rev().zip(b.chars().rev()) {
        if x != y || length + x.len_utf8() > a.len().min(b.len()) {
            break;
        }
        length += x.len_utf8();
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(before: &str, after: &str) -> Vec<Edit> {
        let edits = minimal_edits(before, after);
        assert_eq!(
            apply(before, &edits),
            after,
            "las ediciones no reproducen el texto"
        );
        // Non-overlapping and ascending.
        for pair in edits.windows(2) {
            assert!(
                pair[0].range.end <= pair[1].range.start,
                "ediciones superpuestas: {pair:?}"
            );
        }
        edits
    }

    #[test]
    fn equal_texts_produce_nothing() {
        assert!(minimal_edits("a\nb\n", "a\nb\n").is_empty());
        assert!(minimal_edits("", "").is_empty());
    }

    #[test]
    fn one_changed_word_is_one_small_edit() {
        let before = "fn suma(a: i32, b: i32) -> i32 {\n    a - b\n}\n";
        let after = "fn suma(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
        let edits = round_trip(before, after);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "+");
        assert_eq!(&before[edits[0].range.clone()], "-");
    }

    #[test]
    fn inserted_lines_do_not_touch_the_rest() {
        let before = "uno\ndos\ntres\n";
        let after = "uno\ndos\nnuevo\ntres\n";
        let edits = round_trip(before, after);
        assert_eq!(edits.len(), 1);
        assert!(edits[0].range.is_empty(), "una inserción no borra nada");
        assert_eq!(edits[0].new_text, "nuevo\n");
    }

    #[test]
    fn deleted_lines_are_a_single_range() {
        let before = "uno\ndos\ntres\n";
        let after = "uno\ntres\n";
        let edits = round_trip(before, after);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "");
        assert_eq!(&before[edits[0].range.clone()], "dos\n");
    }

    #[test]
    fn nearby_character_changes_become_one_edit() {
        // "hola" and "chau" share an `h` and an `a`; one replacement reads
        // better than three.
        let edits = round_trip("say(\"hola\");\n", "say(\"chau\");\n");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "chau");
    }

    #[test]
    fn distant_changes_in_one_line_stay_separate() {
        let before = "let alfa = 1; let beta = 2; let gama = 3;\n";
        let after = "let alfa = 9; let beta = 2; let gama = 7;\n";
        let edits = round_trip(before, after);
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn several_hunks_stay_separate() {
        let before = "a\nb\nc\nd\ne\nf\ng\n";
        let after = "a\nB\nc\nd\ne\nF\ng\n";
        let edits = round_trip(before, after);
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn empty_sides() {
        let edits = round_trip("", "hola\n");
        assert_eq!(edits.len(), 1);
        let edits = round_trip("hola\n", "");
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn works_on_non_ascii() {
        let before = "// año pasado\nlet x = 1;\n";
        let after = "// año próximo\nlet x = 1;\n";
        let edits = round_trip(before, after);
        assert!(!edits.is_empty());
        for edit in &edits {
            assert!(before.is_char_boundary(edit.range.start));
            assert!(before.is_char_boundary(edit.range.end));
        }
    }

    #[test]
    fn a_rewrite_is_still_correct() {
        let before = "una cosa\notra cosa\n";
        let after = "nada que ver\ncon lo anterior\ny una línea más\n";
        round_trip(before, after);
    }

    #[test]
    fn a_long_line_change_falls_back_to_the_whole_hunk() {
        let before = format!("{}\n", "a".repeat(REFINE_LIMIT + 10));
        let after = format!("{}\n", "b".repeat(REFINE_LIMIT + 10));
        let edits = round_trip(&before, &after);
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn trailing_newline_changes_are_one_edit() {
        let edits = round_trip("a\nb\n", "a\nb");
        assert_eq!(edits.len(), 1);
        let edits = round_trip("a\nb", "a\nb\n");
        assert_eq!(edits.len(), 1);
    }
}
