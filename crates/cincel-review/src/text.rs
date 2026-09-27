//! Small text helpers over ropes and strings.
//!
//! Rows here are always `\n` rows (the same unit `imara-diff` uses), never
//! ropey's Unicode line breaks, so a form feed or U+2028 inside a line can
//! not shift a hunk.

use std::ops::Range;

use cincel_text::Rope;

/// Normalizes text coming from outside the buffer: drops a leading BOM and
/// turns `\r\n` and lone `\r` into `\n`, like `Buffer` does on load.
pub fn normalize(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_owned()
    }
}

/// Number of `\n` in `text`.
pub fn count_newlines(text: &str) -> usize {
    text.bytes().filter(|&b| b == b'\n').count()
}

/// Number of `\n` in a byte range of a rope.
pub fn rope_newlines(rope: &Rope, range: Range<usize>) -> usize {
    rope.byte_slice(range).chunks().map(count_newlines).sum()
}

/// Number of `\n` rows of a whole rope (a trailing empty row counts).
pub fn rope_rows(rope: &Rope) -> usize {
    rope_newlines(rope, 0..rope.len_bytes()) + 1
}

/// The text of a byte range.
pub fn rope_text(rope: &Rope, range: Range<usize>) -> String {
    rope.byte_slice(range).to_string()
}

/// Replaces a byte range of a rope.
pub fn rope_replace(rope: &mut Rope, range: Range<usize>, text: &str) {
    let start = rope.byte_to_char(range.start);
    let end = rope.byte_to_char(range.end);
    if end > start {
        rope.remove(start..end);
    }
    if !text.is_empty() {
        rope.insert(start, text);
    }
}

/// Whether `offset` starts a line.
pub fn at_line_start(rope: &Rope, offset: usize) -> bool {
    offset == 0 || rope.byte(offset - 1) == b'\n'
}

/// Whether `offset` ends a region of whole lines (right after a `\n`, at the
/// very start, or at the end of the text).
pub fn at_line_end(rope: &Rope, offset: usize) -> bool {
    offset == 0 || offset == rope.len_bytes() || rope.byte(offset - 1) == b'\n'
}

/// Offset of the last `\n` in `lo..hi`.
pub fn prev_newline(rope: &Rope, lo: usize, hi: usize) -> Option<usize> {
    let mut bytes = rope.bytes_at(hi);
    let mut at = hi;
    while at > lo {
        at -= 1;
        if bytes.prev() == Some(b'\n') {
            return Some(at);
        }
    }
    None
}

/// Offset of the first `\n` in `lo..hi`.
pub fn next_newline(rope: &Rope, lo: usize, hi: usize) -> Option<usize> {
    let bytes = rope.bytes_at(lo);
    for (k, byte) in bytes.take(hi.saturating_sub(lo)).enumerate() {
        if byte == b'\n' {
            return Some(lo + k);
        }
    }
    None
}

/// Whether the first [`BINARY_SNIFF_BYTES`](crate::diff::BINARY_SNIFF_BYTES)
/// of a rope contain a NUL.
pub fn rope_is_binary(rope: &Rope) -> bool {
    let end = rope.len_bytes().min(crate::diff::BINARY_SNIFF_BYTES);
    rope.byte_slice(..end)
        .chunks()
        .any(|chunk| chunk.as_bytes().contains(&0))
}

/// Whether a rope is over the inline review limits.
pub fn rope_is_too_large(rope: &Rope) -> bool {
    rope.len_bytes() > crate::diff::MAX_INLINE_BYTES
        || (rope.len_bytes() >= crate::diff::MAX_INLINE_LINES
            && rope_rows(rope) > crate::diff::MAX_INLINE_LINES)
}

/// Joins line texts so that every line but the last ends with `\n`.
pub fn join_lines(parts: &[String]) -> String {
    let mut out = String::new();
    let count = parts.len();
    for (index, part) in parts.iter().enumerate() {
        out.push_str(part);
        if index + 1 < count && !part.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Length of the common prefix of two strings, on a `char` boundary.
pub fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or(a.len().min(b.len()), |((index, _), _)| index)
        .min(b.len())
}

/// Length of the common suffix of two strings, on a `char` boundary.
pub fn common_suffix(a: &str, b: &str) -> usize {
    let mut length = 0;
    for (x, y) in a.chars().rev().zip(b.chars().rev()) {
        if x != y {
            break;
        }
        length += x.len_utf8();
    }
    length
}

/// Hex of a byte slice.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// SHA-256 of a text, as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex(&sha2::Sha256::digest(bytes))
}

/// SHA-256 of a rope, as lowercase hex.
pub fn rope_sha256(rope: &Rope) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    for chunk in rope.chunks() {
        hasher.update(chunk.as_bytes());
    }
    hex(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newline_search() {
        let rope = Rope::from_str("ab\ncd\nef");
        assert_eq!(prev_newline(&rope, 0, 5), Some(2));
        assert_eq!(prev_newline(&rope, 3, 5), None);
        assert_eq!(next_newline(&rope, 3, 8), Some(5));
        assert_eq!(next_newline(&rope, 6, 8), None);
        assert!(at_line_start(&rope, 3));
        assert!(!at_line_start(&rope, 4));
        assert!(at_line_end(&rope, 8));
    }

    #[test]
    fn normalize_line_endings() {
        assert_eq!(normalize("\u{feff}a\r\nb\rc"), "a\nb\nc");
    }

    #[test]
    fn affixes() {
        assert_eq!(common_prefix("abcx", "abcy"), 3);
        assert_eq!(common_suffix("xabc", "yabc"), 3);
        assert_eq!(common_prefix("ñx", "ñy"), 2);
        assert_eq!(
            join_lines(&["a".into(), "b\n".into(), "c".into()]),
            "a\nb\nc"
        );
    }
}
