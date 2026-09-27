//! Prints the review hunks between two files, for manual checks.
//!
//! ```text
//! cargo run -p cincel-review --example review_cli -- base.txt new.txt
//! ```
//!
//! Removed lines are prefixed with `-`, added lines with `+`; changed words
//! of short hunks are wrapped in `[-…-]` / `{+…+}`.

use std::ops::Range;
use std::process::ExitCode;

use cincel_review::diff::{RawHunk, diff_texts, split_lines};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [base_path, new_path] = args.as_slice() else {
        eprintln!("uso: review_cli <base.txt> <nuevo.txt>");
        return ExitCode::from(2);
    };
    let read = |path: &str| match std::fs::read_to_string(path) {
        Ok(text) => Some(text.replace("\r\n", "\n")),
        Err(error) => {
            eprintln!("no se pudo leer {path}: {error}");
            None
        }
    };
    let (Some(base), Some(new)) = (read(base_path), read(new_path)) else {
        return ExitCode::FAILURE;
    };
    let hunks = diff_texts(&base, &new);
    if hunks.is_empty() {
        println!("sin cambios");
        return ExitCode::SUCCESS;
    }
    for (index, hunk) in hunks.iter().enumerate() {
        print_hunk(index, hunk, &base, &new);
    }
    ExitCode::SUCCESS
}

fn print_hunk(index: usize, hunk: &RawHunk, base: &str, new: &str) {
    println!(
        "@@ hunk {} ({:?}) · base {} · nuevo {} · {} pares de líneas @@",
        index + 1,
        hunk.kind,
        rows(&hunk.base_rows),
        rows(&hunk.buffer_rows),
        hunk.lines.len()
    );
    let base_text = &base[hunk.base_bytes.clone()];
    let new_text = &new[hunk.buffer_bytes.clone()];
    let (base_marks, new_marks) = match &hunk.word_diffs {
        Some(words) => (words.base.clone(), words.buffer.clone()),
        None => (Vec::new(), Vec::new()),
    };
    print_side('-', base_text, &base_marks, "[-", "-]");
    print_side('+', new_text, &new_marks, "{+", "+}");
}

/// 1-based, inclusive row range for humans.
fn rows(range: &Range<u32>) -> String {
    if range.is_empty() {
        format!("sin filas (antes de la {})", range.start + 1)
    } else {
        format!("filas {}-{}", range.start + 1, range.end)
    }
}

fn print_side(prefix: char, text: &str, marks: &[Range<usize>], open: &str, close: &str) {
    for line in split_lines(text) {
        let mut out = String::new();
        let mut at = line.start;
        for mark in marks {
            let start = mark.start.max(line.start);
            let end = mark.end.min(line.end);
            if start >= end {
                continue;
            }
            out.push_str(&text[at..start]);
            out.push_str(open);
            out.push_str(text[start..end].trim_end_matches('\n'));
            out.push_str(close);
            if text[start..end].ends_with('\n') {
                out.push('\n');
            }
            at = end;
        }
        out.push_str(&text[at..line.end]);
        let shown = out.trim_end_matches('\n');
        println!("{prefix}{shown}");
    }
}
