//! Text transforms behind the line and bracket commands of the editor
//! (`editor::toggle_comments`, `editor::join_lines`, `editor::sort_lines`,
//! auto-closed pairs, auto-indent on paste…).
//!
//! Everything here is a pure function over strings and byte offsets, so the
//! rules are unit tested without a window; `EditorView` turns the results into
//! one buffer transaction.

use std::ops::Range;

/// One edit in buffer bytes: replace `range` (in the text *before* any of the
/// edits of the same batch) with the string.
pub type Edit = (Range<usize>, String);

/// How a language comments code out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentSyntax {
    /// A token that comments to the end of the line (`//`, `#`, `--`).
    Line(&'static str),
    /// A pair wrapped around each line (`<!-- … -->`, `/* … */`).
    Block(&'static str, &'static str),
}

/// The line comment token of a language, by its `cincel_syntax` name.
///
/// `cincel_syntax::Language` carries no comment metadata yet, so the table
/// lives here, keyed by [`cincel_syntax::Language::name`].
pub fn line_comment(language: &str) -> Option<&'static str> {
    match language {
        "rust" | "go" | "javascript" | "typescript" | "tsx" | "c" | "cpp" | "java" => Some("//"),
        "python" | "bash" | "yaml" | "toml" | "dockerfile" => Some("#"),
        "sql" => Some("--"),
        _ => None,
    }
}

/// The block comment pair of a language that has no line comment.
pub fn block_comment(language: &str) -> Option<(&'static str, &'static str)> {
    match language {
        "html" | "markdown" => Some(("<!--", "-->")),
        "css" => Some(("/*", "*/")),
        _ => None,
    }
}

/// What `editor::toggle_comments` uses for a language; `None` (JSON, plain
/// text) makes the command a no-op.
pub fn comment_syntax(language: &str) -> Option<CommentSyntax> {
    line_comment(language)
        .map(CommentSyntax::Line)
        .or_else(|| block_comment(language).map(|(open, close)| CommentSyntax::Block(open, close)))
}

/// Leading spaces and tabs of a line, in bytes.
pub fn indent_len(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// Edits that comment or uncomment `lines` (`(byte offset of the line start,
/// line text without the newline)`).
///
/// Blank lines are left alone and do not vote. When every other line is
/// already commented the lines are uncommented (the token and one space after
/// it go away); otherwise — mixed or none commented — every non-blank line is
/// commented: a line token at the smallest indentation of the block, so the
/// comment column lines up; a block pair around each line's own content.
pub fn toggle_comment_edits(lines: &[(usize, String)], syntax: CommentSyntax) -> Vec<Edit> {
    let content: Vec<&(usize, String)> = lines
        .iter()
        .filter(|(_, text)| !text.trim().is_empty())
        .collect();
    if content.is_empty() {
        return Vec::new();
    }
    let is_commented = |text: &str| -> bool {
        let trimmed = text.trim_start_matches([' ', '\t']);
        match syntax {
            CommentSyntax::Line(token) => trimmed.starts_with(token),
            CommentSyntax::Block(open, close) => {
                trimmed.starts_with(open) && trimmed.trim_end().ends_with(close)
            }
        }
    };
    let mut edits = Vec::new();
    if content.iter().all(|(_, text)| is_commented(text)) {
        for (start, text) in content {
            let indent = indent_len(text);
            match syntax {
                CommentSyntax::Line(token) => {
                    let mut end = indent + token.len();
                    if text[end..].starts_with(' ') {
                        end += 1;
                    }
                    edits.push((start + indent..start + end, String::new()));
                }
                CommentSyntax::Block(open, close) => {
                    let mut open_end = indent + open.len();
                    if text[open_end..].starts_with(' ') {
                        open_end += 1;
                    }
                    let body_end = text.trim_end().len();
                    let mut close_start = body_end - close.len();
                    if close_start > open_end && text[..close_start].ends_with(' ') {
                        close_start -= 1;
                    }
                    let close_start = close_start.max(open_end);
                    edits.push((start + indent..start + open_end, String::new()));
                    edits.push((start + close_start..start + body_end, String::new()));
                }
            }
        }
    } else {
        let column = content
            .iter()
            .map(|(_, text)| indent_len(text))
            .min()
            .unwrap_or(0);
        for (start, text) in content {
            // The common column is a byte count of leading whitespace, which
            // is exact when the block indents consistently (all spaces or all
            // tabs); a line with less whitespace than that cannot exist.
            let at = column.min(indent_len(text));
            match syntax {
                CommentSyntax::Line(token) => {
                    edits.push((start + at..start + at, format!("{token} ")));
                }
                CommentSyntax::Block(open, close) => {
                    // A block wraps each line's own content, so it goes after
                    // that line's indentation.
                    let at = indent_len(text);
                    let end = text.trim_end().len();
                    edits.push((start + at..start + at, format!("{open} ")));
                    edits.push((start + end..start + end, format!(" {close}")));
                }
            }
        }
    }
    edits
}

/// Where `offset` lands after `edits` (sorted, non-overlapping, in the
/// coordinates before them).
///
/// An offset inside a replaced range is clamped into the replacement; an
/// offset exactly at an insertion point moves past the inserted text, like a
/// cursor that typed it.
pub fn map_offset(offset: usize, edits: &[Edit]) -> usize {
    map_offset_biased(offset, edits, true)
}

/// Like [`map_offset`], but an offset exactly at an insertion point stays
/// before the inserted text (the start of a selection keeps covering it).
pub fn map_offset_left(offset: usize, edits: &[Edit]) -> usize {
    map_offset_biased(offset, edits, false)
}

fn map_offset_biased(offset: usize, edits: &[Edit], right: bool) -> usize {
    let mut delta: i64 = 0;
    for (range, text) in edits {
        if range.end < offset || (range.end == offset && range.start < offset) {
            delta += text.len() as i64 - (range.end - range.start) as i64;
        } else if right && range.start == offset && range.is_empty() {
            delta += text.len() as i64;
        } else if range.start < offset {
            // Inside a replaced range.
            let inside = (offset - range.start).min(text.len());
            return (range.start as i64 + delta) as usize + inside;
        } else {
            break;
        }
    }
    (offset as i64 + delta).max(0) as usize
}

/// Joins `lines` into one line the way `editor::join_lines` does: the
/// trailing whitespace of each line and the indentation of the next one
/// collapse into a single space (none when either side is empty or the next
/// line starts with a closing bracket).
///
/// Returns the joined text and the byte offsets (inside it) of every join
/// point.
pub fn join_lines(lines: &[String]) -> (String, Vec<usize>) {
    let mut joined = String::new();
    let mut points = Vec::new();
    for (ix, line) in lines.iter().enumerate() {
        if ix == 0 {
            joined.push_str(line);
            continue;
        }
        let trimmed_len = joined.trim_end_matches([' ', '\t']).len();
        joined.truncate(trimmed_len);
        let next = line.trim_start_matches([' ', '\t']);
        let glue = !joined.is_empty()
            && !next.is_empty()
            && !next.starts_with([')', ']', '}'])
            && !joined.ends_with(['(', '[', '{']);
        if glue {
            joined.push(' ');
        }
        points.push(joined.len());
        joined.push_str(next);
    }
    (joined, points)
}

/// Sorts lines by byte order, stable.
pub fn sort_lines(lines: &mut [String]) {
    lines.sort();
}

/// Opening characters that auto-close, with their closer.
pub const AUTO_PAIRS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('"', '"'),
    ('\'', '\''),
    ('`', '`'),
];

/// The closer of an auto-closing opener.
pub fn closer_for(open: char) -> Option<char> {
    AUTO_PAIRS
        .iter()
        .find(|(start, _)| *start == open)
        .map(|(_, end)| *end)
}

/// Whether `ch` closes some auto pair.
pub fn is_closer(ch: char) -> bool {
    AUTO_PAIRS.iter().any(|(_, end)| *end == ch)
}

/// Whether `ch` is a quote (its own closer).
pub fn is_quote(ch: char) -> bool {
    matches!(ch, '"' | '\'' | '`')
}

/// Whether typing `open` between `before` and `after` should insert the pair.
///
/// The pair goes in when the next character is whitespace, a closer or the end
/// of the line. A quote also needs the previous character not to be a word
/// character (so `don't` stays one quote), and `'` never pairs in Rust, where
/// it starts lifetimes as often as char literals.
pub fn should_auto_close(
    open: char,
    before: Option<char>,
    after: Option<char>,
    language: Option<&str>,
) -> bool {
    if closer_for(open).is_none() {
        return false;
    }
    if open == '\'' && language == Some("rust") {
        return false;
    }
    let next_ok = match after {
        None => true,
        Some(ch) => ch.is_whitespace() || is_closer(ch) || matches!(ch, ',' | ';' | ':'),
    };
    if !next_ok {
        return false;
    }
    if is_quote(open) && before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == open) {
        return false;
    }
    true
}

/// The bracket pairs `editor::move_to_matching_bracket` and the highlight know.
pub const BRACKETS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}')];

/// `(opener, closer, is_opener)` of a bracket character.
pub fn bracket_info(ch: char) -> Option<(char, char, bool)> {
    BRACKETS.iter().find_map(|(open, close)| {
        if ch == *open {
            Some((*open, *close, true))
        } else if ch == *close {
            Some((*open, *close, false))
        } else {
            None
        }
    })
}

/// Re-indents a multi-line paste so it sits at the cursor's indentation.
///
/// `prefix` is the text of the cursor row before the cursor and `row_indent`
/// the indentation of that row. The block keeps its own relative indentation:
/// its base is the smallest indentation of the lines after the first (plus the
/// first one when it carries leading whitespace, i.e. it was copied from the
/// start of a line), and every line after the first is moved from that base to
/// the target. A paste at column 0 or a single line is left untouched, which
/// keeps whole-line copy and paste exact.
pub fn reindent_paste(text: &str, prefix: &str, row_indent: &str) -> String {
    if prefix.is_empty() || !text.contains('\n') {
        return text.to_string();
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let only_whitespace_before = prefix.trim_start_matches([' ', '\t']).is_empty();
    let target = if only_whitespace_before {
        prefix
    } else {
        row_indent
    };
    let first_indent = indent_len(lines[0]);
    let mut base = lines[1..]
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| indent_len(line))
        .min();
    if only_whitespace_before && first_indent > 0 && !lines[0].trim().is_empty() {
        base = Some(base.map_or(first_indent, |base| base.min(first_indent)));
    }
    let Some(base) = base else {
        return text.to_string();
    };
    let mut out = String::with_capacity(text.len() + lines.len() * target.len());
    for (ix, line) in lines.iter().enumerate() {
        if ix > 0 {
            out.push('\n');
        }
        if ix == 0 {
            if only_whitespace_before {
                // The cursor already stands on the target indentation.
                out.push_str(&line[first_indent.min(base)..]);
            } else {
                out.push_str(line);
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        out.push_str(target);
        out.push_str(&line[indent_len(line).min(base)..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<(usize, String)> {
        let mut offset = 0;
        text.split('\n')
            .map(|line| {
                let start = offset;
                offset += line.len() + 1;
                (start, line.to_string())
            })
            .collect()
    }

    fn apply(text: &str, edits: &[Edit]) -> String {
        let mut out = text.to_string();
        for (range, new) in edits.iter().rev() {
            out.replace_range(range.clone(), new);
        }
        out
    }

    #[test]
    fn every_listed_language_has_a_comment_token() {
        for name in [
            "rust",
            "go",
            "javascript",
            "typescript",
            "tsx",
            "c",
            "cpp",
            "java",
        ] {
            assert_eq!(line_comment(name), Some("//"), "{name}");
        }
        for name in ["python", "bash", "yaml", "toml", "dockerfile"] {
            assert_eq!(line_comment(name), Some("#"), "{name}");
        }
        assert_eq!(line_comment("sql"), Some("--"));
        assert_eq!(
            comment_syntax("html"),
            Some(CommentSyntax::Block("<!--", "-->"))
        );
        assert_eq!(
            comment_syntax("markdown"),
            Some(CommentSyntax::Block("<!--", "-->"))
        );
        assert_eq!(
            comment_syntax("css"),
            Some(CommentSyntax::Block("/*", "*/"))
        );
        assert_eq!(comment_syntax("json"), None);
    }

    #[test]
    fn commenting_aligns_the_token_at_the_smallest_indent() {
        let text = "    a\n        b\n\n    c";
        let edits = toggle_comment_edits(&lines(text), CommentSyntax::Line("//"));
        assert_eq!(apply(text, &edits), "    // a\n    //     b\n\n    // c");
    }

    #[test]
    fn mixed_lines_are_all_commented() {
        let text = "// a\nb";
        let edits = toggle_comment_edits(&lines(text), CommentSyntax::Line("//"));
        assert_eq!(apply(text, &edits), "// // a\n// b");
    }

    #[test]
    fn commented_lines_are_uncommented_with_one_space() {
        let text = "  # a\n  #b\n";
        let edits = toggle_comment_edits(&lines(text), CommentSyntax::Line("#"));
        assert_eq!(apply(text, &edits), "  a\n  b\n");
    }

    #[test]
    fn block_comments_wrap_each_line_and_unwrap_it() {
        let text = "<p>hola</p>\n  <b>x</b>";
        let edits = toggle_comment_edits(&lines(text), CommentSyntax::Block("<!--", "-->"));
        let commented = apply(text, &edits);
        assert_eq!(commented, "<!-- <p>hola</p> -->\n  <!-- <b>x</b> -->");
        let edits = toggle_comment_edits(&lines(&commented), CommentSyntax::Block("<!--", "-->"));
        assert_eq!(apply(&commented, &edits), text);
    }

    #[test]
    fn offsets_follow_insertions_and_deletions() {
        let edits = vec![(2..2, "xx".to_string()), (5..7, String::new())];
        assert_eq!(map_offset(0, &edits), 0);
        assert_eq!(map_offset(2, &edits), 4, "an insertion point moves past it");
        assert_eq!(map_offset(4, &edits), 6);
        assert_eq!(map_offset(6, &edits), 7, "inside a deletion clamps to it");
        assert_eq!(map_offset(9, &edits), 9);
        assert_eq!(map_offset_left(2, &edits), 2, "left bias stays before it");
        assert_eq!(map_offset_left(4, &edits), 6);
    }

    #[test]
    fn join_collapses_whitespace_into_one_space() {
        let (joined, points) = join_lines(&[
            "let x = foo(  ".to_string(),
            "    1,".to_string(),
            "    2".to_string(),
            ");".to_string(),
        ]);
        assert_eq!(joined, "let x = foo(1, 2);");
        assert_eq!(points, vec![12, 15, 16]);
        let (joined, _) = join_lines(&["a".to_string(), "".to_string(), "b".to_string()]);
        assert_eq!(joined, "a b");
    }

    #[test]
    fn sort_is_by_byte_order() {
        let mut lines = vec!["b".to_string(), "B".to_string(), "a".to_string()];
        sort_lines(&mut lines);
        assert_eq!(lines, vec!["B", "a", "b"]);
    }

    #[test]
    fn pairs_only_close_before_whitespace_closers_or_the_end() {
        assert!(should_auto_close('(', None, None, None));
        assert!(should_auto_close('(', Some('x'), Some(')'), None));
        assert!(should_auto_close('[', None, Some(' '), None));
        assert!(!should_auto_close('(', None, Some('x'), None));
        assert!(
            !should_auto_close('"', Some('a'), None, None),
            "after a word"
        );
        assert!(!should_auto_close('\'', Some(' '), None, Some("rust")));
        assert!(should_auto_close('\'', Some(' '), None, Some("python")));
        assert!(!should_auto_close('x', None, None, None));
    }

    #[test]
    fn paste_takes_the_cursor_indentation() {
        let pasted = "if x {\n    y();\n}";
        assert_eq!(
            reindent_paste(pasted, "        ", "        "),
            "if x {\n            y();\n        }"
        );
        // Copied from the start of an indented line.
        assert_eq!(reindent_paste("    a\n        b", "  ", "  "), "a\n      b");
        // After code on the row: the block follows the row's indentation.
        assert_eq!(
            reindent_paste("foo(\n    1\n)", "    let x = ", "    "),
            "foo(\n        1\n    )"
        );
        // Column 0 and single lines stay exact.
        assert_eq!(reindent_paste("  a\n  b\n", "", ""), "  a\n  b\n");
        assert_eq!(reindent_paste("x", "    ", "    "), "x");
    }
}
