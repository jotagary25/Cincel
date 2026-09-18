//! The root view: integrated title bar, three-panel dock, status bar.
//!
//! See `docs/specs/02-visual.md` §1 and `docs/specs/modulos/workspace.md`.
//! Stage E0-a builds the frame only: the panels are placeholders and the
//! status bar shows fixed text.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::component::dock::{DockArea, DockLayout, DockPlacement, DockSkin, panel_handle};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{ActiveTheme as _, Root, TitleBar, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, FontWeight, KeyBinding, Task, Window,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, div, px,
};

use crate::panels::{CenterPanel, ChatPanel, FilesPanel};
use crate::theme::Theme;
use crate::window_state::{self, WindowState};

gpui_kit::actions!(workspace, [ToggleChat, ToggleTree]);

/// Identity of the dock layout, for the persisted layout of later stages.
const DOCK_AREA_ID: &str = "asteroid-main";
/// Bumped whenever the default layout changes shape.
const DOCK_AREA_VERSION: usize = 1;

/// Initial width of the chat dock (`02-visual.md` §1).
const CHAT_WIDTH: f32 = 380.;
/// Initial width of the file tree dock.
const TREE_WIDTH: f32 = 240.;
/// Status bar height (`02-visual.md` §4).
const STATUS_BAR_HEIGHT: f32 = 26.;

/// Registers actions, key bindings and the theme. Call once, after
/// `gpui_kit::init`.
pub fn init(cx: &mut App) {
    crate::theme::apply_to_kit(Theme::asteroid_dark(), cx);

    cx.bind_keys([
        KeyBinding::new("ctrl-shift-a", ToggleChat, None),
        KeyBinding::new("ctrl-shift-e", ToggleTree, None),
    ]);
}

/// How many frames the workspace has rendered, shared with the caller so a
/// smoke test can assert that the window actually drew something.
pub type FrameCounter = Arc<AtomicUsize>;

/// The root view of the application window.
pub struct Workspace {
    dock_area: Entity<DockArea>,
    /// Kept alive: the skin owns the dock's presentation settings.
    _skin: Rc<DockSkin>,
    focus_handle: FocusHandle,
    frames: FrameCounter,
    /// Latest geometry seen while rendering, so quitting can persist it
    /// without touching the window (which is already gone by then).
    last_bounds: Rc<Cell<Option<WindowState>>>,
}

impl Workspace {
    /// Builds the workspace and its default three-panel layout.
    pub fn new(frames: FrameCounter, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (dock_area, skin) =
            DockSkin::dock_area(DOCK_AREA_ID, Some(DOCK_AREA_VERSION), window, cx);
        // `PanelStyle::Auto` (the default) draws a plain title strip for a
        // group holding a single panel, which is what the three panels of
        // this stage are: "Chat" and "Archivos" get their titles, and the
        // center — whose title is deliberately empty — gets the empty tab
        // strip the editor will fill in stage 1.

        let chat = ChatPanel::new(cx);
        let center = CenterPanel::new(cx);
        let files = FilesPanel::new(cx);

        dock_area.update(cx, |area, cx| {
            area.set_center(
                DockLayout::v_split().child(
                    DockLayout::tabs().panel_view(panel_handle(center), cx),
                    None,
                ),
                window,
                cx,
            );

            for (placement, panel, width) in [
                (DockPlacement::Left, panel_handle(chat), px(CHAT_WIDTH)),
                (DockPlacement::Right, panel_handle(files), px(TREE_WIDTH)),
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

        let last_bounds = Rc::new(Cell::new(None));
        cx.on_app_quit({
            let last_bounds = last_bounds.clone();
            move |_: &mut Self, _: &mut Context<Self>| {
                save_window_state(&last_bounds);
                async {}
            }
        })
        .detach();

        // GPUI dispatches an action along the focus path, so the global
        // commands (`Ctrl+Shift+A`, `Ctrl+Shift+E`) only reach the handlers
        // below once something inside the workspace holds focus. Take it at
        // startup; clicking a panel later moves focus to a descendant, which
        // still bubbles here.
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        Self {
            dock_area,
            _skin: skin,
            focus_handle,
            frames,
            last_bounds,
        }
    }

    /// Opens the main window with the workspace inside it.
    ///
    /// The returned task resolves once the window exists; a failure here means
    /// GPUI could not get a surface at all (see `03-arquitectura.md` §7).
    pub fn open_window(
        frames: FrameCounter,
        cx: &mut App,
    ) -> Task<anyhow::Result<WindowHandle<Root>>> {
        let window_bounds = window_state::initial_window_bounds(cx);

        cx.spawn(async move |cx| {
            let options = WindowOptions {
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
                window_decorations: Some(gpui_kit::WindowDecorations::Client),
                ..TitleBar::window_options()
            };

            let window = cx.open_window(options, |window, cx| {
                let workspace = cx.new(|cx| Workspace::new(frames, window, cx));
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

    fn on_toggle_chat(&mut self, _: &ToggleChat, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle(DockPlacement::Left, window, cx);
    }

    fn on_toggle_tree(&mut self, _: &ToggleTree, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle(DockPlacement::Right, window, cx);
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
}

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

        let theme = *Theme::global(cx);
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);

        div()
            .id("workspace")
            .track_focus(&self.focus_handle)
            .key_context("Workspace")
            .on_action(cx.listener(Self::on_toggle_chat))
            .on_action(cx.listener(Self::on_toggle_tree))
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
                        .child("Asteroid"),
                ),
            )
            .child(div().flex_1().min_h_0().child(self.dock_area.clone()))
            .child(
                StatusBar::new()
                    .h(px(STATUS_BAR_HEIGHT))
                    .flex_none()
                    .py_0()
                    .left(h_flex().gap_2().child("Sin proyecto"))
                    .right(
                        h_flex()
                            .gap_2()
                            .text_color(cx.theme().muted_foreground)
                            .child("Sin agente"),
                    ),
            )
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
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
/// `Backends::VULKAN | Backends::GL` hard-coded (no `WGPU_BACKEND` and no
/// other env var is read), enumerates every adapter of both backends, and
/// sorts them by device type and then by backend before testing each one
/// against the real surface. So the cascade already happens inside GPUI, and
/// the only knob it exposes is `ZED_DEVICE_ID=0x1234`, which pins one PCI
/// device. What is left for us is the last rung: detecting that the winner is
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
