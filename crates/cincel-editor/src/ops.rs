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

/// `indent` with one level removed, by the rule of `Shift+Tab`
/// (`EditorView::indent_rows` with `add == false`): one `unit` if the indent
/// starts with it, else one tab, else the leading spaces there are, never
/// more than the length of `unit`. `indent` is the leading whitespace of a
/// row; the result is a suffix of it.
pub fn outdent_once<'a>(indent: &'a str, unit: &str) -> &'a str {
    let removed = if indent.starts_with(unit) {
        unit.len()
    } else if indent.starts_with('\t') {
        1
    } else {
        indent.len() - indent.trim_start_matches(' ').len()
    }
    .min(unit.len().max(1));
    indent.get(removed..).unwrap_or("")
}

/// How many spaces `Backspace` deletes when the cursor stands at `column`
/// with only spaces before it: up to the previous indent stop of width
/// `size`, i.e. `((column - 1) % size) + 1`. Zero at column 0; a `size` of 0
/// counts as 1.
pub fn spaces_to_previous_stop(column: usize, size: usize) -> usize {
    if column == 0 {
        return 0;
    }
    (column - 1) % size.max(1) + 1
}

/// Edits that clean a file up before it is saved
/// (`docs/specs/10-etapa7-ronda2.md` §7.8), as buffer-byte edits over the text
/// *before* any of them applies.
///
/// `lines` are the rows of the buffer without their `\n` (the `str::lines`
/// shape: a text that ends in a line break has no extra empty row), so the
/// offset of each row is the sum of the previous lengths plus one per break.
/// `ends_with_newline` says whether the last row is followed by a break.
/// `protected_rows` are rows (0-based, end exclusive) that are never touched.
///
/// With `trim`, the spaces and tabs at the end of every unprotected row go.
/// With `final_newline`, a non-empty text that does not end in a line break
/// gets one (`"\n"`; `cincel-text` writes it as `\r\n` in a CRLF file). The
/// check runs on the trimmed text, so a last row of only blanks is trimmed
/// and the text is not given a second break. An empty text stays empty and
/// blank rows at the end are kept. A protected last row gets no break. Returns no edits when there is nothing to
/// clean.
pub fn save_cleanup_edits(
    lines: &[&str],
    protected_rows: &[Range<u32>],
    trim: bool,
    final_newline: bool,
    ends_with_newline: bool,
) -> Vec<Edit> {
    let is_protected = |row: usize| {
        let row = row as u32;
        protected_rows.iter().any(|range| range.contains(&row))
    };
    let mut edits: Vec<Edit> = Vec::new();
    let mut offset = 0usize;
    // Where the text ends and whether its last row is still non-blank once
    // trimmed (so it needs the break).
    let mut text_end = 0usize;
    let mut last_row_kept = None;
    let mut last_row_protected = false;
    for (row, line) in lines.iter().enumerate() {
        let start = offset;
        let end = start + line.len();
        offset = end + 1;
        text_end = end;
        last_row_protected = is_protected(row);
        let trimmed_len = if trim && !last_row_protected {
            line.trim_end_matches([' ', '\t']).len()
        } else {
            line.len()
        };
        if trimmed_len < line.len() {
            edits.push((start + trimmed_len..end, String::new()));
        }
        last_row_kept = Some(trimmed_len > 0);
    }
    if final_newline && !ends_with_newline {
        // A last row that trims to nothing is either the only row (an empty
        // file stays empty) or follows a break already.
        // A last row inside a pending agent segment is not touched either:
        // its line break would be a change the user has not reviewed.
        let needs_break = last_row_kept == Some(true) && !last_row_protected;
        if needs_break {
            edits.push((text_end..text_end, "\n".to_owned()));
        }
    }
    edits
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

    #[test]
    fn outdent_once_follows_the_shift_tab_rule() {
        let unit = "    ";
        assert_eq!(outdent_once("        ", unit), "    ", "one unit");
        assert_eq!(outdent_once("    ", unit), "");
        assert_eq!(outdent_once("      ", unit), "  ", "6 spaces lose one unit");
        assert_eq!(
            outdent_once("  ", unit),
            "",
            "fewer spaces than a unit: all"
        );
        assert_eq!(outdent_once(" ", unit), "");
        assert_eq!(
            outdent_once("\t\t", unit),
            "\t",
            "a tab when the unit is spaces"
        );
        assert_eq!(outdent_once("\t\t", "\t"), "\t");
        assert_eq!(outdent_once("\t", "\t"), "");
        assert_eq!(outdent_once("  \t", unit), "\t", "mixed: the spaces first");
        assert_eq!(outdent_once("  ", "\t"), " ", "never more than the unit");
        assert_eq!(outdent_once("", unit), "");
        assert_eq!(outdent_once("   ", "  "), " ", "a two-space unit");
    }

    #[test]
    fn spaces_to_previous_stop_reaches_the_previous_multiple() {
        assert_eq!(spaces_to_previous_stop(0, 4), 0);
        assert_eq!(spaces_to_previous_stop(1, 4), 1);
        assert_eq!(spaces_to_previous_stop(2, 4), 2);
        assert_eq!(spaces_to_previous_stop(4, 4), 4);
        assert_eq!(spaces_to_previous_stop(5, 4), 1);
        assert_eq!(spaces_to_previous_stop(6, 4), 2);
        assert_eq!(spaces_to_previous_stop(8, 4), 4);
        assert_eq!(spaces_to_previous_stop(3, 2), 1);
        assert_eq!(spaces_to_previous_stop(4, 2), 2);
        assert_eq!(spaces_to_previous_stop(5, 1), 1, "a one-column unit");
        assert_eq!(spaces_to_previous_stop(5, 0), 1, "size 0 counts as 1");
    }

    /// Runs `save_cleanup_edits` over `text` the way the editor does and
    /// returns the cleaned text.
    fn cleaned(text: &str, protected: &[Range<u32>], trim: bool, final_newline: bool) -> String {
        let ends = text.ends_with('\n');
        let body = text.strip_suffix('\n').unwrap_or(text);
        let rows: Vec<&str> = if text.is_empty() {
            Vec::new()
        } else {
            body.split('\n').collect()
        };
        let edits = save_cleanup_edits(&rows, protected, trim, final_newline, ends);
        apply(text, &edits)
    }

    #[test]
    fn save_cleanup_trims_blanks_and_adds_the_final_newline() {
        assert_eq!(cleaned("a  \nb\t\nc", &[], true, true), "a\nb\nc\n");
        assert_eq!(cleaned("a \t \nb\n", &[], true, true), "a\nb\n");
    }

    #[test]
    fn save_cleanup_options_are_independent() {
        assert_eq!(cleaned("a  \nb", &[], true, false), "a\nb");
        assert_eq!(cleaned("a  \nb", &[], false, true), "a  \nb\n");
        assert_eq!(cleaned("a  \nb", &[], false, false), "a  \nb");
        assert!(save_cleanup_edits(&["a  ", "b"], &[], false, false, false).is_empty());
    }

    #[test]
    fn save_cleanup_without_work_returns_no_edits() {
        assert!(save_cleanup_edits(&["a", "b"], &[], true, true, true).is_empty());
        assert!(save_cleanup_edits(&[], &[], true, true, false).is_empty());
        assert!(save_cleanup_edits(&["a"], &[], true, false, false).is_empty());
    }

    #[test]
    fn save_cleanup_edit_offsets_count_one_per_break() {
        let edits = save_cleanup_edits(&["ab ", "c", "d\t "], &[], true, true, false);
        // "ab \nc\nd\t " is 9 bytes: the first row trims at 2..3, the last at
        // 7..9, and the final break goes at the end of the text.
        assert_eq!(
            edits,
            vec![
                (2..3, String::new()),
                (7..9, String::new()),
                (9..9, "\n".to_owned())
            ]
        );
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn save_cleanup_leaves_protected_rows_alone() {
        assert_eq!(
            cleaned("a  \nb  \nc  \nd  ", &[1..3], true, true),
            "a\nb  \nc  \nd\n"
        );
        // Several ranges; a protected last row keeps its blanks and gets no
        // break (see the next test).
        assert_eq!(
            cleaned("a  \nb  \nc  ", &[0..1, 2..3], true, true),
            "a  \nb\nc  "
        );
        assert_eq!(
            cleaned("a  \nb  \nc  \n", &[0..1, 2..3], true, true),
            "a  \nb\nc  \n"
        );
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn save_cleanup_adds_no_break_after_a_protected_last_row() {
        assert_eq!(cleaned("a  \nb  ", &[1..2], true, true), "a\nb  ");
        assert_eq!(cleaned("a  \nb", &[1..2], true, true), "a\nb");
        assert_eq!(cleaned("a  \nb", &[0..1], true, true), "a  \nb\n");
    }

    #[test]
    fn save_cleanup_keeps_an_empty_file_empty() {
        assert_eq!(cleaned("", &[], true, true), "");
        assert!(save_cleanup_edits(&[], &[], true, true, false).is_empty());
        // Only blanks and no break: trimmed away, nothing to terminate.
        assert_eq!(cleaned("   ", &[], true, true), "");
        // With the trim off the blanks are content, so the break is added.
        assert_eq!(cleaned("   ", &[], false, true), "   \n");
    }

    #[test]
    fn save_cleanup_keeps_trailing_blank_rows() {
        assert_eq!(cleaned("a\n\n\n", &[], true, true), "a\n\n\n");
        assert_eq!(cleaned("a\n\n  ", &[], true, true), "a\n\n");
        assert_eq!(cleaned("a\n   ", &[], true, true), "a\n");
        assert_eq!(cleaned("\n", &[], true, true), "\n");
        assert_eq!(cleaned("  \n", &[], true, true), "\n");
    }

    #[test]
    fn save_cleanup_only_trims_spaces_and_tabs() {
        // Non-breaking space and other Unicode blanks are content.
        assert_eq!(
            cleaned("a\u{a0}\nb\u{2003}\n", &[], true, true),
            "a\u{a0}\nb\u{2003}\n"
        );
        // Blanks inside the row stay.
        assert_eq!(cleaned("a  b \n", &[], true, true), "a  b\n");
    }
}
