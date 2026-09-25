//! Painting the chat (`docs/specs/02-visual.md` §7).
//!
//! Every clickable thing gets a pointer cursor and a hover state, and the real
//! buttons are `gpui-kit` [`Button`]s so they follow the palette the workspace
//! projects onto gpui-kit's theme instead of inventing their own contrast.
//!
//! The transcript has two voices and they look different on purpose: what the
//! user said is a bubble against the right edge, what the agent answered is
//! full width with a 2 px accent rule down its left side. Everything in
//! between — thoughts, tool cards, plans — is a compact row.

use std::path::Path;
use std::sync::Arc;

use asteroid_acp::acp::schema::v1::{
    PermissionOptionId, PlanEntryStatus, SessionConfigId, SessionConfigValueId, SessionModeId,
    ToolCallStatus, ToolKind,
};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Focusable as _, FontWeight, SharedString, Window, div,
    px, relative,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};

use crate::actions;
use crate::markdown::markdown_element;
use crate::model::*;
use crate::panel::{
    COMPOSER_HINT, ChatPanel, Popover, current_label, is_footer_option, select_options,
};
use crate::settings::{
    AGENT_RULE_WIDTH, BUBBLE_MAX_WIDTH, BUBBLE_RADIUS, CARD_RADIUS, COMMAND_CARD_RADIUS,
    INPUT_MIN_HEIGHT, INPUT_RADIUS, MAX_OUTPUT_LINES, MESSAGE_GAP, POPOVER_RADIUS,
    POPOVER_ROW_HEIGHT, ROW_GAP, TOOL_ROW_HEIGHT, TURN_GAP,
};
use crate::theme::{ChatTheme, alpha};

/// Height of the header row.
const HEADER_HEIGHT: f32 = 32.;
/// How far above the bottom edge a composer popover floats, which is the
/// height of the composer itself (selectors + input + hint + paddings).
const COMPOSER_HEIGHT: f32 = 108.;

impl Render for ChatPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *self.theme();
        self.sync_placeholder(window, cx);
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
            .text_size(px(13.))
            .text_color(theme.text)
            .child(self.render_header(&theme, cx))
            .child(self.render_transcript(&theme, cx))
            .child(self.render_composer(&theme, window, cx))
            .children(self.render_overlay(&theme, cx))
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

    fn on_newline(&mut self, _: &actions::Newline, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.input.focus_handle(cx);
        if !handle.is_focused(window) {
            window.focus(&handle, cx);
        }
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

    /// `Esc`. Bound both on the panel and on the composer
    /// (`actions::CONTEXT_COMPOSER`): with nothing open the key goes back to
    /// the textarea, and from there to `chat::cancel_turn`.
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

    /// `Enter` / `Tab`. Confirms the highlighted row **without sending**; with
    /// no popover open the key propagates, which is what lets the textarea and
    /// then `chat::send` have it.
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

    fn render_header(&self, theme: &ChatTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *theme;
        let name = self
            .active_agent()
            .map_or_else(|| "Elegí un agente".to_string(), |agent| agent.name.clone());
        let status = self.status();

        h_flex()
            .id("chat-header")
            .h(px(HEADER_HEIGHT))
            .px_2()
            .gap_2()
            .items_center()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .id("chat-agent-selector")
                    .gap_1()
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(CARD_RADIUS))
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.bg_surface))
                    .child(div().text_color(theme.text_accent).child(IconName::Bot))
                    .child(div().child(SharedString::from(name)))
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .child(IconName::ChevronDown),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.toggle_popover(Popover::Agents, cx);
                    })),
            )
            .child(status_pill(status, &theme))
            .child(div().flex_1())
            .child(
                icon_button(
                    "chat-new-session",
                    IconName::Plus,
                    "Nueva conversación",
                    &theme,
                )
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.new_session(window, cx)),
                ),
            )
            .child(
                icon_button("chat-sessions", IconName::Clock, "Historial", &theme).on_click(
                    cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.toggle_popover(Popover::Conversations, cx);
                    }),
                ),
            )
    }

    // ---------------------------------------------------------- transcript

    fn render_transcript(&mut self, theme: &ChatTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *theme;
        let rows: Vec<AnyElement> = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| self.render_entry(index, entry, &theme, cx))
            .collect();

        let empty = rows.is_empty();
        let notice = self.history_notice().map(|text| {
            div()
                .w_full()
                .px_2()
                .py_1()
                .mb_1()
                .rounded(px(CARD_RADIUS))
                .bg(theme.bg_surface)
                .text_size(px(11.))
                .text_color(theme.text_muted)
                .child(SharedString::from(text.to_string()))
        });
        // No `gap`: each row brings its own top spacing, so two messages are
        // 16 px apart while the tool rows of one turn stay compact.
        v_flex()
            .id("chat-transcript")
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p_2()
            .children(notice)
            .when(empty, |this| this.child(self.render_empty_state(&theme)))
            .children(rows)
    }

    /// `02-visual.md` §9: what the chat says when there is nothing to show.
    fn render_empty_state(&self, theme: &ChatTheme) -> impl IntoElement {
        let missing: Vec<&ChatAgent> = self
            .agents
            .iter()
            .filter(|agent| !agent.installed)
            .collect();
        let no_agents = self.agents.is_empty() || self.agents.iter().all(|agent| !agent.installed);

        v_flex()
            .gap_1()
            .p_3()
            .text_color(theme.text_muted)
            .when(no_agents, |this| {
                this.child(div().text_color(theme.text).child("Sin agentes instalados"))
                    .child(
                        div().child(
                            "`npx` los baja solo; hace falta Node 18 o más nuevo en el PATH.",
                        ),
                    )
                    .children(missing.iter().map(|agent| {
                        div().child(SharedString::from(format!(
                            "{} · {}",
                            agent.name,
                            agent.hint.clone().unwrap_or_default()
                        )))
                    }))
            })
            .when(!no_agents, |this| {
                this.child(div().child("Escribí abajo para empezar."))
                    .child(div().child("@ menciona archivos · / muestra los comandos del agente."))
            })
    }

    fn render_entry(
        &self,
        index: usize,
        entry: &Entry,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let inner = match entry {
            Entry::UserMessage(message) => self.render_user_message(index, message, theme),
            Entry::AgentText(text) => self.render_agent_text(index, text, theme, cx),
            Entry::AgentThought(thought) => self.render_thought(index, thought, theme, cx),
            Entry::ToolCall(call) => self.render_tool_call(index, call, theme, cx),
            Entry::Plan(plan) => self.render_plan(index, plan, theme, cx),
            Entry::Permission(permission) => self.render_permission(index, permission, theme, cx),
            Entry::AuthRequired(auth) => self.render_auth(index, auth, theme, cx),
            Entry::Notice(notice) => render_notice(notice, theme),
            // `02-visual.md` §7 has no "turno terminado · 12 s" line any more:
            // a turn ends with air, not with a word.
            Entry::TurnSeparator(_) => {
                return div().w_full().h(px(TURN_GAP)).into_any_element();
            }
        };
        let top = match entry {
            Entry::UserMessage(_) | Entry::AgentText(_) => MESSAGE_GAP,
            _ => ROW_GAP,
        };
        div().w_full().pt(px(top)).child(inner).into_any_element()
    }

    /// What the user said: a bubble against the right edge, so the two voices
    /// of the transcript never blur into one column of text.
    ///
    /// A mention is painted **where it was typed**, as an inline chip between
    /// the words around it, which is the whole point of the inline `@`
    /// (`docs/etapas/etapa-2.md` § correcciones).
    fn render_user_message(
        &self,
        index: usize,
        message: &UserMessage,
        theme: &ChatTheme,
    ) -> AnyElement {
        let theme = *theme;
        let root = self.project_root().map(Path::to_path_buf);
        // The chip carries its own margins instead of the row carrying a gap,
        // so `@main.rs, ¿qué te parece?` keeps the comma glued to the chip the
        // way the author typed it.
        let pieces: Vec<AnyElement> = message
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(position, block)| match block {
                MessageBlock::Text(text) => {
                    let text = text.trim();
                    (!text.is_empty()).then(|| {
                        div()
                            .child(SharedString::from(text.to_string()))
                            .into_any_element()
                    })
                }
                MessageBlock::File(path) => {
                    let glued = matches!(
                        message.blocks.get(position + 1),
                        Some(MessageBlock::Text(text))
                            if text.trim_start().starts_with(is_closing_punctuation)
                    );
                    Some(
                        file_chip(
                            ("message-chip", index * 64 + position),
                            path,
                            root.as_deref(),
                            &theme,
                        )
                        .when(position > 0, |this| this.ml_1())
                        .when(!glued, |this| this.mr_1())
                        .into_any_element(),
                    )
                }
            })
            .collect();

        h_flex()
            .id(("user-message", index))
            .w_full()
            .justify_end()
            .child(
                v_flex()
                    .items_end()
                    .gap_0p5()
                    .max_w(relative(BUBBLE_MAX_WIDTH))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.text_muted)
                            .child("Vos"),
                    )
                    .child(
                        h_flex()
                            .p_2()
                            .flex_wrap()
                            .items_center()
                            .rounded(px(BUBBLE_RADIUS))
                            .bg(theme.bg_surface)
                            .border_1()
                            .border_color(theme.border)
                            .children(pieces),
                    ),
            )
            .into_any_element()
    }

    /// What the agent answered: full width, no bubble, a 2 px accent rule down
    /// the left and the agent's name above it.
    fn render_agent_text(
        &self,
        index: usize,
        text: &AgentText,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *theme;
        let name = self
            .active_agent()
            .map_or_else(|| "Agente".to_string(), |agent| agent.name.clone());
        let body: AnyElement = match text.view.clone() {
            Some(view) => {
                let panel = cx.entity().downgrade();
                let on_copy: crate::markdown::CopyHandler =
                    Arc::new(move |code: String, cx: &mut App| {
                        panel.update(cx, |panel, cx| panel.copy(code, cx)).ok();
                    });
                markdown_element(&view, &theme, self.highlighter(), on_copy).into_any_element()
            }
            None => div()
                .child(SharedString::from(text.markdown.clone()))
                .into_any_element(),
        };

        v_flex()
            .id(("agent-text", index))
            .w_full()
            .gap_0p5()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(name)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_stretch()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .w(px(AGENT_RULE_WIDTH))
                            .flex_shrink_0()
                            .rounded(px(1.))
                            .bg(theme.text_accent),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(body)),
            )
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
                .rounded(px(CARD_RADIUS))
                .bg(theme.bg_surface)
                .text_size(px(12.))
                .text_color(theme.text_muted)
                .child(SharedString::from(thought.text.clone()))
        });

        v_flex()
            .id(("thought", index))
            .w_full()
            .gap_0p5()
            .child(
                h_flex()
                    .id(("thought-toggle", index))
                    .h(px(TOOL_ROW_HEIGHT))
                    .px_1()
                    .gap_1()
                    .items_center()
                    .rounded(px(CARD_RADIUS))
                    .cursor_pointer()
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
        let detail = call
            .expanded
            .then(|| self.render_tool_detail(index, call, &theme, cx));

        v_flex()
            .id(("tool", index))
            .w_full()
            .child(
                h_flex()
                    .id(("tool-row", index))
                    .h(px(TOOL_ROW_HEIGHT))
                    .px_1()
                    .gap_1p5()
                    .items_center()
                    .rounded(px(CARD_RADIUS))
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
                            .child(SharedString::from(call.title.clone())),
                    )
                    .children(tool_stat(call, &theme))
                    .child(status_glyph(call.status, &theme))
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

        v_flex()
            .id(("command", index))
            .w_full()
            .rounded(px(COMMAND_CARD_RADIUS))
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_surface)
            .child(
                h_flex()
                    .id(("command-row", index))
                    .h(px(TOOL_ROW_HEIGHT))
                    .px_1p5()
                    .gap_1p5()
                    .items_center()
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
                            .font_family("monospace")
                            .text_size(px(11.))
                            .child(SharedString::from(format!("$ {command}"))),
                    )
                    .children(exit_code.map(|label| {
                        div()
                            .flex_shrink_0()
                            .font_family("monospace")
                            .text_size(px(11.))
                            .text_color(theme.status_error)
                            .child(label)
                    }))
                    .child(status_glyph(call.status, &theme))
                    .child(div().text_color(theme.text_muted).child(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.toggle_entry(index, cx);
                    })),
            )
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
                .text_size(px(11.))
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
                    .bg(theme.bg_app)
                    .font_family("monospace")
                    .text_size(px(11.))
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
        let root = self.project_root().map(Path::to_path_buf);
        let rows: Vec<AnyElement> = call
            .content
            .iter()
            .enumerate()
            .map(|(position, content)| match content {
                ToolContent::Text(text) => mono_block(text, &theme).into_any_element(),
                ToolContent::Command { command, output } => v_flex()
                    .gap_1()
                    .child(mono_block(&format!("$ {command}"), &theme))
                    .when(!output.is_empty(), |this| {
                        this.child(mono_block(output, &theme))
                    })
                    .into_any_element(),
                ToolContent::Edit { path, preview } => {
                    let open_path = path.clone();
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_color(theme.text_muted)
                                .text_size(px(11.))
                                .child(relative_label(root.as_deref(), path)),
                        )
                        .children(
                            preview
                                .iter()
                                .map(|line| mono_block(line, &theme).into_any_element()),
                        )
                        .child(
                            div()
                                .text_color(theme.text_muted)
                                .text_size(px(11.))
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
            .rounded(px(CARD_RADIUS))
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
            .rounded(px(CARD_RADIUS))
            .bg(theme.bg_surface)
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
                        .child(plan_glyph(item.status.clone(), &theme))
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
                .text_size(px(11.))
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
            .rounded(px(CARD_RADIUS))
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
                                .text_size(px(11.))
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
                        .when(expanded, |this| this.child(mono_block(&detail, &theme))),
                )
            })
            .child(h_flex().gap_1().flex_wrap().children(buttons))
            .child(
                div()
                    .text_size(px(11.))
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
            .rounded(px(CARD_RADIUS))
            .bg(theme.bg_surface)
            .border_1()
            .border_color(theme.status_warning)
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Hace falta autenticarse"),
            )
            .when(!command.is_empty(), |this| {
                this.child(mono_block(&command, &theme))
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
    fn render_composer(
        &self,
        theme: &ChatTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *theme;
        let blocked = self.is_awaiting_permission();
        let thinking = self.status() == AgentStatus::Thinking;
        let focused = self.input.focus_handle(cx).is_focused(window);

        // No strip of chips above the input any more: a mention is the
        // `@archivo` token the user can see and edit inside the text
        // (`docs/etapas/etapa-2.md` § correcciones).
        v_flex()
            .id("chat-composer")
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
                    .w_full()
                    .min_h(px(INPUT_MIN_HEIGHT))
                    .px_1()
                    .py_0p5()
                    .gap_1()
                    .items_end()
                    .rounded(px(INPUT_RADIUS))
                    .bg(theme.bg_surface)
                    .border_1()
                    .border_color(if focused {
                        theme.border_focus
                    } else {
                        theme.border
                    })
                    .child(
                        div().flex_1().min_w(px(0.)).child(
                            Textarea::new(&self.input)
                                .appearance(false)
                                .bordered(false)
                                .disabled(blocked),
                        ),
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
                    .text_size(px(11.))
                    .text_color(theme.text_muted)
                    .child(COMPOSER_HINT),
            )
    }

    /// The row of compact selectors above the input (`02-visual.md` §7).
    fn render_selectors(&self, theme: &ChatTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *theme;
        let autonomy = self.autonomy();
        let mut row = h_flex()
            .id("chat-selectors")
            .gap_1()
            .flex_wrap()
            .child(selector(
                "autonomy",
                autonomy_label(autonomy),
                Some(IconName::Shield),
                Some(autonomy_tooltip(autonomy)),
                &theme,
                cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.toggle_popover(Popover::AutonomySelect, cx);
                }),
            ));

        for (index, option) in self.config_options.iter().enumerate() {
            if !is_footer_option(option) {
                continue;
            }
            row = row.child(selector(
                ("config", index),
                current_label(option),
                None,
                None,
                &theme,
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
                None,
                None,
                &theme,
                cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.toggle_popover(Popover::Modes, cx);
                }),
            ));
        }
        row
    }

    // ------------------------------------------------------------- overlay

    /// The history popover: the directory's conversations grouped by agent
    /// (the agent's name is the group header), newest first inside each group,
    /// each row showing its title, its relative time and a ✕
    /// (`docs/etapas/etapa-2.md` § correcciones).
    fn render_history_rows(&self, theme: &ChatTheme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = *theme;
        // The workspace hands them over newest first; grouping keeps that
        // order both between groups (by their newest conversation) and inside.
        let mut groups: Vec<&str> = Vec::new();
        for conversation in &self.conversations {
            if !groups.contains(&conversation.agent_name.as_str()) {
                groups.push(&conversation.agent_name);
            }
        }

        let mut rows: Vec<AnyElement> = Vec::new();
        for agent in groups {
            rows.push(
                div()
                    .px_1p5()
                    .pt_1()
                    .pb_0p5()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(SharedString::from(agent.to_string()))
                    .into_any_element(),
            );
            for (index, conversation) in self
                .conversations
                .iter()
                .enumerate()
                .filter(|(_, conversation)| conversation.agent_name == agent)
            {
                let open_id = conversation.id.clone();
                let delete_id = conversation.id.clone();
                let selected = self.active_conversation() == Some(conversation.id.as_str());
                rows.push(
                    h_flex()
                        .id(("conversation", index))
                        .h(px(POPOVER_ROW_HEIGHT))
                        .px_1p5()
                        .gap_1p5()
                        .items_center()
                        .rounded(px(CARD_RADIUS))
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
                                .text_size(px(11.))
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
        let root = self.project_root().map(Path::to_path_buf);
        let rows: Vec<AnyElement> = match self.popover().clone() {
            Popover::Closed => return None,
            Popover::Agents => self
                .agents
                .iter()
                .enumerate()
                .map(|(index, agent)| {
                    let id = agent.id.clone();
                    let installed = agent.installed;
                    let label = if installed {
                        agent.name.clone()
                    } else {
                        format!(
                            "{} · {}",
                            agent.name,
                            agent.hint.clone().unwrap_or_default()
                        )
                    };
                    popover_row(("agent", index), label, false, !installed, &theme)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            cx.stop_propagation();
                            this.set_active_agent(&id, cx);
                        }))
                        .into_any_element()
                })
                .collect(),
            Popover::Conversations => self.render_history_rows(&theme, cx),
            Popover::Files { selected, .. } => self
                .filtered_files()
                .into_iter()
                .enumerate()
                .map(|(index, path)| {
                    let owned = path.clone();
                    file_popover_row(index, &path, root.as_deref(), index == selected, &theme)
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
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.insert_command(&name, window, cx);
                    }))
                    .into_any_element()
                })
                .collect(),
            Popover::AutonomySelect => AUTONOMY_VALUES
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let value = *value;
                    let tooltip = autonomy_tooltip(value);
                    popover_row(
                        ("autonomy", index),
                        autonomy_label(value).to_string(),
                        value == self.autonomy(),
                        false,
                        &theme,
                    )
                    .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        cx.stop_propagation();
                        this.set_autonomy(value, cx);
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
                        popover_row(("mode", index), mode.name.clone(), selected, false, &theme)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                cx.stop_propagation();
                                this.choose_mode(id.clone(), cx);
                            }))
                            .into_any_element()
                    })
                    .collect()
            }
        };

        let anchored_to_header = matches!(self.popover(), Popover::Agents | Popover::Conversations);
        let panel = v_flex()
            .id("chat-popover")
            .absolute()
            .left_2()
            .right_2()
            .map(|this| {
                if anchored_to_header {
                    this.top(px(HEADER_HEIGHT + 4.))
                } else {
                    this.bottom(px(COMPOSER_HEIGHT))
                }
            })
            .max_h(px(POPOVER_ROW_HEIGHT * 10. + 8.))
            .overflow_y_scroll()
            .p_1()
            .rounded(px(POPOVER_RADIUS))
            .bg(theme.bg_elevated)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .when(rows.is_empty(), |this| {
                this.child(
                    div()
                        .px_1()
                        .text_color(theme.text_muted)
                        .child("Sin resultados"),
                )
            })
            .children(rows);
        Some(panel.into_any_element())
    }
}

// ------------------------------------------------------------------ helpers

/// The status pill of the header (`02-visual.md` §7).
fn status_pill(status: AgentStatus, theme: &ChatTheme) -> impl IntoElement {
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
        .rounded(px(CARD_RADIUS))
        .bg(alpha(color, 0.15))
        .text_size(px(11.))
        .text_color(color)
        .child(status.label())
}

/// A header icon button.
fn icon_button(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    theme: &ChatTheme,
) -> gpui::Stateful<gpui::Div> {
    let theme = *theme;
    div()
        .id(id)
        .aria_label(label)
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(CARD_RADIUS))
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(move |style| style.bg(theme.bg_surface).text_color(theme.text))
        .child(icon)
}

/// A compact selector chip of the composer (`Aplicar y revisar después ▾`).
///
/// A bordered chip instead of bare text: the row above the input has to read
/// as controls, not as a caption of the message. `icon` marks the autonomy one
/// with a shield and `tooltip` says in one sentence what the value does.
fn selector(
    id: impl Into<gpui::ElementId>,
    label: impl Into<String>,
    icon: Option<IconName>,
    tooltip: Option<&'static str>,
    theme: &ChatTheme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let theme = *theme;
    h_flex()
        .id(id.into())
        .gap_0p5()
        .px_1p5()
        .py_0p5()
        .rounded(px(CARD_RADIUS))
        .bg(theme.bg_surface)
        .border_1()
        .border_color(theme.border)
        .text_size(px(11.))
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(move |style| style.bg(theme.bg_elevated).text_color(theme.text))
        .when_some(icon, |this, icon| this.child(icon))
        .child(SharedString::from(label.into()))
        .child(IconName::ChevronDown)
        .when_some(tooltip, |this, text| {
            this.tooltip(move |window, cx| Tooltip::new(text).build(window, cx))
        })
        .on_click(on_click)
}

/// One row of an overlay list.
fn popover_row(
    id: impl Into<gpui::ElementId>,
    label: impl Into<String>,
    selected: bool,
    dimmed: bool,
    theme: &ChatTheme,
) -> gpui::Stateful<gpui::Div> {
    let theme = *theme;
    div()
        .id(id.into())
        .h(px(POPOVER_ROW_HEIGHT))
        .px_1p5()
        .flex()
        .items_center()
        .rounded(px(CARD_RADIUS))
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
) -> gpui::Stateful<gpui::Div> {
    let theme = *theme;
    let parent = parent_label(root, path);
    div()
        .id(("file", index))
        .h(px(POPOVER_ROW_HEIGHT))
        .px_1p5()
        .flex()
        .flex_row()
        .items_center()
        .gap_1p5()
        .rounded(px(CARD_RADIUS))
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
                    .text_size(px(11.))
                    .text_color(theme.text_muted)
                    .child(parent),
            )
        })
}

/// The `@archivo` chip of a user message, with its relative path as tooltip.
fn file_chip(
    id: impl Into<gpui::ElementId>,
    path: &Path,
    root: Option<&Path>,
    theme: &ChatTheme,
) -> gpui::Stateful<gpui::Div> {
    let tooltip = relative_label(root, path);
    div()
        .id(id.into())
        .px_1p5()
        .py_0p5()
        .rounded(px(CARD_RADIUS))
        .bg(theme.bg_elevated)
        .text_size(px(11.))
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

/// A monospaced block for commands, outputs and diff context.
fn mono_block(text: &str, theme: &ChatTheme) -> impl IntoElement {
    div()
        .w_full()
        .px_1p5()
        .py_0p5()
        .rounded(px(CARD_RADIUS))
        .bg(theme.bg_app)
        .font_family("monospace")
        .text_size(px(11.))
        .text_color(theme.text)
        .child(SharedString::from(text.to_string()))
}

/// Spinner / ✓ / ✗ of a tool row.
fn status_glyph(status: ToolCallStatus, theme: &ChatTheme) -> impl IntoElement {
    let (glyph, color) = match status {
        ToolCallStatus::Completed => (IconName::Check, theme.status_ok),
        ToolCallStatus::Failed => (IconName::X, theme.status_error),
        ToolCallStatus::InProgress => (IconName::LoaderCircle, theme.text_accent),
        _ => (IconName::Circle, theme.text_muted),
    };
    div().text_color(color).text_size(px(12.)).child(glyph)
}

/// The number on the right of a tool row: `+N −M` for an edit, a count of
/// lines or results for the rest.
fn tool_stat(call: &ToolCallEntry, theme: &ChatTheme) -> Option<AnyElement> {
    let theme = *theme;
    if let Some((added, removed)) = call.stats {
        return Some(
            h_flex()
                .gap_1()
                .flex_shrink_0()
                .text_size(px(11.))
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
            .text_size(px(11.))
            .text_color(theme.text_muted)
            .child(SharedString::from(label))
            .into_any_element(),
    )
}

/// The checkbox of a plan item.
fn plan_glyph(status: PlanEntryStatus, theme: &ChatTheme) -> impl IntoElement {
    let (glyph, color) = match status {
        PlanEntryStatus::Completed => (IconName::CircleCheck, theme.status_ok),
        PlanEntryStatus::InProgress => (IconName::LoaderCircle, theme.text_accent),
        _ => (IconName::Circle, theme.text_muted),
    };
    div().text_color(color).text_size(px(12.)).child(glyph)
}

/// A one-line message from Asteroid (`02-visual.md` §9).
fn render_notice(notice: &Notice, theme: &ChatTheme) -> AnyElement {
    let color = match notice.level {
        NoticeLevel::Info => theme.text_muted,
        NoticeLevel::Warning => theme.status_warning,
        NoticeLevel::Error => theme.status_error,
    };
    div()
        .w_full()
        .p_2()
        .rounded(px(CARD_RADIUS))
        .bg(alpha(color, 0.12))
        .text_color(color)
        .child(SharedString::from(notice.text.clone()))
        .into_any_element()
}

/// The tool kinds whose cards open the editor, for the tests.
#[must_use]
pub fn opens_editor(kind: ToolKind) -> bool {
    matches!(kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move)
}
