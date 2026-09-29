//! The chat dock panel.
//!
//! `cincel-chat` owns the whole chat experience (transcript, composer,
//! popovers, permission cards); this module only adapts it to `gpui-kit`'s
//! dock, which needs its own [`gpui_kit::component::dock::Panel`] /
//! [`gpui_kit::component::dock::BasePanel`] impls. Rust's orphan rule forbids
//! implementing a foreign trait (`Panel`) for a foreign type
//! (`cincel_chat::ChatPanel`), so [`ChatDock`] wraps it instead of
//! reimplementing it: it renders the whole wrapped entity as its single
//! child and forwards the focus handle to it, so `Ctrl+Shift+A` and the focus
//! wheel (`crate::focus`) can still move the keyboard straight to the
//! composer via [`cincel_chat::ChatPanel::focus_input`].

use cincel_chat::ChatPanel;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window};

/// Adapts [`cincel_chat::ChatPanel`] to the dock on the left.
pub struct ChatDock {
    chat: Entity<ChatPanel>,
}

impl ChatDock {
    /// Wraps an already-built chat panel entity.
    pub fn new(chat: Entity<ChatPanel>, cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self { chat })
    }

    /// The wrapped chat panel.
    pub fn chat(&self) -> &Entity<ChatPanel> {
        &self.chat
    }
}

impl EventEmitter<PanelEvent> for ChatDock {}

impl Focusable for ChatDock {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.chat.read(cx).focus_handle(cx)
    }
}

impl BasePanel for ChatDock {
    fn panel_name(&self) -> &'static str {
        "ChatPanel"
    }

    fn closable(&self, _: &App) -> bool {
        // Closing it would leave no way to bring it back; `Ctrl+Shift+A`
        // collapses the whole dock instead.
        false
    }
}

impl Panel for ChatDock {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        SharedString::from("Chat")
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for ChatDock {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.chat.clone()
    }
}
