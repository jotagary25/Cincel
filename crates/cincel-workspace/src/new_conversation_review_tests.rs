//! A new conversation starts from scratch (spec 13 §3.1, criteria A1-A4):
//! the patches the user rejected and the comments left unsent belong to the
//! conversation they were made in, so creating another one drops them, while
//! the hunks under review stay. Resuming a stored conversation drops nothing.
//!
//! Turns are run by the real fake agent (`FAKE_SHELL_EDITS` moves a staged
//! file onto the project) and `FAKE_PROMPT_LOG` records what it received.
//!
//! ```text
//! cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins
//! cargo test -p cincel-workspace --features test-support new_conversation_review
//! ```

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cincel_acp::AgentEvent;
use cincel_chat::{ChatEvent, ChatPanel, Entry, MessageBlock};
use cincel_editor::{EditorView, ReviewAction};
use cincel_settings::Config;
use cincel_text::Point;
use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};

use crate::agents::Agents;
use crate::project::ProjectOptions;
use crate::review::Review;
use crate::test_support::{FakeEnv, isolate_state};
use crate::workspace::{Workspace, WorkspaceOptions};

const CALC_TWO: &str = "def total(items, discount=0):\n    return sum(items) * (1 - discount)\n\n\ndef media(items):\n    return total(items) / len(items)\n\n\ndef maximo(items):\n    return max(items)\n";
const CALC_TWO_AGENT: &str = "def total(items):\n    return sum(items)\n\n\ndef media(items):\n    return total(items) / len(items)\n\n\ndef maximo(items):\n    return sorted(items)[-1]\n";

fn init(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

fn agent_budget() -> Duration {
    let factor = std::env::var("CINCEL_PERF_BUDGET_FACTOR")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    Duration::from_secs(30) * factor
}

/// A project (`proj/`), a staging folder (`stage/`) and the prompt log.
struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    stage: PathBuf,
    log: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let stage = dir.path().join("stage");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(root.join("calc.py"), CALC_TWO).unwrap();
        let log = dir.path().join("prompts.jsonl");
        Self {
            root,
            stage,
            log,
            _dir: dir,
        }
    }

    fn calc(&self) -> PathBuf {
        self.root.join("calc.py")
    }

    /// What the agent writes into `calc.py` on its next turn.
    fn stage(&self, content: &str) {
        std::fs::write(self.stage.join("calc.py"), content).unwrap();
    }

    /// The `<user_review_feedback>` block of the last prompt, if it had one.
    fn last_feedback(&self) -> Option<String> {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let line = text.lines().last().expect("el agente recibió un mensaje");
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        let first = value["prompt"].as_array()?.first()?.get("text")?.as_str()?;
        first
            .starts_with("<user_review_feedback>")
            .then(|| first.to_string())
    }
}

struct Parts {
    review: Entity<Review>,
    agents: Entity<Agents>,
    chat: Entity<ChatPanel>,
    center: Entity<crate::center::CenterPanel>,
}

fn window<'a>(root: &Path, cx: &'a mut TestAppContext) -> (Parts, &'a mut VisualTestContext) {
    let options = WorkspaceOptions {
        project: Some(root.to_path_buf()),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    let (review, agents, chat, center) = workspace.read_with(cx, |workspace, _| {
        (
            workspace.review().clone(),
            workspace.agents().clone(),
            workspace.chat().clone(),
            workspace.center().clone(),
        )
    });
    (
        Parts {
            review,
            agents,
            chat,
            center,
        },
        cx,
    )
}

fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(120));
    cx.run_until_parked();
}

/// Feeds the fake agent's events to the workspace until `stop` matches.
fn drain_until(parts: &Parts, cx: &mut VisualTestContext, stop: impl Fn(&AgentEvent) -> bool) {
    let events = parts
        .agents
        .read_with(cx, |agents, _| agents.connection_events())
        .expect("hay conexión");
    let deadline = Instant::now() + agent_budget();
    loop {
        match events.try_recv() {
            Ok(event) => {
                let done = stop(&event);
                parts
                    .agents
                    .update(cx, |agents, cx| agents.handle_agent_event(event, cx));
                settle(cx);
                if done {
                    return;
                }
            }
            Err(async_channel::TryRecvError::Empty) => {
                assert!(
                    Instant::now() < deadline,
                    "el agente falso no respondió a tiempo"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(async_channel::TryRecvError::Closed) => panic!("el agente falso se cerró"),
        }
    }
}

fn connect(parts: &Parts, fixture: &Fixture, cx: &mut VisualTestContext) -> FakeEnv {
    let log = fixture.log.display().to_string();
    let env = FakeEnv::with_agent_env(&[
        ("FAKE_SHELL_EDITS", "../stage/calc.py=>calc.py"),
        ("FAKE_PROMPT_LOG", &log),
    ]);
    let id = env.add_connection("claude-acp", "Fake");
    parts.agents.update(cx, |agents, cx| {
        agents.set_connections(env.connections.clone(), cx)
    });
    parts
        .chat
        .update(cx, |chat, cx| chat.select_connection(&id.to_string(), cx));
    cx.run_until_parked();
    wait_session(parts, cx);
    env
}

fn wait_session(parts: &Parts, cx: &mut VisualTestContext) {
    drain_until(parts, cx, |event| {
        matches!(event, AgentEvent::SessionCreated { .. })
    });
}

fn send_message(parts: &Parts, text: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        parts.chat.update(cx, |chat, cx| {
            chat.set_input_text(text, window, cx);
            chat.send(window, cx);
        })
    });
    settle(cx);
}

fn send_turn(parts: &Parts, text: &str, cx: &mut VisualTestContext) {
    send_message(parts, text, cx);
    drain_until(parts, cx, |event| {
        matches!(event, AgentEvent::TurnEnded { .. })
    });
    settle(cx);
}

/// "Nueva conversación", and the new session is up.
fn new_conversation(parts: &Parts, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        parts
            .chat
            .update(cx, |chat, cx| chat.new_session(window, cx))
    });
    settle(cx);
    wait_session(parts, cx);
}

fn open(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> Entity<EditorView> {
    cx.update(|window, cx| {
        parts
            .center
            .update(cx, |center, cx| center.open_file(path, true, window, cx));
    });
    settle(cx);
    let editor = parts
        .center
        .read_with(cx, |center, _| {
            center
                .tabs()
                .iter()
                .find(|tab| tab.path == path)
                .map(|tab| tab.editor().clone())
        })
        .expect("la pestaña está abierta");
    cx.update(|window, cx| {
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    });
    cx.run_until_parked();
    editor
}

fn hunk_ids(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> Vec<u64> {
    parts.review.read_with(cx, |review, _| {
        review
            .store()
            .hunks(path)
            .iter()
            .map(|hunk| hunk.id.0)
            .collect()
    })
}

fn reject(parts: &Parts, path: &Path, hunk: u64, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        parts.review.update(cx, |review, cx| {
            review.handle_action(path, ReviewAction::RejectHunk(hunk), None, window, cx)
        })
    });
    settle(cx);
}

fn comment_count(parts: &Parts, cx: &mut VisualTestContext) -> usize {
    parts
        .review
        .read_with(cx, |review, _| review.comment_count())
}

fn tag_count(parts: &Parts, cx: &mut VisualTestContext) -> usize {
    parts
        .chat
        .read_with(cx, |chat, _| chat.pending_comments().len())
}

fn card_count(parts: &Parts, cx: &mut VisualTestContext) -> usize {
    parts.chat.read_with(cx, |chat, _| {
        chat.entries()
            .iter()
            .map(|entry| match entry {
                Entry::UserMessage(message) => message
                    .blocks
                    .iter()
                    .filter(|block| matches!(block, MessageBlock::Comment(_)))
                    .count(),
                _ => 0,
            })
            .sum()
    })
}

/// One conversation with an agent turn that left two pending hunks in
/// `calc.py`.
fn first_turn(fixture: &Fixture, parts: &Parts, cx: &mut VisualTestContext) -> PathBuf {
    fixture.stage(CALC_TWO_AGENT);
    send_turn(parts, "cambiá calc", cx);
    let path = fixture.calc();
    assert_eq!(hunk_ids(parts, &path, cx).len(), 2);
    path
}

/// A1) Reject, new conversation, send: the prompt carries no feedback. The
/// hunk left pending stays there (R2).
#[gpui::test]
fn a_new_conversation_drops_the_rejected_patches(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new();
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, cx);
    let path = first_turn(&fixture, &parts, cx);
    let first = hunk_ids(&parts, &path, cx)[0];
    reject(&parts, &path, first, cx);
    new_conversation(&parts, cx);

    assert_eq!(
        hunk_ids(&parts, &path, cx).len(),
        1,
        "el otro segmento sigue"
    );
    send_turn(&parts, "hola", cx);
    assert_eq!(fixture.last_feedback(), None);
}

/// A2) Reject, new conversation, reject another, send: only the second.
#[gpui::test]
fn what_is_rejected_after_the_new_conversation_is_sent(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new();
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, cx);
    let path = first_turn(&fixture, &parts, cx);
    let first = hunk_ids(&parts, &path, cx)[0];
    reject(&parts, &path, first, cx);
    new_conversation(&parts, cx);
    let second = hunk_ids(&parts, &path, cx)[0];
    reject(&parts, &path, second, cx);

    send_turn(&parts, "seguí", cx);
    let feedback = fixture.last_feedback().expect("hay devolución");
    assert!(feedback.contains("maximo"), "{feedback}");
    assert!(!feedback.contains("def total"), "{feedback}");
}

/// A3) Unsent comments are dropped with the new conversation: no margin
/// mark, no tag, no card in the next message. The hunks stay pending.
#[gpui::test]
fn a_new_conversation_drops_the_unsent_comments(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new();
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, cx);
    let path = first_turn(&fixture, &parts, cx);
    let editor = open(&parts, &path, cx);
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(1, 0), cx));
    cx.simulate_keystrokes("ctrl-shift-m");
    cx.simulate_input("Dejá el descuento.");
    cx.simulate_keystrokes("ctrl-enter");
    settle(cx);
    assert_eq!(comment_count(&parts, cx), 1);
    assert_eq!(tag_count(&parts, cx), 1);

    new_conversation(&parts, cx);
    assert_eq!(comment_count(&parts, cx), 0);
    assert_eq!(tag_count(&parts, cx), 0);
    assert!(editor.read_with(cx, |editor, _| editor.comments().is_empty()));
    assert_eq!(hunk_ids(&parts, &path, cx).len(), 2, "los segmentos siguen");

    send_turn(&parts, "hola", cx);
    assert_eq!(fixture.last_feedback(), None);
    assert_eq!(card_count(&parts, cx), 0);
}

/// A4) Resuming a stored conversation keeps the queue.
#[gpui::test]
fn resuming_a_conversation_keeps_the_queue(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new();
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, cx);
    let path = first_turn(&fixture, &parts, cx);
    let first_conversation = parts
        .agents
        .read_with(cx, |agents, _| agents.conversation_binding())
        .expect("hay conversación")
        .0;
    new_conversation(&parts, cx);
    let first = hunk_ids(&parts, &path, cx)[0];
    reject(&parts, &path, first, cx);

    parts.chat.update(cx, |_, cx| {
        cx.emit(ChatEvent::OpenConversation {
            id: first_conversation,
        })
    });
    settle(cx);

    let feedback = parts
        .agents
        .update(cx, |agents, cx| agents.prepare_prompt(None, cx))
        .expect("la cola sigue");
    assert!(feedback.contains("def total"), "{feedback}");
}
