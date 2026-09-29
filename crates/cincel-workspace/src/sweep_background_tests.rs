//! GPUI tests of the end-of-turn sweep off the main thread
//! (`docs/specs/08-etapa6-cierre-1-0.md` §5.3, D12; `review_snapshot.rs`).
//!
//! `set_photo_in_background(true)` makes an inert test project take its
//! photo and run its sweep on the background executor, as the application
//! does. GPUI's test executor is deterministic: nothing in the background
//! runs until the test parks, so "during the sweep" is simply "before the
//! next `run_until_parked`".
//!
//! ```text
//! cargo build -p cincel-acp --bin cincel-acp-fake-agent -p cincel-connections --bins
//! cargo test -p cincel-workspace --features test-support sweep_background -- --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cincel_acp::acp::schema::v1::{SessionId, StopReason};
use cincel_acp::{AgentCommand, AgentEvent, PromptBlock};
use cincel_chat::ChatPanel;
use cincel_editor::EditorView;
use cincel_project::FsEvent;
use cincel_settings::Config;
use cincel_text::Point;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::agents::Agents;
use crate::center::CenterPanel;
use crate::project::ProjectOptions;
use crate::review::{FileState, Review, SWEEP_NOTICE};
use crate::test_support::{FakeEnv, isolate_state};
use crate::workspace::{Workspace, WorkspaceOptions};

const FIVE: &str = "a\nb\nc\nd\ne\n";
const FIVE_EDITED: &str = "a\nB\nc\nd\nE\n";

/// The spec's budget for one stretch of main-thread work (§5.3: "bloqueo
/// máximo del hilo principal ≤ 8 ms"), times `CINCEL_PERF_BUDGET_FACTOR`
/// (D15).
fn main_thread_budget() -> Duration {
    let factor = std::env::var("CINCEL_PERF_BUDGET_FACTOR")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    Duration::from_millis(8) * factor
}

fn init(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), FIVE).unwrap();
    std::fs::write(dir.path().join("b.txt"), FIVE).unwrap();
    std::fs::write(dir.path().join("c.txt"), FIVE).unwrap();
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
    review.update(cx, |review, _| review.set_photo_in_background(true));
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

/// A hand-driven prompt: its photo lands in the background (settled here).
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

/// `TurnEnded` without parking: the sweep has only been launched.
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

fn sweeping(parts: &Parts, cx: &mut VisualTestContext) -> bool {
    parts.review.read_with(cx, |review, _| {
        review.is_sweeping() && review.is_turn_active()
    })
}

fn turn_active(parts: &Parts, cx: &mut VisualTestContext) -> bool {
    parts
        .review
        .read_with(cx, |review, _| review.is_turn_active())
}

fn hunks(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> usize {
    parts
        .review
        .read_with(cx, |review, _| review.store().hunks(path).len())
}

fn tracked(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> bool {
    parts
        .review
        .read_with(cx, |review, _| review.store().file(path).is_some())
}

fn turn_of(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> Option<u64> {
    parts.review.read_with(cx, |review, _| {
        review.store().file(path).map(|file| file.turn_id.0)
    })
}

fn project_of(parts: &Parts, cx: &mut VisualTestContext) -> Entity<crate::project::Project> {
    parts
        .workspace
        .read_with(cx, |workspace, _| workspace.project().cloned())
        .expect("hay proyecto")
}

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
    drain_until(parts, cx, true, |event| {
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

/// Hands the fake agent's events over until `stop`; the last one is not
/// followed by a park unless `settle_last`.
fn drain_until(
    parts: &Parts,
    cx: &mut VisualTestContext,
    settle_last: bool,
    stop: impl Fn(&AgentEvent) -> bool,
) {
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
        if done {
            if settle_last {
                settle(cx);
            }
            break;
        }
        settle(cx);
    }
}

fn is_turn_end(event: &AgentEvent) -> bool {
    matches!(event, AgentEvent::TurnEnded { .. })
}

// ------------------------------------------------------------------ tests

/// The sweep runs on the background executor: the turn stays active (its
/// decisions disabled) until the results are adopted, then closes with
/// everything the watcher missed.
#[gpui::test]
fn the_sweep_runs_in_the_background_and_the_turn_stays_active(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let nuevo = dir.path().join("nuevo.txt");
    let editor = open(&parts, &a, cx);

    start_turn(&parts, cx);
    std::fs::write(&a, FIVE_EDITED).unwrap();
    std::fs::write(&nuevo, "hola\n").unwrap();
    std::fs::remove_file(dir.path().join("c.txt")).unwrap();
    turn_ended_now(&parts, cx);

    assert!(
        sweeping(&parts, cx),
        "el turno sigue activo durante el repaso"
    );
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.summary().borrow().turn_active)
    );
    assert!(!tracked(&parts, &a, cx), "todavía no llegó el resultado");

    settle(cx);
    assert!(!turn_active(&parts, cx));
    assert_eq!(hunks(&parts, &a, cx), 2);
    let summary = parts.review.read_with(cx, |review, _| {
        let summary = review.summary();
        let summary = summary.borrow();
        summary
            .files
            .iter()
            .map(|(path, file)| (path.clone(), file.state))
            .collect::<BTreeMap<PathBuf, FileState>>()
    });
    assert_eq!(summary.get(&a), Some(&FileState::Modified));
    assert_eq!(summary.get(&nuevo), Some(&FileState::Created));
    assert_eq!(
        summary.get(&dir.path().join("c.txt")),
        Some(&FileState::Deleted)
    );
    let view = editor.read_with(cx, |editor, _| editor.review().clone());
    assert!(!view.turn_active, "los botones se habilitan al terminar");
    assert_eq!(view.hunks.len(), 2);
    let stats = parts
        .review
        .read_with(cx, |review, _| review.last_sweep())
        .expect("hubo repaso");
    assert!(stats.in_background);
    assert_eq!(stats.changes, 3);
    assert_eq!(stats.adopted, 3);
    assert!(
        parts
            .review
            .read_with(cx, |review, _| review.photo().is_none()),
        "la foto se libera al terminar"
    );
}

/// A `Ctrl+S` of the user during the sweep is the user's, not the agent's.
#[gpui::test]
fn a_user_save_during_the_sweep_is_not_the_agents(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");

    start_turn(&parts, cx);
    std::fs::write(&a, FIVE_EDITED).unwrap();
    let editor = open(&parts, &b, cx);
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(0, 0), cx));
    cx.run_until_parked();
    cx.simulate_input("mío ");
    turn_ended_now(&parts, cx);
    assert!(sweeping(&parts, cx));

    // The keystroke is handled before the background comparison runs.
    cx.simulate_keystrokes("ctrl-s");
    settle(cx);
    assert_eq!(
        std::fs::read_to_string(&b).unwrap(),
        "mío a\nb\nc\nd\ne\n",
        "se guardó"
    );
    assert!(!turn_active(&parts, cx));
    assert!(
        !tracked(&parts, &b, cx),
        "lo que guardó el usuario no es del agente"
    );
    assert_eq!(hunks(&parts, &a, cx), 2);
}

/// A batch of the watcher during the sweep is adopted against the photo,
/// right away, and the sweep does not count it twice.
#[gpui::test]
fn a_watcher_change_during_the_sweep_is_adopted(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");

    start_turn(&parts, cx);
    std::fs::write(&a, FIVE_EDITED).unwrap();
    turn_ended_now(&parts, cx);
    assert!(sweeping(&parts, cx));

    // Another program writes b.txt while the sweep runs.
    std::fs::write(&b, "a\nb\nX\nd\ne\n").unwrap();
    let project = project_of(&parts, cx);
    project.update(cx, |project, cx| {
        project.apply_fs_events(&[FsEvent::Modified(b.clone())], cx)
    });
    assert!(tracked(&parts, &b, cx), "adoptado durante el repaso");
    assert!(sweeping(&parts, cx));

    settle(cx);
    assert!(!turn_active(&parts, cx));
    assert_eq!(hunks(&parts, &b, cx), 1);
    assert_eq!(hunks(&parts, &a, cx), 2);
    let turn = parts.review.read_with(cx, |review, _| review.latest_turn());
    assert_eq!(turn_of(&parts, &b, cx), turn);
}

/// The agent going away during the sweep does not cut it short.
#[gpui::test]
fn agent_gone_during_the_sweep_still_finishes_it(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");

    start_turn(&parts, cx);
    std::fs::write(&a, FIVE_EDITED).unwrap();
    turn_ended_now(&parts, cx);
    parts.review.update(cx, |review, cx| review.agent_gone(cx));
    assert!(sweeping(&parts, cx), "el repaso sigue");

    settle(cx);
    assert!(!turn_active(&parts, cx));
    assert_eq!(hunks(&parts, &a, cx), 2);
}

/// A prompt sent during the sweep goes out only once it is over: the next
/// turn starts after it, with its own photo taken after the sweep.
#[gpui::test]
fn a_prompt_sent_during_the_sweep_waits_for_it(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let edits = format!("a.txt={}", FIVE_EDITED.replace('\n', "\\n"));
    let (_env, session) = connect_shell_agent(&parts, &edits, cx);

    prompt(&parts, &session, cx);
    settle(cx);
    let first = parts
        .review
        .read_with(cx, |review, _| review.latest_turn())
        .expect("turno");
    drain_until(&parts, cx, false, is_turn_end);
    assert!(sweeping(&parts, cx));

    prompt(&parts, &session, cx);
    assert!(sweeping(&parts, cx), "el prompt espera el repaso");
    assert_eq!(
        parts.review.read_with(cx, |review, _| review.latest_turn()),
        Some(first),
        "el turno nuevo todavía no empezó"
    );
    assert!(!tracked(&parts, &a, cx));

    settle(cx);
    let second = parts
        .review
        .read_with(cx, |review, _| review.latest_turn())
        .expect("turno");
    assert!(second > first, "empezó el turno encadenado");
    assert_eq!(
        turn_of(&parts, &a, cx),
        Some(first),
        "el cambio es del primero"
    );
    let copy = parts.review.read_with(cx, |review, _| {
        review
            .photo()
            .and_then(|photo| photo.entry(&a))
            .and_then(|entry| entry.text().map(|text| text.to_string()))
    });
    assert_eq!(
        copy.as_deref(),
        Some(FIVE_EDITED),
        "la foto del turno nuevo se sacó después del repaso"
    );
    // The prompt went out: the agent runs its turn and ends it.
    drain_until(&parts, cx, true, is_turn_end);
    assert!(!turn_active(&parts, cx));
    assert_eq!(hunks(&parts, &a, cx), 2);
    assert_eq!(
        parts.review.read_with(cx, |review, _| review
            .store()
            .file(&a)
            .map(|file| file.base.to_string())),
        Some(FIVE.to_string()),
        "no se re-basa"
    );
}

/// Stopping a prompt that waits for the sweep drops it: the sweep closes
/// its turn and no new turn starts.
#[gpui::test]
fn cancelling_a_prompt_waiting_for_the_sweep_drops_it(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (parts, cx) = window(dir.path(), cx);
    let a = dir.path().join("a.txt");
    let edits = format!("a.txt={}", FIVE_EDITED.replace('\n', "\\n"));
    let (_env, session) = connect_shell_agent(&parts, &edits, cx);

    prompt(&parts, &session, cx);
    settle(cx);
    let first = parts.review.read_with(cx, |review, _| review.latest_turn());
    drain_until(&parts, cx, false, is_turn_end);
    prompt(&parts, &session, cx);
    assert!(sweeping(&parts, cx));
    parts.agents.update(cx, |agents, cx| {
        agents.forward_command(
            &AgentCommand::Cancel {
                session_id: session.clone(),
            },
            cx,
        )
    });
    settle(cx);

    assert!(!turn_active(&parts, cx), "no empezó otro turno");
    assert_eq!(
        parts.review.read_with(cx, |review, _| review.latest_turn()),
        first
    );
    assert_eq!(hunks(&parts, &a, cx), 2, "el repaso terminó igual");
    assert_eq!(
        parts.chat.read_with(cx, |chat, _| chat.status()),
        cincel_chat::AgentStatus::Ready
    );
}

/// 5 000 files and 120 changes: the comparison runs in the background, no
/// stretch of main-thread work goes over the spec's 8 ms, and the result is
/// exactly the one of the inline sweep.
#[gpui::test]
fn the_sweep_of_five_thousand_files_never_blocks_the_main_thread(cx: &mut TestAppContext) {
    init(cx);
    let background = big_project();
    let inline = big_project();

    let (parts, vcx) = window(background.path(), cx);
    start_turn(&parts, vcx);
    agent_edits(background.path());
    let launch = Instant::now();
    parts.review.update(vcx, |review, cx| review.end_turn(cx));
    let launch = launch.elapsed();
    assert!(sweeping(&parts, vcx));
    let clock = Instant::now();
    settle(vcx);
    let wall = clock.elapsed();
    assert!(!turn_active(&parts, vcx));
    let stats = parts
        .review
        .read_with(vcx, |review, _| review.last_sweep())
        .expect("hubo repaso");
    let found = pending_files(&parts, background.path(), vcx);
    eprintln!(
        "repaso en fondo de {} archivos: comparación {:?}, bloqueo máximo del hilo principal {:?} \
         en {} tramos (lanzarlo: {launch:?}), total {:?} (reloj {wall:?}), {} cambios",
        stats.files,
        stats.compare,
        stats.max_main_slice,
        stats.main_slices,
        stats.total,
        stats.changes
    );
    assert!(stats.in_background);
    assert_eq!(stats.files, 5_000);
    assert_eq!(stats.changes, 120);
    assert_eq!(stats.adopted, 120);
    let budget = main_thread_budget();
    assert!(
        stats.max_main_slice <= budget,
        "bloqueó el hilo principal {:?} (máximo {budget:?})",
        stats.max_main_slice
    );
    assert!(launch <= budget, "lanzar el repaso tardó {launch:?}");

    // The same turn swept inline (the previous behavior) finds the same.
    let (inline_parts, icx) = window(inline.path(), cx);
    inline_parts
        .review
        .update(icx, |review, _| review.set_photo_in_background(false));
    start_turn(&inline_parts, icx);
    agent_edits(inline.path());
    inline_parts
        .review
        .update(icx, |review, cx| review.end_turn(cx));
    assert!(
        !turn_active(&inline_parts, icx),
        "en línea termina enseguida"
    );
    settle(icx);
    let expected = pending_files(&inline_parts, inline.path(), icx);
    assert_eq!(expected.len(), 120);
    assert_eq!(found, expected);
}

/// 5 000 files of about 1 KB in 50 folders.
fn big_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let body = "let valor = 42; // relleno para la foto\n".repeat(25);
    for index in 0..5_000 {
        let folder = dir.path().join(format!("m{:02}", index % 50));
        if index < 50 {
            std::fs::create_dir_all(&folder).unwrap();
        }
        std::fs::write(folder.join(format!("f{index}.rs")), &body).unwrap();
    }
    dir
}

/// 100 files changed, 10 created, 10 deleted, straight on disk.
fn agent_edits(root: &Path) {
    for index in 0..100 {
        let path = root.join(format!("m{:02}/f{index}.rs", index % 50));
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("// cambio del agente\n");
        std::fs::write(&path, text).unwrap();
    }
    for index in 0..10 {
        std::fs::write(root.join(format!("m{index:02}/nuevo{index}.rs")), "x\n").unwrap();
    }
    for index in 4_990..5_000 {
        std::fs::remove_file(root.join(format!("m{:02}/f{index}.rs", index % 50))).unwrap();
    }
}

/// Every pending file, relative, with its state and line counts.
fn pending_files(
    parts: &Parts,
    root: &Path,
    cx: &mut VisualTestContext,
) -> BTreeMap<String, (FileState, u32, u32)> {
    parts.review.read_with(cx, |review, _| {
        review
            .summary()
            .borrow()
            .files
            .iter()
            .map(|(path, file)| {
                (
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    (file.state, file.added, file.removed),
                )
            })
            .collect()
    })
}

/// The status bar notice text is the spec's.
#[test]
fn the_sweep_notice_is_the_specs() {
    assert_eq!(SWEEP_NOTICE, "Revisando los cambios del agente…");
}
