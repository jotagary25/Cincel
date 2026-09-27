//! The "Casos de borde obligatorios" of `docs/specs/modulos/review.md`.

mod common;

use cincel_review::{FileOp, HunkKind, Revert, TurnId};
use cincel_text::Rope;
use common::{Harness, p};

const BASE: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";

#[test]
fn accept_keeps_offsets_of_other_hunks() {
    let mut h = Harness::new();
    h.start("a.txt", BASE);
    h.agent_write(
        "a.txt",
        "ONE\ntwo\nthree\nfour and more\nextra\nfive\nsix\nseven\neight\nnine\nTEN\n",
    );
    h.settle("a.txt");
    h.store.end_turn(TurnId(1));
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 3);
    // Accept the middle one, then the others must still accept exactly.
    h.store.accept_hunk(hunks[1].id).unwrap();
    h.check("a.txt");
    let rest = h.hunks("a.txt");
    assert_eq!(rest.len(), 2);
    assert_eq!(rest[0].id, hunks[0].id);
    assert_eq!(rest[1].id, hunks[2].id);
    assert_eq!(
        rest[1].base_rows,
        10..11,
        "rows after the accepted hunk shifted by one"
    );
    h.store.accept_hunk(rest[1].id).unwrap();
    h.store.accept_hunk(rest[0].id).unwrap();
    assert!(
        h.store.file(&p("a.txt")).is_none(),
        "fully accepted file leaves the review"
    );
    assert_eq!(
        h.text("a.txt"),
        "ONE\ntwo\nthree\nfour and more\nextra\nfive\nsix\nseven\neight\nnine\nTEN\n"
    );
}

#[test]
fn accept_top_down_matches_bottom_up() {
    let target = "zero\none\ntwo\nTHREE\nfour\nfive\n6\nseven\neight\nnine\n";
    for order in [false, true] {
        let mut h = Harness::new();
        h.start("a.txt", BASE);
        h.agent_write("a.txt", target);
        h.settle("a.txt");
        h.store.end_turn(TurnId(1));
        let mut ids: Vec<_> = h.hunks("a.txt").iter().map(|x| x.id).collect();
        assert!(ids.len() >= 3);
        if order {
            ids.reverse();
        }
        for id in ids {
            h.store.accept_hunk(id).unwrap();
            h.check("a.txt");
        }
        assert!(h.store.file(&p("a.txt")).is_none());
        assert_eq!(h.store.pending_count(), 0);
    }
}

#[test]
fn reject_touches_only_its_range() {
    let mut h = Harness::new();
    h.start("a.txt", BASE);
    let written = "ONE\ntwo\nthree\nFOUR\nfive\nsix\nseven\neight\nnine\nTEN\n";
    h.agent_write("a.txt", written);
    h.settle("a.txt");
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 3);
    let revert = h.store.reject_hunk(hunks[1].id).unwrap();
    let Revert::Edits { edits, .. } = &revert else {
        panic!("edits expected")
    };
    let range = hunks[1].buffer_byte_range();
    assert!(
        edits
            .iter()
            .all(|e| e.range.start >= range.start && e.range.end <= range.end)
    );
    h.apply(revert);
    // Base text in the rejected range, agent text everywhere else.
    assert_eq!(
        h.text("a.txt"),
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n"
    );
    let rest = h.hunks("a.txt");
    assert_eq!(rest.len(), 2);
    assert_eq!(rest[0].id, hunks[0].id);
    assert_eq!(rest[1].id, hunks[2].id);
    assert_eq!(h.base("a.txt"), BASE);
    h.check("a.txt");
    assert_eq!(h.disk[&p("a.txt")], h.text("a.txt"), "reject saves");
}

#[test]
fn two_agent_writes_in_one_turn_make_one_hunk_against_original_base() {
    let mut h = Harness::new();
    h.start("a.txt", BASE);
    h.agent_write(
        "a.txt",
        "one\ntwo\nTHREE\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    h.settle("a.txt");
    h.agent_write(
        "a.txt",
        "one\ntwo\nTHREE!!\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    h.settle("a.txt");
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].base_rows, 2..3);
    assert_eq!(h.base("a.txt"), BASE);
    assert_eq!(h.store.stats(&p("a.txt")), (1, 1));
}

#[test]
fn user_edit_outside_hunks_goes_to_base() {
    let mut h = Harness::new();
    h.start("a.txt", BASE);
    h.agent_write(
        "a.txt",
        "one\ntwo\nthree\nFOUR\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    h.settle("a.txt");
    let before = h.hunks("a.txt");
    // Type at the end of line "nine" and at the very start.
    let at = h.text("a.txt").find("nine").unwrap() + 4;
    h.user_edit("a.txt", at..at, " (edited)");
    h.user_edit("a.txt", 0..0, "// header\n");
    h.check("a.txt");
    let after = h.hunks("a.txt");
    assert_eq!(after.len(), 1, "no new hunk");
    assert_eq!(after[0].id, before[0].id);
    assert_eq!(after[0].base_rows, 4..5);
    assert!(h.base("a.txt").starts_with("// header\none\n"));
    assert!(h.base("a.txt").contains("nine (edited)\n"));
    h.settle("a.txt");
    assert_eq!(h.hunks("a.txt").len(), 1);
}

#[test]
fn user_edit_inside_a_hunk_keeps_it_pending_with_user_text() {
    let mut h = Harness::new();
    h.start("a.txt", BASE);
    h.agent_write(
        "a.txt",
        "one\ntwo\nthree\nFOUR\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    h.settle("a.txt");
    let id = h.hunks("a.txt")[0].id;
    let at = h.text("a.txt").find("FOUR").unwrap() + 4;
    h.user_edit("a.txt", at..at, " by user");
    h.check("a.txt");
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].id, id, "the hunk keeps its id");
    assert!(hunks[0].is_pending());
    let file = h.store.file(&p("a.txt")).unwrap();
    assert_eq!(
        file.current().text_in(hunks[0].buffer_byte_range()),
        "FOUR by user\n"
    );
    assert_eq!(
        h.base("a.txt"),
        BASE,
        "the base does not get the user's text"
    );
    // Rejecting it removes the user's text too.
    let revert = h.store.reject_hunk(id).unwrap();
    h.apply(revert);
    assert_eq!(h.text("a.txt"), BASE);
}

#[test]
fn created_file_reject_deletes_it() {
    let mut h = Harness::new();
    h.store.begin_turn(TurnId(1));
    h.store.file_created(&p("new.rs"), "fn main() {}\n", None);
    h.settle("new.rs");
    let hunks: Vec<_> = h.store.hunks(&p("new.rs")).into_iter().cloned().collect();
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].kind, HunkKind::Added);
    let revert = h.store.reject_file(&p("new.rs")).unwrap();
    assert_eq!(revert, Revert::File(FileOp::DeleteFile(p("new.rs"))));
    assert!(h.store.file(&p("new.rs")).is_none());
}

#[test]
fn created_file_edited_by_user_is_not_deleted() {
    let mut h = Harness::new();
    h.store.begin_turn(TurnId(1));
    h.store.file_created(&p("new.rs"), "fn main() {}\n", None);
    // The host opens the file and the user types in it.
    h.open("new.rs", "fn main() {}\n");
    let snapshot = h.buffers[&p("new.rs")].snapshot();
    h.store
        .file_written(&p("new.rs"), &snapshot, cincel_text::EditSource::Load);
    h.user_edit("new.rs", 0..0, "// mine\n");
    h.store.end_turn(TurnId(1));
    let revert = h.store.reject_file(&p("new.rs")).unwrap();
    let Revert::Edits { path, edits } = &revert else {
        panic!("a user-edited new file is not deleted: {revert:?}")
    };
    assert_eq!(path, &p("new.rs"));
    assert!(!edits.is_empty());
    h.apply(revert);
    assert!(h.buffers.contains_key(&p("new.rs")), "file kept");
}

#[test]
fn newline_added_at_eof_is_a_one_line_hunk() {
    let mut h = Harness::new();
    h.start("a.txt", "a\nb\nc");
    h.agent_write("a.txt", "a\nb\nc\n");
    h.settle("a.txt");
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].base_rows, 2..3);
    assert_eq!(h.store.stats(&p("a.txt")), (1, 1));
    h.check("a.txt");
}

#[test]
fn newline_removed_at_eof_is_a_one_line_hunk() {
    let mut h = Harness::new();
    h.start("a.txt", BASE);
    h.agent_write("a.txt", BASE.trim_end_matches('\n'));
    h.settle("a.txt");
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].base_rows, 9..10);
    let revert = h.store.reject_hunk(hunks[0].id).unwrap();
    h.apply(revert);
    assert_eq!(h.text("a.txt"), BASE);
}

#[test]
fn accepting_the_middle_line_leaves_two_one_line_hunks() {
    let mut h = Harness::new();
    h.start("a.txt", "keep\nalpha one\nbeta two\ngamma three\nkeep\n");
    h.agent_write("a.txt", "keep\nalpha ONE\nbeta TWO\ngamma THREE\nkeep\n");
    h.settle("a.txt");
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].lines.len(), 3);
    assert!(
        hunks[0]
            .lines
            .iter()
            .all(|l| l.base_row.is_some() && l.buffer_row.is_some())
    );
    let middle = hunks[0].lines[1];
    h.store.accept_line(&p("a.txt"), &middle).unwrap();
    h.check("a.txt");
    let after = h.hunks("a.txt");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].base_rows, 1..2);
    assert_eq!(after[1].base_rows, 3..4);
    assert_eq!(
        h.base("a.txt"),
        "keep\nalpha one\nbeta TWO\ngamma three\nkeep\n"
    );
    // A full recompute agrees.
    h.settle("a.txt");
    assert_eq!(h.hunks("a.txt").len(), 2);
}

#[test]
fn rejecting_the_middle_line_leaves_two_one_line_hunks() {
    let mut h = Harness::new();
    h.start("a.txt", "keep\nalpha one\nbeta two\ngamma three\nkeep\n");
    h.agent_write("a.txt", "keep\nalpha ONE\nbeta TWO\ngamma THREE\nkeep\n");
    h.settle("a.txt");
    let middle = h.line("a.txt", 0, 1);
    let revert = h.store.reject_line(&p("a.txt"), &middle).unwrap();
    h.apply(revert);
    h.check("a.txt");
    assert_eq!(
        h.text("a.txt"),
        "keep\nalpha ONE\nbeta two\ngamma THREE\nkeep\n"
    );
    let after = h.hunks("a.txt");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].base_rows, 1..2);
    assert_eq!(after[1].base_rows, 3..4);
}

#[test]
fn undo_last_reject_restores_text_and_hunk() {
    let mut h = Harness::new();
    h.start("a.txt", BASE);
    let written = "one\ntwo\nthree\nFOUR\nfive\nsix\nseven\nEIGHT\nnine\nten\n";
    h.agent_write("a.txt", written);
    h.settle("a.txt");
    let hunks = h.hunks("a.txt");
    let revert = h.store.reject_hunk(hunks[0].id).unwrap();
    h.apply(revert);
    assert_eq!(h.hunks("a.txt").len(), 1);
    // The user types elsewhere before undoing: the region follows.
    h.user_edit("a.txt", 0..0, "// top\n");
    let reverts = h.store.undo_last_reject();
    assert_eq!(reverts.len(), 1);
    h.apply_all(reverts);
    assert_eq!(h.text("a.txt"), format!("// top\n{written}"));
    let back = h.hunks("a.txt");
    assert_eq!(back.len(), 2, "the rejected hunk reappears");
    assert_eq!(back[0].base_rows, 4..5);
    h.check("a.txt");
    assert!(!h.store.can_undo_reject());
    assert!(h.store.undo_last_reject().is_empty());
}

#[test]
fn undo_last_reject_of_a_whole_file() {
    let mut h = Harness::new();
    h.start("a.txt", BASE);
    h.agent_write(
        "a.txt",
        "one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
    );
    h.settle("a.txt");
    let written = h.text("a.txt");
    let revert = h.store.reject_file(&p("a.txt")).unwrap();
    h.apply(revert);
    assert_eq!(h.text("a.txt"), BASE);
    assert_eq!(h.store.pending_count(), 0);
    let reverts = h.store.undo_last_reject();
    h.apply_all(reverts);
    assert_eq!(h.text("a.txt"), written);
    h.settle("a.txt");
    assert_eq!(h.hunks("a.txt").len(), 2);
}

#[test]
fn undo_reject_of_a_created_file_recreates_it() {
    let mut h = Harness::new();
    h.store.begin_turn(TurnId(1));
    h.store.file_created(&p("n.txt"), "hello\n", None);
    h.store.end_turn(TurnId(1));
    let revert = h.store.reject_file(&p("n.txt")).unwrap();
    assert_eq!(revert, Revert::File(FileOp::DeleteFile(p("n.txt"))));
    let reverts = h.store.undo_last_reject();
    assert_eq!(
        reverts,
        vec![Revert::File(FileOp::WriteFile(
            p("n.txt"),
            "hello\n".into()
        ))]
    );
    h.apply_all(reverts);
    assert_eq!(h.store.pending_count(), 1);
    assert!(matches!(
        h.store.file(&p("n.txt")).unwrap().status,
        cincel_review::FileStatus::Created { .. }
    ));
}

#[test]
fn persistence_round_trip_reproduces_hunks() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new();
    h.start("a.txt", BASE);
    h.agent_write(
        "a.txt",
        "one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
    );
    h.store.file_created(&p("n.txt"), "new\n", None);
    h.store
        .file_deleted(&p("gone.txt"), Rope::from_str("bye\n"));
    h.store.end_turn(TurnId(1));
    h.settle_all();
    h.store.save(dir.path()).unwrap();

    let disk = h.disk.clone();
    let mut restored = cincel_review::ReviewStore::new();
    let report = restored
        .load(dir.path(), |path| {
            if path == p("n.txt") {
                Some(b"new\n".to_vec())
            } else {
                disk.get(path).map(|t| t.clone().into_bytes())
            }
        })
        .unwrap();
    assert_eq!(report.dropped, vec![]);
    assert_eq!(report.restored.len(), 3);
    let original: Vec<_> = h
        .store
        .hunks(&p("a.txt"))
        .iter()
        .map(|x| (x.base_rows.clone(), x.buffer_byte_range()))
        .collect();
    let loaded: Vec<_> = restored
        .hunks(&p("a.txt"))
        .iter()
        .map(|x| (x.base_rows.clone(), x.buffer_byte_range()))
        .collect();
    assert_eq!(original, loaded);
    assert_eq!(restored.pending_count(), h.store.pending_count());
    assert!(matches!(
        restored.file(&p("gone.txt")).unwrap().status,
        cincel_review::FileStatus::Deleted { .. }
    ));
    assert_eq!(restored.file(&p("a.txt")).unwrap().turn_id, TurnId(1));
}

#[test]
fn persistence_drops_entries_whose_file_changed() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new();
    h.start("a.txt", BASE);
    h.agent_write(
        "a.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    h.start("b.txt", "b\n");
    h.agent_write("b.txt", "B\n");
    h.store
        .file_deleted(&p("gone.txt"), Rope::from_str("bye\n"));
    h.store.save(dir.path()).unwrap();

    let mut restored = cincel_review::ReviewStore::new();
    let report = restored
        .load(dir.path(), |path| match path.to_str() {
            Some("a.txt") => Some(b"changed outside\n".to_vec()),
            Some("gone.txt") => Some(b"it is back\n".to_vec()),
            _ => None,
        })
        .unwrap();
    use cincel_review::DropReason;
    assert!(report.restored.is_empty());
    assert!(report.dropped.contains(&(p("a.txt"), DropReason::Changed)));
    assert!(report.dropped.contains(&(p("b.txt"), DropReason::Missing)));
    assert!(
        report
            .dropped
            .contains(&(p("gone.txt"), DropReason::Reappeared))
    );
    assert_eq!(restored.pending_count(), 0);
}

#[test]
fn persistence_is_crlf_agnostic_and_detects_corrupt_objects() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new();
    h.start("a.txt", "x\ny\n");
    h.agent_write("a.txt", "x\nY\n");
    h.store.save(dir.path()).unwrap();
    let mut restored = cincel_review::ReviewStore::new();
    let report = restored
        .load(dir.path(), |_| Some(b"x\r\nY\r\n".to_vec()))
        .unwrap();
    assert_eq!(report.restored, vec![p("a.txt")]);
    assert_eq!(restored.hunks(&p("a.txt")).len(), 1);

    // Corrupt the base object.
    for entry in std::fs::read_dir(dir.path().join("objects")).unwrap() {
        std::fs::write(entry.unwrap().path(), b"garbage").unwrap();
    }
    let mut again = cincel_review::ReviewStore::new();
    let report = again
        .load(dir.path(), |_| Some(b"x\nY\n".to_vec()))
        .unwrap();
    assert_eq!(
        report.dropped,
        vec![(p("a.txt"), cincel_review::DropReason::BadObject)]
    );
}

#[test]
fn save_collects_orphan_objects() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new();
    h.start("a.txt", "x\n");
    h.agent_write("a.txt", "y\n");
    h.store.save(dir.path()).unwrap();
    assert_eq!(
        std::fs::read_dir(dir.path().join("objects"))
            .unwrap()
            .count(),
        1
    );
    h.store.end_turn(TurnId(1));
    h.store.accept_all();
    h.store.save(dir.path()).unwrap();
    assert_eq!(
        std::fs::read_dir(dir.path().join("objects"))
            .unwrap()
            .count(),
        0
    );
    let mut restored = cincel_review::ReviewStore::new();
    let report = restored.load(dir.path(), |_| None).unwrap();
    assert!(report.restored.is_empty() && report.dropped.is_empty());
}

#[test]
fn load_without_state_restores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = cincel_review::ReviewStore::new();
    let report = store.load(dir.path(), |_| None).unwrap();
    assert_eq!(report, cincel_review::LoadReport::default());
}
