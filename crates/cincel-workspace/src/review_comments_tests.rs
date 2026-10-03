//! End-to-end tests of the comments for the agent in the workspace
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §6.7, the
//! scenarios a–m of §6.9 and the criteria of §6.10).
//!
//! Every turn is run by the real fake agent: `FAKE_SHELL_EDITS` makes it
//! change files straight on disk (a rename of a file staged next to the
//! project, so each turn can bring a different text), and `FAKE_PROMPT_LOG`
//! records exactly what it received, so the `<user_review_feedback>` block is
//! compared byte for byte. Comments are made through the real editor: the
//! pill's "Comentar" clicked, `Ctrl+Shift+M`, typing in the box and
//! `Ctrl+Enter`; messages go out through the chat's own `send`.
//!
//! ```text
//! cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins
//! cargo test -p cincel-workspace --features test-support review_comments
//! ```

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{SessionId, StopReason};
use cincel_chat::{ChatPanel, Entry, MessageBlock, SentCommentCard, SentCommentKind};
use cincel_editor::{EditorView, ReviewAction, ReviewCommentView, ReviewHunkView};
use cincel_review::CommentId;
use cincel_settings::Config;
use cincel_text::Point;
use gpui::{
    Entity, Focusable, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px,
};

use crate::agents::Agents;
use crate::center::CenterPanel;
use crate::project::ProjectOptions;
use crate::review::{Review, close_comments_line, status_label};
use crate::test_support::{FakeEnv, isolate_state};
use crate::workspace::{Workspace, WorkspaceOptions};

// ------------------------------------------------------------------ texts

const CALC: &str = "def total(items, discount=0):\n    return sum(items) * (1 - discount)\n\n\ndef media(items):\n    return total(items) / len(items)\n";
const CALC_AGENT: &str = "def total(items):\n    return sum(items)\n\n\ndef media(items):\n    return total(items) / len(items)\n";
const CALC_AGENT_2: &str = "def total(items, rate):\n    return sum(items) * rate\n\n\ndef media(items):\n    return total(items) / len(items)\n";

const CALC_TWO: &str = "def total(items, discount=0):\n    return sum(items) * (1 - discount)\n\n\ndef media(items):\n    return total(items) / len(items)\n\n\ndef maximo(items):\n    return max(items)\n";
const CALC_TWO_AGENT: &str = "def total(items):\n    return sum(items)\n\n\ndef media(items):\n    return total(items) / len(items)\n\n\ndef maximo(items):\n    return sorted(items)[-1]\n";
const CALC_TWO_FIX: &str = "def total(items):\n    return float(sum(items))\n\n\ndef media(items):\n    return total(items) / len(items)\n\n\ndef maximo(items):\n    return sorted(items)[-1]\n";

const UTIL: &str = "import os\n\nHOST = \"localhost\"\nPORT = 8080\n\n\nTIMEOUT = 3\n";
const FIVE: &str = "uno = 1\ndos = 2\ntres = 3\ncuatro = 4\ncinco = 5\n";

const LINES: &str = "a = 1\nb = 2\n\nc = 3\nd = 4\n\ne = 5\nf = 6\n\ng = 7\nh = 8\n";
const LINES_AGENT: &str = "a = 10\nb = 20\n\nc = 3\nd = 4\n\ne = 5\nf = 6\n\ng = 7\nh = 8\n";
const LINES_AGENT_2: &str =
    "a = 10\nb = 20\n\nc = 30\nd = 40\n\ne = 50\nf = 60\n\ng = 70\nh = 80\n";

const SECTION: &str = "The user left 1 comment on the code. Line numbers are 1-based and refer to each file as it is now. Read every comment together with its code and act on it in this turn.";

// ---------------------------------------------------------------- fixture

fn init(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

/// How long a test waits for the fake agent (a real process), times
/// `CINCEL_PERF_BUDGET_FACTOR`.
fn agent_budget() -> Duration {
    let factor = std::env::var("CINCEL_PERF_BUDGET_FACTOR")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    Duration::from_secs(30) * factor
}

/// A project (`proj/`), a staging folder next to it (`stage/`, where each
/// turn's new text waits for the agent's `mv`) and the prompt log.
struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    stage: PathBuf,
    log: PathBuf,
}

impl Fixture {
    fn new(files: &[(&str, &[u8])]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let stage = dir.path().join("stage");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&stage).unwrap();
        for (name, content) in files {
            std::fs::write(root.join(name), content).unwrap();
        }
        let log = dir.path().join("prompts.jsonl");
        Self {
            root,
            stage,
            log,
            _dir: dir,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// What the agent writes into `name` on its next turn.
    fn stage(&self, name: &str, content: &[u8]) {
        std::fs::write(self.stage.join(name), content).unwrap();
    }

    /// `FAKE_SHELL_EDITS` moving every staged file of `names` onto the
    /// project (a missing one is skipped by the agent: no change).
    fn edits(names: &[&str]) -> String {
        names
            .iter()
            .map(|name| format!("../stage/{name}=>{name}"))
            .collect::<Vec<_>>()
            .join(";")
    }

    /// Every prompt the agent received, as JSON blocks.
    fn prompts(&self) -> Vec<Vec<serde_json::Value>> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let value: serde_json::Value = serde_json::from_str(line).unwrap();
                value["prompt"].as_array().cloned().unwrap_or_default()
            })
            .collect()
    }

    /// The `<user_review_feedback>` block of the last prompt, if it had one.
    fn last_feedback(&self) -> Option<String> {
        let prompts = self.prompts();
        let last = prompts.last().expect("el agente recibió un mensaje");
        let first = last.first()?.get("text")?.as_str()?.to_string();
        first.starts_with("<user_review_feedback>").then_some(first)
    }

    /// The text the user typed in the last prompt (its last text block).
    fn last_user_text(&self) -> String {
        let prompts = self.prompts();
        let last = prompts.last().expect("el agente recibió un mensaje");
        last.iter()
            .rev()
            .find_map(|block| block.get("text").and_then(|text| text.as_str()))
            .unwrap_or_default()
            .to_string()
    }
}

struct Parts {
    workspace: Entity<Workspace>,
    review: Entity<Review>,
    center: Entity<CenterPanel>,
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
    let (review, center, agents, chat) = workspace.read_with(cx, |workspace, _| {
        (
            workspace.review().clone(),
            workspace.center().clone(),
            workspace.agents().clone(),
            workspace.chat().clone(),
        )
    });
    (
        Parts {
            workspace,
            review,
            center,
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

/// Lets the debounced save of the review run.
fn let_persist(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(1100));
    cx.run_until_parked();
}

/// Opens `path` in a pinned tab, focuses its editor and turns its render
/// probe on.
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
    focus(&editor, cx);
    editor.update(cx, |editor, _| editor.set_render_probe(true));
    editor
}

fn focus(editor: &Entity<EditorView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    });
    cx.run_until_parked();
}

fn set_cursor(editor: &Entity<EditorView>, row: u32, column: u32, cx: &mut VisualTestContext) {
    focus(editor, cx);
    editor.update(cx, |editor, cx| {
        editor.set_cursor(Point::new(row, column), cx)
    });
    cx.run_until_parked();
}

/// Forces a repaint and returns the frame it painted.
fn frame(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> cincel_editor::FrameRender {
    editor.update(cx, |editor, cx| {
        editor.take_render_frames();
        cx.notify();
    });
    cx.run_until_parked();
    editor
        .update(cx, |editor, _| editor.take_render_frames())
        .pop()
        .expect("se pintó un cuadro")
}

fn click_at(cx: &mut VisualTestContext, bounds: gpui::Bounds<gpui::Pixels>) {
    let center = bounds.center();
    cx.simulate_mouse_move(center, None, Modifiers::default());
    cx.run_until_parked();
    cx.simulate_click(center, Modifiers::default());
    cx.run_until_parked();
}

fn click_selector(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} está pintado"));
    click_at(cx, bounds);
}

/// Types `text` in the open comment box and saves it with `Ctrl+Enter`.
fn write_and_save(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    cx.simulate_keystrokes("ctrl-enter");
    settle(cx);
}

/// `Ctrl+Shift+M` with the cursor at `row` (no selection), then the text.
fn comment_line(editor: &Entity<EditorView>, row: u32, text: &str, cx: &mut VisualTestContext) {
    set_cursor(editor, row, 0, cx);
    cx.simulate_keystrokes("ctrl-shift-m");
    assert!(
        editor.read_with(cx, |editor, _| editor.comment_draft().is_some()),
        "la caja se abrió"
    );
    write_and_save(text, cx);
}

/// The pill's "Comentar" of the hunk under the cursor at `row`, clicked.
fn click_comentar(editor: &Entity<EditorView>, row: u32, cx: &mut VisualTestContext) {
    set_cursor(editor, row, 0, cx);
    let painted = frame(editor, cx);
    let pill = *painted
        .review
        .pills
        .first()
        .expect("los botones del segmento están a la vista");
    click_at(cx, pill.comment_bounds);
    assert!(
        editor.read_with(cx, |editor, _| editor.comment_draft().is_some()),
        "«Comentar» abrió la caja"
    );
}

fn hunks(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> Vec<ReviewHunkView> {
    editor.read_with(cx, |editor, _| editor.review().hunks.clone())
}

fn hunk_at(editor: &Entity<EditorView>, row: u32, cx: &mut VisualTestContext) -> ReviewHunkView {
    hunks(editor, cx)
        .into_iter()
        .find(|hunk| hunk.buffer_rows.contains(&row))
        .unwrap_or_else(|| panic!("hay un segmento en la fila {row}"))
}

fn editor_comments(
    editor: &Entity<EditorView>,
    cx: &mut VisualTestContext,
) -> Vec<ReviewCommentView> {
    editor.read_with(cx, |editor, _| editor.comments().to_vec())
}

fn decide(parts: &Parts, path: &Path, action: ReviewAction, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        parts.review.update(cx, |review, cx| {
            review.handle_action(path, action, None, window, cx)
        })
    });
    settle(cx);
}

/// Accepts or rejects the line of the hunk shown at buffer `row`.
fn decide_line(
    parts: &Parts,
    editor: &Entity<EditorView>,
    path: &Path,
    row: u32,
    accept: bool,
    cx: &mut VisualTestContext,
) {
    let hunk = hunk_at(editor, row, cx);
    let line = hunk
        .lines
        .iter()
        .position(|line| line.buffer_row == Some(row))
        .expect("la línea está en el segmento");
    let action = if accept {
        ReviewAction::AcceptLine {
            hunk: hunk.id,
            line,
        }
    } else {
        ReviewAction::RejectLine {
            hunk: hunk.id,
            line,
        }
    };
    decide(parts, path, action, cx);
}

fn comment_count(parts: &Parts, cx: &mut VisualTestContext) -> usize {
    parts
        .review
        .read_with(cx, |review, _| review.comment_count())
}

fn tags(parts: &Parts, cx: &mut VisualTestContext) -> Vec<String> {
    parts.chat.read_with(cx, |chat, _| {
        chat.pending_comments()
            .iter()
            .map(|tag| tag.label.clone())
            .collect()
    })
}

fn toasts(parts: &Parts, cx: &mut VisualTestContext) -> Vec<String> {
    parts.workspace.read_with(cx, |workspace, cx| {
        workspace
            .toasts()
            .read(cx)
            .items()
            .iter()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

/// The comment cards of the last message the user sent.
fn last_cards(parts: &Parts, cx: &mut VisualTestContext) -> Vec<SentCommentCard> {
    parts.chat.read_with(cx, |chat, _| {
        chat.entries()
            .iter()
            .rev()
            .find_map(|entry| match entry {
                Entry::UserMessage(message) => Some(
                    message
                        .blocks
                        .iter()
                        .filter_map(|block| match block {
                            MessageBlock::Comment(card) => Some(card.clone()),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    })
}

// ------------------------------------------------------------ the agent

/// Connects the fake Claude with `edits` and the prompt log. Keep the
/// returned [`FakeEnv`] alive for the whole test.
fn connect(parts: &Parts, fixture: &Fixture, edits: &str, cx: &mut VisualTestContext) -> FakeEnv {
    let log = fixture.log.display().to_string();
    let env = FakeEnv::with_agent_env(&[("FAKE_SHELL_EDITS", edits), ("FAKE_PROMPT_LOG", &log)]);
    let id = env.add_connection("claude-acp", "Fake");
    parts.agents.update(cx, |agents, cx| {
        agents.set_connections(env.connections.clone(), cx)
    });
    select(parts, &id.to_string(), cx);
    env
}

fn select(parts: &Parts, id: &str, cx: &mut VisualTestContext) {
    parts
        .chat
        .update(cx, |chat, cx| chat.select_connection(id, cx));
    cx.run_until_parked();
    drain_until(parts, cx, |event| {
        matches!(event, AgentEvent::SessionCreated { .. })
    });
}

/// Feeds the fake agent's events to the workspace until `stop` matches,
/// waiting at most [`agent_budget`].
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

/// Writes `text` in the chat and sends it, the user's way.
fn send_message(parts: &Parts, text: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        parts.chat.update(cx, |chat, cx| {
            chat.set_input_text(text, window, cx);
            chat.send(window, cx);
        })
    });
    settle(cx);
}

/// Sends `text` and lets the whole turn run.
fn send_turn(parts: &Parts, text: &str, cx: &mut VisualTestContext) {
    send_message(parts, text, cx);
    drain_until(parts, cx, |event| {
        matches!(event, AgentEvent::TurnEnded { .. })
    });
    settle(cx);
}

/// `<user_review_feedback>` around `inside`, as `cincel-acp` wraps it.
fn block(inside: &str) -> String {
    format!("<user_review_feedback>\n{inside}\n</user_review_feedback>")
}

// ===================================================================== a

/// a) A comment on a pending hunk, made with a real click on "Comentar",
/// reaches the agent alone (no patch); its fix keeps the hunk against the
/// original base and leaves the other hunk of the file as it was.
#[gpui::test]
fn comment_on_pending_hunk_reaches_agent_and_its_fix_updates_the_same_hunk(
    cx: &mut TestAppContext,
) {
    init(cx);
    let fixture = Fixture::new(&[("calc.py", CALC_TWO.as_bytes())]);
    let calc = fixture.path("calc.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["calc.py"]), cx);
    fixture.stage("calc.py", CALC_TWO_AGENT.as_bytes());
    send_turn(&parts, "cambiá calc", cx);

    let editor = open(&parts, &calc, cx);
    let before = hunks(&editor, cx);
    assert_eq!(before.len(), 2, "{before:?}");
    click_comentar(&editor, 0, cx);
    write_and_save("Usá un diccionario para los descuentos.", cx);
    assert_eq!(comment_count(&parts, cx), 1);
    assert_eq!(tags(&parts, cx), vec!["calc.py:1-2".to_string()]);

    fixture.stage("calc.py", CALC_TWO_FIX.as_bytes());
    send_turn(&parts, "atendé el comentario", cx);
    assert_eq!(
        fixture.last_feedback().as_deref(),
        Some(
            block(&format!(
                "{SECTION}\n\nComment 1 of 1\nFile: calc.py\nLines: 1-2\nReview state: your change, not reviewed yet\nCurrent code:\n```py\ndef total(items):\n    return sum(items)\n```\nComment:\nUsá un diccionario para los descuentos."
            ))
            .as_str()
        )
    );
    assert_eq!(fixture.last_user_text(), "atendé el comentario");

    // The agent's fix landed in the same hunk, still against the original
    // base; the other hunk did not move.
    assert_eq!(std::fs::read_to_string(&calc).unwrap(), CALC_TWO_FIX);
    let after = hunks(&editor, cx);
    assert_eq!(after.len(), 2, "{after:?}");
    assert_eq!(
        after[0].deleted_lines,
        vec![
            "def total(items, discount=0):".to_string(),
            "    return sum(items) * (1 - discount)".to_string()
        ]
    );
    assert_eq!(after[0].buffer_rows, 0..2);
    assert_eq!(after[1].deleted_lines, before[1].deleted_lines);
    assert_eq!(after[1].buffer_rows, before[1].buffer_rows);
    assert_eq!(after[1].kind, before[1].kind);
    assert!(editor_comments(&editor, cx).is_empty(), "ya se envió");
}

// ===================================================================== b

/// b) Comment and reject: the patch of the reject and the note, with the
/// restored code and the state `rejected`.
#[gpui::test]
fn comment_and_reject_sends_patch_and_note(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("calc.py", CALC.as_bytes())]);
    let calc = fixture.path("calc.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["calc.py"]), cx);
    fixture.stage("calc.py", CALC_AGENT.as_bytes());
    send_turn(&parts, "cambiá calc", cx);

    let editor = open(&parts, &calc, cx);
    // `Ctrl+Shift+M` inside a pending hunk is its "Comentar".
    comment_line(&editor, 1, "Dejá el descuento.", cx);
    let hunk = hunk_at(&editor, 0, cx);
    assert_eq!(
        editor_comments(&editor, cx)[0].from_hunk,
        Some(hunk.id),
        "hecho desde el segmento"
    );
    decide(&parts, &calc, ReviewAction::RejectHunk(hunk.id), cx);
    assert_eq!(std::fs::read_to_string(&calc).unwrap(), CALC);
    assert_eq!(editor_comments(&editor, cx).len(), 1, "sigue en el margen");

    send_turn(&parts, "seguí", cx);
    assert_eq!(
        fixture.last_feedback().as_deref(),
        Some(
            block(&format!(
                "The user made the following updates to your changes:\n\ndiff --git a/calc.py b/calc.py\n--- a/calc.py\n+++ b/calc.py\n@@ -1,5 +1,5 @@\n-def total(items):\n-    return sum(items)\n+def total(items, discount=0):\n+    return sum(items) * (1 - discount)\n \n \n def media(items):\n\n{SECTION}\n\nComment 1 of 1\nFile: calc.py\nLines: 1-2\nReview state: your change, rejected by the user\nCurrent code:\n```py\ndef total(items, discount=0):\n    return sum(items) * (1 - discount)\n```\nComment:\nDejá el descuento."
            ))
            .as_str()
        )
    );
    let cards = last_cards(&parts, cx);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].state_label, "rechazado");
    assert_eq!(cards[0].display_path, "calc.py");
    assert_eq!((cards[0].first_line, cards[0].last_line), (1, 2));
    assert_eq!(cards[0].text, "Dejá el descuento.");
    assert!(!cards[0].not_sent);
}

// ===================================================================== c

/// c) Comment and accept: only the note travels (no `REPORT_HEADER`),
/// state `accepted`.
#[gpui::test]
fn comment_and_accept_sends_only_the_note(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("calc.py", CALC.as_bytes())]);
    let calc = fixture.path("calc.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["calc.py"]), cx);
    fixture.stage("calc.py", CALC_AGENT.as_bytes());
    send_turn(&parts, "cambiá calc", cx);

    let editor = open(&parts, &calc, cx);
    click_comentar(&editor, 0, cx);
    write_and_save("Bien, pero documentalo.", cx);
    let hunk = hunk_at(&editor, 0, cx);
    decide(&parts, &calc, ReviewAction::AcceptHunk(hunk.id), cx);
    assert_eq!(editor_comments(&editor, cx).len(), 1, "sigue en el margen");

    send_turn(&parts, "seguí", cx);
    let feedback = fixture.last_feedback().expect("hay bloque");
    assert!(!feedback.contains(cincel_review::REPORT_HEADER));
    assert_eq!(
        feedback,
        block(&format!(
            "{SECTION}\n\nComment 1 of 1\nFile: calc.py\nLines: 1-2\nReview state: your change, accepted by the user\nCurrent code:\n```py\ndef total(items):\n    return sum(items)\n```\nComment:\nBien, pero documentalo."
        ))
    );
    assert_eq!(last_cards(&parts, cx)[0].state_label, "aceptado");
}

// ===================================================================== d

/// d) Comment and edit the lines by hand: the patch of the edit and the
/// note with the edited code, `not reviewed yet`; unsaved, the "as shown in
/// the editor" variant; saved, the plain one.
#[gpui::test]
fn comment_and_manual_edit_sends_patch_and_current_code(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("calc.py", CALC.as_bytes())]);
    let calc = fixture.path("calc.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["calc.py"]), cx);
    fixture.stage("calc.py", CALC_AGENT.as_bytes());
    send_turn(&parts, "cambiá calc", cx);

    let editor = open(&parts, &calc, cx);
    comment_line(&editor, 0, "Así no.", cx);
    // The user types at the end of the hunk's second line, without saving.
    set_cursor(&editor, 1, "    return sum(items)".len() as u32, cx);
    cx.simulate_input(" + 0");
    settle(cx);
    assert!(editor.read_with(cx, |editor, _| editor.is_dirty()));

    send_turn(&parts, "mirá lo que cambié", cx);
    assert_eq!(
        fixture.last_feedback().as_deref(),
        Some(
            block(&format!(
                "The user made the following updates to your changes:\n\ndiff --git a/calc.py b/calc.py\n--- a/calc.py\n+++ b/calc.py\n@@ -1,5 +1,5 @@\n def total(items):\n-    return sum(items)\n+    return sum(items) + 0\n \n \n def media(items):\n\n{SECTION}\n\nComment 1 of 1\nFile: calc.py\nLines: 1-2\nReview state: your change, not reviewed yet\nCurrent code (as shown in the editor, not saved yet):\n```py\ndef total(items):\n    return sum(items) + 0\n```\nComment:\nAsí no."
            ))
            .as_str()
        )
    );

    // Saved, the code is just "Current code".
    focus(&editor, cx);
    cx.simulate_keystrokes("ctrl-s");
    settle(cx);
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty()));
    comment_line(&editor, 1, "Ahora sí.", cx);
    send_turn(&parts, "guardé", cx);
    assert_eq!(
        fixture.last_feedback().as_deref(),
        Some(
            block(&format!(
                "{SECTION}\n\nComment 1 of 1\nFile: calc.py\nLines: 1-2\nReview state: your change, not reviewed yet\nCurrent code:\n```py\ndef total(items):\n    return sum(items) + 0\n```\nComment:\nAhora sí."
            ))
            .as_str()
        )
    );
}

// ===================================================================== e

/// e) A selection the agent never touched, commented with `Ctrl+Shift+M`:
/// `no change of yours…`, no patch.
#[gpui::test]
fn comment_on_untouched_selection(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("util.py", UTIL.as_bytes())]);
    let util = fixture.path("util.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["util.py"]), cx);

    let editor = open(&parts, &util, cx);
    // Rows 2 and 3 selected (the selection ends at column 0 of row 4).
    set_cursor(&editor, 2, 0, cx);
    cx.simulate_keystrokes("shift-down shift-down");
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-shift-m");
    write_and_save("Esto debería salir de la configuración.", cx);
    assert_eq!(tags(&parts, cx), vec!["util.py:3-4".to_string()]);

    send_turn(&parts, "revisá util", cx);
    assert_eq!(
        fixture.last_feedback().as_deref(),
        Some(
            block(&format!(
                "{SECTION}\n\nComment 1 of 1\nFile: util.py\nLines: 3-4\nReview state: no change of yours (the user selected these lines)\nCurrent code:\n```py\nHOST = \"localhost\"\nPORT = 8080\n```\nComment:\nEsto debería salir de la configuración."
            ))
            .as_str()
        )
    );
    assert_eq!(
        last_cards(&parts, cx)[0].state_label,
        "sin cambios del agente"
    );
}

// ===================================================================== f

/// f) Several comments in several files: sorted by path and line, numbered
/// in order, and the tags in the same order.
#[gpui::test]
fn several_comments_are_sorted_by_file_and_line(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("a.py", FIVE.as_bytes()), ("b.py", FIVE.as_bytes())]);
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["a.py"]), cx);

    let b = open(&parts, &fixture.path("b.py"), cx);
    comment_line(&b, 4, "tercero", cx);
    let a = open(&parts, &fixture.path("a.py"), cx);
    comment_line(&a, 2, "segundo", cx);
    comment_line(&a, 0, "primero", cx);
    assert_eq!(
        tags(&parts, cx),
        vec![
            "a.py:1".to_string(),
            "a.py:3".to_string(),
            "b.py:5".to_string()
        ]
    );

    send_turn(&parts, "tres notas", cx);
    let state = "Review state: no change of yours (the user selected these lines)";
    assert_eq!(
        fixture.last_feedback().as_deref(),
        Some(
            block(&format!(
                "The user left 3 comments on the code. Line numbers are 1-based and refer to each file as it is now. Read every comment together with its code and act on it in this turn.\n\nComment 1 of 3\nFile: a.py\nLine: 1\n{state}\nCurrent code:\n```py\nuno = 1\n```\nComment:\nprimero\n\nComment 2 of 3\nFile: a.py\nLine: 3\n{state}\nCurrent code:\n```py\ntres = 3\n```\nComment:\nsegundo\n\nComment 3 of 3\nFile: b.py\nLine: 5\n{state}\nCurrent code:\n```py\ncinco = 5\n```\nComment:\ntercero"
            ))
            .as_str()
        )
    );
    let order: Vec<(String, u32)> = last_cards(&parts, cx)
        .into_iter()
        .map(|card| (card.display_path, card.first_line))
        .collect();
    assert_eq!(
        order,
        vec![
            ("a.py".to_string(), 1),
            ("a.py".to_string(), 3),
            ("b.py".to_string(), 5)
        ]
    );
}

// ===================================================================== g

/// g) Comment, then decide with "Aceptar todo" and with the confirmed
/// "Rechazar todo": the state is the last decision's, the comment stays in
/// the margin after deciding.
#[gpui::test]
fn decide_after_commenting_including_accept_all_and_reject_all(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("calc.py", CALC.as_bytes())]);
    let calc = fixture.path("calc.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["calc.py"]), cx);
    fixture.stage("calc.py", CALC_AGENT.as_bytes());
    send_turn(&parts, "cambiá calc", cx);

    let editor = open(&parts, &calc, cx);
    comment_line(&editor, 0, "primera nota", cx);
    parts.review.update(cx, |review, cx| review.accept_all(cx));
    settle(cx);
    assert!(hunks(&editor, cx).is_empty());
    assert_eq!(editor_comments(&editor, cx).len(), 1, "sigue en el margen");

    fixture.stage("calc.py", CALC_AGENT_2.as_bytes());
    send_turn(&parts, "otra vez", cx);
    assert_eq!(
        fixture.last_feedback().as_deref(),
        Some(
            block(&format!(
                "{SECTION}\n\nComment 1 of 1\nFile: calc.py\nLines: 1-2\nReview state: your change, accepted by the user\nCurrent code:\n```py\ndef total(items):\n    return sum(items)\n```\nComment:\nprimera nota"
            ))
            .as_str()
        )
    );

    comment_line(&editor, 0, "segunda nota", cx);
    // The floating bar's "Rechazar todo", confirmed.
    decide(&parts, &calc, ReviewAction::RejectTurn, cx);
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.reject_turn_prompt().is_some())
    );
    cx.update(|window, cx| {
        parts
            .review
            .update(cx, |review, cx| review.confirm_reject_turn(window, cx))
    });
    settle(cx);
    assert_eq!(std::fs::read_to_string(&calc).unwrap(), CALC_AGENT);
    assert_eq!(editor_comments(&editor, cx).len(), 1, "sigue en el margen");

    send_turn(&parts, "y ahora", cx);
    assert_eq!(
        fixture.last_feedback().as_deref(),
        Some(
            block(&format!(
                "The user made the following updates to your changes:\n\ndiff --git a/calc.py b/calc.py\n--- a/calc.py\n+++ b/calc.py\n@@ -1,5 +1,5 @@\n-def total(items, rate):\n-    return sum(items) * rate\n+def total(items):\n+    return sum(items)\n \n \n def media(items):\n\n{SECTION}\n\nComment 1 of 1\nFile: calc.py\nLines: 1-2\nReview state: your change, rejected by the user\nCurrent code:\n```py\ndef total(items):\n    return sum(items)\n```\nComment:\nsegunda nota"
            ))
            .as_str()
        )
    );
}

// ===================================================================== h

/// h) A comment deleted before sending, with the tag's `×` or with "Borrar"
/// in the margin (real clicks), sends nothing: no block at all.
#[gpui::test]
fn deleted_comment_sends_nothing(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("util.py", UTIL.as_bytes())]);
    let util = fixture.path("util.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["util.py"]), cx);
    let editor = open(&parts, &util, cx);

    // The `×` of the tag in the chat.
    comment_line(&editor, 6, "se va por la cruz", cx);
    assert_eq!(tags(&parts, cx).len(), 1);
    click_selector(cx, "comment-tag-remove-0");
    settle(cx);
    assert_eq!(comment_count(&parts, cx), 0);
    assert!(editor_comments(&editor, cx).is_empty(), "y del margen");
    send_turn(&parts, "hola", cx);
    assert_eq!(fixture.last_feedback(), None);
    assert_eq!(fixture.last_user_text(), "hola");
    assert!(last_cards(&parts, cx).is_empty());

    // "Borrar" of the box in the margin.
    comment_line(&editor, 6, "se va por el margen", cx);
    let id = editor_comments(&editor, cx)[0].id;
    let selector: &'static str = Box::leak(format!("comment-block-{id}").into_boxed_str());
    let delete: &'static str = Box::leak(format!("comment-delete-{id}").into_boxed_str());
    frame(&editor, cx);
    let block_bounds = cx.debug_bounds(selector).expect("la caja plegada");
    cx.simulate_mouse_move(block_bounds.center(), None, Modifiers::default());
    cx.run_until_parked();
    frame(&editor, cx);
    click_selector(cx, delete);
    settle(cx);
    assert_eq!(comment_count(&parts, cx), 0);
    assert!(tags(&parts, cx).is_empty(), "y de la caja del chat");
    send_turn(&parts, "chau", cx);
    assert_eq!(fixture.last_feedback(), None);
}

// ===================================================================== i

/// i) Line decisions on commented hunks: with lines still pending the
/// comment stays on them and travels `not reviewed yet`; with every line
/// decided, `accepted`, `rejected` or `partly…`.
#[gpui::test]
fn line_decisions_on_a_commented_hunk(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("lines.py", LINES.as_bytes())]);
    let path = fixture.path("lines.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["lines.py"]), cx);
    fixture.stage("lines.py", LINES_AGENT.as_bytes());
    send_turn(&parts, "cambiá a y b", cx);

    let editor = open(&parts, &path, cx);
    comment_line(&editor, 0, "a y b", cx);
    decide_line(&parts, &editor, &path, 0, true, cx);
    assert_eq!(hunks(&editor, cx).len(), 1, "queda la línea de b");
    assert_eq!(
        editor_comments(&editor, cx)[0].rows,
        0..2,
        "sigue sobre ellas"
    );

    fixture.stage("lines.py", LINES_AGENT_2.as_bytes());
    send_turn(&parts, "seguí", cx);
    assert_eq!(
        fixture.last_feedback().as_deref(),
        Some(
            block(&format!(
                "{SECTION}\n\nComment 1 of 1\nFile: lines.py\nLines: 1-2\nReview state: your change, not reviewed yet\nCurrent code:\n```py\na = 10\nb = 20\n```\nComment:\na y b"
            ))
            .as_str()
        )
    );

    comment_line(&editor, 3, "c y d", cx);
    comment_line(&editor, 6, "e y f", cx);
    comment_line(&editor, 9, "g y h", cx);
    // c accepted, d rejected: mixed.
    decide_line(&parts, &editor, &path, 3, true, cx);
    decide_line(&parts, &editor, &path, 4, false, cx);
    // e and f accepted line by line.
    decide_line(&parts, &editor, &path, 6, true, cx);
    decide_line(&parts, &editor, &path, 7, true, cx);
    // g and h rejected line by line.
    decide_line(&parts, &editor, &path, 9, false, cx);
    decide_line(&parts, &editor, &path, 10, false, cx);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "a = 10\nb = 20\n\nc = 30\nd = 4\n\ne = 50\nf = 60\n\ng = 7\nh = 8\n"
    );

    send_turn(&parts, "y ahora", cx);
    let section = SECTION.replace("1 comment", "3 comments");
    assert_eq!(
        fixture.last_feedback().as_deref(),
        Some(
            block(&format!(
                "The user made the following updates to your changes:\n\ndiff --git a/lines.py b/lines.py\n--- a/lines.py\n+++ b/lines.py\n@@ -2,10 +2,10 @@\n b = 20\n \n c = 30\n-d = 40\n+d = 4\n \n e = 50\n f = 60\n \n-g = 70\n-h = 80\n+g = 7\n+h = 8\n\n{section}\n\nComment 1 of 3\nFile: lines.py\nLines: 4-5\nReview state: your change, partly accepted and partly rejected by the user, line by line\nCurrent code:\n```py\nc = 30\nd = 4\n```\nComment:\nc y d\n\nComment 2 of 3\nFile: lines.py\nLines: 7-8\nReview state: your change, accepted by the user\nCurrent code:\n```py\ne = 50\nf = 60\n```\nComment:\ne y f\n\nComment 3 of 3\nFile: lines.py\nLines: 10-11\nReview state: your change, rejected by the user\nCurrent code:\n```py\ng = 7\nh = 8\n```\nComment:\ng y h"
            ))
            .as_str()
        )
    );
    let states: Vec<String> = last_cards(&parts, cx)
        .into_iter()
        .map(|card| card.state_label)
        .collect();
    assert_eq!(states, vec!["mixto", "aceptado", "rechazado"]);
}

// ===================================================================== j

/// j) Once sent, the comments leave the margin, the chat's box, the
/// counters and `state.json`; the next message does not bring them; the
/// card stays in the sent message.
#[gpui::test]
fn sent_comments_disappear_and_never_repeat(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("util.py", UTIL.as_bytes())]);
    let util = fixture.path("util.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["util.py"]), cx);
    let editor = open(&parts, &util, cx);
    comment_line(&editor, 6, "Sacalo de la configuración.", cx);
    let_persist(cx);
    let state_file = parts
        .review
        .read_with(cx, |review, cx| review.persist_dir(cx))
        .expect("hay carpeta de la revisión")
        .join("state.json");
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert_eq!(saved["version"], 2);
    assert_eq!(saved["comments"].as_array().unwrap().len(), 1);

    send_turn(&parts, "primero", cx);
    assert!(fixture.last_feedback().is_some());
    assert!(editor_comments(&editor, cx).is_empty(), "margen");
    assert!(
        frame(&editor, cx).review.comment_marks.is_empty(),
        "sin marca"
    );
    assert!(tags(&parts, cx).is_empty(), "caja del chat");
    let summary = parts.review.read_with(cx, |review, _| review.summary());
    assert_eq!(summary.borrow().comments, 0);
    assert!(summary.borrow().comments_per_file.is_empty());
    let tree = parts.workspace.read_with(cx, |workspace, cx| {
        workspace
            .files()
            .read(cx)
            .comment_label(Path::new("util.py"), cx)
    });
    assert_eq!(tree, None);
    let_persist(cx);
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert!(saved["comments"].as_array().unwrap().is_empty());

    let cards = last_cards(&parts, cx);
    assert_eq!(
        cards,
        vec![SentCommentCard {
            display_path: "util.py".to_string(),
            path: util.clone(),
            first_line: 7,
            last_line: 7,
            kind: SentCommentKind::Lines,
            state_label: "sin cambios del agente".to_string(),
            code: Some("TIMEOUT = 3".to_string()),
            removed: None,
            truncated_lines: 0,
            lang: Some("py".to_string()),
            text: "Sacalo de la configuración.".to_string(),
            not_sent: false,
        }]
    );

    send_turn(&parts, "segundo", cx);
    assert_eq!(fixture.last_feedback(), None, "no se repiten");
    assert!(last_cards(&parts, cx).is_empty());
    // The card of the first message is still there.
    let first_cards = parts.chat.read_with(cx, |chat, _| {
        chat.entries()
            .iter()
            .filter_map(|entry| match entry {
                Entry::UserMessage(message) => Some(
                    message
                        .blocks
                        .iter()
                        .filter(|block| matches!(block, MessageBlock::Comment(_)))
                        .count(),
                ),
                _ => None,
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(first_cards, vec![1, 0]);
}

// ===================================================================== k

/// k) Switching connection with unsent comments: they stay and leave with
/// the next message, to the new connection.
#[gpui::test]
fn comments_follow_the_next_message_after_switching_connection(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("util.py", UTIL.as_bytes())]);
    let util = fixture.path("util.py");
    let (parts, cx) = window(&fixture.root, cx);
    let env = connect(&parts, &fixture, &Fixture::edits(&["util.py"]), cx);
    let other = env.add_connection("claude-acp", "Otra");
    parts.agents.update(cx, |agents, cx| {
        agents.set_connections(env.connections.clone(), cx)
    });
    settle(cx);

    let editor = open(&parts, &util, cx);
    comment_line(&editor, 0, "Ordená los imports.", cx);
    select(&parts, &other.to_string(), cx);
    assert_eq!(comment_count(&parts, cx), 1, "siguen pendientes");
    assert_eq!(tags(&parts, cx), vec!["util.py:1".to_string()]);
    assert_eq!(
        parts.chat.read_with(cx, |chat, _| chat
            .active_connection()
            .map(|c| c.label.clone())),
        Some("Otra".to_string())
    );

    send_turn(&parts, "a la otra", cx);
    assert_eq!(
        fixture.last_feedback().as_deref(),
        Some(
            block(&format!(
                "{SECTION}\n\nComment 1 of 1\nFile: util.py\nLine: 1\nReview state: no change of yours (the user selected these lines)\nCurrent code:\n```py\nimport os\n```\nComment:\nOrdená los imports."
            ))
            .as_str()
        )
    );
    assert_eq!(comment_count(&parts, cx), 0);
}

// ===================================================================== l

/// l) Closing with unsent comments: they come back on the same rows when the
/// project reopens; changed outside Cincel, they move with their text; a
/// file that is gone drops its comment with a notice; a `state.json` of
/// the 0.1.0 (version 1) opens without comments and is saved as version 2.
#[gpui::test]
fn comments_survive_closing_and_reopening(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("util.py", UTIL.as_bytes()), ("gone.py", FIVE.as_bytes())]);
    let util = fixture.path("util.py");
    let gone = fixture.path("gone.py");
    let persist_dir;
    {
        let (parts, cx) = window(&fixture.root, cx);
        let editor = open(&parts, &util, cx);
        comment_line(&editor, 0, "imports", cx);
        set_cursor(&editor, 2, 0, cx);
        cx.simulate_keystrokes("shift-down shift-down");
        cx.simulate_keystrokes("ctrl-shift-m");
        write_and_save("host y puerto", cx);
        let gone_editor = open(&parts, &gone, cx);
        comment_line(&gone_editor, 2, "se va a ir", cx);
        persist_dir = parts
            .review
            .read_with(cx, |review, cx| review.persist_dir(cx))
            .unwrap();
        // Closing the folder saves the review (the same `save_now` as quit).
        parts
            .review
            .update(cx, |review, cx| review.set_project(None, cx));
        settle(cx);
    }
    let rows = |parts: &Parts,
                cx: &mut VisualTestContext|
     -> Vec<(String, std::ops::Range<u32>, String)> {
        parts.review.read_with(cx, |review, _| {
            review
                .comment_views()
                .into_iter()
                .map(|view| (view.display_path, view.rows, view.text))
                .collect()
        })
    };
    {
        let (parts, cx) = window(&fixture.root, cx);
        assert_eq!(
            rows(&parts, cx),
            vec![
                ("gone.py".to_string(), 2..3, "se va a ir".to_string()),
                ("util.py".to_string(), 0..1, "imports".to_string()),
                ("util.py".to_string(), 2..4, "host y puerto".to_string()),
            ]
        );
        let editor = open(&parts, &util, cx);
        assert_eq!(editor_comments(&editor, cx).len(), 2, "en el margen");
        assert_eq!(tags(&parts, cx).len(), 3, "en la caja del chat");
        parts
            .review
            .update(cx, |review, cx| review.set_project(None, cx));
        settle(cx);
    }
    // Two lines added on top by another program, and a file deleted.
    std::fs::write(&util, format!("# cabecera\n# otra\n{UTIL}")).unwrap();
    std::fs::remove_file(&gone).unwrap();
    {
        let (parts, cx) = window(&fixture.root, cx);
        assert_eq!(
            rows(&parts, cx),
            vec![
                ("util.py".to_string(), 2..3, "imports".to_string()),
                ("util.py".to_string(), 4..6, "host y puerto".to_string()),
            ]
        );
        assert!(
            toasts(&parts, cx).iter().any(|toast| toast
                == "El comentario sobre «gone.py» se descartó: el archivo ya no existe"),
            "{:?}",
            toasts(&parts, cx)
        );
        parts
            .review
            .update(cx, |review, cx| review.set_project(None, cx));
        settle(cx);
    }

    // A version 1 file (the 0.1.0) has no comments: it opens, and the next
    // save writes version 2.
    let state_file = persist_dir.join("state.json");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    saved["version"] = 1.into();
    saved.as_object_mut().unwrap().remove("comments");
    std::fs::write(&state_file, serde_json::to_vec(&saved).unwrap()).unwrap();
    {
        let (parts, cx) = window(&fixture.root, cx);
        assert_eq!(comment_count(&parts, cx), 0);
        let editor = open(&parts, &util, cx);
        comment_line(&editor, 0, "de nuevo", cx);
        parts.review.update(cx, |review, cx| review.save_now(cx));
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
        assert_eq!(saved["version"], 2);
        assert_eq!(saved["comments"].as_array().unwrap().len(), 1);
    }
}

/// l) The close dialog (with pending changes) adds the comment line and its
/// buttons do what they always did; with only comments it does not open
/// (they are saved and come back).
#[gpui::test]
fn close_dialog_counts_comments_without_changing_behavior(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("calc.py", CALC.as_bytes()), ("util.py", UTIL.as_bytes())]);
    let calc = fixture.path("calc.py");
    let util = fixture.path("util.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["calc.py"]), cx);
    let should_close = |parts: &Parts, cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            parts
                .workspace
                .update(cx, |workspace, cx| workspace.should_close(window, cx))
        })
    };
    let dialog_open = |parts: &Parts, cx: &mut VisualTestContext| {
        parts
            .workspace
            .read_with(cx, |workspace, _| workspace.is_review_close_dialog_open())
    };
    let line = |parts: &Parts, cx: &mut VisualTestContext| {
        parts
            .workspace
            .read_with(cx, |workspace, cx| workspace.review_close_comments_line(cx))
    };
    let pending = |parts: &Parts, cx: &mut VisualTestContext| {
        parts
            .review
            .read_with(cx, |review, _| review.pending_counts())
    };

    // Only comments: no dialog (they are saved and come back).
    let editor = open(&parts, &util, cx);
    comment_line(&editor, 0, "uno", cx);
    assert!(should_close(&parts, cx), "nada que decidir");
    assert!(!dialog_open(&parts, cx));
    assert_eq!(comment_count(&parts, cx), 1);

    // With a pending change, the dialog says how many comments there are.
    fixture.stage("calc.py", CALC_AGENT.as_bytes());
    send_turn(&parts, "cambiá calc", cx);
    assert_eq!(
        comment_count(&parts, cx),
        0,
        "el primero salió con el mensaje"
    );
    comment_line(&editor, 0, "uno otra vez", cx);
    assert!(!should_close(&parts, cx));
    assert!(dialog_open(&parts, cx));
    assert_eq!(
        line(&parts, cx).as_deref(),
        Some("También hay 1 comentario sin enviar: se guarda y vuelve al abrir la carpeta.")
    );
    assert!(cx.debug_bounds("review-close-comments").is_some(), "se ve");
    cx.update(|window, cx| {
        parts.workspace.update(cx, |workspace, cx| {
            workspace.cancel_review_close_for_test(window, cx)
        })
    });
    settle(cx);
    assert!(!dialog_open(&parts, cx));
    assert_eq!(pending(&parts, cx), (1, 1), "Cancelar no decide nada");
    assert_eq!(comment_count(&parts, cx), 1, "ni borra comentarios");

    comment_line(&editor, 2, "dos", cx);
    assert!(!should_close(&parts, cx));
    assert_eq!(
        line(&parts, cx).as_deref(),
        Some("También hay 2 comentarios sin enviar: se guardan y vuelven al abrir la carpeta.")
    );
    assert_eq!(close_comments_line(0), None);
    let state_file = parts
        .review
        .read_with(cx, |review, cx| review.persist_dir(cx))
        .unwrap()
        .join("state.json");
    // "Aceptar todo" accepts and goes on with the quit (the window closes
    // and the review is saved, comments included).
    cx.update(|window, cx| {
        parts.workspace.update(cx, |workspace, cx| {
            workspace.accept_pending_and_close_for_test(window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&calc).unwrap(), CALC_AGENT);
    // The window is gone; the app still holds the review, and quitting
    // saves it (`on_app_quit` runs `save_now`).
    let app: &mut TestAppContext = cx;
    let (pending_after, comments_after) = app.update(|cx| {
        let review = parts.review.read(cx);
        (review.pending_counts(), review.comment_count())
    });
    assert_eq!(pending_after, (0, 0), "Aceptar todo acepta");
    assert_eq!(comments_after, 2, "los comentarios quedan");
    app.update(|cx| parts.review.update(cx, |review, cx| review.save_now(cx)));
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert!(
        saved["files"].as_array().unwrap().is_empty(),
        "nada pendiente"
    );
    assert_eq!(saved["comments"].as_array().unwrap().len(), 2, "se guardan");
}

// ===================================================================== m

/// m) A binary's tab offers no "Comentar", no menu, and the shortcut does
/// nothing; a commented text file the agent turns into a binary loses its
/// comment with a notice.
#[gpui::test]
fn binaries_take_no_comments(cx: &mut TestAppContext) {
    init(cx);
    let binary: &[u8] = &[0xff, 0xfe, 0x00, 0x01, 0x02, 0x80];
    let fixture = Fixture::new(&[("bin.dat", binary), ("texto.txt", FIVE.as_bytes())]);
    let (parts, cx) = window(&fixture.root, cx);

    let bin = open(&parts, &fixture.path("bin.dat"), cx);
    assert!(bin.read_with(cx, |editor, _| editor.is_read_only()));
    set_cursor(&bin, 0, 0, cx);
    cx.simulate_keystrokes("ctrl-shift-m");
    cx.run_until_parked();
    assert!(
        bin.read_with(cx, |editor, _| editor.comment_draft().is_none()),
        "el atajo no hace nada"
    );
    let bounds = frame(&bin, cx).bounds;
    let at = point(bounds.origin.x + px(80.), bounds.origin.y + px(10.));
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    assert!(
        !bin.read_with(cx, |editor, _| editor.is_context_menu_open()),
        "sin menú"
    );
    assert!(frame(&bin, cx).review.pills.is_empty(), "sin «Comentar»");
    assert_eq!(comment_count(&parts, cx), 0);

    // A commented text file the agent turns into a binary during a turn.
    let texto = fixture.path("texto.txt");
    let text = open(&parts, &texto, cx);
    parts
        .agents
        .update(cx, |agents, cx| agents.prepare_prompt(None, cx));
    settle(cx);
    comment_line(&text, 1, "dos", cx);
    assert_eq!(comment_count(&parts, cx), 1, "se comenta mientras escribe");
    std::fs::write(&texto, binary).unwrap();
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
    assert_eq!(comment_count(&parts, cx), 0);
    assert!(
        toasts(&parts, cx).iter().any(|toast| toast
            == "El comentario sobre «texto.txt» se descartó: el archivo ya no es de texto"),
        "{:?}",
        toasts(&parts, cx)
    );
    assert!(tags(&parts, cx).is_empty());
}

// ================================================================ D16

/// D16: a message cancelled while it waits for the photo never left: its
/// comments come back to the margin and the chat, the card says so, and the
/// next message takes them.
#[gpui::test]
fn cancelled_prompt_gives_comments_back(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("util.py", UTIL.as_bytes())]);
    let util = fixture.path("util.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["util.py"]), cx);
    let editor = open(&parts, &util, cx);
    comment_line(&editor, 6, "volvé", cx);
    parts
        .review
        .update(cx, |review, _| review.set_photo_in_background(true));

    // Sent, and cancelled before the photo is ready (nothing runs the
    // background executor in between).
    cx.update(|window, cx| {
        parts.chat.update(cx, |chat, cx| {
            chat.set_input_text("no va a salir", window, cx);
            chat.send(window, cx);
        })
    });
    // Taken (the message holds it) while the prompt waits for the photo.
    assert_eq!(comment_count(&parts, cx), 0);
    assert_eq!(last_cards(&parts, cx).len(), 1);
    assert!(!last_cards(&parts, cx)[0].not_sent);
    parts.chat.update(cx, |chat, cx| chat.cancel_turn(cx));
    settle(cx);

    assert!(fixture.prompts().is_empty(), "nunca salió");
    assert_eq!(comment_count(&parts, cx), 1, "volvió al store");
    assert_eq!(editor_comments(&editor, cx).len(), 1, "y al margen");
    assert_eq!(tags(&parts, cx), vec!["util.py:7".to_string()]);
    let cards = last_cards(&parts, cx);
    assert!(cards[0].not_sent, "«No se envió…»");

    parts
        .review
        .update(cx, |review, _| review.set_photo_in_background(false));
    send_turn(&parts, "ahora sí", cx);
    assert!(
        fixture
            .last_feedback()
            .is_some_and(|feedback| feedback.contains("Comment:\nvolvé")),
        "salió con el mensaje siguiente"
    );
    assert_eq!(comment_count(&parts, cx), 0);
}

/// "Comentar" works while the agent writes (it is not a decision).
#[gpui::test]
fn comment_while_agent_writes(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("util.py", UTIL.as_bytes())]);
    let util = fixture.path("util.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["util.py"]), cx);
    let editor = open(&parts, &util, cx);
    send_message(&parts, "never-end", cx);
    drain_until(&parts, cx, |event| {
        matches!(event, AgentEvent::Update { .. })
    });
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.is_turn_active())
    );

    comment_line(&editor, 0, "mientras escribe", cx);
    assert_eq!(comment_count(&parts, cx), 1);
    assert_eq!(tags(&parts, cx), vec!["util.py:1".to_string()]);

    parts.chat.update(cx, |chat, cx| chat.cancel_turn(cx));
    drain_until(&parts, cx, |event| {
        matches!(event, AgentEvent::TurnEnded { .. })
    });
    assert_eq!(comment_count(&parts, cx), 1, "nadie lo tomó");
}

// =========================================================== counters

/// The status bar and the tree count the comments (§6.2.9): "N cambios
/// pendientes · M comentarios", the comments alone, the singular; the
/// icon and count on each file's row; " · N comentarios" on the root.
#[gpui::test]
fn counters_show_comments_in_tree_and_status_bar(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("calc.py", CALC.as_bytes()), ("util.py", UTIL.as_bytes())]);
    let calc = fixture.path("calc.py");
    let util = fixture.path("util.py");
    let (parts, cx) = window(&fixture.root, cx);
    let _env = connect(&parts, &fixture, &Fixture::edits(&["calc.py"]), cx);
    fixture.stage("calc.py", CALC_AGENT.as_bytes());
    send_turn(&parts, "cambiá calc", cx);

    let counters = |parts: &Parts, cx: &mut VisualTestContext| {
        let summary = parts.review.read_with(cx, |review, _| review.summary());
        let status = {
            let summary = summary.borrow();
            status_label(summary.pending, summary.comments)
        };
        let (root, calc_tag, util_tag) = parts.workspace.read_with(cx, |workspace, cx| {
            let files = workspace.files().read(cx);
            (
                files.root_review_label(),
                files.comment_label(Path::new("calc.py"), cx),
                files.comment_label(Path::new("util.py"), cx),
            )
        });
        (status, root, calc_tag, util_tag)
    };
    assert_eq!(
        counters(&parts, cx),
        (
            "1 cambio pendiente".to_string(),
            Some("1 archivo · 1 cambio · +2 −2".to_string()),
            None,
            None
        )
    );

    let calc_editor = open(&parts, &calc, cx);
    comment_line(&calc_editor, 0, "uno", cx);
    assert_eq!(counters(&parts, cx).0, "1 cambio pendiente · 1 comentario");
    let util_editor = open(&parts, &util, cx);
    comment_line(&util_editor, 0, "dos", cx);
    comment_line(&util_editor, 6, "tres", cx);
    assert_eq!(
        counters(&parts, cx),
        (
            "1 cambio pendiente · 3 comentarios".to_string(),
            Some("1 archivo · 1 cambio · +2 −2 · 3 comentarios".to_string()),
            Some("1".to_string()),
            Some("2".to_string())
        )
    );
    assert!(cx.debug_bounds("status-pending").is_some());

    parts.review.update(cx, |review, cx| review.accept_all(cx));
    settle(cx);
    assert_eq!(
        counters(&parts, cx),
        (
            "3 comentarios".to_string(),
            Some("3 comentarios".to_string()),
            Some("1".to_string()),
            Some("2".to_string())
        )
    );
    assert_eq!(status_label(0, 1), "1 comentario");
    assert_eq!(status_label(0, 0), "0 cambios pendientes");
    assert_eq!(status_label(2, 0), "2 cambios pendientes");
}

// ============================================================ anchors

/// The comment follows its lines while the user types above them (the
/// tags of the chat say the new lines) and keeps them when the user types
/// inside; the editor's text never moves sideways (D16 of spec 07).
#[gpui::test]
fn comments_follow_their_lines_and_never_move_the_text(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("util.py", UTIL.as_bytes())]);
    let util = fixture.path("util.py");
    let (parts, cx) = window(&fixture.root, cx);
    let editor = open(&parts, &util, cx);
    let before = frame(&editor, cx);

    set_cursor(&editor, 2, 0, cx);
    cx.simulate_keystrokes("shift-down shift-down");
    cx.simulate_keystrokes("ctrl-shift-m");
    let open_box = frame(&editor, cx);
    write_and_save("host y puerto", cx);
    let folded = frame(&editor, cx);
    for painted in [&open_box, &folded] {
        assert_eq!(
            painted.gutter_width, before.gutter_width,
            "el margen no cambia"
        );
        assert_eq!(
            painted.text_origin_x, before.text_origin_x,
            "el texto no se corre"
        );
    }
    assert_eq!(folded.review.comment_marks.len(), 1);
    assert_eq!(tags(&parts, cx), vec!["util.py:3-4".to_string()]);

    // Two lines typed above.
    set_cursor(&editor, 0, 0, cx);
    cx.simulate_input("# a\n# b\n");
    settle(cx);
    assert_eq!(tags(&parts, cx), vec!["util.py:5-6".to_string()]);
    assert_eq!(editor_comments(&editor, cx)[0].rows, 4..6);
    // Typing inside keeps the range.
    set_cursor(&editor, 4, 0, cx);
    cx.simulate_input("X");
    settle(cx);
    assert_eq!(editor_comments(&editor, cx)[0].rows, 4..6);

    // Removing it with "Borrar" leaves the text where it was.
    let id = CommentId(editor_comments(&editor, cx)[0].id);
    parts
        .review
        .update(cx, |review, cx| review.remove_comment(id, cx));
    settle(cx);
    let after = frame(&editor, cx);
    assert_eq!(after.gutter_width, before.gutter_width);
    assert_eq!(after.text_origin_x, before.text_origin_x);
    assert!(after.review.comment_marks.is_empty());
}

// ====================================================== deleted files

/// §6.2.12: a file the agent deletes keeps its comments on the read-only
/// tab of the deletion (they travel as "you deleted this file"); a commented
/// file another program deletes outside a turn drops them with a notice.
#[gpui::test]
fn deleted_files_keep_or_drop_their_comments(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("viejo.py", FIVE.as_bytes()), ("otro.py", FIVE.as_bytes())]);
    let viejo = fixture.path("viejo.py");
    let otro = fixture.path("otro.py");
    let (parts, cx) = window(&fixture.root, cx);

    // The agent deletes `viejo.py` during a turn the comment was made in.
    let editor = open(&parts, &viejo, cx);
    parts
        .agents
        .update(cx, |agents, cx| agents.prepare_prompt(None, cx));
    settle(cx);
    comment_line(&editor, 1, "no lo borres", cx);
    std::fs::remove_file(&viejo).unwrap();
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
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.is_pending_deletion(&viejo))
    );
    assert_eq!(comment_count(&parts, cx), 1, "sigue");
    assert_eq!(tags(&parts, cx), vec!["viejo.py (borrado)".to_string()]);
    let tab = parts
        .center
        .read_with(cx, |center, _| {
            center
                .tabs()
                .iter()
                .find(|tab| tab.path == viejo)
                .map(|tab| (tab.deleted_review, tab.editor().clone()))
        })
        .expect("la pestaña del borrado");
    assert!(tab.0, "pestaña de solo lectura");
    let shown = editor_comments(&tab.1, cx);
    assert_eq!(shown.len(), 1);
    assert_eq!(
        shown[0].from_hunk,
        Some(crate::review::DELETED_FILE_HUNK),
        "sobre su segmento"
    );
    let feedback = parts
        .agents
        .update(cx, |agents, cx| agents.prepare_prompt(None, cx))
        .expect("hay bloque");
    assert!(
        feedback.contains("File: viejo.py (you deleted this file)\n"),
        "{feedback}"
    );
    assert!(feedback.contains("Comment:\nno lo borres"), "{feedback}");
    settle(cx);
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

    // Another program deletes `otro.py` outside a turn.
    let editor = open(&parts, &otro, cx);
    comment_line(&editor, 0, "se pierde", cx);
    assert_eq!(comment_count(&parts, cx), 1);
    std::fs::remove_file(&otro).unwrap();
    let project = parts
        .workspace
        .read_with(cx, |workspace, _| workspace.project().cloned())
        .expect("hay proyecto");
    project.update(cx, |project, cx| {
        project.apply_fs_events(&[cincel_project::FsEvent::Removed(otro.clone())], cx)
    });
    settle(cx);
    assert_eq!(comment_count(&parts, cx), 0);
    assert!(
        toasts(&parts, cx)
            .iter()
            .any(|toast| toast
                == "El comentario sobre «otro.py» se descartó: el archivo ya no existe"),
        "{:?}",
        toasts(&parts, cx)
    );
}

/// A click on a tag of the chat's box opens the file at the comment's line
/// (`ChatEvent::OpenLocation`).
#[gpui::test]
fn a_tag_click_opens_the_file_at_its_line(cx: &mut TestAppContext) {
    init(cx);
    let fixture = Fixture::new(&[("util.py", UTIL.as_bytes()), ("a.py", FIVE.as_bytes())]);
    let util = fixture.path("util.py");
    let (parts, cx) = window(&fixture.root, cx);
    let editor = open(&parts, &util, cx);
    comment_line(&editor, 6, "acá", cx);
    open(&parts, &fixture.path("a.py"), cx);
    set_cursor(&editor, 0, 0, cx);
    open(&parts, &fixture.path("a.py"), cx);

    click_selector(cx, "comment-tag-0");
    settle(cx);
    let (active, cursor) = parts.center.read_with(cx, |center, _| {
        let tab = center.active_tab().expect("pestaña activa");
        (tab.path.clone(), tab.cursor)
    });
    assert_eq!(active, util);
    assert_eq!(cursor.row, 6);
}
