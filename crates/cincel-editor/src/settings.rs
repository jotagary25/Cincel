//! Settings the editor element needs, and the buffer handle it is built on.
//!
//! These are plain structs with `Default`s on purpose: `cincel-workspace`
//! maps its own settings type onto them, so `cincel-editor` never depends on
//! `cincel-settings`.

use std::sync::Arc;

use cincel_text::Buffer;
use parking_lot::Mutex;

/// A buffer shared between the editor, the project's `BufferStore` and the
/// agent side of the app.
///
/// The editor never owns the buffer: it locks it for each read or edit and
/// releases the lock before doing anything else, so the store can hand the same
/// handle to more than one view (split panes) and to background work.
pub type SharedBuffer = Arc<Mutex<Buffer>>;

/// Wraps a buffer in a [`SharedBuffer`]. Convenience for hosts that do not have
/// a `BufferStore` yet (the example, the tests).
pub fn shared(buffer: Buffer) -> SharedBuffer {
    Arc::new(Mutex::new(buffer))
}

/// How much of the editor's own furniture is painted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorChrome {
    /// The code editor: gutter with line numbers and diff bars, current-line
    /// background, matching-bracket boxes, search bar and go-to-line prompt.
    #[default]
    Full,
    /// A text field built on the editor (the chat composer): no gutter, no
    /// current-line background, no bracket boxes and no search bar or prompt.
    /// The text starts at the left edge of the element.
    Minimal,
}

/// Height the element asks for itself instead of filling its parent: the
/// number of wrap rows of the text, clamped to `min_rows..=max_rows`. Past
/// `max_rows` the text scrolls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoHeight {
    /// Rows shown even when the text is shorter (at least 1).
    pub min_rows: u32,
    /// Rows the element grows to before it starts scrolling.
    pub max_rows: u32,
}

/// Everything the element reads from the user's settings.
///
/// Defaults follow `docs/specs/02-visual.md` §3 and §5.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorSettings {
    /// Soft wrap at the viewport width. On by default (02-visual §5);
    /// `Alt+Z` toggles it per view.
    pub soft_wrap: bool,
    /// Columns a tab is worth, and the width of one indent level.
    pub tab_size: u32,
    /// Whether `tab` inserts spaces instead of a tab character.
    pub insert_spaces: bool,
    /// Paint `·` for spaces and `→` for tabs.
    pub show_whitespace: bool,
    /// Vertical guide at this column, if any (02-visual §5 suggests 100).
    pub ruler: Option<u32>,
    /// Whether the cursor blinks at all.
    pub cursor_blink: bool,
    /// Auto-close typed brackets and quotes (`(`, `[`, `{`, `"`, `'`, `` ` ``),
    /// as [`EditorView::set_auto_close_pairs`](crate::EditorView::set_auto_close_pairs)
    /// toggles at runtime. `EditorView::new` and `EditorView::set_settings`
    /// both take the initial value from here.
    pub auto_close_pairs: bool,
    /// Monospace families, best first. The first one the platform knows wins,
    /// the rest become the font's fallback chain.
    pub font_family: Vec<String>,
    /// Font size in pixels.
    pub font_size: f32,
    /// Row height as a multiple of the font size.
    pub line_height: f32,
    /// After accepting or rejecting a hunk, move the cursor to the next pending
    /// one when the host sends the new review (`review.jump_to_next_on_decide`).
    /// Off by default, like the setting.
    pub jump_to_next_on_decide: bool,
    /// Gutter and other code-editor furniture ([`EditorChrome::Full`] by
    /// default).
    pub chrome: EditorChrome,
    /// Ask for a height that fits the text instead of filling the parent.
    /// `None` (the default) fills the parent, as the code editor does.
    pub auto_height: Option<AutoHeight>,
    /// Proportional families for the rows that are not code, best first.
    ///
    /// `None` (the default) paints every row with [`Self::font_family`]. When
    /// set, only the rows a [`crate::TextDecorations`] marks as monospace keep
    /// the code font; the rest (and the placeholder) use this one, and soft
    /// wrap measures real glyph advances instead of monospace columns.
    pub prose_font_family: Option<Vec<String>>,
    /// Zoom of the interface (`Ctrl+=` / `Ctrl+-`), 1 by default: the comment
    /// boxes, the comment mark and the hunk buttons are scaled by it, as the
    /// rest of the interface is (`02-visual.md` §7). The code itself already
    /// comes scaled in [`Self::font_size`].
    pub ui_scale: f32,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            soft_wrap: true,
            tab_size: 4,
            insert_spaces: true,
            show_whitespace: false,
            ruler: None,
            cursor_blink: true,
            auto_close_pairs: true,
            font_family: vec![
                "JetBrains Mono".to_string(),
                "JetBrainsMono Nerd Font Mono".to_string(),
                "Zed Mono".to_string(),
                "DejaVu Sans Mono".to_string(),
                "monospace".to_string(),
            ],
            font_size: 14.,
            line_height: 1.5,
            jump_to_next_on_decide: false,
            chrome: EditorChrome::Full,
            auto_height: None,
            prose_font_family: None,
            ui_scale: 1.,
        }
    }
}

impl EditorSettings {
    /// [`Self::ui_scale`], never below a tenth.
    pub fn scale(&self) -> f32 {
        if self.ui_scale.is_finite() {
            self.ui_scale.max(0.1)
        } else {
            1.
        }
    }

    /// The string one indent level inserts.
    pub fn indent_unit(&self) -> String {
        if self.insert_spaces {
            " ".repeat(self.tab_size.max(1) as usize)
        } else {
            "\t".to_string()
        }
    }
}

/// Indentation detected in the text of a buffer, used before falling back to
/// [`EditorSettings::tab_size`] / [`EditorSettings::insert_spaces`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indentation {
    /// Whether the file indents with spaces.
    pub insert_spaces: bool,
    /// Width of one indent level, in columns.
    pub size: u32,
}

/// Guesses the indentation of a file from its first `sample_lines` lines.
///
/// A file that starts a line with a tab is a tab file. Otherwise the most
/// common non-zero difference between the leading-space counts of consecutive
/// lines is the indent width; ties go to the smaller width.
pub fn detect_indentation(text: &str, sample_lines: usize) -> Option<Indentation> {
    let mut counts = [0usize; 9];
    let mut previous = 0usize;
    let mut saw_space_indent = false;
    for line in text.lines().take(sample_lines) {
        if line.starts_with('\t') {
            return Some(Indentation {
                insert_spaces: false,
                size: 4,
            });
        }
        if line.trim().is_empty() {
            continue;
        }
        let spaces = line.len() - line.trim_start_matches(' ').len();
        if spaces > 0 {
            saw_space_indent = true;
        }
        if spaces > previous {
            let width = spaces - previous;
            if (1..counts.len()).contains(&width) {
                counts[width] += 1;
            }
        }
        previous = spaces;
    }
    if !saw_space_indent {
        return None;
    }
    let size = counts
        .iter()
        .enumerate()
        .skip(1)
        .max_by_key(|(width, count)| (**count, std::cmp::Reverse(*width)))
        .filter(|(_, count)| **count > 0)
        .map(|(width, _)| width as u32)?;
    Some(Indentation {
        insert_spaces: true,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_tabs() {
        let detected = detect_indentation("fn main() {\n\tlet x = 1;\n}\n", 50);
        assert_eq!(
            detected,
            Some(Indentation {
                insert_spaces: false,
                size: 4
            })
        );
    }

    #[test]
    fn detects_two_space_indentation() {
        let text = "a\n  b\n    c\n  d\ne\n  f\n";
        let detected = detect_indentation(text, 50).expect("indentation");
        assert!(detected.insert_spaces);
        assert_eq!(detected.size, 2);
    }

    #[test]
    fn detects_four_space_indentation() {
        let text = "fn main() {\n    let x = 1;\n    if x > 0 {\n        run();\n    }\n}\n";
        let detected = detect_indentation(text, 50).expect("indentation");
        assert_eq!(detected.size, 4);
    }

    #[test]
    fn flat_text_has_no_indentation() {
        assert_eq!(detect_indentation("a\nb\nc\n", 50), None);
    }

    #[test]
    fn indent_unit_follows_the_settings() {
        let mut settings = EditorSettings::default();
        assert_eq!(settings.indent_unit(), "    ");
        settings.insert_spaces = false;
        assert_eq!(settings.indent_unit(), "\t");
    }
}
