//! GPUI tests of the review integration (`crate::review`): the agent side
//! (real fake agent and hand-made ACP events), the editor side (keys and
//! actions), navigation, the panel, persistence and the dirty-buffer dialog.
//!
//! Behind `test-support`, like every GPUI test of the crate:
//!
//! ```text
//! cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins
//! cargo test -p cincel-workspace --features test-support
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use cincel_acp::acp::schema::v1::{
    Diff, SessionId, SessionUpdate, StopReason, ToolCall, ToolCallContent, ToolCallLocation,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use cincel_acp::{AgentCommand, AgentEvent, PromptBlock};
use cincel_chat::ChatPanel;
use cincel_editor::{EditorView, ReviewAction};
use cincel_review::REPORT_HEADER;
use cincel_settings::Config;
use cincel_text::Point;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::agents::Agents;
use crate::center::CenterPanel;
use crate::project::ProjectOptions;
use crate::review::{DirtyChoice, FileState, PanelState, Review, hunk_views};
use crate::test_support::{FakeEnv, isolate_state};
use crate::workspace::{Workspace, WorkspaceOptions};

const FIVE: &str = "a\nb\nc\nd\ne\n";
const FIVE_EDITED: &str = "a\nB\nc\nd\nE\n";

fn init(cx: &mut TestAppContext) {
    init_with(cx, |_| {});
}

fn init_with(cx: &mut TestAppContext, configure: impl FnOnce(&mut Config)) {
    isolate_state();
    let mut config = Config::default();
    configure(&mut config);
    cx.update(|cx| crate::init(config, cx));
}

/// A project with `a.txt`, `b.txt` (five lines each) and `demo.txt` (the file
/// the fake agent's default turn edits).
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), FIVE).unwrap();
    std::fs::write(dir.path().join("b.txt"), FIVE).unwrap();
    std::fs::write(dir.path().join("demo.txt"), "alfa\nbeta\n").unwrap();
    dir
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
    let parts = Parts {
        workspace,
        review,
        center,
        agents,
        chat,
    };
    (parts, cx)
}

/// Lets the debounced recompute (50 ms) and the background job land.
fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(120));
    cx.run_until_parked();
}

fn open(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> Entity<EditorView> {
    cx.update(|window, cx| {
        parts
            .center
            .update(cx, |center, cx| center.open_file(path, true, window, cx));
    });
    settle(cx);
    editor_of(parts, path, cx).expect("la pestaña está abierta")
}

fn editor_of(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> Option<Entity<EditorView>> {
    parts.center.read_with(cx, |center, _| {
        center
            .tabs()
            .iter()
            .find(|tab| tab.path == path)
            .map(|tab| tab.editor().clone())
    })
}

fn sid() -> SessionId {
    SessionId::new("prueba")
}

fn agent(parts: &Parts, event: AgentEvent, cx: &mut VisualTestContext) {
    parts
        .agents
        .update(cx, |agents, cx| agents.handle_agent_event(event, cx));
    settle(cx);
}

/// What sending a prompt does to the review; returns the feedback.
fn start_turn(parts: &Parts, cx: &mut VisualTestContext) -> Option<String> {
    let feedback = parts
        .agents
        .update(cx, |agents, cx| agents.prepare_prompt(None, cx));
    settle(cx);
    feedback
}

fn end_turn(parts: &Parts, cx: &mut VisualTestContext) {
    agent(
        parts,
        AgentEvent::TurnEnded {
            session_id: sid(),
            stop_reason: StopReason::EndTurn,
        },
        cx,
    );
}

fn edit_call(id: &str, path: &Path) -> AgentEvent {
    AgentEvent::Update {
        session_id: sid(),
        update: Box::new(SessionUpdate::ToolCall(
            ToolCall::new(id.to_string(), "Editar")
                .kind(ToolKind::Edit)
                .status(ToolCallStatus::InProgress)
                .locations(vec![ToolCallLocation::new(path.to_path_buf())]),
        )),
    }
}

fn call_done(id: &str) -> AgentEvent {
    AgentEvent::Update {
        session_id: sid(),
        update: Box::new(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            id.to_string(),
            ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
        ))),
    }
}

/// Sends `fs/write_text_file` and returns the receiving end of its answer.
fn fs_write(
    parts: &Parts,
    path: &Path,
    content: &str,
    cx: &mut VisualTestContext,
) -> tokio::sync::oneshot::Receiver<Result<(), cincel_acp::FsError>> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    agent(
        parts,
        AgentEvent::FsWrite {
            session_id: sid(),
            path: path.to_path_buf(),
            content: content.to_string(),
            reply,
        },
        cx,
    );
    answer
}

/// `fs/write_text_file` that must be answered right away.
fn write_now(parts: &Parts, path: &Path, content: &str, cx: &mut VisualTestContext) {
    let mut answer = fs_write(parts, path, content, cx);
    assert!(
        matches!(answer.try_recv(), Ok(Ok(()))),
        "la escritura se contestó"
    );
}

/// A whole turn where the agent writes `content` into `path`.
fn agent_writes(parts: &Parts, path: &Path, content: &str, cx: &mut VisualTestContext) {
    start_turn(parts, cx);
    let mut answer = fs_write(parts, path, content, cx);
    assert!(matches!(answer.try_recv(), Ok(Ok(()))));
    end_turn(parts, cx);
}

fn hunk_count(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> usize {
    parts
        .review
        .read_with(cx, |review, _| review.store().hunks(path).len())
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

fn act(parts: &Parts, path: &Path, action: ReviewAction, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        parts.review.update(cx, |review, cx| {
            review.handle_action(path, action, None, window, cx)
        });
    });
    settle(cx);
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

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

// ------------------------------------------------------------- fake agent

/// A fake connection engine with one "Fake" connection, activated from the
/// chat the way the user does it (the popover's row). Keep the returned
/// [`FakeEnv`] alive for the whole test: it owns the profile directory.
fn activate_fake(parts: &Parts, cx: &mut VisualTestContext) -> FakeEnv {
    let env = FakeEnv::new();
    let id = env.add_connection("claude-acp", "Fake");
    parts.agents.update(cx, |agents, cx| {
        agents.set_connections(env.connections.clone(), cx)
    });
    parts
        .chat
        .update(cx, |chat, cx| chat.select_connection(&id.to_string(), cx));
    env
}

fn drain_until(parts: &Parts, cx: &mut VisualTestContext, stop: impl Fn(&AgentEvent) -> bool) {
    let events = parts
        .agents
        .read_with(cx, |agents, _| agents.connection_events())
        .expect("hay conexión");
    loop {
        let event = events.recv_blocking().expect("evento del agente falso");
        let done = stop(&event);
        agent(parts, event, cx);
        if done {
            break;
        }
    }
}

// ------------------------------------------------------------------ tests

/// (a) A real turn of the fake agent writes `demo.txt` through
/// `fs/write_text_file`: the file is in review and its editor paints the
/// hunk.
#[gpui::test]
fn a_fake_agent_write_is_reviewed_in_the_editor(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let demo = dir.path().join("demo.txt");
    let editor = open(&parts, &demo, cx);

    let _env = activate_fake(&parts, cx);
    cx.run_until_parked();
    drain_until(&parts, cx, |event| {
        matches!(event, AgentEvent::SessionCreated { .. })
    });
    let session_id = parts
        .chat
        .read_with(cx, |chat, _| chat.session_id().cloned().expect("sesión"));
    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(
            &AgentCommand::Prompt {
                session_id,
                blocks: vec![PromptBlock::Text("hola".to_string())],
                feedback: None,
            },
            cx,
        )
    });
    drain_until(&parts, cx, |event| {
        matches!(event, AgentEvent::TurnEnded { .. })
    });
    settle(cx);

    assert_eq!(read(&demo), "alfa\nbeta\nescrito por el agente falso\n");
    assert_eq!(hunk_count(&parts, &demo, cx), 1);
    let view = editor.read_with(cx, |editor, _| editor.review().clone());
    assert_eq!(view.hunks.len(), 1, "{view:?}");
    assert_eq!(view.hunks[0].buffer_rows, 2..3);
    assert!(view.hunks[0].deleted_lines.is_empty());
    assert!(!view.turn_active, "el turno terminó");
    assert_eq!(view.pending_in_file, 1);
    let pending = parts
        .review
        .read_with(cx, |review, _| review.summary().borrow().pending);
    assert_eq!(pending, 1);
}

/// (b) The agent edits the file on disk by itself while an edit tool call is
/// in progress: the base comes from the first contact, the truth from the
/// re-read when the tool call completes.
#[gpui::test]
fn an_edit_straight_to_disk_is_reviewed_after_the_tool_call(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");

    start_turn(&parts, cx);
    agent(&parts, edit_call("c1", &path), cx);
    // Written behind our back, as a shell command or the agent's own tool
    // would.
    std::fs::write(&path, FIVE_EDITED).unwrap();
    assert_eq!(hunk_count(&parts, &path, cx), 0, "nada hasta que termina");
    agent(&parts, call_done("c1"), cx);
    end_turn(&parts, cx);

    assert_eq!(hunk_count(&parts, &path, cx), 2);
    let base = parts.review.read_with(cx, |review, _| {
        review.store().file(&path).map(|f| f.base.to_string())
    });
    assert_eq!(
        base.as_deref(),
        Some(FIVE),
        "la base es el archivo de antes"
    );
    // A tab opened afterwards gets the hunks too.
    let editor = open(&parts, &path, cx);
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.review().hunks.len()),
        2
    );
}

/// (c) Accepting a hunk from the keyboard shrinks the review and leaves the
/// disk alone.
#[gpui::test]
fn accepting_a_hunk_keeps_the_disk(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    agent_writes(&parts, &path, FIVE_EDITED, cx);
    let editor = open(&parts, &path, cx);

    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(1, 0), cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-enter");
    settle(cx);

    assert_eq!(hunk_count(&parts, &path, cx), 1);
    assert_eq!(read(&path), FIVE_EDITED, "aceptar no toca el disco");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.review().hunks.len()),
        1
    );
    let base = parts.review.read_with(cx, |review, _| {
        review.store().file(&path).map(|f| f.base.to_string())
    });
    assert_eq!(base.as_deref(), Some("a\nB\nc\nd\ne\n"));
}

/// (d) Rejecting a hunk puts the original text back on disk and offers the
/// undo toast.
#[gpui::test]
fn rejecting_a_hunk_reverts_the_disk_and_offers_undo(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    agent_writes(&parts, &path, FIVE_EDITED, cx);
    let editor = open(&parts, &path, cx);

    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(1, 0), cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-backspace");
    settle(cx);

    assert_eq!(read(&path), "a\nb\nc\nd\nE\n");
    assert_eq!(hunk_count(&parts, &path, cx), 1);
    assert!(
        !editor.read_with(cx, |editor, _| editor.is_dirty()),
        "se guardó"
    );
    let messages = toasts(&parts, cx);
    assert!(
        messages
            .iter()
            .any(|message| message == "Segmento rechazado · Deshacer (Alt+Shift+U)"),
        "{messages:?}"
    );
}

/// Undoing the reject (the toast's button) brings the agent's text and the
/// hunk back.
#[gpui::test]
fn the_undo_toast_brings_the_rejected_hunk_back(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    agent_writes(&parts, &path, FIVE_EDITED, cx);
    let first = hunk_ids(&parts, &path, cx)[0];
    act(&parts, &path, ReviewAction::RejectHunk(first), cx);
    assert_eq!(read(&path), "a\nb\nc\nd\nE\n");

    let toasts_entity = parts
        .workspace
        .read_with(cx, |workspace, _| workspace.toasts().clone());
    let id = toasts_entity.read_with(cx, |toasts, _| {
        toasts
            .items()
            .iter()
            .find(|toast| toast.message.starts_with("Segmento rechazado"))
            .map(|toast| toast.id)
            .expect("hay aviso")
    });
    toasts_entity.update(cx, |toasts, cx| toasts.run_action(id, 0, cx));
    settle(cx);

    assert_eq!(read(&path), FIVE_EDITED);
    assert_eq!(hunk_count(&parts, &path, cx), 2);
}

/// `Alt+Shift+U` in the editor does the same.
#[gpui::test]
fn alt_shift_u_undoes_the_last_reject(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    agent_writes(&parts, &path, FIVE_EDITED, cx);
    let _editor = open(&parts, &path, cx);
    let last = *hunk_ids(&parts, &path, cx).last().unwrap();
    act(&parts, &path, ReviewAction::RejectHunk(last), cx);
    assert_eq!(read(&path), "a\nB\nc\nd\ne\n");

    cx.simulate_keystrokes("alt-shift-u");
    settle(cx);
    assert_eq!(read(&path), FIVE_EDITED);
    assert_eq!(hunk_count(&parts, &path, cx), 2);
}

/// (e) The user typing away from every hunk goes into the base: no new hunk.
#[gpui::test]
fn typing_outside_a_hunk_creates_no_hunk(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    agent_writes(&parts, &path, FIVE_EDITED, cx);
    let editor = open(&parts, &path, cx);

    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(2, 1), cx));
    cx.run_until_parked();
    cx.simulate_input("X");
    settle(cx);

    assert_eq!(hunk_count(&parts, &path, cx), 2);
    let base = parts.review.read_with(cx, |review, _| {
        review.store().file(&path).map(|f| f.base.to_string())
    });
    assert_eq!(base.as_deref(), Some("a\nb\ncX\nd\ne\n"), "va a la base");
    let rows: Vec<_> = editor.read_with(cx, |editor, _| {
        editor
            .review()
            .hunks
            .iter()
            .map(|hunk| hunk.buffer_rows.clone())
            .collect()
    });
    assert_eq!(rows, vec![1..2, 4..5]);
}

/// (f) Typing inside a hunk keeps it pending, with the user's text on its new
/// side.
#[gpui::test]
fn typing_inside_a_hunk_keeps_it_pending(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    agent_writes(&parts, &path, FIVE_EDITED, cx);
    let editor = open(&parts, &path, cx);

    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(1, 1), cx));
    cx.run_until_parked();
    cx.simulate_input("Z");
    settle(cx);

    assert_eq!(hunk_count(&parts, &path, cx), 2);
    let (base, first) = parts.review.read_with(cx, |review, _| {
        let file = review.store().file(&path).unwrap();
        let hunk = review.store().hunks(&path)[0].clone();
        (
            file.base.to_string(),
            file.current().text_in(hunk.buffer_byte_range()),
        )
    });
    assert_eq!(base, FIVE, "la base no cambia");
    assert_eq!(first, "BZ\n", "el texto del usuario queda del lado nuevo");
}

/// (g) `Alt+J` walks every hunk of every file, opening the second file's tab,
/// and wraps around to the first one.
#[gpui::test]
fn alt_j_wraps_across_files_opening_tabs(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    start_turn(&parts, cx);
    write_now(&parts, &a, "a\nB\nc\nd\ne\n", cx);
    write_now(&parts, &b, "a\nb\nc\nD\ne\n", cx);
    end_turn(&parts, cx);
    let editor_a = open(&parts, &a, cx);
    editor_a.update(cx, |editor, cx| editor.set_cursor(Point::new(0, 0), cx));
    cx.run_until_parked();

    let active = |parts: &Parts, cx: &mut VisualTestContext| {
        parts.center.read_with(cx, |center, _| {
            center.active_tab().map(|tab| tab.path.clone())
        })
    };

    cx.simulate_keystrokes("alt-j");
    settle(cx);
    assert_eq!(active(&parts, cx), Some(a.clone()));
    assert_eq!(
        editor_a.read_with(cx, |editor, _| editor.cursor_point().row),
        1
    );

    cx.simulate_keystrokes("alt-j");
    settle(cx);
    assert_eq!(
        active(&parts, cx),
        Some(b.clone()),
        "abre la segunda pestaña"
    );
    assert_eq!(
        parts.center.read_with(cx, |center, _| center.tabs().len()),
        2
    );
    let editor_b = editor_of(&parts, &b, cx).unwrap();
    assert_eq!(
        editor_b.read_with(cx, |editor, _| editor.cursor_point().row),
        3
    );

    cx.simulate_keystrokes("alt-j");
    settle(cx);
    assert_eq!(active(&parts, cx), Some(a.clone()), "y da la vuelta");
}

/// (h) The review panel lists the files with their stats and state.
#[gpui::test]
fn the_panel_lists_files_with_stats(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    start_turn(&parts, cx);
    write_now(&parts, &a, FIVE_EDITED, cx);
    write_now(&parts, &b, "a\nb\nc\nd\ne\nf\ng\n", cx);
    end_turn(&parts, cx);

    cx.simulate_keystrokes("ctrl-shift-r");
    cx.run_until_parked();
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.is_panel_open())
    );

    let rows = parts.review.read_with(cx, |review, _| review.panel_rows());
    let summary: Vec<_> = rows
        .iter()
        .map(|row| (row.relative.as_str(), row.added, row.removed, row.state))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("a.txt", 2, 2, PanelState::Pending),
            ("b.txt", 2, 0, PanelState::Pending)
        ]
    );

    parts
        .review
        .update(cx, |review, cx| review.accept_file(&b, cx));
    settle(cx);
    let rows = parts.review.read_with(cx, |review, _| review.panel_rows());
    assert_eq!(rows[1].state, PanelState::Decided);
    assert_eq!(
        parts
            .review
            .read_with(cx, |review, _| review.summary().borrow().pending),
        2
    );

    // A click on a row opens the file on its first pending hunk and closes
    // the panel.
    cx.update(|window, cx| {
        parts.review.update(cx, |review, cx| {
            review.open_file_at_first_hunk(&a, window, cx)
        })
    });
    settle(cx);
    assert!(
        !parts
            .review
            .read_with(cx, |review, _| review.is_panel_open())
    );
    let editor = editor_of(&parts, &a, cx).expect("se abrió");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.cursor_point().row),
        1
    );
}

/// (i) Pending hunks survive closing and reopening the project; a file that
/// changed outside Cincel meanwhile is dropped with a notice.
#[gpui::test]
fn the_review_survives_a_reopen(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    start_turn(&parts, cx);
    write_now(&parts, &a, FIVE_EDITED, cx);
    write_now(&parts, &b, "a\nb\nX\nd\ne\n", cx);
    end_turn(&parts, cx);
    parts.review.update(cx, |review, cx| review.save_now(cx));
    let dir_on_disk = parts
        .review
        .read_with(cx, |review, cx| review.persist_dir(cx))
        .expect("hay directorio");
    assert!(dir_on_disk.join("state.json").is_file());
    assert!(dir_on_disk.to_string_lossy().contains("cincel/review/"));

    // Someone else edits b.txt while Cincel is "closed".
    std::fs::write(&b, "otra cosa\n").unwrap();
    let root = dir.path().to_path_buf();
    cx.update(|window, cx| {
        parts.workspace.update(cx, |workspace, cx| {
            workspace.open_project(&root, window, cx)
        })
    });
    settle(cx);

    assert_eq!(
        hunk_count(&parts, &a, cx),
        2,
        "a.txt vuelve con sus segmentos"
    );
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.store().file(&b).is_none()),
        "b.txt cambió por fuera"
    );
    let messages = toasts(&parts, cx);
    assert!(
        messages.iter().any(|message| message
            == "La revisión de «b.txt» se descartó: el archivo cambió fuera de Cincel"),
        "{messages:?}"
    );
    // The restored file is ready in the background, and a tab shows it.
    let editor = open(&parts, &a, cx);
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.review().hunks.len()),
        2
    );
    // Restoring never writes a file.
    assert_eq!(read(&a), FIVE_EDITED);
}

/// (j) What the user rejected goes to the agent with the next prompt, once.
#[gpui::test]
fn the_next_prompt_carries_the_review_feedback(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    agent_writes(&parts, &path, FIVE_EDITED, cx);
    let last = *hunk_ids(&parts, &path, cx).last().unwrap();
    act(&parts, &path, ReviewAction::RejectHunk(last), cx);

    let feedback = start_turn(&parts, cx).expect("hay devolución");
    assert!(feedback.starts_with(REPORT_HEADER), "{feedback}");
    assert!(feedback.contains("a.txt"), "{feedback}");
    assert!(
        feedback.contains("-E") && feedback.contains("+e"),
        "{feedback}"
    );
    end_turn(&parts, cx);

    assert_eq!(start_turn(&parts, cx), None, "se manda una sola vez");
}

/// (k) Nothing is accepted automatically when a turn ends: the fixed
/// permission policy has no "Aplicar sin revisar" any more.
#[gpui::test]
fn a_turn_that_ends_leaves_its_changes_pending(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    start_turn(&parts, cx);
    write_now(&parts, &path, FIVE_EDITED, cx);
    end_turn(&parts, cx);

    assert_eq!(hunk_count(&parts, &path, cx), 2, "sigue pendiente");
    assert_eq!(read(&path), FIVE_EDITED);
}

/// While the agent writes a file its view is `turn_active` (every decision
/// disabled), and the decisions sent anyway are ignored.
#[gpui::test]
fn the_editor_knows_while_the_agent_is_writing(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    let editor = open(&parts, &path, cx);

    start_turn(&parts, cx);
    write_now(&parts, &path, FIVE_EDITED, cx);
    assert!(editor.read_with(cx, |editor, _| editor.review().turn_active));
    let first = hunk_ids(&parts, &path, cx)[0];
    act(&parts, &path, ReviewAction::AcceptHunk(first), cx);
    assert_eq!(
        hunk_count(&parts, &path, cx),
        2,
        "sin decisiones durante el turno"
    );

    end_turn(&parts, cx);
    assert!(!editor.read_with(cx, |editor, _| editor.review().turn_active));
}

/// Rejecting one line of a two-line hunk reverts only that line.
#[gpui::test]
fn rejecting_one_line_reverts_only_that_line(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    agent_writes(&parts, &path, "a\nB\nC\nd\ne\n", cx);
    let editor = open(&parts, &path, cx);
    let hunk = editor.read_with(cx, |editor, _| editor.review().hunks[0].clone());
    assert_eq!(hunk.deleted_lines, vec!["b", "c"]);
    // The pair of the second line: "c" replaced by "C".
    let line = hunk
        .lines
        .iter()
        .position(|line| line.base_line == Some(1) && line.buffer_row == Some(2))
        .expect("el par de la segunda línea");
    act(
        &parts,
        &path,
        ReviewAction::RejectLine {
            hunk: hunk.id,
            line,
        },
        cx,
    );

    assert_eq!(read(&path), "a\nB\nc\nd\ne\n");
    assert_eq!(hunk_count(&parts, &path, cx), 1);
    let messages = toasts(&parts, cx);
    assert!(
        messages
            .iter()
            .any(|message| message.starts_with("Línea rechazada"))
    );
}

/// A file the agent created is green in the tree, and rejecting it deletes it
/// and closes its tab.
#[gpui::test]
fn rejecting_a_created_file_deletes_it_and_closes_its_tab(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("nuevo.txt");
    agent_writes(&parts, &path, "hola\nmundo\n", cx);
    let state = parts.review.read_with(cx, |review, _| {
        review.summary().borrow().file(&path).map(|file| file.state)
    });
    assert_eq!(state, Some(FileState::Created));
    let rows = parts.review.read_with(cx, |review, _| review.panel_rows());
    assert!(
        rows.iter()
            .any(|row| row.created && row.relative == "nuevo.txt")
    );
    let _editor = open(&parts, &path, cx);

    act(&parts, &path, ReviewAction::RejectFile, cx);
    assert!(!path.exists(), "el archivo nuevo se borra");
    assert!(
        editor_of(&parts, &path, cx).is_none(),
        "y su pestaña se cierra"
    );
}

/// Tabs, tree and status bar show the counters; the old "◆" is gone.
#[gpui::test]
fn tabs_and_tree_show_the_counters(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    let _editor = open(&parts, &path, cx);
    agent_writes(&parts, &path, "a\nB\nc\nd\ne\nf\n", cx);

    let title = parts
        .center
        .read_with(cx, |center, _| center.tabs()[0].display_title().to_string());
    assert_eq!(title, "a.txt", "sin ◆");
    let label = parts.workspace.read_with(cx, |workspace, cx| {
        workspace
            .files()
            .read(cx)
            .review_label(Path::new("a.txt"), cx)
    });
    assert_eq!(label.as_deref(), Some("+2 −1"));
    let (pending, total) = parts.review.read_with(cx, |review, _| {
        let summary = review.summary();
        let summary = summary.borrow();
        (summary.pending, summary.total)
    });
    assert_eq!((pending, total), (2, (2, 1)));
    assert!(parts.review.read_with(cx, |review, _| {
        review.summary().borrow().has_pending_under(dir.path())
    }));
}

/// `Ctrl+Alt+Backspace` rejects the whole turn from anywhere, and
/// `Ctrl+Alt+Enter` accepts it.
#[gpui::test]
fn the_turn_commands_decide_every_file(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    start_turn(&parts, cx);
    write_now(&parts, &a, FIVE_EDITED, cx);
    write_now(&parts, &b, FIVE_EDITED, cx);
    end_turn(&parts, cx);

    cx.simulate_keystrokes("ctrl-alt-backspace");
    settle(cx);
    assert_eq!(read(&a), FIVE);
    assert_eq!(read(&b), FIVE);
    assert_eq!(
        parts
            .review
            .read_with(cx, |review, _| review.store().pending_count()),
        0
    );

    agent_writes(&parts, &a, FIVE_EDITED, cx);
    cx.simulate_keystrokes("ctrl-alt-enter");
    settle(cx);
    assert_eq!(read(&a), FIVE_EDITED);
    assert_eq!(
        parts
            .review
            .read_with(cx, |review, _| review.store().pending_count()),
        0
    );
}

/// Before the agent writes over unsaved changes the dialog asks; "Guardar"
/// saves them first, so the hunk is only the agent's change.
#[gpui::test]
fn the_dirty_dialog_saves_before_the_agent_writes(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    let editor = open(&parts, &path, cx);
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(0, 0), cx));
    cx.run_until_parked();
    cx.simulate_input("mío ");
    settle(cx);
    assert!(editor.read_with(cx, |editor, _| editor.is_dirty()));

    start_turn(&parts, cx);
    let mut answer = fs_write(&parts, &path, "mío a\nb\nc\nd\nE\n", cx);
    assert!(answer.try_recv().is_err(), "el agente espera la respuesta");
    let asked = parts
        .review
        .read_with(cx, |review, _| review.dirty_prompt().map(Path::to_path_buf));
    assert_eq!(asked, Some(path.clone()));
    let shown = parts.review.read_with(cx, |review, _| {
        review.summary().borrow().dirty_prompt.clone()
    });
    assert_eq!(shown.as_deref(), Some("a.txt"));

    parts.center.update(cx, |_, cx| {
        cx.emit(crate::center::CenterEvent::DirtyAnswer(DirtyChoice::Save))
    });
    settle(cx);
    assert!(matches!(answer.try_recv(), Ok(Ok(()))));
    end_turn(&parts, cx);

    assert_eq!(read(&path), "mío a\nb\nc\nd\nE\n");
    let base = parts.review.read_with(cx, |review, _| {
        review.store().file(&path).map(|f| f.base.to_string())
    });
    assert_eq!(base.as_deref(), Some("mío a\nb\nc\nd\ne\n"));
    assert_eq!(hunk_count(&parts, &path, cx), 1);
}

/// "Mantener" lets the agent write over the unsaved changes: the base is the
/// file as last saved, so the hunk includes the user's changes.
#[gpui::test]
fn keep_lets_the_agent_write_over_and_reviews_both(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    let editor = open(&parts, &path, cx);
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(0, 0), cx));
    cx.run_until_parked();
    cx.simulate_input("mío ");
    settle(cx);

    start_turn(&parts, cx);
    let mut answer = fs_write(&parts, &path, "mío a\nb\nc\nd\nE\n", cx);
    parts
        .review
        .update(cx, |review, cx| review.answer_dirty(DirtyChoice::Keep, cx));
    settle(cx);
    assert!(matches!(answer.try_recv(), Ok(Ok(()))));
    end_turn(&parts, cx);

    let base = parts.review.read_with(cx, |review, _| {
        review.store().file(&path).map(|f| f.base.to_string())
    });
    assert_eq!(base.as_deref(), Some(FIVE), "la base es lo guardado");
    assert_eq!(
        hunk_count(&parts, &path, cx),
        2,
        "tu cambio y el del agente"
    );
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty()));
}

/// With the permission already granted automatically nobody can wait for the
/// answer: an edit tool call on a dirty buffer saves it without asking.
#[gpui::test]
fn an_auto_granted_edit_saves_the_dirty_buffer_without_asking(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    let editor = open(&parts, &path, cx);
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(0, 0), cx));
    cx.run_until_parked();
    cx.simulate_input("mío ");
    settle(cx);

    start_turn(&parts, cx);
    agent(&parts, edit_call("c1", &path), cx);
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.dirty_prompt().is_none())
    );
    assert_eq!(read(&path), "mío a\nb\nc\nd\ne\n", "se guardó");
    assert!(!editor.read_with(cx, |editor, _| editor.is_dirty()));
    let base = parts.review.read_with(cx, |review, _| {
        review.store().file(&path).map(|f| f.base.to_string())
    });
    assert_eq!(base.as_deref(), Some("mío a\nb\nc\nd\ne\n"));
}

/// Autosave on focus change leaves files in review alone while a turn runs.
#[gpui::test]
fn autosave_waits_for_the_turn_to_end(cx: &mut TestAppContext) {
    init_with(cx, |config| {
        config.settings.files.autosave = cincel_settings::Autosave::OnFocusChange;
    });
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    let editor = open(&parts, &path, cx);

    start_turn(&parts, cx);
    agent(&parts, edit_call("c1", &path), cx);
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(0, 0), cx));
    cx.run_until_parked();
    cx.simulate_input("mío ");
    settle(cx);
    cx.update(|window, cx| {
        parts
            .center
            .update(cx, |center, cx| center.autosave(window, cx))
    });
    assert!(
        editor.read_with(cx, |editor, _| editor.is_dirty()),
        "suspendido"
    );

    agent(&parts, call_done("c1"), cx);
    end_turn(&parts, cx);
    cx.update(|window, cx| {
        parts
            .center
            .update(cx, |center, cx| center.autosave(window, cx))
    });
    assert!(
        !editor.read_with(cx, |editor, _| editor.is_dirty()),
        "ahora sí"
    );
}

/// `review.max_lines` below a file's size: reviewed as a whole file, no
/// inline hunks, and the panel says why.
#[gpui::test]
fn files_over_the_limits_are_reviewed_as_a_whole(cx: &mut TestAppContext) {
    init_with(cx, |config| config.settings.review.max_lines = 3);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    agent_writes(&parts, &path, FIVE_EDITED, cx);
    let editor = open(&parts, &path, cx);

    let view = editor.read_with(cx, |editor, _| editor.review().clone());
    assert!(view.hunks.is_empty(), "sin segmentos en línea");
    assert_eq!(view.pending_in_file, 1);
    let rows = parts.review.read_with(cx, |review, _| review.panel_rows());
    assert!(rows[0].too_large);
    assert!(parts.review.read_with(cx, |review, _| {
        review.summary().borrow().file(&path).unwrap().file_level
    }));
    parts
        .review
        .update(cx, |review, cx| review.reject_file(&path, cx));
    settle(cx);
    assert_eq!(read(&path), FIVE);
}

/// The views the editor gets: deleted lines from the base, word diffs, line
/// pairs in buffer rows.
#[gpui::test]
fn hunk_views_describe_the_store(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    agent_writes(&parts, &path, FIVE_EDITED, cx);
    let views = parts.review.read_with(cx, |review, _| {
        hunk_views(review.store().file(&path).unwrap(), false)
    });
    assert_eq!(views.len(), 2);
    let (first, pairs) = &views[0];
    assert_eq!(first.deleted_lines, vec!["b"]);
    assert_eq!(first.buffer_rows, 1..2);
    assert_eq!(pairs.len(), first.lines.len());
    assert_eq!(
        first.lines[0]
            .buffer_row
            .or(first.lines.last().unwrap().buffer_row),
        Some(1)
    );
    let words = first.word_diffs.as_ref().expect("segmento corto");
    assert_eq!(words.deleted, vec![0..1]);
    assert_eq!(words.added, vec![0..1]);
}

/// "Descartar" throws the unsaved changes away before the agent writes: the
/// base is the file on disk and the hunk is only the agent's.
#[gpui::test]
fn discard_drops_the_unsaved_changes_before_the_agent_writes(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");
    let editor = open(&parts, &path, cx);
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(0, 0), cx));
    cx.run_until_parked();
    cx.simulate_input("mío ");
    settle(cx);

    start_turn(&parts, cx);
    let mut answer = fs_write(&parts, &path, FIVE_EDITED, cx);
    parts.review.update(cx, |review, cx| {
        review.answer_dirty(DirtyChoice::Discard, cx)
    });
    settle(cx);
    assert!(matches!(answer.try_recv(), Ok(Ok(()))));
    end_turn(&parts, cx);

    assert_eq!(read(&path), FIVE_EDITED);
    assert_eq!(editor.read_with(cx, |editor, _| editor.text()), FIVE_EDITED);
    let base = parts.review.read_with(cx, |review, _| {
        review.store().file(&path).map(|f| f.base.to_string())
    });
    assert_eq!(
        base.as_deref(),
        Some(FIVE),
        "lo tuyo no quedó en ningún lado"
    );
    assert_eq!(hunk_count(&parts, &path, cx), 2);
}

/// A tool call first heard of when it is already complete (the disk holds
/// the agent's text by then): the `oldText` of its diff is the base.
#[gpui::test]
fn a_late_tool_call_uses_the_old_text_of_its_diff(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("b.txt");
    start_turn(&parts, cx);
    std::fs::write(&path, FIVE_EDITED).unwrap();
    let diff = Diff::new(path.clone(), FIVE_EDITED).old_text(FIVE.to_string());
    agent(
        &parts,
        AgentEvent::Update {
            session_id: sid(),
            update: Box::new(SessionUpdate::ToolCall(
                ToolCall::new("tarde".to_string(), "Editar b.txt")
                    .kind(ToolKind::Edit)
                    .status(ToolCallStatus::Completed)
                    .content(vec![ToolCallContent::Diff(diff)]),
            )),
        },
        cx,
    );
    end_turn(&parts, cx);

    assert_eq!(hunk_count(&parts, &path, cx), 2);
    let base = parts.review.read_with(cx, |review, _| {
        review.store().file(&path).map(|f| f.base.to_string())
    });
    assert_eq!(base.as_deref(), Some(FIVE));
}

// ------------------------------------------------------------- turn ended

/// Whether the editor of `path` still has its review controls disabled.
fn dimmed(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> bool {
    editor_of(parts, path, cx)
        .expect("la pestaña está abierta")
        .read_with(cx, |editor, _| editor.review().turn_active)
}

/// Every stop reason ends the review turn: every open editor gets
/// `turn_active == false`, and so does a tab opened after the turn ended
/// (`docs/etapas/etapa-3.md` § correcciones, item 5).
#[gpui::test]
fn every_stop_reason_releases_the_review_controls(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let reasons = [
        StopReason::EndTurn,
        StopReason::MaxTokens,
        StopReason::MaxTurnRequests,
        StopReason::Refusal,
        StopReason::Cancelled,
    ];
    for (index, reason) in reasons.into_iter().enumerate() {
        let open_now = dir.path().join(format!("abierto-{index}.txt"));
        let open_later = dir.path().join(format!("despues-{index}.txt"));
        std::fs::write(&open_now, FIVE).unwrap();
        std::fs::write(&open_later, FIVE).unwrap();
        open(&parts, &open_now, cx);

        start_turn(&parts, cx);
        agent(&parts, edit_call(&format!("e-{index}"), &open_now), cx);
        write_now(&parts, &open_now, FIVE_EDITED, cx);
        write_now(&parts, &open_later, FIVE_EDITED, cx);
        agent(&parts, call_done(&format!("e-{index}")), cx);
        assert!(
            dimmed(&parts, &open_now, cx),
            "{reason:?}: durante el turno"
        );

        agent(
            &parts,
            AgentEvent::TurnEnded {
                session_id: sid(),
                stop_reason: reason,
            },
            cx,
        );

        let summary = parts
            .review
            .read_with(cx, |review, _| review.summary().borrow().turn_active);
        assert!(!summary, "{reason:?}: el resumen sigue en turno");
        let tabs: Vec<PathBuf> = parts.center.read_with(cx, |center, _| {
            center.tabs().iter().map(|tab| tab.path.clone()).collect()
        });
        for tab in &tabs {
            assert!(
                !dimmed(&parts, tab, cx),
                "{reason:?}: {} atenuado",
                tab.display()
            );
        }
        open(&parts, &open_later, cx);
        assert!(
            !dimmed(&parts, &open_later, cx),
            "{reason:?}: la pestaña abierta después del turno quedó atenuada"
        );
        assert_eq!(hunk_count(&parts, &open_later, cx), 2, "{reason:?}");
    }
}

/// The same with the real fake agent: an auto-answered `edit` permission, a
/// write, the final message and `end_turn`; then an `execute` permission that
/// reaches the chat and a cancel while it is still pending. After each,
/// nothing is dimmed, including a tab opened afterwards.
#[gpui::test]
fn the_fake_agent_turns_release_the_review_controls(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let demo = dir.path().join("demo.txt");
    open(&parts, &a, cx);

    let _env = activate_fake(&parts, cx);
    cx.run_until_parked();
    drain_until(&parts, cx, |event| {
        matches!(event, AgentEvent::SessionCreated { .. })
    });
    let session_id = parts
        .chat
        .read_with(cx, |chat, _| chat.session_id().cloned().expect("sesión"));

    // Turn 1: the default scenario writes `demo.txt` (not open yet).
    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(
            &AgentCommand::Prompt {
                session_id: session_id.clone(),
                blocks: vec![PromptBlock::Text("hola".to_string())],
                feedback: None,
            },
            cx,
        )
    });
    drain_until(&parts, cx, |event| {
        matches!(event, AgentEvent::TurnEnded { .. })
    });
    settle(cx);
    assert!(!dimmed(&parts, &a, cx));
    open(&parts, &demo, cx);
    assert!(
        !dimmed(&parts, &demo, cx),
        "demo.txt abierto después del turno"
    );
    assert_eq!(hunk_count(&parts, &demo, cx), 1);

    // Turn 2: an `execute` permission reaches the chat; the user stops the
    // turn while it is pending.
    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(
            &AgentCommand::Prompt {
                session_id: session_id.clone(),
                blocks: vec![PromptBlock::Text("execute-permission".to_string())],
                feedback: None,
            },
            cx,
        )
    });
    drain_until(&parts, cx, |event| {
        matches!(event, AgentEvent::PermissionRequest { .. })
    });
    assert!(
        parts
            .chat
            .read_with(cx, |chat, _| chat.is_awaiting_permission()),
        "el execute llega al chat"
    );
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.is_turn_active())
    );
    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(&AgentCommand::Cancel { session_id }, cx)
    });
    let mut stop = None;
    let events = parts
        .agents
        .read_with(cx, |agents, _| agents.connection_events())
        .expect("hay conexión");
    loop {
        let event = events.recv_blocking().expect("evento del agente falso");
        if let AgentEvent::TurnEnded { stop_reason, .. } = &event {
            stop = Some(*stop_reason);
        }
        agent(&parts, event, cx);
        if stop.is_some() {
            break;
        }
    }
    // The fake agent reads the `cancelled` permission outcome as a normal
    // answer and ends with `end_turn`; the hand-made test above covers the
    // `cancelled` stop reason itself.
    assert!(stop.is_some());
    assert!(
        !parts
            .review
            .read_with(cx, |review, _| review.is_turn_active())
    );
    for path in [&a, &demo] {
        assert!(!dimmed(&parts, path, cx), "{} atenuado", path.display());
    }
    let b = dir.path().join("b.txt");
    open(&parts, &b, cx);
    assert!(!dimmed(&parts, &b, cx));
}

/// Connects the fake agent and returns its session.
fn connect_fake(parts: &Parts, cx: &mut VisualTestContext) -> (FakeEnv, SessionId) {
    let env = activate_fake(parts, cx);
    cx.run_until_parked();
    drain_until(parts, cx, |event| {
        matches!(event, AgentEvent::SessionCreated { .. })
    });
    let session = parts
        .chat
        .read_with(cx, |chat, _| chat.session_id().cloned().expect("sesión"));
    (env, session)
}

fn prompt(parts: &Parts, session_id: &SessionId, text: &str, cx: &mut VisualTestContext) {
    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(
            &AgentCommand::Prompt {
                session_id: session_id.clone(),
                blocks: vec![PromptBlock::Text(text.to_string())],
                feedback: None,
            },
            cx,
        )
    });
    settle(cx);
}

/// `cincel-acp` reports a failed `session/prompt` as `AgentEvent::Error`
/// and never sends its `TurnEnded`: the review turn has to end anyway, or
/// the editors keep their controls disabled for good.
#[gpui::test]
fn a_failed_prompt_releases_the_review_controls(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    open(&parts, &a, cx);
    let (_env, session_id) = connect_fake(&parts, cx);

    prompt(&parts, &session_id, "never-end", cx);
    write_now(&parts, &a, FIVE_EDITED, cx);
    assert!(dimmed(&parts, &a, cx), "durante el turno");

    agent(
        &parts,
        AgentEvent::Error {
            message: "el agente no pudo terminar el turno".to_string(),
            stderr_tail: String::new(),
        },
        cx,
    );
    assert!(
        !parts
            .review
            .read_with(cx, |review, _| review.is_turn_active())
    );
    assert!(!dimmed(&parts, &a, cx), "el turno fallido sigue atenuando");
    assert_eq!(
        parts.chat.read_with(cx, |chat, _| chat.status()),
        cincel_chat::AgentStatus::Ready
    );
    assert_eq!(hunk_count(&parts, &a, cx), 2, "lo escrito queda pendiente");
    let b = dir.path().join("b.txt");
    open(&parts, &b, cx);
    assert!(!dimmed(&parts, &b, cx));

    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(&AgentCommand::Cancel { session_id }, cx)
    });
    drain_until(&parts, cx, |event| {
        matches!(event, AgentEvent::TurnEnded { .. })
    });
}

/// An error from another request sent during the turn (here the fake agent
/// does not implement `session/set_mode`) is not the prompt's: the turn stays
/// open until its own `TurnEnded`.
#[gpui::test]
fn an_error_from_another_request_keeps_the_turn_open(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    open(&parts, &a, cx);
    let (_env, session_id) = connect_fake(&parts, cx);

    prompt(&parts, &session_id, "never-end", cx);
    write_now(&parts, &a, FIVE_EDITED, cx);
    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(
            &AgentCommand::SetMode {
                session_id: session_id.clone(),
                mode_id: "plan".into(),
            },
            cx,
        )
    });
    drain_until(&parts, cx, |event| {
        matches!(event, AgentEvent::Error { .. })
    });
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.is_turn_active())
    );
    assert!(dimmed(&parts, &a, cx), "el turno sigue corriendo");

    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(&AgentCommand::Cancel { session_id }, cx)
    });
    drain_until(&parts, cx, |event| {
        matches!(event, AgentEvent::TurnEnded { .. })
    });
    assert!(!dimmed(&parts, &a, cx));
}
