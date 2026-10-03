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

use cincel_chat::ChatPanel;
use cincel_project::Recents;
use cincel_settings::{SettingsEvent, SettingsWatcher};
use cincel_text::LineEnding;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, Pixels, Subscription, Task, Window,
    WindowBounds, WindowHandle,
};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::{DockArea, DockLayout, DockPlacement, DockSkin, panel_handle};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Root, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, SharedString, WindowKind, WindowOptions, div, px};

use crate::actions;
use crate::agents::Agents;
use crate::center::CenterPanel;
use crate::file_finder::FileFinder;
use crate::focus::FocusZone;
use crate::image_viewer::ImageViewer;
use crate::layout::{DockLayout as DockLayoutState, WorkspaceLayout};
use crate::new_file::NewFilePrompt;
use crate::panels::ChatDock;
use crate::project::{self, Project, ProjectOptions};
use crate::review::{Nav, PanelState, Review, stats_label};
use crate::review_close::ReviewCloseDialog;
use crate::settings::Reloaded;
use crate::settings_view::{SettingsSection, SettingsViewEvent};
use crate::shortcuts_modal::ShortcutsModal;
use crate::theme::ThemeColors;
use crate::title_menu::QuitDialog;
use crate::toast::{self, Toasts};
use crate::tree_panel::FilesPanel;
use crate::window_state::{self, WindowState};

/// The GPUI key context of the root view. Every binding without a context of
/// its own, and the widget bindings of [`crate::keymap`], match against it.
pub const KEY_CONTEXT: &str = "Workspace";

/// Identity of the dock layout, for gpui-kit's own persistence.
const DOCK_AREA_ID: &str = "cincel-main";
/// Bumped whenever the default layout changes shape.
const DOCK_AREA_VERSION: usize = 1;

/// Initial width of the chat dock (`02-visual.md` §1).
pub const CHAT_WIDTH: f32 = 380.;
/// Initial width of the file tree dock.
pub const TREE_WIDTH: f32 = 240.;
/// Status bar height (`02-visual.md` §4).
const STATUS_BAR_HEIGHT: f32 = 26.;
/// Width of the review panel popover.
const REVIEW_PANEL_WIDTH: f32 = 460.;

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
    /// The file tree's "Mencionar en el chat" context-menu item was used:
    /// stage `path` as an `@` chip (the fallback for a tree drag source,
    /// which `gpui-kit`'s tree does not have, see `docs/etapas/etapa-2.md`).
    MentionFile {
        /// Absolute path.
        path: PathBuf,
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
    /// Informational notices from startup (currently just the XDG directory
    /// migration, `docs/specs/06-etapa4-conexiones-y-cincel.md` §7), shown as
    /// toasts once the window exists.
    pub startup_notices: Vec<String>,
    /// Which background pieces the project starts. The tests turn them off.
    pub project_options: ProjectOptions,
}

/// The root view of the application window.
pub struct Workspace {
    dock_area: Entity<DockArea>,
    /// Kept alive: the skin owns the dock's presentation settings.
    _skin: Rc<DockSkin>,
    chat: Entity<ChatPanel>,
    /// Kept alive: adapts `chat` to `gpui-kit`'s dock (`crate::panels`).
    _chat_dock: Entity<ChatDock>,
    /// Agent lifecycle: the saved connections, the active `AgentConnection`,
    /// the connection modals and the ACP event/command glue
    /// (`crate::agents`).
    agents: Entity<Agents>,
    /// The review of agent edits (`crate::review`).
    review: Entity<Review>,
    center: Entity<CenterPanel>,
    files: Entity<FilesPanel>,
    toasts: Entity<Toasts>,
    project: Option<Entity<Project>>,
    /// The quick file finder (`Ctrl+P`, `crate::file_finder`, E5-F): built
    /// once per open project (like the review panel), so it keeps its typed
    /// query and tab-activation history between presses.
    file_finder: Option<Entity<FileFinder>>,
    /// The "Nuevo archivo…" floating field (`Ctrl+N`, `crate::new_file`,
    /// E5-I): built once per open project, like the file finder.
    new_file: Option<Entity<NewFilePrompt>>,
    /// The keyboard shortcuts modal (`F1`, `crate::shortcuts_modal`, E5-H):
    /// built once for the whole window, unlike the file finder, since it
    /// does not need a project.
    shortcuts_modal: Entity<ShortcutsModal>,
    /// The full-size image viewer a click on a sent thumbnail opens
    /// (`crate::image_viewer`, E7-G): built once for the window.
    image_viewer: Entity<ImageViewer>,
    /// The "¿Guardar cambios?" dialog `workspace::quit` and the window's `×`
    /// share (D15, `crate::title_menu`, E5-I).
    quit_dialog: Option<QuitDialog>,
    /// The "Hay N cambios de agente sin decidir" dialog that quitting,
    /// closing and replacing the project ask before anything else
    /// (`crate::review_close`).
    review_close_dialog: Option<ReviewCloseDialog>,
    /// Set once the quit dialog's own buttons decide; lets
    /// `Workspace::should_close` answer `true` without asking again
    /// (`crate::title_menu`, E5-I).
    quit_confirmed: bool,
    /// Whether the title bar's menu is open, for `Workspace::is_modal_open`
    /// (§7.4, `crate::title_menu`, E5-I). A plain `Cell` because
    /// `Button::dropdown_menu`'s `on_open_change` callback is not a
    /// `cx.listener` (it never receives `&mut Self`).
    title_menu_open: Rc<Cell<bool>>,
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

        let chat_theme = crate::theme::chat_theme(cx);
        let chat_settings = crate::theme::chat_settings(cx);
        let chat = cx.new(|cx| ChatPanel::new(chat_theme, chat_settings, window, cx));
        let chat_dock = ChatDock::new(chat.clone(), cx);
        let center = CenterPanel::new(cx);
        let files = FilesPanel::new(cx);
        let toasts = cx.new(|_| Toasts::new());
        toast::set_global(&toasts, cx);
        // The shortcuts modal (E5-H): needs no project, so it is built once
        // here instead of in `open_project` like the file finder.
        let shortcuts_modal = ShortcutsModal::new(center.clone(), window, cx);
        let image_viewer = crate::image_viewer::build(cx);

        // `Ctrl+Shift+A`/`Ctrl+L` and the chat's own bindings need a real
        // window; `watch_files` doubles as "may this window use real OS
        // threads", the same signal `ProjectOptions::inert` gives every other
        // background piece (`docs/etapas/etapa-2.md`).
        let agents_background = options.project_options.watch_files;
        let review = cx.new(|cx| Review::new(&center, window, cx));
        let summary = review.read(cx).summary();
        center.update(cx, |center, cx| center.set_review_summary(summary, cx));
        files.update(cx, |files, cx| files.set_review(&review, cx));
        // The unsent comments show as tags of the chat's composer (spec 09
        // §6.7).
        review.update(cx, |review, cx| review.attach_chat(&chat, window, cx));
        let agents =
            cx.new(|cx| Agents::new(chat.clone(), review.clone(), agents_background, window, cx));

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
                    panel_handle(chat_dock.clone()),
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
        // The tree asks for files; the tab area opens them. A "Mencionar en
        // el chat" click stages the same path as an `@` chip.
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
                WorkspaceEvent::MentionFile { path } => {
                    // An image becomes an attachment, anything else stays an
                    // `@` mention (`ChatPanel::mention_or_attach`, E7-G).
                    let path = path.clone();
                    this.chat
                        .update(cx, |chat, cx| chat.mention_or_attach(path, window, cx));
                }
            },
        ));
        // A click on a thumbnail of a sent message opens it full size
        // (`crate::image_viewer`, E7-G). The other chat events belong to
        // `Agents`, which subscribes to the panel itself.
        subscriptions.push(cx.subscribe_in(
            &chat,
            window,
            |this, _, event: &cincel_chat::ChatEvent, window, cx| {
                if let cincel_chat::ChatEvent::OpenImage {
                    source,
                    name,
                    width,
                    height,
                    bytes_len,
                } = event
                {
                    this.open_image_viewer(
                        crate::image_viewer::ViewerImage {
                            source: source.clone(),
                            name: name.clone(),
                            width: *width,
                            height: *height,
                            bytes_len: *bytes_len,
                        },
                        window,
                        cx,
                    );
                }
            },
        ));

        // Everything `cincel_chat::ChatPanel` emits reaches `Agents`, which
        // subscribes to the panel itself (`crate::agents`): the ACP worker,
        // the project's buffers, the conversation store and the desktop all
        // hang off that one handler.

        // The status bar reads the active tab and the chat's connection, so
        // the workspace repaints whenever either changes (cursor, dirty dot,
        // tab switch, streaming, permission cards, …).
        subscriptions.push(cx.observe_in(&center, window, |this, center, window, cx| {
            // Without a project, closing the settings tab takes the center
            // off screen with the keyboard in it: the root view takes it back
            // so the global commands keep working (§4.1 of spec 07).
            if this.project.is_none()
                && !center.read(cx).has_items()
                && center.read(cx).focus_handle(cx).is_focused(window)
            {
                window.focus(&this.focus_handle, cx);
            }
            cx.notify();
        }));
        subscriptions.push(cx.observe(&chat, |_, _, cx| cx.notify()));
        // The connection modals float over the whole window.
        let modal = agents.read(cx).modal().clone();
        subscriptions.push(cx.observe(&modal, |_, _, cx| cx.notify()));
        // The pending counter and the review panel.
        subscriptions.push(cx.observe(&review, |_, _, cx| cx.notify()));
        // The settings tab's events, which the center passes on (§4).
        subscriptions.push(cx.subscribe_in(
            &center,
            window,
            |this, _, event: &SettingsViewEvent, window, cx| {
                this.on_settings_view_event(event, window, cx);
            },
        ));

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
                        let reported = this.update(cx, |workspace, cx| {
                            workspace.on_settings_event(event, cx);
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
                workspace
                    .review
                    .update(cx, |review, cx| review.save_now(cx));
                workspace
                    .agents
                    .update(cx, |agents, cx| agents.save_conversation_now(cx));
                async {}
            }
        })
        .detach();

        // `theme.mode = "system"` follows the desktop: the appearance is read
        // now and observed from here on (`docs/specs/modulos/settings.md`).
        let dark = is_dark(window.appearance());
        crate::settings::set_system_dark(dark, cx);
        // The desktop can answer late (Wayland reports "light" for the first
        // frames and "dark" right after): the open editors, the chat and the
        // review must follow, not just the window chrome.
        let this = cx.weak_entity();
        subscriptions.push(window.observe_window_appearance(move |window, cx| {
            let dark = is_dark(window.appearance());
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| this.follow_system_appearance(dark, cx));
            } else {
                crate::settings::set_system_dark(dark, cx);
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
            _chat_dock: chat_dock,
            agents,
            review,
            center,
            files,
            toasts,
            project: None,
            file_finder: None,
            new_file: None,
            shortcuts_modal,
            image_viewer,
            quit_dialog: None,
            review_close_dialog: None,
            quit_confirmed: false,
            title_menu_open: Rc::new(Cell::new(false)),
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
        for notice in options.startup_notices {
            toast::info(notice, cx);
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
        let opened = {
            // `CINCEL_TRACE_TIMINGS=1` (`crate::bench::TIMING_TARGET`).
            let _span = tracing::info_span!(target: "cincel::timing", "project_open").entered();
            project::open_with(path, &settings, self.project_options, cx)
        };
        let project = match opened {
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
        // A fresh finder per project (E5-F): its candidates and its
        // tab-activation history belong to this project's tabs, not the
        // previous one's.
        self.file_finder = Some(FileFinder::new(
            project.clone(),
            self.center.clone(),
            window,
            cx,
        ));
        // The "Nuevo archivo…" field (E5-I): same lifetime as the finder.
        self.new_file = Some(NewFilePrompt::new(
            project.clone(),
            self.center.clone(),
            window,
            cx,
        ));
        self.files
            .update(cx, |files, cx| files.set_project(Some(project.clone()), cx));
        self.center.update(cx, |center, cx| {
            center.set_project(Some(project.clone()), cx)
        });
        self.agents.update(cx, |agents, cx| {
            agents.set_project(Some(project), window, cx)
        });
        self.recents = Recents::load();

        let layout = WorkspaceLayout::load(&root).unwrap_or_default();
        self.apply_layout(&layout, window, cx);
        window.set_window_title(&format!(
            "{} — Cincel",
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

    /// The chat panel: "Conectar" control, transcript, composer.
    pub fn chat(&self) -> &Entity<ChatPanel> {
        &self.chat
    }

    /// The agent lifecycle controller: the active `AgentConnection`, if any,
    /// and the ACP event/command glue.
    pub fn agents(&self) -> &Entity<Agents> {
        &self.agents
    }

    /// The review of agent edits.
    pub fn review(&self) -> &Entity<Review> {
        &self.review
    }

    /// The file tree panel.
    pub fn files(&self) -> &Entity<FilesPanel> {
        &self.files
    }

    /// The quick file finder (`Ctrl+P`), once a project has built it
    /// (`crate::file_finder`, E5-F).
    pub fn file_finder(&self) -> Option<&Entity<FileFinder>> {
        self.file_finder.as_ref()
    }

    /// The "Nuevo archivo…" field (`Ctrl+N`), once a project has built it
    /// (`crate::new_file`, E5-I).
    pub fn new_file_prompt(&self) -> Option<&Entity<NewFilePrompt>> {
        self.new_file.as_ref()
    }

    /// The keyboard shortcuts modal (`F1`, `crate::shortcuts_modal`, E5-H).
    pub fn shortcuts_modal(&self) -> &Entity<ShortcutsModal> {
        &self.shortcuts_modal
    }

    /// The full-size image viewer (`crate::image_viewer`, E7-G).
    pub fn image_viewer(&self) -> &Entity<ImageViewer> {
        &self.image_viewer
    }

    /// The recently opened projects, most recent first (`crate::title_menu`).
    pub(crate) fn recents_snapshot(&self) -> &[PathBuf] {
        self.recents.projects()
    }

    /// Replaces the recent-projects list and persists it to disk
    /// (`crate::title_menu`).
    pub(crate) fn set_recents(&mut self, recents: Recents) {
        self.recents = recents;
        if let Err(error) = self.recents.save() {
            tracing::warn!(%error, "no se pudo guardar la lista de proyectos recientes");
        }
    }

    /// The shared flag `crate::title_menu`'s menu button flips through
    /// `Button::dropdown_menu`'s `on_open_change`.
    pub(crate) fn title_menu_open_flag(&self) -> Rc<Cell<bool>> {
        self.title_menu_open.clone()
    }

    /// Whether the "¿Guardar cambios?" quit dialog is up (`crate::title_menu`).
    pub fn is_quit_dialog_open(&self) -> bool {
        self.quit_dialog.is_some()
    }

    pub(crate) fn set_quit_dialog(&mut self, dialog: Option<QuitDialog>) {
        self.quit_dialog = dialog;
    }

    /// How many files the open quit dialog is asking about, or `None`
    /// without one (`crate::title_menu`'s `render_quit_dialog`).
    pub(crate) fn quit_dialog_dirty_count(&self) -> Option<usize> {
        self.quit_dialog.as_ref().map(QuitDialog::dirty_count)
    }

    /// Takes the focus the quit dialog is meant to restore, or `None`
    /// without a dialog open.
    pub(crate) fn take_quit_dialog_previous_focus(&mut self) -> Option<Option<FocusHandle>> {
        self.quit_dialog.take().map(QuitDialog::into_previous_focus)
    }

    /// The open "cambios de agente sin decidir" dialog, if any
    /// (`crate::review_close`).
    pub(crate) fn review_close_dialog(&self) -> Option<&ReviewCloseDialog> {
        self.review_close_dialog.as_ref()
    }

    pub(crate) fn review_close_dialog_mut(&mut self) -> Option<&mut ReviewCloseDialog> {
        self.review_close_dialog.as_mut()
    }

    pub(crate) fn set_review_close_dialog(&mut self, dialog: Option<ReviewCloseDialog>) {
        self.review_close_dialog = dialog;
    }

    pub(crate) fn is_quit_confirmed(&self) -> bool {
        self.quit_confirmed
    }

    pub(crate) fn set_quit_confirmed(&mut self, confirmed: bool) {
        self.quit_confirmed = confirmed;
    }

    /// The toast queue.
    pub fn toasts(&self) -> &Entity<Toasts> {
        &self.toasts
    }

    /// Opens the chat's "Conectar" popover, expanding the chat dock first if
    /// it was collapsed (the status bar chip).
    pub fn open_connections(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dock_area.update(cx, |area, cx| {
            if !area.is_dock_open(DockPlacement::Left) {
                area.toggle_dock(DockPlacement::Left, window, cx);
            }
        });
        self.chat.update(cx, |chat, cx| chat.open_connections(cx));
        cx.notify();
    }

    /// `workspace::open_settings` (`Ctrl+,`) and the menu's "Conexiones"
    /// (`workspace::open_connections`, D7): opens or activates the settings
    /// tab, on `section` when given (`docs/specs/07-etapa5-productividad.md`
    /// §4.1, §8.2). Works without a project too: the tab then takes the
    /// place of the empty screen.
    pub fn open_settings_section(
        &mut self,
        section: Option<SettingsSection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self
            .center
            .update(cx, |center, cx| center.open_settings(section, window, cx));
        self.agents
            .update(cx, |agents, cx| agents.attach_settings_view(&view, cx));
        cx.notify();
    }

    /// What the settings tab says: a key it wrote is applied at once, the
    /// Connections section is `Agents`' business.
    fn on_settings_view_event(
        &mut self,
        event: &SettingsViewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SettingsViewEvent::Written => {
                let reloaded =
                    crate::settings::reload(cincel_settings::SettingsEvent::SettingsChanged, cx);
                if reloaded.changed {
                    // The tab is the one who asked: no "Configuración
                    // recargada" (§4.3). The watcher's event that follows
                    // has the same fingerprint and is ignored.
                    self.apply_reloaded(&reloaded, false, cx);
                }
            }
            SettingsViewEvent::OpenSettingsFile => {}
            other => self.agents.update(cx, |agents, cx| {
                agents.handle_settings_event(other, window, cx)
            }),
        }
    }

    /// One event of the configuration watcher: re-read the document and
    /// apply it, with the "Configuración recargada" toast.
    pub(crate) fn on_settings_event(&mut self, event: SettingsEvent, cx: &mut Context<Self>) {
        let reloaded = crate::settings::reload(event, cx);
        if reloaded.changed {
            self.apply_reloaded(&reloaded, true, cx);
        }
    }

    /// Records the desktop's appearance and, when it changed, hands the
    /// theme now in force to every open editor, the chat and the Markdown
    /// previews. Returns whether it changed.
    pub(crate) fn follow_system_appearance(&mut self, dark: bool, cx: &mut Context<Self>) -> bool {
        if !crate::settings::set_system_dark(dark, cx) {
            return false;
        }
        self.refresh_theme_everywhere(cx);
        cx.refresh_windows();
        true
    }

    /// Hands the theme and settings in force to the editors (code font,
    /// colors, grammar), the Markdown previews and the chat.
    fn refresh_theme_everywhere(&mut self, cx: &mut Context<Self>) {
        self.center
            .update(cx, |center, cx| center.refresh_editor_style(cx));
        let chat_theme = crate::theme::chat_theme(cx);
        let chat_settings = crate::theme::chat_settings(cx);
        self.chat.update(cx, |chat, cx| {
            chat.set_theme(chat_theme, cx);
            chat.set_settings(chat_settings, cx);
        });
    }

    /// Hands what a reload changed to everything that caches settings: the
    /// open editors (settings, fonts, colours; the element has no globals),
    /// the review, the agents and the chat. Problems are always reported;
    /// `notify` adds "Configuración recargada" when there were none (the
    /// watcher does, the settings tab does not).
    pub(crate) fn apply_reloaded(
        &mut self,
        reloaded: &Reloaded,
        notify: bool,
        cx: &mut Context<Self>,
    ) {
        self.refresh_theme_everywhere(cx);
        self.review
            .update(cx, |review, cx| review.reload_settings(cx));
        self.agents
            .update(cx, |agents, cx| agents.reload_settings(cx));
        for issue in &reloaded.issues {
            toast::warn(issue.to_string(), cx);
        }
        if notify && reloaded.issues.is_empty() {
            toast::info_keyed("settings-reloaded", "Configuración recargada", cx);
        }
        cx.notify();
    }

    /// The label the status bar shows for the connection ("Sin conexión"
    /// when none is active).
    pub fn status_connection_label(&self, cx: &App) -> String {
        self.chat.read(cx).active_connection().map_or_else(
            || "Sin conexión".to_string(),
            |connection| connection.label.clone(),
        )
    }

    /// Whether the dock at `placement` is expanded.
    pub fn is_dock_open(&self, placement: DockPlacement, cx: &App) -> bool {
        self.dock_area.read(cx).is_dock_open(placement)
    }

    /// Width of the dock at `placement` (what dragging its border changes).
    pub fn dock_width(&self, placement: DockPlacement, cx: &App) -> Option<Pixels> {
        self.dock_area.read(cx).dock_size(placement)
    }

    /// Resizes the dock at `placement`, as dragging its border does.
    pub fn set_dock_width(
        &mut self,
        placement: DockPlacement,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dock_area.update(cx, |area, cx| {
            area.set_dock_size(placement, width, window, cx)
        });
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
                app_id: Some("dev.cincel.Cincel".into()),
                // Client-side decorations: the title bar is ours to draw
                // (`02-visual.md` §1). The transparent background is what
                // lets gpui-kit paint the rounded border and the shadow.
                #[cfg(target_os = "linux")]
                window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
                #[cfg(target_os = "linux")]
                window_decorations: Some(match decorations {
                    cincel_settings::Decorations::Server => gpui_kit::WindowDecorations::Server,
                    cincel_settings::Decorations::Client => gpui_kit::WindowDecorations::Client,
                }),
                ..TitleBar::window_options()
            };

            let window = cx.open_window(window_options, |window, cx| {
                let workspace = cx.new(|cx| Workspace::new(options, window, cx));
                cx.new(|cx| Root::new(workspace, window, cx))
            })?;

            window.update(cx, |root, window, cx| {
                window.set_window_title("Cincel");
                window.activate_window();
                report_gpu(window);

                // `×` (D15, `docs/specs/07-etapa5-productividad.md` §8.2):
                // the same question `workspace::quit` asks
                // (`Workspace::should_close`), which opens the "¿Guardar
                // cambios?" dialog instead of closing when there are unsaved
                // files. Closing the only window ends the application (see
                // `cx.on_release` below), and the geometry has to be on disk
                // before that happens.
                // A weak handle: the window owns this callback and the
                // workspace lives in the window, so a strong one would keep
                // the workspace alive past the window (leaked handle).
                let workspace = root
                    .view()
                    .clone()
                    .downcast::<Workspace>()
                    .ok()
                    .map(|workspace| workspace.downgrade());
                // The title bar's own `×` takes the same road
                // (`Workspace::close_from_title_bar`).
                window.on_window_should_close(cx, move |window, cx| {
                    let Some(workspace) = workspace.as_ref().and_then(|w| w.upgrade()) else {
                        return true;
                    };
                    workspace.update(cx, |workspace, cx| workspace.request_close(window, cx))
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
                // Pending agent changes of the project being left are
                // decided first (`crate::review_close`).
                this.request_open_project(&path, window, cx);
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
        self.toggle_focus(FocusZone::Chat, window, cx);
    }

    fn on_toggle_tree(
        &mut self,
        _: &actions::ToggleTree,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_focus(FocusZone::Files, window, cx);
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
        // The composer itself, not just the panel.
        self.chat
            .update(cx, |chat, cx| chat.focus_input(window, cx));
    }

    fn on_focus_next_zone(
        &mut self,
        _: &actions::FocusNextZone,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_next_zone(window, cx);
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

    /// Sets the interface zoom and makes every size follow it: gpui-kit's
    /// (rems), the code font of the open editors, the chat panel (its type
    /// scale and pixel sizes, through `ChatSettings::scale`), and the pixel
    /// sizes of the tree, the tabs, the breadcrumb and the status bar (read
    /// from `crate::settings::ui_scale` when they render).
    pub(crate) fn zoom(&mut self, scale: f32, window: &mut Window, cx: &mut Context<Self>) {
        let scale = crate::settings::set_ui_scale(scale, cx);
        // Every gpui-kit size is expressed in rems, so the root size is what
        // actually zooms the interface; the theme carries the font sizes.
        window.set_rem_size(px(16. * scale));
        // The code font zooms with the interface.
        self.center
            .update(cx, |center, cx| center.refresh_editor_style(cx));
        // The chat paints in pixels, not rems: it gets the factor itself.
        let chat_settings = crate::theme::chat_settings(cx);
        self.chat
            .update(cx, |chat, cx| chat.set_settings(chat_settings, cx));
        cx.refresh_windows();
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

    /// `workspace::next_change` / `prev_change` / `next_file_with_changes`.
    fn navigate(&mut self, nav: Nav, window: &mut Window, cx: &mut Context<Self>) {
        self.review
            .update(cx, |review, cx| review.navigate(nav, None, window, cx));
    }

    /// Expands or collapses the dock at `placement`, leaving the focus alone.
    pub(crate) fn toggle_dock(
        &mut self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        // Every pixel size follows the zoom (`workspace::zoom_*`).
        let scale = crate::settings::ui_scale(cx);
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
                        crate::logo::logo(px(48. * scale))
                            .debug_selector(|| "empty-state-logo".to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(22. * scale))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child("Cincel"),
                    ),
            )
            .child(
                // The same neutral button as the dialogs (`02-visual.md` §4).
                div()
                    .id("empty-open-folder")
                    .h(px(28. * scale))
                    .px(px(12. * scale))
                    .flex()
                    .items_center()
                    .rounded(px(4. * scale))
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
                                .rounded(px(4. * scale))
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
        // Every pixel size follows the zoom (`workspace::zoom_*`).
        let scale = crate::settings::ui_scale(cx);
        let center = self.center.read(cx);
        let settings_active = center.is_settings_active();
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
        // The Markdown preview has no cursor of its own to report
        // (`docs/specs/02-visual.md` §1).
        let position_label = if tab.is_some_and(crate::center::Tab::is_showing_markdown_preview) {
            SharedString::from("Vista previa")
        } else {
            let (line, column) = tab.map(|tab| tab.cursor_line_column()).unwrap_or((1, 1));
            SharedString::from(format!("Ln {line}, Col {column}"))
        };
        let read_only = tab.is_some_and(|tab| tab.read_only);
        let zoom = (crate::settings::ui_scale(cx) * 100.).round() as i32;
        let chat = self.chat.read(cx);
        let (pending, sweeping, comments) = {
            let summary = self.review.read(cx).summary();
            let summary = summary.borrow();
            (summary.pending, summary.sweeping, summary.comments)
        };
        // The chip of the active connection (`docs/specs/06-etapa4-conexiones-
        // y-cincel.md` §6): provider icon + label, or "Sin conexión".
        let connection_chip = match chat.active_connection() {
            Some(connection) => h_flex()
                .gap_1()
                .items_center()
                .child(cincel_chat::provider_icon(
                    &connection.agent_id,
                    px(14. * scale),
                    theme.text,
                    theme.bg_surface,
                    theme.border,
                ))
                .child(SharedString::from(connection.label.clone()))
                .into_any_element(),
            None => div()
                .child(SharedString::from("Sin conexión"))
                .into_any_element(),
        };

        StatusBar::new()
            .h(px(STATUS_BAR_HEIGHT * scale))
            .flex_none()
            .py_0()
            .left(if settings_active {
                // The settings tab has no cursor, encoding nor language
                // (`docs/specs/07-etapa5-productividad.md` §4.1).
                h_flex()
                    .gap_3()
                    .text_color(theme.text_muted)
                    .child(crate::settings_view::TAB_TITLE)
            } else {
                h_flex()
                    .gap_3()
                    .text_color(theme.text_muted)
                    .child(name)
                    .child(position_label)
                    .child(SharedString::from("UTF-8"))
                    .child(SharedString::from(eol))
                    // Clicking the language will open the language picker in a
                    // later stage; today it is only a label.
                    .child(div().id("status-language").child(language))
                    .children(read_only.then(|| SharedString::from("Solo lectura")))
            })
            .right(
                h_flex()
                    .gap_3()
                    .text_color(theme.text_muted)
                    // The active connection; clicking it opens the chat's
                    // "Conectar" popover (and the chat dock, if it was
                    // collapsed).
                    .child(
                        div()
                            .id("status-connection")
                            .debug_selector(|| "status-connection".to_string())
                            .cursor_pointer()
                            .child(connection_chip)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.open_connections(window, cx);
                            })),
                    )
                    // Clicking the pending counter opens the review panel
                    // (`02-visual.md` §1).
                    .child(
                        div()
                            .id("status-pending")
                            .cursor_pointer()
                            .when(pending > 0, |this| this.text_color(theme.status_warning))
                            // A long end-of-turn sweep says so in the same
                            // place (`crate::review::SWEEP_NOTICE`).
                            .debug_selector(|| "status-pending".to_string())
                            .child(SharedString::from(if sweeping {
                                crate::review::SWEEP_NOTICE.to_string()
                            } else {
                                // "3 cambios pendientes · 2 comentarios"
                                // (spec 09 §6.2.9).
                                crate::review::status_label(pending, comments)
                            }))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.review.update(cx, |review, cx| review.toggle_panel(cx));
                            })),
                    )
                    // The shortcuts modal button (D5, `crate::shortcuts_modal`,
                    // E5-H): before the zoom label, like the other clickable
                    // status bar controls.
                    .child(
                        div()
                            .id("status-shortcuts")
                            .cursor_pointer()
                            .text_color(theme.text_muted)
                            .hover(|style| style.text_color(theme.text))
                            .tooltip(|window, cx| {
                                Tooltip::new("Atajos de teclado (F1)").build(window, cx)
                            })
                            .child(
                                div()
                                    .w(px(14. * scale))
                                    .flex_none()
                                    .child(gpui_kit::assets::IconName::Keyboard),
                            )
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.show_shortcuts(window, cx);
                            })),
                    )
                    .child(SharedString::from(format!("{zoom} %"))),
            )
    }
}

impl Workspace {
    /// The review panel (`02-visual.md` §6.3): a popover anchored to the
    /// status bar, over a scrim that closes it.
    fn render_review_panel(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let review = self.review.read(cx);
        if !review.is_panel_open() {
            return None;
        }
        let rows = review.panel_rows();
        let theme = ThemeColors::global(cx).clone();
        // Every pixel size follows the zoom (`workspace::zoom_*`).
        let scale = crate::settings::ui_scale(cx);

        let header = h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child("Revisión"),
            )
            .child(
                Button::new("review-accept-all")
                    .label("Aceptar todo (Ctrl+Alt+↵)")
                    .small()
                    .outline()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.review.update(cx, |review, cx| review.accept_all(cx));
                    })),
            )
            .child(
                Button::new("review-reject-all")
                    .label("Rechazar todo (Ctrl+Alt+⌫)")
                    .small()
                    .outline()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.review.update(cx, |review, cx| review.reject_all(cx));
                    })),
            );

        let list = v_flex().gap_0p5().children(rows.into_iter().enumerate().map(|(index, row)| {
            let pending = row.state == PanelState::Pending;
            let tag = if row.created {
                "nuevo"
            } else if row.deleted {
                "eliminado"
            } else if pending {
                "pendiente"
            } else {
                "decidido"
            };
            let open_path = row.path.clone();
            let accept_path = row.path.clone();
            let reject_path = row.path.clone();
            v_flex()
                .child(
                    h_flex()
                        .id(("review-row", index))
                        .gap_2()
                        .px_2()
                        .h(px(26. * scale))
                        .items_center()
                        .rounded(px(4. * scale))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.bg_surface))
                        .child(
                            div()
                                .w(px(16. * scale))
                                .flex_none()
                                .text_color(if row.created {
                                    theme.status_ok
                                } else if pending {
                                    theme.status_warning
                                } else {
                                    theme.text_muted
                                })
                                .child(gpui_kit::assets::IconName::File),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_color(if pending { theme.text } else { theme.text_muted })
                                .child(SharedString::from(row.relative.clone())),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(11. * scale))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(stats_label(row.added, row.removed))),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px_1()
                                .rounded(px(4. * scale))
                                .bg(theme.bg_surface)
                                .text_size(px(11. * scale))
                                .text_color(theme.text_muted)
                                .child(tag),
                        )
                        .when(pending, |this| {
                            this.child(
                                Button::new(("review-row-accept", index))
                                    .label("✓")
                                    .xsmall()
                                    .ghost()
                                    .tooltip("Aceptar el archivo")
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        // The row under the button opens the file.
                                        cx.stop_propagation();
                                        let path = accept_path.clone();
                                        this.review
                                            .update(cx, |review, cx| review.accept_file(&path, cx));
                                    })),
                            )
                            .child(
                                Button::new(("review-row-reject", index))
                                    .label("✗")
                                    .xsmall()
                                    .ghost()
                                    .tooltip("Rechazar el archivo")
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        // The row under the button opens the file.
                                        cx.stop_propagation();
                                        let path = reject_path.clone();
                                        this.review
                                            .update(cx, |review, cx| review.reject_file(&path, cx));
                                    })),
                            )
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            let path = open_path.clone();
                            this.review.update(cx, |review, cx| {
                                review.open_file_at_first_hunk(&path, window, cx)
                            });
                        })),
                )
                .when(row.too_large, |this| {
                    this.child(
                        div()
                            .pl(px(32. * scale))
                            .text_size(px(11. * scale))
                            .text_color(theme.text_muted)
                            .child("Demasiado grande para revisar por segmentos: solo el archivo entero"),
                    )
                })
                .when(row.no_copy, |this| {
                    this.child(
                        div()
                            .pl(px(32. * scale))
                            .text_size(px(11. * scale))
                            .text_color(theme.text_muted)
                            .child("Cincel no guardó una copia previa: solo se puede aceptar"),
                    )
                })
        }));

        let empty = review.summary().borrow().files.is_empty() && review.panel_rows().is_empty();
        Some(
            div()
                .absolute()
                .inset_0()
                .child(div().id("review-scrim").absolute().inset_0().on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| {
                        this.review
                            .update(cx, |review, cx| review.set_panel_open(false, cx));
                    }),
                ))
                .child(
                    v_flex()
                        .id("review-panel")
                        .absolute()
                        .right(px(12. * scale))
                        .bottom(px((STATUS_BAR_HEIGHT + 4.) * scale))
                        .w(px(REVIEW_PANEL_WIDTH * scale))
                        .max_h(px(420. * scale))
                        .overflow_y_scroll()
                        .p_3()
                        .gap_2()
                        .rounded(px(6. * scale))
                        .bg(theme.bg_elevated)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_lg()
                        .text_sm()
                        // Clicks inside the popover must not reach the scrim.
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(header)
                        .when(empty, |this| {
                            this.child(
                                div()
                                    .text_color(theme.text_muted)
                                    .child("No hay cambios pendientes"),
                            )
                        })
                        .child(list),
                ),
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
        // `cincel --bench` (`crate::bench`): start of the frame; a no-op
        // without `--bench`.
        crate::bench::frame_begin();
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
                SharedString::from(format!("Cincel — {}", project.read(cx).root().display()))
            }
            None => SharedString::from("Cincel"),
        };

        let body = if has_project {
            div()
                .flex_1()
                .min_h_0()
                .child(self.dock_area.clone())
                .into_any_element()
        } else if self.center.read(cx).has_items() {
            // The settings tab without a project: the empty screen gives the
            // center area to it until it closes (§4.1).
            div()
                .flex_1()
                .min_h_0()
                .child(self.center.clone())
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
            .on_action(cx.listener(Self::on_focus_next_zone))
            // E5-F: the quick file finder (`crate::file_finder`).
            .on_action(
                cx.listener(|this, _: &actions::ToggleFileFinder, window, cx| {
                    this.toggle_file_finder(window, cx)
                }),
            )
            // E5-G: the settings tab.
            .on_action(cx.listener(|this, _: &actions::OpenSettings, window, cx| {
                this.open_settings_section(None, window, cx)
            }))
            // E5-H: the shortcuts modal.
            .on_action(cx.listener(|this, _: &actions::ShowShortcuts, window, cx| {
                this.show_shortcuts(window, cx)
            }))
            // E5-I: "Nuevo archivo" (`crate::new_file`).
            .on_action(cx.listener(|this, _: &actions::NewFile, window, cx| {
                this.open_new_file_prompt(window, cx)
            }))
            // E5-I: "Salir", with the unsaved-files dialog
            // (`crate::title_menu`).
            .on_action(
                cx.listener(|this, _: &actions::Quit, window, cx| this.request_quit(window, cx)),
            )
            // E5-G / E5-I: the settings tab on its Connections section.
            .on_action(
                cx.listener(|this, _: &actions::OpenConnections, window, cx| {
                    this.open_settings_section(Some(SettingsSection::Connections), window, cx)
                }),
            )
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_save_all))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_reset))
            .on_action(cx.listener(|this, _: &actions::NextChange, window, cx| {
                this.navigate(Nav::Next, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::PrevChange, window, cx| {
                this.navigate(Nav::Prev, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &actions::NextFileWithChanges, window, cx| {
                    this.navigate(Nav::NextFile, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &actions::AcceptTurn, _, cx| {
                this.review.update(cx, |review, cx| review.accept_turn(cx))
            }))
            .on_action(cx.listener(|this, _: &actions::RejectTurn, _, cx| {
                this.review.update(cx, |review, cx| review.reject_turn(cx))
            }))
            .on_action(cx.listener(|this, _: &actions::UndoLastReject, _, cx| {
                this.review
                    .update(cx, |review, cx| review.undo_last_reject(cx))
            }))
            .on_action(cx.listener(|this, _: &actions::OpenReviewPanel, _, cx| {
                this.review.update(cx, |review, cx| review.toggle_panel(cx))
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg_app)
            .text_color(theme.text)
            .child(self.render_title_bar(title, window, cx))
            .child(body)
            .child(self.render_status_bar(cx))
            .children(self.render_review_panel(cx))
            .children({
                let modal = self.agents.read(cx).modal().clone();
                modal.read(cx).is_open().then_some(modal)
            })
            // The quick file finder floats over everything else, like the
            // modal above (`crate::file_finder`, E5-F).
            .children(
                self.file_finder
                    .as_ref()
                    .filter(|finder| finder.read(cx).is_open())
                    .cloned(),
            )
            // The "Nuevo archivo…" field floats over everything else too
            // (`crate::new_file`, E5-I).
            .children(
                self.new_file
                    .as_ref()
                    .filter(|prompt| prompt.read(cx).is_open())
                    .cloned(),
            )
            // The shortcuts modal floats over everything else too
            // (`crate::shortcuts_modal`, E5-H).
            .children(
                Some(&self.shortcuts_modal)
                    .filter(|modal| modal.read(cx).is_open())
                    .cloned(),
            )
            // The full-size image viewer floats over everything but the
            // dialogs and the toasts (`crate::image_viewer`, E7-G).
            .children(
                Some(&self.image_viewer)
                    .filter(|viewer| viewer.read(cx).is_open())
                    .cloned(),
            )
            // The "¿Guardar cambios?" quit dialog (D15, `crate::title_menu`,
            // E5-I).
            .children(self.render_quit_dialog(cx))
            // "Hay N cambios de agente sin decidir" and the bar's "¿Rechazar
            // todo el turno?" (`crate::review_close`).
            .children(self.render_review_close_dialog(cx))
            .children(self.render_reject_turn_dialog(cx))
            // Toasts float over the editor area, bottom center
            // (`02-visual.md` §6.5). "Editor area" is the center of the dock,
            // so the two side docks are subtracted from the band.
            .child(
                div()
                    .absolute()
                    .bottom(px((STATUS_BAR_HEIGHT + 16.) * crate::settings::ui_scale(cx)))
                    .left(px(dock_inset.0))
                    .right(px(dock_inset.1))
                    .flex()
                    .justify_center()
                    .child(self.toasts.clone()),
            )
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
            // `cincel --bench`: the element whose paint marks the end of the
            // frame (`crate::bench::FrameProbe`); absent without `--bench`.
            .children(crate::bench::frame_probe())
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
/// rasterizer unless `CINCEL_ALLOW_SOFTWARE_GPU=1`
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
        let allowed = std::env::var("CINCEL_ALLOW_SOFTWARE_GPU").as_deref() == Ok("1");
        if allowed {
            tracing::warn!(
                "sin GPU: se está usando un rasterizador por software ({}); el rendimiento va a ser malo",
                specs.device_name
            );
        } else {
            tracing::error!(
                "no se encontró una GPU utilizable (adaptador por software: {}). \
                 Ejecutá con CINCEL_ALLOW_SOFTWARE_GPU=1 para seguir igual.",
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
