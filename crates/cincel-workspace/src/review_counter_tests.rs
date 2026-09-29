//! GPUI tests of the review counters: the file tree's root line and the
//! editor's floating bar must count the same things, before and after the
//! review is restored from disk.

use std::path::Path;
use std::time::Duration;

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{SessionId, StopReason};
use cincel_settings::Config;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::project::ProjectOptions;
use crate::review::Review;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

fn init(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(120));
    cx.run_until_parked();
}

/// Forty numbered lines.
fn forty() -> String {
    (1..=40).map(|n| format!("línea {n}\n")).collect()
}

/// `forty()` without three separate blocks of twelve lines (36 in all).
fn forty_without_three_blocks() -> String {
    (1..=40)
        .filter(|n| !((2..=13).contains(n) || (15..=26).contains(n) || (28..=39).contains(n)))
        .map(|n| format!("línea {n}\n"))
        .collect()
}

fn window<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let options = WorkspaceOptions {
        project: Some(root.to_path_buf()),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    (workspace, cx)
}

fn review(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<Review> {
    workspace.read_with(cx, |workspace, _| workspace.review().clone())
}

fn agent(workspace: &Entity<Workspace>, event: AgentEvent, cx: &mut VisualTestContext) {
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    agents.update(cx, |agents, cx| agents.handle_agent_event(event, cx));
    settle(cx);
}

/// A whole agent turn writing `content` into `path`.
fn agent_writes(
    workspace: &Entity<Workspace>,
    path: &Path,
    content: &str,
    cx: &mut VisualTestContext,
) {
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    agents.update(cx, |agents, cx| agents.prepare_prompt(None, cx));
    settle(cx);
    let (reply, mut answer) = tokio::sync::oneshot::channel();
    agent(
        workspace,
        AgentEvent::FsWrite {
            session_id: SessionId::new("prueba"),
            path: path.to_path_buf(),
            content: content.to_string(),
            reply,
        },
        cx,
    );
    assert!(matches!(answer.try_recv(), Ok(Ok(()))));
    agent(
        workspace,
        AgentEvent::TurnEnded {
            session_id: SessionId::new("prueba"),
            stop_reason: StopReason::EndTurn,
        },
        cx,
    );
}

/// `(pending decisions, files, (added, removed))` of the summary, and the
/// hunks the editor of `path` paints.
fn counters(
    workspace: &Entity<Workspace>,
    path: &Path,
    cx: &mut VisualTestContext,
) -> (usize, usize, (u32, u32), usize) {
    let review = review(workspace, cx);
    let (pending, files, total) = review.read_with(cx, |review, _| {
        let summary = review.summary();
        let summary = summary.borrow();
        (summary.pending, summary.files.len(), summary.total)
    });
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace
                .center()
                .clone()
                .update(cx, |center, cx| center.open_file(path, true, window, cx))
        })
    });
    settle(cx);
    let hunks = workspace.read_with(cx, |workspace, cx| {
        workspace
            .center()
            .read(cx)
            .tabs()
            .iter()
            .find(|tab| tab.path == path)
            .map(|tab| tab.editor().read(cx).review().hunks.len())
            .unwrap_or(0)
    });
    (pending, files, total, hunks)
}

#[gpui::test]
fn tree_and_bar_count_the_same_changes_before_and_after_a_reopen(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("largo.txt");
    std::fs::write(&path, forty()).unwrap();
    let (workspace, cx) = window(dir.path(), cx);

    agent_writes(&workspace, &path, &forty_without_three_blocks(), cx);
    let live = counters(&workspace, &path, cx);
    assert_eq!(live, (3, 1, (0, 36), 3), "en vivo");
    // The tree says what each number counts; the bar counts the same three
    // changes ("cambio N de 3").
    let label = workspace.read_with(cx, |workspace, cx| {
        workspace.files().read(cx).root_review_label()
    });
    assert_eq!(label.as_deref(), Some("1 archivo · 3 cambios · +0 −36"));

    // Without a tab: the restored file only has its background buffer.
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace
                .center()
                .clone()
                .update(cx, |center, cx| center.close_active(window, cx))
        })
    });
    settle(cx);
    let review = review(&workspace, cx);
    review.update(cx, |review, cx| review.save_now(cx));
    let root = dir.path().to_path_buf();
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_project(&root, window, cx)
        })
    });
    settle(cx);
    let restored = counters(&workspace, &path, cx);
    assert_eq!(restored, live, "restaurada");
}

#[gpui::test]
fn a_fresh_window_restores_the_same_counters(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("largo.txt");
    std::fs::write(&path, forty()).unwrap();
    let live = {
        let (workspace, cx) = window(dir.path(), cx);
        agent_writes(&workspace, &path, &forty_without_three_blocks(), cx);
        let live = counters(&workspace, &path, cx);
        let review = review(&workspace, cx);
        review.update(cx, |review, cx| review.save_now(cx));
        live
    };
    let (workspace, cx) = window(dir.path(), cx);
    settle(cx);
    let review = review(&workspace, cx);
    let before_tab = review.read_with(cx, |review, _| {
        let summary = review.summary();
        let summary = summary.borrow();
        (summary.pending, summary.files.len(), summary.total)
    });
    assert_eq!(before_tab, (live.0, live.1, live.2), "sin pestaña");
    let restored = counters(&workspace, &path, cx);
    assert_eq!(restored, live, "ventana nueva");
}
