//! Comments in `state.json` v2 (spec 09 D15, §6.4): migration from v1,
//! round trip, relocation, drops and the snippet objects.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cincel_review::{
    CommentDropReason, CommentId, CommentState, ReviewStore, STATE_VERSION, TurnId,
};
use cincel_text::Rope;
use common::{Harness, p};

const CALC: &str = "/proj/src/calc.py";
const UTIL: &str = "/proj/src/util.py";
const CALC_BASE: &str = "import math\n\ndef total(items):\n    return sum(items)\n\ndef mean(items):\n    return total(items) / len(items)\n";
const CALC_AGENT: &str = "import math\n\ndef total(items, discount=0):\n    return sum(items) * (1 - discount)\n\ndef mean(items):\n    return sum(items) / len(items)\n";
const UTIL_TEXT: &str = "import os\n\nHOST = 'x'\nPORT = 1\n\n\nTIMEOUT = 3\nRETRIES = 2\n";

fn reviewed() -> Harness {
    let mut h = Harness::new();
    h.store.set_workspace_root(Some(PathBuf::from("/proj")));
    h.start(CALC, CALC_BASE);
    h.agent_write(CALC, CALC_AGENT);
    h.settle(CALC);
    h.store.end_turn(TurnId(1));
    h.open(UTIL, UTIL_TEXT);
    h
}

fn add(
    h: &mut Harness,
    path: &str,
    rows: std::ops::Range<u32>,
    from_hunk: bool,
    text: &str,
) -> CommentId {
    let snapshot = h.buffers[&p(path)].snapshot();
    let hunk = from_hunk.then(|| h.hunks(path)[0].id);
    h.store
        .add_comment(&p(path), &snapshot, rows, hunk, text.to_owned())
        .unwrap()
}

fn load(dir: &Path, disk: &BTreeMap<PathBuf, String>) -> (ReviewStore, cincel_review::LoadReport) {
    let mut store = ReviewStore::new();
    store.set_workspace_root(Some(PathBuf::from("/proj")));
    let report = store
        .load(dir, |path| disk.get(path).map(|t| t.as_bytes().to_vec()))
        .unwrap();
    (store, report)
}

fn view_rows(store: &ReviewStore) -> Vec<(String, std::ops::Range<u32>, String)> {
    store
        .comments(|_| None)
        .into_iter()
        .map(|c| (c.display_path, c.rows, c.text))
        .collect()
}

fn state_json(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(dir.join("state.json")).unwrap()).unwrap()
}

#[test]
fn version_1_loads_without_comments() {
    let mut h = reviewed();
    add(&mut h, UTIL, 6..7, false, "x");
    let dir = tempfile::tempdir().unwrap();
    h.store.save(dir.path()).unwrap();
    // What a 0.1.0 wrote: version 1, no `comments`.
    let mut json = state_json(dir.path());
    json["version"] = 1.into();
    json.as_object_mut().unwrap().remove("comments");
    std::fs::write(
        dir.path().join("state.json"),
        serde_json::to_vec(&json).unwrap(),
    )
    .unwrap();
    let (store, report) = load(dir.path(), &h.disk);
    assert_eq!(report.restored, vec![p(CALC)]);
    assert_eq!(report.comments_restored, 0);
    assert!(report.comments_dropped.is_empty());
    assert_eq!(store.comment_count(), 0);
    assert_eq!(store.hunks(&p(CALC)).len(), 2);
}

#[test]
fn version_2_round_trip() {
    let mut h = reviewed();
    let on_hunk = add(&mut h, CALC, 2..4, true, "firma");
    let selection = add(&mut h, UTIL, 6..7, false, "config\ncon dos líneas");
    // A mark survives: accept the second hunk under a third comment.
    let mean = add(&mut h, CALC, 6..7, false, "mean");
    let hunk = h.hunks(CALC)[1].id;
    h.store.accept_hunk(hunk).unwrap();
    let dir = tempfile::tempdir().unwrap();
    h.store.save(dir.path()).unwrap();

    let json = state_json(dir.path());
    assert_eq!(json["version"], STATE_VERSION);
    assert_eq!(STATE_VERSION, 2);
    let comments = json["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 3);
    let first = &comments[0];
    assert_eq!(first["id"], on_hunk.0);
    assert_eq!(first["path"], CALC);
    assert_eq!(
        (first["start_row"].as_u64(), first["end_row"].as_u64()),
        (Some(2), Some(4))
    );
    assert_eq!(first["from_hunk"], true);
    assert_eq!(first["accepted"], false);
    assert_eq!(first["rejected"], false);
    assert!(first["created_at"].as_u64().unwrap() > 0);
    for key in ["file_hash", "snippet_hash"] {
        let hash = first[key].as_str().unwrap();
        assert_eq!(hash.len(), 64, "{key}");
    }
    assert_eq!(comments[2]["accepted"], true);

    let (store, report) = load(dir.path(), &h.disk);
    assert_eq!(report.comments_restored, 3);
    assert_eq!(report.comments_relocated, 0);
    assert_eq!(view_rows(&store), view_rows(&h.store));
    let views = store.comments(|_| None);
    assert_eq!(views[0].id, on_hunk);
    // The hunk ids are new after a load; the button's hunk is found again.
    assert_eq!(views[0].from_hunk, Some(store.hunks(&p(CALC))[0].id));
    assert_eq!(views[1].from_hunk, None);
    assert_eq!(store.comment_state(on_hunk), Some(CommentState::Pending));
    assert_eq!(store.comment_state(mean), Some(CommentState::Accepted));
    assert_eq!(
        store.comment_state(selection),
        Some(CommentState::NoAgentChange)
    );
    // New comments do not reuse loaded ids.
    let mut store = store;
    let snapshot = h.buffers[&p(UTIL)].snapshot();
    let fresh = store
        .add_comment(&p(UTIL), &snapshot, 0..1, None, "nuevo".into())
        .unwrap();
    assert!(fresh.0 > on_hunk.0.max(selection.0).max(mean.0));
}

#[test]
fn changed_file_relocates_by_snippet_or_to_the_nearest_row() {
    let mut h = reviewed();
    add(&mut h, UTIL, 6..8, false, "timeout");
    add(&mut h, UTIL, 3..4, false, "port");
    let dir = tempfile::tempdir().unwrap();
    h.store.save(dir.path()).unwrap();
    // Outside Cincel: two lines above, PORT rewritten.
    let mut disk = h.disk.clone();
    disk.insert(
        p(UTIL),
        "# a\n# b\nimport os\n\nHOST = 'x'\nPORT = 2\n\n\nTIMEOUT = 3\nRETRIES = 2\n".into(),
    );
    let (store, report) = load(dir.path(), &disk);
    assert_eq!(report.comments_restored, 2);
    assert_eq!(report.comments_relocated, 2);
    assert_eq!(
        view_rows(&store),
        vec![
            ("src/util.py".into(), 3..4, "port".into()),
            ("src/util.py".into(), 8..10, "timeout".into()),
        ]
    );
    // The nearest occurrence wins.
    let mut disk = h.disk.clone();
    disk.insert(
        p(UTIL),
        "TIMEOUT = 3\nRETRIES = 2\n\nx\n\n\n\n\nTIMEOUT = 3\nRETRIES = 2\n".into(),
    );
    let (store, _) = load(dir.path(), &disk);
    assert_eq!(view_rows(&store)[1].1, 8..10);
    // A shorter file: clamped to its last row.
    let mut disk = h.disk.clone();
    disk.insert(p(UTIL), "solo\n".into());
    let (store, report) = load(dir.path(), &disk);
    assert_eq!(report.comments_relocated, 2);
    let rows: Vec<_> = view_rows(&store).into_iter().map(|v| v.1).collect();
    assert_eq!(rows, vec![1..2, 1..2]);
}

#[test]
fn missing_or_binary_files_drop_their_comments() {
    let mut h = reviewed();
    add(&mut h, UTIL, 6..7, false, "x");
    add(&mut h, "/proj/src/calc.py", 0..1, false, "y");
    let dir = tempfile::tempdir().unwrap();
    h.store.save(dir.path()).unwrap();
    let mut disk = h.disk.clone();
    disk.remove(&p(UTIL));
    let (store, report) = load(dir.path(), &disk);
    assert_eq!(
        report.comments_dropped,
        vec![(p(UTIL), CommentDropReason::Missing)]
    );
    assert_eq!(store.comment_count(), 1);

    let mut store = ReviewStore::new();
    let report = store
        .load(dir.path(), |path| {
            if path == p(UTIL) {
                Some(vec![0x50, 0, 0, 1])
            } else if path == p(CALC) {
                Some(vec![0xff, 0xfe, 0x00])
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        report.comments_dropped,
        vec![
            (p(UTIL), CommentDropReason::NotText),
            (p(CALC), CommentDropReason::NotText),
        ]
    );
    assert_eq!(store.comment_count(), 0);
}

#[test]
fn snippet_objects_survive_the_cleanup_while_referenced() {
    let mut h = reviewed();
    let id = add(&mut h, UTIL, 6..8, false, "x");
    let dir = tempfile::tempdir().unwrap();
    h.store.save(dir.path()).unwrap();
    let hash = state_json(dir.path())["comments"][0]["snippet_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let object = dir.path().join("objects").join(&hash);
    assert_eq!(
        std::fs::read_to_string(&object).unwrap(),
        "TIMEOUT = 3\nRETRIES = 2\n"
    );
    h.store.save(dir.path()).unwrap();
    assert!(object.is_file());
    h.store.remove_comment(id);
    h.store.save(dir.path()).unwrap();
    assert!(!object.is_file());
}

#[test]
fn sent_comments_are_not_saved() {
    let mut h = reviewed();
    add(&mut h, UTIL, 6..7, false, "x");
    let sent = h.store.take_comments_for_prompt(|_| None);
    assert_eq!(sent.len(), 1);
    let dir = tempfile::tempdir().unwrap();
    h.store.save(dir.path()).unwrap();
    assert_eq!(
        state_json(dir.path())["comments"].as_array().unwrap().len(),
        0
    );
    let (store, report) = load(dir.path(), &h.disk);
    assert_eq!(report.comments_restored, 0);
    assert_eq!(store.comment_count(), 0);
}

#[test]
fn a_deleted_file_in_review_keeps_its_comment() {
    let mut h = Harness::new();
    let old = "/proj/old.py";
    let previous = "def legacy():\n    pass\n";
    h.store.begin_turn(TurnId(1));
    h.store.file_deleted(&p(old), Rope::from_str(previous));
    h.store.end_turn(TurnId(1));
    h.open(old, previous);
    h.disk.remove(&p(old));
    add(&mut h, old, 0..2, false, "sigue en uso");
    let dir = tempfile::tempdir().unwrap();
    h.store.save(dir.path()).unwrap();
    let (store, report) = load(dir.path(), &h.disk);
    assert_eq!(report.restored, vec![p(old)]);
    assert_eq!(report.comments_restored, 1);
    let views = store.comments(|_| None);
    assert_eq!(views[0].rows, 0..2);
    assert_eq!(
        store.comment_state(views[0].id),
        Some(CommentState::Pending)
    );
}
