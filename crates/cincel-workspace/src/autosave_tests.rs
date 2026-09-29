//! "Tras N milisegundos sin escribir" end to end
//! (`docs/specs/07-etapa5-productividad.md` §10.5): typing reschedules a
//! per-tab delay, a fresh edit before it elapses postpones it, a turn
//! active on that file blocks the save and is retried, and closing the tab
//! or turning the setting off drops the pending save instead of writing it.
//!
//! Behind `test-support`, like every GPUI test of the crate:
//!
//! ```text
//! cargo test -p cincel-workspace --features test-support
//! ```

use std::path::Path;
use std::time::Duration;

use cincel_settings::{Autosave, Config};
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::center::CenterPanel;
use crate::project::ProjectOptions;
use crate::review::ReviewSummary;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

/// Isolated state and `files.autosave = "after_delay"` with `delay_ms`.
fn init_with_autosave_delay(delay_ms: u64, cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| {
        let mut config = Config::default();
        config.settings.files.autosave = Autosave::AfterDelay;
        config.settings.files.autosave_delay_ms = delay_ms;
        crate::init(config, cx);
    });
}

/// One file, `f.txt`, with `body`.
fn sample_project(body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f.txt"), body).unwrap();
    dir
}

fn workspace_window<'a>(
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

/// Opens `relative` in a pinned, focused tab (typing reaches its editor).
fn open_pinned(
    workspace: &Entity<Workspace>,
    relative: &str,
    cx: &mut VisualTestContext,
) -> Entity<CenterPanel> {
    let files = workspace.read_with(cx, |workspace, _| workspace.files().clone());
    files.update(cx, |files, cx| {
        files.open_for_test(Path::new(relative), true, cx)
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| workspace.center().clone())
}

fn is_dirty(center: &Entity<CenterPanel>, cx: &mut VisualTestContext) -> bool {
    center.read_with(cx, |center, cx| center.is_tab_dirty(0, cx))
}

#[gpui::test]
fn a_pause_after_typing_saves_the_file(cx: &mut TestAppContext) {
    init_with_autosave_delay(1000, cx);
    let dir = sample_project("uno\n");
    let path = dir.path().join("f.txt");
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "f.txt", cx);

    cx.simulate_input("dos\n");
    cx.run_until_parked();
    assert!(is_dirty(&center, cx), "recién escrito: sucio");

    cx.executor().advance_clock(Duration::from_millis(999));
    cx.run_until_parked();
    assert!(
        is_dirty(&center, cx),
        "a los 999 ms todavía no se autoguardó"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "uno\n");

    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert!(!is_dirty(&center, cx), "al milisegundo 1000 se autoguarda");
    assert_ne!(std::fs::read_to_string(&path).unwrap(), "uno\n");
}

#[gpui::test]
fn a_new_edit_before_the_delay_postpones_it(cx: &mut TestAppContext) {
    init_with_autosave_delay(1000, cx);
    let dir = sample_project("uno\n");
    let path = dir.path().join("f.txt");
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "f.txt", cx);

    cx.simulate_input("a");
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(999));
    cx.run_until_parked();
    assert!(is_dirty(&center, cx));

    // A second edit at t≈999ms reprograms the delay for another 1000ms from
    // here (t≈1999ms), not from the first edit (t=1000ms): advancing only
    // to t≈1998ms must still find it dirty, which a bug that failed to
    // reset the timer would not.
    cx.simulate_input("b");
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(999));
    cx.run_until_parked();
    assert!(
        is_dirty(&center, cx),
        "el segundo tecleo reinició la cuenta: todavía no pasó 1000 ms desde él"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "uno\n");

    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert!(
        !is_dirty(&center, cx),
        "ahora sí pasó 1000 ms desde el último tecleo"
    );
}

#[gpui::test]
fn a_file_with_an_active_turn_is_not_autosaved_but_retried_after(cx: &mut TestAppContext) {
    init_with_autosave_delay(1000, cx);
    let dir = sample_project("uno\n");
    let path = dir.path().join("f.txt");
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "f.txt", cx);

    center.update(cx, |center, cx| {
        center.set_review_summary(
            std::rc::Rc::new(std::cell::RefCell::new(ReviewSummary {
                turn_active: true,
                tracked: std::collections::HashSet::from([path.clone()]),
                ..ReviewSummary::default()
            })),
            cx,
        );
    });

    cx.simulate_input("dos\n");
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(1000));
    cx.run_until_parked();
    assert!(
        is_dirty(&center, cx),
        "con un turno activo sobre el archivo no se autoguarda"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "uno\n");

    // The turn ends: the rule is the same as `on_focus_change`'s ("se
    // reintenta al terminar el turno"), so the next retry saves it.
    center.update(cx, |center, cx| {
        center.set_review_summary(
            std::rc::Rc::new(std::cell::RefCell::new(ReviewSummary::default())),
            cx,
        );
    });
    cx.executor().advance_clock(Duration::from_millis(1000));
    cx.run_until_parked();
    assert!(
        !is_dirty(&center, cx),
        "terminado el turno, el reintento guarda"
    );
}

#[gpui::test]
fn closing_the_tab_cancels_the_pending_autosave(cx: &mut TestAppContext) {
    init_with_autosave_delay(1000, cx);
    let dir = sample_project("uno\n");
    let path = dir.path().join("f.txt");
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "f.txt", cx);

    cx.simulate_input("dos\n");
    cx.run_until_parked();

    // `Ctrl+W` asks first (the tab is dirty); "Descartar" actually closes
    // it, dropping its pending autosave `Task` along with the rest of the
    // `Tab`.
    cx.simulate_keystrokes("ctrl-w");
    cx.run_until_parked();
    assert!(center.read_with(cx, |center, _| center.is_asking_about_unsaved_changes()));
    cx.update(|window, cx| {
        center.update(cx, |center, cx| {
            center.discard_and_close_for_test(window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(center.read_with(cx, |center, _| center.tabs().len()), 0);

    // The tab (and its pending autosave `Task`) is gone: advancing the
    // clock past the delay must not write anything.
    cx.executor().advance_clock(Duration::from_millis(1000));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "uno\n");
}

#[gpui::test]
fn turning_autosave_off_before_the_delay_elapses_does_not_save(cx: &mut TestAppContext) {
    init_with_autosave_delay(1000, cx);
    let dir = sample_project("uno\n");
    let path = dir.path().join("f.txt");
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "f.txt", cx);

    cx.simulate_input("dos\n");
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert!(is_dirty(&center, cx));

    // Turning the setting off mid-flight: the pending timer, once it would
    // have fired, checks the setting again and does nothing
    // (`docs/specs/07-etapa5-productividad.md` §10.5, "cambiar el ajuste
    // cancela el temporizador").
    cx.update(|_window, cx| crate::settings::install(Config::default(), cx));
    cx.executor().advance_clock(Duration::from_millis(1000));
    cx.run_until_parked();
    assert!(
        is_dirty(&center, cx),
        "el ajuste ya no es «after_delay»: no se autoguarda"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "uno\n");
}

#[gpui::test]
fn on_focus_change_is_unaffected_by_the_after_delay_machinery(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| {
        let mut config = Config::default();
        config.settings.files.autosave = Autosave::OnFocusChange;
        crate::init(config, cx);
    });
    let dir = sample_project("uno\n");
    let path = dir.path().join("f.txt");
    let (workspace, cx) = workspace_window(dir.path(), cx);
    let center = open_pinned(&workspace, "f.txt", cx);

    cx.simulate_input("dos\n");
    cx.run_until_parked();
    // No delay is configured for `on_focus_change`: a long wait alone must
    // never save (only a focus change does, tested elsewhere in `tests.rs`).
    cx.executor().advance_clock(Duration::from_millis(5000));
    cx.run_until_parked();
    assert!(is_dirty(&center, cx));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "uno\n");
}
