//! Painting the chat (`docs/specs/02-visual.md` §7).
//!
//! Every clickable thing gets a pointer cursor and a hover state, and the real
//! buttons are `gpui-kit` [`Button`]s so they follow the palette the workspace
//! projects onto gpui-kit's theme instead of inventing their own contrast.
//!
//! The transcript has two voices and they look different on purpose, with no
//! "Vos" / agent-name labels: what the user said is a bubble against the
//! right edge, tinted with `text.accent` (14 % fill, 35 % border); what the
//! agent answered is plain text on the panel background, and the code inside
//! it sits on `bg.editor` with a 1 px `border` and a language / "Copiar"
//! header, so neither can pass for the other. Everything in between —
//! thoughts, tool cards, plans — is a compact row on `bg.surface`.
//!
//! Every size comes from the type scale of [`crate::settings`]: body 13 px,
//! code 12.5 px, tool rows / plan / hints 12 px, labels 11 px.

use std::path::Path;
use std::sync::Arc;

use cincel_acp::acp::schema::v1::{
    PermissionOptionId, PlanEntryStatus, SessionConfigId, SessionConfigValueId, SessionModeId,
    ToolCallStatus, ToolKind,
};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, Focusable as _, FontWeight, SharedString, Window,
    div, list, px, relative,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};

use crate::actions;
use crate::markdown::markdown_element_scaled;
use crate::model::*;
use crate::panel::{
    COMPOSER_HINT, ChatPanel, Popover, current_label, is_footer_option, select_options,
};
use crate::settings::{
    BUBBLE_BORDER_ALPHA, BUBBLE_FILL_ALPHA, BUBBLE_MAX_WIDTH, BUBBLE_RADIUS, CARD_RADIUS,
    COMMAND_CARD_RADIUS, ChatSettings, INPUT_MIN_HEIGHT, INPUT_RADIUS, MAX_OUTPUT_LINES,
    MESSAGE_GAP, POPOVER_RADIUS, POPOVER_ROW_HEIGHT, ROW_GAP, TEXT_BODY, TEXT_CODE, TEXT_LABEL,
    TEXT_SMALL, TOOL_ROW_HEIGHT, TURN_GAP,
};
use crate::theme::{ChatTheme, alpha};

/// Height of the header row.
const HEADER_HEIGHT: f32 = 32.;
/// Height of one selector chip: an 11 px label at gpui's default line height
/// (× 1.618), 2 px of padding above and below and a 1 px border.
const SELECTOR_ROW_HEIGHT: f32 = 24.;

impl Render for ChatPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *self.theme();
        let s = *self.settings();
        self.sync_placeholder(window, cx);
        self.sync_composer(cx);
        // The header's popovers hang from the header; the composer's (`@`,
        // `/`, selectors) sit right above the composer, however tall it grew.
        let overlay = self.render_overlay(&theme, cx);
        let from_header = matches!(
            self.popover(),
            Popover::Connections | Popover::ConnectionMenu(_) | Popover::Conversations
        );
        let (header_overlay, composer_overlay) = if from_header {
            (overlay, None)
        } else {
            (None, overlay)
        };
        v_flex()
            .id("chat-panel")
            .key_context(self.key_context())
            .track_focus(&self.focus_handle(cx))
            .on_action(cx.listener(Self::on_send))
            .on_action(cx.listener(Self::on_newline))
            .on_action(cx.listener(Self::on_cancel_turn))
            .on_action(cx.listener(Self::on_focus_input))
            .on_action(cx.listener(Self::on_new_session))
            .on_action(cx.listener(Self::on_accept_permission))
            .on_action(cx.listener(Self::on_reject_permission))
            .on_action(cx.listener(Self::on_close_popover))
            .on_action(cx.listener(Self::on_popover_next))
            .on_action(cx.listener(Self::on_popover_prev))
            .on_action(cx.listener(Self::on_popover_confirm))
            .size_full()
            .relative()
            .bg(theme.bg_app)
            .text_size(s.px(TEXT_BODY))
            .text_color(theme.text)
            .child(self.render_header(&theme, cx))
            .child(self.render_transcript(&theme, cx))
            .child(self.render_composer(&theme, composer_overlay, window, cx))
            .children(header_overlay)
    }
}

impl ChatPanel {
    // ------------------------------------------------------------- actions

    fn on_send(&mut self, _: &actions::Send, window: &mut Window, cx: &mut Context<Self>) {
        if self.popover_confirms() {
            self.confirm_popover(window, cx);
        } else {
            self.send(window, cx);
        }
    }

    /// `Shift+Enter`: a line break in the composer.
    fn on_newline(&mut self, _: &actions::Newline, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_newline(window, cx);
    }

    fn on_cancel_turn(&mut self, _: &actions::CancelTurn, _: &mut Window, cx: &mut Context<Self>) {
        self.cancel_turn(cx);
    }

    fn on_focus_input(
        &mut self,
        _: &actions::FocusInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_input(window, cx);
    }

    fn on_new_session(
        &mut self,
        _: &actions::NewSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_session(window, cx);
    }

    fn on_accept_permission(
        &mut self,
        _: &actions::AcceptPermission,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.accept_permission(cx);
    }

    fn on_reject_permission(
        &mut self,
        _: &actions::RejectPermission,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reject_permission(cx);
    }

    /// `chat::close_popover`, for a keymap that binds it. Inside the composer
    /// `Esc` is the editor's `editor::cancel`, taken by the composer box
    /// ([`Self::on_composer_escape`]).
    fn on_close_popover(
        &mut self,
        _: &actions::ClosePopover,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.popover() == &Popover::Closed {
            cx.propagate();
            return;
        }
        self.close_popover(cx);
    }

    /// `↓`. Belongs to the popover while it lists something, to the caret
    /// otherwise.
    fn on_popover_next(
        &mut self,
        _: &actions::PopoverNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.popover_rows() == 0 {
            cx.propagate();
            return;
        }
        self.move_popover_selection(1, cx);
    }

    /// `↑`, the mirror of [`Self::on_popover_next`].
    fn on_popover_prev(
        &mut self,
        _: &actions::PopoverPrev,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.popover_rows() == 0 {
            cx.propagate();
            return;
        }
        self.move_popover_selection(-1, cx);
    }

    /// `Tab` on the panel (and `chat::popover_confirm` in a user keymap).
    /// Confirms the highlighted row **without sending**; with no popover open
    /// the key propagates. Inside the composer the editor's `editor::tab` and
    /// `editor::insert_newline` do this ([`Self::on_composer_tab`],
    /// [`Self::on_composer_enter`]).
    fn on_popover_confirm(
        &mut self,
        _: &actions::PopoverConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.popover_confirms() {
            cx.propagate();
            return;
        }
        self.confirm_popover(window, cx);
    }

    // -------------------------------------------------------------- header

    /// The header: the "Conectar" control where the agent selector used to
    /// be (`docs/specs/06-etapa4-conexiones-y-cincel.md` §6). With no active
    /// connection it is a "Conectar" button; with one, the provider icon, the
    /// label, the identity in muted text and a caret. Both open the popover.
    fn render_header(&self, theme: &ChatTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *theme;
        let s = *self.settings();
        let status = self.status();
        let active = self.active_connection().cloned();

        let control = match &active {
            Some(connection) => h_flex()
                .id("chat-connection")
                .debug_selector(|| "chat-connection".to_string())
                .min_w(px(0.))
                .gap_1()
                .px_1p5()
                .py_0p5()
                .items_center()
                .rounded(s.px(CARD_RADIUS))
                .cursor_pointer()
                .hover(move |style| style.bg(theme.bg_surface))
                .child(provider_icon(
                    &connection.agent_id,
                    s.px(16.),
                    theme.text,
                    theme.bg_surface,
                    theme.border,
                ))
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .child(SharedString::from(connection.label.clone())),
                )
                .children(connection.identity.clone().map(|identity| {
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(s.px(TEXT_LABEL))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(identity))
                }))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme.text_muted)
                        .child(IconName::ChevronDown),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.toggle_popover(Popover::Connections, cx);
                }))
                .into_any_element(),
            None => Button::new("chat-connect")
                .label("Conectar")
                .small()
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.toggle_popover(Popover::Connections, cx);
                }))
                .into_any_element(),
        };

        h_flex()
            .id("chat-header")
            .h(s.px(HEADER_HEIGHT))
            .px_2()
            .gap_2()
            .items_center()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme.border)
            .child(control)
            .when(active.is_some(), |this| {
                this.child(status_pill(status, &theme, &s))
            })
            .child(div().flex_1())
            .child(
                icon_button(
                    "chat-new-session",
                    IconName::Plus,
                    "Nueva conversación",
                    &theme,
                    &s,
                )
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.new_session(window, cx)),
                ),
            )
            .child(
                icon_button("chat-sessions", IconName::Clock, "Historial", &theme, &s).on_click(
                    cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.toggle_popover(Popover::Conversations, cx);
                    }),
                ),
            )
    }

    /// "La sesión de «X» venció" / "«X» no está disponible" over the
    /// transcript (`docs/specs/06-etapa4-conexiones-y-cincel.md` §4 F4, F5).
    fn render_banner(
        &self,
        banner: &ConnectionBanner,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        let id = banner.connection_id().to_string();
        let action = match banner {
            ConnectionBanner::Expired { .. } => Button::new("banner-reconnect")
                .label("Volver a conectar")
                .small()
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.request_reconnect(&id, cx);
                })),
            ConnectionBanner::Unavailable { .. } => Button::new("banner-repair")
                .label("Reparar")
                .small()
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.request_repair(&id, cx);
                })),
        };
        h_flex()
            .id("chat-connection-banner")
            .debug_selector(|| "chat-connection-banner".to_string())
            .mx_2()
            .mt_2()
            .px_2()
            .py_1p5()
            .gap_2()
            .items_center()
            .rounded(s.px(CARD_RADIUS))
            .bg(alpha(theme.status_warning, 0.12))
            .border_1()
            .border_color(theme.status_warning)
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .gap_0p5()
                    .child(
                        div()
                            .whitespace_normal()
                            .text_size(s.px(TEXT_SMALL))
                            .text_color(theme.text)
                            .child(SharedString::from(banner.text())),
                    )
                    // "El agente dijo: «motivo»" (§10.1): only an `Expired`
                    // banner whose `AuthRequired` carried a message has one.
                    .when_some(banner.reason_text(), |this, reason| {
                        this.child(
                            div()
                                .id("chat-connection-banner-reason")
                                .debug_selector(|| "chat-connection-banner-reason".to_string())
                                .whitespace_normal()
                                .text_size(s.px(TEXT_SMALL))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(reason)),
                        )
                    }),
            )
            .child(action)
            .into_any_element()
    }

    /// The rows of the "Conectar" popover: icon, label, identity, "Usado hace
    /// …", badge, and the "Volver a conectar" / "Reparar" shortcut of an
    /// expired or unavailable connection. A right click opens the row's
    /// context menu ("Renombrar").
    fn render_connection_rows(&self, theme: &ChatTheme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = *theme;
        let s = *self.settings();
        let highlighted = self
            .active_connection
            .clone()
            .or_else(|| self.preselected_connection.clone());
        if self.connections.is_empty() {
            return vec![
                div()
                    .px_1p5()
                    .py_1()
                    .text_size(s.px(TEXT_SMALL))
                    .text_color(theme.text_muted)
                    .child("Todavía no hay conexiones.")
                    .into_any_element(),
            ];
        }
        self.connections
            .iter()
            .enumerate()
            .map(|(index, connection)| {
                let select_id = connection.id.clone();
                let menu_id = connection.id.clone();
                let action_id = connection.id.clone();
                let selected = highlighted.as_deref() == Some(connection.id.as_str());
                let badge_color = match connection.badge {
                    ConnectionBadge::Connected => theme.status_ok,
                    ConnectionBadge::SessionExpired => theme.status_warning,
                    ConnectionBadge::Unavailable { .. } => theme.status_error,
                };
                let shortcut = match &connection.badge {
                    ConnectionBadge::Connected => None,
                    ConnectionBadge::SessionExpired => Some(
                        Button::new(("connection-reconnect", index))
                            .label("Volver a conectar")
                            .xsmall()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                cx.stop_propagation();
                                this.request_reconnect(&action_id, cx);
                            })),
                    ),
                    ConnectionBadge::Unavailable { .. } => Some(
                        Button::new(("connection-repair", index))
                            .label("Reparar")
                            .xsmall()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                cx.stop_propagation();
                                this.request_repair(&action_id, cx);
                            })),
                    ),
                };
                let tooltip = match &connection.badge {
                    ConnectionBadge::Unavailable { reason } => Some(reason.clone()),
                    _ => None,
                };
                h_flex()
                    .id(("connection", index))
                    .debug_selector(move || format!("connection-row-{index}"))
                    .px_1p5()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .rounded(s.px(CARD_RADIUS))
                    .cursor_pointer()
                    .when(selected, |this| this.bg(theme.selection))
                    .hover(move |style| style.bg(theme.selection))
                    .child(provider_icon(
                        &connection.agent_id,
                        s.px(18.),
                        theme.text,
                        theme.bg_surface,
                        theme.border,
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .min_w(px(0.))
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .truncate()
                                            .child(SharedString::from(connection.label.clone())),
                                    )
                                    .children(connection.identity.clone().map(|identity| {
                                        div()
                                            .min_w(px(0.))
                                            .truncate()
                                            .text_size(s.px(TEXT_LABEL))
                                            .text_color(theme.text_muted)
                                            .child(SharedString::from(identity))
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(s.px(TEXT_LABEL))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(connection.last_used.clone())),
                            ),
                    )
                    .child(
                        div()
                            .id(("connection-badge", index))
                            .flex_shrink_0()
                            .px_1p5()
                            .py_0p5()
                            .rounded(s.px(CARD_RADIUS))
                            .bg(alpha(badge_color, 0.15))
                            .text_size(s.px(TEXT_LABEL))
                            .text_color(badge_color)
                            .child(connection.badge.label())
                            .when_some(tooltip, |this, reason| {
                                this.tooltip(move |window, cx| {
                                    Tooltip::new(SharedString::from(reason.clone()))
                                        .build(window, cx)
                                })
                            }),
                    )
                    .children(shortcut)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        cx.stop_propagation();
                        this.select_connection(&select_id, cx);
                    }))
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(move |this, _: &gpui::MouseDownEvent, _window, cx| {
                            cx.stop_propagation();
                            this.open_connection_menu(&menu_id, cx);
                        }),
                    )
                    .into_any_element()
            })
            .collect()
    }

    /// The foot of the "Conectar" popover.
    fn render_connection_footer(&self, theme: &ChatTheme, cx: &mut Context<Self>) -> AnyElement {
        let theme = *theme;
        let empty = self.connections.is_empty();
        v_flex()
            .mt_1()
            .pt_1()
            .gap_1()
            .border_t_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("connection-new")
                            .label("Conectar nuevo agente…")
                            .small()
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                cx.stop_propagation();
                                this.request_new_connection(cx);
                            })),
                    )
                    .child(
                        Button::new("connection-delete")
                            .label("Eliminar conexión…")
                            .small()
                            .disabled(empty)
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                cx.stop_propagation();
                                this.request_delete_connection(cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    // ---------------------------------------------------------- transcript

    fn render_transcript(&mut self, theme: &ChatTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *theme;
        let s = *self.settings();
        let count = self.entries.len();
        self.sync_list();
        // Streaming chunks, tool updates and toggles change entries in place;
        // the list only lays out what is on screen, so this costs what the
        // old scrolling `div` cost on every frame.
        self.list.remeasure_items(0..count);

        let empty = count == 0;
        let banner = self
            .banner()
            .cloned()
            .map(|banner| self.render_banner(&banner, &theme, cx));
        let notice = self.history_notice().map(|text| {
            div()
                .mx_2()
                .mt_2()
                .px_2()
                .py_1()
                .rounded(s.px(CARD_RADIUS))
                .bg(theme.bg_surface)
                .text_size(s.px(TEXT_SMALL))
                .text_color(theme.text_muted)
                .child(SharedString::from(text.to_string()))
        });
        v_flex()
            .id("chat-transcript")
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .children(banner)
            .children(notice)
            .when(empty, |this| {
                this.child(div().p_2().child(self.render_empty_state(&theme, cx)))
            })
            .when(!empty, |this| {
                this.child(
                    list(
                        self.list.clone(),
                        cx.processor(|this, index: usize, _window, cx| {
                            this.render_list_item(index, cx)
                        }),
                    )
                    .flex_1()
                    .min_h(px(0.))
                    .w_full(),
                )
            })
    }

    /// One item of the transcript list.
    fn render_list_item(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = *self.theme();
        let last = index + 1 == self.entries.len();
        match self.entries.get(index) {
            Some(entry) => {
                let row = self.render_entry(index, entry, &theme, cx);
                div()
                    .w_full()
                    .px_2()
                    .when(index == 0, |this| this.pt_1())
                    .when(last, |this| this.pb_2())
                    .child(row)
                    .into_any_element()
            }
            None => div().into_any_element(),
        }
    }

    /// `02-visual.md` §9: what the chat says when there is nothing to show.
    /// With no active connection: "No hay ningún agente conectado" and the
    /// "Conectar" button (`docs/specs/06-etapa4-conexiones-y-cincel.md` §6).
    fn render_empty_state(&self, theme: &ChatTheme, _cx: &mut Context<Self>) -> AnyElement {
        if self.active_connection.is_none() {
            return v_flex()
                .gap_2()
                .p_3()
                .items_start()
                .text_color(theme.text_muted)
                // The header already has the "Conectar" control: the empty
                // state only explains, it does not repeat the button.
                .child(div().child(NO_CONNECTION_TEXT))
                .child(div().child("Usá «Conectar», arriba, para elegir un agente."))
                .into_any_element();
        }
        v_flex()
            .gap_1()
            .p_3()
            .text_color(theme.text_muted)
            .child(div().child("Escribí abajo para empezar."))
            .child(div().child("@ menciona archivos · / muestra los comandos del agente."))
            .into_any_element()
    }

    fn render_entry(
        &self,
        index: usize,
        entry: &Entry,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let s = *self.settings();
        let inner = match entry {
            Entry::UserMessage(message) => self.render_user_message(index, message, theme, cx),
            Entry::AgentText(text) => self.render_agent_text(index, text, theme, cx),
            Entry::AgentThought(thought) => self.render_thought(index, thought, theme, cx),
            Entry::ToolCall(call) => self.render_tool_call(index, call, theme, cx),
            Entry::Plan(plan) => self.render_plan(index, plan, theme, cx),
            Entry::Permission(permission) => self.render_permission(index, permission, theme, cx),
            Entry::AuthRequired(auth) => self.render_auth(index, auth, theme, cx),
            Entry::Notice(notice) => render_notice(notice, theme, &s),
            // `02-visual.md` §7 has no "turno terminado · 12 s" line any more:
            // a turn ends with air, not with a word.
            Entry::TurnSeparator(_) => {
                return div().w_full().h(s.px(TURN_GAP)).into_any_element();
            }
        };
        let top = match entry {
            Entry::UserMessage(_) | Entry::AgentText(_) => MESSAGE_GAP,
            _ => ROW_GAP,
        };
        div().w_full().pt(s.px(top)).child(inner).into_any_element()
    }

    /// What the user said: a bubble against the right edge, tinted with
    /// `text.accent` (14 % fill over `bg.app`, 1 px border at 35 %), radius 8,
    /// at most 85 % of the panel wide. No "Vos" label: the shape and the tint
    /// are the whole distinction (`docs/etapas/etapa-3.md` § correcciones).
    ///
    /// The text is **Markdown**, painted by the same renderer as the agent's
    /// answers (lists, bold, inline code, fenced blocks with the code-block
    /// look). A paragraph that holds a mention is the exception: it is laid
    /// out as a flow of plain text and inline chips, the chip painted **where
    /// it was typed** (`docs/etapas/etapa-2.md` § correcciones); every other
    /// paragraph goes through Markdown ([`user_pieces`]).
    ///
    /// The text wraps at the bubble's width, and a long token with no spaces
    /// breaks mid-word: every piece is `min_w_0`, since the min-content width
    /// of a gpui text is its unwrapped width, and the bubble clips whatever
    /// still overflows.
    fn render_user_message(
        &self,
        index: usize,
        message: &UserMessage,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        let root = self.project_root().map(Path::to_path_buf);
        let pieces: Vec<AnyElement> = user_pieces(&message.blocks)
            .into_iter()
            .enumerate()
            .map(|(position, piece)| match piece {
                UserPiece::Markdown(text) => {
                    let view = self.bubble_view(index, position, &text, cx);
                    let panel = cx.entity().downgrade();
                    let on_copy: crate::markdown::CopyHandler =
                        Arc::new(move |code: String, cx: &mut App| {
                            panel.update(cx, |panel, cx| panel.copy(code, cx)).ok();
                        });
                    div()
                        .min_w(px(0.))
                        .max_w_full()
                        .child(markdown_element_scaled(
                            &view,
                            &theme,
                            s.scale,
                            self.highlighter(),
                            on_copy,
                        ))
                        .into_any_element()
                }
                UserPiece::Flow(parts) => {
                    // The chip carries its own margins instead of the row
                    // carrying a gap, so `@main.rs, ¿qué te parece?` keeps
                    // the comma glued to the chip the way the author typed it.
                    let count = parts.len();
                    let items: Vec<AnyElement> = parts
                        .iter()
                        .enumerate()
                        .filter_map(|(part_ix, part)| match part {
                            FlowPart::Text(text) => {
                                let text = text.trim();
                                (!text.is_empty()).then(|| {
                                    div()
                                        .min_w(px(0.))
                                        .max_w_full()
                                        .whitespace_normal()
                                        .child(SharedString::from(text.to_string()))
                                        .into_any_element()
                                })
                            }
                            FlowPart::File(path) => {
                                let glued = matches!(
                                    parts.get(part_ix + 1),
                                    Some(FlowPart::Text(text))
                                        if text.trim_start().starts_with(is_closing_punctuation)
                                );
                                Some(
                                    file_chip(
                                        ("message-chip", index * 4096 + position * 64 + part_ix),
                                        path,
                                        root.as_deref(),
                                        &theme,
                                        &s,
                                    )
                                    .when(part_ix > 0, |this| this.ml_1())
                                    .when(!glued && part_ix + 1 < count, |this| this.mr_1())
                                    .into_any_element(),
                                )
                            }
                        })
                        .collect();
                    h_flex()
                        .min_w(px(0.))
                        .max_w_full()
                        .flex_wrap()
                        .items_center()
                        .children(items)
                        .into_any_element()
                }
            })
            .collect();

        h_flex()
            .id(("user-message", index))
            .w_full()
            .justify_end()
            .child(
                v_flex()
                    .id(("user-bubble", index))
                    .debug_selector(|| format!("user-bubble-{index}"))
                    .max_w(relative(BUBBLE_MAX_WIDTH))
                    .min_w(px(0.))
                    .overflow_hidden()
                    .px(s.px(10.))
                    .py(s.px(7.))
                    .gap(s.px(6.))
                    .rounded(s.px(BUBBLE_RADIUS))
                    .bg(alpha(theme.text_accent, BUBBLE_FILL_ALPHA))
                    .border_1()
                    .border_color(alpha(theme.text_accent, BUBBLE_BORDER_ALPHA))
                    .text_color(theme.text)
                    .child(
                        v_flex()
                            .debug_selector(|| format!("user-text-{index}"))
                            .min_w(px(0.))
                            .max_w_full()
                            .gap(s.px(6.))
                            .children(pieces),
                    ),
            )
            .into_any_element()
    }

    /// The Markdown state of piece `position` of entry `index`, built once per
    /// text and kept across frames.
    fn bubble_view(
        &self,
        index: usize,
        position: usize,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Entity<gpui_kit::base::text::TextViewState> {
        let mut views = self.bubble_views.borrow_mut();
        if let Some((built_from, view)) = views.get(&(index, position))
            && built_from == text
        {
            return view.clone();
        }
        let view = crate::markdown::markdown_state(text, cx);
        views.insert((index, position), (text.to_string(), view.clone()));
        view
    }

    /// What the agent answered: plain text on the panel background, full
    /// width, no bubble, no rule and no name above it. Its code blocks bring
    /// their own `bg.editor` box ([`crate::markdown::code_block_style`]).
    fn render_agent_text(
        &self,
        index: usize,
        text: &AgentText,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        let body: AnyElement = match text.view.clone() {
            Some(view) => {
                let panel = cx.entity().downgrade();
                let on_copy: crate::markdown::CopyHandler =
                    Arc::new(move |code: String, cx: &mut App| {
                        panel.update(cx, |panel, cx| panel.copy(code, cx)).ok();
                    });
                markdown_element_scaled(&view, &theme, s.scale, self.highlighter(), on_copy)
                    .into_any_element()
            }
            None => div()
                .whitespace_normal()
                .child(SharedString::from(text.markdown.clone()))
                .into_any_element(),
        };

        div()
            .id(("agent-text", index))
            .debug_selector(|| format!("agent-text-{index}"))
            .w_full()
            .min_w(px(0.))
            .px_0p5()
            .text_color(theme.text)
            .child(body)
            .into_any_element()
    }

    fn render_thought(
        &self,
        index: usize,
        thought: &AgentThought,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        // The spinner is the point: without it the user cannot tell a thinking
        // agent from a stuck one.
        let live = self.thought_started.is_some() && index + 1 == self.entries.len();
        let summary = SharedString::from(format!("Pensando… ({} s)", thought.duration_secs));
        let collapsed = thought.collapsed;
        let body = (!collapsed).then(|| {
            div()
                .ml_4()
                .px_2()
                .py_1()
                .rounded(s.px(CARD_RADIUS))
                .bg(theme.bg_surface)
                .text_size(s.px(TEXT_SMALL))
                .text_color(theme.text_muted)
                .whitespace_normal()
                .child(SharedString::from(thought.text.clone()))
        });

        v_flex()
            .id(("thought", index))
            .w_full()
            .gap_0p5()
            .child(
                h_flex()
                    .id(("thought-toggle", index))
                    .h(s.px(TOOL_ROW_HEIGHT))
                    .px_1()
                    .gap_1()
                    .items_center()
                    .rounded(s.px(CARD_RADIUS))
                    .cursor_pointer()
                    .text_size(s.px(TEXT_SMALL))
                    .text_color(theme.text_muted)
                    .hover(move |style| style.bg(theme.bg_surface))
                    .child(if collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .child(if live {
                        div().child(Spinner::new().xsmall().color(theme.text_accent))
                    } else {
                        div().text_color(theme.text_muted).child(IconName::Brain)
                    })
                    .child(div().child(summary))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.toggle_entry(index, cx);
                    })),
            )
            .children(body)
            .into_any_element()
    }

    fn render_tool_call(
        &self,
        index: usize,
        call: &ToolCallEntry,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if call.kind == ToolKind::Execute {
            return self.render_command_card(index, call, theme, cx);
        }
        let theme = *theme;
        let s = *self.settings();
        let detail = call
            .expanded
            .then(|| self.render_tool_detail(index, call, &theme, cx));

        v_flex()
            .id(("tool", index))
            .w_full()
            .child(
                h_flex()
                    .id(("tool-row", index))
                    .h(s.px(TOOL_ROW_HEIGHT))
                    .px_1()
                    .gap_1p5()
                    .items_center()
                    .rounded(s.px(CARD_RADIUS))
                    .text_size(s.px(TEXT_SMALL))
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.bg_surface))
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .child(tool_kind_icon(call.kind)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(SharedString::from(one_line_summary(&call.title))),
                    )
                    .children(tool_stat(call, &theme, &s))
                    .child(status_glyph(call.status, &theme, &s))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.toggle_entry(index, cx);
                    })),
            )
            .children(detail)
            .into_any_element()
    }

    /// The `execute` card: a terminal, not a row of prose.
    ///
    /// The header is the command with a shell prompt in front of it
    /// (`$ ls -la`), monospaced; the output below is monospaced and muted, cut
    /// at [`MAX_OUTPUT_LINES`] lines, with the agent's ```` ``` ```` fences
    /// already stripped by [`crate::model::strip_code_fences`]. A failure adds
    /// the exit code when the output says which one it was
    /// (`docs/etapas/etapa-2.md` § correcciones).
    fn render_command_card(
        &self,
        index: usize,
        call: &ToolCallEntry,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        let command = call
            .content
            .iter()
            .find_map(|content| match content {
                ToolContent::Command { command, .. } => Some(command.clone()),
                _ => None,
            })
            .unwrap_or_else(|| call.title.clone());
        let output = call
            .content
            .iter()
            .find_map(|content| match content {
                ToolContent::Command { output, .. } => Some(output.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let expanded = call.expanded;
        let failed = call.status == ToolCallStatus::Failed;
        let exit_code = failed
            .then(|| exit_code_in(&output))
            .flatten()
            .map(|code| SharedString::from(format!("salió {code}")));
        // A multi-line command (a `python3 - <<'EOF'` heredoc, say) must not
        // spill its extra lines past the one-line row: the text layout
        // breaks at an embedded newline regardless of `.truncate()`'s
        // `whitespace_nowrap`, painting them over the cards below
        // (`docs/etapas/etapa-2.md` § correcciones). The row shows only the
        // first non-empty line plus a line count; the full command, with its
        // own line breaks, only shows once expanded.
        let multiline_command = command.lines().count() > 1;
        let command_row_text = one_line_summary(&command);

        v_flex()
            .id(("command", index))
            .w_full()
            .rounded(s.px(COMMAND_CARD_RADIUS))
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_surface)
            .child(
                h_flex()
                    .id(("command-row", index))
                    .debug_selector(move || format!("command-row-{index}"))
                    .h(s.px(TOOL_ROW_HEIGHT))
                    .px_1p5()
                    .gap_1p5()
                    .items_center()
                    .text_size(s.px(TEXT_SMALL))
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.bg_elevated))
                    .child(
                        div()
                            .text_color(theme.text_accent)
                            .child(IconName::SquareTerminal),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .font_family(mono_family(cx))
                            .text_size(s.px(TEXT_CODE))
                            .child(SharedString::from(format!("$ {command_row_text}"))),
                    )
                    .children(exit_code.map(|label| {
                        div()
                            .flex_shrink_0()
                            .text_size(s.px(TEXT_LABEL))
                            .text_color(theme.status_error)
                            .child(label)
                    }))
                    .child(status_glyph(call.status, &theme, &s))
                    .child(div().text_color(theme.text_muted).child(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.toggle_entry(index, cx);
                    })),
            )
            .when(expanded && multiline_command, |this| {
                this.child(
                    div()
                        .id(("command-full", index))
                        .debug_selector(move || format!("command-full-{index}"))
                        .w_full()
                        .px_1p5()
                        .py_1()
                        .border_t_1()
                        .border_color(theme.border)
                        .bg(theme.bg_editor)
                        .font_family(mono_family(cx))
                        .text_size(s.px(TEXT_CODE))
                        .text_color(theme.text)
                        .child(SharedString::from(command.clone())),
                )
            })
            .when(expanded && !output.is_empty(), |this| {
                this.child(self.render_output(index, &output, call.output_expanded, &theme, cx))
            })
            .into_any_element()
    }

    /// A command's output: monospaced, muted, and at most
    /// [`MAX_OUTPUT_LINES`] lines before "ver más".
    fn render_output(
        &self,
        index: usize,
        output: &str,
        output_expanded: bool,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        let lines: Vec<&str> = output.lines().collect();
        let hidden = lines.len().saturating_sub(MAX_OUTPUT_LINES);
        let shown: String = if output_expanded || hidden == 0 {
            output.to_string()
        } else {
            lines[..MAX_OUTPUT_LINES].join("\n")
        };
        let more = (hidden > 0).then(|| {
            let label = if output_expanded {
                "ver menos".to_string()
            } else {
                format!("ver más ({hidden} líneas)")
            };
            div()
                .id(("command-more", index))
                .px_1p5()
                .py_0p5()
                .text_size(s.px(TEXT_SMALL))
                .text_color(theme.text_accent)
                .cursor_pointer()
                .child(SharedString::from(label))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.toggle_tool_output(index, cx);
                }))
        });

        v_flex()
            .id(("command-output", index))
            .w_full()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .w_full()
                    .px_1p5()
                    .py_1()
                    .bg(theme.bg_editor)
                    .font_family(mono_family(cx))
                    .text_size(s.px(TEXT_CODE))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(shown)),
            )
            .children(more)
            .into_any_element()
    }

    /// The detail of an expanded tool card.
    ///
    /// `chat.md`: an edit **never** shows the full diff here, only the first
    /// lines of context and the button that opens the editor.
    fn render_tool_detail(
        &self,
        index: usize,
        call: &ToolCallEntry,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        let root = self.project_root().map(Path::to_path_buf);
        let rows: Vec<AnyElement> = call
            .content
            .iter()
            .enumerate()
            .map(|(position, content)| match content {
                ToolContent::Text(text) => mono_block(text, &theme, &s, cx).into_any_element(),
                ToolContent::Command { command, output } => v_flex()
                    .gap_1()
                    .child(mono_block(&format!("$ {command}"), &theme, &s, cx))
                    .when(!output.is_empty(), |this| {
                        this.child(mono_block(output, &theme, &s, cx))
                    })
                    .into_any_element(),
                ToolContent::Edit { path, preview } => {
                    let open_path = path.clone();
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_color(theme.text_muted)
                                .text_size(s.px(TEXT_LABEL))
                                .child(relative_label(root.as_deref(), path)),
                        )
                        .children(
                            preview
                                .iter()
                                .map(|line| mono_block(line, &theme, &s, cx).into_any_element()),
                        )
                        .child(
                            div()
                                .text_color(theme.text_muted)
                                .text_size(s.px(TEXT_SMALL))
                                .child("Revisá en el editor"),
                        )
                        .child(
                            Button::new(("open-hunk", index * 16 + position))
                                .label("Ver en el editor")
                                .small()
                                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                    this.open_file_at_hunk(open_path.clone(), cx);
                                })),
                        )
                        .into_any_element()
                }
            })
            .collect();

        v_flex()
            .id(("tool-detail", index))
            .w_full()
            .ml_4()
            .p_2()
            .gap_1()
            .rounded(s.px(CARD_RADIUS))
            .bg(theme.bg_surface)
            .children(rows)
            .into_any_element()
    }

    fn render_plan(
        &self,
        index: usize,
        plan: &PlanEntry,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        let collapsed = plan.collapsed;
        let done = plan
            .items
            .iter()
            .filter(|item| item.status == PlanEntryStatus::Completed)
            .count();
        let header = SharedString::from(format!("Plan · {done}/{}", plan.items.len()));

        v_flex()
            .id(("plan", index))
            .w_full()
            .p_2()
            .gap_1()
            .rounded(s.px(CARD_RADIUS))
            .bg(theme.bg_surface)
            .text_size(s.px(TEXT_SMALL))
            .child(
                h_flex()
                    .id(("plan-toggle", index))
                    .gap_1()
                    .items_center()
                    .cursor_pointer()
                    .hover(move |style| style.text_color(theme.text_accent))
                    .child(if collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .child(div().font_weight(FontWeight::MEDIUM).child(header))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.toggle_entry(index, cx);
                    })),
            )
            .when(!collapsed, |this| {
                this.children(plan.items.iter().map(|item| {
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .child(plan_glyph(item.status.clone(), &theme, &s))
                        .child(
                            div()
                                .flex_1()
                                .child(SharedString::from(item.text.clone()))
                                .when(item.status == PlanEntryStatus::Completed, |this| {
                                    this.text_color(theme.text_muted)
                                }),
                        )
                }))
            })
            .into_any_element()
    }

    fn render_permission(
        &self,
        index: usize,
        permission: &PermissionEntry,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        // Answered: one line, as `chat.md` asks.
        if let Some(answer) = permission.answered.clone() {
            let color = if permission.allowed {
                theme.status_ok
            } else {
                theme.text_muted
            };
            return h_flex()
                .id(("permission-done", index))
                .gap_1()
                .px_1()
                .text_size(s.px(TEXT_SMALL))
                .text_color(color)
                .child(if permission.allowed {
                    IconName::Check
                } else {
                    IconName::X
                })
                .child(SharedString::from(format!(
                    "{} · {answer}",
                    permission.title
                )))
                .into_any_element();
        }

        let buttons: Vec<AnyElement> = permission
            .options
            .iter()
            .enumerate()
            .map(|(position, option)| {
                let id: PermissionOptionId = option.id.clone();
                let button = Button::new(("permission", index * 16 + position))
                    .label(SharedString::from(option.name.clone()))
                    .small()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.answer_permission(&id, cx);
                    }));
                // The first option is the default, the one `Enter` chooses.
                if position == 0 {
                    button.primary().into_any_element()
                } else {
                    button.outline().into_any_element()
                }
            })
            .collect();

        v_flex()
            .id(("permission", index))
            .w_full()
            .p_2()
            .gap_2()
            .rounded(s.px(CARD_RADIUS))
            .bg(theme.bg_surface)
            .border_1()
            .border_color(theme.border_focus)
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child(SharedString::from(permission.title.clone())),
            )
            .when_some(permission.detail.clone(), |this, detail| {
                let expanded = permission.expanded;
                this.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .id(("permission-detail-toggle", index))
                                .gap_1()
                                .items_center()
                                .text_size(s.px(TEXT_LABEL))
                                .text_color(theme.text_muted)
                                .cursor_pointer()
                                .child(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .child("Detalle")
                                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                    this.toggle_entry(index, cx);
                                })),
                        )
                        .when(expanded, |this| {
                            this.child(mono_block(&detail, &theme, &s, cx))
                        }),
                )
            })
            .child(h_flex().gap_1().flex_wrap().children(buttons))
            .child(
                div()
                    .text_size(s.px(TEXT_SMALL))
                    .text_color(theme.text_muted)
                    .child("Enter acepta la primera · Esc rechaza"),
            )
            .into_any_element()
    }

    fn render_auth(
        &self,
        index: usize,
        auth: &AuthEntry,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let s = *self.settings();
        let command = auth.command().unwrap_or_default().to_string();
        let retry_id = auth
            .methods
            .first()
            .map(|method| method.id.clone())
            .unwrap_or_default();

        v_flex()
            .id(("auth", index))
            .w_full()
            .p_2()
            .gap_2()
            .rounded(s.px(CARD_RADIUS))
            .bg(theme.bg_surface)
            .border_1()
            .border_color(theme.status_warning)
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Hace falta autenticarse"),
            )
            .when(!command.is_empty(), |this| {
                this.child(mono_block(&command, &theme, &s, cx))
            })
            .child(
                h_flex()
                    .gap_1()
                    .child({
                        let command = command.clone();
                        Button::new(("auth-copy", index))
                            .label("Copiar")
                            .small()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.copy(command.clone(), cx);
                            }))
                    })
                    .child({
                        let command = command.clone();
                        Button::new(("auth-terminal", index))
                            .label("Abrir terminal")
                            .small()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.open_terminal(command.clone(), cx);
                            }))
                    })
                    .child(
                        Button::new(("auth-retry", index))
                            .label("Reintentar")
                            .small()
                            .primary()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.authenticate(retry_id.clone(), cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    // ------------------------------------------------------------ composer

    /// The composer: selectors on their own row, then the input box with the
    /// send button inside its right edge, then the hint line (`02-visual.md`
    /// §7). Three separated things instead of one undifferentiated block.
    ///
    /// The input is an `cincel_editor::EditorView` configured as a Markdown
    /// text field ([`crate::panel::composer_settings`]): it grows from one to
    /// eight rows and then scrolls, and paints the Markdown structure with the
    /// syntax tokens ([`crate::composer`]). The box takes the editor's own
    /// actions in the **capture** phase, before the editor handles them:
    /// `editor::insert_newline` (`Enter`) confirms the open `@` / `/` popover
    /// or sends; `editor::move_up` / `move_down`, `editor::tab` and
    /// `editor::cancel` belong to the popover while it lists something and fall
    /// through to the editor otherwise. `Shift+Enter` has no editor binding, so
    /// it reaches `chat::newline`, which inserts the line break.
    fn render_composer(
        &self,
        theme: &ChatTheme,
        overlay: Option<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *theme;
        let s = *self.settings();
        let blocked = self.is_awaiting_permission();
        let thinking = self.status() == AgentStatus::Thinking;
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);

        // No strip of chips above the input any more: a mention is the
        // `@archivo` token the user can see and edit inside the text
        // (`docs/etapas/etapa-2.md` § correcciones).
        v_flex()
            .id("chat-composer")
            .debug_selector(|| "chat-composer-box".to_string())
            .relative()
            .flex_shrink_0()
            .w_full()
            .p_2()
            .gap_1p5()
            .border_t_1()
            .border_color(theme.border)
            .child(self.render_selectors(&theme, cx))
            .child(
                h_flex()
                    .id("chat-input-box")
                    .debug_selector(|| "chat-input-box".to_string())
                    .key_context(actions::COMPOSER_NODE)
                    .capture_action(cx.listener(Self::on_composer_enter))
                    .capture_action(cx.listener(Self::on_composer_up))
                    .capture_action(cx.listener(Self::on_composer_down))
                    .capture_action(cx.listener(Self::on_composer_tab))
                    .capture_action(cx.listener(Self::on_composer_escape))
                    .w_full()
                    .min_h(s.px(INPUT_MIN_HEIGHT))
                    .pl(s.px(8.))
                    .pr_1()
                    .py(s.px(6.))
                    .gap_1()
                    .items_end()
                    .rounded(s.px(INPUT_RADIUS))
                    .bg(theme.bg_surface)
                    .border_1()
                    .border_color(if focused {
                        theme.border_focus
                    } else {
                        theme.border
                    })
                    .child(
                        div()
                            .id("chat-input")
                            .flex_1()
                            .min_w(px(0.))
                            .self_center()
                            .child(self.input.clone()),
                    )
                    .child(if thinking {
                        Button::new("chat-stop")
                            .icon(IconName::Square)
                            .small()
                            .tooltip("Detener el turno")
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                this.cancel_turn(cx);
                            }))
                    } else {
                        Button::new("chat-send")
                            .icon(IconName::SendHorizontal)
                            .small()
                            .primary()
                            .disabled(blocked)
                            .tooltip("Enviar (Enter)")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.send(window, cx);
                            }))
                    }),
            )
            .child(
                div()
                    .text_size(s.px(TEXT_SMALL))
                    .text_color(theme.text_muted)
                    .child(COMPOSER_HINT),
            )
            .children(overlay)
    }

    /// `Enter` in the composer (the editor's `insert_newline`, taken before
    /// the editor sees it): answers a pending permission, confirms the
    /// popover's row, or sends.
    fn on_composer_enter(
        &mut self,
        _: &cincel_editor::InsertNewline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if self.is_awaiting_permission() {
            // `Enter` answers the permission with the agent's first option
            // (`chat.md`), as `Chat && permission` does outside the composer.
            self.accept_permission(cx);
        } else if self.popover_confirms() {
            self.confirm_popover(window, cx);
        } else {
            self.send(window, cx);
        }
    }

    /// `↑` in the composer: the popover's while it lists something.
    fn on_composer_up(
        &mut self,
        _: &cincel_editor::MoveUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.popover_rows() > 0 {
            cx.stop_propagation();
            self.move_popover_selection(-1, cx);
        }
    }

    /// `↓` in the composer: the popover's while it lists something.
    fn on_composer_down(
        &mut self,
        _: &cincel_editor::MoveDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.popover_rows() > 0 {
            cx.stop_propagation();
            self.move_popover_selection(1, cx);
        }
    }

    /// `Tab` in the composer: confirms the popover's row without sending.
    fn on_composer_tab(
        &mut self,
        _: &cincel_editor::Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.popover_confirms() {
            cx.stop_propagation();
            self.confirm_popover(window, cx);
        }
    }

    /// `Esc` in the composer: closes the popover; with nothing open it is
    /// `chat::cancel_turn` (which answers a pending permission with its
    /// reject option), and the editor still drops its selection.
    fn on_composer_escape(
        &mut self,
        _: &cincel_editor::Cancel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.popover() != &Popover::Closed {
            cx.stop_propagation();
            self.close_popover(cx);
            return;
        }
        self.cancel_turn(cx);
    }

    /// The row of compact selectors above the input (`02-visual.md` §7).
    fn render_selectors(&self, theme: &ChatTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *theme;
        let s = *self.settings();
        // Only the agent's own selectors (its modes, model and effort, from
        // ACP): Cincel has no autonomy selector of its own any more, the
        // permission policy is fixed (`cincel_acp::PermissionPolicy`).
        // One chip tall even when the agent announced no selector, so the
        // composer does not jump when the first selector arrives.
        let mut row = h_flex()
            .id("chat-selectors")
            .min_h(s.px(SELECTOR_ROW_HEIGHT))
            .gap_1()
            .flex_wrap();

        for (index, option) in self.config_options.iter().enumerate() {
            if !is_footer_option(option) {
                continue;
            }
            row = row.child(selector(
                ("config", index),
                current_label(option),
                &theme,
                &s,
                cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.toggle_popover(Popover::Config(index), cx);
                }),
            ));
        }

        // Legacy `availableModes` for agents without `configOptions`.
        if self.config_options.is_empty()
            && let Some(modes) = self.modes.as_ref()
        {
            let current = modes
                .available_modes
                .iter()
                .find(|mode| mode.id == modes.current_mode_id)
                .map_or_else(
                    || modes.current_mode_id.0.to_string(),
                    |mode| mode.name.clone(),
                );
            row = row.child(selector(
                "modes",
                current,
                &theme,
                &s,
                cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.toggle_popover(Popover::Modes, cx);
                }),
            ));
        }
        row
    }

    // ------------------------------------------------------------- overlay

    /// The history popover: the directory's conversations grouped by
    /// connection (its label is the group header; conversations from before
    /// connections go under [`LEGACY_CONNECTION_GROUP`]), newest first inside
    /// each group,
    /// each row showing its title, its relative time and a ✕
    /// (`docs/etapas/etapa-2.md` § correcciones).
    fn render_history_rows(&self, theme: &ChatTheme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = *theme;
        let s = *self.settings();
        // The workspace hands them over newest first; grouping keeps that
        // order both between groups (by their newest conversation) and inside.
        let mut groups: Vec<&str> = Vec::new();
        for conversation in &self.conversations {
            let group = group_of(conversation);
            if !groups.contains(&group) {
                groups.push(group);
            }
        }

        let mut rows: Vec<AnyElement> = Vec::new();
        for agent in groups {
            rows.push(
                div()
                    .px_1p5()
                    .pt_1()
                    .pb_0p5()
                    .text_size(s.px(TEXT_LABEL))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(SharedString::from(agent.to_string()))
                    .into_any_element(),
            );
            for (index, conversation) in self
                .conversations
                .iter()
                .enumerate()
                .filter(|(_, conversation)| group_of(conversation) == agent)
            {
                let open_id = conversation.id.clone();
                let delete_id = conversation.id.clone();
                let selected = self.active_conversation() == Some(conversation.id.as_str());
                rows.push(
                    h_flex()
                        .id(("conversation", index))
                        .h(s.px(POPOVER_ROW_HEIGHT))
                        .px_1p5()
                        .gap_1p5()
                        .items_center()
                        .rounded(s.px(CARD_RADIUS))
                        .cursor_pointer()
                        .when(selected, |this| this.bg(theme.selection))
                        .hover(move |style| style.bg(theme.selection))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .child(SharedString::from(conversation.title.clone())),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(s.px(TEXT_LABEL))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(conversation.when.clone())),
                        )
                        .child(
                            div()
                                .id(("conversation-delete", index))
                                .flex_shrink_0()
                                .aria_label("Eliminar conversación")
                                .text_color(theme.text_muted)
                                .hover(move |style| style.text_color(theme.status_error))
                                .child(IconName::X)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                    cx.stop_propagation();
                                    this.delete_conversation(&delete_id, cx);
                                })),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            cx.stop_propagation();
                            this.open_conversation(&open_id, cx);
                        }))
                        .into_any_element(),
                );
            }
        }
        rows
    }

    fn render_overlay(&self, theme: &ChatTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = *theme;
        let s = *self.settings();
        let root = self.project_root().map(Path::to_path_buf);
        let footer = matches!(self.popover(), Popover::Connections)
            .then(|| self.render_connection_footer(&theme, cx));
        let rows: Vec<AnyElement> = match self.popover().clone() {
            Popover::Closed => return None,
            Popover::Connections => self.render_connection_rows(&theme, cx),
            Popover::ConnectionMenu(id) => vec![
                popover_row("connection-rename", "Renombrar", false, false, &theme, &s)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        cx.stop_propagation();
                        this.request_rename_connection(&id, cx);
                    }))
                    .into_any_element(),
            ],
            Popover::Conversations => self.render_history_rows(&theme, cx),
            Popover::Files { selected, .. } => self
                .filtered_files()
                .into_iter()
                .enumerate()
                .map(|(index, path)| {
                    let owned = path.clone();
                    file_popover_row(index, &path, root.as_deref(), index == selected, &theme, &s)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            // The click belongs to the popover and to nothing
                            // else: it inserts the chip, it never sends.
                            cx.stop_propagation();
                            this.insert_mention(owned.clone(), window, cx);
                        }))
                        .into_any_element()
                })
                .collect(),
            Popover::Commands { selected, .. } => self
                .filtered_commands()
                .into_iter()
                .enumerate()
                .map(|(index, command)| {
                    let name = command.name.clone();
                    popover_row(
                        ("command", index),
                        format!("/{} — {}", command.name, command.description),
                        index == selected,
                        false,
                        &theme,
                        &s,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.insert_command(&name, window, cx);
                    }))
                    .into_any_element()
                })
                .collect(),
            Popover::Config(option_index) => {
                let option = self.config_options.get(option_index)?;
                let config_id: SessionConfigId = option.id.clone();
                let current = current_label(option);
                select_options(option)
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let config_id = config_id.clone();
                        let chosen: SessionConfigValueId = value.value.clone();
                        popover_row(
                            ("config-value", option_index * 64 + index),
                            value.name.clone(),
                            value.name == current,
                            false,
                            &theme,
                            &s,
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            cx.stop_propagation();
                            this.choose_config_value(&config_id, chosen.clone(), cx);
                        }))
                        .into_any_element()
                    })
                    .collect()
            }
            Popover::Modes => {
                let modes = self.modes.as_ref()?;
                modes
                    .available_modes
                    .iter()
                    .enumerate()
                    .map(|(index, mode)| {
                        let id: SessionModeId = mode.id.clone();
                        let selected = mode.id == modes.current_mode_id;
                        popover_row(
                            ("mode", index),
                            mode.name.clone(),
                            selected,
                            false,
                            &theme,
                            &s,
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            cx.stop_propagation();
                            this.choose_mode(id.clone(), cx);
                        }))
                        .into_any_element()
                    })
                    .collect()
            }
        };

        let anchored_to_header = matches!(
            self.popover(),
            Popover::Connections | Popover::ConnectionMenu(_) | Popover::Conversations
        );
        let tall = matches!(self.popover(), Popover::Connections);
        let panel = v_flex()
            .id("chat-popover")
            .debug_selector(|| "chat-popover".to_string())
            .absolute()
            .left_2()
            .right_2()
            .map(|this| {
                if anchored_to_header {
                    this.top(s.px(HEADER_HEIGHT + 4.))
                } else {
                    // Inside the composer: its bottom edge on the composer's
                    // top edge.
                    this.bottom(relative(1.))
                }
            })
            .max_h(if tall {
                s.px(CONNECTION_POPOVER_MAX_HEIGHT)
            } else {
                s.px(POPOVER_ROW_HEIGHT * 10. + 8.)
            })
            .overflow_y_scroll()
            .p_1()
            .rounded(s.px(POPOVER_RADIUS))
            .bg(theme.bg_elevated)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .when(rows.is_empty() && footer.is_none(), |this| {
                this.child(
                    div()
                        .px_1()
                        .text_color(theme.text_muted)
                        .child("Sin resultados"),
                )
            })
            .children(rows)
            .children(footer);
        Some(panel.into_any_element())
    }
}

// ------------------------------------------------------------------ helpers

/// Maximum height of the "Conectar" popover (its rows are two lines tall).
const CONNECTION_POPOVER_MAX_HEIGHT: f32 = 380.;

/// The group header a history row is listed under.
fn group_of(conversation: &ConversationSummary) -> &str {
    if conversation.group.is_empty() {
        &conversation.agent_name
    } else {
        &conversation.group
    }
}

/// The provider icon of a connection: the provider's monogram in a small
/// rounded square ([`provider_monogram`]). Shared with the workspace's status
/// bar chip, which passes its own colours.
pub fn provider_icon(
    agent_id: &str,
    size: gpui::Pixels,
    fg: gpui::Hsla,
    bg: gpui::Hsla,
    border: gpui::Hsla,
) -> gpui::Div {
    div()
        .flex_shrink_0()
        .size(size)
        .flex()
        .items_center()
        .justify_center()
        .rounded(size * 0.25)
        .bg(bg)
        .border_1()
        .border_color(border)
        .text_size(size * 0.62)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg)
        .child(provider_monogram(agent_id))
}

/// The status pill of the header (`02-visual.md` §7).
fn status_pill(status: AgentStatus, theme: &ChatTheme, s: &ChatSettings) -> impl IntoElement {
    let color = match status {
        AgentStatus::Ready => theme.status_ok,
        AgentStatus::Thinking => theme.text_accent,
        AgentStatus::WaitingPermission | AgentStatus::AuthRequired => theme.status_warning,
        AgentStatus::Disconnected => theme.status_error,
    };
    h_flex()
        .gap_1()
        .px_1p5()
        .py_0p5()
        .rounded(s.px(CARD_RADIUS))
        .bg(alpha(color, 0.15))
        .text_size(s.px(TEXT_LABEL))
        .text_color(color)
        .child(status.label())
}

/// A header icon button.
fn icon_button(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    theme: &ChatTheme,
    s: &ChatSettings,
) -> gpui::Stateful<gpui::Div> {
    let theme = *theme;
    div()
        .id(id)
        .aria_label(label)
        .size(s.px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(s.px(CARD_RADIUS))
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(move |style| style.bg(theme.bg_surface).text_color(theme.text))
        .child(icon)
}

/// A compact selector chip of the composer (`Sonnet ▾`): one of the agent's
/// own ACP selectors (mode, model, effort).
///
/// A bordered chip instead of bare text: the row above the input has to read
/// as controls, not as a caption of the message.
fn selector(
    id: impl Into<gpui::ElementId>,
    label: impl Into<String>,
    theme: &ChatTheme,
    s: &ChatSettings,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let theme = *theme;
    h_flex()
        .id(id.into())
        .gap_0p5()
        .px_1p5()
        .py_0p5()
        .rounded(s.px(CARD_RADIUS))
        .bg(theme.bg_surface)
        .border_1()
        .border_color(theme.border)
        .text_size(s.px(TEXT_LABEL))
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(move |style| style.bg(theme.bg_elevated).text_color(theme.text))
        .child(SharedString::from(label.into()))
        .child(IconName::ChevronDown)
        .on_click(on_click)
}

/// One row of an overlay list.
fn popover_row(
    id: impl Into<gpui::ElementId>,
    label: impl Into<String>,
    selected: bool,
    dimmed: bool,
    theme: &ChatTheme,
    s: &ChatSettings,
) -> gpui::Stateful<gpui::Div> {
    let theme = *theme;
    div()
        .id(id.into())
        .h(s.px(POPOVER_ROW_HEIGHT))
        .px_1p5()
        .flex()
        .items_center()
        .rounded(s.px(CARD_RADIUS))
        .cursor_pointer()
        .text_color(if dimmed { theme.text_muted } else { theme.text })
        .when(selected, |this| this.bg(theme.selection))
        .hover(move |style| style.bg(theme.selection))
        .child(SharedString::from(label.into()))
}

/// One row of the `@` picker: the file name, then its directory in muted text.
///
/// An absolute path fills the popover with `/home/…/crates/…` and hides the one
/// word the user is looking for, so the name leads and the directory follows.
fn file_popover_row(
    index: usize,
    path: &Path,
    root: Option<&Path>,
    selected: bool,
    theme: &ChatTheme,
    s: &ChatSettings,
) -> gpui::Stateful<gpui::Div> {
    let theme = *theme;
    let parent = parent_label(root, path);
    div()
        .id(("file", index))
        .h(s.px(POPOVER_ROW_HEIGHT))
        .px_1p5()
        .flex()
        .flex_row()
        .items_center()
        .gap_1p5()
        .rounded(s.px(CARD_RADIUS))
        .cursor_pointer()
        .when(selected, |this| this.bg(theme.selection))
        .hover(move |style| style.bg(theme.selection))
        .child(
            div()
                .flex_shrink_0()
                .text_color(theme.text)
                .child(file_name_label(path)),
        )
        .when(!parent.is_empty(), |this| {
            this.child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(s.px(TEXT_LABEL))
                    .text_color(theme.text_muted)
                    // A path component may legally hold a newline on Linux;
                    // the same one-line rule as a tool call's title applies.
                    .child(SharedString::from(one_line_summary(&parent))),
            )
        })
}

/// The `@archivo` chip of a user message, with its relative path as tooltip.
fn file_chip(
    id: impl Into<gpui::ElementId>,
    path: &Path,
    root: Option<&Path>,
    theme: &ChatTheme,
    s: &ChatSettings,
) -> gpui::Stateful<gpui::Div> {
    let tooltip = relative_label(root, path);
    div()
        .id(id.into())
        .px_1p5()
        .py_0p5()
        .rounded(s.px(CARD_RADIUS))
        .bg(theme.bg_elevated)
        .text_size(s.px(TEXT_SMALL))
        .text_color(theme.text_accent)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(chip_label(path))
}

/// Punctuation that belongs to the word before it, so a chip followed by it
/// gets no trailing margin.
fn is_closing_punctuation(ch: char) -> bool {
    matches!(
        ch,
        ',' | '.' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '»' | '…' | '\''
    )
}

/// The monospace family of the code in the chat: the one `gpui-kit` paints
/// fenced code blocks with (the workspace points it at the buffer font), so
/// a command, an output and a code block share one face. A bare
/// `"monospace"` is not a family gpui resolves.
/// The first non-empty line of possibly multi-line text, followed by "… N
/// líneas" when there was more than one line. A one-line row painted with
/// `.truncate()` still breaks visually at an embedded newline (GPUI's text
/// layout honours `\n` regardless of `whitespace_nowrap`), so any text that
/// might carry line breaks — a shell command, a tool call's title — must be
/// reduced to a single line before it reaches such a row. The text is never
/// altered where it is shown in full (an expanded card, a monospaced block).
fn one_line_summary(text: &str) -> String {
    let total = text.lines().count();
    let first = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    if total <= 1 {
        first.to_string()
    } else {
        format!("{first} … {total} líneas")
    }
}

fn mono_family(cx: &App) -> SharedString {
    gpui_kit::base::Theme::global(cx)
        .tokens
        .typography
        .mono
        .clone()
}

/// A monospaced block for commands, outputs and diff context.
fn mono_block(text: &str, theme: &ChatTheme, s: &ChatSettings, cx: &App) -> impl IntoElement {
    div()
        .w_full()
        .px_1p5()
        .py_0p5()
        .rounded(s.px(CARD_RADIUS))
        .bg(theme.bg_editor)
        .border_1()
        .border_color(theme.border)
        .font_family(mono_family(cx))
        .text_size(s.px(TEXT_CODE))
        .text_color(theme.text)
        .whitespace_normal()
        .child(SharedString::from(text.to_string()))
}

/// Spinner / ✓ / ✗ of a tool row.
fn status_glyph(status: ToolCallStatus, theme: &ChatTheme, s: &ChatSettings) -> impl IntoElement {
    let (glyph, color) = match status {
        ToolCallStatus::Completed => (IconName::Check, theme.status_ok),
        ToolCallStatus::Failed => (IconName::X, theme.status_error),
        ToolCallStatus::InProgress => (IconName::LoaderCircle, theme.text_accent),
        _ => (IconName::Circle, theme.text_muted),
    };
    div()
        .text_color(color)
        .text_size(s.px(TEXT_SMALL))
        .child(glyph)
}

/// The number on the right of a tool row: `+N −M` for an edit, a count of
/// lines or results for the rest.
fn tool_stat(call: &ToolCallEntry, theme: &ChatTheme, s: &ChatSettings) -> Option<AnyElement> {
    let theme = *theme;
    if let Some((added, removed)) = call.stats {
        return Some(
            h_flex()
                .gap_1()
                .flex_shrink_0()
                .text_size(s.px(TEXT_LABEL))
                .child(
                    div()
                        .text_color(theme.diff_added)
                        .child(SharedString::from(format!("+{added}"))),
                )
                .child(
                    div()
                        .text_color(theme.diff_deleted)
                        .child(SharedString::from(format!("−{removed}"))),
                )
                .into_any_element(),
        );
    }
    let lines: usize = call
        .content
        .iter()
        .map(|content| match content {
            ToolContent::Text(text) => text.lines().count(),
            _ => 0,
        })
        .sum();
    if lines == 0 {
        return None;
    }
    let label = match call.kind {
        ToolKind::Search => format!("{lines} resultados"),
        ToolKind::Read | ToolKind::Fetch => format!("{lines} líneas"),
        _ => return None,
    };
    Some(
        div()
            .flex_shrink_0()
            .text_size(s.px(TEXT_LABEL))
            .text_color(theme.text_muted)
            .child(SharedString::from(label))
            .into_any_element(),
    )
}

/// The checkbox of a plan item.
fn plan_glyph(status: PlanEntryStatus, theme: &ChatTheme, s: &ChatSettings) -> impl IntoElement {
    let (glyph, color) = match status {
        PlanEntryStatus::Completed => (IconName::CircleCheck, theme.status_ok),
        PlanEntryStatus::InProgress => (IconName::LoaderCircle, theme.text_accent),
        _ => (IconName::Circle, theme.text_muted),
    };
    div()
        .text_color(color)
        .text_size(s.px(TEXT_SMALL))
        .child(glyph)
}

/// A one-line message from Cincel (`02-visual.md` §9).
fn render_notice(notice: &Notice, theme: &ChatTheme, s: &ChatSettings) -> AnyElement {
    let color = match notice.level {
        NoticeLevel::Info => theme.text_muted,
        NoticeLevel::Warning => theme.status_warning,
        NoticeLevel::Error => theme.status_error,
    };
    div()
        .w_full()
        .p_2()
        .rounded(s.px(CARD_RADIUS))
        .bg(alpha(color, 0.12))
        .text_size(s.px(TEXT_SMALL))
        .text_color(color)
        .whitespace_normal()
        .child(SharedString::from(notice.text.clone()))
        .into_any_element()
}

/// One piece of a user bubble.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum UserPiece {
    /// Markdown with no mention in it, painted by the answers' renderer.
    Markdown(String),
    /// A paragraph with mentions: plain text and chips, in order.
    Flow(Vec<FlowPart>),
}

/// A part of a [`UserPiece::Flow`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FlowPart {
    /// Plain text.
    Text(String),
    /// A file chip.
    File(std::path::PathBuf),
}

/// First private-use code point standing for a chip while the message is cut
/// into paragraphs.
const CHIP_MARK: u32 = 0xF0000;

/// Cuts a user message into [`UserPiece`]s: paragraphs (blank-line separated,
/// never inside a fence) that hold a mention become a flow of text and chips;
/// runs of the other paragraphs become one Markdown piece each.
pub(crate) fn user_pieces(blocks: &[MessageBlock]) -> Vec<UserPiece> {
    let mut text = String::new();
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for block in blocks {
        match block {
            MessageBlock::Text(chunk) => text.push_str(chunk),
            MessageBlock::File(path) => {
                let mark = char::from_u32(CHIP_MARK + files.len() as u32).unwrap_or('\u{FFFC}');
                text.push(mark);
                files.push(path.clone());
            }
        }
    }
    let is_mark = |ch: char| (CHIP_MARK..CHIP_MARK + files.len() as u32).contains(&(ch as u32));

    // Paragraphs: split on blank lines outside fenced blocks.
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_fence = false;
    for line in text.split('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }
        if !in_fence && line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            continue;
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }

    let mut pieces: Vec<UserPiece> = Vec::new();
    for paragraph in paragraphs {
        if paragraph.chars().any(is_mark) {
            let mut parts = Vec::new();
            let mut run = String::new();
            for ch in paragraph.chars() {
                if is_mark(ch) {
                    if !run.is_empty() {
                        parts.push(FlowPart::Text(std::mem::take(&mut run)));
                    }
                    let file = files[(ch as u32 - CHIP_MARK) as usize].clone();
                    parts.push(FlowPart::File(file));
                } else {
                    run.push(ch);
                }
            }
            if !run.is_empty() {
                parts.push(FlowPart::Text(run));
            }
            pieces.push(UserPiece::Flow(parts));
        } else {
            match pieces.last_mut() {
                Some(UserPiece::Markdown(markdown)) => {
                    markdown.push_str("\n\n");
                    markdown.push_str(&paragraph);
                }
                _ => pieces.push(UserPiece::Markdown(paragraph)),
            }
        }
    }
    pieces
}

/// The tool kinds whose cards open the editor, for the tests.
#[must_use]
pub fn opens_editor(kind: ToolKind) -> bool {
    matches!(kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move)
}

#[cfg(test)]
mod tests {
    use super::one_line_summary;

    #[test]
    fn one_line_summary_keeps_a_single_line_untouched() {
        assert_eq!(one_line_summary("ls -la"), "ls -la");
        assert_eq!(one_line_summary(""), "");
    }

    #[test]
    fn one_line_summary_collapses_a_multiline_command() {
        let command = "python3 - <<'EOF'\nprint(1)\nprint(2)\nprint(3)\nprint(4)\nEOF";
        assert_eq!(
            one_line_summary(command),
            "python3 - <<'EOF' … 6 líneas",
            "solo la primera línea, sin saltos, mas el total"
        );
    }

    #[test]
    fn one_line_summary_skips_leading_blank_lines() {
        assert_eq!(one_line_summary("\n\nhola\nchau"), "hola … 4 líneas");
    }
}
