//! GPUI tests of a file the agent deleted, still waiting for a decision
//! (`crate::review::Review::view_for_tab`, `crate::center`'s
//! `deleted_review` tabs, `crate::tree_panel`'s struck-through rows).
//!
//! The turns are run by the real fake agent with `FAKE_SHELL_EDITS`
//! (`ruta=` deletes, `ruta=@…` creates): it changes the files straight on
//! disk and reports one `execute` tool call that names no path. The test
//! projects run without a watcher, so its batches are handed to
//! [`Project::apply_fs_events`](crate::project::Project::apply_fs_events)
//! by hand, exactly as its drain would.
//!
//! ```text
//! cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins
//! cargo test -p cincel-workspace --features test-support
//! ```

use std::path::Path;
use std::time::Duration;

use cincel_acp::acp::schema::v1::{SessionId, SessionUpdate, ToolCallStatus};
use cincel_acp::{AgentCommand, AgentEvent, PromptBlock};
use cincel_chat::ChatPanel;
use cincel_editor::{EditorView, ReviewAction, ReviewHunkKind, RowKind};
use cincel_project::FsEvent;
use cincel_settings::Config;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::agents::Agents;
use crate::center::CenterPanel;
use crate::project::ProjectOptions;
use crate::review::{DELETED_FILE_HUNK, DELETED_TOOLTIP, FileState, Review};
use crate::test_support::{FakeEnv, isolate_state};
use crate::theme::ThemeColors;
use crate::toast::ToastKind;
use crate::workspace::{Workspace, WorkspaceOptions};

/// Six lines, the file the agent deletes.
const UNARY: &str = "def neg(x):\n    return -x\n\n\ndef pos(x):\n    return +x\n";
const A_PY: &str = "x = 1\n";

fn init(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

/// `a.py` and `unary.py`.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.py"), A_PY).unwrap();
    std::fs::write(dir.path().join("unary.py"), UNARY).unwrap();
    dir
}

fn unary_lines() -> Vec<String> {
    UNARY.lines().map(str::to_owned).collect()
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

fn agent(parts: &Parts, event: AgentEvent, cx: &mut VisualTestContext) {
    parts
        .agents
        .update(cx, |agents, cx| agents.handle_agent_event(event, cx));
    settle(cx);
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

/// One whole turn of the fake agent running `edits`, with the watcher's
/// batch `events` delivered while it runs. Keep the returned [`FakeEnv`]
/// alive for the whole test.
fn run_turn(
    parts: &Parts,
    edits: &str,
    events: Vec<FsEvent>,
    cx: &mut VisualTestContext,
) -> FakeEnv {
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
                blocks: vec![PromptBlock::Text("limpiá el proyecto".to_string())],
                feedback: None,
            },
            cx,
        )
    });
    drain_until(parts, cx, is_shell_done);
    let project = parts
        .workspace
        .read_with(cx, |workspace, _| workspace.project().cloned())
        .expect("hay proyecto");
    project.update(cx, |project, cx| project.apply_fs_events(&events, cx));
    settle(cx);
    drain_until(parts, cx, is_turn_end);
    settle(cx);
    env
}

/// The agent deletes `unary.py` (and, with `also`, runs those edits too).
fn delete_unary(
    parts: &Parts,
    root: &Path,
    also: &str,
    mut events: Vec<FsEvent>,
    cx: &mut VisualTestContext,
) -> FakeEnv {
    let unary = root.join("unary.py");
    events.push(FsEvent::Removed(unary.clone()));
    let edits = format!("{also}unary.py=");
    let env = run_turn(parts, &edits, events, cx);
    assert!(!unary.exists(), "el agente borró el archivo");
    assert_eq!(
        parts.review.read_with(cx, |review, _| {
            review
                .summary()
                .borrow()
                .file(&unary)
                .map(|file| file.state)
        }),
        Some(FileState::Deleted)
    );
    env
}

/// The tab of `path`: `(deleted_review, read_only, editor)`.
fn tab(
    parts: &Parts,
    path: &Path,
    cx: &mut VisualTestContext,
) -> Option<(bool, bool, Entity<EditorView>)> {
    parts.center.read_with(cx, |center, _| {
        center
            .tabs()
            .iter()
            .find(|tab| tab.path == path)
            .map(|tab| (tab.deleted_review, tab.read_only, tab.editor().clone()))
    })
}

/// Clicks `relative` in the tree (a single click: preview).
fn click_in_tree(parts: &Parts, relative: &str, cx: &mut VisualTestContext) {
    let files = parts
        .workspace
        .read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new(relative), false, cx)
    });
    settle(cx);
}

fn tree_labels(parts: &Parts, cx: &mut VisualTestContext) -> Vec<String> {
    parts.workspace.read_with(cx, |workspace, cx| {
        workspace.files().read(cx).visible_labels(cx)
    })
}

fn error_toasts(parts: &Parts, cx: &mut VisualTestContext) -> Vec<String> {
    parts.workspace.read_with(cx, |workspace, cx| {
        workspace
            .toasts()
            .read(cx)
            .items()
            .iter()
            .filter(|toast| toast.kind == ToastKind::Error)
            .map(|toast| toast.message.to_string())
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

/// Opens the deletion's tab from the tree and checks it is the read-only
/// red view of the whole file.
fn open_deleted_tab(parts: &Parts, unary: &Path, cx: &mut VisualTestContext) -> Entity<EditorView> {
    click_in_tree(parts, "unary.py", cx);
    let (deleted_review, read_only, editor) = tab(parts, unary, cx).expect("se abrió la pestaña");
    assert!(deleted_review && read_only, "pestaña de solo lectura");
    editor
}

// ------------------------------------------------------------------ tests

/// (a) The tree keeps the deleted file, struck through in `status.error`
/// with `−N` and the tooltip; a created file is green with `+N`.
#[gpui::test]
fn the_tree_strikes_the_deleted_file_through(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let _env = delete_unary(
        &parts,
        dir.path(),
        "nuevo.py=@y = 1\\n;",
        vec![FsEvent::Created(dir.path().join("nuevo.py"))],
        cx,
    );

    // The watcher already dropped it from the worktree; the tree still
    // lists it until it is decided.
    assert!(
        tree_labels(&parts, cx).contains(&"unary.py".to_string()),
        "{:?}",
        tree_labels(&parts, cx)
    );
    let (deleted, created, theme) = parts.workspace.read_with(cx, |workspace, cx| {
        let files = workspace.files().read(cx);
        (
            files.review_style(Path::new("unary.py"), cx),
            files.review_style(Path::new("nuevo.py"), cx),
            ThemeColors::global(cx).clone(),
        )
    });
    let deleted = deleted.expect("el borrado tiene estilo de revisión");
    assert!(deleted.strikethrough, "tachado");
    assert_eq!(deleted.color, theme.status_error);
    assert_eq!(deleted.suffix.as_ref(), "−6");
    assert_eq!(deleted.tooltip, Some(DELETED_TOOLTIP));

    let created = created.expect("el creado tiene estilo de revisión");
    assert!(!created.strikethrough);
    assert_eq!(created.color, theme.status_ok);
    assert_eq!(created.suffix.as_ref(), "+1");
    assert_eq!(created.tooltip, None);
}

/// (b) A click opens a read-only tab with the whole file as one red hunk,
/// the bar offering the file's decision, and no error toast.
#[gpui::test]
fn clicking_the_deleted_file_opens_its_red_read_only_tab(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let unary = dir.path().join("unary.py");
    let _env = delete_unary(&parts, dir.path(), "", Vec::new(), cx);

    let editor = open_deleted_tab(&parts, &unary, cx);
    assert!(
        error_toasts(&parts, cx).is_empty(),
        "{:?}",
        error_toasts(&parts, cx)
    );
    let (text, read_only, view) = editor.read_with(cx, |editor, _| {
        (
            editor.text(),
            editor.is_read_only(),
            editor.review().clone(),
        )
    });
    assert_eq!(text, "", "nada se leyó del disco");
    assert!(read_only);
    assert_eq!(view.hunks.len(), 1, "una única sección «archivo borrado»");
    let hunk = &view.hunks[0];
    assert_eq!(hunk.id, DELETED_FILE_HUNK);
    assert_eq!(hunk.kind, ReviewHunkKind::Deleted);
    assert_eq!(hunk.deleted_lines, unary_lines());
    assert!(view.file_actions && !view.turn_active);
    let title = parts
        .center
        .read_with(cx, |center, _| center.active_tab().unwrap().display_title());
    assert_eq!(title.as_ref(), "unary.py (eliminado)");

    // Painted: six red phantom rows and the bar with the file's buttons.
    editor.update(cx, |editor, _| editor.set_render_probe(true));
    // The floating bar only shows in the active window.
    cx.update(|window, _| {
        window.activate_window();
        window.refresh();
    });
    cx.run_until_parked();
    let frame = editor
        .read_with(cx, |editor, _| editor.render_frames().last().cloned())
        .expect("un cuadro pintado");
    let phantoms = frame
        .rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::Phantom(_)))
        .count();
    assert_eq!(phantoms, 6);
    let bar = frame.review.bar.expect("barra flotante");
    for part in [
        "✓ Aceptar archivo",
        "✗ Rechazar archivo",
        "✓ Aceptar todo",
        "✗ Rechazar todo",
        "Revisar todo",
    ] {
        assert!(bar.contains(part), "{part} en {bar}");
    }

    // Typing does nothing.
    cx.simulate_input("x");
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |editor, _| editor.text()), "");
}

/// (c) Rejecting brings the file back and the tab becomes a normal editor
/// of it.
#[gpui::test]
fn rejecting_restores_the_file_and_the_tab_becomes_editable(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let unary = dir.path().join("unary.py");
    let _env = delete_unary(&parts, dir.path(), "", Vec::new(), cx);
    open_deleted_tab(&parts, &unary, cx);

    // The bar's "✗ Rechazar archivo".
    act(&parts, &unary, ReviewAction::RejectFile, cx);

    assert_eq!(std::fs::read_to_string(&unary).unwrap(), UNARY);
    let (deleted_review, read_only, editor) = tab(&parts, &unary, cx).expect("la pestaña sigue");
    assert!(!deleted_review && !read_only);
    let (text, editor_read_only, hunks) = editor.read_with(cx, |editor, _| {
        (
            editor.text(),
            editor.is_read_only(),
            editor.review().hunks.len(),
        )
    });
    assert_eq!(text, UNARY);
    assert!(!editor_read_only);
    assert_eq!(hunks, 0);
    assert!(
        error_toasts(&parts, cx).is_empty(),
        "{:?}",
        error_toasts(&parts, cx)
    );
    let style = parts.workspace.read_with(cx, |workspace, cx| {
        workspace
            .files()
            .read(cx)
            .review_style(Path::new("unary.py"), cx)
    });
    assert_eq!(style, None, "vuelve normal en el árbol");

    // It is a real editor again: typing edits the restored file.
    cx.simulate_input("# ");
    cx.run_until_parked();
    let text = editor.read_with(cx, |editor, _| editor.text());
    assert!(text.starts_with("# def neg"), "{text}");
}

/// (d) Accepting (the pill of the only hunk) closes the tab and drops the
/// file from the tree.
#[gpui::test]
fn accepting_removes_the_file_and_its_tab(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let unary = dir.path().join("unary.py");
    let _env = delete_unary(&parts, dir.path(), "", Vec::new(), cx);
    open_deleted_tab(&parts, &unary, cx);

    act(
        &parts,
        &unary,
        ReviewAction::AcceptHunk(DELETED_FILE_HUNK),
        cx,
    );

    assert!(!unary.exists());
    assert!(tab(&parts, &unary, cx).is_none(), "la pestaña se cerró");
    assert!(!tree_labels(&parts, cx).contains(&"unary.py".to_string()));
    assert_eq!(
        parts
            .review
            .read_with(cx, |review, _| review.pending_count()),
        0
    );
    assert!(
        error_toasts(&parts, cx).is_empty(),
        "{:?}",
        error_toasts(&parts, cx)
    );
}

/// (e) `Alt+L` from another file with changes lands on the deletion, and so
/// does the review panel's row.
#[gpui::test]
fn alt_l_and_the_panel_reach_the_deleted_file(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.py");
    let unary = dir.path().join("unary.py");
    let _env = delete_unary(
        &parts,
        dir.path(),
        "a.py=x = 2\\n;",
        vec![FsEvent::Modified(a.clone())],
        cx,
    );

    cx.update(|window, cx| {
        parts
            .center
            .update(cx, |center, cx| center.open_file(&a, true, window, cx))
    });
    settle(cx);
    cx.simulate_keystrokes("alt-l");
    settle(cx);

    let active = parts.center.read_with(cx, |center, _| {
        center.active_tab().map(|tab| tab.path.clone())
    });
    assert_eq!(active.as_deref(), Some(unary.as_path()));
    let (deleted_review, _, editor) = tab(&parts, &unary, cx).expect("pestaña del borrado");
    assert!(deleted_review);
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.review().hunks[0]
            .deleted_lines
            .clone()),
        unary_lines()
    );

    // The panel ("Revisar todo") opens it too, once its tab is gone.
    let index = parts.center.read_with(cx, |center, _| {
        center
            .tabs()
            .iter()
            .position(|tab| tab.path == unary)
            .unwrap()
    });
    cx.update(|window, cx| {
        parts
            .center
            .update(cx, |center, cx| center.close_tab(index, window, cx))
    });
    settle(cx);
    let rows = parts.review.read_with(cx, |review, _| review.panel_rows());
    assert!(rows.iter().any(|row| row.deleted && row.path == unary));
    cx.update(|window, cx| {
        parts.review.update(cx, |review, cx| {
            review.open_file_at_first_hunk(&unary, window, cx)
        })
    });
    settle(cx);
    let (deleted_review, _, _) = tab(&parts, &unary, cx).expect("abierta desde el panel");
    assert!(deleted_review);
    assert!(
        error_toasts(&parts, cx).is_empty(),
        "{:?}",
        error_toasts(&parts, cx)
    );
}

/// (f) Closing the window counts the deletion as a pending change, from
/// `Alt+F4` and from the title bar's `×` alike.
#[gpui::test]
fn the_close_dialog_counts_the_deletion(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let _env = delete_unary(&parts, dir.path(), "", Vec::new(), cx);

    let may_close = cx.update(|window, cx| {
        parts
            .workspace
            .update(cx, |workspace, cx| workspace.request_close(window, cx))
    });
    assert!(!may_close, "hay un cambio de agente sin decidir");
    assert_eq!(
        parts
            .workspace
            .read_with(cx, |workspace, _| workspace.review_close_dialog_counts()),
        Some((1, 1))
    );
    cx.update(|window, cx| {
        parts.workspace.update(cx, |workspace, cx| {
            workspace.cancel_review_close_for_test(window, cx)
        })
    });
    cx.run_until_parked();

    cx.update(|window, cx| {
        parts.workspace.update(cx, |workspace, cx| {
            workspace.close_from_title_bar(window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "la × no cerró");
    assert_eq!(
        parts
            .workspace
            .read_with(cx, |workspace, _| workspace.review_close_dialog_counts()),
        Some((1, 1))
    );
}
