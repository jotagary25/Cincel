//! GPUI tests of the settings tab (`docs/specs/07-etapa5-productividad.md`
//! §4.6, §4.7) and of the focus rules with it (§7.6, §9.1).
//!
//! Each test gets its own configuration directory
//! (`cincel_settings::Paths::under`), so the writes of one never reach
//! another; the XDG state goes through [`isolate_state`] like every other
//! module's tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cincel_acp::AgentRegistry;
use cincel_connections::{
    Adapters, AgentKind, CancelToken, Connections, NodePaths, NodeVersion, PackageInstaller,
    Runtime,
};
use cincel_settings::{Config, Paths, SettingsEdit, SettingsEvent};
use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use serde_json::json;

use crate::center::{CenterItem, CenterPanel};
use crate::connection_modal::{DeleteStep, Mode, Step};
use crate::focus::FocusZone;
use crate::project::ProjectOptions;
use crate::settings_view::{
    CatalogState, EMPTY_NAME_ERROR, OFFLINE_CATALOG, SettingsSection, SettingsView,
    SettingsViewEvent, UpdateOperation, UpdateOutcome, UpdateTarget,
};
use crate::test_support::{FakeEnv, isolate_state};
use crate::workspace::{Workspace, WorkspaceOptions};

/// A `settings.json` a user wrote by hand, comments and all.
const COMMENTED: &str = "{\n  // Mi configuración: no tocar los comentarios.\n  \"ui_font_size\": 13,\n\n  /* El editor */\n  \"editor\": {\n    // cuatro espacios\n    \"tab_size\": 4,\n  },\n}\n";

/// Boots the workspace over a private configuration directory holding
/// `settings` (when given). Keep the directory alive for the whole test.
fn init_test(settings: Option<&str>, cx: &mut TestAppContext) -> (tempfile::TempDir, Paths) {
    isolate_state();
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path().join("config"));
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    if let Some(text) = settings {
        std::fs::write(&paths.settings, text).unwrap();
    }
    let config = Config::load_from(&paths).value;
    cx.update(|cx| crate::init(config, cx));
    (dir, paths)
}

/// A small project: `src/main.rs` and `src/lib.rs`.
fn sample_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn suma() {}\n").unwrap();
    dir
}

/// A window with the workspace, on `root` (or on nothing).
fn workspace_window<'a>(
    root: Option<&Path>,
    cx: &'a mut TestAppContext,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let options = WorkspaceOptions {
        project: root.map(Path::to_path_buf),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    (workspace, cx)
}

fn center(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<CenterPanel> {
    workspace.read_with(cx, |workspace, _| workspace.center().clone())
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
}

/// Opens `relative` in a pinned tab, as a double click in the tree does.
fn open_pinned(
    workspace: &Entity<Workspace>,
    root: &Path,
    relative: &str,
    cx: &mut VisualTestContext,
) {
    let path = root.join(relative);
    let center = center(workspace, cx);
    cx.update(|window, cx| {
        center.update(cx, |center, cx| center.open_file(&path, true, window, cx))
    });
    cx.run_until_parked();
}

/// The settings tab (panics when it is not open).
fn view(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<SettingsView> {
    center(workspace, cx).read_with(cx, |center, _| {
        center
            .settings_view()
            .cloned()
            .expect("la pestaña de configuración está abierta")
    })
}

fn settings_tabs(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> usize {
    center(workspace, cx).read_with(cx, |center, _| {
        center
            .items()
            .iter()
            .filter(|item| matches!(item, CenterItem::Settings(_)))
            .count()
    })
}

fn toast_messages(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    let toasts = workspace.read_with(cx, |workspace, _| workspace.toasts().clone());
    toasts.read_with(cx, |toasts, _| {
        toasts
            .items()
            .iter()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

/// What the watcher would do after a change on disk.
fn watcher_event(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    workspace.update(cx, |workspace, cx| {
        workspace.on_settings_event(SettingsEvent::SettingsChanged, cx)
    });
    cx.run_until_parked();
}

fn settings_view_has_focus(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    let view = view(workspace, cx);
    cx.update(|window, cx| view.read(cx).focus_handle(cx).is_focused(window))
}

// ---------------------------------------------------------------- the tab

/// §4.6: `Ctrl+,` opens "Configuración"; a second `Ctrl+,` does not open
/// another; `Ctrl+W` closes it.
#[gpui::test]
fn ctrl_comma_opens_one_settings_tab_and_ctrl_w_closes_it(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let project = sample_project();
    let (workspace, cx) = workspace_window(Some(project.path()), cx);

    press("ctrl-,", cx);
    assert_eq!(settings_tabs(&workspace, cx), 1);
    assert!(center(&workspace, cx).read_with(cx, |center, _| center.is_settings_active()));
    assert!(settings_view_has_focus(&workspace, cx));

    press("ctrl-,", cx);
    assert_eq!(settings_tabs(&workspace, cx), 1, "no se abre otra");

    press("ctrl-w", cx);
    assert_eq!(settings_tabs(&workspace, cx), 0);
    assert!(center(&workspace, cx).read_with(cx, |center, _| center.settings_view().is_none()));
}

/// §4.1: pinned to the right of the active tab; activating a file tab and
/// coming back works; `close_active` with it active closes it and the
/// neighbour takes over.
#[gpui::test]
fn the_settings_tab_opens_next_to_the_active_one_and_closes_like_a_tab(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let project = sample_project();
    let (workspace, cx) = workspace_window(Some(project.path()), cx);
    open_pinned(&workspace, project.path(), "src/main.rs", cx);
    open_pinned(&workspace, project.path(), "src/lib.rs", cx);
    let center = center(&workspace, cx);
    cx.update(|window, cx| center.update(cx, |center, cx| center.activate(0, window, cx)));

    press("ctrl-,", cx);
    let order = center.read_with(cx, |center, _| {
        center
            .items()
            .iter()
            .map(|item| match item {
                CenterItem::File(tab) => tab.title.to_string(),
                CenterItem::Settings(_) => "Configuración".to_string(),
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(order, ["main.rs", "Configuración", "lib.rs"]);
    assert_eq!(
        center.read_with(cx, |center, _| center
            .active_tab()
            .map(|tab| tab.title.to_string())),
        None,
        "con la configuración activa no hay pestaña de archivo activa"
    );

    cx.update(|window, cx| center.update(cx, |center, cx| center.activate(1, window, cx)));
    assert!(!center.read_with(cx, |center, _| center.is_settings_active()));
    cx.update(|window, cx| center.update(cx, |center, cx| center.activate_settings(window, cx)));
    assert!(center.read_with(cx, |center, _| center.is_settings_active()));

    cx.update(|window, cx| center.update(cx, |center, cx| center.close_active(window, cx)));
    cx.run_until_parked();
    assert_eq!(settings_tabs(&workspace, cx), 0);
    assert_eq!(
        center.read_with(cx, |center, _| center
            .active_tab()
            .map(|tab| tab.title.to_string())),
        Some("lib.rs".to_string()),
        "la vecina de la derecha queda activa"
    );
    assert_eq!(center.read_with(cx, |center, _| center.tabs().len()), 2);
}

/// §4.1: without a project the tab takes the place of the empty screen,
/// and closing it brings the empty screen back.
#[gpui::test]
fn the_settings_tab_works_without_a_project(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let (workspace, cx) = workspace_window(None, cx);
    press("ctrl-,", cx);
    assert_eq!(settings_tabs(&workspace, cx), 1);
    assert!(
        cx.debug_bounds("settings-view").is_some(),
        "la pestaña se pinta en lugar de la pantalla vacía"
    );
    assert!(settings_view_has_focus(&workspace, cx));

    press("ctrl-w", cx);
    assert_eq!(settings_tabs(&workspace, cx), 0);
    assert!(cx.debug_bounds("settings-view").is_none());
    // The global shortcuts still work: the root view has the keyboard.
    press("ctrl-,", cx);
    assert_eq!(settings_tabs(&workspace, cx), 1);
}

/// §4.6 / D4: the settings tab never reaches `layout.json`.
#[gpui::test]
fn the_settings_tab_is_left_out_of_the_layout(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let project = sample_project();
    let (workspace, cx) = workspace_window(Some(project.path()), cx);
    open_pinned(&workspace, project.path(), "src/main.rs", cx);
    press("ctrl-,", cx);

    let (tabs, active) = center(&workspace, cx).read_with(cx, |center, _| center.to_layout());
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].path, PathBuf::from("src/main.rs"));
    assert_eq!(active, Some(0));

    workspace.read_with(cx, |workspace, cx| workspace.save_layout(cx));
    let saved = crate::layout::WorkspaceLayout::load(project.path()).expect("layout.json");
    assert_eq!(saved.tabs.len(), 1);
    assert!(
        saved
            .tabs
            .iter()
            .all(|tab| tab.path == Path::new("src/main.rs"))
    );
}

// --------------------------------------------------------------- writing

/// §4.6: turning off "Ajustar líneas largas" changes the open editor at
/// once and writes `"soft_wrap": false` inside `"editor"`, leaving the rest
/// of the file byte for byte; and no "Configuración recargada" toast, not
/// even when the watcher's own event arrives afterwards.
#[gpui::test]
fn turning_off_soft_wrap_writes_the_file_and_changes_the_editor(cx: &mut TestAppContext) {
    let (_config, paths) = init_test(Some(COMMENTED), cx);
    let project = sample_project();
    let (workspace, cx) = workspace_window(Some(project.path()), cx);
    open_pinned(&workspace, project.path(), "src/main.rs", cx);
    let editor = center(&workspace, cx).read_with(cx, |center, _| {
        center.active_tab().expect("hay pestaña").editor().clone()
    });
    assert!(editor.read_with(cx, |editor, _| editor.settings().soft_wrap));

    press("ctrl-,", cx);
    let view = view(&workspace, cx);
    view.update(cx, |view, cx| {
        assert!(view.set_value("editor.soft_wrap", json!(false), cx))
    });
    cx.run_until_parked();

    let written = std::fs::read_to_string(&paths.settings).unwrap();
    let expected = cincel_settings::apply_edit(
        COMMENTED,
        &SettingsEdit::Set {
            path: &["editor", "soft_wrap"],
            value: json!(false),
        },
    )
    .unwrap();
    assert_eq!(written, expected, "solo cambia el valor tocado");
    assert!(written.contains("// Mi configuración: no tocar los comentarios."));
    assert!(written.contains("/* El editor */"));
    assert!(written.contains("// cuatro espacios"));
    let editor_section = &written[written.find("\"editor\"").unwrap()..];
    assert!(editor_section.contains("\"soft_wrap\": false"), "{written}");

    assert!(
        !editor.read_with(cx, |editor, _| editor.settings().soft_wrap),
        "el editor abierto cambia en el acto"
    );
    assert!(view.read_with(cx, |view, cx| view.is_modified("editor.soft_wrap", cx)));

    watcher_event(&workspace, cx);
    assert!(
        !toast_messages(&workspace, cx)
            .iter()
            .any(|message| message == "Configuración recargada"),
        "{:?}",
        toast_messages(&workspace, cx)
    );
}

/// §4.6: "Restablecer" removes the key and the row goes back to the default.
#[gpui::test]
fn reset_removes_the_key_and_the_row_shows_the_default(cx: &mut TestAppContext) {
    let (_config, paths) = init_test(
        Some("{\n  // mío\n  \"editor\": { \"tab_size\": 2 }\n}\n"),
        cx,
    );
    let (workspace, cx) = workspace_window(None, cx);
    press("ctrl-,", cx);
    let view = view(&workspace, cx);
    assert_eq!(
        view.read_with(cx, |view, cx| view.number_text("editor.tab_size", cx)),
        Some("2".to_string())
    );
    assert!(view.read_with(cx, |view, cx| view.is_modified("editor.tab_size", cx)));

    view.update(cx, |view, cx| assert!(view.reset("editor.tab_size", cx)));
    cx.run_until_parked();

    let written = std::fs::read_to_string(&paths.settings).unwrap();
    assert!(!written.contains("tab_size"), "{written}");
    assert!(written.contains("// mío"));
    assert_eq!(
        view.read_with(cx, |view, cx| view.number_text("editor.tab_size", cx)),
        Some("4".to_string())
    );
    assert!(!view.read_with(cx, |view, cx| view.is_modified("editor.tab_size", cx)));
}

/// Numbers: typed with a comma, clamped to the range, written as JSON
/// numbers; a select writes its value.
#[gpui::test]
fn numbers_and_selects_write_what_the_spec_says(cx: &mut TestAppContext) {
    let (_config, paths) = init_test(None, cx);
    let (workspace, cx) = workspace_window(None, cx);
    press("ctrl-,", cx);
    let view = view(&workspace, cx);

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.type_number("buffer_line_height", "1,6", window, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.type_number("ui_font_size", "200", window, cx);
        })
    });
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        view.set_value("files.autosave", json!("after_delay"), cx);
    });
    cx.run_until_parked();

    let settings = cx.update(|_, cx| crate::settings::settings(cx));
    assert_eq!(settings.buffer_line_height, 1.6);
    assert_eq!(settings.ui_font_size, 72., "se limita al máximo");
    assert_eq!(
        settings.files.autosave,
        cincel_settings::Autosave::AfterDelay
    );
    let written = std::fs::read_to_string(&paths.settings).unwrap();
    assert!(written.contains("\"buffer_line_height\": 1.6"), "{written}");
    assert!(written.contains("\"ui_font_size\": 72"), "{written}");
    assert_eq!(
        view.read_with(cx, |view, cx| view.number_text("buffer_line_height", cx)),
        Some("1,6".to_string())
    );
    assert_eq!(
        view.read_with(cx, |view, cx| view.selected_title("files.autosave", cx)),
        Some("Tras una pausa".to_string())
    );

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.add_pattern_to("files.exclude", "**/dist", window, cx);
        })
    });
    cx.run_until_parked();
    let settings = cx.update(|_, cx| crate::settings::settings(cx));
    assert!(settings.files.exclude.contains(&"**/dist".to_string()));
    assert!(settings.files.exclude.contains(&"**/.git".to_string()));
}

/// §4.6: editing `settings.json` by hand updates the field of the open tab
/// (through the watcher, which does say "Configuración recargada").
#[gpui::test]
fn a_hand_edit_updates_the_open_tab(cx: &mut TestAppContext) {
    let (_config, paths) = init_test(Some("{ \"ui_font_size\": 13 }"), cx);
    let (workspace, cx) = workspace_window(None, cx);
    press("ctrl-,", cx);
    let view = view(&workspace, cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.set_section(SettingsSection::Fonts, window, cx)
        })
    });
    assert_eq!(
        view.read_with(cx, |view, cx| view.number_text("ui_font_size", cx)),
        Some("13".to_string())
    );

    std::fs::write(&paths.settings, "{\n  \"ui_font_size\": 16\n}\n").unwrap();
    watcher_event(&workspace, cx);

    assert_eq!(
        view.read_with(cx, |view, cx| view.number_text("ui_font_size", cx)),
        Some("16".to_string())
    );
    assert!(
        toast_messages(&workspace, cx)
            .iter()
            .any(|message| message == "Configuración recargada")
    );
}

/// §4.6: with a syntax error in the file the tab shows the banner and
/// writes nothing.
#[gpui::test]
fn a_syntax_error_shows_the_banner_and_nothing_is_written(cx: &mut TestAppContext) {
    let (_config, paths) = init_test(None, cx);
    let (workspace, cx) = workspace_window(None, cx);
    press("ctrl-,", cx);
    let view = view(&workspace, cx);
    assert_eq!(view.read_with(cx, |view, _| view.banner()), None);

    let broken = "{\n  \"editor\": { \"soft_wrap\": false,, }\n}\n";
    std::fs::write(&paths.settings, broken).unwrap();
    watcher_event(&workspace, cx);

    let banner = view
        .read_with(cx, |view, _| view.banner())
        .expect("hay un aviso");
    assert!(
        banner.starts_with("settings.json tiene un error en la línea 2, columna"),
        "{banner}"
    );
    assert!(banner.ends_with("Corregilo para poder cambiar ajustes desde acá."));
    assert!(view.read_with(cx, |view, _| view.is_read_only()));

    view.update(cx, |view, cx| {
        view.set_value("editor.tab_size", json!(8), cx);
        view.reset("editor.soft_wrap", cx);
    });
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&paths.settings).unwrap(), broken);

    // Fixed by hand: the banner goes away and the controls work again.
    std::fs::write(&paths.settings, "{ \"editor\": { \"soft_wrap\": false } }").unwrap();
    watcher_event(&workspace, cx);
    assert_eq!(view.read_with(cx, |view, _| view.banner()), None);
    assert!(!view.read_with(cx, |view, _| view.is_read_only()));
}

/// §4.1: the search filters by title, description and key, ignoring case
/// and accents, and says so when nothing matches.
#[gpui::test]
fn the_search_filters_rows_and_sections(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let (workspace, cx) = workspace_window(None, cx);
    press("ctrl-,", cx);
    let view = view(&workspace, cx);

    cx.update(|window, cx| view.update(cx, |view, cx| view.set_query("LINEAS largas", window, cx)));
    assert_eq!(
        view.read_with(cx, |view, cx| view.visible_keys(cx)),
        ["editor.soft_wrap"]
    );
    assert_eq!(
        view.read_with(cx, |view, cx| view.visible_sections(cx)),
        [SettingsSection::Editor]
    );

    cx.update(|window, cx| view.update(cx, |view, cx| view.set_query("autosave", window, cx)));
    let keys = view.read_with(cx, |view, cx| view.visible_keys(cx));
    assert_eq!(keys, ["files.autosave", "files.autosave_delay_ms"]);

    cx.update(|window, cx| view.update(cx, |view, cx| view.set_query("zzzz", window, cx)));
    assert!(
        view.read_with(cx, |view, cx| view.visible_sections(cx))
            .is_empty()
    );
}

/// §5.1 (`docs/specs/08-etapa6-cierre-1-0.md`): the snapshot memory cap row
/// is found by "memoria" and by "foto", writes `review.snapshot_max_total_mb`
/// keeping the user's comments, and "Restablecer" takes it out again.
#[gpui::test]
fn snapshot_memory_cap_row_is_searchable_writable_and_resettable(cx: &mut TestAppContext) {
    let (_config, paths) = init_test(Some("{\n  // mío\n  \"ui_font_size\": 13\n}\n"), cx);
    let (workspace, cx) = workspace_window(None, cx);
    press("ctrl-,", cx);
    let view = view(&workspace, cx);

    cx.update(|window, cx| view.update(cx, |view, cx| view.set_query("memoria", window, cx)));
    assert_eq!(
        view.read_with(cx, |view, cx| view.visible_keys(cx)),
        ["review.snapshot_max_total_mb"]
    );
    cx.update(|window, cx| view.update(cx, |view, cx| view.set_query("foto", window, cx)));
    assert_eq!(
        view.read_with(cx, |view, cx| view.visible_keys(cx)),
        ["review.snapshot_max_total_mb"]
    );

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.type_number("review.snapshot_max_total_mb", "512", window, cx);
        })
    });
    cx.run_until_parked();

    let settings = cx.update(|_, cx| crate::settings::settings(cx));
    assert_eq!(settings.review.snapshot_max_total_mb, 512);
    let written = std::fs::read_to_string(&paths.settings).unwrap();
    assert!(written.contains("// mío"), "{written}");
    let review_section = &written[written.find("\"review\"").unwrap()..];
    assert!(
        review_section.contains("\"snapshot_max_total_mb\": 512"),
        "{written}"
    );
    assert!(view.read_with(cx, |view, cx| {
        view.is_modified("review.snapshot_max_total_mb", cx)
    }));

    view.update(cx, |view, cx| {
        assert!(view.reset("review.snapshot_max_total_mb", cx))
    });
    cx.run_until_parked();
    let written = std::fs::read_to_string(&paths.settings).unwrap();
    assert!(!written.contains("snapshot_max_total_mb"), "{written}");
    let settings = cx.update(|_, cx| crate::settings::settings(cx));
    assert_eq!(settings.review.snapshot_max_total_mb, 300);
}

/// §4.5 (D2): nothing gpui-kit would say in English is on screen.
#[gpui::test]
fn no_english_text_is_visible(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let env = FakeEnv::new();
    env.add_connection("claude-acp", "Claude · personal");
    let (workspace, cx) = workspace_window(None, cx);
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    agents.update(cx, |agents, cx| {
        agents.set_connections(offline_engine(&env, None), cx)
    });
    press("ctrl-,", cx);
    let view = view(&workspace, cx);
    for section in SettingsSection::ALL {
        cx.update(|window, cx| view.update(cx, |view, cx| view.set_section(section, window, cx)));
        cx.run_until_parked();
        let texts = view.read_with(cx, |view, cx| view.visible_texts(cx));
        assert!(!texts.is_empty());
        for text in texts {
            for english in ["Search", "Reset", "No results", "Select"] {
                assert!(!text.contains(english), "«{text}» contiene «{english}»");
            }
        }
    }
}

// ----------------------------------------------------------- connections

/// The engine over the fake environment with a pinned Node (so no check
/// ever asks nodejs.org) and, when given, a fake npm.
fn offline_engine(env: &FakeEnv, installer: Option<Box<dyn PackageInstaller>>) -> Arc<Connections> {
    let runtime = Runtime::new(env.paths.clone()).with_version(NodeVersion::parse("24.1.0"));
    let mut adapters = Adapters::new(env.paths.clone());
    if let Some(installer) = installer {
        adapters = adapters.with_installer(installer);
    }
    Arc::new(
        Connections::new(env.paths.clone())
            .with_runtime(runtime)
            .with_adapters(adapters),
    )
}

/// How `Agents` gets the registry (`settings_bridge::RegistryLoader`).
type Loader = Arc<dyn Fn(bool) -> Result<AgentRegistry, String> + Send + Sync>;

/// A registry that publishes Claude's adapter at `version`.
fn registry_with_claude(version: &'static str) -> Loader {
    Arc::new(move |_| {
        AgentRegistry::parse(&format!(
            r#"{{"version":"1.0.0","agents":[{{"id":"claude-acp","name":"Claude","version":"{version}",
               "distribution":{{"npx":{{"package":"@agentclientprotocol/claude-agent-acp@{version}"}}}}}}]}}"#
        ))
        .map_err(|error| error.to_string())
    })
}

/// `npm install` as a test double: lays out the package with its `bin`.
struct FakeNpm;

impl PackageInstaller for FakeNpm {
    fn install(
        &self,
        _node: &NodePaths,
        _package_spec: &str,
        prefix: &Path,
        _cache: &Path,
        _npmrc: &Path,
        _cancel: &CancelToken,
    ) -> cincel_connections::Result<()> {
        let package = prefix.join("node_modules/@agentclientprotocol/claude-agent-acp");
        std::fs::create_dir_all(package.join("dist")).unwrap();
        std::fs::write(package.join("package.json"), r#"{"bin":"dist/index.js"}"#).unwrap();
        std::fs::write(package.join("dist/index.js"), "// adaptador\n").unwrap();
        Ok(())
    }
}

/// The workspace with the fake engine behind `Agents`, and the settings
/// tab open on Connections (the menu's "Conexiones", D7).
fn connections_window<'a>(
    env: &FakeEnv,
    engine: Arc<Connections>,
    loader: Option<Loader>,
    cx: &'a mut TestAppContext,
) -> (
    Entity<Workspace>,
    Entity<SettingsView>,
    &'a mut VisualTestContext,
) {
    let _ = env;
    let (workspace, cx) = workspace_window(None, cx);
    let agents = workspace.read_with(cx, |workspace, _| workspace.agents().clone());
    agents.update(cx, |agents, cx| {
        if let Some(loader) = loader {
            agents.set_registry_loader(loader);
        }
        agents.set_connections(engine, cx);
    });
    cx.update(|window, cx| window.dispatch_action(Box::new(crate::actions::OpenConnections), cx));
    cx.run_until_parked();
    let view = view(&workspace, cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.section()),
        SettingsSection::Connections
    );
    (workspace, view, cx)
}

/// §4.6: renaming from the tab changes the label in the chat's popover and
/// in `connections.json`; an empty name is refused.
#[gpui::test]
fn renaming_from_the_tab_reaches_the_popover_and_the_index(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let env = FakeEnv::new();
    let id = env.add_connection("claude-acp", "Claude · personal");
    let engine = offline_engine(&env, None);
    let (workspace, view, cx) = connections_window(&env, engine.clone(), None, cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.connections_info().connections.len()),
        1
    );

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.start_rename(id, window, cx);
            view.set_rename_text("   ", window, cx);
            view.save_rename(window, cx);
        })
    });
    assert_eq!(
        view.read_with(cx, |view, _| view.rename_error().map(str::to_string)),
        Some(EMPTY_NAME_ERROR.to_string())
    );

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.set_rename_text("Claude · trabajo", window, cx);
            view.save_rename(window, cx);
        })
    });
    cx.run_until_parked();

    assert_eq!(engine.store().get(id).unwrap().label, "Claude · trabajo");
    let chat = workspace.read_with(cx, |workspace, _| workspace.chat().clone());
    assert_eq!(
        chat.read_with(cx, |chat, _| chat.connections()[0].label.clone()),
        "Claude · trabajo"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.connections_info().connections[0]
            .label
            .clone()),
        "Claude · trabajo"
    );
    assert!(settings_view_has_focus(&workspace, cx));
}

/// §4.6: with the fake registry publishing a newer adapter, "Actualizar a
/// X.Y.Z" shows up and using it installs it (the old version goes, nothing
/// running uses it).
#[gpui::test]
fn a_newer_adapter_in_the_registry_is_offered_and_installed(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let env = FakeEnv::new();
    env.add_connection("claude-acp", "Claude · personal");
    let engine = offline_engine(&env, Some(Box::new(FakeNpm)));
    let (_workspace, view, cx) = connections_window(
        &env,
        engine.clone(),
        Some(registry_with_claude("0.0.2")),
        cx,
    );

    let info = view.read_with(cx, |view, _| view.connections_info().clone());
    assert_eq!(info.catalog, CatalogState::Checked);
    let claude = info
        .agents
        .iter()
        .find(|agent| agent.kind == AgentKind::Claude)
        .expect("Claude está instalado");
    assert_eq!(claude.version, "0.0.1");
    assert_eq!(claude.update.as_deref(), Some("0.0.2"));
    assert!(
        info.agents
            .iter()
            .filter(|agent| agent.kind != AgentKind::Claude)
            .all(|agent| agent.update.is_none())
    );
    assert_eq!(info.node.installed.as_deref(), Some("v24.1.0"));
    assert_eq!(
        info.node.update, None,
        "Node fijado: nunca hay actualización"
    );

    view.update(cx, |view, cx| {
        view.request(SettingsViewEvent::UpdateAdapter(AgentKind::Claude), cx)
    });
    cx.run_until_parked();

    assert_eq!(
        engine.adapters().installed("claude-acp").as_deref(),
        Some("0.0.2")
    );
    let info = view.read_with(cx, |view, _| view.connections_info().clone());
    let claude = info
        .agents
        .iter()
        .find(|agent| agent.kind == AgentKind::Claude)
        .unwrap();
    assert_eq!(claude.version, "0.0.2");
    assert_eq!(claude.update, None);
    assert_eq!(
        claude.outcome,
        Some(UpdateOutcome::Updated("Actualizado a 0.0.2".to_string()))
    );
    assert!(info.operation.is_none());
    assert!(
        !env.paths
            .agents_dir()
            .join("claude-acp")
            .join("0.0.1")
            .exists(),
        "sin agente en marcha, la versión vieja se borra"
    );
}

/// §4.6 fix: even when something upstream hands the tab a stale "update"
/// that is actually a downgrade (the bundled catalog can lag behind an
/// adapter installed through a fresher "Buscar actualizaciones" check), the
/// row must not offer it — the render layer's own numeric guard is
/// authoritative, on top of `Adapters::update_available` already refusing to
/// report one.
#[gpui::test]
fn a_row_never_offers_a_version_lower_than_what_is_installed(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let env = FakeEnv::new();
    env.add_connection("claude-acp", "Claude · personal");
    let engine = offline_engine(&env, None);
    let (_workspace, view, cx) = connections_window(&env, engine.clone(), None, cx);

    let mut info = view.read_with(cx, |view, _| view.connections_info().clone());
    info.catalog = CatalogState::Checked;
    let claude = info
        .agents
        .iter_mut()
        .find(|agent| agent.kind == AgentKind::Claude)
        .expect("Claude está instalado");
    assert_eq!(claude.version, "0.0.1");
    claude.update = Some("0.0.0".to_string());
    view.update(cx, |view, cx| view.set_connections_info(info.clone(), cx));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("settings-update-0").is_none(),
        "no debe ofrecerse «Actualizar» a 0.0.0, menor que 0.0.1 instalado"
    );

    // A genuine newer version still renders: the guard does not swallow it.
    let claude = info
        .agents
        .iter_mut()
        .find(|agent| agent.kind == AgentKind::Claude)
        .expect("Claude está instalado");
    claude.update = Some("0.0.2".to_string());
    view.update(cx, |view, cx| view.set_connections_info(info, cx));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("settings-update-0").is_some(),
        "0.0.2 es más nuevo que 0.0.1: debe ofrecerse"
    );
}

/// §10.4 fix: the row that replaces "Actualizar a X.Y.Z" while an update is
/// in progress must stay inside its own container — the text used to
/// overflow and clip "Cancelar" down to "Cancela".
///
/// **Deviation:** reaching this state through a real update needs a
/// controllable slow installer; `FakeNpm` (like `FakeEnv`'s connect flow,
/// see `connection_cancel_tests.rs`) has none, so a real update resolves
/// almost instantly and races the test's single threaded executor. This
/// uses `SettingsView::set_operation_for_test` to reach the exact state
/// deterministically instead. Mutual exclusion with the "Actualizar a…"
/// button is structural, not asserted here: `update_area`'s progress branch
/// `return`s before that button is ever built, so both can never render in
/// the same call.
#[gpui::test]
fn the_update_progress_row_never_overflows_its_container(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let env = FakeEnv::new();
    env.add_connection("claude-acp", "Claude · personal");
    let engine = offline_engine(&env, Some(Box::new(FakeNpm)));
    let (_workspace, view, cx) = connections_window(
        &env,
        engine.clone(),
        Some(registry_with_claude("0.0.2")),
        cx,
    );

    view.update(cx, |view, cx| {
        view.set_operation_for_test(
            UpdateOperation {
                target: UpdateTarget::Adapter(AgentKind::Claude),
                percent: Some(45.),
                text: "Actualizando el adaptador de Claude… 45 %".to_string(),
            },
            cx,
        )
    });
    cx.run_until_parked();

    // `position` is the agent's index in `info.agents` (0: only Claude is
    // installed in this fixture).
    let area = cx
        .debug_bounds("settings-update-progress-area-0")
        .expect("la fila de progreso se pintó");
    let row = cx
        .debug_bounds("settings-update-progress-row-0")
        .expect("el texto y «Cancelar» se pintaron");
    let cancel = cx
        .debug_bounds("settings-update-cancel-0")
        .expect("«Cancelar» se pintó completo, no cortado");

    assert!(
        row.size.width <= area.size.width,
        "la fila {row:?} no debe superar el ancho de su contenedor {area:?}"
    );
    assert!(
        cancel.right() <= area.right(),
        "«Cancelar» {cancel:?} se sale de su contenedor {area:?}"
    );
    assert!(
        cancel.left() >= area.left(),
        "«Cancelar» {cancel:?} arranca antes que su contenedor {area:?}"
    );
}

/// §4.4: offline, the catalog says so and no update is offered.
#[gpui::test]
fn offline_the_section_says_so_and_offers_no_update(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let env = FakeEnv::new();
    let engine = offline_engine(&env, None);
    let (_workspace, view, cx) = connections_window(&env, engine, None, cx);
    let info = view.read_with(cx, |view, _| view.connections_info().clone());
    assert_eq!(info.catalog, CatalogState::Offline);
    assert!(info.agents.iter().all(|agent| agent.update.is_none()));
    assert!(
        view.read_with(cx, |view, cx| view.visible_texts(cx))
            .iter()
            .any(|text| text == OFFLINE_CATALOG)
    );
}

/// §4.4: the other buttons reach the existing flows of the connections
/// modal, through `Agents`; closing the modal gives the keyboard back to
/// the tab.
#[gpui::test]
fn connection_buttons_open_the_modal_flows(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let env = FakeEnv::new();
    let id = env.add_connection("claude-acp", "Claude · personal");
    let engine = offline_engine(&env, None);
    let (workspace, view, cx) = connections_window(&env, engine, None, cx);
    let modal = workspace.read_with(cx, |workspace, cx| {
        workspace.agents().read(cx).modal().clone()
    });

    view.update(cx, |view, cx| {
        view.request(SettingsViewEvent::Delete { id }, cx)
    });
    cx.run_until_parked();
    assert!(modal.read_with(cx, |modal, _| matches!(
        modal.mode(),
        Mode::Delete(DeleteStep::Confirm { id: shown, .. }) if *shown == id
    )));
    cx.update(|window, cx| modal.update(cx, |modal, cx| modal.close(window, cx)));
    cx.run_until_parked();
    assert!(settings_view_has_focus(&workspace, cx));

    view.update(cx, |view, cx| {
        view.request(SettingsViewEvent::NewConnection, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        modal.read_with(cx, |modal, _| modal.step().cloned()),
        Some(Step::Choose)
    );
}

// ------------------------------------------------------------------ focus

/// §7.6: with the settings tab active, closing a panel gives the keyboard
/// back to the settings tab (not to a hidden editor).
#[gpui::test]
fn closing_a_panel_returns_the_focus_to_the_settings_tab(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let project = sample_project();
    let (workspace, cx) = workspace_window(Some(project.path()), cx);
    open_pinned(&workspace, project.path(), "src/main.rs", cx);
    press("ctrl-,", cx);
    assert!(settings_view_has_focus(&workspace, cx));
    assert!(!workspace.read_with(cx, |workspace, cx| workspace.is_modal_open(cx)));

    press("ctrl-shift-a", cx);
    assert_eq!(
        cx.update(|window, cx| workspace.read(cx).focus_zone(window, cx)),
        Some(FocusZone::Chat)
    );
    press("ctrl-shift-a", cx);
    assert!(settings_view_has_focus(&workspace, cx));

    press("ctrl-shift-e", cx);
    assert_eq!(
        cx.update(|window, cx| workspace.read(cx).focus_zone(window, cx)),
        Some(FocusZone::Files)
    );
    press("ctrl-shift-e", cx);
    assert!(settings_view_has_focus(&workspace, cx));
}

/// §9.1: the settings tab counts as the center for `Ctrl+L`.
#[gpui::test]
fn ctrl_l_treats_the_settings_tab_as_the_center(cx: &mut TestAppContext) {
    let (_config, _) = init_test(None, cx);
    let project = sample_project();
    let (workspace, cx) = workspace_window(Some(project.path()), cx);
    press("ctrl-,", cx);
    let zone = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| workspace.read(cx).focus_zone(window, cx))
    };
    assert_eq!(zone(cx), Some(FocusZone::Center));

    press("ctrl-l", cx);
    assert_eq!(zone(cx), Some(FocusZone::Files));
    press("ctrl-l", cx);
    assert_eq!(zone(cx), Some(FocusZone::Chat));
    press("ctrl-l", cx);
    assert!(settings_view_has_focus(&workspace, cx));
}
