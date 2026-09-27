//! `report_for_agent`: patches from what the agent left to what the review
//! kept, checked with `git apply --check` (when `git` is on PATH) and with
//! the crate's own applier.

mod common;

use std::path::Path;
use std::process::Command;

use cincel_review::patch::{apply_patch, unified_patch};
use cincel_review::{FORMATTING_HEADER, REPORT_HEADER, TurnId};
use cincel_text::Rope;
use common::{Harness, Rng, mutate, p, random_text};

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Writes `files` into a fresh repo and runs `git apply` (`--check` first,
/// then for real). Returns the texts afterwards.
fn git_apply(files: &[(&str, Option<&str>)], patch: &str) -> Vec<Option<String>> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let status = Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success());
    for (name, text) in files {
        if let Some(text) = text {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }
    std::fs::write(root.join("report.patch"), patch).unwrap();
    for args in [
        &["apply", "--check", "report.patch"][..],
        &["apply", "report.patch"][..],
    ] {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}\n{patch}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    files
        .iter()
        .map(|(name, _)| std::fs::read_to_string(root.join(name)).ok())
        .collect()
}

/// Splits a multi-file patch per file.
fn per_file(patch: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in patch.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            out.push(String::new());
        }
        out.last_mut().expect("starts with a header").push_str(line);
    }
    out
}

#[test]
fn report_patch_applies_to_what_the_agent_left() {
    let mut h = Harness::new();
    let base =
        "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\nfn f() {}\nfn g() {}\nfn h() {}\n";
    h.start("src/lib.rs", base);
    let agent =
        "fn a() {}\nfn B() {}\nfn c() {}\nfn d() {}\nfn e() {}\nfn f() {}\nfn G() {}\nfn h() {}";
    h.agent_write("src/lib.rs", agent);
    h.store
        .file_created(&p("src/new.rs"), "pub fn new() {}\n", None);
    h.store.file_deleted(&p("old.rs"), Rope::from_str("old\n"));
    h.store.end_turn(TurnId(1));
    h.settle("src/lib.rs");

    // Reject the first hunk, edit inside the second, reject the new file,
    // restore the deleted one.
    let hunks = h.hunks("src/lib.rs");
    assert_eq!(hunks.len(), 2);
    let revert = h.store.reject_hunk(hunks[0].id).unwrap();
    h.apply(revert);
    let at = h.text("src/lib.rs").find("fn G").unwrap() + 4;
    h.user_edit("src/lib.rs", at..at, "_user");
    h.store.reject_file(&p("src/new.rs")).unwrap();
    h.store.reject_file(&p("old.rs")).unwrap();

    let report = h
        .store
        .report_patches(TurnId(1))
        .expect("the user changed things");
    assert!(report.formatting_patch.is_none());
    let text = h.store.report_for_agent(TurnId(1)).unwrap();
    assert!(text.starts_with(REPORT_HEADER));

    let after_lib = h.text("src/lib.rs");
    // Own applier, per file.
    let patches = per_file(&report.user_patch);
    assert_eq!(patches.len(), 3, "{}", report.user_patch);
    let lib = patches.iter().find(|x| x.contains("a/src/lib.rs")).unwrap();
    assert_eq!(apply_patch(agent, lib).as_deref(), Some(after_lib.as_str()));

    if git_available() {
        let result = git_apply(
            &[
                ("src/lib.rs", Some(agent)),
                ("src/new.rs", Some("pub fn new() {}\n")),
                ("old.rs", None),
            ],
            &report.user_patch,
        );
        assert_eq!(result[0].as_deref(), Some(after_lib.as_str()));
        assert_eq!(
            result[1], None,
            "the rejected new file is deleted by the patch"
        );
        assert_eq!(
            result[2].as_deref(),
            Some("old\n"),
            "the restored file is created by the patch"
        );
    }
}

#[test]
fn report_is_none_when_everything_was_accepted() {
    let mut h = Harness::new();
    h.start("a.txt", "x\ny\n");
    h.agent_write("a.txt", "x\nY\n");
    h.store.end_turn(TurnId(1));
    h.store.accept_all();
    assert_eq!(h.store.report_for_agent(TurnId(1)), None);
    assert_eq!(h.store.report_for_agent(TurnId(99)), None);
}

#[test]
fn report_separates_formatting() {
    let mut h = Harness::new();
    h.start("a.rs", "fn a(){}\n");
    h.agent_write("a.rs", "fn a(){ 1 }\nfn b(){}\n");
    // The host formats the agent's text into the buffer, then the user
    // renames `b`.
    let formatted = "fn a() {\n    1\n}\nfn b() {}\n";
    h.buffers
        .get_mut(&p("a.rs"))
        .unwrap()
        .set_text_minimal(formatted, cincel_text::EditSource::Load);
    h.pump("a.rs");
    h.store.end_turn(TurnId(1));
    h.store.set_formatted_text(TurnId(1), &p("a.rs"), formatted);
    let at = h.text("a.rs").find("fn b").unwrap() + 3;
    h.user_edit("a.rs", at..at + 1, "bee");
    h.settle("a.rs");
    let text = h.store.report_for_agent(TurnId(1)).unwrap();
    assert!(text.contains(FORMATTING_HEADER), "{text}");
    let report = h.store.report_patches(TurnId(1)).unwrap();
    let formatting = report.formatting_patch.unwrap();
    assert_eq!(
        apply_patch("fn a(){ 1 }\nfn b(){}\n", &formatting).as_deref(),
        Some(formatted)
    );
    // The user patch goes from the formatted text to what is there now.
    assert_eq!(
        apply_patch("fn a() {\n    1\n}\nfn b() {}\n", &report.user_patch).as_deref(),
        Some(h.text("a.rs").as_str())
    );
    h.store.forget_turn(TurnId(1));
    assert_eq!(h.store.report_for_agent(TurnId(1)), None);
}

#[test]
fn report_paths_are_relative_to_the_workspace() {
    let mut h = Harness::new();
    h.store.set_workspace_root(Some(p("/work/space")));
    h.start("/work/space/dir/a.txt", "x\n");
    h.agent_write("/work/space/dir/a.txt", "y\n");
    h.store.end_turn(TurnId(1));
    let revert = h
        .store
        .reject_file(Path::new("/work/space/dir/a.txt"))
        .unwrap();
    h.apply(revert);
    let report = h.store.report_patches(TurnId(1)).unwrap();
    assert!(
        report
            .user_patch
            .starts_with("diff --git a/dir/a.txt b/dir/a.txt\n"),
        "{}",
        report.user_patch
    );
}

#[test]
fn random_patches_apply() {
    let git = git_available();
    for seed in 0..200 {
        let mut rng = Rng::new(seed + 90_000);
        let old = random_text(&mut rng);
        let new = mutate(&mut rng, &old);
        let patch = unified_patch("f.txt", Some(&old), Some(&new));
        if old == new {
            assert!(patch.is_empty());
            continue;
        }
        assert_eq!(
            apply_patch(&old, &patch).as_deref(),
            Some(new.as_str()),
            "seed {seed}\n{patch}"
        );
        if git && seed % 20 == 0 {
            let result = git_apply(&[("f.txt", Some(&old))], &patch);
            assert_eq!(result[0].as_deref(), Some(new.as_str()), "seed {seed}");
        }
    }
}
