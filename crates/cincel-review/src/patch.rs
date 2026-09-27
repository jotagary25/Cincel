//! Unified patches in the format `git apply` accepts.

use imara_diff::{Algorithm, Diff, InternedInput};

/// Lines of context around each change.
pub const CONTEXT_LINES: u32 = 3;

/// A git-style unified patch turning `old` into `new` for `path` (relative,
/// `/` separated). `None` on a side means the file does not exist there
/// (creation or deletion). Returns an empty string when nothing changes.
pub fn unified_patch(path: &str, old: Option<&str>, new: Option<&str>) -> String {
    if old == new {
        return String::new();
    }
    let old_text = old.unwrap_or("");
    let new_text = new.unwrap_or("");
    let mut out = format!("diff --git a/{path} b/{path}\n");
    match (old, new) {
        (None, _) => {
            out.push_str("new file mode 100644\n--- /dev/null\n");
            out.push_str(&format!("+++ b/{path}\n"));
        }
        (_, None) => {
            out.push_str("deleted file mode 100644\n");
            out.push_str(&format!("--- a/{path}\n+++ /dev/null\n"));
        }
        _ => out.push_str(&format!("--- a/{path}\n+++ b/{path}\n")),
    }
    if old_text == new_text {
        // Creating an empty file or deleting one: headers only.
        return out;
    }

    let old_lines = lines(old_text);
    let new_lines = lines(new_text);
    let input = InternedInput::new(old_text, new_text);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    let hunks: Vec<imara_diff::Hunk> = diff.hunks().collect();

    // Group hunks whose context would touch or overlap.
    let mut groups: Vec<Vec<imara_diff::Hunk>> = Vec::new();
    for hunk in hunks {
        match groups.last_mut() {
            Some(group)
                if hunk.before.start - group.last().expect("non-empty").before.end
                    <= 2 * CONTEXT_LINES =>
            {
                group.push(hunk)
            }
            _ => groups.push(vec![hunk]),
        }
    }

    let old_count = old_lines.len() as u32;
    for group in groups {
        let first = group.first().expect("non-empty");
        let last = group.last().expect("non-empty");
        let old_start = first.before.start.saturating_sub(CONTEXT_LINES);
        let old_end = (last.before.end + CONTEXT_LINES).min(old_count);
        let new_start = first.after.start - (first.before.start - old_start);
        let new_end = last.after.end + (old_end - last.before.end);
        let old_len = old_end - old_start;
        let new_len = new_end - new_start;
        out.push_str(&format!(
            "@@ -{} +{} @@\n",
            header_range(old_start, old_len),
            header_range(new_start, new_len)
        ));
        let mut cursor = old_start;
        for hunk in &group {
            for line in cursor..hunk.before.start {
                push_line(&mut out, ' ', old_lines[line as usize]);
            }
            for line in hunk.before.clone() {
                push_line(&mut out, '-', old_lines[line as usize]);
            }
            for line in hunk.after.clone() {
                push_line(&mut out, '+', new_lines[line as usize]);
            }
            cursor = hunk.before.end;
        }
        for line in cursor..old_end {
            push_line(&mut out, ' ', old_lines[line as usize]);
        }
    }
    out
}

fn header_range(start: u32, len: u32) -> String {
    // A count of zero names the line *before* the (empty) range.
    let first = if len == 0 { start } else { start + 1 };
    format!("{first},{len}")
}

fn push_line(out: &mut String, prefix: char, line: &str) {
    out.push(prefix);
    out.push_str(line);
    if !line.ends_with('\n') {
        out.push_str("\n\\ No newline at end of file\n");
    }
}

fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// Applies a patch produced by [`unified_patch`] to `old` (only the subset
/// of the format it emits: one file, `@@` hunks, no-newline markers). Returns
/// `None` if the patch does not apply. Meant for tests and sanity checks.
pub fn apply_patch(old: &str, patch: &str) -> Option<String> {
    let old_lines = lines(old);
    let mut out = String::new();
    let mut cursor = 0usize;
    let mut body = patch.lines().peekable();
    // Skip headers.
    while let Some(line) = body.peek() {
        if line.starts_with("@@") {
            break;
        }
        body.next();
    }
    let mut last_kind = ' ';
    for line in body {
        if let Some(header) = line.strip_prefix("@@ -") {
            let old_spec = header.split(' ').next()?;
            let mut parts = old_spec.split(',');
            let start: usize = parts.next()?.parse().ok()?;
            let len: usize = parts.next().unwrap_or("1").parse().ok()?;
            let first = if len == 0 {
                start
            } else {
                start.checked_sub(1)?
            };
            if first < cursor || first > old_lines.len() {
                return None;
            }
            for l in &old_lines[cursor..first] {
                out.push_str(l);
            }
            cursor = first;
            continue;
        }
        if line == "\\ No newline at end of file" {
            if matches!(last_kind, ' ' | '+') && out.ends_with('\n') {
                out.pop();
            }
            continue;
        }
        let (kind, text) = line.split_at(1.min(line.len()));
        let text = format!("{text}\n");
        last_kind = kind.chars().next().unwrap_or(' ');
        match last_kind {
            ' ' | '-' => {
                let expected = old_lines.get(cursor)?;
                if expected.trim_end_matches('\n') != text.trim_end_matches('\n') {
                    return None;
                }
                if last_kind == ' ' {
                    out.push_str(&text);
                }
                cursor += 1;
            }
            '+' => out.push_str(&text),
            _ => return None,
        }
    }
    for l in &old_lines[cursor.min(old_lines.len())..] {
        out.push_str(l);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let cases = [
            ("a\nb\nc\n", "a\nB\nc\n"),
            ("a\nb\nc", "a\nb\nc\n"),
            ("a\nb\nc\n", "a\nb\nc"),
            ("", "x\ny\n"),
            (
                "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n",
                "1\nx\n3\n4\n5\n6\n7\n8\n9\n10\ny\n12\n",
            ),
            ("keep\n", "keep\nmore\n"),
        ];
        for (old, new) in cases {
            let patch = unified_patch("f.txt", Some(old), Some(new));
            assert_eq!(apply_patch(old, &patch).as_deref(), Some(new), "{patch}");
        }
    }

    #[test]
    fn creation_and_deletion_headers() {
        let created = unified_patch("n.txt", None, Some("x\n"));
        assert!(
            created.contains(
                "new file mode 100644\n--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1,1 @@\n+x\n"
            )
        );
        let deleted = unified_patch("n.txt", Some("x\n"), None);
        assert!(deleted.contains(
            "deleted file mode 100644\n--- a/n.txt\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-x\n"
        ));
        assert!(unified_patch("n.txt", Some("x"), Some("x")).is_empty());
    }
}
