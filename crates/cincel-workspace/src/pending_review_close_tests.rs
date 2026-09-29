//! GPUI tests of `crate::review_close`: agent changes nobody decided are
//! asked about before quitting, closing the window or replacing the project,
//! and the floating bar's "Rechazar todo" asks before undoing a turn.

use std::path::Path;
use std::time::Duration;

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{SessionId, StopReason};
use cincel_editor::ReviewAction;
use cincel_settings::Config;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::project::ProjectOptions;
use crate::review::Review;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

const FIVE: &str = "a\nb\nc\nd\ne\n";
const FIVE_EDITED: &str = "a\nB\nc\nd\nE\n";

fn init(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

/// `a.txt` and `b.txt`, five lines each.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), FIVE).unwrap();
    std::fs::write(dir.path().join("b.txt"), FIVE).unwrap();
    dir
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

/// Lets the debounced recompute and the background job land.
fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(120));
    cx.run_until_parked();
}

fn review(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<Review> {
    workspace.read_with(cx, |workspace, _| workspace.review().clone())
}

fn agent(workspace: &Entity<Workspace>, event: AgentEvent, cx: &mut VisualTestContext) {
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    agents.update(cx, |agents, cx| agents.handle_agent_event(event, cx));
    settle(cx);
}

/// What sending a prompt does to the review.
fn start_turn(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    agents.update(cx, |agents, cx| agents.prepare_prompt(None, cx));
    settle(cx);
}

/// `fs/write_text_file` of `content` into `path`, answered right away.
fn write(workspace: &Entity<Workspace>, path: &Path, content: &str, cx: &mut VisualTestContext) {
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
}

fn end_turn(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    agent(
        workspace,
        AgentEvent::TurnEnded {
            session_id: SessionId::new("prueba"),
            stop_reason: StopReason::EndTurn,
        },
        cx,
    );
}

/// A whole agent turn writing `content` into `path`.
fn agent_writes(
    workspace: &Entity<Workspace>,
    path: &Path,
    content: &str,
    cx: &mut VisualTestContext,
) {
    start_turn(workspace, cx);
    write(workspace, path, content, cx);
    end_turn(workspace, cx);
}

fn pending(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> (usize, usize) {
    let review = review(workspace, cx);
    review.read_with(cx, |review, _| review.pending_counts())
}

fn dialog_counts(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Option<(usize, usize)> {
    workspace.read_with(cx, |workspace, _| workspace.review_close_dialog_counts())
}

fn quit_dialog_open(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |workspace, _| workspace.is_quit_dialog_open())
}

fn should_close(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| workspace.update(cx, |workspace, cx| workspace.should_close(window, cx)))
}

fn root(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> std::path::PathBuf {
    workspace.read_with(cx, |workspace, cx| {
        workspace.project().unwrap().read(cx).root().to_path_buf()
    })
}

fn tree_label(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<String> {
    workspace.read_with(cx, |workspace, cx| {
        workspace.files().read(cx).root_review_label()
    })
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// Opens `relative` pinned from the tree and types `text`, so its tab is
/// dirty.
fn dirty_a_tab(
    workspace: &Entity<Workspace>,
    relative: &str,
    text: &str,
    cx: &mut VisualTestContext,
) {
    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new(relative), true, cx)
    });
    cx.run_until_parked();
    cx.simulate_input(text);
    cx.run_until_parked();
}

// ------------------------------------------------------------- closing

/// `Ctrl+Q` with pending agent changes asks about them, not about saving.
#[gpui::test]
fn quitting_with_pending_changes_shows_the_dialog(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (workspace, cx) = window(dir.path(), cx);
    agent_writes(&workspace, &dir.path().join("a.txt"), FIVE_EDITED, cx);
    assert_eq!(pending(&workspace, cx), (2, 1));
    assert_eq!(
        tree_label(&workspace, cx).as_deref(),
        Some("1 archivo · 2 cambios · +2 −2")
    );

    cx.simulate_keystrokes("ctrl-q");
    cx.run_until_parked();
    assert_eq!(dialog_counts(&workspace, cx), Some((2, 1)));
    assert!(!quit_dialog_open(&workspace, cx));
    assert!(workspace.read_with(cx, |workspace, cx| workspace.is_modal_open(cx)));
    // The window's `×` asks the same question and does not close.
    assert!(!should_close(&workspace, cx));
    assert_eq!(dialog_counts(&workspace, cx), Some((2, 1)));
}

/// "Aceptar todo" decides every change (the tree has nothing pending) and
/// the quit goes on: the window closes.
#[gpui::test]
fn accept_all_leaves_nothing_pending_and_quits(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    let (review, files) = {
        let (workspace, cx) = window(dir.path(), cx);
        start_turn(&workspace, cx);
        write(&workspace, &a, FIVE_EDITED, cx);
        write(&workspace, &b, "a\nb\nX\nd\ne\n", cx);
        end_turn(&workspace, cx);
        // One more turn, so "Aceptar todo" has to cover every turn.
        agent_writes(&workspace, &b, "a\nb\nX\nd\nY\n", cx);
        assert!(!should_close(&workspace, cx));
        assert_eq!(dialog_counts(&workspace, cx), Some((4, 2)));
        let review = review(&workspace, cx);
        let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.accept_pending_and_close_for_test(window, cx)
            })
        });
        cx.run_until_parked();
        (review, files)
    };
    cx.run_until_parked();
    assert!(cx.windows().is_empty(), "la ventana se cerró");
    assert_eq!(
        review.read_with(cx, |review, _| review.pending_counts()),
        (0, 0)
    );
    assert_eq!(
        files.read_with(cx, |files, _| files.root_review_label()),
        None
    );
    assert_eq!(read(&a), FIVE_EDITED, "aceptar deja lo del agente");
    assert_eq!(read(&b), "a\nb\nX\nd\nY\n");
}

/// "Cancelar" neither closes nor decides anything; `Esc` is "Cancelar".
#[gpui::test]
fn cancel_does_not_quit(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (workspace, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    agent_writes(&workspace, &a, FIVE_EDITED, cx);

    cx.simulate_keystrokes("ctrl-q");
    cx.run_until_parked();
    assert!(dialog_counts(&workspace, cx).is_some());
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.cancel_review_close_for_test(window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(dialog_counts(&workspace, cx), None);
    assert!(!cx.windows().is_empty(), "la ventana sigue abierta");
    assert_eq!(pending(&workspace, cx), (2, 1), "nada se decidió");
    assert_eq!(read(&a), FIVE_EDITED);

    cx.simulate_keystrokes("ctrl-q");
    cx.run_until_parked();
    assert!(dialog_counts(&workspace, cx).is_some());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(dialog_counts(&workspace, cx), None, "Esc cancela");
    assert_eq!(pending(&workspace, cx), (2, 1));
}

/// Opening another folder (recent folders; "Abrir carpeta…" goes through the
/// same `request_open_project`) asks first; "Rechazar todo" reverts the
/// files and then opens the folder.
#[gpui::test]
fn opening_another_folder_with_pending_changes_asks(cx: &mut TestAppContext) {
    init(cx);
    let dir_a = project();
    let dir_b = project();
    let (workspace, cx) = window(dir_a.path(), cx);
    let a = dir_a.path().join("a.txt");
    agent_writes(&workspace, &a, FIVE_EDITED, cx);

    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_recent(dir_b.path(), window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(dialog_counts(&workspace, cx), Some((2, 1)));
    assert_eq!(root(&workspace, cx), dir_a.path(), "todavía no cambió");

    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.reject_pending_and_close_for_test(window, cx)
        })
    });
    settle(cx);
    assert_eq!(dialog_counts(&workspace, cx), None);
    assert_eq!(read(&a), FIVE, "rechazar deshace lo del agente");
    assert_eq!(root(&workspace, cx), dir_b.path());
    assert_eq!(pending(&workspace, cx), (0, 0));

    // Back to the first folder: the decided review does not come back.
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_recent(dir_a.path(), window, cx)
        })
    });
    settle(cx);
    assert_eq!(root(&workspace, cx), dir_a.path());
    assert_eq!(pending(&workspace, cx), (0, 0));
    assert_eq!(tree_label(&workspace, cx), None);
}

/// Without pending changes nothing new is asked: the window closes and the
/// folder opens right away.
#[gpui::test]
fn without_pending_changes_nothing_is_asked(cx: &mut TestAppContext) {
    init(cx);
    let dir_a = project();
    let dir_b = project();
    let (workspace, cx) = window(dir_a.path(), cx);
    assert!(should_close(&workspace, cx));
    assert_eq!(dialog_counts(&workspace, cx), None);

    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_recent(dir_b.path(), window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(dialog_counts(&workspace, cx), None);
    assert_eq!(root(&workspace, cx), dir_b.path());
}

/// Pending changes and an unsaved file: first the changes, then the
/// "¿Guardar cambios?" dialog that already existed.
#[gpui::test]
fn pending_changes_and_a_dirty_file_ask_in_order(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (workspace, cx) = window(dir.path(), cx);
    agent_writes(&workspace, &dir.path().join("a.txt"), FIVE_EDITED, cx);
    dirty_a_tab(&workspace, "b.txt", "sucio ", cx);

    assert!(!should_close(&workspace, cx));
    assert!(
        dialog_counts(&workspace, cx).is_some(),
        "primero los cambios"
    );
    assert!(!quit_dialog_open(&workspace, cx));

    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.accept_pending_and_close_for_test(window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(dialog_counts(&workspace, cx), None);
    assert!(quit_dialog_open(&workspace, cx), "después, guardar");
    assert!(!cx.windows().is_empty(), "todavía no sale");
    assert_eq!(pending(&workspace, cx), (0, 0));
}

/// An agent in the middle of a turn: the turn is stopped first, then its
/// changes are asked about (they can be decided right away).
#[gpui::test]
fn a_running_turn_is_stopped_before_asking(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (workspace, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    start_turn(&workspace, cx);
    write(&workspace, &a, FIVE_EDITED, cx);
    let review = review(&workspace, cx);
    assert!(review.read_with(cx, |review, _| review.is_turn_active()));

    cx.simulate_keystrokes("ctrl-q");
    cx.run_until_parked();
    assert!(!review.read_with(cx, |review, _| review.is_turn_active()));
    assert_eq!(dialog_counts(&workspace, cx), Some((2, 1)));

    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.reject_pending_and_close_for_test(window, cx)
        })
    });
    cx.run_until_parked();
    assert!(cx.windows().is_empty());
    assert_eq!(read(&a), FIVE);
}

// ------------------------------------------------------- floating bar

fn bar_action(
    workspace: &Entity<Workspace>,
    path: &Path,
    action: ReviewAction,
    cx: &mut VisualTestContext,
) {
    let review = review(workspace, cx);
    cx.update(|window, cx| {
        review.update(cx, |review, cx| {
            review.handle_action(path, action, None, window, cx)
        })
    });
    settle(cx);
}

/// The bar's "Aceptar todo" decides the whole turn, every file.
#[gpui::test]
fn the_bar_accepts_the_whole_turn(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (workspace, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    start_turn(&workspace, cx);
    write(&workspace, &a, FIVE_EDITED, cx);
    write(&workspace, &b, FIVE_EDITED, cx);
    end_turn(&workspace, cx);

    bar_action(&workspace, &a, ReviewAction::AcceptTurn, cx);
    assert_eq!(pending(&workspace, cx), (0, 0));
    assert_eq!(read(&a), FIVE_EDITED);
    assert_eq!(read(&b), FIVE_EDITED);
}

/// The bar's "Rechazar todo" asks "Rechazar N cambios en M archivos" first;
/// "Cancelar" keeps everything, confirming reverts every file of the turn and
/// `Alt+Shift+U` brings it back.
#[gpui::test]
fn the_bar_rejects_the_whole_turn_after_confirming(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (workspace, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    start_turn(&workspace, cx);
    write(&workspace, &a, FIVE_EDITED, cx);
    write(&workspace, &b, "a\nb\nX\nd\ne\n", cx);
    end_turn(&workspace, cx);
    let review = review(&workspace, cx);

    bar_action(&workspace, &a, ReviewAction::RejectTurn, cx);
    assert_eq!(
        review.read_with(cx, |review, _| review.reject_turn_prompt()),
        Some((3, 2))
    );
    assert_eq!(
        crate::review_close::reject_turn_label(3, 2),
        "Rechazar 3 cambios en 2 archivos"
    );
    assert_eq!(read(&a), FIVE_EDITED, "todavía nada");
    cx.update(|window, cx| review.update(cx, |review, cx| review.cancel_reject_turn(window, cx)));
    settle(cx);
    assert_eq!(
        review.read_with(cx, |review, _| review.reject_turn_prompt()),
        None
    );
    assert_eq!(pending(&workspace, cx), (3, 2));

    bar_action(&workspace, &a, ReviewAction::RejectTurn, cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(
        review.read_with(cx, |review, _| review.reject_turn_prompt()),
        None,
        "Esc cancela"
    );

    bar_action(&workspace, &a, ReviewAction::RejectTurn, cx);
    cx.update(|window, cx| review.update(cx, |review, cx| review.confirm_reject_turn(window, cx)));
    settle(cx);
    assert_eq!(read(&a), FIVE);
    assert_eq!(read(&b), FIVE);
    assert_eq!(pending(&workspace, cx), (0, 0));

    cx.simulate_keystrokes("alt-shift-u");
    settle(cx);
    assert_eq!(read(&a), FIVE_EDITED, "Alt+Shift+U lo recupera");
    assert_eq!(read(&b), "a\nb\nX\nd\ne\n");
    assert_eq!(pending(&workspace, cx), (3, 2));
}
