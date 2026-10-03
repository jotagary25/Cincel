//! Pending agent changes are decided before they are left behind, and the
//! floating bar's "Rechazar todo" asks before undoing a whole turn.
//!
//! **Closing.** Quitting (`Ctrl+Q`, the window's `×`, both through
//! [`Workspace::should_close`]), "Abrir carpeta…" and "Carpetas recientes"
//! (both through [`Workspace::request_open_project`]) first look at the
//! review: with agent changes nobody accepted or rejected yet, they open
//! "Hay N cambios de agente sin decidir en M archivos" with "Aceptar todo",
//! "Rechazar todo" and "Cancelar" instead of going on, so a review is never
//! saved half-decided to reappear the next time the folder opens. Both
//! decisions cover every pending change (every turn, every file: what the
//! dialog counted) and then pick the interrupted action up again — for a
//! quit, that is where the "¿Guardar cambios?" dialog of unsaved files comes
//! in (`crate::title_menu`). "Cancelar" (or `Esc`) leaves everything as it
//! was. An agent in the middle of a turn is told to stop first (the chat's
//! own cancel), and the turn is closed in the review so its changes can be
//! decided.
//!
//! **"Rechazar todo" from the bar.** [`crate::review::Review`] keeps the
//! question ("Rechazar N cambios en M archivos" / "Cancelar"); this module
//! paints it over the window. `workspace::undo_last_reject` takes the
//! rejection back like any other.
//!
//! Both dialogs share the look of the quit dialog: neutral buttons on
//! `bg.surface`, `text` labels, no default action.

use std::path::{Path, PathBuf};

use cincel_chat::AgentStatus;
use gpui::{ClickEvent, Context, FocusHandle, Focusable, KeyDownEvent, Window};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, SharedString, div, px};

use crate::review::count_label;
use crate::theme::ThemeColors;
use crate::title_menu::quit_dialog_button;
use crate::workspace::Workspace;

/// Same width as the quit dialog.
const DIALOG_WIDTH: f32 = 420.;
/// The GPUI key context of the pending-changes dialog.
pub const REVIEW_CLOSE_KEY_CONTEXT: &str = "ReviewCloseDialog";
/// The GPUI key context of the bar's "Rechazar todo" confirmation.
pub const REJECT_TURN_KEY_CONTEXT: &str = "RejectTurnDialog";

/// What was interrupted to ask about the pending changes, and is resumed
/// once they are decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseIntent {
    /// `Ctrl+Q` or the window's `×`.
    Quit,
    /// "Abrir carpeta…" or "Carpetas recientes": this folder replaces the
    /// project.
    OpenProject(PathBuf),
}

/// The open "cambios de agente sin decidir" dialog.
pub(crate) struct ReviewCloseDialog {
    changes: usize,
    files: usize,
    intent: CloseIntent,
    previous_focus: Option<FocusHandle>,
}

/// "N cambios de agente sin decidir en M archivos", with the singulars.
pub fn pending_message(changes: usize, files: usize) -> String {
    let changes = if changes == 1 {
        "1 cambio de agente sin decidir".to_string()
    } else {
        format!("{changes} cambios de agente sin decidir")
    };
    format!(
        "Hay {changes} en {}",
        count_label(files, "archivo", "archivos")
    )
}

/// "Rechazar N cambios en M archivos", the confirmation's button.
pub fn reject_turn_label(changes: usize, files: usize) -> String {
    format!(
        "Rechazar {} en {}",
        count_label(changes, "cambio", "cambios"),
        count_label(files, "archivo", "archivos")
    )
}

impl Workspace {
    /// Opens `path` as the project, replacing the current one, once the
    /// current one's pending agent changes are decided: the entry point of
    /// "Abrir carpeta…" and "Carpetas recientes" (`Self::open_project` itself
    /// never asks, for the command line and the tests).
    pub fn request_open_project(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ask_about_pending_review(CloseIntent::OpenProject(path.to_path_buf()), window, cx) {
            return;
        }
        self.open_project(path, window, cx);
    }

    /// Opens the "cambios de agente sin decidir" dialog for `intent` when the
    /// open project has pending agent changes, cancelling a running turn
    /// first. Returns whether it did (the caller must stop there).
    pub(crate) fn ask_about_pending_review(
        &mut self,
        intent: CloseIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.project().is_none() {
            return false;
        }
        let review = self.review().clone();
        if review.read(cx).pending_counts().0 == 0 {
            return false;
        }
        if review.read(cx).is_turn_active() {
            let chat = self.chat().clone();
            let running = matches!(
                chat.read(cx).status(),
                AgentStatus::Thinking | AgentStatus::WaitingPermission
            );
            if running {
                chat.update(cx, |chat, cx| chat.cancel_turn(cx));
            }
            review.update(cx, |review, cx| review.agent_gone(cx));
            crate::toast::warn(
                "Se detuvo el turno del agente para decidir sus cambios.",
                cx,
            );
        }
        let (changes, files) = review.read(cx).pending_counts();
        if changes == 0 {
            return false;
        }
        // The bar's own confirmation would sit under this one.
        review.update(cx, |review, cx| review.cancel_reject_turn(window, cx));
        if let Some(dialog) = self.review_close_dialog_mut() {
            dialog.changes = changes;
            dialog.files = files;
            dialog.intent = intent;
            cx.notify();
            return true;
        }
        let previous_focus = window.focused(cx);
        self.set_review_close_dialog(Some(ReviewCloseDialog {
            changes,
            files,
            intent,
            previous_focus,
        }));
        let handle = self.focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
        true
    }

    /// Whether the "cambios de agente sin decidir" dialog is up.
    pub fn is_review_close_dialog_open(&self) -> bool {
        self.review_close_dialog().is_some()
    }

    /// `(changes, files)` the dialog asks about, while it is up.
    pub fn review_close_dialog_counts(&self) -> Option<(usize, usize)> {
        self.review_close_dialog()
            .map(|dialog| (dialog.changes, dialog.files))
    }

    /// "Aceptar todo" / "Rechazar todo": decides every pending change, then
    /// resumes what was interrupted (which may ask about unsaved files).
    fn decide_pending_and_continue(
        &mut self,
        accept: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self
            .review_close_dialog_mut()
            .map(|dialog| (dialog.intent.clone(), dialog.previous_focus.clone()))
        else {
            return;
        };
        self.set_review_close_dialog(None);
        let (intent, previous_focus) = dialog;
        if let Some(previous) = previous_focus {
            window.focus(&previous, cx);
        }
        self.review().clone().update(cx, |review, cx| {
            if accept {
                review.accept_all(cx);
            } else {
                review.reject_all(cx);
            }
        });
        cx.notify();
        match intent {
            CloseIntent::Quit => self.request_quit(window, cx),
            CloseIntent::OpenProject(path) => self.request_open_project(&path, window, cx),
        }
    }

    /// "Cancelar" (`Esc`): nothing is decided and nothing closes.
    fn cancel_review_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(previous_focus) = self
            .review_close_dialog_mut()
            .map(|dialog| dialog.previous_focus.clone())
        else {
            return;
        };
        self.set_review_close_dialog(None);
        match previous_focus {
            Some(previous) => window.focus(&previous, cx),
            None => {
                let handle = self.focus_handle(cx);
                window.focus(&handle, cx);
            }
        }
        cx.notify();
    }

    /// The "cambios de agente sin decidir" dialog, floating over everything
    /// like the quit dialog.
    pub(crate) fn render_review_close_dialog(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let (changes, files) = self.review_close_dialog_counts()?;
        let theme = ThemeColors::global(cx).clone();
        let scale = crate::settings::ui_scale(cx);
        let message = pending_message(changes, files);
        // D17 of spec 09: the unsent comments are counted too; nothing else
        // changes (they are saved and come back with the folder).
        let comments = self.review_close_comments_line(cx);

        Some(
            div()
                .id("review-close-scrim")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.bg_app.alpha(0.4))
                .child(
                    v_flex()
                        .id("review-close-dialog")
                        .debug_selector(|| "review-close-dialog".to_string())
                        .key_context(REVIEW_CLOSE_KEY_CONTEXT)
                        .track_focus(&self.focus_handle(cx))
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape" {
                                cx.stop_propagation();
                                this.cancel_review_close(window, cx);
                            }
                        }))
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
                                .child("¿Qué hacemos con los cambios del agente?"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.text_muted)
                                .child(SharedString::from(message)),
                        )
                        .children(comments.map(|line| {
                            div()
                                .debug_selector(|| "review-close-comments".to_string())
                                .text_size(px(12. * scale))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(line))
                        }))
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    quit_dialog_button(
                                        "review-close-cancel",
                                        "Cancelar",
                                        false,
                                        &theme,
                                        scale,
                                    )
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, window, cx| {
                                            this.cancel_review_close(window, cx)
                                        },
                                    )),
                                )
                                .child(
                                    quit_dialog_button(
                                        "review-close-reject",
                                        "Rechazar todo",
                                        false,
                                        &theme,
                                        scale,
                                    )
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, window, cx| {
                                            this.decide_pending_and_continue(false, window, cx)
                                        },
                                    )),
                                )
                                .child(
                                    quit_dialog_button(
                                        "review-close-accept",
                                        "Aceptar todo",
                                        false,
                                        &theme,
                                        scale,
                                    )
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, window, cx| {
                                            this.decide_pending_and_continue(true, window, cx)
                                        },
                                    )),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child("Se aplica a todos los archivos · Esc cancela"),
                        ),
                ),
        )
    }

    /// The floating bar's "¿Rechazar todo el turno?" confirmation.
    pub(crate) fn render_reject_turn_dialog(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let review = self.review().clone();
        let (changes, files) = review.read(cx).reject_turn_prompt()?;
        let focus = review.read(cx).reject_turn_focus().clone();
        let theme = ThemeColors::global(cx).clone();
        let scale = crate::settings::ui_scale(cx);
        let confirm = reject_turn_label(changes, files);
        let on_cancel = {
            let review = review.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut gpui::App| {
                review.update(cx, |review, cx| review.cancel_reject_turn(window, cx));
            }
        };
        let on_confirm = {
            let review = review.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut gpui::App| {
                review.update(cx, |review, cx| review.confirm_reject_turn(window, cx));
            }
        };
        let on_key = {
            let review = review.clone();
            move |event: &KeyDownEvent, window: &mut Window, cx: &mut gpui::App| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    review.update(cx, |review, cx| review.cancel_reject_turn(window, cx));
                }
            }
        };

        Some(
            div()
                .id("reject-turn-scrim")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.bg_app.alpha(0.4))
                .child(
                    v_flex()
                        .id("reject-turn-dialog")
                        .debug_selector(|| "reject-turn-dialog".to_string())
                        .key_context(REJECT_TURN_KEY_CONTEXT)
                        .track_focus(&focus)
                        .on_key_down(on_key)
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
                                .child("¿Rechazar todo el turno?"),
                        )
                        .child(div().text_sm().text_color(theme.text_muted).child(
                            "Se deshacen los cambios del agente en todos los archivos. \
                                     Alt+Shift+U los recupera.",
                        ))
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    quit_dialog_button(
                                        "reject-turn-cancel",
                                        "Cancelar",
                                        false,
                                        &theme,
                                        scale,
                                    )
                                    .on_click(on_cancel),
                                )
                                .child(
                                    quit_dialog_button(
                                        "reject-turn-confirm",
                                        confirm,
                                        false,
                                        &theme,
                                        scale,
                                    )
                                    .on_click(on_confirm),
                                ),
                        ),
                ),
        )
    }

    /// The dialog's line about the unsent comments ("También hay 2
    /// comentarios sin enviar…"), while it is up and there are any.
    pub fn review_close_comments_line(&self, cx: &gpui::App) -> Option<String> {
        self.review_close_dialog()?;
        crate::review::close_comments_line(self.review().read(cx).comment_count())
    }

    /// "Aceptar todo" in the dialog, for the tests (which cannot click).
    pub fn accept_pending_and_close_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.decide_pending_and_continue(true, window, cx);
    }

    /// "Rechazar todo" in the dialog, for the tests.
    pub fn reject_pending_and_close_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.decide_pending_and_continue(false, window, cx);
    }

    /// "Cancelar" in the dialog, for the tests.
    pub fn cancel_review_close_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_review_close(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_messages_say_what_they_count() {
        assert_eq!(
            pending_message(3, 1),
            "Hay 3 cambios de agente sin decidir en 1 archivo"
        );
        assert_eq!(
            pending_message(1, 1),
            "Hay 1 cambio de agente sin decidir en 1 archivo"
        );
        assert_eq!(reject_turn_label(5, 2), "Rechazar 5 cambios en 2 archivos");
        assert_eq!(
            crate::review::totals_label(1, 3, (0, 36)),
            "1 archivo · 3 cambios · +0 −36"
        );
    }
}
