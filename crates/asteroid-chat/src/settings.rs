//! Tunables of the chat panel (`docs/specs/02-visual.md` §7 and
//! `docs/specs/modulos/chat.md`).
//!
//! Like [`crate::ChatTheme`], a plain `Default` struct the workspace fills in
//! from `asteroid-settings` without this crate depending on it.

/// Sizes and behaviours the workspace may override.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatSettings {
    /// Rows the input starts at (`02-visual.md` §7).
    pub input_min_rows: usize,
    /// Rows the input grows to before scrolling (§7: 8 lines).
    pub input_max_rows: usize,
    /// Lines of diff context an edit tool card shows when expanded.
    ///
    /// `chat.md`: **never** a full diff; at most the first three lines.
    pub diff_preview_lines: usize,
    /// Whether the agent's thinking starts collapsed (§7: yes).
    pub collapse_thoughts: bool,
    /// Entries kept in the transcript before the oldest ones are dropped.
    pub max_entries: usize,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            input_min_rows: 1,
            input_max_rows: 8,
            diff_preview_lines: 3,
            collapse_thoughts: true,
            max_entries: 2000,
        }
    }
}

/// Height of a tool-call row, in pixels (`02-visual.md` §7).
pub const TOOL_ROW_HEIGHT: f32 = 28.;
/// Corner radius of a user message (`02-visual.md` §7: 6 px).
pub const MESSAGE_RADIUS: f32 = 6.;
/// Corner radius of buttons and cards (`02-visual.md` §4: 4 px).
pub const CARD_RADIUS: f32 = 4.;
/// Corner radius of popovers (`02-visual.md` §4: 6 px).
pub const POPOVER_RADIUS: f32 = 6.;
/// Corner radius of the `execute` card, which reads as a terminal panel
/// rather than as a chip (`docs/etapas/etapa-2.md` § correcciones: 6 px,
/// 1 px border in `border`).
pub const COMMAND_CARD_RADIUS: f32 = 6.;
/// Height of a row in the `@` / `/` popovers.
pub const POPOVER_ROW_HEIGHT: f32 = 24.;
/// Rows a popover shows before it scrolls.
pub const POPOVER_MAX_ROWS: usize = 10;
/// Corner radius of a user message bubble (`02-visual.md` §4, cards).
pub const BUBBLE_RADIUS: f32 = 8.;
/// Widest a user message bubble gets, as a fraction of the panel.
pub const BUBBLE_MAX_WIDTH: f32 = 0.85;
/// Corner radius of the input box.
pub const INPUT_RADIUS: f32 = 6.;
/// Minimum height of the input box, in pixels.
pub const INPUT_MIN_HEIGHT: f32 = 40.;
/// Width of the rule that marks an agent answer, in pixels.
pub const AGENT_RULE_WIDTH: f32 = 2.;
/// Vertical gap between two messages, in pixels (`02-visual.md` §7).
pub const MESSAGE_GAP: f32 = 16.;
/// Vertical gap between the compact rows of a turn (tool cards, thoughts).
pub const ROW_GAP: f32 = 4.;
/// Vertical space that replaces the old "turno terminado · X s" separator.
pub const TURN_GAP: f32 = 12.;
/// Lines of command output a tool card shows before "ver más".
pub const MAX_OUTPUT_LINES: usize = 20;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_follow_the_spec() {
        let settings = ChatSettings::default();
        assert_eq!(settings.input_max_rows, 8);
        assert_eq!(settings.diff_preview_lines, 3);
        assert!(settings.collapse_thoughts);
        assert_eq!(TOOL_ROW_HEIGHT, 28.);
    }
}
