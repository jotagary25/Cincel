//! The chat dock panel.
//!
//! The chat itself is `asteroid-chat`, in stage 2; what lives here is the
//! panel shell that holds it, so the dock and `Ctrl+Shift+A` work today. The
//! file tree is [`crate::tree_panel`] and the tab area is [`crate::center`].

use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window, div,
};

use crate::theme::ThemeColors;

/// The chat panel, on the left. Empty until stage 2.
pub struct ChatPanel {
    focus_handle: FocusHandle,
}

impl ChatPanel {
    /// Builds the panel entity.
    pub fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
        })
    }
}

impl EventEmitter<PanelEvent> for ChatPanel {}

impl Focusable for ChatPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ChatPanel {
    fn panel_name(&self) -> &'static str {
        "ChatPanel"
    }

    fn closable(&self, _: &App) -> bool {
        // Closing it would leave no way to bring it back; `Ctrl+Shift+A`
        // collapses the whole dock instead.
        false
    }
}

impl Panel for ChatPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        SharedString::from("Chat")
    }
}

impl Render for ChatPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeColors::global(cx).clone();
        v_flex()
            .id("chat-panel")
            .key_context("Chat")
            .track_focus(&self.focus_handle)
            .size_full()
            .p_3()
            .bg(theme.bg_app)
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("El chat con el agente vive acá."),
            )
    }
}
