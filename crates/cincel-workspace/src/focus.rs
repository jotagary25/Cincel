//! Focus zones: the panels that take the keyboard with `Ctrl+Shift+A`,
//! `Ctrl+Shift+E` and the focus wheel (`Ctrl+L`).
//!
//! See `docs/specs/07-etapa5-productividad.md` §7 and §9. The window has three
//! zones, in the order the wheel visits them: the chat on the left, the
//! center (tabs, editors, Markdown preview) and the files on the right.
//!
//! - [`Workspace::toggle_focus`] is the rule of §7.1 for the two side panels:
//!   hidden → open and focus it; visible with the focus inside → close it and
//!   give the keyboard back to the center; visible with the focus elsewhere →
//!   focus it.
//! - [`Workspace::focus_next_zone`] is the wheel of §9.1, which never opens or
//!   closes anything; the pure part of it is [`next_zone`].
//!
//! Both do nothing while a modal is up ([`Workspace::is_modal_open`]).

use gpui::{App, Context, Focusable as _, Window};
use gpui_kit::component::dock::DockPlacement;

use crate::workspace::Workspace;

/// One of the three places the keyboard can live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FocusZone {
    /// The chat panel (left dock).
    Chat,
    /// The tab area: editors, Markdown previews and the settings tab.
    Center,
    /// The file tree panel (right dock).
    Files,
}

impl FocusZone {
    /// The order of the wheel (§9.1): Chat → Center → Files → Chat.
    pub const WHEEL: [FocusZone; 3] = [FocusZone::Chat, FocusZone::Center, FocusZone::Files];

    /// The dock placement of a side panel; `None` for the center.
    pub fn placement(self) -> Option<DockPlacement> {
        match self {
            FocusZone::Chat => Some(DockPlacement::Left),
            FocusZone::Center => None,
            FocusZone::Files => Some(DockPlacement::Right),
        }
    }
}

/// Which side panels are on screen. The center is always visible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisibleZones {
    /// The chat dock is open.
    pub chat: bool,
    /// The files dock is open.
    pub files: bool,
}

impl VisibleZones {
    /// Whether `zone` is on screen.
    pub fn contains(self, zone: FocusZone) -> bool {
        match zone {
            FocusZone::Chat => self.chat,
            FocusZone::Center => true,
            FocusZone::Files => self.files,
        }
    }
}

/// Where `Ctrl+L` sends the keyboard (§9.1), as a pure function.
///
/// - From outside every zone (`None`): the chat if it is visible, otherwise
///   the center.
/// - From a zone: the next visible one in [`FocusZone::WHEEL`] order, wrapping
///   around. When no other zone is visible the answer is the current one
///   (the caller then leaves the focus alone). A current zone that is not
///   visible (a stale frame) still advances from its place in the wheel.
pub fn next_zone(current: Option<FocusZone>, visible: VisibleZones) -> FocusZone {
    let Some(current) = current else {
        return if visible.chat {
            FocusZone::Chat
        } else {
            FocusZone::Center
        };
    };
    let start = FocusZone::WHEEL
        .iter()
        .position(|zone| *zone == current)
        .unwrap_or(0);
    (1..=FocusZone::WHEEL.len())
        .map(|step| FocusZone::WHEEL[(start + step) % FocusZone::WHEEL.len()])
        .find(|zone| visible.contains(*zone))
        // Unreachable: the center is always visible.
        .unwrap_or(FocusZone::Center)
}

impl Workspace {
    /// The zone that holds the keyboard, or `None` when the focus is on the
    /// title bar, the status bar, the workspace root or a floating modal.
    ///
    /// Answers from the last rendered frame, like every GPUI focus query.
    pub fn focus_zone(&self, window: &Window, cx: &App) -> Option<FocusZone> {
        if self.chat().read(cx).contains_focus(window, cx) {
            Some(FocusZone::Chat)
        } else if self.files().read(cx).contains_focus(window, cx) {
            Some(FocusZone::Files)
        } else if self
            .center()
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
        {
            Some(FocusZone::Center)
        } else {
            None
        }
    }

    /// Whether `zone` is on screen. The center always is; the side panels
    /// only with a project open and their dock expanded.
    pub fn is_zone_visible(&self, zone: FocusZone, cx: &App) -> bool {
        match zone.placement() {
            None => true,
            Some(placement) => self.project().is_some() && self.is_dock_open(placement, cx),
        }
    }

    /// Both side panels' visibility, for [`next_zone`].
    pub fn visible_zones(&self, cx: &App) -> VisibleZones {
        VisibleZones {
            chat: self.is_zone_visible(FocusZone::Chat, cx),
            files: self.is_zone_visible(FocusZone::Files, cx),
        }
    }

    /// Whether something modal is on screen, which freezes the focus
    /// commands (§7.4): the connections modal, the "¿Guardar cambios?"
    /// dialog of a tab, the review panel, the file finder, the shortcuts
    /// modal, the "Nuevo archivo" field, the quit dialog, the title bar
    /// menu (E5-I) and the full-size image viewer (E7-G).
    /// The settings tab is deliberately *not* here: it is a tab, part of the
    /// center zone, and the panel shortcuts and `Ctrl+L` treat it like an
    /// editor.
    pub fn is_modal_open(&self, cx: &App) -> bool {
        self.agents().read(cx).modal().read(cx).is_open()
            || self.center().read(cx).is_asking_about_unsaved_changes()
            || self.review().read(cx).is_panel_open()
            || self
                .file_finder()
                .is_some_and(|finder| finder.read(cx).is_open())
            || self
                .new_file_prompt()
                .is_some_and(|prompt| prompt.read(cx).is_open())
            || self.shortcuts_modal().read(cx).is_open()
            || self.is_image_viewer_open(cx)
            || self.is_quit_dialog_open()
            || self.is_review_close_dialog_open()
            || self.review().read(cx).reject_turn_prompt().is_some()
            || self.is_title_menu_open()
    }

    /// Gives the keyboard to `zone` (§7.1 "Enfocar", §7.3 for the center):
    /// the chat's composer; the selected (or first) row of the tree; the
    /// active tab's editor, or its Markdown preview when the tab shows it,
    /// the tab area without tabs, and the workspace itself without a project.
    pub fn focus(&mut self, zone: FocusZone, window: &mut Window, cx: &mut Context<Self>) {
        match zone {
            FocusZone::Chat => self
                .chat()
                .clone()
                .update(cx, |chat, cx| chat.focus_input(window, cx)),
            FocusZone::Files => self
                .files()
                .clone()
                .update(cx, |files, cx| files.focus_selected_or_first(window, cx)),
            FocusZone::Center => self.focus_center(window, cx),
        }
    }

    /// §7.3: where the keyboard goes when a side panel closes.
    fn focus_center(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let center = self.center().clone();
        if self.project().is_none() && !center.read(cx).has_items() {
            // The tab area is not even on screen: keep the global commands
            // alive by focusing the root view.
            let handle = self.focus_handle(cx);
            window.focus(&handle, cx);
            return;
        }
        // The settings tab, the Markdown preview of a tab showing it, or
        // the editor (`CenterPanel::focus_active`).
        center.update(cx, |center, cx| center.focus_active(window, cx));
    }

    /// The rule of §7.1 for `workspace::toggle_chat` / `toggle_tree` (and the
    /// title bar buttons of §8.3).
    pub fn toggle_focus(&mut self, zone: FocusZone, window: &mut Window, cx: &mut Context<Self>) {
        let Some(placement) = zone.placement() else {
            return;
        };
        if self.is_modal_open(cx) {
            tracing::debug!(
                ?zone,
                "hay un modal abierto: el atajo del panel no hace nada"
            );
            return;
        }
        if self.project().is_none() {
            // No dock on screen: remember the choice for when a project
            // opens, and leave the focus where it is.
            self.toggle_dock(placement, window, cx);
            return;
        }
        if !self.is_dock_open(placement, cx) {
            self.toggle_dock(placement, window, cx);
            self.focus(zone, window, cx);
        } else if self.focus_zone(window, cx) == Some(zone) {
            if zone == FocusZone::Chat {
                // A popover must not wait, open, for the panel to come back.
                self.chat()
                    .clone()
                    .update(cx, |chat, cx| chat.close_popover(cx));
            }
            self.toggle_dock(placement, window, cx);
            self.focus(FocusZone::Center, window, cx);
        } else {
            self.focus(zone, window, cx);
        }
        cx.notify();
    }

    /// `workspace::focus_next_zone` (`Ctrl+L`, §9.1).
    pub fn focus_next_zone(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_modal_open(cx) {
            tracing::debug!("hay un modal abierto: Ctrl+L no mueve el foco");
            return;
        }
        let current = self.focus_zone(window, cx);
        let target = next_zone(current, self.visible_zones(cx));
        if Some(target) == current {
            return;
        }
        tracing::debug!(?current, ?target, "rueda de foco");
        self.focus(target, window, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FocusZone::{Center, Chat, Files};

    const NONE: VisibleZones = VisibleZones {
        chat: false,
        files: false,
    };
    const CHAT: VisibleZones = VisibleZones {
        chat: true,
        files: false,
    };
    const FILES: VisibleZones = VisibleZones {
        chat: false,
        files: true,
    };
    const BOTH: VisibleZones = VisibleZones {
        chat: true,
        files: true,
    };

    /// Every current zone (and `None`) × every visibility (§9.4).
    #[test]
    fn next_zone_covers_the_whole_table() {
        let table: &[(Option<FocusZone>, VisibleZones, FocusZone)] = &[
            // Outside every zone: the chat if visible, else the center.
            (None, BOTH, Chat),
            (None, CHAT, Chat),
            (None, FILES, Center),
            (None, NONE, Center),
            // All three visible: the full wheel.
            (Some(Chat), BOTH, Center),
            (Some(Center), BOTH, Files),
            (Some(Files), BOTH, Chat),
            // Chat hidden: Center ↔ Files.
            (Some(Center), FILES, Files),
            (Some(Files), FILES, Center),
            (Some(Chat), FILES, Center),
            // Files hidden: Chat ↔ Center.
            (Some(Chat), CHAT, Center),
            (Some(Center), CHAT, Chat),
            (Some(Files), CHAT, Chat),
            // Both hidden: the center stays.
            (Some(Center), NONE, Center),
            (Some(Chat), NONE, Center),
            (Some(Files), NONE, Center),
        ];
        for (current, visible, expected) in table {
            assert_eq!(
                next_zone(*current, *visible),
                *expected,
                "desde {current:?} con {visible:?}"
            );
        }
    }

    #[test]
    fn the_wheel_never_lands_on_a_hidden_zone() {
        for visible in [NONE, CHAT, FILES, BOTH] {
            for current in [None, Some(Chat), Some(Center), Some(Files)] {
                assert!(visible.contains(next_zone(current, visible)));
            }
        }
    }

    #[test]
    fn only_the_side_panels_have_a_dock() {
        assert_eq!(Chat.placement(), Some(DockPlacement::Left));
        assert_eq!(Files.placement(), Some(DockPlacement::Right));
        assert_eq!(Center.placement(), None);
    }
}
