//! The root view: integrated title bar, three-panel dock, status bar.
//!
//! See `docs/specs/02-visual.md` §1 and `docs/specs/modulos/workspace.md`. The
//! workspace owns the window and wires the pieces together: it opens the
//! project, hands it to the file tree and the tab area, routes the
//! `workspace::*` commands, keeps the toast queue, and saves the layout.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use asteroid_project::Recents;
use asteroid_settings::{SettingsEvent, SettingsWatcher};
use asteroid_text::LineEnding;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, Subscription, Task, Window,
    WindowBounds, WindowHandle,
};
use gpui_kit::component::dock::{DockArea, DockLayout, DockPlacement, DockSkin, panel_handle};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{Root, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, SharedString, WindowKind, WindowOptions, div, px};

use crate::actions;
use crate::center::CenterPanel;
use crate::layout::{DockLayout as DockLayoutState, WorkspaceLayout};
use crate::panels::ChatPanel;
use crate::project::{self, Project, ProjectOptions};
use crate::theme::ThemeColors;
use crate::toast::{self, Toasts};
use crate::tree_panel::FilesPanel;
use crate::window_state::{self, WindowState};

/// The GPUI key context of the root view. Every binding without a context of
/// its own, and the widget bindings of [`crate::keymap`], match against it.
pub const KEY_CONTEXT: &str = "Workspace";

/// Identity of the dock layout, for gpui-kit's own persistence.
const DOCK_AREA_ID: &str = "asteroid-main";
/// Bumped whenever the default layout changes shape.
const DOCK_AREA_VERSION: usize = 1;

/// Initial width of the chat dock (`02-visual.md` §1).
pub const CHAT_WIDTH: f32 = 380.;
/// Initial width of the file tree dock.
pub const TREE_WIDTH: f32 = 240.;
/// Status bar height (`02-visual.md` §4).
const STATUS_BAR_HEIGHT: f32 = 26.;

/// How many frames the workspace has rendered, shared with the caller so a
/// smoke test can assert that the window actually drew something.
pub type FrameCounter = Arc<AtomicUsize>;

/// What the workspace emits for other parts of the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceEvent {
    /// A file should be opened: previewed (`pin: false`) or pinned.
    OpenFile {
        /// Absolute path.
        path: PathBuf,
        /// Whether the tab is pinned rather than a preview.
        pin: bool,
    },
}

/// Everything the binary hands over when it opens the window.
#[derive(Default)]
pub struct WorkspaceOptions {
    /// Rendered frame counter, for `--smoke-test`.
    pub frames: FrameCounter,
    /// The project to open, when the command line gave one.
    pub project: Option<PathBuf>,
    /// Whether to fall back to the most recent project.
    pub open_last_project: bool,
    /// The configuration watcher and its channel, for hot reload.
    pub settings_watcher: Option<(SettingsWatcher, async_channel::Receiver<SettingsEvent>)>,
    /// Problems found while reading the configuration, to show as toasts.
    pub startup_issues: Vec<String>,
    /// Which background pieces the project starts. The tests turn them off.
    pub project_options: ProjectOptions,
}

/// The root view of the application window.
pub struct Workspace {
    dock_area: Entity<DockArea>,
    /// Kept alive: the skin owns the dock's presentation settings.
    _skin: Rc<DockSkin>,
    chat: Entity<ChatPanel>,
    center: Entity<CenterPanel>,
    files: Entity<FilesPanel>,
    toasts: Entity<Toasts>,
    project: Option<Entity<Project>>,
    recents: Recents,
    project_options: ProjectOptions,
    focus_handle: FocusHandle,
    frames: FrameCounter,
    /// Latest geometry seen while rendering, so quitting can persist it
    /// without touching the window (which is already gone by then).
    last_bounds: Rc<Cell<Option<WindowState>>>,
    /// Kept alive: dropping it stops the configuration watch.
    _settings_watcher: Option<SettingsWatcher>,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    /// Builds the workspace and its default three-panel layout.
    pub fn new(options: WorkspaceOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (dock_area, skin) =
            DockSkin::dock_area(DOCK_AREA_ID, Some(DOCK_AREA_VERSION), window, cx);

        let chat = ChatPanel::new(cx);
        let center = CenterPanel::new(cx);
        let files = FilesPanel::new(cx);
        let toasts = cx.new(|_| Toasts::new());
        toast::set_global(&toasts, cx);

        dock_area.update(cx, |area, cx| {
            area.set_center(
                DockLayout::v_split().child(
                    DockLayout::tabs().panel_view(panel_handle(center.clone()), cx),
                    None,
                ),
                window,
                cx,
            );

            for (placement, panel, width) in [
                (
                    DockPlacement::Left,
                    panel_handle(chat.clone()),
                    px(CHAT_WIDTH),
                ),
                (
                    DockPlacement::Right,
                    panel_handle(files.clone()),
                    px(TREE_WIDTH),
                ),
            ] {
                area.set_dock(
                    placement,
                    DockLayout::v_split().child(DockLayout::tabs().panel_view(panel, cx), None),
                    window,
                    cx,
                );
                area.set_dock_size(placement, width, window, cx);
                area.set_dock_collapsible(placement, true, window, cx);
            }
        });

        let mut subscriptions = Vec::new();
        // The tree asks for files; the tab area opens them.
        subscriptions.push(cx.subscribe_in(
            &files,
            window,
            |this, _, event, window, cx| match event {
                WorkspaceEvent::OpenFile { path, pin } => {
                    let path = path.clone();
                    let pin = *pin;
                    this.center
                        .update(cx, |center, cx| center.open_file(&path, pin, window, cx));
                }
            },
        ));

        // The status bar reads the active tab, so the workspace repaints
        // whenever the tab area changes (cursor, dirty dot, tab switch).
        subscriptions.push(cx.observe(&center, |_, _, cx| cx.notify()));

        // `files.autosave = "on_focus_change"`: leaving the window is a focus
        // change (`docs/specs/modulos/settings.md`).
        subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.center
                    .update(cx, |center, cx| center.autosave(window, cx));
            }
        }));

        let mut tasks = Vec::new();
        let settings_watcher = match options.settings_watcher {
            Some((watcher, events)) => {
                tasks.push(cx.spawn(async move |this, cx| {
                    while let Ok(event) = events.recv().await {
                        let reloaded = cx.update(|cx| crate::settings::reload(event, cx));
                        if !reloaded.changed {
                            continue;
                        }
                        let reported = this.update(cx, |workspace, cx| {
                            // Settings, fonts and colours reach the open
                            // editors here; the element has no globals.
                            workspace
                                .center
                                .update(cx, |center, cx| center.refresh_editor_style(cx));
                            for issue in &reloaded.issues {
                                toast::warn(issue.to_string(), cx);
                            }
                            if reloaded.issues.is_empty() {
                                toast::info_keyed(
                                    "settings-reloaded",
                                    "Configuración recargada",
                                    cx,
                                );
                            }
                            cx.notify();
                        });
                        if reported.is_err() {
                            break;
                        }
                    }
                }));
                Some(watcher)
            }
            None => None,
        };

        let last_bounds = Rc::new(Cell::new(None));
        cx.on_app_quit({
            let last_bounds = last_bounds.clone();
            move |workspace: &mut Self, cx: &mut Context<Self>| {
                save_window_state(&last_bounds);
                workspace.save_layout(cx);
                async {}
            }
        })
        .detach();

        // `theme.mode = "system"` follows the desktop: the appearance is read
        // now and observed from here on (`docs/specs/modulos/settings.md`).
        let dark = is_dark(window.appearance());
        crate::settings::set_system_dark(dark, cx);
        subscriptions.push(window.observe_window_appearance(|window, cx| {
            if crate::settings::set_system_dark(is_dark(window.appearance()), cx) {
                cx.refresh_windows();
            }
        }));

        // GPUI dispatches an action along the focus path, so the global
        // commands only reach the handlers below once something inside the
        // workspace holds focus. Take it at startup; clicking a panel later
        // moves focus to a descendant, which still bubbles here.
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let mut workspace = Self {
            dock_area,
            _skin: skin,
            chat,
            center,
            files,
            toasts,
            project: None,
            recents: Recents::load(),
            project_options: options.project_options,
            focus_handle,
            frames: options.frames,
            last_bounds,
            _settings_watcher: settings_watcher,
            _tasks: tasks,
            _subscriptions: subscriptions,
        };

        for issue in options.startup_issues {
            toast::warn(issue, cx);
        }

        let initial = options.project.or_else(|| {
            options
                .open_last_project
                .then(|| workspace.recents.most_recent().map(Path::to_path_buf))
                .flatten()
        });
        if let Some(path) = initial {
            workspace.open_project(&path, window, cx);
        }

        workspace
    }

    /// Opens `path` as the project, replacing whatever was open.
    pub fn open_project(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if self.project.is_some() {
            self.save_layout(cx);
        }
        let settings = crate::settings::settings(cx);
        let project = match project::open_with(path, &settings, self.project_options, cx) {
            Ok(project) => project,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "no se pudo abrir el proyecto");
                toast::error(
                    format!("No se pudo abrir «{}»: {error}", path.display()),
                    cx,
                );
                return;
            }
        };

        let root = project.read(cx).root().to_path_buf();
        self.project = Some(project.clone());
        self.files
            .update(cx, |files, cx| files.set_project(Some(project.clone()), cx));
        self.center
            .update(cx, |center, cx| center.set_project(Some(project), cx));
        self.recents = Recents::load();

        let layout = WorkspaceLayout::load(&root).unwrap_or_default();
        self.apply_layout(&layout, window, cx);
        window.set_window_title(&format!(
            "{} — Asteroid",
            root.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| root.display().to_string())
        ));
        cx.notify();
    }

    /// The project on screen, if any.
    pub fn project(&self) -> Option<&Entity<Project>> {
        self.project.as_ref()
    }

    /// The tab area.
    pub fn center(&self) -> &Entity<CenterPanel> {
        &self.center
    }

    /// The file tree panel.
    pub fn files(&self) -> &Entity<FilesPanel> {
        &self.files
    }

    /// The toast queue.
    pub fn toasts(&self) -> &Entity<Toasts> {
        &self.toasts
    }

    /// Whether the dock at `placement` is expanded.
    pub fn is_dock_open(&self, placement: DockPlacement, cx: &App) -> bool {
        self.dock_area.read(cx).is_dock_open(placement)
    }

    /// Restores dock sizes, collapsed state and tabs.
    fn apply_layout(
        &mut self,
        layout: &WorkspaceLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dock_area.update(cx, |area, cx| {
            for (placement, dock) in [
                (DockPlacement::Left, layout.chat),
                (DockPlacement::Right, layout.tree),
            ] {
                if dock.size > 0. {
                    area.set_dock_size(placement, px(dock.size), window, cx);
                }
                if area.is_dock_open(placement) != dock.open {
                    area.toggle_dock(placement, window, cx);
                }
            }
        });
        let tabs = layout.tabs.clone();
        let active = layout.active;
        self.center
            .update(cx, |center, cx| center.restore(&tabs, active, window, cx));
    }

    /// Writes the layout of the open project.
    pub fn save_layout(&self, cx: &App) {
        let Some(project) = &self.project else {
            return;
        };
        let root = project.read(cx).root().to_path_buf();
        let area = self.dock_area.read(cx);
        let (tabs, active) = self.center.read(cx).to_layout();
        let layout = WorkspaceLayout {
            version: crate::layout::LAYOUT_VERSION,
            chat: DockLayoutState {
                size: area
                    .dock_size(DockPlacement::Left)
                    .map(f32::from)
                    .unwrap_or(CHAT_WIDTH),
                open: area.is_dock_open(DockPlacement::Left),
            },
            tree: DockLayoutState {
                size: area
                    .dock_size(DockPlacement::Right)
                    .map(f32::from)
                    .unwrap_or(TREE_WIDTH),
                open: area.is_dock_open(DockPlacement::Right),
            },
            tabs,
            active,
        };
        if let Err(error) = layout.save(&root) {
            tracing::warn!(%error, "no se pudo guardar el layout del proyecto");
        }
    }

    /// Opens the main window with the workspace inside it.
    ///
    /// The returned task resolves once the window exists; a failure here means
    /// GPUI could not get a surface at all (see `03-arquitectura.md` §7).
    pub fn open_window(
        options: WorkspaceOptions,
        cx: &mut App,
    ) -> Task<anyhow::Result<WindowHandle<Root>>> {
        let window_bounds = window_state::initial_window_bounds(cx);
        let decorations = crate::settings::settings(cx).window.decorations;

        cx.spawn(async move |cx| {
            let window_options = WindowOptions {
                window_bounds: Some(window_bounds),
                window_min_size: Some(window_state::min_window_size()),
                kind: WindowKind::Normal,
                app_id: Some("dev.asteroid.Asteroid".into()),
                // Client-side decorations: the title bar is ours to draw
                // (`02-visual.md` §1). The transparent background is what
                // lets gpui-kit paint the rounded border and the shadow.
                #[cfg(target_os = "linux")]
                window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
                #[cfg(target_os = "linux")]
                window_decorations: Some(match decorations {
                    asteroid_settings::Decorations::Server => gpui_kit::WindowDecorations::Server,
                    asteroid_settings::Decorations::Client => gpui_kit::WindowDecorations::Client,
                }),
                ..TitleBar::window_options()
            };

            let window = cx.open_window(window_options, |window, cx| {
                let workspace = cx.new(|cx| Workspace::new(options, window, cx));
                cx.new(|cx| Root::new(workspace, window, cx))
            })?;

            window.update(cx, |_, window, cx| {
                window.set_window_title("Asteroid");
                window.activate_window();
                report_gpu(window);

                // Closing the only window ends the application, and the
                // geometry has to be on disk before that happens.
                window.on_window_should_close(cx, |window, _| {
                    if let Err(error) =
                        WindowState::from_window_bounds(window.window_bounds()).save()
                    {
                        tracing::warn!(%error, "no se pudo guardar el estado de la ventana");
                    }
                    true
                });

                cx.on_release(|_, cx| cx.quit()).detach();
            })?;

            Ok(window)
        })
    }

    fn on_open_folder(
        &mut self,
        _: &actions::OpenFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pick_folder(window, cx);
    }

    /// Asks the desktop for a folder through the xdg portal and opens it.
    ///
    /// `rfd::AsyncFileDialog` talks to the portal over D-Bus; its future is
    /// `Send`, so it runs on GPUI's background executor and the answer comes
    /// back to the main thread through a channel
    /// (`docs/specs/03-arquitectura.md` §1: "Diálogos: `rfd` vía portal xdg").
    fn pick_folder(&mut self, window: &Window, cx: &mut Context<Self>) {
        let start = self
            .project
            .as_ref()
            .map(|project| project.read(cx).root().to_path_buf());
        let (sender, receiver) = async_channel::bounded::<Option<PathBuf>>(1);

        cx.background_executor()
            .spawn(async move {
                let mut dialog = rfd::AsyncFileDialog::new().set_title("Abrir carpeta");
                if let Some(start) = start {
                    dialog = dialog.set_directory(start);
                }
                let picked = dialog
                    .pick_folder()
                    .await
                    .map(|handle| handle.path().to_path_buf());
                let _ = sender.send(picked).await;
            })
            .detach();

        cx.spawn_in(window, async move |this, cx| {
            let Ok(Some(path)) = receiver.recv().await else {
                tracing::debug!("el usuario canceló el diálogo de carpeta");
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.open_project(&path, window, cx);
            });
        })
        .detach();
    }

    fn on_toggle_chat(
        &mut self,
        _: &actions::ToggleChat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle(DockPlacement::Left, window, cx);
    }

    fn on_toggle_tree(
        &mut self,
        _: &actions::ToggleTree,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle(DockPlacement::Right, window, cx);
    }

    fn on_focus_chat(
        &mut self,
        _: &actions::FocusChat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dock_area.update(cx, |area, cx| {
            if !area.is_dock_open(DockPlacement::Left) {
                area.toggle_dock(DockPlacement::Left, window, cx);
            }
        });
        let handle = self.chat.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    fn on_close_tab(&mut self, _: &actions::CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.center
            .update(cx, |center, cx| center.close_active(window, cx));
    }

    fn on_zoom_in(&mut self, _: &actions::ZoomIn, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom(
            crate::settings::ui_scale(cx) + crate::settings::UI_SCALE_STEP,
            window,
            cx,
        );
    }

    fn on_zoom_out(&mut self, _: &actions::ZoomOut, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom(
            crate::settings::ui_scale(cx) - crate::settings::UI_SCALE_STEP,
            window,
            cx,
        );
    }

    fn on_zoom_reset(
        &mut self,
        _: &actions::ZoomReset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.zoom(1., window, cx);
    }

    fn zoom(&mut self, scale: f32, window: &mut Window, cx: &mut Context<Self>) {
        let scale = crate::settings::set_ui_scale(scale, cx);
        // Every gpui-kit size is expressed in rems, so the root size is what
        // actually zooms the interface; the theme carries the font sizes.
        window.set_rem_size(px(16. * scale));
        // The code font zooms with the interface.
        self.center
            .update(cx, |center, cx| center.refresh_editor_style(cx));
        // Keyed: pressing `Ctrl+=` five times leaves one message, not five.
        toast::info_keyed(
            "zoom",
            format!("Zoom {} %", (scale * 100.).round() as i32),
            cx,
        );
        cx.notify();
    }

    /// `editor::save_all`.
    fn on_save_all(&mut self, _: &actions::SaveAll, window: &mut Window, cx: &mut Context<Self>) {
        self.center
            .update(cx, |center, cx| center.save_all(window, cx));
    }

    /// Every stage 3 command lands here until the review lands.
    fn on_stage_three(&mut self, cx: &mut Context<Self>) {
        toast::info_keyed("stage-three", "Disponible en Etapa 3", cx);
    }

    fn toggle(&mut self, placement: DockPlacement, window: &mut Window, cx: &mut Context<Self>) {
        self.dock_area.update(cx, |area, cx| {
            area.toggle_dock(placement, window, cx);
            tracing::debug!(
                ?placement,
                open = area.is_dock_open(placement),
                "dock alternado"
            );
        });
        cx.notify();
    }

    /// The "no project" screen of `docs/specs/02-visual.md` §9.
    fn render_empty_state(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeColors::global(cx).clone();
        let recents: Vec<PathBuf> = self.recents.projects().to_vec();

        v_flex()
            .id("empty-state")
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(theme.bg_app)
            .child(
                v_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(34.))
                            .text_color(theme.text_accent)
                            .child("✦"),
                    )
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child("Asteroid"),
                    ),
            )
            .child(
                // The same neutral button as the dialogs (`02-visual.md` §4).
                div()
                    .id("empty-open-folder")
                    .h(px(28.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .bg(theme.bg_surface)
                    .border_1()
                    .border_color(theme.border_focus)
                    .text_color(theme.text)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.bg_elevated))
                    .child("Abrir carpeta (Ctrl+O)")
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.pick_folder(window, cx)
                        }),
                    ),
            )
            .when(!recents.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child("Proyectos recientes"),
                        )
                        .children(recents.into_iter().enumerate().map(|(index, path)| {
                            let label = SharedString::from(path.display().to_string());
                            div()
                                .id(("recent", index))
                                .px_2()
                                .py_0p5()
                                .rounded(px(4.))
                                .text_sm()
                                .text_color(theme.text_accent)
                                .cursor_pointer()
                                .hover(|style| style.bg(theme.bg_surface))
                                .child(label)
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    let path = path.clone();
                                    this.open_project(&path, window, cx);
                                }))
                        })),
                )
            })
    }

    /// The status bar of `docs/specs/02-visual.md` §1.
    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeColors::global(cx).clone();
        let center = self.center.read(cx);
        let tab = center.active_tab();
        let language = tab
            .and_then(|tab| tab.language.as_ref())
            .map(|language| SharedString::from(language.name()))
            .unwrap_or_else(|| SharedString::from("Texto"));
        let eol = match tab.map(|tab| tab.line_ending()) {
            Some(LineEnding::Crlf) => "CRLF",
            _ => "LF",
        };
        let name = tab
            .map(|tab| tab.display_title())
            .unwrap_or_else(|| SharedString::from("Sin archivo"));
        let (line, column) = tab.map(|tab| tab.cursor_line_column()).unwrap_or((1, 1));
        let read_only = tab.is_some_and(|tab| tab.read_only);
        let zoom = (crate::settings::ui_scale(cx) * 100.).round() as i32;

        StatusBar::new()
            .h(px(STATUS_BAR_HEIGHT))
            .flex_none()
            .py_0()
            .left(
                h_flex()
                    .gap_3()
                    .text_color(theme.text_muted)
                    .child(name)
                    .child(SharedString::from(format!("Ln {line}, Col {column}")))
                    .child(SharedString::from("UTF-8"))
                    .child(SharedString::from(eol))
                    // Clicking the language will open the language picker in a
                    // later stage; today it is only a label.
                    .child(div().id("status-language").child(language))
                    .children(read_only.then(|| SharedString::from("Solo lectura"))),
            )
            .right(
                h_flex()
                    .gap_3()
                    .text_color(theme.text_muted)
                    .child(SharedString::from("Sin agente"))
                    // Clicking the pending counter opens the review panel
                    // (`02-visual.md` §1); the panel itself is stage 3.
                    .child(
                        div()
                            .id("status-pending")
                            .cursor_pointer()
                            .child(SharedString::from("0 pendientes"))
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.on_stage_three(cx)),
                            ),
                    )
                    .child(SharedString::from(format!("{zoom} %"))),
            )
    }
}

impl gpui::EventEmitter<WorkspaceEvent> for Workspace {}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames.fetch_add(1, Ordering::Relaxed);
        self.last_bounds.set(Some(WindowState::from_window_bounds(
            window.window_bounds(),
        )));

        let theme = ThemeColors::global(cx).clone();
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);
        let has_project = self.project.is_some();
        let dock_inset = if has_project {
            let area = self.dock_area.read(cx);
            let side = |placement| {
                if area.is_dock_open(placement) {
                    area.dock_size(placement).map(f32::from).unwrap_or(0.)
                } else {
                    0.
                }
            };
            (side(DockPlacement::Left), side(DockPlacement::Right))
        } else {
            (0., 0.)
        };
        let title = match &self.project {
            Some(project) => {
                SharedString::from(format!("Asteroid — {}", project.read(cx).root().display()))
            }
            None => SharedString::from("Asteroid"),
        };

        let body = if has_project {
            div()
                .flex_1()
                .min_h_0()
                .child(self.dock_area.clone())
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .child(self.render_empty_state(cx))
                .into_any_element()
        };

        div()
            .id("workspace")
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::on_open_folder))
            .on_action(cx.listener(Self::on_toggle_chat))
            .on_action(cx.listener(Self::on_toggle_tree))
            .on_action(cx.listener(Self::on_focus_chat))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_save_all))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_reset))
            .on_action(cx.listener(|this, _: &actions::NextChange, _, cx| this.on_stage_three(cx)))
            .on_action(cx.listener(|this, _: &actions::PrevChange, _, cx| this.on_stage_three(cx)))
            .on_action(
                cx.listener(|this, _: &actions::NextFileWithChanges, _, cx| {
                    this.on_stage_three(cx)
                }),
            )
            .on_action(cx.listener(|this, _: &actions::AcceptTurn, _, cx| this.on_stage_three(cx)))
            .on_action(cx.listener(|this, _: &actions::RejectTurn, _, cx| this.on_stage_three(cx)))
            .on_action(
                cx.listener(|this, _: &actions::UndoLastReject, _, cx| this.on_stage_three(cx)),
            )
            .on_action(
                cx.listener(|this, _: &actions::OpenReviewPanel, _, cx| this.on_stage_three(cx)),
            )
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg_app)
            .text_color(theme.text)
            .child(
                TitleBar::new().child(
                    div()
                        .flex()
                        .items_center()
                        .px_1()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                ),
            )
            .child(body)
            .child(self.render_status_bar(cx))
            // Toasts float over the editor area, bottom center
            // (`02-visual.md` §6.5). "Editor area" is the center of the dock,
            // so the two side docks are subtracted from the band.
            .child(
                div()
                    .absolute()
                    .bottom(px(STATUS_BAR_HEIGHT + 16.))
                    .left(px(dock_inset.0))
                    .right(px(dock_inset.1))
                    .flex()
                    .justify_center()
                    .child(self.toasts.clone()),
            )
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
}

/// Whether a window appearance counts as dark.
fn is_dark(appearance: gpui::WindowAppearance) -> bool {
    matches!(
        appearance,
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
    )
}

/// Writes the last geometry the workspace saw. Used on quit, when the window
/// may already be gone.
fn save_window_state(last_bounds: &Rc<Cell<Option<WindowState>>>) {
    let Some(state) = last_bounds.get() else {
        return;
    };
    if let Err(error) = state.save() {
        tracing::warn!(%error, "no se pudo guardar el estado de la ventana");
    }
}

/// Logs which GPU GPUI ended up on, and refuses to run on a software
/// rasterizer unless `ASTEROID_ALLOW_SOFTWARE_GPU=1`
/// (`03-arquitectura.md` §7).
///
/// The Vulkan → OpenGL cascade the spec asks for is not something this crate
/// can drive: `gpui-pre-wgpu` 0.3.5 builds its wgpu instance with
/// `Backends::VULKAN | Backends::GL` hard-coded, enumerates every adapter of
/// both backends and sorts them before testing each one against the real
/// surface. What is left for us is the last rung: detecting that the winner is
/// a CPU adapter and stopping.
fn report_gpu(window: &Window) {
    let Some(specs) = window.gpu_specs() else {
        tracing::warn!("GPUI no informó datos de GPU");
        return;
    };

    tracing::info!(
        device = %specs.device_name,
        driver = %specs.driver_name,
        info = %specs.driver_info,
        software = specs.is_software_emulated,
        "GPU seleccionada"
    );

    if specs.is_software_emulated {
        let allowed = std::env::var("ASTEROID_ALLOW_SOFTWARE_GPU").as_deref() == Ok("1");
        if allowed {
            tracing::warn!(
                "sin GPU: se está usando un rasterizador por software ({}); el rendimiento va a ser malo",
                specs.device_name
            );
        } else {
            tracing::error!(
                "no se encontró una GPU utilizable (adaptador por software: {}). \
                 Ejecutá con ASTEROID_ALLOW_SOFTWARE_GPU=1 para seguir igual.",
                specs.device_name
            );
            std::process::exit(2);
        }
    }
}

/// The window bounds a fresh profile gets, exposed for tests and tooling.
pub fn default_window_bounds(cx: &App) -> WindowBounds {
    window_state::initial_window_bounds(cx)
}
