//! The three dock panels of the workspace.
//!
//! Stage E0-a only needs their shells: the chat panel, the center area with
//! its (still empty) tab bar, and the file tree. Each is a `gpui-kit` dock
//! panel, which is what makes them draggable, resizable and collapsible for
//! free.

use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window, div, px,
};

use crate::theme::Theme;

/// Height of a tab, from `docs/specs/02-visual.md` §4. Used as the minimum
/// height of the center area so the strip above it never collapses.
const TAB_HEIGHT: f32 = 32.;

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
        placeholder(
            "chat-panel",
            "El chat con el agente vive acá.",
            &self.focus_handle,
            cx,
        )
    }
}

/// The file tree panel, on the right. Empty until stage 1.
pub struct FilesPanel {
    focus_handle: FocusHandle,
}

impl FilesPanel {
    /// Builds the panel entity.
    pub fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
        })
    }
}

impl EventEmitter<PanelEvent> for FilesPanel {}

impl Focusable for FilesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for FilesPanel {
    fn panel_name(&self) -> &'static str {
        "FilesPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for FilesPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        SharedString::from("Archivos")
    }
}

impl Render for FilesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        placeholder(
            "files-panel",
            "Abrí una carpeta para ver el árbol.",
            &self.focus_handle,
            cx,
        )
    }
}

/// The center area: the tab strip plus whatever tab is open. No editor yet,
/// so the strip is empty and the body is the "nothing open" hint of
/// `docs/specs/02-visual.md` §9.
pub struct CenterPanel {
    focus_handle: FocusHandle,
}

impl CenterPanel {
    /// Builds the panel entity.
    pub fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
        })
    }
}

impl EventEmitter<PanelEvent> for CenterPanel {}

impl Focusable for CenterPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for CenterPanel {
    fn panel_name(&self) -> &'static str {
        "CenterPanel"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for CenterPanel {
    /// Deliberately empty: with a single panel in the group, gpui-kit draws a
    /// plain title strip, and an empty title turns that strip into the empty
    /// tab bar of `02-visual.md` §1. Real tabs replace it in stage 1.
    ///
    /// gpui-kit 0.6.1 has no way to suppress a group's own title strip (the
    /// `Panel::title_bar` hook only exists in its unreleased sources), so the
    /// panel cannot draw its own tab bar without stacking two strips.
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for CenterPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::global(cx);

        v_flex()
            .id("center-panel")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_h(px(TAB_HEIGHT))
            .bg(theme.bg_editor)
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(cx.theme().muted_foreground)
                    .child("Abrí un archivo o hablale al agente"),
            )
    }
}

/// The body every empty panel shares: muted text at the top of the panel.
fn placeholder(
    id: &'static str,
    text: &'static str,
    focus_handle: &FocusHandle,
    cx: &App,
) -> impl IntoElement {
    div()
        .id(id)
        .track_focus(focus_handle)
        .size_full()
        .p_3()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}
