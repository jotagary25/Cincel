//! GPUI tests of the watcher's batches adopted in slices
//! (`docs/rendimiento.md`, E6-G, M11; `review_snapshot.rs`, `WatchQueue`).
//!
//! A burst of agent writes used to be adopted in one main-thread stretch
//! (31 ms for 200 files of the bench's medium project, 41 ms on the large
//! one). Now a batch of more than `WATCH_INLINE_PATHS` is queued and adopted
//! in slices of at most `WATCH_SLICE`;
//! `set_watch_slice_budget(Duration::ZERO)` makes every slice one
//! path, so the tests can see the queue deterministically. As in
//! `sweep_background_tests.rs`, `set_photo_in_background(true)` makes the
//! inert test project behave as the application does, and nothing queued
//! runs until the test parks.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{SessionId, StopReason};
use cincel_project::FsEvent;
use cincel_settings::Config;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::agents::Agents;
use crate::project::{Project, ProjectOptions};
use crate::review::{Review, WATCH_SLICE};
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

const FILES: usize = 30;
const BEFORE: &str = "a\nb\nc\nd\ne\n";
const AFTER: &str = "a\nB\nc\nd\ne\n";

fn init(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

fn project() -> (tempfile::TempDir, Vec<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let files = (0..FILES)
        .map(|index| {
            let path = dir.path().join(format!("f{index:02}.txt"));
            std::fs::write(&path, BEFORE).unwrap();
            path
        })
        .collect();
    (dir, files)
}

struct Parts {
    workspace: Entity<Workspace>,
    review: Entity<Review>,
    agents: Entity<Agents>,
}

fn window<'a>(root: &Path, cx: &'a mut TestAppContext) -> (Parts, &'a mut VisualTestContext) {
    let options = WorkspaceOptions {
        project: Some(root.to_path_buf()),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    let (review, agents) = workspace.read_with(cx, |workspace, _| {
        (workspace.review().clone(), workspace.agents().clone())
    });
    review.update(cx, |review, _| review.set_photo_in_background(true));
    (
        Parts {
            workspace,
            review,
            agents,
        },
        cx,
    )
}

fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(120));
    cx.run_until_parked();
}

fn start_turn(parts: &Parts, cx: &mut VisualTestContext) {
    parts
        .agents
        .update(cx, |agents, cx| agents.prepare_prompt(None, cx));
    settle(cx);
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.is_photo_ready())
    );
}

fn turn_ended_now(parts: &Parts, cx: &mut VisualTestContext) {
    parts.agents.update(cx, |agents, cx| {
        agents.handle_agent_event(
            AgentEvent::TurnEnded {
                session_id: SessionId::new("prueba"),
                stop_reason: StopReason::EndTurn,
            },
            cx,
        )
    });
}

fn project_of(parts: &Parts, cx: &mut VisualTestContext) -> Entity<Project> {
    parts
        .workspace
        .read_with(cx, |workspace, _| workspace.project().cloned())
        .expect("hay proyecto")
}

/// The agent writes every file and the watcher reports them in one batch.
fn agent_burst(parts: &Parts, files: &[PathBuf], cx: &mut VisualTestContext) {
    for path in files {
        std::fs::write(path, AFTER).unwrap();
    }
    let events: Vec<FsEvent> = files.iter().cloned().map(FsEvent::Modified).collect();
    project_of(parts, cx).update(cx, |project, cx| project.apply_fs_events(&events, cx));
}

fn tracked(parts: &Parts, cx: &mut VisualTestContext) -> usize {
    parts
        .review
        .read_with(cx, |review, _| review.store().files().count())
}

fn queued(parts: &Parts, cx: &mut VisualTestContext) -> usize {
    parts
        .review
        .read_with(cx, |review, _| review.watch_queue_len())
}

/// A burst is not adopted in one go: the handler queues it, the slices adopt
/// it, and in the end every file is in review.
#[gpui::test]
fn a_burst_is_adopted_in_slices(cx: &mut TestAppContext) {
    init(cx);
    let (dir, files) = project();
    let (parts, cx) = window(dir.path(), cx);
    parts.review.update(cx, |review, _| {
        review.set_watch_slice_budget(Duration::ZERO)
    });
    start_turn(&parts, cx);

    agent_burst(&parts, &files, cx);
    assert_eq!(tracked(&parts, cx), 0, "a burst waits for its slices");
    assert_eq!(queued(&parts, cx), FILES);

    settle(cx);
    assert_eq!(queued(&parts, cx), 0);
    assert_eq!(tracked(&parts, cx), FILES);
    let stats = parts.review.read_with(cx, |review, _| review.last_watch());
    assert_eq!(stats.paths, FILES);
    assert_eq!(stats.adopted, FILES);
    assert!(
        stats.main_slices > FILES,
        "one slice per path and one to refresh the views: {stats:?}"
    );
    let pending = parts
        .review
        .read_with(cx, |review, _| review.summary().borrow().files.len());
    assert_eq!(pending, FILES, "the views show the whole burst");
}

/// With the real budget, no slice takes much longer than it: the spec's
/// 8 ms (times `CINCEL_PERF_BUDGET_FACTOR`, D15) is never reached.
#[gpui::test]
fn every_slice_stays_within_the_budget(cx: &mut TestAppContext) {
    init(cx);
    let (dir, files) = project();
    let (parts, cx) = window(dir.path(), cx);
    start_turn(&parts, cx);

    agent_burst(&parts, &files, cx);
    settle(cx);
    assert_eq!(tracked(&parts, cx), FILES);
    let factor = std::env::var("CINCEL_PERF_BUDGET_FACTOR")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    let stats = parts.review.read_with(cx, |review, _| review.last_watch());
    assert!(
        stats.max_main_slice <= Duration::from_millis(8) * factor,
        "a slice took {:?} (budget {WATCH_SLICE:?})",
        stats.max_main_slice
    );
}

/// The turn ends while part of a burst is still queued: the sweep adopts
/// the rest before it closes the turn, nothing is lost.
#[gpui::test]
fn the_sweep_adopts_what_is_still_queued(cx: &mut TestAppContext) {
    init(cx);
    let (dir, files) = project();
    let (parts, cx) = window(dir.path(), cx);
    parts.review.update(cx, |review, _| {
        review.set_watch_slice_budget(Duration::ZERO)
    });
    start_turn(&parts, cx);

    agent_burst(&parts, &files, cx);
    assert!(queued(&parts, cx) > 0);
    turn_ended_now(&parts, cx);

    settle(cx);
    assert!(
        !parts
            .review
            .read_with(cx, |review, _| review.is_turn_active())
    );
    assert_eq!(queued(&parts, cx), 0);
    assert_eq!(tracked(&parts, cx), FILES);
}

/// Cincel writes a file whose agent change is still queued (a save right
/// after the burst): the agent's change is adopted first, against the photo,
/// so it is not folded into the base.
#[gpui::test]
fn a_host_write_adopts_the_queued_change_first(cx: &mut TestAppContext) {
    init(cx);
    let (dir, files) = project();
    let (parts, cx) = window(dir.path(), cx);
    parts.review.update(cx, |review, _| {
        review.set_watch_slice_budget(Duration::ZERO)
    });
    start_turn(&parts, cx);

    agent_burst(&parts, &files, cx);
    let last = files.last().unwrap().clone();
    assert!(
        !parts
            .review
            .read_with(cx, |review, _| review.store().file(&last).is_some()),
        "still queued"
    );
    project_of(&parts, cx).update(cx, |project, cx| project.note_host_write(&last, cx));
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.store().file(&last).is_some()),
        "adopted before the photo took Cincel's write"
    );

    settle(cx);
    assert_eq!(tracked(&parts, cx), FILES);
    let hunks = parts
        .review
        .read_with(cx, |review, _| review.store().hunks(&last).len());
    assert_eq!(hunks, 1, "the agent's line is still in review");
}
