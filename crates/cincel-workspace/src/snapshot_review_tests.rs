//! GPUI tests of the v2 review: the photo of the project is the base, the
//! tools are only a hint (`docs/specs/03-arquitectura.md` §4,
//! `crate::review`, `review_snapshot.rs`).
//!
//! Most turns are run by the real fake agent with `FAKE_SHELL_EDITS`: it
//! writes files straight on disk during the prompt and only reports one
//! `execute` tool call that names no path, the way a `python3 - <<'EOF'`
//! would. The test projects run without a watcher (GPUI's deterministic
//! executor), so the watcher's batches are handed to
//! [`Project::apply_fs_events`] by hand, exactly as its drain would; the
//! end-of-turn sweep needs nothing.
//!
//! ```text
//! cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins
//! cargo test -p cincel-workspace --features test-support
//! ```

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cincel_acp::acp::schema::v1::{
    SessionId, SessionUpdate, StopReason, ToolCall, ToolCallLocation, ToolCallStatus,
    ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use cincel_acp::{AgentCommand, AgentEvent, PromptBlock};
use cincel_chat::ChatPanel;
use cincel_editor::{EditorView, ReviewAction};
use cincel_project::FsEvent;
use cincel_settings::Config;
use cincel_text::Point;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::agents::Agents;
use crate::center::CenterPanel;
use crate::project::ProjectOptions;
use crate::review::{FileState, Review};
use crate::test_support::{FakeEnv, isolate_state};
use crate::workspace::{Workspace, WorkspaceOptions};

const CALC: &str = "def suma(a, b):\n    return a + b\n\n\ndef resta(a, b):\n    return a - b\n";
const CALC_SHELL: &str =
    "def suma(a, b):\n    return a + b + 0\n\n\ndef resta(x, y):\n    return x - y\n";
const EXPR: &str = "import re\n\nNUM = re.compile(r\"\\d+\")\n";
const EXPR_SHELL: &str = "import re\n\nNUM = re.compile(r\"-?\\d+\")\n";
const FIVE: &str = "a\nb\nc\nd\ne\n";
const FIVE_EDITED: &str = "a\nB\nc\nd\nE\n";

fn init(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

/// `calc.py`, `expr.py`, `a.txt`, `b.txt` and `viejo.py`.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("calc.py"), CALC).unwrap();
    std::fs::write(dir.path().join("expr.py"), EXPR).unwrap();
    std::fs::write(dir.path().join("a.txt"), FIVE).unwrap();
    std::fs::write(dir.path().join("b.txt"), FIVE).unwrap();
    std::fs::write(dir.path().join("viejo.py"), "print(\"adiós\")\n").unwrap();
    dir
}

/// `FAKE_SHELL_EDITS` for a text: `\n` escaped, as the fake agent expects.
fn escaped(text: &str) -> String {
    text.replace('\n', "\\n")
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

fn open(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> Entity<EditorView> {
    cx.update(|window, cx| {
        parts
            .center
            .update(cx, |center, cx| center.open_file(path, true, window, cx));
    });
    settle(cx);
    parts
        .center
        .read_with(cx, |center, _| {
            center
                .tabs()
                .iter()
                .find(|tab| tab.path == path)
                .map(|tab| tab.editor().clone())
        })
        .expect("la pestaña está abierta")
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

/// What sending a prompt does to the review (hand-driven turns).
fn start_turn(parts: &Parts, cx: &mut VisualTestContext) {
    parts
        .agents
        .update(cx, |agents, cx| agents.prepare_prompt(None, cx));
    settle(cx);
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

/// What the watcher's drain does with one batch.
fn watcher(parts: &Parts, events: Vec<FsEvent>, cx: &mut VisualTestContext) {
    let project = parts
        .workspace
        .read_with(cx, |workspace, _| workspace.project().cloned())
        .expect("hay proyecto");
    project.update(cx, |project, cx| project.apply_fs_events(&events, cx));
    settle(cx);
}

/// Connects the fake Claude agent with `FAKE_SHELL_EDITS = edits`. Keep the
/// returned [`FakeEnv`] alive for the whole test.
fn connect_shell_agent(
    parts: &Parts,
    edits: &str,
    cx: &mut VisualTestContext,
) -> (FakeEnv, SessionId) {
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
    let session = parts
        .chat
        .read_with(cx, |chat, _| chat.session_id().cloned().expect("sesión"));
    (env, session)
}

fn prompt(parts: &Parts, session: &SessionId, cx: &mut VisualTestContext) {
    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(
            &AgentCommand::Prompt {
                session_id: session.clone(),
                blocks: vec![PromptBlock::Text("arreglá el parser".to_string())],
                feedback: None,
            },
            cx,
        )
    });
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

/// The `execute` tool call of the shell edits reported its completion: the
/// files are on disk, the turn is still running.
fn is_shell_done(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::Update { update, .. } if matches!(
            update.as_ref(),
            SessionUpdate::ToolCallUpdate(update)
                if update.fields.status == Some(ToolCallStatus::Completed)
        )
    )
}

fn is_turn_end(event: &AgentEvent) -> bool {
    matches!(event, AgentEvent::TurnEnded { .. })
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

fn base_of(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> Option<String> {
    parts.review.read_with(cx, |review, _| {
        review.store().file(path).map(|file| file.base.to_string())
    })
}

fn state_of(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> Option<FileState> {
    parts.review.read_with(cx, |review, _| {
        review.summary().borrow().file(path).map(|file| file.state)
    })
}

fn root_label(parts: &Parts, cx: &mut VisualTestContext) -> Option<String> {
    parts.workspace.read_with(cx, |workspace, cx| {
        workspace.files().read(cx).root_review_label()
    })
}

fn tree_label(parts: &Parts, relative: &str, cx: &mut VisualTestContext) -> Option<String> {
    parts.workspace.read_with(cx, |workspace, cx| {
        workspace
            .files()
            .read(cx)
            .review_label(Path::new(relative), cx)
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

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

// ------------------------------------------------------------------ tests

/// (a) A script run by the agent rewrites an open file: the editor paints
/// red and green segments and the tree counts "1 archivo · 2 cambios".
#[gpui::test]
fn a_shell_edit_of_an_open_file_is_reviewed_inline(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let calc = dir.path().join("calc.py");
    let editor = open(&parts, &calc, cx);
    let edits = format!("calc.py={}", escaped(CALC_SHELL));
    let (_env, session) = connect_shell_agent(&parts, &edits, cx);

    prompt(&parts, &session, cx);
    drain_until(&parts, cx, is_shell_done);
    assert_eq!(read(&calc), CALC_SHELL, "el script ya escribió");
    // The watcher reports it while the turn still runs.
    watcher(&parts, vec![FsEvent::Modified(calc.clone())], cx);
    assert_eq!(hunk_count(&parts, &calc, cx), 2, "llegó por el vigilante");
    drain_until(&parts, cx, is_turn_end);
    settle(cx);

    assert_eq!(base_of(&parts, &calc, cx).as_deref(), Some(CALC));
    let view = editor.read_with(cx, |editor, _| editor.review().clone());
    assert_eq!(view.hunks.len(), 2, "{view:?}");
    assert!(!view.turn_active);
    assert_eq!(view.hunks[0].deleted_lines, vec!["    return a + b"]);
    assert_eq!(view.hunks[0].buffer_rows, 1..2);
    assert_eq!(
        view.hunks[1].deleted_lines,
        vec!["def resta(a, b):", "    return a - b"]
    );
    assert_eq!(view.hunks[1].buffer_rows, 4..6);
    assert_eq!(editor.read_with(cx, |editor, _| editor.text()), CALC_SHELL);
    assert_eq!(
        root_label(&parts, cx).as_deref(),
        Some("1 archivo · 2 cambios · +3 −3")
    );
    assert_eq!(tree_label(&parts, "calc.py", cx).as_deref(), Some("+3 −3"));
}

/// (b) The same for a file the user did not have open: the tree marks it,
/// and opening it shows the segments.
#[gpui::test]
fn a_shell_edit_of_a_file_that_is_not_open_is_marked_and_reviewable(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let expr = dir.path().join("expr.py");
    let edits = format!("expr.py={}", escaped(EXPR_SHELL));
    let (_env, session) = connect_shell_agent(&parts, &edits, cx);

    prompt(&parts, &session, cx);
    drain_until(&parts, cx, is_shell_done);
    watcher(&parts, vec![FsEvent::Modified(expr.clone())], cx);
    drain_until(&parts, cx, is_turn_end);
    settle(cx);

    assert_eq!(tree_label(&parts, "expr.py", cx).as_deref(), Some("+1 −1"));
    assert_eq!(state_of(&parts, &expr, cx), Some(FileState::Modified));
    assert_eq!(
        root_label(&parts, cx).as_deref(),
        Some("1 archivo · 1 cambio · +1 −1")
    );
    let editor = open(&parts, &expr, cx);
    let view = editor.read_with(cx, |editor, _| editor.review().clone());
    assert_eq!(view.hunks.len(), 1);
    assert_eq!(
        view.hunks[0].deleted_lines,
        vec!["NUM = re.compile(r\"\\d+\")"]
    );
    assert_eq!(view.hunks[0].buffer_rows, 2..3);
}

/// (c) A file created and a file deleted by the script: both in review,
/// and rejecting them deletes the new one and brings the old one back.
#[gpui::test]
fn files_created_and_deleted_by_a_script_are_reviewed(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let nuevo = dir.path().join("pkg/nuevo.py");
    let viejo = dir.path().join("viejo.py");
    let (_env, session) = connect_shell_agent(&parts, "pkg/nuevo.py=@x = 1\\n;viejo.py=", cx);

    prompt(&parts, &session, cx);
    drain_until(&parts, cx, is_shell_done);
    assert!(nuevo.is_file() && !viejo.exists());
    watcher(
        &parts,
        vec![
            FsEvent::Created(dir.path().join("pkg")),
            FsEvent::Removed(viejo.clone()),
        ],
        cx,
    );
    drain_until(&parts, cx, is_turn_end);
    settle(cx);

    assert_eq!(state_of(&parts, &nuevo, cx), Some(FileState::Created));
    assert_eq!(state_of(&parts, &viejo, cx), Some(FileState::Deleted));
    let rows = parts.review.read_with(cx, |review, _| review.panel_rows());
    assert!(
        rows.iter()
            .any(|row| row.created && row.relative == "pkg/nuevo.py")
    );
    assert!(
        rows.iter()
            .any(|row| row.deleted && row.relative == "viejo.py")
    );

    parts
        .review
        .update(cx, |review, cx| review.reject_file(&nuevo, cx));
    parts
        .review
        .update(cx, |review, cx| review.reject_file(&viejo, cx));
    settle(cx);
    assert!(!nuevo.exists(), "el archivo nuevo se borra");
    assert_eq!(read(&viejo), "print(\"adiós\")\n", "el borrado vuelve");
    assert_eq!(
        parts
            .review
            .read_with(cx, |review, _| review.pending_count()),
        0
    );
}

/// (d) A rename by the script is a deletion plus a creation.
#[gpui::test]
fn a_rename_by_a_script_is_a_deletion_plus_a_creation(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let calc = dir.path().join("calc.py");
    let renamed = dir.path().join("calculo.py");
    let (_env, session) = connect_shell_agent(&parts, "calc.py=>calculo.py", cx);

    prompt(&parts, &session, cx);
    drain_until(&parts, cx, is_shell_done);
    watcher(
        &parts,
        vec![FsEvent::Renamed {
            from: calc.clone(),
            to: renamed.clone(),
        }],
        cx,
    );
    assert_eq!(state_of(&parts, &calc, cx), Some(FileState::Deleted));
    assert_eq!(state_of(&parts, &renamed, cx), Some(FileState::Created));
    drain_until(&parts, cx, is_turn_end);
    settle(cx);

    assert_eq!(state_of(&parts, &calc, cx), Some(FileState::Deleted));
    assert_eq!(state_of(&parts, &renamed, cx), Some(FileState::Created));
    assert_eq!(read(&renamed), CALC);
    // Rejecting both undoes the rename.
    parts.review.update(cx, |review, cx| review.reject_all(cx));
    settle(cx);
    assert_eq!(read(&calc), CALC);
    assert!(!renamed.exists());
}

/// (e) The user saves another file with `Ctrl+S` while the agent works:
/// that change is the user's, not the agent's.
#[gpui::test]
fn a_user_save_during_the_turn_is_not_the_agents(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let calc = dir.path().join("calc.py");
    let b = dir.path().join("b.txt");
    let edits = format!("calc.py={}", escaped(CALC_SHELL));
    let (_env, session) = connect_shell_agent(&parts, &edits, cx);

    prompt(&parts, &session, cx);
    drain_until(&parts, cx, is_shell_done);
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.is_turn_active())
    );
    let editor = open(&parts, &b, cx);
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(0, 0), cx));
    cx.run_until_parked();
    cx.simulate_input("mío ");
    cx.simulate_keystrokes("ctrl-s");
    settle(cx);
    assert_eq!(read(&b), "mío a\nb\nc\nd\ne\n", "se guardó");
    watcher(
        &parts,
        vec![
            FsEvent::Modified(b.clone()),
            FsEvent::Modified(calc.clone()),
        ],
        cx,
    );
    drain_until(&parts, cx, is_turn_end);
    settle(cx);

    assert!(
        state_of(&parts, &b, cx).is_none(),
        "lo que guardó el usuario no es del agente"
    );
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.store().file(&b).is_none())
    );
    assert_eq!(state_of(&parts, &calc, cx), Some(FileState::Modified));
    assert_eq!(hunk_count(&parts, &calc, cx), 2);
}

/// (f) A change made right before the turn ends, with no watcher event (the
/// test projects have no watcher at all), and one that even keeps size and
/// modification time: the final sweep finds both.
#[gpui::test]
fn the_final_sweep_finds_changes_the_watcher_missed(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");

    start_turn(&parts, cx);
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.is_photo_ready())
    );
    // Different size, no event.
    std::fs::write(&a, "a\nb\nc\nd\ne\nf\n").unwrap();
    // Same size and the very same modification time as in the photo.
    let before = std::fs::metadata(&b).unwrap().modified().unwrap();
    std::fs::write(&b, FIVE_EDITED).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&b)
        .unwrap()
        .set_modified(before)
        .unwrap();
    assert_eq!(std::fs::metadata(&b).unwrap().modified().unwrap(), before);
    assert_eq!(hunk_count(&parts, &a, cx), 0, "nadie avisó todavía");
    end_turn(&parts, cx);

    assert_eq!(hunk_count(&parts, &a, cx), 1);
    assert_eq!(base_of(&parts, &a, cx).as_deref(), Some(FIVE));
    assert_eq!(hunk_count(&parts, &b, cx), 2);
    assert_eq!(base_of(&parts, &b, cx).as_deref(), Some(FIVE));
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.photo().is_none()),
        "la foto se libera al terminar"
    );
}

/// (g) Rejecting the segments of a shell edit puts back exactly what the
/// photo had.
#[gpui::test]
fn rejecting_a_shell_edit_restores_the_photo_exactly(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let calc = dir.path().join("calc.py");
    let edits = format!("calc.py={}", escaped(CALC_SHELL));
    let (_env, session) = connect_shell_agent(&parts, &edits, cx);

    prompt(&parts, &session, cx);
    drain_until(&parts, cx, is_turn_end);
    settle(cx);
    let ids = hunk_ids(&parts, &calc, cx);
    assert_eq!(ids.len(), 2, "el repaso final lo encontró");

    act(&parts, &calc, ReviewAction::RejectHunk(ids[1]), cx);
    assert_eq!(
        read(&calc),
        "def suma(a, b):\n    return a + b + 0\n\n\ndef resta(a, b):\n    return a - b\n",
        "solo ese segmento"
    );
    act(&parts, &calc, ReviewAction::RejectHunk(ids[0]), cx);
    assert_eq!(read(&calc), CALC, "exactamente la foto");
    assert_eq!(hunk_count(&parts, &calc, cx), 0);
}

/// (h) The photo of 5 000 files (50 MB) through the real path of a prompt:
/// well under a second (tolerant: fails only over 3 s).
#[gpui::test]
fn the_photo_of_five_thousand_files_is_fast(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let line = "let valor = 42; // una línea de relleno para la foto\n";
    let body = line.repeat(10_000 / line.len() + 1);
    for index in 0..5_000 {
        let folder = dir.path().join(format!("m{:02}", index % 50));
        if index < 50 {
            std::fs::create_dir_all(&folder).unwrap();
        }
        std::fs::write(folder.join(format!("f{index}.rs")), &body).unwrap();
    }
    let (parts, cx) = window(dir.path(), cx);

    let started = Instant::now();
    parts
        .agents
        .update(cx, |agents, cx| agents.prepare_prompt(None, cx));
    let elapsed = started.elapsed();
    let (files, bytes, own) = parts.review.read_with(cx, |review, _| {
        let photo = review.photo().expect("foto lista");
        (photo.len(), photo.copied_bytes(), photo.elapsed())
    });
    eprintln!("foto de {files} archivos / {bytes} bytes: {own:?} (prompt listo en {elapsed:?})");
    assert_eq!(files, 5_000);
    assert!(bytes >= 50_000_000);
    assert!(elapsed < Duration::from_secs(3), "tardó {elapsed:?}");
    end_turn(&parts, cx);
}

/// (i) An edit through a tool call gives exactly what it gave before the
/// photo (the reference is `review_tests::
/// an_edit_straight_to_disk_is_reviewed_after_the_tool_call`).
#[gpui::test]
fn an_edit_tool_call_still_gives_the_same_review(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let path = dir.path().join("a.txt");

    start_turn(&parts, cx);
    agent(
        &parts,
        AgentEvent::Update {
            session_id: sid(),
            update: Box::new(SessionUpdate::ToolCall(
                ToolCall::new("c1".to_string(), "Editar")
                    .kind(ToolKind::Edit)
                    .status(ToolCallStatus::InProgress)
                    .locations(vec![ToolCallLocation::new(path.clone())]),
            )),
        },
        cx,
    );
    assert_eq!(base_of(&parts, &path, cx).as_deref(), Some(FIVE));
    std::fs::write(&path, FIVE_EDITED).unwrap();
    assert_eq!(hunk_count(&parts, &path, cx), 0, "nada hasta que termina");
    agent(
        &parts,
        AgentEvent::Update {
            session_id: sid(),
            update: Box::new(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "c1".to_string(),
                ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
            ))),
        },
        cx,
    );
    end_turn(&parts, cx);

    assert_eq!(hunk_count(&parts, &path, cx), 2);
    assert_eq!(base_of(&parts, &path, cx).as_deref(), Some(FIVE));
    let editor = open(&parts, &path, cx);
    let rows: Vec<_> = editor.read_with(cx, |editor, _| {
        editor
            .review()
            .hunks
            .iter()
            .map(|hunk| (hunk.buffer_rows.clone(), hunk.deleted_lines.clone()))
            .collect()
    });
    assert_eq!(
        rows,
        vec![(1..2, vec!["b".to_string()]), (4..5, vec!["e".to_string()])]
    );
    // Nothing else of the project entered the review.
    let pending: Vec<PathBuf> = parts.review.read_with(cx, |review, _| {
        review.summary().borrow().files.keys().cloned().collect()
    });
    assert_eq!(pending, vec![path]);
}

/// Chained turns: the second turn takes its own photo, but a file still
/// pending from the first keeps its original base.
#[gpui::test]
fn chained_turns_keep_the_first_base(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");

    start_turn(&parts, cx);
    std::fs::write(&a, "a\nB\nc\nd\ne\n").unwrap();
    end_turn(&parts, cx);
    assert_eq!(hunk_count(&parts, &a, cx), 1);

    start_turn(&parts, cx);
    std::fs::write(&a, FIVE_EDITED).unwrap();
    watcher(&parts, vec![FsEvent::Modified(a.clone())], cx);
    end_turn(&parts, cx);
    assert_eq!(
        base_of(&parts, &a, cx).as_deref(),
        Some(FIVE),
        "no se re-basa"
    );
    assert_eq!(hunk_count(&parts, &a, cx), 2);
}

/// Binary files are out of the review: changed (watcher), created and
/// deleted (the sweep) by the agent, nothing is pending and the disk keeps
/// what the agent left. Opening one shows the notice, not its bytes, and no
/// review buttons.
#[gpui::test]
fn binary_files_changed_by_the_agent_stay_out_of_the_review(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let image = dir.path().join("logo.bin");
    let gone = dir.path().join("viejo.bin");
    let created = dir.path().join("nuevo.png");
    std::fs::write(&image, [0xff_u8, 0xd8, 0x00, 0x10, 0x80]).unwrap();
    std::fs::write(&gone, [0x00_u8, 0x01, 0xfe]).unwrap();
    let (parts, cx) = window(dir.path(), cx);

    start_turn(&parts, cx);
    let changed = [0xff_u8, 0xd8, 0x00, 0x11, 0x81, 0x90];
    std::fs::write(&image, changed).unwrap();
    watcher(&parts, vec![FsEvent::Modified(image.clone())], cx);
    std::fs::write(&created, [0x89_u8, b'P', b'N', b'G', 0x00]).unwrap();
    std::fs::remove_file(&gone).unwrap();
    end_turn(&parts, cx);

    for (path, relative) in [
        (&image, "logo.bin"),
        (&gone, "viejo.bin"),
        (&created, "nuevo.png"),
    ] {
        assert!(state_of(&parts, path, cx).is_none(), "{relative}");
        assert!(tree_label(&parts, relative, cx).is_none(), "{relative}");
    }
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.panel_rows())
            .is_empty()
    );
    assert_eq!(
        parts
            .review
            .read_with(cx, |review, _| review.pending_count()),
        0
    );
    assert_eq!(std::fs::read(&image).unwrap(), changed);
    assert!(created.is_file() && !gone.exists());

    // The tab of a binary: the notice, read only, no review.
    let editor = open(&parts, &image, cx);
    let (text, read_only, hunks) = editor.read_with(cx, |editor, _| {
        (
            editor.text(),
            editor.is_read_only(),
            editor.review().hunks.len(),
        )
    });
    assert_eq!(text, crate::center::BINARY_NOTICE);
    assert!(read_only);
    assert_eq!(hunks, 0);
}

/// With the photo taken on the background executor (the real application),
/// the prompt waits for it: nothing goes out until it is ready.
#[gpui::test]
fn the_prompt_waits_for_the_photo(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let calc = dir.path().join("calc.py");
    let edits = format!("calc.py={}", escaped(CALC_SHELL));
    let (_env, session) = connect_shell_agent(&parts, &edits, cx);
    parts
        .review
        .update(cx, |review, _| review.set_photo_in_background(true));

    prompt(&parts, &session, cx);
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.is_turn_active()
                && !review.is_photo_ready()),
        "la foto se está sacando"
    );
    assert_eq!(read(&calc), CALC, "el prompt todavía no salió");
    cx.run_until_parked();
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.is_photo_ready())
    );
    drain_until(&parts, cx, is_turn_end);
    settle(cx);
    assert_eq!(base_of(&parts, &calc, cx).as_deref(), Some(CALC));
    assert_eq!(hunk_count(&parts, &calc, cx), 2);
}

/// Stopping a prompt that is still waiting for its photo: it never goes
/// out, the review turn ends and the chat is free again.
#[gpui::test]
fn cancelling_while_the_photo_is_taken_drops_the_prompt(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let calc = dir.path().join("calc.py");
    let edits = format!("calc.py={}", escaped(CALC_SHELL));
    let (_env, session) = connect_shell_agent(&parts, &edits, cx);
    parts
        .review
        .update(cx, |review, _| review.set_photo_in_background(true));

    prompt(&parts, &session, cx);
    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(
            &AgentCommand::Cancel {
                session_id: session,
            },
            cx,
        )
    });
    settle(cx);
    assert!(
        !parts
            .review
            .read_with(cx, |review, _| review.is_turn_active())
    );
    assert_eq!(
        parts.chat.read_with(cx, |chat, _| chat.status()),
        cincel_chat::AgentStatus::Ready
    );
    assert_eq!(read(&calc), CALC, "el agente nunca recibió el prompt");
    assert_eq!(
        parts
            .review
            .read_with(cx, |review, _| review.pending_count()),
        0
    );
}
