//! The text of the review block the agent receives (spec 09 §6.8).

use std::fmt::Write;

use crate::comments::{SentComment, SentRange};

/// The inside of `<user_review_feedback>`: the patches of
/// [`ReviewStore::report_for_agent`](crate::ReviewStore::report_for_agent)
/// exactly as they are, then (after a blank line, when there were patches)
/// the section with the comments, in the order given (the order of
/// [`ReviewStore::take_comments_for_prompt`](crate::ReviewStore::take_comments_for_prompt)).
/// `None` when there is neither.
///
/// Plain text, `\n` line ends, no trailing spaces on the lines it writes,
/// one blank line between comments; the code and the user's text go as
/// they are.
pub fn format_feedback(report: Option<&str>, comments: &[SentComment]) -> Option<String> {
    let report = report.filter(|r| !r.is_empty());
    if comments.is_empty() {
        return report.map(str::to_owned);
    }
    let mut out = String::new();
    if let Some(report) = report {
        out.push_str(report);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    let total = comments.len();
    let noun = if total == 1 { "comment" } else { "comments" };
    let _ = writeln!(
        out,
        "The user left {total} {noun} on the code. Line numbers are 1-based and refer to each file as it is now. Read every comment together with its code and act on it in this turn."
    );
    for (index, comment) in comments.iter().enumerate() {
        out.push('\n');
        write_comment(&mut out, index + 1, total, comment);
    }
    while out.ends_with('\n') {
        out.pop();
    }
    Some(out)
}

fn write_comment(out: &mut String, index: usize, total: usize, comment: &SentComment) {
    let _ = writeln!(out, "Comment {index} of {total}");
    let deleted = if comment.kind == SentRange::DeletedFile {
        " (you deleted this file)"
    } else {
        ""
    };
    let _ = writeln!(out, "File: {}{deleted}", comment.display_path);
    match comment.kind {
        SentRange::RemovedBefore => {
            let _ = writeln!(out, "Lines: removed before line {}", comment.first_line);
        }
        _ if comment.first_line == comment.last_line => {
            let _ = writeln!(out, "Line: {}", comment.first_line);
        }
        _ => {
            let _ = writeln!(out, "Lines: {}-{}", comment.first_line, comment.last_line);
        }
    }
    let _ = writeln!(out, "Review state: {}", comment.state.agent_text());
    let lang = comment.lang.as_deref().unwrap_or("");
    if let Some(code) = &comment.code {
        let unsaved = if comment.unsaved {
            " (as shown in the editor, not saved yet)"
        } else {
            ""
        };
        let _ = writeln!(out, "Current code{unsaved}:");
        write_fenced(out, code, lang);
        write_truncated(out, comment.truncated_lines);
    }
    if let Some(removed) = &comment.removed {
        out.push_str("Lines you removed (not reviewed yet):\n");
        write_fenced(out, removed, lang);
        if comment.code.is_none() {
            write_truncated(out, comment.truncated_lines);
        }
    }
    out.push_str("Comment:\n");
    out.push_str(comment.text.trim_end_matches('\n'));
    out.push('\n');
}

fn write_fenced(out: &mut String, text: &str, lang: &str) {
    let fence = fence_for(text);
    let _ = writeln!(out, "{fence}{lang}");
    out.push_str(text);
    out.push('\n');
    let _ = writeln!(out, "{fence}");
}

fn write_truncated(out: &mut String, lines: u32) {
    if lines > 0 {
        let _ = writeln!(out, "({lines} more lines not shown)");
    }
}

/// As many backticks as the longest run of backticks in `text` plus one,
/// at least three.
pub fn fence_for(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for ch in text.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}
