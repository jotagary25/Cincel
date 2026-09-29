//! Tests of the quick file finder (`crate::file_finder`,
//! `docs/specs/07-etapa5-productividad.md` §3.5).
//!
//! Kept out of `tests.rs` (E5-F only touches new files while other
//! sub-stages work on `cincel-workspace` in parallel): the pure half below
//! needs no `App` at all, and the `TestAppContext` half follows the same
//! `workspace_window` / `press` pattern the rest of the crate's test modules
//! use, rebuilt locally since `tests.rs`'s own helpers are private to that
//! module.

use std::path::Path;
use std::sync::Arc;

use cincel_settings::Config;
use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};

use crate::file_finder::{FileFinder, rank};
use crate::project::ProjectOptions;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

fn arc(text: &str) -> Arc<str> {
    Arc::from(text)
}

fn arcs(paths: &[&str]) -> Vec<Arc<str>> {
    paths.iter().map(|path| arc(path)).collect()
}

fn order_of(matches: &[crate::file_finder::FinderMatch]) -> Vec<&str> {
    matches
        .iter()
        .map(|found| found.relative.as_ref())
        .collect()
}

// ------------------------------------------------------------- pure tests

#[test]
fn scores_descending_and_highlights_in_ascending_order() {
    // §3.4: "la consulta `mrs` pone `src/main.rs` primero y resalta `m`,
    // `r`, `s`".
    let candidates = arcs(&["src/main.rs", "src/lib.rs", "docs/leeme.md"]);
    let results = rank(&candidates, "mrs", &[], 200);
    assert_eq!(results[0].relative.as_ref(), "src/main.rs", "{results:?}");
    assert!(
        results[0]
            .positions
            .windows(2)
            .all(|pair| pair[0] < pair[1]),
        "{:?}",
        results[0].positions
    );
}

#[test]
fn ties_break_by_shorter_path_then_alphabetically() {
    // All three end in the same "x.rs"; matching only that ties the score,
    // so length then alphabetical order decides (§3.1).
    let candidates = arcs(&["b/x.rs", "a/x.rs", "aa/x.rs"]);
    let results = rank(&candidates, "xrs", &[], 200);
    assert_eq!(
        order_of(&results),
        vec!["a/x.rs", "b/x.rs", "aa/x.rs"],
        "{results:?}"
    );
}

#[test]
fn the_limit_is_never_exceeded_by_either_path() {
    let candidates: Vec<Arc<str>> = (0..500).map(|i| arc(&format!("file{i}.rs"))).collect();
    assert_eq!(rank(&candidates, "file", &[], 200).len(), 200);
    assert_eq!(rank(&candidates, "", &[], 200).len(), 200);
}

#[test]
fn an_empty_query_lists_recent_first_then_the_tree_order() {
    let candidates = arcs(&["a.rs", "b.rs", "c.rs", "d.rs"]);
    let recent = arcs(&["c.rs", "a.rs"]);
    let results = rank(&candidates, "", &recent, 200);
    assert_eq!(
        order_of(&results),
        vec!["c.rs", "a.rs", "b.rs", "d.rs"],
        "{results:?}"
    );
}

#[test]
fn an_empty_query_never_repeats_a_path_and_ignores_stale_recents() {
    let candidates = arcs(&["a.rs", "b.rs"]);
    // A duplicate and a path that is not a candidate any more (a closed,
    // deleted file) are both handled without duplicating or crashing.
    let recent = arcs(&["a.rs", "a.rs", "gone.rs"]);
    let results = rank(&candidates, "   ", &recent, 200);
    assert_eq!(order_of(&results), vec!["a.rs", "b.rs"], "{results:?}");
}

#[test]
fn positions_land_on_char_boundaries_with_accents() {
    // D1: "probar con una ruta con «ñ» y «á»"; the needle itself carries an
    // accent so frizbee takes its unicode path.
    let candidates = arcs(&["docs/año.txt", "docs/leeme.md"]);
    let results = rank(&candidates, "año", &[], 200);
    let hit = results
        .iter()
        .find(|found| found.relative.as_ref() == "docs/año.txt")
        .expect("«año» coincide con docs/año.txt");
    assert!(!hit.positions.is_empty());
    for &position in &hit.positions {
        assert!(
            hit.relative.is_char_boundary(position),
            "{position} no es un límite de char en {:?}",
            hit.relative
        );
        // Slicing here must never panic: that is the whole point of
        // snapping every offset to a char boundary.
        let _ = &hit.relative[position..];
    }
    // One offset per matched character: no duplicates from a multi-byte
    // character's inner bytes.
    let mut deduped = hit.positions.clone();
    deduped.dedup();
    assert_eq!(deduped, hit.positions, "{:?}", hit.positions);
}

#[test]
fn an_uppercase_query_matches_the_same_case_and_respects_a_different_one() {
    // D1 pins `Config::default()`, whose case handling is "Smart": case is
    // ignored unless the query itself has an uppercase letter, in which
    // case it is *respected* rather than just preferred. A query that
    // matches the file's case still finds it…
    let candidates = arcs(&["src/Main.rs"]);
    let same_case = rank(&candidates, "Main", &[], 200);
    assert_eq!(order_of(&same_case), vec!["src/Main.rs"], "{same_case:?}");
    // …one whose case does not (every letter upper, against "ain" lower)
    // does not, under that same default configuration.
    let different_case = rank(&candidates, "MAIN", &[], 200);
    assert!(different_case.is_empty(), "{different_case:?}");
}

#[test]
fn a_query_with_no_match_returns_nothing() {
    let candidates = arcs(&["src/main.rs"]);
    assert!(rank(&candidates, "zzz-no-existe", &[], 200).is_empty());
}

// ------------------------------------------------------------- GPUI tests

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(Config::default(), cx));
}

/// `a.rs`, `b.rs`, `c.rs` at the root, plus a `.gitignore`d `target/` file
/// that must never reach the finder (§3.4).
fn sample_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.path().join("b.rs"), "fn b() {}\n").unwrap();
    std::fs::write(dir.path().join("c.rs"), "fn c() {}\n").unwrap();
    std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir(dir.path().join("target")).unwrap();
    std::fs::write(dir.path().join("target/ignoreme.rs"), "// nunca\n").unwrap();
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

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
}

fn finder(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<FileFinder> {
    workspace
        .read_with(cx, |workspace, _| workspace.file_finder().cloned())
        .expect("el proyecto ya construyó el buscador (Workspace::open_project)")
}

/// §3.4: "`Ctrl+P` con proyecto abre el buscador con el foco en el campo".
#[gpui::test]
fn ctrl_p_opens_with_the_focus_in_the_query_field(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    press("ctrl-p", cx);

    let finder = finder(&workspace, cx);
    assert!(finder.read_with(cx, |finder, _| finder.is_open()));
    cx.simulate_input("rs");
    // Typing only reaches the query field if it had the keyboard.
    assert!(
        finder.read_with(cx, |finder, cx| finder.query().read(cx).value() == "rs"),
        "el campo no recibió el tecleo"
    );

    // `Ctrl+P` again closes it (§3.1).
    press("ctrl-p", cx);
    assert!(!finder.read_with(cx, |finder, _| finder.is_open()));
}

/// §3.4: "sin proyecto, toast y nada más".
#[gpui::test]
fn ctrl_p_without_a_project_only_toasts(cx: &mut TestAppContext) {
    init_test(cx);
    let options = WorkspaceOptions {
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();

    assert!(workspace.read_with(cx, |workspace, _| workspace.file_finder().is_none()));
    let toasts = workspace.read_with(cx, |workspace, _| workspace.toasts().clone());

    press("ctrl-p", cx);

    assert!(workspace.read_with(cx, |workspace, _| workspace.file_finder().is_none()));
    assert_eq!(toasts.read_with(cx, |toasts, _| toasts.items().len()), 1);
}

/// §3.4: "`↓ ↓ Enter` abre el tercer resultado en una pestaña en cursiva
/// (previsualización) y el foco queda en su editor".
#[gpui::test]
fn down_down_enter_opens_the_third_result_as_a_preview(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    press("ctrl-p", cx);
    // All three end in ".rs" and tie on score; the tie-break (shorter path,
    // then alphabetical) puts them in `a.rs`, `b.rs`, `c.rs` order (this is
    // the same rule `rank`'s own unit tests cover above).
    cx.simulate_input("rs");
    press("down", cx);
    press("down", cx);
    press("enter", cx);

    let finder = finder(&workspace, cx);
    assert!(!finder.read_with(cx, |finder, _| finder.is_open()));

    let center = workspace.read_with(cx, |workspace, _| workspace.center().clone());
    let tab = center.read_with(cx, |center, _| {
        center
            .active_tab()
            .expect("una pestaña quedó activa")
            .path
            .clone()
    });
    assert_eq!(tab, dir.path().join("c.rs"));
    let is_preview = center.read_with(cx, |center, _| center.active_tab().unwrap().preview);
    assert!(is_preview, "Enter tiene que abrir en previsualización");

    let editor_focused = cx.update(|window, cx| {
        center
            .read(cx)
            .active_tab()
            .unwrap()
            .editor()
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
    });
    assert!(editor_focused, "el foco tiene que quedar en el editor");
}

/// §3.4: "Un archivo ignorado por `.gitignore`... no aparece nunca".
#[gpui::test]
fn a_gitignored_file_never_appears(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    press("ctrl-p", cx);
    cx.simulate_input("ignoreme");

    let finder = finder(&workspace, cx);
    assert!(
        finder.read_with(cx, |finder, _| finder.results().is_empty()),
        "target/ignoreme.rs está en .gitignore: no puede aparecer"
    );
}

/// §3.4: "Consulta sin coincidencias: se ve «Ningún archivo coincide con
/// «…»»" — checked here through the state the empty message reads
/// (`results()`/`candidates` are not empty, only the match is).
#[gpui::test]
fn a_query_with_no_matches_leaves_the_result_list_empty(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    press("ctrl-p", cx);
    cx.simulate_input("zzz-no-existe");

    let finder = finder(&workspace, cx);
    assert!(finder.read_with(cx, |finder, _| finder.results().is_empty()));
}

/// §3.4: "`Esc` cierra y devuelve el foco al elemento que lo tenía (el
/// chat, si se abrió desde el chat)".
#[gpui::test]
fn escape_restores_the_previous_focus(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    // Focus the chat's composer first, then open the finder from there.
    press("ctrl-shift-a", cx);
    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    let composer_focused_before = cx.update(|window, cx| {
        chat.read(cx)
            .composer()
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(composer_focused_before, "el compositor tenía el foco");

    press("ctrl-p", cx);
    let finder = finder(&workspace, cx);
    assert!(finder.read_with(cx, |finder, _| finder.is_open()));

    press("escape", cx);

    assert!(!finder.read_with(cx, |finder, _| finder.is_open()));
    let composer_focused_after = cx.update(|window, cx| {
        chat.read(cx)
            .composer()
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(
        composer_focused_after,
        "Esc tiene que devolver el foco al chat"
    );
}

/// §9.3 (E5-A dejó pendiente este caso, D8): con el buscador de archivos
/// abierto, ni `Ctrl+L` ni `Ctrl+Shift+A` hacen nada (§7.4/§9.1, "modal
/// abierto").
#[gpui::test]
fn ctrl_l_and_ctrl_shift_a_do_nothing_with_the_file_finder_open(cx: &mut TestAppContext) {
    init_test(cx);
    let dir = sample_project();
    let (workspace, cx) = workspace_window(dir.path(), cx);

    press("ctrl-p", cx);
    let finder = finder(&workspace, cx);
    assert!(finder.read_with(cx, |finder, _| finder.is_open()));
    assert!(workspace.read_with(cx, |workspace, cx| workspace.is_modal_open(cx)));

    let chat_dock_open_before = workspace.read_with(cx, |workspace, cx| {
        workspace.is_dock_open(gpui_kit::component::dock::DockPlacement::Left, cx)
    });

    press("ctrl-l", cx);
    assert!(
        finder.read_with(cx, |finder, _| finder.is_open()),
        "Ctrl+L no tiene que mover el foco con el buscador abierto"
    );

    press("ctrl-shift-a", cx);
    assert!(
        finder.read_with(cx, |finder, _| finder.is_open()),
        "Ctrl+Shift+A no tiene que hacer nada con el buscador abierto"
    );
    let chat_dock_open_after = workspace.read_with(cx, |workspace, cx| {
        workspace.is_dock_open(gpui_kit::component::dock::DockPlacement::Left, cx)
    });
    assert_eq!(chat_dock_open_before, chat_dock_open_after);
}
