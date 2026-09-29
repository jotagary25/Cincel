//! GPUI tests of the binary files the agent touches: they are out of the
//! review (`review_snapshot.rs`, "Binaries"). A `.pyc` of `__pycache__`
//! rewritten and an image created by a shell script during the turn are
//! applied without asking: nothing pending (tree, "Revisar todo", status
//! bar, close dialog), and the disk and `git status` show the agent's
//! change. A text file of the same turn is reviewed as usual.
//!
//! The turn is run by the real fake agent with `FAKE_SHELL_EDITS` (it writes
//! on disk and names no path; the end-of-turn sweep finds the changes). The
//! variable only carries text, so the binary bytes are prepared outside the
//! project and the script moves them in (`ruta=>destino`, a real `mv`).
//!
//! ```text
//! cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins
//! cargo test -p cincel-workspace --features test-support review_binary_exclusion
//! ```

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use cincel_acp::acp::schema::v1::{SessionId, StopReason};
use cincel_acp::{AgentCommand, AgentEvent, PromptBlock};
use cincel_chat::ChatPanel;
use cincel_settings::Config;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::agents::Agents;
use crate::project::ProjectOptions;
use crate::review::Review;
use crate::test_support::{FakeEnv, isolate_state};
use crate::workspace::{Workspace, WorkspaceOptions};

const FIVE: &str = "a\nb\nc\nd\ne\n";
const FIVE_EDITED: &str = "a\nB\nc\nd\nE\n";
/// A compiled Python module: magic number, flags and marshalled code.
const PYC: [u8; 12] = [
    0xcb, 0x0d, 0x0d, 0x0a, 0x00, 0x00, 0x00, 0x00, 0xe3, 0x00, 0xff, 0x01,
];
/// The one the agent's run leaves.
const PYC_NEW: [u8; 12] = [
    0xcb, 0x0d, 0x0d, 0x0a, 0x00, 0x00, 0x00, 0x00, 0xe3, 0x02, 0xfe, 0x07,
];
/// The start of a PNG.
const PNG: [u8; 10] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00];

fn init(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `a.txt` and `__pycache__/modulo.cpython-312.pyc`, committed when this
/// machine has a usable `git` (`true` then).
fn project() -> (tempfile::TempDir, bool) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), FIVE).unwrap();
    std::fs::create_dir_all(dir.path().join("__pycache__")).unwrap();
    std::fs::write(dir.path().join("__pycache__/modulo.cpython-312.pyc"), PYC).unwrap();
    let committed = git(dir.path(), &["init", "-q"]).is_some()
        && git(dir.path(), &["config", "user.email", "test@cincel"]).is_some()
        && git(dir.path(), &["config", "user.name", "Test"]).is_some()
        && git(dir.path(), &["add", "."]).is_some()
        && git(dir.path(), &["commit", "-qm", "inicial"]).is_some();
    (dir, committed)
}

struct Parts {
    workspace: Entity<Workspace>,
    review: Entity<Review>,
    agents: Entity<Agents>,
    chat: Entity<ChatPanel>,
}

fn window<'a>(root: &Path, cx: &'a mut TestAppContext) -> (Parts, &'a mut VisualTestContext) {
    let options = WorkspaceOptions {
        project: Some(root.to_path_buf()),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    let (review, agents, chat) = workspace.read_with(cx, |workspace, _| {
        (
            workspace.review().clone(),
            workspace.agents().clone(),
            workspace.chat().clone(),
        )
    });
    (
        Parts {
            workspace,
            review,
            agents,
            chat,
        },
        cx,
    )
}

fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(120));
    cx.run_until_parked();
}

fn drain_until(parts: &Parts, cx: &mut VisualTestContext, stop: impl Fn(&AgentEvent) -> bool) {
    let events = parts
        .agents
        .read_with(cx, |agents, _| agents.connection_events())
        .expect("hay conexión");
    loop {
        let event = events.recv_blocking().expect("evento del agente falso");
        let done = stop(&event);
        parts
            .agents
            .update(cx, |agents, cx| agents.handle_agent_event(event, cx));
        settle(cx);
        if done {
            break;
        }
    }
}

/// Runs one shell turn of the fake agent (`FAKE_SHELL_EDITS = edits`) to
/// its end. Keep the returned [`FakeEnv`] alive for the whole test.
fn shell_turn(parts: &Parts, edits: &str, cx: &mut VisualTestContext) -> FakeEnv {
    let env = FakeEnv::with_agent_env(&[("FAKE_SHELL_EDITS", edits)]);
    let id = env.add_connection("claude-acp", "Fake");
    parts.agents.update(cx, |agents, cx| {
        agents.set_connections(env.connections.clone(), cx)
    });
    parts
        .chat
        .update(cx, |chat, cx| chat.select_connection(&id.to_string(), cx));
    cx.run_until_parked();
    drain_until(parts, cx, |event| {
        matches!(event, AgentEvent::SessionCreated { .. })
    });
    let session: SessionId = parts
        .chat
        .read_with(cx, |chat, _| chat.session_id().cloned().expect("sesión"));
    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(
            &AgentCommand::Prompt {
                session_id: session,
                blocks: vec![PromptBlock::Text("recompilá y agregá el logo".to_string())],
                feedback: None,
            },
            cx,
        )
    });
    drain_until(parts, cx, |event| {
        matches!(event, AgentEvent::TurnEnded { .. })
    });
    settle(cx);
    env
}

fn tree_label(parts: &Parts, relative: &str, cx: &mut VisualTestContext) -> Option<String> {
    parts.workspace.read_with(cx, |workspace, cx| {
        workspace
            .files()
            .read(cx)
            .review_label(Path::new(relative), cx)
    })
}

/// The `.pyc` rewritten and the image created by the agent's shell are
/// applied without review; `a.txt`, changed in the same turn, is reviewed.
#[gpui::test]
fn binaries_changed_by_a_shell_are_applied_without_review(cx: &mut TestAppContext) {
    init(cx);
    let (dir, committed) = project();
    let root = dir.path();
    // The agent's new bytes, outside the project, for its `mv`.
    let staging = tempfile::tempdir().unwrap();
    std::fs::write(staging.path().join("modulo.pyc"), PYC_NEW).unwrap();
    std::fs::write(staging.path().join("logo.png"), PNG).unwrap();
    let outside = |name: &str| {
        Path::new("..")
            .join(staging.path().file_name().unwrap())
            .join(name)
            .display()
            .to_string()
    };
    let pyc = root.join("__pycache__/modulo.cpython-312.pyc");
    let logo = root.join("logo.png");
    let a = root.join("a.txt");

    let (parts, cx) = window(root, cx);
    let edits = format!(
        "{}=>__pycache__/modulo.cpython-312.pyc;{}=>logo.png;a.txt={}",
        outside("modulo.pyc"),
        outside("logo.png"),
        FIVE_EDITED.replace('\n', "\\n")
    );
    let _env = shell_turn(&parts, &edits, cx);

    // The disk has what the agent left.
    assert_eq!(std::fs::read(&pyc).unwrap(), PYC_NEW);
    assert_eq!(std::fs::read(&logo).unwrap(), PNG);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), FIVE_EDITED);

    // The text file is in review (its two segments); the binaries are not.
    let (pending, counts, rows, files) = parts.review.read_with(cx, |review, _| {
        let summary = review.summary();
        let files: Vec<_> = summary.borrow().files.keys().cloned().collect();
        (
            summary.borrow().pending,
            review.pending_counts(),
            review.panel_rows(),
            files,
        )
    });
    assert_eq!(files, vec![a.clone()], "solo el texto está pendiente");
    assert_eq!(pending, 2, "la barra de estado cuenta los dos segmentos");
    assert_eq!(counts, (2, 1), "el diálogo de cierre pregunta por el texto");
    let listed: Vec<&str> = rows.iter().map(|row| row.relative.as_str()).collect();
    assert_eq!(listed, vec!["a.txt"], "«Revisar todo» lista solo el texto");
    assert!(tree_label(&parts, "__pycache__/modulo.cpython-312.pyc", cx).is_none());
    assert!(tree_label(&parts, "logo.png", cx).is_none());
    assert!(tree_label(&parts, "__pycache__", cx).is_none());
    assert_eq!(tree_label(&parts, "a.txt", cx).as_deref(), Some("+2 −2"));
    let root_label = parts.workspace.read_with(cx, |workspace, cx| {
        workspace.files().read(cx).root_review_label()
    });
    assert_eq!(root_label.as_deref(), Some("1 archivo · 2 cambios · +2 −2"));

    // Accepting everything leaves the binaries alone, as the agent left them.
    parts.review.update(cx, |review, cx| review.accept_all(cx));
    settle(cx);
    assert_eq!(
        parts
            .review
            .read_with(cx, |review, _| review.pending_count()),
        0
    );
    assert_eq!(std::fs::read(&pyc).unwrap(), PYC_NEW);
    assert_eq!(std::fs::read(&logo).unwrap(), PNG);

    // `git status` sees the agent's change to the binaries.
    if committed {
        let status =
            git(root, &["status", "--porcelain", "--untracked-files=all"]).expect("git status");
        assert!(
            status
                .lines()
                .any(|line| line == " M __pycache__/modulo.cpython-312.pyc"),
            "{status}"
        );
        assert!(status.lines().any(|line| line == "?? logo.png"), "{status}");
        assert!(status.lines().any(|line| line == " M a.txt"), "{status}");
    }
}

/// Rejecting the turn puts the text back and leaves the binaries as the
/// agent left them: they were never in the review.
#[gpui::test]
fn rejecting_the_turn_leaves_the_binaries_as_the_agent_left_them(cx: &mut TestAppContext) {
    init(cx);
    let (dir, _) = project();
    let root = dir.path();
    let pyc = root.join("__pycache__/modulo.cpython-312.pyc");
    let a = root.join("a.txt");
    let (parts, cx) = window(root, cx);

    // By hand this time, like `snapshot_review_tests.rs`: the photo, the
    // agent's writes, the end of the turn and its sweep.
    parts
        .agents
        .update(cx, |agents, cx| agents.prepare_prompt(None, cx));
    settle(cx);
    std::fs::write(&pyc, PYC_NEW).unwrap();
    std::fs::write(&a, FIVE_EDITED).unwrap();
    parts.agents.update(cx, |agents, cx| {
        agents.handle_agent_event(
            AgentEvent::TurnEnded {
                session_id: SessionId::new("prueba"),
                stop_reason: StopReason::EndTurn,
            },
            cx,
        )
    });
    settle(cx);
    assert_eq!(
        parts
            .review
            .read_with(cx, |review, _| review.pending_counts()),
        (2, 1)
    );

    parts.review.update(cx, |review, cx| review.reject_turn(cx));
    settle(cx);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), FIVE);
    assert_eq!(
        std::fs::read(&pyc).unwrap(),
        PYC_NEW,
        "el binario no se toca"
    );
    assert_eq!(
        parts
            .review
            .read_with(cx, |review, _| review.pending_count()),
        0
    );
}
