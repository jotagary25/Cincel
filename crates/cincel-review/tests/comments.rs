//! Comments for the agent (spec 09 §6.4, §6.8, §6.9 a–m on the engine
//! side, §6.11): anchors, decision marks, state, take/restore, snippets and
//! the exact text of the block.

mod common;

use std::path::{Path, PathBuf};

use cincel_review::{
    COMMENT_MAX_CHARS, CommentId, CommentState, HunkId, REPORT_HEADER, SentComment, SentRange,
    TurnId, fence_for, format_feedback,
};
use cincel_text::{Buffer, BufferSnapshot, Rope};
use common::{Harness, p};

const ROOT: &str = "/proj";
const CALC: &str = "/proj/src/calc.py";
const UTIL: &str = "/proj/src/util.py";

const CALC_BASE: &str = "import math\n\ndef total(items):\n    return sum(items)\n\ndef mean(items):\n    return total(items) / len(items)\n";
const CALC_AGENT: &str = "import math\n\ndef total(items, discount=0):\n    return sum(items) * (1 - discount)\n\ndef mean(items):\n    return sum(items) / len(items)\n";
const UTIL_TEXT: &str = "import os\n\nHOST = 'x'\nPORT = 1\n\n\nTIMEOUT = 3\nRETRIES = 2\n";

const INTRO_1: &str = "The user left 1 comment on the code. Line numbers are 1-based and refer to each file as it is now. Read every comment together with its code and act on it in this turn.\n";
const INTRO_2: &str = "The user left 2 comments on the code. Line numbers are 1-based and refer to each file as it is now. Read every comment together with its code and act on it in this turn.\n";

fn snap(h: &Harness, path: &str) -> BufferSnapshot {
    h.buffers[&p(path)].snapshot()
}

/// A harness whose agent changed `calc.py` (two hunks: rows 2..4 and row 6)
/// in turn 1, which is over.
fn calc_reviewed() -> Harness {
    let mut h = Harness::new();
    h.store.set_workspace_root(Some(PathBuf::from(ROOT)));
    h.start(CALC, CALC_BASE);
    h.agent_write(CALC, CALC_AGENT);
    h.settle(CALC);
    h.store.end_turn(TurnId(1));
    assert_eq!(rows_of_hunks(&h, CALC), vec![2..4, 6..7]);
    h
}

fn rows_of_hunks(h: &Harness, path: &str) -> Vec<std::ops::Range<u32>> {
    let file = h.store.file(&p(path)).expect("tracked");
    h.store
        .hunks(&p(path))
        .into_iter()
        .map(|hunk| file.buffer_rows(hunk))
        .collect()
}

fn hunk_id(h: &Harness, path: &str, index: usize) -> HunkId {
    h.hunks(path)[index].id
}

fn add(
    h: &mut Harness,
    path: &str,
    rows: std::ops::Range<u32>,
    hunk: Option<HunkId>,
    text: &str,
) -> CommentId {
    let snapshot = snap(h, path);
    h.store
        .add_comment(&p(path), &snapshot, rows, hunk, text.to_owned())
        .expect("comment added")
}

/// [`add`] from the button of hunk `index`.
fn add_on(
    h: &mut Harness,
    path: &str,
    rows: std::ops::Range<u32>,
    index: usize,
    text: &str,
) -> CommentId {
    let hunk = hunk_id(h, path, index);
    add(h, path, rows, Some(hunk), text)
}

fn rows(h: &Harness, id: CommentId) -> std::ops::Range<u32> {
    let buffers = &h.buffers;
    h.store
        .comments(|path| buffers.get(path).map(Buffer::snapshot))
        .into_iter()
        .find(|c| c.id == id)
        .expect("comment")
        .rows
}

/// Takes the comments like the host: the buffer's snapshot and whether it
/// differs from "disk".
fn take(h: &mut Harness) -> Vec<SentComment> {
    let buffers = &h.buffers;
    let disk = &h.disk;
    h.store.take_comments_for_prompt(|path: &Path| {
        buffers.get(path).map(|buffer| {
            let dirty = disk.get(path).is_none_or(|text| *text != buffer.text());
            (buffer.snapshot(), dirty)
        })
    })
}

fn restore(h: &mut Harness, sent: &[SentComment]) {
    let buffers = &h.buffers;
    h.store
        .restore_comments(sent, |path| buffers.get(path).map(Buffer::snapshot));
}

fn state(h: &Harness, id: CommentId) -> CommentState {
    h.store.comment_state(id).expect("unsent comment")
}

/// Decides every line of the hunks meeting `rows` of `path`, accepting the
/// first line and doing `rest` with the others.
fn decide_lines(
    h: &mut Harness,
    path: &str,
    rows: std::ops::Range<u32>,
    first_accept: bool,
    rest_accept: bool,
) {
    let mut first = true;
    loop {
        let found = {
            let file = h.store.file(&p(path));
            file.and_then(|file| {
                h.store
                    .hunks(&p(path))
                    .into_iter()
                    .find(|hunk| {
                        let r = file.buffer_rows(hunk);
                        let (a, b) = if r.is_empty() {
                            (r.start, r.start)
                        } else {
                            (r.start, r.end - 1)
                        };
                        a < rows.end && b >= rows.start
                    })
                    .map(|hunk| hunk.lines[0])
            })
        };
        let Some(pair) = found else { break };
        let accept = if first { first_accept } else { rest_accept };
        first = false;
        if accept {
            h.store.accept_line(&p(path), &pair).unwrap();
        } else {
            let revert = h.store.reject_line(&p(path), &pair).unwrap();
            h.apply(revert);
        }
        h.settle(path);
    }
}

// ------------------------------------------------------------------ anchors

#[test]
fn anchors_follow_edits_above_inside_and_below() {
    let mut h = Harness::new();
    h.open(UTIL, UTIL_TEXT);
    let id = add(&mut h, UTIL, 2..4, None, "nota");
    assert_eq!(rows(&h, id), 2..4);
    // Above: a new first line.
    h.user_edit(UTIL, 0..0, "# top\n");
    assert_eq!(rows(&h, id), 3..5);
    // Inside: a word in its first row, a new line in the middle.
    let start = snap(&h, UTIL).line_start_offset(3);
    h.user_edit(UTIL, start + 4..start + 4, "_NAME");
    assert_eq!(rows(&h, id), 3..5);
    let middle = snap(&h, UTIL).line_start_offset(4);
    h.user_edit(UTIL, middle..middle, "EXTRA = 0\n");
    assert_eq!(rows(&h, id), 3..6);
    // Below: nothing moves.
    let below = snap(&h, UTIL).line_start_offset(8);
    h.user_edit(UTIL, below..below, "# below\n");
    assert_eq!(rows(&h, id), 3..6);
    // The rows disappear: the comment stays on the nearest row.
    let s = snap(&h, UTIL);
    let range = s.line_start_offset(3)..s.line_start_offset(6);
    h.user_edit(UTIL, range, "");
    assert_eq!(rows(&h, id), 3..4);
    assert_eq!(h.text(UTIL).lines().nth(3), Some(""));
    // At the end of the file, collapsed onto the last row.
    let len = snap(&h, UTIL).len();
    let from = snap(&h, UTIL).line_start_offset(2);
    h.user_edit(UTIL, from..len, "");
    let count = snap(&h, UTIL).line_count();
    assert_eq!(rows(&h, id), count - 1..count);
}

#[test]
fn edit_remove_and_counts() {
    let mut h = Harness::new();
    h.store.set_workspace_root(Some(PathBuf::from(ROOT)));
    h.open(UTIL, UTIL_TEXT);
    h.open(CALC, CALC_BASE);
    let a = add(&mut h, UTIL, 6..7, None, "uno");
    let b = add(&mut h, CALC, 0..1, None, "dos");
    assert_eq!(h.store.comment_count(), 2);
    assert_eq!(h.store.comment_count_in(&p(UTIL)), 1);
    assert_eq!(h.store.commented_paths(), vec![p(CALC), p(UTIL)]);
    assert!(h.store.edit_comment(a, "uno, editado".into()));
    let views = h.store.comments(|_| None);
    assert_eq!(views[0].display_path, "src/calc.py");
    assert_eq!(views[1].text, "uno, editado");
    // Blank text: nothing added; an emptied edit removes the comment.
    let snapshot = snap(&h, UTIL);
    assert!(
        h.store
            .add_comment(&p(UTIL), &snapshot, 0..1, None, "  \n".into())
            .is_none()
    );
    assert!(h.store.edit_comment(b, " ".into()));
    assert_eq!(h.store.comment_count(), 1);
    assert!(!h.store.remove_comment(b));
    assert!(h.store.remove_comment(a));
    assert_eq!(h.store.comment_count(), 0);
    assert!(h.store.commented_paths().is_empty());
    // The text is capped.
    let long = "é".repeat(COMMENT_MAX_CHARS + 50);
    let c = add(&mut h, UTIL, 0..1, None, &long);
    let views = h.store.comments(|_| None);
    assert_eq!(views[0].id, c);
    assert_eq!(views[0].text.chars().count(), COMMENT_MAX_CHARS);
}

// ------------------------------------------------------------------ marks

/// The ten decisions mark the comment over the decided rows, and leave
/// alone the one over rows nobody decided.
#[test]
fn every_decision_marks_the_comments_it_cuts() {
    type Decide = fn(&mut Harness);
    let accept_hunk: Decide = |h| {
        let id = hunk_id(h, CALC, 1);
        h.store.accept_hunk(id).unwrap();
    };
    let reject_hunk: Decide = |h| {
        let id = hunk_id(h, CALC, 1);
        let revert = h.store.reject_hunk(id).unwrap();
        h.apply(revert);
    };
    let accept_line: Decide = |h| decide_lines(h, CALC, 6..7, true, true);
    let reject_line: Decide = |h| decide_lines(h, CALC, 6..7, false, false);
    let accept_file: Decide = |h| h.store.accept_file(&p(CALC)).unwrap();
    let reject_file: Decide = |h| {
        let revert = h.store.reject_file(&p(CALC)).unwrap();
        h.apply(revert);
    };
    let accept_turn: Decide = |h| {
        h.store.accept_turn(TurnId(1));
    };
    let reject_turn: Decide = |h| {
        let reverts = h.store.reject_turn(TurnId(1));
        h.apply_all(reverts);
    };
    let accept_all: Decide = |h| {
        h.store.accept_all();
    };
    let reject_all: Decide = |h| {
        let reverts = h.store.reject_all();
        h.apply_all(reverts);
    };
    let cases: [(&str, Decide, CommentState); 10] = [
        ("accept_hunk", accept_hunk, CommentState::Accepted),
        ("reject_hunk", reject_hunk, CommentState::Rejected),
        ("accept_line", accept_line, CommentState::Accepted),
        ("reject_line", reject_line, CommentState::Rejected),
        ("accept_file", accept_file, CommentState::Accepted),
        ("reject_file", reject_file, CommentState::Rejected),
        ("accept_turn", accept_turn, CommentState::Accepted),
        ("reject_turn", reject_turn, CommentState::Rejected),
        ("accept_all", accept_all, CommentState::Accepted),
        ("reject_all", reject_all, CommentState::Rejected),
    ];
    for (name, decide, expected) in cases {
        let mut h = calc_reviewed();
        let on_hunk = add_on(&mut h, CALC, 6..7, 1, "mean");
        let untouched = add(&mut h, CALC, 0..1, None, "import");
        assert_eq!(state(&h, on_hunk), CommentState::Pending, "{name}");
        decide(&mut h);
        h.settle_all();
        assert_eq!(state(&h, on_hunk), expected, "{name}");
        assert_eq!(state(&h, untouched), CommentState::NoAgentChange, "{name}");
        // The comment survives the decision, on its row.
        assert_eq!(h.store.comment_count(), 2, "{name}");
        assert_eq!(rows(&h, on_hunk), 6..7, "{name}");
    }
}

#[test]
fn undoing_a_reject_takes_its_marks_back() {
    let mut h = calc_reviewed();
    let id = add_on(&mut h, CALC, 2..4, 0, "total");
    let revert = h.store.reject_hunk(hunk_id(&h, CALC, 0)).unwrap();
    h.apply(revert);
    assert_eq!(state(&h, id), CommentState::Rejected);
    let reverts = h.store.undo_last_reject();
    h.apply_all(reverts);
    h.settle(CALC);
    assert_eq!(state(&h, id), CommentState::Pending);
    assert_eq!(rows(&h, id), 2..4);
    h.store.accept_hunk(hunk_id(&h, CALC, 0)).unwrap();
    assert_eq!(state(&h, id), CommentState::Accepted);
}

#[test]
fn a_mark_from_before_survives_the_undo_of_a_later_reject() {
    let mut h = calc_reviewed();
    // One comment over both hunks: reject the first, undo nothing yet.
    let id = add(&mut h, CALC, 2..7, None, "todo");
    let revert = h.store.reject_hunk(hunk_id(&h, CALC, 0)).unwrap();
    h.apply(revert);
    let revert = h.store.reject_hunk(hunk_id(&h, CALC, 0)).unwrap();
    h.apply(revert);
    assert_eq!(state(&h, id), CommentState::Rejected);
    // Undoing the second reject keeps the mark of the first.
    let reverts = h.store.undo_last_reject();
    h.apply_all(reverts);
    h.settle(CALC);
    h.store.accept_hunk(hunk_id(&h, CALC, 0)).unwrap();
    assert_eq!(state(&h, id), CommentState::Mixed);
}

// ------------------------------------------------------------------ scenarios

/// (a) A comment on a pending hunk goes alone (no patch), `not reviewed
/// yet`, with the agent's code; the next turn rewriting those lines keeps
/// the hunk against the original base and the other hunk identical.
#[test]
fn a_comment_on_pending_hunk_goes_alone_and_the_fix_updates_the_same_hunk() {
    let mut h = calc_reviewed();
    add_on(&mut h, CALC, 2..4, 0, "No cambies la firma.");
    let sent = take(&mut h);
    let report = h.store.report_for_agent(TurnId(1));
    assert_eq!(report, None);
    let expected = format!(
        "{INTRO_1}\nComment 1 of 1\nFile: src/calc.py\nLines: 3-4\nReview state: your change, not reviewed yet\nCurrent code:\n```py\ndef total(items, discount=0):\n    return sum(items) * (1 - discount)\n```\nComment:\nNo cambies la firma."
    );
    assert_eq!(format_feedback(report.as_deref(), &sent).unwrap(), expected);
    h.store.forget_turn(TurnId(1));

    let before = h.hunks(CALC);
    let base = h.base(CALC);
    h.turn = 2;
    h.store.begin_turn(TurnId(2));
    h.agent_write(
        CALC,
        "import math\n\ndef total(items):\n    return sum(items)  # no discount\n\ndef mean(items):\n    return sum(items) / len(items)\n",
    );
    h.settle(CALC);
    h.store.end_turn(TurnId(2));
    let after = h.hunks(CALC);
    assert_eq!(h.base(CALC), base, "the base is still the original one");
    assert_eq!(after.len(), 2);
    // Its red rows are those of the original base, not the first turn's.
    assert_eq!(
        &base[after[0].base_byte_range.clone()],
        "    return sum(items)\n"
    );
    assert_eq!(after[1].base_rows, before[1].base_rows);
    assert_eq!(after[1].base_byte_range, before[1].base_byte_range);
    assert_eq!(rows_of_hunks(&h, CALC), vec![3..4, 6..7]);
    assert_eq!(h.store.comment_count(), 0);
    assert!(take(&mut h).is_empty());
}

/// (b) Comment and reject: the patch of the reject and the note, `rejected`,
/// with the restored code. This is the example of §6.8, byte for byte.
#[test]
fn b_comment_and_reject_sends_patch_and_note_like_the_spec_example() {
    let mut h = Harness::new();
    h.store.set_workspace_root(Some(PathBuf::from(ROOT)));
    let mut base = String::new();
    for line in 1..=23 {
        base.push_str(&format!("# line {line}\n"));
    }
    let tail = "\ndef mean(items):\n    return total(items) / len(items)\n";
    let agent = format!(
        "{base}def total(items, discount=0):\n    return sum(items) * (1 - discount)\n{tail}"
    );
    let base = format!("{base}def total(items):\n    return sum(items)\n{tail}");
    h.start(CALC, &base);
    h.agent_write(CALC, &agent);
    h.settle(CALC);
    h.store.end_turn(TurnId(1));
    h.open(UTIL, UTIL_TEXT);

    let hunk = hunk_id(&h, CALC, 0);
    add(
        &mut h,
        CALC,
        23..25,
        Some(hunk),
        "No cambies la firma: agregá el descuento como una función aparte.",
    );
    add(
        &mut h,
        UTIL,
        6..7,
        None,
        "Esto debería salir de la configuración.",
    );
    let revert = h.store.reject_hunk(hunk).unwrap();
    h.apply(revert);

    let sent = take(&mut h);
    let report = h
        .store
        .report_for_agent(TurnId(1))
        .expect("the reject is reported");
    assert!(report.starts_with(REPORT_HEADER));
    assert!(report.contains(
        "-def total(items, discount=0):\n-    return sum(items) * (1 - discount)\n+def total(items):\n+    return sum(items)\n"
    ));
    let section = format!(
        "{INTRO_2}
Comment 1 of 2
File: src/calc.py
Lines: 24-25
Review state: your change, rejected by the user
Current code:
```py
def total(items):
    return sum(items)
```
Comment:
No cambies la firma: agregá el descuento como una función aparte.

Comment 2 of 2
File: src/util.py
Line: 7
Review state: no change of yours (the user selected these lines)
Current code:
```py
TIMEOUT = 3
```
Comment:
Esto debería salir de la configuración."
    );
    let feedback = format_feedback(Some(&report), &sent).unwrap();
    assert_eq!(feedback, format!("{report}\n{section}"));
    assert_eq!(sent[0].kind, SentRange::Lines);
    assert_eq!((sent[0].first_line, sent[0].last_line), (24, 25));
    assert_eq!(sent[1].state, CommentState::NoAgentChange);
}

/// (c) Comment and accept: only the note travels (no `REPORT_HEADER`).
#[test]
fn c_comment_and_accept_sends_only_the_note() {
    let mut h = calc_reviewed();
    let hunk = hunk_id(&h, CALC, 0);
    add(&mut h, CALC, 2..4, Some(hunk), "Bien, pero documentalo.");
    h.store.accept_hunk(hunk).unwrap();
    let sent = take(&mut h);
    let report = h.store.report_for_agent(TurnId(1));
    assert_eq!(report, None);
    let feedback = format_feedback(report.as_deref(), &sent).unwrap();
    assert!(!feedback.contains(REPORT_HEADER));
    assert_eq!(
        feedback,
        format!(
            "{INTRO_1}\nComment 1 of 1\nFile: src/calc.py\nLines: 3-4\nReview state: your change, accepted by the user\nCurrent code:\n```py\ndef total(items, discount=0):\n    return sum(items) * (1 - discount)\n```\nComment:\nBien, pero documentalo."
        )
    );
}

/// (d) Comment and a manual edit of those lines: the patch of the edit and
/// the note with the edited code, `not reviewed yet`, marked as not saved.
#[test]
fn d_comment_and_manual_edit_sends_patch_and_current_code() {
    let mut h = calc_reviewed();
    add_on(&mut h, CALC, 2..4, 0, "Revisá esto.");
    let row = snap(&h, CALC).line_start_offset(3);
    let at = row + "    return sum(items)".len();
    h.user_edit(CALC, at..at, " + 0");
    let sent = take(&mut h);
    let report = h
        .store
        .report_for_agent(TurnId(1))
        .expect("the edit is reported");
    assert!(report.contains("+    return sum(items) + 0 * (1 - discount)\n"));
    let feedback = format_feedback(Some(&report), &sent).unwrap();
    assert_eq!(
        feedback,
        format!(
            "{report}\n{INTRO_1}\nComment 1 of 1\nFile: src/calc.py\nLines: 3-4\nReview state: your change, not reviewed yet\nCurrent code (as shown in the editor, not saved yet):\n```py\ndef total(items, discount=0):\n    return sum(items) + 0 * (1 - discount)\n```\nComment:\nRevisá esto."
        )
    );
    // Saved, the plain variant.
    let mut h = calc_reviewed();
    add(&mut h, CALC, 2..4, None, "Revisá esto.");
    h.user_edit(CALC, at..at, " + 0");
    let text = h.text(CALC);
    h.disk.insert(p(CALC), text);
    let sent = take(&mut h);
    assert!(!sent[0].unsaved);
    assert_eq!(sent[0].state, CommentState::Pending);
}

/// (e) A selection the agent did not touch: `no change of yours`, no patch.
#[test]
fn e_comment_on_untouched_selection() {
    let mut h = calc_reviewed();
    h.open(UTIL, UTIL_TEXT);
    add(&mut h, CALC, 0..1, None, "¿Hace falta?");
    add(&mut h, UTIL, 2..4, None, "Juntalos.");
    let sent = take(&mut h);
    assert!(sent.iter().all(|c| c.state == CommentState::NoAgentChange));
    assert_eq!(h.store.report_for_agent(TurnId(1)), None);
    let feedback = format_feedback(None, &sent).unwrap();
    assert_eq!(
        feedback,
        format!(
            "{INTRO_2}\nComment 1 of 2\nFile: src/calc.py\nLine: 1\nReview state: no change of yours (the user selected these lines)\nCurrent code:\n```py\nimport math\n```\nComment:\n¿Hace falta?\n\nComment 2 of 2\nFile: src/util.py\nLines: 3-4\nReview state: no change of yours (the user selected these lines)\nCurrent code:\n```py\nHOST = 'x'\nPORT = 1\n```\nComment:\nJuntalos."
        )
    );
}

/// (f) Several comments in several files: sorted by path (bytes) and line,
/// numbered in that order; the views use the same order.
#[test]
fn f_several_comments_are_sorted_by_file_and_line() {
    let mut h = Harness::new();
    h.store.set_workspace_root(Some(PathBuf::from(ROOT)));
    h.open(UTIL, UTIL_TEXT);
    h.open(CALC, CALC_BASE);
    h.open("/proj/README.md", "# x\n\ny\n");
    h.open("/proj/src/a/b.rs", "fn a() {}\nfn b() {}\n");
    add(&mut h, UTIL, 6..7, None, "u7");
    add(&mut h, CALC, 5..7, None, "c6");
    add(&mut h, UTIL, 0..1, None, "u1");
    add(&mut h, "/proj/src/a/b.rs", 1..2, None, "b2");
    add(&mut h, "/proj/README.md", 2..3, None, "r3");
    add(&mut h, CALC, 0..1, None, "c1");
    let order = ["r3", "b2", "c1", "c6", "u1", "u7"];
    let views: Vec<String> = h
        .store
        .comments(|_| None)
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(views, order);
    let sent = take(&mut h);
    let texts: Vec<&str> = sent.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts, order);
    let feedback = format_feedback(None, &sent).unwrap();
    assert!(feedback.starts_with("The user left 6 comments on the code."));
    let mut at = 0;
    for (i, name) in [
        "README.md",
        "src/a/b.rs",
        "src/calc.py",
        "src/calc.py",
        "src/util.py",
        "src/util.py",
    ]
    .iter()
    .enumerate()
    {
        let header = format!("Comment {} of 6\nFile: {name}\n", i + 1);
        let found = feedback[at..].find(&header).expect(&header);
        at += found + header.len();
    }
    assert!(feedback.contains("File: README.md\nLine: 3\n"));
    assert!(feedback.contains("```md\ny\n```"));
    assert!(feedback.contains("```rs\nfn b() {}\n```"));
}

/// (g) Decide after commenting, including "accept all" and "reject all":
/// the state follows the last decision, the comment stays on its rows.
#[test]
fn g_decide_after_commenting_including_accept_all_and_reject_all() {
    let mut h = calc_reviewed();
    let id = add_on(&mut h, CALC, 2..4, 0, "x");
    h.store.accept_all();
    assert_eq!(state(&h, id), CommentState::Accepted);
    assert_eq!(rows(&h, id), 2..4);
    let sent = take(&mut h);
    assert_eq!(sent[0].state, CommentState::Accepted);
    assert_eq!(h.store.report_for_agent(TurnId(1)), None);

    let mut h = calc_reviewed();
    let id = add_on(&mut h, CALC, 2..4, 0, "x");
    let reverts = h.store.reject_all();
    h.apply_all(reverts);
    assert_eq!(state(&h, id), CommentState::Rejected);
    assert_eq!(rows(&h, id), 2..4);
    assert_eq!(h.store.comment_count(), 1);
    let sent = take(&mut h);
    assert_eq!(
        sent[0].code.as_deref(),
        Some("def total(items):\n    return sum(items)")
    );
    let report = h
        .store
        .report_for_agent(TurnId(1))
        .expect("patch of the reject");
    let feedback = format_feedback(Some(&report), &sent).unwrap();
    assert!(feedback.starts_with(&report));
    assert!(feedback.contains("Review state: your change, rejected by the user\n"));
}

/// (h) A deleted comment sends nothing; with nothing else there is no block.
#[test]
fn h_deleted_comment_sends_nothing() {
    let mut h = calc_reviewed();
    let id = add_on(&mut h, CALC, 2..4, 0, "x");
    assert!(h.store.remove_comment(id));
    let sent = take(&mut h);
    assert!(sent.is_empty());
    assert_eq!(
        format_feedback(h.store.report_for_agent(TurnId(1)).as_deref(), &sent),
        None
    );
}

/// (i) Line decisions on a commented hunk: pending lines left in the range
/// keep it `not reviewed yet`; all decided gives accepted, rejected or
/// mixed.
#[test]
fn i_line_decisions_on_a_commented_hunk() {
    // One line accepted, the rest pending.
    let mut h = calc_reviewed();
    let id = add_on(&mut h, CALC, 2..4, 0, "x");
    let pair = h.line(CALC, 0, 0);
    h.store.accept_line(&p(CALC), &pair).unwrap();
    h.settle(CALC);
    assert_eq!(state(&h, id), CommentState::Pending);
    assert!(h.store.comments(|_| None)[0].from_hunk.is_some());
    // The others rejected: mixed.
    decide_lines(&mut h, CALC, 2..4, false, false);
    assert_eq!(state(&h, id), CommentState::Mixed);
    let sent = take(&mut h);
    let feedback = format_feedback(None, &sent).unwrap();
    assert!(feedback.contains(
        "Review state: your change, partly accepted and partly rejected by the user, line by line\n"
    ));

    for (accept, expected) in [
        (true, CommentState::Accepted),
        (false, CommentState::Rejected),
    ] {
        let mut h = calc_reviewed();
        let id = add(&mut h, CALC, 2..4, None, "x");
        decide_lines(&mut h, CALC, 2..4, accept, accept);
        assert_eq!(state(&h, id), expected);
        assert!(
            h.store.file(&p(CALC)).is_some(),
            "the other hunk is still pending"
        );
    }
}

/// (j) After sending: nothing left (views, counts, next take) and nothing
/// repeats.
#[test]
fn j_sent_comments_disappear_and_never_repeat() {
    let mut h = calc_reviewed();
    add(&mut h, CALC, 2..4, None, "x");
    let sent = take(&mut h);
    assert_eq!(sent.len(), 1);
    assert_eq!(h.store.comment_count(), 0);
    assert_eq!(h.store.comment_count_in(&p(CALC)), 0);
    assert!(h.store.comments(|_| None).is_empty());
    assert!(h.store.commented_paths().is_empty());
    assert!(take(&mut h).is_empty());
    assert_eq!(format_feedback(None, &take(&mut h)), None);
}

/// (k) Comments belong to the project, not to a turn or a connection: they
/// wait through new turns and go with the next message.
#[test]
fn k_comments_wait_through_turns() {
    let mut h = calc_reviewed();
    let id = add(&mut h, CALC, 0..1, None, "x");
    h.store.forget_turn(TurnId(1));
    h.turn = 2;
    h.store.begin_turn(TurnId(2));
    h.agent_write(CALC, &CALC_AGENT.replace("import math", "import math, os"));
    h.store.end_turn(TurnId(2));
    assert_eq!(rows(&h, id), 0..1);
    let sent = take(&mut h);
    assert_eq!(sent[0].code.as_deref(), Some("import math, os"));
    assert_eq!(sent[0].state, CommentState::Pending);
}

/// (m) Binary files take no comments; a commented file that turns binary
/// loses them.
#[test]
fn m_binaries_take_no_comments() {
    let mut h = Harness::new();
    h.open("/proj/img.bin", "PNG\0\0data");
    let snapshot = snap(&h, "/proj/img.bin");
    assert!(
        h.store
            .add_comment(&p("/proj/img.bin"), &snapshot, 0..1, None, "x".into())
            .is_none()
    );
    h.open(UTIL, UTIL_TEXT);
    add(&mut h, UTIL, 0..1, None, "x");
    add(&mut h, UTIL, 2..3, None, "y");
    assert_eq!(h.store.drop_comments_in(&p(UTIL)), 2);
    assert_eq!(h.store.comment_count(), 0);
    assert!(take(&mut h).is_empty());
}

// ------------------------------------------------------------------ take / restore

#[test]
fn restore_gives_back_what_did_not_go_out() {
    let mut h = calc_reviewed();
    let hunk = hunk_id(&h, CALC, 0);
    let id = add(&mut h, CALC, 2..4, Some(hunk), "x");
    let revert = h.store.reject_hunk(hunk).unwrap();
    h.apply(revert);
    let sent = take(&mut h);
    assert_eq!(h.store.comment_count(), 0);
    // The user types above while the prompt waits: the comment follows.
    h.user_edit(CALC, 0..0, "# new\n");
    restore(&mut h, &sent);
    assert_eq!(h.store.comment_count(), 1);
    assert_eq!(rows(&h, id), 3..5);
    assert_eq!(state(&h, id), CommentState::Rejected);
    // Restoring twice does not duplicate.
    restore(&mut h, &sent);
    assert_eq!(h.store.comment_count(), 1);
    let again = take(&mut h);
    assert_eq!(again[0].id, id);
    assert_eq!(again[0].first_line, 4);
}

#[test]
fn restore_rebuilds_comments_no_longer_kept() {
    let mut h = calc_reviewed();
    let hunk = hunk_id(&h, CALC, 1);
    add(&mut h, CALC, 6..7, Some(hunk), "x");
    h.store.accept_hunk(hunk).unwrap();
    let sent = take(&mut h);
    // A second take drops what the first one kept aside.
    assert!(take(&mut h).is_empty());
    restore(&mut h, &sent);
    let views = h.store.comments(|_| None);
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].rows, 6..7);
    assert_eq!(views[0].id, sent[0].id);
    assert_eq!(
        h.store.comment_state(views[0].id),
        Some(CommentState::Accepted)
    );
}

// ------------------------------------------------------------------ kinds

/// A pure deletion: the comment has no rows; it goes as "removed before
/// line" with the removed lines while pending, and covers the lines once a
/// reject brings them back.
#[test]
fn pure_deletion_and_its_reject() {
    let mut h = Harness::new();
    h.store.set_workspace_root(Some(PathBuf::from(ROOT)));
    h.start(UTIL, "a = 1\nb = 2\nc = 3\nd = 4\n");
    h.agent_write(UTIL, "a = 1\nd = 4\n");
    h.settle(UTIL);
    h.store.end_turn(TurnId(1));
    assert_eq!(rows_of_hunks(&h, UTIL), vec![1..1]);
    let hunk = hunk_id(&h, UTIL, 0);
    let id = add(&mut h, UTIL, 1..1, Some(hunk), "¿Por qué los quitaste?");
    assert_eq!(rows(&h, id), 1..1);
    assert_eq!(h.store.comments(|_| None)[0].from_hunk, Some(hunk));
    assert_eq!(state(&h, id), CommentState::Pending);

    let sent = take(&mut h);
    assert_eq!(sent[0].kind, SentRange::RemovedBefore);
    assert_eq!(
        format_feedback(None, &sent).unwrap(),
        format!(
            "{INTRO_1}\nComment 1 of 1\nFile: src/util.py\nLines: removed before line 2\nReview state: your change, not reviewed yet\nLines you removed (not reviewed yet):\n```py\nb = 2\nc = 3\n```\nComment:\n¿Por qué los quitaste?"
        )
    );
    restore(&mut h, &sent);

    let revert = h.store.reject_hunk(hunk).unwrap();
    h.apply(revert);
    assert_eq!(h.text(UTIL), "a = 1\nb = 2\nc = 3\nd = 4\n");
    assert_eq!(rows(&h, id), 1..3);
    let sent = take(&mut h);
    assert_eq!(
        format_feedback(None, &sent).unwrap(),
        format!(
            "{INTRO_1}\nComment 1 of 1\nFile: src/util.py\nLines: 2-3\nReview state: your change, rejected by the user\nCurrent code:\n```py\nb = 2\nc = 3\n```\nComment:\n¿Por qué los quitaste?"
        )
    );
}

#[test]
fn accepted_pure_deletion_has_no_code() {
    let mut h = Harness::new();
    h.store.set_workspace_root(Some(PathBuf::from(ROOT)));
    h.start(UTIL, "a = 1\nb = 2\nd = 4\n");
    h.agent_write(UTIL, "a = 1\nd = 4\n");
    h.settle(UTIL);
    h.store.end_turn(TurnId(1));
    let hunk = hunk_id(&h, UTIL, 0);
    add(&mut h, UTIL, 1..1, Some(hunk), "ok");
    h.store.accept_hunk(hunk).unwrap();
    let sent = take(&mut h);
    assert_eq!(
        format_feedback(None, &sent).unwrap(),
        format!(
            "{INTRO_1}\nComment 1 of 1\nFile: src/util.py\nLines: removed before line 2\nReview state: your change, accepted by the user\nComment:\nok"
        )
    );
}

/// A file the agent deleted, commented on its read-only tab.
#[test]
fn deleted_file() {
    let mut h = Harness::new();
    h.store.set_workspace_root(Some(PathBuf::from(ROOT)));
    let old = "/proj/old.py";
    let previous = "def legacy():\n    pass\n\nX = 1\n";
    h.store.begin_turn(TurnId(1));
    h.store.file_deleted(&p(old), Rope::from_str(previous));
    h.store.end_turn(TurnId(1));
    // The deleted tab: a buffer with the deleted text.
    h.open(old, previous);
    h.disk.remove(&p(old));
    let id = add(
        &mut h,
        old,
        0..2,
        Some(HunkId(u64::MAX)),
        "Esta función todavía se usa.",
    );
    assert_eq!(state(&h, id), CommentState::Pending);
    let sent = take(&mut h);
    let pending = format!(
        "{INTRO_1}\nComment 1 of 1\nFile: old.py (you deleted this file)\nLines: 1-2\nReview state: your change, not reviewed yet\nLines you removed (not reviewed yet):\n```py\ndef legacy():\n    pass\n```\nComment:\nEsta función todavía se usa."
    );
    assert_eq!(format_feedback(None, &sent).unwrap(), pending);
    restore(&mut h, &sent);

    h.store.accept_file(&p(old)).unwrap();
    let sent = take(&mut h);
    assert_eq!(sent[0].kind, SentRange::DeletedFile);
    assert_eq!(
        format_feedback(None, &sent).unwrap(),
        format!(
            "{INTRO_1}\nComment 1 of 1\nFile: old.py (you deleted this file)\nLines: 1-2\nReview state: your change, accepted by the user\nComment:\nEsta función todavía se usa."
        )
    );
}

// ------------------------------------------------------------------ snippet and format

#[test]
fn snippet_is_capped_by_lines_and_bytes() {
    let mut h = Harness::new();
    let mut text = String::new();
    for i in 0..200 {
        text.push_str(&format!("line {i}\n"));
    }
    h.open("/proj/long.txt", &text);
    add(&mut h, "/proj/long.txt", 0..200, None, "largo");
    let sent = take(&mut h);
    assert_eq!(sent[0].code.as_ref().unwrap().lines().count(), 120);
    assert_eq!(sent[0].truncated_lines, 80);
    assert_eq!(sent[0].lang.as_deref(), Some("txt"));
    let feedback = format_feedback(None, &sent).unwrap();
    assert!(feedback.contains("line 119\n```\n(80 more lines not shown)\nComment:\nlargo"));

    let mut h = Harness::new();
    let wide = format!("{}\n", "x".repeat(1000)).repeat(20);
    h.open("/proj/Wide", &wide);
    add(&mut h, "/proj/Wide", 0..20, None, "ancho");
    let sent = take(&mut h);
    // 12 lines of 1000 bytes plus 11 newlines fit in 12 KB, a 13th does not.
    assert_eq!(sent[0].code.as_ref().unwrap().lines().count(), 12);
    assert_eq!(sent[0].truncated_lines, 8);
    assert_eq!(sent[0].lang, None);

    let mut h = Harness::new();
    h.open("/proj/one.js", &format!("{}\n", "é".repeat(10_000)));
    add(&mut h, "/proj/one.js", 0..1, None, "una");
    let sent = take(&mut h);
    let code = sent[0].code.as_ref().unwrap();
    assert!(code.len() <= 12 * 1024 && code.len() > 12 * 1024 - 4);
    assert_eq!(sent[0].truncated_lines, 0);
}

#[test]
fn fences_outgrow_backticks_in_the_code() {
    assert_eq!(fence_for("plain"), "```");
    assert_eq!(fence_for("a `b` c"), "```");
    assert_eq!(fence_for("```rust\nx\n```"), "````");
    assert_eq!(fence_for("`````"), "``````");
    let mut h = Harness::new();
    h.store.set_workspace_root(Some(PathBuf::from(ROOT)));
    h.open("/proj/README.MD", "# Título\n```sh\nmake\n```\n");
    add(
        &mut h,
        "/proj/README.MD",
        1..4,
        None,
        "Explicá el comando.\nCon un ejemplo.\n",
    );
    let sent = take(&mut h);
    assert_eq!(
        format_feedback(None, &sent).unwrap(),
        format!(
            "{INTRO_1}\nComment 1 of 1\nFile: README.MD\nLines: 2-4\nReview state: no change of yours (the user selected these lines)\nCurrent code:\n````md\n```sh\nmake\n```\n````\nComment:\nExplicá el comando.\nCon un ejemplo."
        )
    );
}

#[test]
fn format_feedback_without_comments_is_the_report() {
    assert_eq!(format_feedback(None, &[]), None);
    assert_eq!(format_feedback(Some(""), &[]), None);
    let mut h = calc_reviewed();
    let revert = h.store.reject_hunk(hunk_id(&h, CALC, 1)).unwrap();
    h.apply(revert);
    let report = h.store.report_for_agent(TurnId(1)).unwrap();
    assert_eq!(
        format_feedback(Some(&report), &[]).as_deref(),
        Some(report.as_str())
    );
    assert_eq!(
        format_feedback(Some(&report), &take(&mut h)).as_deref(),
        Some(report.as_str())
    );
}

#[test]
fn states_have_the_agent_texts_of_the_spec() {
    assert_eq!(
        CommentState::Pending.agent_text(),
        "your change, not reviewed yet"
    );
    assert_eq!(
        CommentState::Accepted.agent_text(),
        "your change, accepted by the user"
    );
    assert_eq!(
        CommentState::Rejected.agent_text(),
        "your change, rejected by the user"
    );
    assert_eq!(
        CommentState::Mixed.agent_text(),
        "your change, partly accepted and partly rejected by the user, line by line"
    );
    assert_eq!(
        CommentState::NoAgentChange.agent_text(),
        "no change of yours (the user selected these lines)"
    );
}

#[test]
fn from_hunk_follows_the_pending_hunk() {
    let mut h = calc_reviewed();
    let hunk = hunk_id(&h, CALC, 0);
    add(&mut h, CALC, 2..4, Some(hunk), "x");
    add(&mut h, CALC, 0..1, None, "y");
    let views = h.store.comments(|_| None);
    assert_eq!(views[0].from_hunk, None);
    assert_eq!(views[1].from_hunk, Some(hunk));
    // The user edits the hunk: it may get a new shape, the comment follows.
    let at = snap(&h, CALC).line_start_offset(2);
    h.user_edit(CALC, at..at, "@decorator\n");
    h.settle(CALC);
    let views = h.store.comments(|_| None);
    let followed = views[1].from_hunk.expect("still from a hunk");
    assert!(h.hunks(CALC).iter().any(|hunk| hunk.id == followed));
}
