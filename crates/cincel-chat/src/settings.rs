//! Tunables of the chat panel (`docs/specs/02-visual.md` §7 and
//! `docs/specs/modulos/chat.md`).
//!
//! Like [`crate::ChatTheme`], a plain `Default` struct the workspace fills in
//! from `cincel-settings` without this crate depending on it.

/// Sizes and behaviours the workspace may override.
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// Interface zoom (`workspace::zoom_*`), 1.0 being 100 %. Every pixel size
    /// the panel paints — the type scale, row heights, radii, the composer —
    /// is multiplied by it (see [`ChatSettings::px`]).
    pub scale: f32,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            input_min_rows: 1,
            input_max_rows: 8,
            diff_preview_lines: 3,
            collapse_thoughts: true,
            max_entries: 2000,
            scale: 1.0,
        }
    }
}

impl ChatSettings {
    /// A pixel size of the panel at the current zoom.
    #[must_use]
    pub fn px(&self, value: f32) -> gpui::Pixels {
        gpui::px(value * self.scale)
    }

    /// [`TEXT_BODY`] at the current zoom, in pixels.
    #[must_use]
    pub fn text_body(&self) -> f32 {
        TEXT_BODY * self.scale
    }

    /// [`TEXT_CODE`] at the current zoom, in pixels.
    #[must_use]
    pub fn text_code(&self) -> f32 {
        TEXT_CODE * self.scale
    }

    /// [`TEXT_SMALL`] at the current zoom, in pixels.
    #[must_use]
    pub fn text_small(&self) -> f32 {
        TEXT_SMALL * self.scale
    }

    /// [`TEXT_LABEL`] at the current zoom, in pixels.
    #[must_use]
    pub fn text_label(&self) -> f32 {
        TEXT_LABEL * self.scale
    }

    /// [`heading_size`] at the current zoom, in pixels.
    #[must_use]
    pub fn heading(&self, level: u8) -> f32 {
        heading_size(level) * self.scale
    }
}

/// Height of a tool-call row, in pixels (`02-visual.md` §7).
pub const TOOL_ROW_HEIGHT: f32 = 28.;
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
/// Fill of a user bubble: `text.accent` at this alpha over `bg.app`.
pub const BUBBLE_FILL_ALPHA: f32 = 0.14;
/// Border of a user bubble: 1 px of `text.accent` at this alpha.
pub const BUBBLE_BORDER_ALPHA: f32 = 0.35;
/// Corner radius of a fenced code block in an agent answer.
pub const CODE_BLOCK_RADIUS: f32 = 6.;
/// Room above the code of a fenced block for its language / "Copiar" header.
pub const CODE_BLOCK_HEADER: f32 = 22.;
/// Vertical gap between two messages, in pixels (`02-visual.md` §7).
pub const MESSAGE_GAP: f32 = 16.;
/// Vertical gap between the compact rows of a turn (tool cards, thoughts).
pub const ROW_GAP: f32 = 4.;
/// Vertical space that replaces the old "turno terminado · X s" separator.
pub const TURN_GAP: f32 = 12.;
/// Lines of command output a tool card shows before "ver más".
pub const MAX_OUTPUT_LINES: usize = 20;

// ------------------------------------------------------------- type scale
//
// One scale for the whole panel (`docs/etapas/etapa-3.md` § correcciones):
// nothing in the chat is painted at a size outside this list.

/// Body text: messages, answers, cards, popover rows.
pub const TEXT_BODY: f32 = 13.;
/// Code, inline and in blocks (monospace): fenced blocks, commands, outputs,
/// diff context.
pub const TEXT_CODE: f32 = 12.5;
/// Tool rows, plan items, thoughts and hints.
pub const TEXT_SMALL: f32 = 12.;
/// Labels and timestamps: selector chips, the status pill, `+N −M`, dates.
pub const TEXT_LABEL: f32 = 11.;
/// The largest a markdown heading gets inside an answer.
pub const TEXT_HEADING_MAX: f32 = 15.;

/// The size of a markdown heading of `level` (1–6): 15 / 14 / 13 px, never
/// above [`TEXT_HEADING_MAX`]; the weight, not the size, carries the rest.
#[must_use]
pub fn heading_size(level: u8) -> f32 {
    match level {
        1 => TEXT_HEADING_MAX,
        2 => 14.,
        _ => TEXT_BODY,
    }
}

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

    #[test]
    fn the_zoom_multiplies_every_size() {
        let zoomed = ChatSettings {
            scale: 1.5,
            ..ChatSettings::default()
        };
        assert_eq!(zoomed.text_body(), 19.5);
        assert_eq!(zoomed.text_code(), 18.75);
        assert_eq!(zoomed.text_small(), 18.);
        assert_eq!(zoomed.text_label(), 16.5);
        assert_eq!(zoomed.heading(1), 22.5);
        assert_eq!(zoomed.px(TOOL_ROW_HEIGHT), gpui::px(42.));
        let plain = ChatSettings::default();
        assert_eq!(plain.scale, 1.0);
        assert_eq!(plain.text_body(), TEXT_BODY);
    }

    #[test]
    fn the_type_scale_is_the_one_the_author_asked_for() {
        assert_eq!(TEXT_BODY, 13.);
        assert_eq!(TEXT_CODE, 12.5);
        assert_eq!(TEXT_SMALL, 12.);
        assert_eq!(TEXT_LABEL, 11.);
        for level in 1..=6 {
            assert!(heading_size(level) <= TEXT_HEADING_MAX, "h{level}");
            assert!(heading_size(level) >= TEXT_BODY, "h{level}");
        }
        assert_eq!(heading_size(1), 15.);
    }
}
