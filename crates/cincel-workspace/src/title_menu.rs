//! The title bar's menu, recent-folder submenu, panel toggle buttons and the
//! "¿Guardar cambios?" quit dialog (`docs/specs/07-etapa5-productividad.md`
//! §8, E5-I).
//!
//! `Workspace::render` (`crate::workspace`) delegates the whole title bar to
//! [`Workspace::render_title_bar`], built here so the sub-stage's new code
//! lands in a file of its own rather than growing `workspace.rs` (the other
//! E5-J sub-stage touches that file too, in parallel).
//!
//! The menu button (`IconName::Menu`, styled like Zed) opens a
//! `gpui_kit::component::menu::PopupMenu` through `Button::dropdown_menu`
//! (D nothing new: the same mechanism gpui-kit ships for any dropdown).
//! Its `action_context` is the focus handle of the active file tab's editor,
//! or the workspace's own handle without one, so the shown shortcuts and the
//! dispatched actions match whatever has the keyboard right now (§8.2).
//!
//! "Salir" (`Ctrl+Q`) and the window's `×` both fall into
//! [`Workspace::should_close`] (D15): with unsaved files and no confirmation
//! yet it opens the dialog below and answers `false`; otherwise it answers
//! `true` and the caller (the window's own close hook, or `request_quit` for
//! `Ctrl+Q`) may proceed.

use std::path::{Path, PathBuf};

use cincel_project::Recents;
use gpui::{App, ClickEvent, Context, FocusHandle, Focusable, Window};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{TitleBar, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, SharedString, assets::IconName, div, px};

use crate::actions;
use crate::focus::FocusZone;
use crate::theme::ThemeColors;
use crate::workspace::Workspace;

/// `quit_dialog::confirm_save_all`: "Guardar todo y salir" (`Enter`).
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = quit_dialog, name = "confirm_save_all")]
pub struct ConfirmSaveAll;
/// `quit_dialog::cancel`: "Cancelar" (`Esc`).
#[derive(Clone, Copy, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = quit_dialog, name = "cancel")]
pub struct CancelQuit;

/// The GPUI key context of the quit dialog; `keymap.rs` binds `Enter` and
/// `Esc` against it, like the file finder and the shortcuts modal.
pub const QUIT_KEY_CONTEXT: &str = "QuitDialog";

/// Same neutral width as the tab-close and agent-write dialogs
/// (`crate::center`'s own `DIALOG_WIDTH`, duplicated here: a `pub(crate)`
/// constant in that file would be one more cross-sub-stage edit for a single
/// number two files already agree on by eye).
const DIALOG_WIDTH: f32 = 420.;
const DIALOG_BUTTON_HEIGHT: f32 = 28.;
/// Title bar buttons: 28×28 px, ghost, `docs/specs/07-etapa5-…md` §8.1.
const TITLE_BUTTON_SIZE: f32 = 28.;
const TITLE_BUTTON_ICON_SIZE: f32 = 16.;

/// What [`Workspace::open_quit_dialog`] remembers while the dialog of §8.2 is
/// up: how many files it is asking about, and where the keyboard was so
/// "Cancelar" can give it back.
pub(crate) struct QuitDialog {
    dirty_count: usize,
    previous_focus: Option<FocusHandle>,
}

impl QuitDialog {
    /// How many files the dialog is asking about.
    pub(crate) fn dirty_count(&self) -> usize {
        self.dirty_count
    }

    /// The focus handle to restore once the dialog closes.
    pub(crate) fn into_previous_focus(self) -> Option<FocusHandle> {
        self.previous_focus
    }
}

/// `path` with the user's home directory folded to `~` (§8.2: "etiqueta =
/// ruta con `~` para el home"). Pure: the caller supplies `home` so this is
/// testable without reading the real environment.
pub fn recent_label_with_home(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        return if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.display())
        };
    }
    path.display().to_string()
}

/// [`recent_label_with_home`] against the real home directory.
fn recent_label(path: &Path) -> String {
    recent_label_with_home(path, dirs::home_dir().as_deref())
}

/// One button of the quit dialog: same look as `crate::center`'s
/// `dialog_button` (surface, one pixel of border, radius 4, 28 px tall) —
/// none of "Guardar todo y salir", "Salir sin guardar" or "Cancelar" is more
/// dangerous than the others, so only the `Enter` default gets the focus
/// ring.
pub(crate) fn quit_dialog_button(
    id: &'static str,
    label: impl Into<SharedString>,
    default: bool,
    theme: &ThemeColors,
    scale: f32,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(DIALOG_BUTTON_HEIGHT * scale))
        .px(px(12. * scale))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4. * scale))
        .bg(theme.bg_surface)
        .border_1()
        .border_color(if default {
            theme.border_focus
        } else {
            theme.border
        })
        .text_color(theme.text)
        .cursor_pointer()
        .hover(|style| style.bg(theme.bg_elevated))
        .child(label.into())
}

/// Everything [`build_title_menu`] needs beyond the `PopupMenu` builder
/// itself and the window/context pair, bundled so the function stays under
/// clippy's argument-count lint.
struct TitleMenuContext {
    workspace: gpui::WeakEntity<Workspace>,
    action_context: FocusHandle,
    has_project: bool,
    can_save: bool,
    recents: Vec<PathBuf>,
}

/// Builds the popup menu of §8.2 over `menu`, wiring every item's action and
/// the "Carpetas recientes" submenu.
fn build_title_menu(
    menu: PopupMenu,
    context: TitleMenuContext,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let TitleMenuContext {
        workspace,
        action_context,
        has_project,
        can_save,
        recents,
    } = context;
    let menu = menu.action_context(action_context);
    let menu = menu.menu("Abrir carpeta…", Box::new(actions::OpenFolder));

    let menu = if recents.is_empty() {
        menu.submenu("Carpetas recientes", window, cx, |submenu, _, _| {
            submenu.item(PopupMenuItem::label("No hay carpetas recientes"))
        })
    } else {
        let workspace_for_submenu = workspace.clone();
        menu.submenu("Carpetas recientes", window, cx, move |submenu, _, _| {
            let mut submenu = recents.iter().fold(submenu, |submenu, path| {
                let label = recent_label(path);
                let path = path.clone();
                let workspace = workspace_for_submenu.clone();
                submenu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                    let path = path.clone();
                    let _ = workspace.update(cx, |workspace, cx| {
                        workspace.open_recent(&path, window, cx);
                    });
                }))
            });
            submenu = submenu.separator();
            let workspace = workspace_for_submenu.clone();
            submenu.item(
                PopupMenuItem::new("Borrar la lista").on_click(move |_, _, cx| {
                    let _ = workspace.update(cx, |workspace, cx| workspace.clear_recents(cx));
                }),
            )
        })
    };

    menu.menu_with_disabled("Nuevo archivo…", Box::new(actions::NewFile), !has_project)
        .separator()
        .menu_with_disabled("Guardar", Box::new(cincel_editor::actions::Save), !can_save)
        .menu_with_disabled("Guardar todo", Box::new(actions::SaveAll), !has_project)
        .separator()
        .menu("Configuración", Box::new(actions::OpenSettings))
        .menu("Atajos de teclado", Box::new(actions::ShowShortcuts))
        .menu("Conexiones", Box::new(actions::OpenConnections))
        .separator()
        .menu("Salir", Box::new(actions::Quit))
}

impl Workspace {
    /// The whole title bar of §8.1: the menu button, the chat toggle, the
    /// title, a spacer, and the files toggle — all inside `TitleBar::new()`,
    /// which keeps drawing the window controls after them and never changes
    /// its own height (D16).
    pub(crate) fn render_title_bar(
        &mut self,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let has_project = self.project().is_some();
        // `.into_any_element()` right away: under Rust 2024's default opaque
        // capture rules each `impl IntoElement` return otherwise keeps
        // borrowing `self`/`cx` for as long as the local lives, and two
        // sequential builder calls in the same block would conflict.
        let menu_button = self.render_title_menu_button(window, cx).into_any_element();
        let logo = self.render_title_bar_logo(cx).into_any_element();

        let mut row = h_flex()
            .id("title-bar-row")
            .w_full()
            .items_center()
            .gap_1()
            .child(logo)
            .child(menu_button);

        if has_project {
            row = row.child(self.render_panel_toggle(FocusZone::Chat, cx));
        }

        row = row
            .child(
                div()
                    .flex()
                    .items_center()
                    .px_1()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(title),
            )
            .child(div().flex_1());

        if has_project {
            row = row.child(self.render_panel_toggle(FocusZone::Files, cx));
        }

        // The `×` asks first (`Self::close_from_title_bar`); without a
        // handler gpui-kit removes the window on the spot.
        TitleBar::new()
            .on_close_window(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.close_from_title_bar(window, cx);
            }))
            .child(row)
    }

    /// Cincel's chisel mark, 16 px, to the left of the menu button
    /// (`crate::logo`); the row stays `items_center()` with no fixed height
    /// of its own, so a 16 px icon next to the existing buttons never
    /// changes the title bar's height (`TitleBar::new()` keeps that, D16).
    fn render_title_bar_logo(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = crate::settings::ui_scale(cx);
        crate::logo::logo(px(TITLE_BUTTON_ICON_SIZE * scale))
            .debug_selector(|| "titlebar-logo".to_string())
    }

    /// The menu button of §8.2: `IconName::Menu`, ghost, with the popup menu
    /// attached through gpui-kit's own `DropdownMenu` trait.
    fn render_title_menu_button(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let scale = crate::settings::ui_scale(cx);
        let has_project = self.project().is_some();
        let center = self.center().read(cx);
        let can_save = center.active_tab().is_some();
        let action_context = center
            .active_tab()
            .map(|tab| tab.editor().read(cx).focus_handle(cx))
            .unwrap_or_else(|| self.focus_handle(cx));
        let recents = self.recents_snapshot().to_vec();
        let workspace = cx.weak_entity();
        let open_flag = self.title_menu_open_flag();

        Button::new("titlebar-menu")
            .icon(IconName::Menu)
            .ghost()
            .w(px(TITLE_BUTTON_SIZE * scale))
            .h(px(TITLE_BUTTON_SIZE * scale))
            .tooltip("Menú")
            .debug_selector(|| "titlebar-menu".to_string())
            .dropdown_menu(move |menu, window, cx| {
                let context = TitleMenuContext {
                    workspace: workspace.clone(),
                    action_context: action_context.clone(),
                    has_project,
                    can_save,
                    recents: recents.clone(),
                };
                build_title_menu(menu, context, window, cx)
            })
            .on_open_change(move |open, _, _| open_flag.set(*open))
    }

    /// One of the two panel toggle buttons of §8.3: same click as
    /// `workspace::toggle_chat` / `toggle_tree` (`Workspace::toggle_focus`,
    /// §7.1), with the icon reflecting open/closed on every render.
    fn render_panel_toggle(&mut self, zone: FocusZone, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeColors::global(cx).clone();
        let scale = crate::settings::ui_scale(cx);
        let Some(placement) = zone.placement() else {
            return div().into_any_element();
        };
        let visible = self.is_dock_open(placement, cx);
        let (icon, tooltip, selector): (IconName, &'static str, &'static str) = match zone {
            FocusZone::Chat => (
                if visible {
                    IconName::PanelLeftClose
                } else {
                    IconName::PanelLeft
                },
                "Mostrar u ocultar el chat (Ctrl+Shift+A)",
                "titlebar-toggle-chat",
            ),
            FocusZone::Files => (
                if visible {
                    IconName::PanelRightClose
                } else {
                    IconName::PanelRight
                },
                "Mostrar u ocultar los archivos (Ctrl+Shift+E)",
                "titlebar-toggle-files",
            ),
            FocusZone::Center => return div().into_any_element(),
        };

        div()
            .id(selector)
            .debug_selector(move || selector.to_string())
            .w(px(TITLE_BUTTON_SIZE * scale))
            .h(px(TITLE_BUTTON_SIZE * scale))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4. * scale))
            .cursor_pointer()
            .text_color(if visible {
                theme.text
            } else {
                theme.text_muted
            })
            .hover(|style| style.text_color(theme.text).bg(theme.bg_surface))
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            .child(
                div()
                    .w(px(TITLE_BUTTON_ICON_SIZE * scale))
                    .flex_none()
                    .child(icon),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.toggle_focus(zone, window, cx);
            }))
            .into_any_element()
    }

    /// Whether the title bar's menu is currently open, for
    /// `Workspace::is_modal_open` (§7.4).
    pub fn is_title_menu_open(&self) -> bool {
        self.title_menu_open_flag().get()
    }

    /// Opens `path` as the project, replacing the current one (§8.2); a
    /// folder that no longer exists is pruned from the list with a toast
    /// instead.
    pub fn open_recent(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if !path.is_dir() {
            self.forget_recent(path, cx);
            crate::toast::error(format!("La carpeta «{}» ya no existe", path.display()), cx);
            return;
        }
        self.request_open_project(path, window, cx);
    }

    /// "Borrar la lista" in the "Carpetas recientes" submenu.
    pub fn clear_recents(&mut self, cx: &mut Context<Self>) {
        self.set_recents(Recents::new());
        cx.notify();
    }

    fn forget_recent(&mut self, path: &Path, cx: &mut Context<Self>) {
        let mut recents = self.recents_snapshot().to_vec();
        recents.retain(|existing| existing != path);
        let mut updated = Recents::new();
        for path in recents {
            updated.push(path);
        }
        self.set_recents(updated);
        cx.notify();
    }

    /// `Ctrl+N` (`workspace::new_file`): see `crate::new_file`.
    /// `workspace::quit` (`Ctrl+Q`, D15, §8.2): the same question the
    /// window's `×` asks (`Self::should_close`); closes the window itself
    /// once the answer is `true` (dropping the root view runs
    /// `cx.on_release` in `Workspace::open_window`, which quits the app —
    /// `cx.on_app_quit`, registered in `Workspace::new`, is what actually
    /// saves the window geometry, the layout, the review and the
    /// conversation).
    pub fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.should_close(window, cx) {
            window.remove_window();
        }
    }

    /// Closing the window from the title bar's `×` or from the desktop
    /// (`Alt+F4`, `Workspace::open_window`'s `on_window_should_close`):
    /// both go through here, so both ask the same questions
    /// ([`Self::should_close`]). Returns whether the window may close now;
    /// if so its geometry is already on disk (closing the only window ends
    /// the application right after). The caller removes the window.
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let may_close = self.should_close(window, cx);
        if may_close
            && let Err(error) =
                crate::window_state::WindowState::from_window_bounds(window.window_bounds()).save()
        {
            tracing::warn!(%error, "no se pudo guardar el estado de la ventana");
        }
        may_close
    }

    /// The title bar's `×`: [`Self::request_close`], then the window goes
    /// away only when it said so. gpui-kit's `TitleBar` would otherwise
    /// remove the window at once, never asking.
    pub fn close_from_title_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_close(window, cx) {
            window.remove_window();
        }
    }

    /// The question behind `Ctrl+Q` and the window's `×`
    /// (`Workspace::open_window`'s `on_window_should_close`, D15, §8.2):
    /// `false` opens the "¿Guardar cambios?" dialog and keeps the window up;
    /// `true` once there is nothing unsaved, or once the dialog already
    /// confirmed (`Self::quit_confirmed`). Before the unsaved files, pending
    /// agent changes are asked about ("Hay N cambios de agente sin decidir",
    /// `crate::review_close`); deciding them asks this again.
    pub fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.is_quit_confirmed() {
            return true;
        }
        if self.ask_about_pending_review(crate::review_close::CloseIntent::Quit, window, cx) {
            return false;
        }
        if !self.has_unsaved_files(cx) {
            return true;
        }
        self.open_quit_dialog(window, cx);
        false
    }

    fn has_unsaved_files(&self, cx: &App) -> bool {
        self.center().read(cx).dirty_count(cx) > 0
    }

    fn open_quit_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_quit_dialog_open() {
            return;
        }
        let dirty_count = self.center().read(cx).dirty_count(cx);
        let previous_focus = window.focused(cx);
        self.set_quit_dialog(Some(QuitDialog {
            dirty_count,
            previous_focus,
        }));
        let handle = self.focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// "Guardar todo y salir" (`Enter`): saves every dirty file and quits
    /// only if every save succeeded — a failed one leaves the dialog open,
    /// with the error already reported by `CenterPanel::save_path`'s toast
    /// (§8.2: "si alguno falla, el diálogo se queda con el error").
    fn confirm_save_all_and_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_quit_dialog_open() {
            return;
        }
        let all_saved = self
            .center()
            .clone()
            .update(cx, |center, cx| center.save_all_for_quit(window, cx));
        if !all_saved {
            cx.notify();
            return;
        }
        self.set_quit_dialog(None);
        self.set_quit_confirmed(true);
        self.request_quit(window, cx);
    }

    /// "Salir sin guardar".
    fn discard_and_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_quit_dialog(None);
        self.set_quit_confirmed(true);
        self.request_quit(window, cx);
    }

    /// "Cancelar" (`Esc`): the window stays open and the keyboard goes back
    /// to wherever it was.
    fn cancel_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(previous_focus) = self.take_quit_dialog_previous_focus() else {
            return;
        };
        match previous_focus {
            Some(previous) => window.focus(&previous, cx),
            None => {
                let handle = self.focus_handle(cx);
                window.focus(&handle, cx);
            }
        }
        cx.notify();
    }

    fn on_confirm_save_all_and_quit(
        &mut self,
        _: &ConfirmSaveAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_save_all_and_quit(window, cx);
    }

    fn on_cancel_quit(&mut self, _: &CancelQuit, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_quit(window, cx);
    }

    /// The "¿Guardar cambios?" dialog of §8.2, floating over everything else
    /// like the tab-close dialog it borrows its look from.
    pub(crate) fn render_quit_dialog(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let dirty_count = self.quit_dialog_dirty_count()?;
        let theme = ThemeColors::global(cx).clone();
        let scale = crate::settings::ui_scale(cx);
        let message = if dirty_count == 1 {
            "Hay 1 archivo con cambios sin guardar".to_string()
        } else {
            format!("Hay {dirty_count} archivos con cambios sin guardar")
        };

        Some(
            div()
                .id("quit-dialog-scrim")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.bg_app.alpha(0.4))
                .child(
                    v_flex()
                        .id("quit-dialog")
                        .debug_selector(|| "quit-dialog".to_string())
                        .key_context(QUIT_KEY_CONTEXT)
                        .track_focus(&self.focus_handle(cx))
                        .on_action(cx.listener(Self::on_confirm_save_all_and_quit))
                        .on_action(cx.listener(Self::on_cancel_quit))
                        .w(px(DIALOG_WIDTH * scale))
                        .p_4()
                        .gap_3()
                        .rounded(px(6. * scale))
                        .bg(theme.bg_elevated)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_lg()
                        .child(
                            div()
                                .text_size(px(14. * scale))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child("¿Guardar cambios?"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.text_muted)
                                .child(SharedString::from(message)),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    quit_dialog_button(
                                        "quit-cancel",
                                        "Cancelar",
                                        false,
                                        &theme,
                                        scale,
                                    )
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, window, cx| {
                                            this.cancel_quit(window, cx)
                                        },
                                    )),
                                )
                                .child(
                                    quit_dialog_button(
                                        "quit-discard",
                                        "Salir sin guardar",
                                        false,
                                        &theme,
                                        scale,
                                    )
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, window, cx| {
                                            this.discard_and_quit(window, cx)
                                        },
                                    )),
                                )
                                .child(
                                    quit_dialog_button(
                                        "quit-save-all",
                                        "Guardar todo y salir",
                                        true,
                                        &theme,
                                        scale,
                                    )
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, window, cx| {
                                            this.confirm_save_all_and_quit(window, cx)
                                        },
                                    )),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child("Enter guarda todo y sale · Esc cancela"),
                        ),
                ),
        )
    }

    /// "Cancelar" in the dialog, for the tests (which cannot click).
    pub fn cancel_quit_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_quit(window, cx);
    }

    /// "Guardar todo y salir" in the dialog, for the tests.
    pub fn confirm_save_all_and_quit_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_save_all_and_quit(window, cx);
    }

    /// "Salir sin guardar" in the dialog, for the tests.
    pub fn discard_and_quit_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.discard_and_quit(window, cx);
    }
}
