//! GPUI tests of the cleanup on save (`docs/specs/10-etapa7-ronda2.md` §7.8):
//! `files.trim_trailing_whitespace_on_save` and
//! `files.ensure_final_newline_on_save`, applied by `CenterPanel::save_path`
//! as one editor transaction right before the file is written. Every check
//! reads the file back from disk.
//!
//! Behind `test-support`, like every GPUI test of the crate:
//!
//! ```text
//! cargo test -p cincel-workspace --features test-support
//! ```

use std::path::Path;
use std::time::Duration;

use cincel_acp::AgentEvent;
use cincel_acp::acp::schema::v1::{
    SessionId, SessionUpdate, StopReason, ToolCall, ToolCallLocation, ToolCallStatus, ToolKind,
};
use cincel_editor::EditorView;
use cincel_settings::{Autosave, Config};
use cincel_text::Point;
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::agents::Agents;
use crate::center::CenterPanel;
use crate::project::ProjectOptions;
use crate::review::Review;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

fn init_with(cx: &mut TestAppContext, configure: impl FnOnce(&mut Config)) {
    isolate_state();
    let mut config = Config::default();
    configure(&mut config);
    cx.update(|cx| crate::init(config, cx));
}

/// The defaults: both cleanups on.
fn init(cx: &mut TestAppContext) {
    init_with(cx, |_| {});
}

struct Parts {
    center: Entity<CenterPanel>,
    review: Entity<Review>,
    agents: Entity<Agents>,
}

/// A project with `files` (relative name, bytes), in a temporary folder.
fn project(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in files {
        std::fs::write(dir.path().join(name), bytes).unwrap();
    }
    dir
}

fn window<'a>(root: &Path, cx: &'a mut TestAppContext) -> (Parts, &'a mut VisualTestContext) {
    let options = WorkspaceOptions {
        project: Some(root.to_path_buf()),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    let parts = workspace.read_with(cx, |workspace, _| Parts {
        center: workspace.center().clone(),
        review: workspace.review().clone(),
        agents: workspace.agents().clone(),
    });
    (parts, cx)
}

/// Lets the debounced review recompute and the background jobs land.
fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(120));
    cx.run_until_parked();
}

/// Opens `path` in a pinned, focused tab.
fn open(parts: &Parts, path: &Path, cx: &mut VisualTestContext) -> Entity<EditorView> {
    cx.update(|window, cx| {
        parts
            .center
            .update(cx, |center, cx| center.open_file(path, true, window, cx));
    });
    settle(cx);
    parts.center.read_with(cx, |center, _| {
        center
            .tabs()
            .iter()
            .find(|tab| tab.path == path)
            .map(|tab| tab.editor().clone())
            .expect("la pestaña está abierta")
    })
}

/// Replaces the buffer text the way an edit would (one user transaction), so
/// the tab is dirty with exactly `text`.
fn set_buffer(editor: &Entity<EditorView>, text: &str, cx: &mut VisualTestContext) {
    editor.update(cx, |editor, cx| editor.set_text(text, 0, cx));
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |editor, _| editor.text()), text);
    assert!(editor.read_with(cx, |editor, _| editor.is_dirty()));
}

fn text(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, _| editor.text())
}

fn is_dirty(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |editor, _| editor.is_dirty())
}

/// `Ctrl+S` on the focused editor: the manual save.
fn save(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("ctrl-s");
    cx.run_until_parked();
}

fn disk(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
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

/// What sending a prompt does to the review: the turn starts.
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

/// The agent announces an edit of `path`, which puts it in the turn.
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

/// A whole turn where the agent writes `content` into `path`.
fn agent_writes(parts: &Parts, path: &Path, content: &str, cx: &mut VisualTestContext) {
    start_turn(parts, cx);
    let (reply, mut answer) = tokio::sync::oneshot::channel();
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
    assert!(matches!(answer.try_recv(), Ok(Ok(()))));
    end_turn(parts, cx);
}

// ------------------------------------------------------------------ tests

/// Criterion 1: blanks gone, final break added, and one `Ctrl+Z` brings the
/// text back (the tab dirty).
#[gpui::test]
fn saving_cleans_the_text_and_one_undo_brings_it_back(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("f.txt", b"x\n")]);
    let path = dir.path().join("f.txt");
    let (parts, cx) = window(dir.path(), cx);
    let editor = open(&parts, &path, cx);

    set_buffer(&editor, "a  \nb\t\nc", cx);
    save(cx);

    assert_eq!(disk(&path), b"a\nb\nc\n");
    assert_eq!(text(&editor, cx), "a\nb\nc\n");
    assert!(!is_dirty(&editor, cx), "guardado: limpio");

    cx.simulate_keystrokes("ctrl-z");
    cx.run_until_parked();
    assert_eq!(
        text(&editor, cx),
        "a  \nb\t\nc",
        "un Ctrl+Z deshace la limpieza entera"
    );
    assert!(is_dirty(&editor, cx), "y la pestaña queda sucia");
    assert_eq!(disk(&path), b"a\nb\nc\n", "el disco no cambia por deshacer");
}

/// Criterion 2: with both settings off the disk gets the buffer verbatim.
#[gpui::test]
fn with_both_settings_off_the_disk_gets_the_buffer_as_is(cx: &mut TestAppContext) {
    init_with(cx, |config| {
        config.settings.files.trim_trailing_whitespace_on_save = false;
        config.settings.files.ensure_final_newline_on_save = false;
    });
    let dir = project(&[("f.txt", b"x\n")]);
    let path = dir.path().join("f.txt");
    let (parts, cx) = window(dir.path(), cx);
    let editor = open(&parts, &path, cx);

    set_buffer(&editor, "a  \nb\t\nc", cx);
    save(cx);

    assert_eq!(disk(&path), b"a  \nb\t\nc");
    assert_eq!(text(&editor, cx), "a  \nb\t\nc");
    assert!(!is_dirty(&editor, cx));
}

/// The two settings are independent of each other.
#[gpui::test]
fn each_setting_works_on_its_own(cx: &mut TestAppContext) {
    init_with(cx, |config| {
        config.settings.files.ensure_final_newline_on_save = false;
    });
    let dir = project(&[("f.txt", b"x\n")]);
    let path = dir.path().join("f.txt");
    let (parts, cx) = window(dir.path(), cx);
    let editor = open(&parts, &path, cx);
    set_buffer(&editor, "a  \nb", cx);
    save(cx);
    assert_eq!(disk(&path), b"a\nb", "solo se recortó");

    cx.update(|_, cx| {
        let mut config = Config::default();
        config.settings.files.trim_trailing_whitespace_on_save = false;
        crate::settings::install(config, cx);
    });
    set_buffer(&editor, "c  \nd", cx);
    save(cx);
    assert_eq!(disk(&path), b"c  \nd\n", "solo se agregó el salto");
}

/// Criterion 3: the rows of a pending agent segment keep their trailing
/// blanks; the other rows are cleaned.
#[gpui::test]
fn pending_agent_rows_keep_their_trailing_blanks(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("f.txt", b"a  \nb\nc  \nd\ne\n")]);
    let path = dir.path().join("f.txt");
    let (parts, cx) = window(dir.path(), cx);
    let editor = open(&parts, &path, cx);

    // The agent rewrites rows 1 and 4, with blanks at their ends.
    agent_writes(&parts, &path, "a  \nB  \nc  \nd\nE  \n", cx);
    let rows: Vec<_> = editor.read_with(cx, |editor, _| {
        editor
            .review()
            .hunks
            .iter()
            .map(|hunk| hunk.buffer_rows.clone())
            .collect()
    });
    assert_eq!(rows, [1..2, 4..5], "dos segmentos pendientes");
    assert_eq!(text(&editor, cx), "a  \nB  \nc  \nd\nE  \n");

    // The user edits an unrelated row, so the tab is dirty.
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(3, 1), cx));
    cx.simulate_input("   ");
    settle(cx);
    assert!(is_dirty(&editor, cx));
    let rows: Vec<_> = editor.read_with(cx, |editor, _| {
        editor
            .review()
            .hunks
            .iter()
            .map(|hunk| hunk.buffer_rows.clone())
            .collect()
    });
    assert_eq!(rows, [1..2, 4..5], "los segmentos siguen pendientes");

    save(cx);
    assert_eq!(
        disk(&path),
        b"a\nB  \nc\nd\nE  \n",
        "las filas del agente conservan los espacios; las demás se limpian"
    );
    // The segments are still there to be decided.
    let rows: Vec<_> = editor.read_with(cx, |editor, _| {
        editor
            .review()
            .hunks
            .iter()
            .map(|hunk| hunk.buffer_rows.clone())
            .collect()
    });
    assert_eq!(rows, [1..2, 4..5]);
}

/// Criterion 4: Markdown keeps its two trailing spaces but still gets the
/// final break.
#[gpui::test]
fn markdown_keeps_trailing_blanks_but_gets_the_final_newline(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("notas.md", b"x\n"), ("otras.markdown", b"x\n")]);
    let notes = dir.path().join("notas.md");
    let (parts, cx) = window(dir.path(), cx);
    let editor = open(&parts, &notes, cx);

    set_buffer(&editor, "hola  \n", cx);
    save(cx);
    assert_eq!(disk(&notes), b"hola  \n", "los dos espacios se conservan");

    set_buffer(&editor, "hola", cx);
    save(cx);
    assert_eq!(disk(&notes), b"hola\n", "el salto final sí se agrega");

    set_buffer(&editor, "uno  \ndos \t", cx);
    save(cx);
    assert_eq!(disk(&notes), b"uno  \ndos \t\n");

    // `.markdown` is Markdown too.
    let other = dir.path().join("otras.markdown");
    let editor = open(&parts, &other, cx);
    set_buffer(&editor, "hola  ", cx);
    save(cx);
    assert_eq!(disk(&other), b"hola  \n");
}

/// Criterion 5: a CRLF file stays CRLF, the final break included.
#[gpui::test]
fn a_crlf_file_stays_crlf(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("f.txt", b"uno\r\ndos\r\n")]);
    let path = dir.path().join("f.txt");
    let (parts, cx) = window(dir.path(), cx);
    let editor = open(&parts, &path, cx);

    set_buffer(&editor, "uno  \ndos\t\ntres", cx);
    save(cx);

    assert_eq!(disk(&path), b"uno\r\ndos\r\ntres\r\n");
}

/// Criterion 6: with a turn running on the file, a manual save writes the
/// buffer as it is. A turn on another file does not stop the cleanup.
#[gpui::test]
fn a_running_turn_on_the_file_saves_the_buffer_untouched(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("f.txt", b"a\nb\n"), ("g.txt", b"a\nb\n")]);
    let path = dir.path().join("f.txt");
    let other = dir.path().join("g.txt");
    let (parts, cx) = window(dir.path(), cx);
    let _first = open(&parts, &path, cx);
    let other_editor = open(&parts, &other, cx);

    start_turn(&parts, cx);
    agent(&parts, edit_call("c1", &path), cx);
    let active = parts
        .review
        .read_with(cx, |review, _| review.summary().borrow().turn_active);
    assert!(active, "el turno está en curso");

    // `g.txt` is not part of the turn: it is cleaned.
    set_buffer(&other_editor, "a  \nb", cx);
    save(cx);
    assert_eq!(disk(&other), b"a\nb\n");

    // `f.txt` is: the buffer goes out as typed.
    // Back to `f.txt`'s tab (opening it again just activates it).
    let editor = open(&parts, &path, cx);
    set_buffer(&editor, "a  \nb", cx);
    save(cx);
    assert_eq!(disk(&path), b"a  \nb");
    assert_eq!(text(&editor, cx), "a  \nb", "el buffer no se tocó");

    // When the turn ends, the next save cleans.
    end_turn(&parts, cx);
    set_buffer(&editor, "c  \nd", cx);
    save(cx);
    assert_eq!(disk(&path), b"c\nd\n");
}

/// Criterion 7: an empty file stays empty.
#[gpui::test]
fn an_empty_file_stays_empty(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("f.txt", b"x\n")]);
    let path = dir.path().join("f.txt");
    let (parts, cx) = window(dir.path(), cx);
    let editor = open(&parts, &path, cx);

    set_buffer(&editor, "", cx);
    save(cx);
    assert_eq!(disk(&path), b"");
    assert_eq!(text(&editor, cx), "");

    // Only blanks and no break: trimmed away, nothing to terminate.
    set_buffer(&editor, "  \t", cx);
    save(cx);
    assert_eq!(disk(&path), b"");
}

/// Nothing to clean: no transaction, so the history is the user's own.
#[gpui::test]
fn nothing_to_clean_leaves_the_history_alone(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("f.txt", b"x\n")]);
    let path = dir.path().join("f.txt");
    let (parts, cx) = window(dir.path(), cx);
    let editor = open(&parts, &path, cx);

    set_buffer(&editor, "a\nb\n\n\n", cx);
    let version = editor.read_with(cx, |editor, _| editor.text_version());
    save(cx);
    assert_eq!(
        disk(&path),
        b"a\nb\n\n\n",
        "las líneas en blanco finales quedan"
    );
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.text_version()),
        version,
        "ni una edición"
    );

    // The one `Ctrl+Z` undoes the user's own edit, not a cleanup.
    cx.simulate_keystrokes("ctrl-z");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "x\n");
}

/// The cursor and the selection follow their text through the cleanup.
#[gpui::test]
fn the_cursor_follows_its_text(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("f.txt", b"x\n")]);
    let path = dir.path().join("f.txt");
    let (parts, cx) = window(dir.path(), cx);
    let editor = open(&parts, &path, cx);

    set_buffer(&editor, "a  \nb\t\nc", cx);
    // End of the last row: the added break must not drag the cursor to a new
    // row.
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(2, 1), cx));
    save(cx);
    assert_eq!(text(&editor, cx), "a\nb\nc\n");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.cursor_point()),
        Point::new(2, 1)
    );

    // A cursor in the middle of a trimmed row moves with the row's text.
    set_buffer(&editor, "ab  \ncd  \n", cx);
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(1, 1), cx));
    save(cx);
    assert_eq!(disk(&path), b"ab\ncd\n");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.cursor_point()),
        Point::new(1, 1)
    );
}

/// Criterion 8: both autosave modes apply the same cleanup.
#[gpui::test]
fn autosave_on_focus_change_cleans_up(cx: &mut TestAppContext) {
    init_with(cx, |config| {
        config.settings.files.autosave = Autosave::OnFocusChange;
    });
    let dir = project(&[("f.txt", b"x\n")]);
    let path = dir.path().join("f.txt");
    let (parts, cx) = window(dir.path(), cx);
    let editor = open(&parts, &path, cx);

    set_buffer(&editor, "a  \nb\t\nc", cx);
    cx.update(|window, cx| {
        parts
            .center
            .update(cx, |center, cx| center.autosave(window, cx))
    });
    cx.run_until_parked();

    assert_eq!(disk(&path), b"a\nb\nc\n");
    assert!(!is_dirty(&editor, cx));
}

#[gpui::test]
fn autosave_after_a_pause_cleans_up(cx: &mut TestAppContext) {
    init_with(cx, |config| {
        config.settings.files.autosave = Autosave::AfterDelay;
        config.settings.files.autosave_delay_ms = 1000;
    });
    let dir = project(&[("f.txt", b"x\n")]);
    let path = dir.path().join("f.txt");
    let (parts, cx) = window(dir.path(), cx);
    let editor = open(&parts, &path, cx);

    // Typing arms the timer (a programmatic edit does not).
    editor.update(cx, |editor, cx| editor.set_cursor(Point::new(0, 1), cx));
    cx.simulate_input("  ");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "x  \n");
    cx.executor().advance_clock(Duration::from_millis(1000));
    cx.run_until_parked();

    assert_eq!(disk(&path), b"x\n");
    assert!(!is_dirty(&editor, cx));
}

/// "Guardar todo" and "Guardar todo y salir" go through the same path.
#[gpui::test]
fn save_all_and_save_all_for_quit_clean_up(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("f.txt", b"x\n"), ("g.txt", b"x\n")]);
    let first = dir.path().join("f.txt");
    let second = dir.path().join("g.txt");
    let (parts, cx) = window(dir.path(), cx);
    let first_editor = open(&parts, &first, cx);
    let second_editor = open(&parts, &second, cx);

    set_buffer(&first_editor, "a  \nb", cx);
    set_buffer(&second_editor, "c\t\nd", cx);
    cx.update(|window, cx| {
        parts
            .center
            .update(cx, |center, cx| center.save_all(window, cx))
    });
    cx.run_until_parked();
    assert_eq!(disk(&first), b"a\nb\n");
    assert_eq!(disk(&second), b"c\nd\n");

    set_buffer(&first_editor, "e  ", cx);
    let saved = cx.update(|window, cx| {
        parts
            .center
            .update(cx, |center, cx| center.save_all_for_quit(window, cx))
    });
    assert!(saved);
    assert_eq!(disk(&first), b"e\n");
}

/// The two switches of Configuración → Archivos, after "Pausa del
/// autoguardado": they show their values and write them to `settings.json`.
#[gpui::test]
fn the_files_section_has_the_two_switches(cx: &mut TestAppContext) {
    isolate_state();
    let dir = tempfile::tempdir().unwrap();
    let paths = cincel_settings::Paths::under(dir.path().join("config"));
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    let config = Config::load_from(&paths).value;
    cx.update(|cx| crate::init(config, cx));
    let options = WorkspaceOptions {
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-,");
    cx.run_until_parked();
    let view = workspace
        .read_with(cx, |workspace, _| workspace.center().clone())
        .read_with(cx, |center, _| {
            center.settings_view().cloned().expect("configuración")
        });

    cx.update(|window, cx| view.update(cx, |view, cx| view.set_query("al guardar", window, cx)));
    assert_eq!(
        view.read_with(cx, |view, cx| view.visible_keys(cx)),
        [
            "files.trim_trailing_whitespace_on_save",
            "files.ensure_final_newline_on_save"
        ]
    );

    view.update(cx, |view, cx| {
        assert!(view.set_value(
            "files.trim_trailing_whitespace_on_save",
            serde_json::json!(false),
            cx
        ));
        assert!(view.set_value(
            "files.ensure_final_newline_on_save",
            serde_json::json!(false),
            cx
        ));
    });
    cx.run_until_parked();
    let files = cx.update(|_, cx| crate::settings::settings(cx).files);
    assert!(!files.trim_trailing_whitespace_on_save);
    assert!(!files.ensure_final_newline_on_save);
    let written = std::fs::read_to_string(&paths.settings).unwrap();
    assert!(
        written.contains("\"trim_trailing_whitespace_on_save\": false")
            && written.contains("\"ensure_final_newline_on_save\": false"),
        "{written}"
    );
}
