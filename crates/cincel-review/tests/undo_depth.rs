//! `ReviewStore::undo_depth`: how many rejects `undo_last_reject` can take
//! back, compared before and after a reject to know whether it stacked one.

mod common;

use cincel_review::UNDO_LIMIT;
use common::{Harness, p};

const BASE: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
const WRITTEN: &str = "one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n";

#[test]
fn undo_depth_follows_rejects_and_undos() {
    let mut h = Harness::new();
    assert_eq!(h.store.undo_depth(), 0);
    h.start("a.txt", BASE);
    h.agent_write("a.txt", WRITTEN);
    h.settle("a.txt");
    let hunks = h.hunks("a.txt");
    assert_eq!(hunks.len(), 2);

    // An accept stacks nothing.
    h.store.accept_hunk(hunks[0].id).unwrap();
    assert_eq!(h.store.undo_depth(), 0);

    let revert = h.store.reject_hunk(hunks[1].id).unwrap();
    h.apply(revert);
    assert_eq!(h.store.undo_depth(), 1);

    // A reject that finds nothing to reject stacks nothing either.
    let before = h.store.undo_depth();
    let _ = h.store.reject_file(&p("a.txt"));
    assert_eq!(h.store.undo_depth(), before);

    let reverts = h.store.undo_last_reject();
    h.apply_all(reverts);
    assert_eq!(h.store.undo_depth(), 0);
    assert!(!h.store.can_undo_reject());
}

#[test]
fn undo_depth_stops_at_the_limit() {
    let mut h = Harness::new();
    let count = UNDO_LIMIT + 3;
    for index in 0..count {
        let path = format!("f{index}.txt");
        h.start(&path, BASE);
        h.agent_write(&path, WRITTEN);
        h.settle(&path);
    }
    for index in 0..count {
        let path = format!("f{index}.txt");
        let revert = h.store.reject_file(&p(&path)).unwrap();
        h.apply(revert);
        assert_eq!(h.store.undo_depth(), (index + 1).min(UNDO_LIMIT));
    }
    for left in (0..UNDO_LIMIT).rev() {
        let reverts = h.store.undo_last_reject();
        assert!(!reverts.is_empty());
        h.apply_all(reverts);
        assert_eq!(h.store.undo_depth(), left);
    }
    assert!(h.store.undo_last_reject().is_empty());
}
