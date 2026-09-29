//! Host protocol, navigation, limits, turns and recompute behaviour.

mod common;

use cincel_review::diff::{MAX_INLINE_BYTES, MAX_INLINE_LINES};
use cincel_review::{FileOp, FileStatus, HunkKind, RecomputeOutcome, Revert, ReviewStore, TurnId};
use cincel_text::{Buffer, EditSource, Rope};
use common::{Harness, p};

const TEN: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";

#[test]
fn crlf_base_matches_lf_buffer() {
    let mut store = ReviewStore::new();
    store.capture_base_text(&p("a.txt"), "one\r\ntwo\r\n");
    let buffer = Buffer::new("one\ntwo\n");
    assert_eq!(
        store.recompute(&p("a.txt"), &buffer.snapshot()),
        RecomputeOutcome::Resolved
    );
    assert_eq!(store.pending_count(), 0);

    let mut store = ReviewStore::new();
    store
        .capture_base_bytes(&p("b.txt"), b"\xEF\xBB\xBFx\r\ny\r\n")
        .unwrap();
    let buffer = Buffer::new("x\nY\n");
    store.recompute(&p("b.txt"), &buffer.snapshot());
    let hunks = store.hunks(&p("b.txt"));
    assert_eq!(hunks.len(), 1);
    assert_eq!(
        hunks[0].base_rows,
        1..2,
        "only the real change, no CRLF/BOM ghost hunk"
    );
    assert!(store.capture_base_bytes(&p("c.txt"), b"\xff\xfe").is_err());
}

#[test]
fn whitespace_changes_are_hunks() {
    let mut h = Harness::new();
    h.start("a.txt", "a b\n\tc\n");
    h.agent_write("a.txt", "a  b\n    c\n");
    h.settle("a.txt");
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].kind, HunkKind::Modified);
    let words = hunks[0]
        .word_diffs
        .as_ref()
        .expect("short hunk has word diffs");
    assert!(!words.base.is_empty() && !words.buffer.is_empty());
}

#[test]
fn word_diffs_only_for_short_hunks() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    h.settle("a.txt");
    let short = &h.hunks("a.txt")[0];
    let words = short.word_diffs.as_ref().unwrap();
    assert_eq!(words.base, vec![0..3]);
    assert_eq!(words.buffer, vec![0..3]);

    let mut h = Harness::new();
    h.start("b.txt", TEN);
    h.agent_write(
        "b.txt",
        "ONE\nTWO\nTHREE\nFOUR\nFIVE\nSIX\nseven\neight\nnine\nten\n",
    );
    h.settle("b.txt");
    let long = &h.hunks("b.txt")[0];
    assert!(long.word_diffs.is_none());
    assert_eq!(long.lines.len(), 6, "index pairing");
    assert!(
        long.lines
            .iter()
            .all(|l| l.base_row.is_some() && l.buffer_row.is_some())
    );
}

#[test]
fn added_and_deleted_hunks_split_into_one_sided_lines() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "one\nnew a\nnew b\ntwo\nthree\nfour\nfive\nsix\nseven\nten\n",
    );
    h.settle("a.txt");
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 2);
    assert_eq!(hunks[0].kind, HunkKind::Added);
    assert!(hunks[0].lines.iter().all(|l| l.base_row.is_none()));
    assert_eq!(hunks[1].kind, HunkKind::Deleted);
    assert_eq!(hunks[1].base_rows, 7..9);
    assert!(hunks[1].lines.iter().all(|l| l.buffer_row.is_none()));
    // Accept one added line and reject one deleted line.
    let added = hunks[0].lines[1];
    h.store.accept_line(&p("a.txt"), &added).unwrap();
    assert!(h.base("a.txt").starts_with("one\nnew b\ntwo\n"));
    let deleted = h.hunks("a.txt")[1].lines[0];
    let revert = h.store.reject_line(&p("a.txt"), &deleted).unwrap();
    h.apply(revert);
    assert!(h.text("a.txt").ends_with("seven\neight\nten\n"));
    h.check("a.txt");
    assert_eq!(h.store.stats(&p("a.txt")), (1, 1));
}

#[test]
fn stale_line_pairs_are_rejected() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    h.settle("a.txt");
    let pair = h.line("a.txt", 0, 0);
    h.user_edit("a.txt", 0..0, "zero\n");
    assert!(matches!(
        h.store.accept_line(&p("a.txt"), &pair),
        Err(cincel_review::ReviewError::UnknownLine(_))
    ));
    let fresh = h.line("a.txt", 0, 0);
    h.store.accept_line(&p("a.txt"), &fresh).unwrap();
    assert!(h.store.hunks(&p("a.txt")).is_empty());
}

#[test]
fn next_and_prev_wrap_across_files() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
    );
    h.open("b.txt", TEN);
    h.capture("b.txt");
    h.agent_write(
        "b.txt",
        "one\ntwo\nthree\nfour\nFIVE\nsix\nseven\neight\nnine\nten\n",
    );
    h.store.file_deleted(&p("c.txt"), Rope::from_str("c\n"));
    h.settle_all();
    let stops: Vec<_> = h
        .store
        .locations()
        .iter()
        .map(|l| (l.path.clone(), l.row))
        .collect();
    assert_eq!(
        stops,
        vec![
            (p("a.txt"), 0),
            (p("a.txt"), 9),
            (p("b.txt"), 4),
            (p("c.txt"), 0)
        ]
    );
    let next = |path: &str, row| h.store.next_hunk((&p(path), row)).map(|l| (l.path, l.row));
    assert_eq!(next("a.txt", 0), Some((p("a.txt"), 9)));
    assert_eq!(next("a.txt", 9), Some((p("b.txt"), 4)));
    assert_eq!(next("b.txt", 4), Some((p("c.txt"), 0)));
    assert_eq!(
        next("c.txt", 0),
        Some((p("a.txt"), 0)),
        "wraps to the first file"
    );
    let prev = |path: &str, row| h.store.prev_hunk((&p(path), row)).map(|l| (l.path, l.row));
    assert_eq!(
        prev("a.txt", 0),
        Some((p("c.txt"), 0)),
        "wraps to the last file"
    );
    assert_eq!(prev("b.txt", 0), Some((p("a.txt"), 9)));
    assert_eq!(prev("a.txt", 5), Some((p("a.txt"), 0)));
    assert!(h.store.locations()[3].hunk.is_none());
    assert_eq!(h.store.pending_count(), 4);
    assert_eq!(
        h.store.pending_files(),
        vec![p("a.txt"), p("b.txt"), p("c.txt")]
    );
    assert_eq!(ReviewStore::new().next_hunk((&p("x"), 0)), None);
}

#[test]
fn too_large_files_are_file_level_only() {
    let big: String = "x\n".repeat(MAX_INLINE_LINES + 10);
    let mut h = Harness::new();
    h.start("big.txt", &big);
    let changed = format!("changed\n{big}");
    h.agent_write("big.txt", &changed);
    h.settle("big.txt");
    let file = h.store.file(&p("big.txt")).unwrap();
    assert!(file.too_large);
    assert!(file.hunks.is_empty());
    assert_eq!(h.store.stats(&p("big.txt")), (1, 0));
    assert_eq!(h.store.pending_count(), 1);
    let locations = h.store.locations();
    assert_eq!(locations.len(), 1);
    assert!(locations[0].hunk.is_none());
    let revert = h.store.reject_file(&p("big.txt")).unwrap();
    h.apply(revert);
    assert_eq!(h.text("big.txt"), big);
    assert_eq!(h.store.pending_count(), 0);

    let mut h = Harness::new();
    h.start("huge.txt", "small\n");
    let huge = "y".repeat(MAX_INLINE_BYTES + 1);
    h.agent_write_batched("huge.txt", &huge);
    h.settle("huge.txt");
    assert!(h.store.file(&p("huge.txt")).unwrap().too_large);
    h.store.accept_file(&p("huge.txt")).unwrap();
    h.store.end_turn(TurnId(1));
    assert!(h.store.file(&p("huge.txt")).is_none());
}

#[test]
fn binary_files_are_written_back_on_reject() {
    let mut h = Harness::new();
    h.start("bin.dat", "ab\0cd\n");
    h.agent_write("bin.dat", "ab\0CD\n");
    h.settle("bin.dat");
    let file = h.store.file(&p("bin.dat")).unwrap();
    assert!(file.binary);
    assert!(file.hunks.is_empty());
    assert_eq!(h.store.stats(&p("bin.dat")), (0, 0));
    assert!(
        h.store
            .accept_line(
                &p("bin.dat"),
                &cincel_review::LinePair {
                    base_row: Some(0),
                    buffer_row: None
                }
            )
            .is_err()
    );
    let revert = h.store.reject_file(&p("bin.dat")).unwrap();
    assert_eq!(
        revert,
        Revert::File(FileOp::WriteFile(p("bin.dat"), "ab\0cd\n".into()))
    );
    assert!(h.store.file(&p("bin.dat")).is_none());
}

#[test]
fn deleted_files_restore_or_stay_deleted() {
    let mut store = ReviewStore::new();
    store.begin_turn(TurnId(3));
    store.file_deleted(&p("d.txt"), Rope::from_str("keep me\r\n"));
    let file = store.file(&p("d.txt")).unwrap();
    assert!(matches!(file.status, FileStatus::Deleted { .. }));
    assert_eq!(file.turn_id, TurnId(3));
    assert_eq!(store.stats(&p("d.txt")), (0, 1));
    let revert = store.reject_file(&p("d.txt")).unwrap();
    assert_eq!(
        revert,
        Revert::File(FileOp::WriteFile(p("d.txt"), "keep me\n".into()))
    );
    // Undo the restore: deleted again, pending again.
    let undo = store.undo_last_reject();
    assert_eq!(undo, vec![Revert::File(FileOp::DeleteFile(p("d.txt")))]);
    assert_eq!(store.pending_count(), 1);
    store.accept_file(&p("d.txt")).unwrap();
    assert_eq!(store.pending_count(), 0);
}

#[test]
fn created_then_deleted_by_the_agent_leaves_nothing() {
    let mut store = ReviewStore::new();
    store.begin_turn(TurnId(1));
    store.file_created(&p("tmp.txt"), "scratch\n", None);
    assert_eq!(store.pending_count(), 1);
    let tracked = store.file_deleted(&p("tmp.txt"), Rope::from_str("scratch\n"));
    assert!(!tracked.tracked);
    assert_eq!(store.pending_count(), 0);
}

#[test]
fn modified_then_deleted_restores_the_base() {
    let mut h = Harness::new();
    h.start("a.txt", "original\n");
    h.agent_write("a.txt", "agent text\n");
    h.store
        .file_deleted(&p("a.txt"), Rope::from_str("agent text\n"));
    let revert = h.store.reject_file(&p("a.txt")).unwrap();
    assert_eq!(
        revert,
        Revert::File(FileOp::WriteFile(p("a.txt"), "original\n".into()))
    );
}

#[test]
fn recreated_file_after_delete_is_modified_against_the_old_base() {
    let mut store = ReviewStore::new();
    store.begin_turn(TurnId(1));
    store.file_deleted(&p("a.txt"), Rope::from_str("one\ntwo\n"));
    store.file_created(&p("a.txt"), "one\nTWO\n", None);
    let file = store.file(&p("a.txt")).unwrap();
    assert_eq!(file.status, FileStatus::Modified);
    assert_eq!(store.hunks(&p("a.txt")).len(), 1);
    assert_eq!(store.hunks(&p("a.txt"))[0].base_rows, 1..2);
}

#[test]
fn created_file_with_previous_content_diffs_against_it() {
    let mut store = ReviewStore::new();
    store.begin_turn(TurnId(1));
    store.file_created(&p("a.txt"), "x\nY\n", Some(Rope::from_str("x\ny\n")));
    assert_eq!(store.hunks(&p("a.txt")).len(), 1);
    let revert = store.reject_file(&p("a.txt")).unwrap();
    assert!(
        matches!(revert, Revert::Edits { .. }),
        "previous content is restored by edits"
    );
}

#[test]
fn turns_scope_decisions() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    assert!(h.store.turn_active(&p("a.txt")));
    h.store.end_turn(TurnId(1));
    assert!(!h.store.turn_active(&p("a.txt")));

    h.turn = 2;
    h.store.begin_turn(TurnId(2));
    h.open("b.txt", TEN);
    h.capture("b.txt");
    h.agent_write(
        "b.txt",
        "one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    assert!(h.store.turn_active(&p("b.txt")));
    assert!(h.store.is_turn_active(TurnId(2)));
    h.store.end_turn(TurnId(2));

    let reverts = h.store.reject_turn(TurnId(2));
    assert_eq!(reverts.len(), 1);
    assert_eq!(reverts[0].path(), p("b.txt"));
    h.apply_all(reverts);
    assert_eq!(h.text("b.txt"), TEN);
    assert_eq!(h.store.pending_files(), vec![p("a.txt")]);
    assert_eq!(h.store.accept_turn(TurnId(1)), vec![p("a.txt")]);
    assert_eq!(h.store.pending_count(), 0);
}

#[test]
fn base_persists_across_turns() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    h.store.end_turn(TurnId(1));
    h.turn = 2;
    h.store.begin_turn(TurnId(2));
    assert!(
        !h.capture("a.txt"),
        "capture is a no-op for a file in review"
    );
    h.agent_write(
        "a.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
    );
    h.store.end_turn(TurnId(2));
    assert_eq!(h.base("a.txt"), TEN);
    assert_eq!(h.store.file(&p("a.txt")).unwrap().turn_id, TurnId(2));
    assert_eq!(h.hunks("a.txt").len(), 2);
}

#[test]
fn stale_recompute_results_are_discarded_and_jobs_cancel() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    let job = h.store.recompute_job(&p("a.txt")).unwrap();
    assert!(job.is_current());
    h.agent_write(
        "a.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
    );
    assert!(!job.is_current());
    let result = job.run();
    assert!(result.data.is_none(), "an outdated job stops early");
    assert_eq!(h.store.apply_recompute(result), RecomputeOutcome::Cancelled);

    // A job that finished before the change is discarded when applied.
    let job = h.store.recompute_job(&p("a.txt")).unwrap();
    let result = job.run();
    h.user_edit("a.txt", 0..0, "x");
    assert_eq!(h.store.apply_recompute(result), RecomputeOutcome::Stale);
    assert!(h.store.needs_recompute(&p("a.txt")));
    let job = h.store.recompute_job(&p("a.txt")).unwrap();
    let job_path = job.path().to_owned();
    let result = std::thread::spawn(move || job.run()).join().unwrap();
    assert_eq!(result.path(), job_path);
    assert_eq!(
        h.store.apply_recompute(result),
        RecomputeOutcome::Applied { hunks: 2 }
    );
    assert!(!h.store.needs_recompute(&p("a.txt")));
    let untracked = ReviewStore::new().recompute(&p("zzz"), &Buffer::new("").snapshot());
    assert_eq!(untracked, RecomputeOutcome::Untracked);
}

#[test]
fn hunk_ids_survive_recompute_and_typing() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
    );
    h.settle("a.txt");
    let ids: Vec<_> = h.hunks("a.txt").iter().map(|x| x.id).collect();
    h.user_edit("a.txt", 1..1, "n");
    h.settle("a.txt");
    let after: Vec<_> = h.hunks("a.txt").iter().map(|x| x.id).collect();
    assert_eq!(ids, after);
    let (path, hunk) = h.store.hunk(ids[1]).unwrap();
    assert_eq!(path, p("a.txt"));
    assert_eq!(hunk.base_rows, 9..10);
}

#[test]
fn undoing_the_agent_edit_removes_the_hunk() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "one\ntwo\nTHREE\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    assert_eq!(h.hunks("a.txt").len(), 1);
    h.undo("a.txt");
    assert!(
        h.hunks("a.txt").is_empty(),
        "Ctrl+Z of the agent edit: the hunk disappears"
    );
    h.store.end_turn(TurnId(1));
    assert!(h.store.file(&p("a.txt")).is_none());
}

#[test]
fn typing_back_the_base_text_resolves_the_hunk() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "one\ntwo\nthreE\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    let at = h.text("a.txt").find('E').unwrap();
    h.user_edit("a.txt", at..at + 1, "e");
    assert!(h.hunks("a.txt").is_empty());
    assert_eq!(h.base("a.txt"), TEN);
}

#[test]
fn user_deleting_the_newline_before_a_hunk_keeps_things_exact() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "one\ntwo\nTHREE\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    // Join "two" with "THREE".
    let at = h.text("a.txt").find("\nTHREE").unwrap();
    h.user_edit("a.txt", at..at + 1, "");
    h.check("a.txt");
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 1);
    let file = h.store.file(&p("a.txt")).unwrap();
    assert_eq!(
        file.current().text_in(hunks[0].buffer_byte_range()),
        "twoTHREE\n"
    );
    assert_eq!(
        h.base("a.txt"),
        "one\ntwothree\nfour\nfive\nsix\nseven\neight\nnine\nten\n"
    );
    let revert = h.store.reject_hunk(hunks[0].id).unwrap();
    h.apply(revert);
    assert_eq!(h.text("a.txt"), h.base("a.txt"));
}

#[test]
fn insertion_at_a_deletion_goes_after_the_deleted_block() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write("a.txt", "one\ntwo\nfive\nsix\nseven\neight\nnine\nten\n");
    h.settle("a.txt");
    let hunk = h.hunks("a.txt")[0].clone();
    assert_eq!(hunk.kind, HunkKind::Deleted);
    let at = hunk.buffer_byte_range().start;
    h.user_edit("a.txt", at..at, "typed\n");
    h.check("a.txt");
    assert_eq!(
        h.base("a.txt"),
        "one\ntwo\nthree\nfour\ntyped\nfive\nsix\nseven\neight\nnine\nten\n"
    );
    assert_eq!(h.hunks("a.txt").len(), 1);
}

#[test]
fn multi_range_events_take_the_fast_path() {
    let mut h = Harness::new();
    h.start("a.txt", "a\nb\nc\nd\ne\nf\ng\n");
    h.agent_write("a.txt", "a\nb\nC\nd\ne\nf\ng\n");
    // A multi-cursor insertion: the same text at three places, one event.
    let buffer = h.buffers.get_mut(&p("a.txt")).unwrap();
    buffer.start_transaction(EditSource::User);
    buffer.edit(&[0..0, 5..5, 11..11], "#");
    buffer.end_transaction();
    h.pump("a.txt");
    h.check("a.txt");
    assert_eq!(h.text("a.txt"), "#a\nb\nC#\nd\ne\nf#\ng\n");
    assert_eq!(h.base("a.txt"), "#a\nb\nc\nd\ne\nf#\ng\n");
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].base_rows, 2..3);
}

#[test]
fn file_written_without_events_is_diffed() {
    let mut store = ReviewStore::new();
    store.begin_turn(TurnId(7));
    store.capture_base_text(&p("a.txt"), TEN);
    let agent = Buffer::new("one\ntwo\nthree\nFOUR\nfive\nsix\nseven\neight\nnine\nten\n");
    let tracked = store.file_written(
        &p("a.txt"),
        &agent.snapshot(),
        EditSource::Agent { turn_id: 7 },
    );
    assert!(tracked.tracked && tracked.changed && tracked.needs_recompute);
    assert_eq!(store.hunks(&p("a.txt")).len(), 1);
    assert_eq!(store.file(&p("a.txt")).unwrap().turn_id, TurnId(7));
    let untracked = store.file_written(&p("nope"), &agent.snapshot(), EditSource::Load);
    assert!(!untracked.tracked);
}

#[test]
fn rejects_in_flight_are_hidden() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
    );
    h.settle("a.txt");
    let hunks = h.hunks("a.txt");
    let revert = h.store.reject_hunk(hunks[0].id).unwrap();
    // Before the host applies it, the hunk is gone from every query.
    assert_eq!(h.store.pending_count(), 1);
    assert!(h.store.accept_hunk(hunks[0].id).is_err());
    assert_eq!(h.store.stats(&p("a.txt")), (1, 1));
    h.apply(revert);
    assert_eq!(h.store.pending_count(), 1);
    h.check("a.txt");
}

#[test]
fn reject_all_mixes_file_kinds() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    h.agent_write(
        "a.txt",
        "one\ntwo\nTHREE\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    h.store.file_created(&p("n.txt"), "new\n", None);
    h.store.file_deleted(&p("d.txt"), Rope::from_str("old\n"));
    h.store.end_turn(TurnId(1));
    let reverts = h.store.reject_all();
    assert_eq!(reverts.len(), 3);
    assert!(reverts.contains(&Revert::File(FileOp::DeleteFile(p("n.txt")))));
    assert!(reverts.contains(&Revert::File(FileOp::WriteFile(p("d.txt"), "old\n".into()))));
    h.apply_all(reverts);
    assert_eq!(h.text("a.txt"), TEN);
    assert_eq!(h.store.pending_count(), 0);
    // One undo takes the three back.
    let undo = h.store.undo_last_reject();
    assert_eq!(undo.len(), 3);
    h.apply_all(undo);
    assert_eq!(h.store.pending_count(), 3);
}

#[test]
fn undo_stack_is_bounded() {
    let mut h = Harness::new();
    h.start("a.txt", TEN);
    for round in 0..cincel_review::UNDO_LIMIT + 5 {
        h.agent_write("a.txt", &format!("{round}\n{TEN}"));
        let id = h.hunks("a.txt")[0].id;
        let revert = h.store.reject_hunk(id).unwrap();
        h.apply(revert);
    }
    let mut undone = 0;
    while !h.store.undo_last_reject().is_empty() {
        undone += 1;
    }
    assert_eq!(undone, cincel_review::UNDO_LIMIT);
}

#[test]
fn unicode_and_odd_line_breaks_keep_rows_exact() {
    let base = "ñandú\u{000C}feed\nmitad\u{2028}línea\n日本語\n";
    let mut h = Harness::new();
    h.start("u.txt", base);
    h.agent_write("u.txt", "ñandú\u{000C}feed\nmitad\u{2028}LÍNEA\n日本語\n");
    h.settle("u.txt");
    let hunks = h.hunks("u.txt");
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].base_rows, 1..2, "rows count only \\n");
    h.check("u.txt");
    let revert = h.store.reject_hunk(hunks[0].id).unwrap();
    h.apply(revert);
    assert_eq!(h.text("u.txt"), base);
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn unknown_ids_and_paths_are_errors() {
    let mut store = ReviewStore::new();
    assert!(store.accept_hunk(cincel_review::HunkId(42)).is_err());
    assert!(store.reject_hunk(cincel_review::HunkId(42)).is_err());
    assert!(store.accept_file(&p("x")).is_err());
    assert!(store.reject_file(&p("x")).is_err());
    assert!(store.reject_all().is_empty());
    assert!(store.accept_all().is_empty());
    assert_eq!(store.stats(&p("x")), (0, 0));
    let event_buffer = Buffer::new("a");
    let tracked = store.buffer_edited(
        &p("x"),
        &cincel_text::BufferEvent::Edited {
            source: EditSource::User,
            old_ranges: vec![0..0],
            new_ranges: vec![0..1],
            version: 1,
        },
        &event_buffer.snapshot(),
        EditSource::User,
    );
    assert!(!tracked.tracked);
}
