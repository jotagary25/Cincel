//! `EditorElement`: the GPUI element that lays out and paints the editor.
//!
//! Only the wrap rows inside the viewport (± one row) are shaped and painted.
//! The shaped lines live in a content-addressed cache in the view (see
//! `EditorView::shaped_line`): a row is shaped again only when its painted text
//! or one of its colours actually changed, so a keystroke reshapes the row it
//! landed on and nothing else.
//!
//! # Review layers (02-visual §6)
//!
//! On top of the rows the element paints, in this order: the diff row
//! backgrounds, the word diffs, the 3 px per-row bar and the 2 px per-hunk
//! border of the gutter, the text, the `+`/`−` icons of the hovered line, the
//! accept/reject pills, the "Turno anterior" tooltip and the floating review
//! bar. The pills are an **overlay**, not a `BlockMap` row: they occupy no
//! vertical space, and since their position comes from the same row → y
//! mapping as the text they scroll with it and follow the hunk when rows are
//! inserted above it, frame for frame.
//!
//! # Comment blocks (spec 09 §6.5)
//!
//! The rows are placed by their *visual* row ([`crate::BlockMap`]): a comment
//! box takes whole rows below the row it hangs from, and the element lays it
//! out and paints it as a child element in the strip of its rows, from the
//! text's left edge to 12 px before the scrollbar — only while it is in the
//! viewport. The gutter keeps its width and the text its column; the comment
//! mark goes in the 12 px gap between the numbers and the text.
//!
//! # Where the pill goes
//!
//! Right aligned at the text area's right edge, 24 px high, on the first row
//! that leaves it room (`place_pill`): the hunk's first row, then its
//! following rows in order, then the row just above the hunk — the first one
//! whose shaped text ends at least [`PILL_TEXT_GAP`] before the pill's left
//! edge. When none does, a compact pill (three 24 × 24 icon buttons: `✓`,
//! `✗` and the comment icon) goes on the first row at 70 % opacity. Nothing
//! moves: the gutter and the text origin never depend on the review.
//!
//! # The `+`/`−` of a line
//!
//! Painted **over the line-number column** of the hovered hunk row (whose
//! number is hidden meanwhile; phantom rows have none), so showing them never
//! shifts the code. The column is at least [`MIN_NUMBER_DIGITS`] digits wide,
//! and the icons are 14 px when two of them and a 2 px gap fit in it, 11 px
//! otherwise ([`line_icon_size`]).

use std::collections::HashMap;
use std::ops::Range;
use std::time::Instant;

use cincel_syntax::HighlightId;
use gpui::BorderStyle;
use gpui::{
    App, AvailableSpace, Bounds, BoxShadow, Corners, CursorStyle, DispatchPhase, Element,
    ElementId, Entity, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    ScrollWheelEvent, ShapedLine, SharedString, Style, TextAlign, TextRun, Window, fill, hsla,
    point, px, quad, relative, size,
};

use crate::block_map::VisualCell;
use crate::display_map::{DisplayCell, DisplayRow, RowKind, RowText};
use crate::git_gutter::GitGutterKind;
use crate::input::WeakInputHandler;
use crate::review::{ReviewAction, ReviewHunkKind};
use crate::search::MatchLocation;
use crate::settings::EditorChrome;
use crate::theme::{
    self, CURRENT_LINE_ALPHA, DIFF_BG_ALPHA, DIFF_WORD_ALPHA, EditorTheme, OCCURRENCE_ALPHA,
    PHANTOM_TEXT_ALPHA, SEARCH_CURRENT_ALPHA, SEARCH_MATCH_ALPHA,
};
use crate::view::{
    BarButtonRender, EditorView, LayoutSnapshot, PillRender, ReviewFrame, RowRender,
};
use crate::wrap_map::WrapRow;

// Gutter, left to right (`docs/specs/07-etapa5-productividad.md` §6.3):
// padding 2, git column 3, gap 2, agent bar 3, gap 3 — the same 13 px the
// padding (4), the agent bar (3) and its gap (6) took before the git column
// existed, so the numbers and the text sit exactly where they always did.

/// Left padding of the gutter, before the git column.
pub(crate) const GUTTER_PADDING_LEFT: f32 = 2.;
/// Width of the git column (added, modified, deletion mark).
pub(crate) const GIT_BAR_WIDTH: f32 = 3.;
/// Gap between the git column and the agent's diff bar.
pub(crate) const GIT_BAR_GAP: f32 = 2.;
/// Width of the agent's per-row diff bar in the gutter.
pub(crate) const GUTTER_BAR_WIDTH: f32 = 3.;
/// Gap between the agent's diff bar and the line numbers.
pub(crate) const GUTTER_BAR_GAP: f32 = 3.;
/// Everything before the line numbers: the width the gutter always had
/// there.
pub(crate) const GUTTER_BEFORE_NUMBERS: f32 =
    GUTTER_PADDING_LEFT + GIT_BAR_WIDTH + GIT_BAR_GAP + GUTTER_BAR_WIDTH + GUTTER_BAR_GAP;
/// Height of the mark where lines of `HEAD` were deleted.
pub(crate) const GIT_DELETED_MARK_HEIGHT: f32 = 6.;
/// Gap between the line numbers and the text.
pub(crate) const GUTTER_TEXT_GAP: f32 = 12.;
/// Height of the accept/reject pill (02-visual §6.1).
pub const PILL_HEIGHT: f32 = 24.;
/// Width of the pill: "✓ Aceptar" · "✗ Rechazar" · "Comentar" in three
/// equal parts (spec 09 §6.3, which widens the 190 px of 02-visual §6.1).
pub const PILL_WIDTH: f32 = 280.;
/// Margin between the pill and the right edge of the text area.
pub const PILL_MARGIN: f32 = 12.;
/// Font size of the pill labels.
const PILL_FONT_SIZE: f32 = 12.;
/// Room the pill wants between the end of a row's text and its left edge.
pub const PILL_TEXT_GAP: f32 = 12.;
/// Side of a button of the compact pill.
pub const COMPACT_PILL_BUTTON: f32 = 24.;
/// Gap between the buttons of the compact pill.
pub const COMPACT_PILL_GAP: f32 = 4.;
/// Opacity of the compact pill, which sits over code.
pub const COMPACT_PILL_OPACITY: f32 = 0.7;
/// Side of the `+`/`−` line icons (02-visual §6.1: 14 px).
pub const LINE_ICON_SIZE: f32 = 14.;
/// Side of the line icons when two 14 px ones do not fit in the number column.
pub const LINE_ICON_SIZE_SMALL: f32 = 11.;
/// Gap between the two line icons.
pub const LINE_ICON_GAP: f32 = 2.;
/// Digits the line-number column is always wide enough for, so the `+`/`−`
/// icons painted over it fit however short the file is.
pub const MIN_NUMBER_DIGITS: usize = 3;
/// Width of the per-hunk border of the gutter (02-visual §6.1: 2 px).
const HUNK_BORDER_WIDTH: f32 = 2.;
/// Margin of the floating bar from the bottom-right corner (02-visual §6.2).
pub const BAR_MARGIN: f32 = 24.;
/// Height of the floating bar.
pub const BAR_HEIGHT: f32 = 30.;
/// Horizontal padding of the floating bar.
pub(crate) const BAR_PADDING: f32 = 12.;
/// Font size of the floating bar (02-visual §3: UI 13 px).
const BAR_FONT_SIZE: f32 = 13.;
/// Side of the spinner and the clock icons.
const ICON_SIZE: f32 = 12.;
/// Width of the scrollbar (02-visual §5).
pub(crate) const SCROLLBAR_WIDTH: f32 = 8.;
/// Padding at both ends of the full pill (its surface, not a button).
pub const PILL_PADDING: f32 = 4.;
/// Side of the "Comentar" icon of the pill (spec 09 §6.3: 12 px).
pub const PILL_COMMENT_ICON: f32 = 12.;
/// Side of a comment mark in the margin (spec 09 §6.3: 10 px).
pub const COMMENT_MARK_SIZE: f32 = 10.;
/// Shortest the scrollbar thumb can get.
const SCROLLBAR_MIN_THUMB: f32 = 24.;
/// Extra wrap rows shaped above and below the viewport.
const OVERSCAN: u32 = 1;

/// Side of the `+`/`−` icons for a line-number column `column` px wide: 14 px
/// when two of them and the 2 px gap fit, else 11 px, else whatever fits.
pub fn line_icon_size(column: f32) -> f32 {
    if 2. * LINE_ICON_SIZE + LINE_ICON_GAP <= column {
        LINE_ICON_SIZE
    } else if 2. * LINE_ICON_SIZE_SMALL + LINE_ICON_GAP <= column {
        LINE_ICON_SIZE_SMALL
    } else {
        ((column - LINE_ICON_GAP) / 2.).floor().max(1.)
    }
}

/// Horizontal geometry of the gutter, from the width of a digit.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GutterMetrics {
    /// Advance of `0` in the code font.
    pub char_width: Pixels,
    /// Width of the line-number column (0 with the minimal chrome).
    pub number_width: Pixels,
    /// Width of the whole gutter (0 with the minimal chrome).
    pub gutter_width: Pixels,
}

/// The gutter of `view`. It never depends on the review nor on the git
/// diff: showing or hiding hunks must not move the text (02-visual §6.1,
/// spec 07 D11/D16).
pub(crate) fn gutter_metrics(view: &EditorView, window: &Window) -> GutterMetrics {
    let char_width = shape(
        "0".to_string(),
        &view.style.font,
        view.style.font_size,
        hsla(0., 0., 0., 1.),
        window,
    )
    .width();
    if view.settings().chrome == EditorChrome::Minimal {
        return GutterMetrics {
            char_width,
            number_width: px(0.),
            gutter_width: px(0.),
        };
    }
    let digits = view
        .display_map
        .buffer_row_count()
        .max(1)
        .to_string()
        .len()
        .max(MIN_NUMBER_DIGITS) as f32;
    let number_width = char_width * digits;
    GutterMetrics {
        char_width,
        number_width,
        gutter_width: px(GUTTER_BEFORE_NUMBERS) + number_width + px(GUTTER_TEXT_GAP),
    }
}

/// A wrap row ready to paint.
struct RowLayout {
    wrap_row: WrapRow,
    display_row: DisplayRow,
    segment: u32,
    kind: RowKind,
    line: ShapedLine,
    whitespace: Option<ShapedLine>,
    number: Option<ShapedLine>,
    origin_y: Pixels,
    /// Horizontal offset of the segment (soft-wrap indentation).
    indent_x: Pixels,
    /// Byte range of the segment inside the row source text.
    source_range: Range<u32>,
    /// Maps source bytes of the row onto the painted text.
    row_text: RowText,
    /// Word-diff quads over the row background.
    words: Vec<(Bounds<Pixels>, Hsla)>,
    /// Full-width background from the host's decorations, if any.
    background: Option<Hsla>,
    /// The bar of the git column on this row (never on a phantom row).
    git_bar: Option<(GitGutterKind, Hsla)>,
}

impl RowLayout {
    /// X of a source byte offset inside this segment.
    fn x_for(&self, source_byte: u32) -> Pixels {
        let start = self.row_text.to_display(self.source_range.start);
        let display = self
            .row_text
            .to_display(source_byte.clamp(self.source_range.start, self.source_range.end));
        self.indent_x + self.line.x_for_index((display - start) as usize)
    }
}

/// The small icon at the left of a pill or of the bar.
#[derive(Clone, Copy, PartialEq)]
enum Glyph {
    /// "Turno anterior".
    Clock,
    /// The agent is writing.
    Spinner(u32),
}

/// An accept/reject pill ready to paint.
struct PillLayout {
    hunk: u64,
    enabled: bool,
    /// Display row it sits on.
    display_row: DisplayRow,
    /// The two-icon version, painted over code.
    compact: bool,
    opacity: f32,
    bounds: Bounds<Pixels>,
    accept: Bounds<Pixels>,
    reject: Bounds<Pixels>,
    /// The third part, "Comentar" (enabled while the agent writes too).
    comment: Bounds<Pixels>,
    accept_line: ShapedLine,
    reject_line: ShapedLine,
    /// The word "Comentar" (`None` on the compact pill, icon only).
    comment_line: Option<ShapedLine>,
    accept_x: Pixels,
    reject_x: Pixels,
    comment_x: Pixels,
    /// Where the "Comentar" icon goes.
    comment_icon: Bounds<Pixels>,
    /// The separators between the parts (none on the compact pill).
    separators: Vec<Pixels>,
    glyph: Option<(Glyph, Bounds<Pixels>)>,
    previous_turn: bool,
    check_color: Hsla,
    cross_color: Hsla,
    label_color: Hsla,
    comment_icon_color: Hsla,
    comment_label_color: Hsla,
    /// The half under the mouse (`true` = accept).
    hovered: Option<bool>,
    /// Whether "Comentar" is under the mouse.
    comment_hovered: bool,
    /// The whole pill, so the text's I-beam never shows over it.
    surface_hitbox: Option<Hitbox>,
    accept_hitbox: Option<Hitbox>,
    reject_hitbox: Option<Hitbox>,
    comment_hitbox: Option<Hitbox>,
}

/// The `+`/`−` icons of the hovered line.
struct LineIconsLayout {
    hunk: u64,
    line: usize,
    /// Display row whose line number they replace.
    display_row: DisplayRow,
    accept: Bounds<Pixels>,
    reject: Bounds<Pixels>,
    accept_hitbox: Option<Hitbox>,
    reject_hitbox: Option<Hitbox>,
}

/// One text segment of the floating bar.
struct BarItem {
    text: String,
    line: ShapedLine,
    x: Pixels,
    action: Option<ReviewAction>,
    hitbox: Option<Hitbox>,
    glyph_color: Hsla,
    label_color: Hsla,
    hovered: bool,
}

/// The floating review bar ready to paint.
struct BarLayout {
    bounds: Bounds<Pixels>,
    glyph: Option<(Glyph, Bounds<Pixels>)>,
    items: Vec<BarItem>,
    enabled: bool,
    /// The whole bar, so the text's I-beam never shows over it.
    surface_hitbox: Option<Hitbox>,
}

/// The "Turno anterior" tooltip.
struct TooltipLayout {
    bounds: Bounds<Pixels>,
    line: ShapedLine,
}

/// State computed during `prepaint`.
pub struct EditorPrepaint {
    hitbox: Hitbox,
    rows: Vec<RowLayout>,
    pills: Vec<PillLayout>,
    line_icons: Option<LineIconsLayout>,
    bar: Option<BarLayout>,
    tooltip: Option<TooltipLayout>,
    /// Per-hunk gutter borders.
    hunk_borders: Vec<(Bounds<Pixels>, Hsla)>,
    /// Deletion marks of the git column (the 3 × 6 mark and its 1 px line),
    /// painted over the row boundaries.
    git_deleted: Vec<(Bounds<Pixels>, Hsla)>,
    /// X of the git column.
    git_x: Pixels,
    /// Right edge of the line numbers.
    numbers_right: Pixels,
    /// The placeholder, shaped, while the buffer is empty.
    placeholder: Option<ShapedLine>,
    cursor: Option<Bounds<Pixels>>,
    marked: Vec<Bounds<Pixels>>,
    selections: Vec<Bounds<Pixels>>,
    /// The bracket at or before the cursor and its match.
    brackets: Vec<Bounds<Pixels>>,
    matches: Vec<(Bounds<Pixels>, bool)>,
    /// The occurrences of the word under the cursor (spec 10 §7.1), painted
    /// right before [`Self::matches`].
    occurrence_quads: Vec<Bounds<Pixels>>,
    current_line: Vec<Bounds<Pixels>>,
    text_origin_x: Pixels,
    gutter_width: Pixels,
    line_height: Pixels,
    scroll_left: Pixels,
    ruler_x: Option<Pixels>,
    max_scroll: f32,
    max_scroll_x: f32,
    scrollbar: Option<(Bounds<Pixels>, Bounds<Pixels>, f32)>,
    theme: EditorTheme,
    /// What is not text and shows the arrow: the gutter and the strip the
    /// vertical scrollbar lives in (its track and thumb). Filled after the
    /// view is laid out.
    margin_hitboxes: Vec<Hitbox>,
    /// The visible comment blocks, laid out and prepainted as children.
    blocks: Vec<BlockPaint>,
    /// The comment marks of the margin.
    marks: Vec<MarkLayout>,
    /// The tooltip of the hovered mark: its surface, its lines, their height
    /// and the padding.
    mark_tooltip: Option<(Bounds<Pixels>, Vec<ShapedLine>, Pixels, Pixels)>,
}

/// A comment block ready to paint: the strip of its rows and its element.
struct BlockPaint {
    /// The strip of its rows (its element is laid out inside it).
    bounds: Bounds<Pixels>,
    element: gpui::AnyElement,
}

/// A comment mark of the margin.
struct MarkLayout {
    id: u64,
    bounds: Bounds<Pixels>,
    hitbox: Option<Hitbox>,
}

/// The editor element.
pub struct EditorElement {
    view: Entity<EditorView>,
    started_at: Instant,
}

impl EditorElement {
    /// Builds the element for a view.
    pub fn new(view: Entity<EditorView>) -> Self {
        Self {
            view,
            started_at: Instant::now(),
        }
    }
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

fn text_run(len: usize, font: &gpui::Font, color: Hsla) -> TextRun {
    TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// Shapes a one-colour line straight through the text system, for the handful
/// of strings that are not worth a cache entry (the digit the gutter is
/// measured with, the pill labels).
fn shape(
    text: String,
    font: &gpui::Font,
    font_size: Pixels,
    color: Hsla,
    window: &Window,
) -> ShapedLine {
    let run = text_run(text.len(), font, color);
    let runs: &[TextRun] = if run.len == 0 {
        &[]
    } else {
        std::slice::from_ref(&run)
    };
    window
        .text_system()
        .shape_line(SharedString::from(text), font_size, runs, None)
}

/// Greedy word wrap of one paragraph into lines no wider than `max_width`,
/// measured with `shape` (a word longer than the line stays on its own line).
fn wrap_words(
    paragraph: &str,
    max_width: Pixels,
    mut shape: impl FnMut(&str) -> ShapedLine,
) -> Vec<ShapedLine> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut shaped: Option<ShapedLine> = None;
    for word in paragraph.split(' ').filter(|word| !word.is_empty()) {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        let line = shape(&candidate);
        if line.width() > max_width && !current.is_empty() {
            lines.extend(shaped.take());
            current = word.to_string();
            shaped = Some(shape(&current));
        } else {
            current = candidate;
            shaped = Some(line);
        }
    }
    lines.extend(shaped);
    if lines.is_empty() {
        lines.push(shape(""));
    }
    lines
}

/// Shapes a one-colour line through the view's cache.
fn shape_cached(
    view: &mut EditorView,
    text: &str,
    font: &gpui::Font,
    font_size: Pixels,
    color: Hsla,
    window: &Window,
) -> (ShapedLine, bool) {
    let run = text_run(text.len(), font, color);
    let runs: &[TextRun] = if run.len == 0 {
        &[]
    } else {
        std::slice::from_ref(&run)
    };
    view.shaped_line(text, runs, font_size, window)
}

/// Builds the per-highlight runs of one painted segment.
///
/// `spans` are buffer byte ranges; `base` is the buffer offset of byte 0 of the
/// row, `range` the source byte range of the segment, and the run lengths come
/// out in bytes of the *painted* (tab expanded) text.
///
/// Returns the runs and how many spans landed on the segment — the number the
/// render probe reports as `highlight_spans`.
#[allow(clippy::too_many_arguments)]
fn runs_for_segment(
    spans: &[(Range<usize>, HighlightId)],
    base: usize,
    range: &Range<u32>,
    row_text: &RowText,
    font: &gpui::Font,
    default: Hsla,
    theme: &EditorTheme,
    opacity: f32,
) -> (Vec<TextRun>, usize) {
    let start_display = row_text.to_display(range.start);
    let end_display = row_text.to_display(range.end);
    let total = (end_display - start_display) as usize;
    if total == 0 {
        return (Vec::new(), 0);
    }
    let mut used = 0usize;
    let mut runs: Vec<TextRun> = Vec::new();
    let push = |len: usize, color: Hsla, runs: &mut Vec<TextRun>| {
        if len == 0 {
            return;
        }
        if let Some(last) = runs.last_mut()
            && last.color == color
        {
            last.len += len;
            return;
        }
        runs.push(text_run(len, font, color));
    };

    let mut cursor = start_display;
    for (span, id) in spans {
        let span_start = span.start.saturating_sub(base) as u32;
        let span_end = span.end.saturating_sub(base) as u32;
        if span_end <= range.start || span_start >= range.end {
            continue;
        }
        let from = row_text.to_display(span_start.max(range.start));
        let to = row_text.to_display(span_end.min(range.end));
        if to <= cursor {
            continue;
        }
        push((from.saturating_sub(cursor)) as usize, default, &mut runs);
        let mut color = theme.syntax_color(*id);
        color.a = opacity;
        push((to - from.max(cursor)) as usize, color, &mut runs);
        cursor = to;
        used += 1;
    }
    push(
        (end_display.saturating_sub(cursor)) as usize,
        default,
        &mut runs,
    );

    // The shaper needs the runs to cover the text exactly.
    let covered: usize = runs.iter().map(|run| run.len).sum();
    if covered < total {
        push(total - covered, default, &mut runs);
    }
    (runs, used)
}

/// The whitespace overlay of a segment: `·` for a space, `→` for a tab, a blank
/// for everything else, so it lines up with the painted text in a monospace
/// font.
fn whitespace_overlay(source: &str, tab_size: u32) -> Option<String> {
    if !source.contains([' ', '\t']) {
        return None;
    }
    let mut overlay = String::with_capacity(source.len());
    for ch in source.chars() {
        match ch {
            ' ' => overlay.push('·'),
            '\t' => {
                overlay.push('→');
                for _ in 1..tab_size.max(1) {
                    overlay.push(' ');
                }
            }
            _ => overlay.push(' '),
        }
    }
    Some(overlay)
}

/// Shapes a one-colour interface string (pill, bar, tooltip) through the
/// view's cache.
fn shape_ui(
    view: &mut EditorView,
    text: &str,
    font_size: f32,
    color: Hsla,
    window: &Window,
) -> ShapedLine {
    let font = view.style.ui_font.clone();
    shape_cached(view, text, &font, px(font_size), color, window).0
}

/// Shapes an interface string made of coloured pieces (`✓` in `status.ok`
/// and " Aceptar" in `text`, say) through the view's cache.
fn shape_ui_runs(
    view: &mut EditorView,
    pieces: &[(&str, Hsla)],
    font_size: f32,
    window: &Window,
) -> ShapedLine {
    let font = view.style.ui_font.clone();
    let text: String = pieces.iter().map(|(piece, _)| *piece).collect();
    let runs: Vec<TextRun> = pieces
        .iter()
        .filter(|(piece, _)| !piece.is_empty())
        .map(|(piece, color)| text_run(piece.len(), &font, *color))
        .collect();
    view.shaped_line(&text, &runs, px(font_size), window).0
}

/// Splits a control label into its leading glyph (`✓`, `✗`, `↑`, `↓`, `→`)
/// and the rest, so each gets its own colour.
fn split_glyph(label: &str) -> (&str, &str) {
    match label.chars().next() {
        Some(first) if matches!(first, '✓' | '✗' | '↑' | '↓' | '→') => {
            label.split_at(first.len_utf8())
        }
        _ => ("", label),
    }
}

/// Paints the clock or one frame of the spinner inside `bounds`.
fn paint_glyph(glyph: Glyph, bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let center = bounds.center();
    let radius = f32::from(bounds.size.width) / 2.;
    match glyph {
        Glyph::Clock => {
            window.paint_quad(quad(
                bounds,
                Corners::all(px(radius)),
                hsla(0., 0., 0., 0.),
                px(1.2),
                color,
                BorderStyle::default(),
            ));
            // Hands at ten past twelve.
            window.paint_quad(fill(
                Bounds::new(
                    point(center.x - px(0.6), center.y - px(radius - 2.5)),
                    size(px(1.2), px(radius - 2.5)),
                ),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(
                    point(center.x - px(0.6), center.y - px(0.6)),
                    size(px(radius - 2.), px(1.2)),
                ),
                color,
            ));
        }
        Glyph::Spinner(phase) => {
            const DOTS: u32 = 8;
            let orbit = radius - 1.5;
            for dot in 0..DOTS {
                let angle = dot as f32 / DOTS as f32 * std::f32::consts::TAU;
                let x = center.x + px(orbit * angle.cos());
                let y = center.y + px(orbit * angle.sin());
                // The dot the phase points at is the brightest, the ones
                // behind it fade out.
                let age = (phase % DOTS + DOTS - dot) % DOTS;
                let mut tint = color;
                tint.a = color.a * (1. - age as f32 / DOTS as f32 * 0.85);
                let mut dot_quad = fill(
                    Bounds::new(point(x - px(1.25), y - px(1.25)), size(px(2.5), px(2.5))),
                    tint,
                );
                dot_quad.corner_radii = Corners::all(px(1.25));
                window.paint_quad(dot_quad);
            }
        }
    }
}

/// Paints a `+` or `−` line icon.
fn paint_line_icon(
    bounds: Bounds<Pixels>,
    plus: bool,
    color: Hsla,
    theme: &EditorTheme,
    window: &mut Window,
) {
    let mut border = color;
    border.a = 0.6;
    window.paint_quad(quad(
        bounds,
        Corners::all(px(3.)),
        theme::color(theme.elevated),
        px(1.),
        border,
        BorderStyle::default(),
    ));
    let center = bounds.center();
    // 4 px arms on a 14 px icon, scaled down with it.
    let arm = (f32::from(bounds.size.width) * 0.29).max(2.);
    let thickness = 1.5;
    window.paint_quad(fill(
        Bounds::new(
            point(center.x - px(arm), center.y - px(thickness / 2.)),
            size(px(arm * 2.), px(thickness)),
        ),
        color,
    ));
    if plus {
        window.paint_quad(fill(
            Bounds::new(
                point(center.x - px(thickness / 2.), center.y - px(arm)),
                size(px(thickness), px(arm * 2.)),
            ),
            color,
        ));
    }
}

/// Paints a rounded, shadowed `bg.elevated` surface (pill, bar, tooltip) at
/// `opacity`.
fn paint_surface(
    bounds: Bounds<Pixels>,
    radius: f32,
    opacity: f32,
    theme: &EditorTheme,
    window: &mut Window,
) {
    let corners = Corners::all(px(radius));
    window.paint_drop_shadows(
        bounds,
        corners,
        &[BoxShadow {
            // 02-visual §4: `0 2px 8px #0006`.
            color: hsla(0., 0., 0., 0.4 * opacity),
            offset: point(px(0.), px(2.)),
            blur_radius: px(8.),
            spread_radius: px(0.),
            inset: false,
        }],
    );
    let mut surface = fill(bounds, theme::alpha(theme.elevated, opacity));
    surface.corner_radii = corners;
    window.paint_quad(surface);
}

/// Colours of a pill's `✓`, `✗` and words: `status.ok`, `status.error` and
/// `text` while it reacts, `text.muted` for all three while the agent writes.
pub(crate) fn pill_colors(theme: &EditorTheme, enabled: bool) -> (Hsla, Hsla, Hsla) {
    if enabled {
        (
            theme::color(theme.status_ok),
            theme::color(theme.status_error),
            theme::color(theme.text),
        )
    } else {
        let muted = theme::color(theme.text_muted);
        (muted, muted, muted)
    }
}

/// A row the pill may sit on: its display row, the y of its first wrap row
/// and where its painted text ends (`None` when it is not laid out).
struct PillCandidate {
    display_row: DisplayRow,
    row_y: Pixels,
    text_right: Pixels,
}

/// Where the pill of a hunk goes (module docs, "Where the pill goes"):
/// `Some((row, y, false))` for the first candidate whose text ends at least
/// [`PILL_TEXT_GAP`] before `pill_left`, else `(first row, y, true)` for the
/// compact pill. `None` when the hunk's first row is not on screen and no
/// candidate qualified.
fn place_pill(
    candidates: &[PillCandidate],
    first: Option<(DisplayRow, Pixels)>,
    pill_left: Pixels,
) -> Option<(DisplayRow, Pixels, bool)> {
    candidates
        .iter()
        .find(|candidate| candidate.text_right + px(PILL_TEXT_GAP) <= pill_left)
        .map(|candidate| (candidate.display_row, candidate.row_y, false))
        .or_else(|| first.map(|(row, y)| (row, y, true)))
}

/// Lays out the pill of hunk `hunk_ix` on the row at `row_y`: the full pill
/// ("✓ Aceptar" · "✗ Rechazar" · "Comentar", three equal parts), or the
/// compact one (three 24 × 24 icon buttons at 70 %) when no row had room.
#[allow(clippy::too_many_arguments)]
fn layout_pill(
    view: &mut EditorView,
    hunk_ix: usize,
    display_row: DisplayRow,
    row_y: Pixels,
    compact: bool,
    right: Pixels,
    line_height: Pixels,
    theme: &EditorTheme,
    window: &Window,
) -> PillLayout {
    let hunk = &view.review.hunks[hunk_ix];
    let id = hunk.id;
    let previous_turn = hunk.from_previous_turn;
    let turn_active = view.review.turn_active;
    let enabled = !turn_active;
    let (check_color, cross_color, label_color) = pill_colors(theme, enabled);
    let hovered = view
        .hover_pill
        .filter(|(hunk, _)| enabled && *hunk == id)
        .map(|(_, accept)| accept);
    let glyph = if turn_active {
        Some(Glyph::Spinner(view.spinner_phase))
    } else if previous_turn && !compact {
        Some(Glyph::Clock)
    } else {
        None
    };
    // Spec 09 §6.3: everything the pill paints follows the interface zoom.
    let scale = view.settings().scale();
    let pill_height = px(PILL_HEIGHT * scale);
    let icon = px(ICON_SIZE * scale);
    let comment_icon_size = px(PILL_COMMENT_ICON * scale);
    let font_size = PILL_FONT_SIZE * scale;
    // "Comentar" is not a decision: it reacts while the agent writes too.
    let comment_icon_color = theme::color(theme.text_accent);
    let comment_label_color = theme::color(theme.text);
    let comment_hovered = view.hover_pill_comment == Some(id);
    let top = row_y + (line_height - pill_height) / 2.;

    if compact {
        let faded = |color: Hsla| Hsla {
            a: color.a * COMPACT_PILL_OPACITY,
            ..color
        };
        let accept_line = shape_ui_runs(view, &[("✓", faded(check_color))], font_size, window);
        let reject_line = shape_ui_runs(view, &[("✗", faded(cross_color))], font_size, window);
        let button = px(COMPACT_PILL_BUTTON * scale);
        let gap = px(COMPACT_PILL_GAP * scale);
        // `✓`, `✗` and the comment icon; while the agent writes, a cell in
        // front holds the spinner.
        let cells = if glyph.is_some() { 4. } else { 3. };
        let width = button * cells + gap * (cells - 1.);
        let bounds = Bounds::new(point(right - width, top), size(width, pill_height));
        let first = bounds.left()
            + if glyph.is_some() {
                button + gap
            } else {
                px(0.)
            };
        let accept = Bounds::new(point(first, top), size(button, button));
        let reject = Bounds::new(point(first + button + gap, top), size(button, button));
        let comment = Bounds::new(
            point(first + (button + gap) * 2., top),
            size(button, button),
        );
        let glyph = glyph.map(|glyph| {
            (
                glyph,
                Bounds::new(
                    point(
                        bounds.left() + (button - icon) / 2.,
                        top + (pill_height - icon) / 2.,
                    ),
                    size(icon, icon),
                ),
            )
        });
        let accept_x = accept.left() + (button - accept_line.width()) / 2.;
        let reject_x = reject.left() + (button - reject_line.width()) / 2.;
        let comment_icon = Bounds::new(
            point(
                comment.left() + (button - comment_icon_size) / 2.,
                comment.top() + (button - comment_icon_size) / 2.,
            ),
            size(comment_icon_size, comment_icon_size),
        );
        return PillLayout {
            hunk: id,
            enabled,
            display_row,
            compact: true,
            opacity: COMPACT_PILL_OPACITY,
            bounds,
            accept,
            reject,
            comment,
            accept_line,
            reject_line,
            comment_line: None,
            accept_x,
            reject_x,
            comment_x: comment_icon.left(),
            comment_icon,
            separators: Vec::new(),
            glyph,
            previous_turn,
            check_color,
            cross_color,
            label_color,
            comment_icon_color: faded(comment_icon_color),
            comment_label_color,
            hovered,
            comment_hovered,
            surface_hitbox: None,
            accept_hitbox: None,
            reject_hitbox: None,
            comment_hitbox: None,
        };
    }

    let accept_line = shape_ui_runs(
        view,
        &[("✓", check_color), (" Aceptar", label_color)],
        font_size,
        window,
    );
    let reject_line = shape_ui_runs(
        view,
        &[("✗", cross_color), (" Rechazar", label_color)],
        font_size,
        window,
    );
    let comment_line = shape_ui_runs(
        view,
        &[(crate::comments::texts::COMMENT, comment_label_color)],
        font_size,
        window,
    );

    let width = px(PILL_WIDTH * scale);
    let bounds = Bounds::new(point(right - width, top), size(width, pill_height));
    // The clock or the spinner keeps a cell of its own at the left; the rest
    // splits into three equal parts.
    // A 4 px padding at both ends is the pill's own surface, not a button.
    let inset = px(PILL_PADDING * scale);
    let glyph_width = if glyph.is_some() {
        icon + px(10. * scale)
    } else {
        inset
    };
    let parts_left = bounds.left() + glyph_width;
    let parts_right = bounds.right() - inset;
    let part = (parts_right - parts_left) / 3.;
    let accept = Bounds::new(point(parts_left, bounds.top()), size(part, pill_height));
    let reject = Bounds::new(
        point(parts_left + part, bounds.top()),
        size(part, pill_height),
    );
    let comment = Bounds::from_corners(
        point(parts_left + part * 2., bounds.top()),
        point(parts_right, bounds.bottom()),
    );
    let glyph = glyph.map(|glyph| {
        (
            glyph,
            Bounds::new(
                point(
                    bounds.left() + px(6. * scale),
                    bounds.top() + (pill_height - icon) / 2.,
                ),
                size(icon, icon),
            ),
        )
    });
    let accept_x = accept.left() + ((part - accept_line.width()) / 2.).max(px(0.));
    let reject_x = reject.left() + ((part - reject_line.width()) / 2.).max(px(0.));
    let comment_gap = px(4. * scale);
    let content = comment_icon_size + comment_gap + comment_line.width();
    let comment_start = comment.left() + ((comment.size.width - content) / 2.).max(px(0.));
    let comment_icon = Bounds::new(
        point(
            comment_start,
            bounds.top() + (pill_height - comment_icon_size) / 2.,
        ),
        size(comment_icon_size, comment_icon_size),
    );
    let comment_x = comment_start + comment_icon_size + comment_gap;
    PillLayout {
        hunk: id,
        enabled,
        display_row,
        compact: false,
        opacity: 1.,
        bounds,
        accept,
        reject,
        comment,
        accept_line,
        reject_line,
        comment_line: Some(comment_line),
        accept_x,
        reject_x,
        comment_x,
        comment_icon,
        separators: vec![reject.left(), comment.left()],
        glyph,
        previous_turn,
        check_color,
        cross_color,
        label_color,
        comment_icon_color,
        comment_label_color,
        hovered,
        comment_hovered,
        surface_hitbox: None,
        accept_hitbox: None,
        reject_hitbox: None,
        comment_hitbox: None,
    }
}

/// One segment of the floating bar before it is shaped: text, colour of its
/// leading glyph, colour of the rest, action and gap before it.
type BarSpec = (String, Hsla, Hsla, Option<ReviewAction>, f32);

/// Lays out the floating review bar (02-visual §6.2), or `None` when it is
/// hidden.
///
/// "✓ Aceptar todo" and "✗ Rechazar todo" act on the agent's whole turn, in
/// every file ([`ReviewAction::AcceptTurn`] / [`ReviewAction::RejectTurn`],
/// the same as `Ctrl+Alt+↵` / `Ctrl+Alt+⌫`): the hunk under the cursor
/// already has its own pill and `Ctrl+↵` / `Ctrl+⌫`. The position ("cambio N
/// de M") follows the hunk under the cursor, else the host's current one
/// ([`EditorView::review_target`]). The buttons react whenever the bar does
/// (not while the agent writes, when the bar only shows the spinner), with
/// the glyph in `status.ok` / `status.error` and the word in `text`. The
/// hovered button gets a `bg.surface` background.
fn layout_bar(
    view: &mut EditorView,
    bounds: Bounds<Pixels>,
    theme: &EditorTheme,
    window: &Window,
) -> Option<BarLayout> {
    // Hidden while the window is not the active one.
    if !window.is_window_active() {
        return None;
    }
    let text = theme::color(theme.text);
    let muted = theme::color(theme.text_muted);
    let accent = theme::color(theme.text_accent);
    let (check, cross, label) = pill_colors(theme, true);
    let mut specs: Vec<BarSpec> = Vec::new();
    let mut glyph = None;
    let review = &view.review;
    let enabled = if review.turn_active {
        glyph = Some(Glyph::Spinner(view.spinner_phase));
        specs.push(("El agente está editando…".into(), text, text, None, 0.));
        false
    } else if !review.hunks.is_empty() {
        let target = view.review_target().unwrap_or(0);
        let total = review.pending_in_file.max(review.hunks.len());
        let sep = |specs: &mut Vec<BarSpec>| {
            specs.push(("·".into(), muted, muted, None, 10.));
        };
        // A file decided only as a whole (a deletion) has no buttons of its
        // own here: its single hunk's pill decides it.
        specs.push((
            "✓ Aceptar todo".into(),
            check,
            label,
            Some(ReviewAction::AcceptTurn),
            0.,
        ));
        specs.push(("Ctrl+Alt+↵".into(), muted, muted, None, 6.));
        sep(&mut specs);
        specs.push((
            "✗ Rechazar todo".into(),
            cross,
            label,
            Some(ReviewAction::RejectTurn),
            10.,
        ));
        specs.push(("Ctrl+Alt+⌫".into(), muted, muted, None, 6.));
        sep(&mut specs);
        specs.push((
            "↑ Alt+K".into(),
            text,
            text,
            Some(ReviewAction::PrevHunk),
            10.,
        ));
        specs.push((
            "↓ Alt+J".into(),
            text,
            text,
            Some(ReviewAction::NextHunk),
            8.,
        ));
        sep(&mut specs);
        specs.push((
            format!("cambio {} de {total}", target + 1),
            text,
            text,
            None,
            10.,
        ));
        sep(&mut specs);
        specs.push((
            "Revisar todo".into(),
            accent,
            accent,
            Some(ReviewAction::OpenReviewPanel),
            10.,
        ));
        true
    } else if view.review_notice {
        specs.push((
            format!(
                "Sin cambios pendientes en este archivo · {} en otros archivos",
                review.pending_in_other_files
            ),
            text,
            text,
            None,
            0.,
        ));
        specs.push((
            "→ siguiente archivo (Alt+L)".into(),
            accent,
            accent,
            Some(ReviewAction::NextFile),
            6.,
        ));
        true
    } else {
        return None;
    };

    let glyph_width = if glyph.is_some() {
        px(ICON_SIZE + 8.)
    } else {
        px(0.)
    };
    let hover_bar = view.hover_bar;
    let mut x = px(BAR_PADDING) + glyph_width;
    let mut items = Vec::with_capacity(specs.len());
    for (index, (text, glyph_color, label_color, action, gap)) in specs.into_iter().enumerate() {
        x += px(gap);
        let (head, tail) = split_glyph(&text);
        let line = shape_ui_runs(
            view,
            &[(head, glyph_color), (tail, label_color)],
            BAR_FONT_SIZE,
            window,
        );
        let width = line.width();
        let hovered = enabled && action.is_some() && hover_bar == Some(index);
        items.push(BarItem {
            text,
            line,
            x,
            action,
            hitbox: None,
            glyph_color,
            label_color,
            hovered,
        });
        x += width;
    }
    let width = x + px(BAR_PADDING);
    let origin = point(
        bounds.right() - px(BAR_MARGIN) - width,
        bounds.bottom() - px(BAR_MARGIN) - px(BAR_HEIGHT),
    );
    for item in &mut items {
        item.x += origin.x;
    }
    let bar_bounds = Bounds::new(origin, size(width, px(BAR_HEIGHT)));
    let glyph = glyph.map(|glyph| {
        (
            glyph,
            Bounds::new(
                point(
                    origin.x + px(BAR_PADDING),
                    origin.y + (px(BAR_HEIGHT) - px(ICON_SIZE)) / 2.,
                ),
                size(px(ICON_SIZE), px(ICON_SIZE)),
            ),
        )
    });
    Some(BarLayout {
        bounds: bar_bounds,
        glyph,
        items,
        enabled,
        surface_hitbox: None,
    })
}

/// Bounds of a clickable item of the floating bar (its hitbox and its hover
/// background).
fn bar_item_bounds(item_x: Pixels, width: Pixels, top: Pixels) -> Bounds<Pixels> {
    Bounds::new(
        point(item_x - px(4.), top),
        size(width + px(8.), px(BAR_HEIGHT)),
    )
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = EditorPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.started_at = Instant::now();
        // The element takes the width its parent gives it, never its
        // content's: a long line must not widen the editor (it scrolls or
        // wraps inside these bounds and is clipped to them in `paint`).
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.min_size.width = px(0.).into();
        style.overflow.x = gpui::Overflow::Hidden;
        let Some(auto) = self.view.read(cx).settings().auto_height else {
            style.size.height = relative(1.).into();
            return (window.request_layout(style, [], cx), ());
        };
        // Auto height (the chat composer): as many rows as the text wraps
        // into at the width the parent offers, clamped to `min..=max`. The
        // wrap computed here is the one prepaint then finds up to date.
        let view = self.view.clone();
        let min_rows = auto.min_rows.max(1);
        let max_rows = auto.max_rows.max(min_rows);
        let layout_id =
            window.request_measured_layout(style, move |known, available, window, cx| {
                let width = known.width.unwrap_or(match available.width {
                    AvailableSpace::Definite(width) => width,
                    _ => px(0.),
                });
                let (rows, line_height) = view.update(cx, |view, cx| {
                    view.sync_buffer(cx);
                    let metrics = gutter_metrics(view, window);
                    let text_width =
                        (width - metrics.gutter_width - px(SCROLLBAR_WIDTH)).max(px(1.));
                    view.ensure_wrap(text_width, metrics.char_width, window);
                    (view.wrap_row_count().max(1), view.style.line_height)
                });
                size(width, line_height * rows.clamp(min_rows, max_rows) as f32)
            });
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        let mut prepaint = self.view.update(cx, |view, cx| {
            view.sync_buffer(cx);

            let theme = *view.theme();
            let font = view.style.font.clone();
            let font_size = view.style.font_size;
            let line_height = view.style.line_height;
            let tab_size = view.settings().tab_size;
            let text_color = theme::color(theme.text);
            let muted_color = theme::color(theme.text_muted);
            let phantom_color = theme::alpha(theme.text, PHANTOM_TEXT_ALPHA);
            let turn_active = view.review.turn_active;

            // Gutter geometry, from the width of a digit. It never depends
            // on the review, so showing or deciding hunks cannot move the
            // text (the `+`/`−` icons go over the number column).
            let minimal = view.settings().chrome == EditorChrome::Minimal;
            let metrics = gutter_metrics(view, window);
            let char_width = metrics.char_width;
            let gutter_width = metrics.gutter_width;
            let text_origin_x = bounds.left() + gutter_width;
            let numbers_right = if minimal {
                text_origin_x
            } else {
                text_origin_x - px(GUTTER_TEXT_GAP)
            };
            let numbers_left = numbers_right - metrics.number_width;
            let text_width = (bounds.size.width - gutter_width - px(SCROLLBAR_WIDTH)).max(px(1.));

            // The git column: hunks resolved at this version (their anchors
            // followed every edit since the host computed them). No gutter,
            // no column.
            let git_hunks = if minimal {
                Vec::new()
            } else {
                view.git_gutter.resolve()
            };
            let git_colors = view.git_gutter.colors;
            let git_x = bounds.left() + px(GUTTER_PADDING_LEFT);

            // Soft wrap: monospace columns, or measured advances with a
            // prose font.
            view.ensure_wrap(text_width, char_width, window);

            // Comment blocks (spec 09 D10): whole rows below the text rows
            // they talk about. Built after the wrap, before anything that
            // needs a y, and the scroll follows the row on top when a block
            // above it changed.
            let wrap_rows = view.wrap_row_count();
            let box_width = view.comment_box_width(text_width);
            let placements = view.layout_comment_blocks(box_width, window, cx);
            view.update_block_map(wrap_rows, placements);
            let visual_rows = view.blocks.total_rows();

            let viewport_height = bounds.size.height;
            let visible_row_count = f32::from(viewport_height) / f32::from(line_height);
            // Spec 10 §6: the last row can scroll up to mid-screen (code
            // editor only); the scrollbar counts the same margin as content.
            let viewport_px = f32::from(viewport_height);
            let end_margin = view.end_margin(viewport_px);
            let max_scroll = view.max_scroll_top(viewport_px);

            // A scroll asked for before the first layout could not be clamped
            // then; do it now, before the cursor gets a say.
            if let Some(row) = view.pending_scroll_row.take() {
                view.scroll_top = row * f32::from(line_height);
            }

            // Autoscroll to the cursor if it moved off-screen; a jump to a
            // change centres it instead, unless it already sits in the middle
            // third (the clamp below keeps it as close as the document lets).
            if view.autoscroll {
                view.autoscroll = false;
                let center = std::mem::take(&mut view.autoscroll_center);
                let cursor_row = view.blocks.to_visual_row(view.cursor_wrap_row());
                let cursor_top = cursor_row as f32 * f32::from(line_height);
                let cursor_bottom = cursor_top + f32::from(line_height);
                let viewport = f32::from(viewport_height);
                if center {
                    let third = viewport / 3.;
                    let centred = cursor_top >= view.scroll_top + third
                        && cursor_bottom <= view.scroll_top + viewport - third;
                    if !centred {
                        view.scroll_top = cursor_top + f32::from(line_height) / 2. - viewport / 2.;
                    }
                } else if cursor_top < view.scroll_top {
                    view.scroll_top = cursor_top;
                } else if cursor_bottom > view.scroll_top + f32::from(viewport_height) {
                    view.scroll_top = cursor_bottom - f32::from(viewport_height);
                }
            }
            // A box just opened comes into view (its bottom first, without
            // pushing its top out).
            if let Some(range) = view
                .autoscroll_block
                .take()
                .and_then(|id| view.blocks.block_range(id))
            {
                let block_top = range.start as f32 * f32::from(line_height);
                let block_bottom = range.end as f32 * f32::from(line_height);
                let viewport = f32::from(viewport_height);
                if block_bottom > view.scroll_top + viewport {
                    view.scroll_top = block_bottom - viewport;
                }
                if block_top < view.scroll_top {
                    view.scroll_top = block_top;
                }
            }
            view.scroll_top = view.scroll_top.clamp(0., max_scroll);

            // One `ScrollChanged` per frame at most, whatever moved the scroll
            // (wheel, scrollbar, autoscroll, the host itself).
            let scroll_row = view.scroll_top / f32::from(line_height).max(1.);
            if (scroll_row - view.emitted_scroll_row).abs() > f32::EPSILON {
                view.emitted_scroll_row = scroll_row;
                cx.emit(crate::view::EditorEvent::ScrollChanged { row: scroll_row });
            }

            let first_row = ((view.scroll_top / f32::from(line_height)).floor() as u32)
                .saturating_sub(OVERSCAN);
            // `first_row..last_row` are *visual* rows (comment blocks
            // included); the text rows among them are the wrap rows below.
            let last_row =
                (first_row + visible_row_count.ceil() as u32 + 2 * OVERSCAN + 1).min(visual_rows);
            let top =
                bounds.top() - px(view.scroll_top - first_row as f32 * f32::from(line_height));
            let visible_wraps = view.blocks.wrap_rows_in(first_row..last_row);

            // Highlights of the visible byte range, queried once per frame.
            let (visible_start, visible_end) =
                view.visible_byte_range(visible_wraps.start, visible_wraps.end);
            let spans = view.highlights(visible_start..visible_end).to_vec();

            // Occurrences of the word under the cursor (spec 10 §7.1): looked
            // for in the same visible bytes, real rows only (phantom text is
            // not in the buffer).
            let occurrences: Vec<Range<usize>> = match view.live_occurrence_query().cloned() {
                Some(query) if visible_end > visible_start => crate::search::find_occurrences(
                    &view.buffer_text_in(visible_start..visible_end),
                    visible_start,
                    &query,
                ),
                _ => Vec::new(),
            };

            let selection = view.selection_range();
            let has_selection = view.has_selection();
            let cursor_display_row = view.cursor.row;
            // Real matches as buffer ranges; phantom ones by (hunk, line), so a
            // phantom row finds its own without scanning the buffer ones.
            let search_matches: Vec<Range<usize>> = view.search.buffer_matches();
            let mut phantom_matches: HashMap<(usize, usize), Vec<Range<usize>>> = HashMap::new();
            for found in view.search.matches() {
                if let MatchLocation::Phantom {
                    hunk_ix,
                    line_ix,
                    range,
                } = found
                {
                    phantom_matches
                        .entry((*hunk_ix, *line_ix))
                        .or_default()
                        .push(range.clone());
                }
            }
            let current_match = view.search.current();
            let marked_range = view.marked_range.clone();
            let show_whitespace = view.settings().show_whitespace;
            let scroll_left = px(view.scroll_left);
            let bracket_offsets = view.matching_brackets_cached();
            let deleted_word = theme::alpha(theme.diff_deleted, DIFF_WORD_ALPHA);
            let added_word = theme::alpha(theme.diff_added, DIFF_WORD_ALPHA);

            let mut rows = Vec::with_capacity(last_row.saturating_sub(first_row) as usize);
            let probe = view.render_probe();
            let mut probe_rows = Vec::new();
            let mut selections = Vec::new();
            let mut brackets = Vec::new();
            let mut matches = Vec::new();
            let mut occurrence_quads = Vec::new();
            let mut current_line = Vec::new();
            let mut marked = Vec::new();
            let mut cursor = None;
            let mut widest = px(0.);

            for visual_row in first_row..last_row {
                // A comment block paints itself (below); it has no text.
                let VisualCell::Wrap(wrap_row) = view.blocks.from_visual_row(visual_row) else {
                    continue;
                };
                if wrap_row >= wrap_rows {
                    break;
                }
                let origin_y = top + line_height * (visual_row - first_row) as f32;
                let (display_row, segment) = view.wrap.to_display(wrap_row);
                let cell = view.display_map.to_buffer(display_row);
                let kind = view.display_map.diff().row_kind(display_row);
                let row_text = view.display_row_text(display_row);
                let range = view
                    .wrap
                    .segment_range(display_row, segment, row_text.len());
                let indent_x = if segment > 0 {
                    view.wrap_indent_x(display_row, char_width)
                } else {
                    px(0.)
                };
                // The code font, or the prose font for a text field's prose
                // rows; and the host's full-width background, if any.
                let row_font = view.font_for_cell(cell);
                let background = match cell {
                    DisplayCell::Buffer(buffer_row) => {
                        view.row_background(buffer_row).map(theme::color)
                    }
                    DisplayCell::Phantom { .. } => None,
                };

                let (base, default_color, opacity) = match cell {
                    DisplayCell::Buffer(buffer_row) => {
                        (view.snapshot_line_start(buffer_row), text_color, 1.)
                    }
                    DisplayCell::Phantom { .. } => (0, phantom_color, PHANTOM_TEXT_ALPHA),
                };
                // The visible spans are queried once per frame and borrowed
                // here; only a phantom row owns its own (tiny) vector.
                let phantom_spans;
                let row_spans: &[(Range<usize>, HighlightId)] = match cell {
                    DisplayCell::Buffer(_) => &spans,
                    DisplayCell::Phantom { hunk_ix, line_ix } => {
                        phantom_spans = view.phantom_highlights(hunk_ix, line_ix);
                        &phantom_spans
                    }
                };

                let start_display = row_text.to_display(range.start) as usize;
                let end_display = row_text.to_display(range.end) as usize;
                let painted = row_text.text();
                let segment_text = painted
                    [start_display.min(painted.len())..end_display.min(painted.len())]
                    .to_string();

                let (runs, used_spans) = runs_for_segment(
                    row_spans,
                    base,
                    &range,
                    &row_text,
                    &row_font,
                    default_color,
                    &theme,
                    opacity,
                );
                let (line, reshaped) = view.shaped_line(&segment_text, &runs, font_size, window);
                widest = widest.max(line.width() + indent_x);

                let whitespace = if show_whitespace {
                    let source = row_text.source();
                    let slice = &source[(range.start as usize).min(source.len())
                        ..(range.end as usize).min(source.len())];
                    whitespace_overlay(slice, tab_size).map(|overlay| {
                        shape_cached(
                            view,
                            &overlay,
                            &row_font,
                            font_size,
                            theme::alpha(theme.text_muted, 0.6),
                            window,
                        )
                        .0
                    })
                } else {
                    None
                };

                // The line number only shows on the first segment of a row.
                let number = match cell {
                    DisplayCell::Buffer(buffer_row) if segment == 0 && !minimal => {
                        let color = if display_row == cursor_display_row {
                            text_color
                        } else {
                            muted_color
                        };
                        Some(
                            shape_cached(
                                view,
                                &(buffer_row + 1).to_string(),
                                &font,
                                font_size,
                                color,
                                window,
                            )
                            .0,
                        )
                    }
                    // Phantom rows carry no line number (02-visual §5).
                    _ => None,
                };

                // Word diffs, in row-local source bytes.
                let (word_ranges, word_color) = match (cell, kind) {
                    (DisplayCell::Phantom { hunk_ix, line_ix }, _) => (
                        view.deleted_words
                            .get(hunk_ix)
                            .and_then(|lines| lines.get(line_ix))
                            .cloned()
                            .unwrap_or_default(),
                        deleted_word,
                    ),
                    (DisplayCell::Buffer(_), RowKind::Added(hunk_ix)) => (
                        view.review
                            .hunks
                            .get(hunk_ix)
                            .map(|hunk| {
                                let first = view.snapshot_line_start(hunk.buffer_rows.start);
                                hunk.added_words_on_row(
                                    base.saturating_sub(first),
                                    row_text.len() as usize,
                                )
                            })
                            .unwrap_or_default(),
                        added_word,
                    ),
                    _ => (Vec::new(), added_word),
                };

                // The git bar covers every segment of a wrapped line; phantom
                // rows do not exist on disk and never get one.
                let git_bar = match cell {
                    DisplayCell::Buffer(buffer_row) => git_hunks
                        .iter()
                        .find(|hunk| {
                            hunk.kind != GitGutterKind::Deleted && hunk.rows.contains(&buffer_row)
                        })
                        .map(|hunk| (hunk.kind, theme::color(git_colors.color(hunk.kind)))),
                    DisplayCell::Phantom { .. } => None,
                };

                let mut layout = RowLayout {
                    wrap_row,
                    display_row,
                    segment,
                    kind,
                    line,
                    whitespace,
                    number,
                    origin_y,
                    indent_x,
                    source_range: range.clone(),
                    row_text,
                    words: Vec::new(),
                    background,
                    git_bar,
                };

                let row_bounds = |from: u32, to: u32| -> Bounds<Pixels> {
                    let start_x = text_origin_x + layout.x_for(from) - scroll_left;
                    let end_x = text_origin_x + layout.x_for(to) - scroll_left;
                    Bounds::from_corners(
                        point(start_x, origin_y),
                        point(end_x.max(start_x), origin_y + line_height),
                    )
                };

                let words: Vec<(Bounds<Pixels>, Hsla)> = word_ranges
                    .iter()
                    .filter_map(|word| {
                        let from = word.start.max(range.start);
                        let to = word.end.min(range.end);
                        (from < to).then(|| (row_bounds(from, to), word_color))
                    })
                    .collect();

                if display_row == cursor_display_row && !minimal {
                    current_line.push(Bounds::new(
                        point(bounds.left(), origin_y),
                        size(bounds.size.width, line_height),
                    ));
                }

                // Selection.
                if has_selection
                    && display_row >= selection.start.row
                    && display_row <= selection.end.row
                {
                    let from = if display_row == selection.start.row {
                        selection.start.column.max(range.start)
                    } else {
                        range.start
                    };
                    let to = if display_row == selection.end.row {
                        selection.end.column.min(range.end)
                    } else {
                        range.end
                    };
                    if from <= to && from < range.end && to > range.start {
                        let mut quad = row_bounds(from, to);
                        // A selection that crosses the end of the row shows the
                        // newline as half a character.
                        if display_row < selection.end.row && range.end == layout.row_text.len() {
                            quad.size.width += char_width / 2.;
                        }
                        selections.push(quad);
                    }
                }

                let matches_before = matches.len();
                // Search matches on a phantom row (07-etapa5 §10.2): searched and
                // highlighted like real ones, never replaced.
                if let DisplayCell::Phantom { hunk_ix, line_ix } = cell
                    && let Some(found) = phantom_matches.get(&(hunk_ix, line_ix))
                {
                    for found in found {
                        let from = (found.start as u32).max(range.start);
                        let to = (found.end as u32).min(range.end);
                        if from >= to {
                            continue;
                        }
                        let is_current = matches!(
                            &current_match,
                            Some(MatchLocation::Phantom { hunk_ix: h, line_ix: l, range: r })
                                if *h == hunk_ix && *l == line_ix && r == found
                        );
                        matches.push((row_bounds(from, to), is_current));
                    }
                }

                // Search matches, brackets and marked text, for real rows.
                if let DisplayCell::Buffer(_) = cell {
                    let row_start = base;
                    let row_end = base + layout.row_text.len() as usize;
                    // Matching brackets (ASCII, one byte each).
                    if let Some((first, second)) = bracket_offsets.filter(|_| !minimal) {
                        for offset in [first, second] {
                            if offset < row_start || offset >= row_end {
                                continue;
                            }
                            let column = (offset - row_start) as u32;
                            if column >= range.start && column < range.end {
                                brackets.push(row_bounds(column, column + 1));
                            }
                        }
                    }
                    for found in &search_matches {
                        if found.end <= row_start || found.start >= row_end.max(row_start) {
                            continue;
                        }
                        let from = (found.start.max(row_start) - row_start) as u32;
                        let to = (found.end.min(row_end) - row_start) as u32;
                        let from = from.max(range.start);
                        let to = to.min(range.end);
                        if from >= to {
                            continue;
                        }
                        let is_current = matches!(
                            &current_match,
                            Some(MatchLocation::Buffer(current)) if current == found
                        );
                        matches.push((row_bounds(from, to), is_current));
                    }
                    // Occurrences of the word under the cursor, clipped to the
                    // segment like the search matches.
                    let first = occurrences.partition_point(|found| found.end <= row_start);
                    for found in &occurrences[first..] {
                        if found.start >= row_end.max(row_start) {
                            break;
                        }
                        let from =
                            ((found.start.max(row_start) - row_start) as u32).max(range.start);
                        let to = ((found.end.min(row_end) - row_start) as u32).min(range.end);
                        if from < to {
                            occurrence_quads.push(row_bounds(from, to));
                        }
                    }
                    // Marked (IME) text.
                    if let Some(marked_range) = &marked_range
                        && marked_range.end > row_start
                        && marked_range.start < row_end.max(row_start)
                    {
                        let from = ((marked_range.start.max(row_start) - row_start) as u32)
                            .max(range.start);
                        let to =
                            ((marked_range.end.min(row_end) - row_start) as u32).min(range.end);
                        if from < to {
                            let quad = row_bounds(from, to);
                            marked.push(Bounds::new(
                                point(quad.left(), quad.bottom() - px(1.)),
                                size(quad.size.width, px(1.)),
                            ));
                        }
                    }
                }

                // Cursor.
                if display_row == cursor_display_row
                    && view.cursor.column >= range.start
                    && (view.cursor.column < range.end
                        || range.end == layout.row_text.len()
                        || segment + 1 == view.wrap.segment_count(display_row))
                {
                    let x = text_origin_x + layout.x_for(view.cursor.column) - scroll_left;
                    cursor = Some(Bounds::new(point(x, origin_y), size(px(2.), line_height)));
                }

                if probe {
                    probe_rows.push(RowRender {
                        display_row,
                        segment,
                        highlight_spans: used_spans,
                        reshaped,
                        kind,
                        line_number: layout.number.is_some(),
                        word_diffs: words.len(),
                        background: layout.background.is_some(),
                        monospace: row_font == view.style.font,
                        git_bar: layout.git_bar.map(|(kind, _)| kind),
                        search_matches: matches.len() - matches_before,
                        current_search_match: matches[matches_before..]
                            .iter()
                            .any(|(_, current)| *current),
                        y: origin_y,
                    });
                }
                layout.words = words;
                rows.push(layout);
            }
            view.occurrence_ranges = occurrences;

            // Horizontal scrolling only exists without soft wrap.
            let max_scroll_x = if view.soft_wrap() {
                0.
            } else {
                (f32::from(widest) - f32::from(text_width)).max(0.)
            };
            view.scroll_left = view.scroll_left.clamp(0., max_scroll_x);

            // The 2 px border of every visible hunk, in the kind's colour.
            // Rows are placed by their visual row: the comment blocks above
            // them push them down.
            let blocks = view.blocks.clone();
            let visual_y =
                |visual_row: u32| top + line_height * (visual_row as f32 - first_row as f32);
            let wrap_y = |wrap_row: WrapRow| visual_y(blocks.to_visual_row(wrap_row));

            // Deletion marks of the git column: 3 × 6 px centred on the
            // boundary between the rows the deleted lines sat between, plus
            // a 1 px line across the column. Pushed inside the real row when
            // the neighbour across the boundary is a phantom row (or there is
            // none: the top of the file, below its last row).
            let mut git_deleted = Vec::new();
            let buffer_rows = view.display_map.buffer_row_count();
            for hunk in git_hunks
                .iter()
                .filter(|hunk| hunk.kind == GitGutterKind::Deleted)
            {
                let row = hunk.rows.start;
                let (boundary, real_above, real_below) = if row < buffer_rows {
                    let display_row = view.display_map.to_display_row(row);
                    let above = display_row > 0
                        && matches!(
                            view.display_map.to_buffer(display_row - 1),
                            DisplayCell::Buffer(_)
                        );
                    (view.wrap.to_wrap_row(display_row, 0), above, true)
                } else if buffer_rows > 0 {
                    let display_row = view.display_map.to_display_row(buffer_rows - 1);
                    (
                        view.wrap.to_wrap_row(display_row, u32::MAX) + 1,
                        true,
                        false,
                    )
                } else {
                    continue;
                };
                let boundary_visual = if boundary > 0 && boundary >= wrap_rows {
                    blocks.to_visual_row(boundary - 1) + 1
                } else {
                    blocks.to_visual_row(boundary)
                };
                if boundary_visual < first_row || boundary_visual > last_row {
                    continue;
                }
                let y = visual_y(boundary_visual);
                let height = px(GIT_DELETED_MARK_HEIGHT);
                let mark_top = match (real_above, real_below) {
                    (true, true) => y - height / 2.,
                    (false, _) => y,
                    (true, false) => y - height,
                };
                let line_top = match (real_above, real_below) {
                    (false, _) => y,
                    (true, false) => y - px(1.),
                    (true, true) => y - px(0.5),
                };
                let color = theme::color(git_colors.deleted);
                git_deleted.push((
                    Bounds::new(point(git_x, mark_top), size(px(GIT_BAR_WIDTH), height)),
                    color,
                ));
                git_deleted.push((
                    Bounds::new(point(git_x, line_top), size(px(GIT_BAR_WIDTH), px(1.))),
                    color,
                ));
            }
            let border_x = text_origin_x - px(GUTTER_TEXT_GAP / 2. + HUNK_BORDER_WIDTH / 2.);
            let mut hunk_borders = Vec::new();
            for (hunk_ix, hunk) in view.review.hunks.iter().enumerate() {
                let range = view.display_map.diff().hunk_display_range(hunk_ix);
                if range.is_empty() {
                    continue;
                }
                let first = blocks.to_visual_row(view.wrap.to_wrap_row(range.start, 0));
                let last_display = range.end - 1;
                let last = blocks.to_visual_row(view.wrap.to_wrap_row(last_display, u32::MAX)) + 1;
                let from = first.max(first_row);
                let to = last.min(last_row);
                if from >= to {
                    continue;
                }
                let color = match hunk.kind {
                    ReviewHunkKind::Added => theme::color(theme.diff_added),
                    ReviewHunkKind::Deleted => theme::color(theme.diff_deleted),
                    ReviewHunkKind::Modified => theme::color(theme.diff_modified),
                };
                hunk_borders.push((
                    Bounds::new(
                        point(border_x, visual_y(from)),
                        size(px(HUNK_BORDER_WIDTH), line_height * (to - from) as f32),
                    ),
                    color,
                ));
            }

            // Pills: always on the hunk with the cursor, and on the one under
            // the mouse (02-visual §6.1), each on the first row that leaves it
            // room (module docs, "Where the pill goes"). "Under the mouse" is
            // the hunk's rows plus its own pill: a pill sticks out of its
            // rows (the row above, or its top edge), and the mouse reaching
            // it from above must not make it go away.
            let mut pill_hunks: Vec<usize> = Vec::new();
            if let Some(ix) = view.hunk_under_cursor() {
                pill_hunks.push(ix);
            }
            let hovered_hunks = [
                view.hover_row.and_then(|row| view.review_hunk_at_row(row)),
                view.hover_pill_zone
                    .and_then(|id| view.review.hunks.iter().position(|hunk| hunk.id == id)),
            ];
            for ix in hovered_hunks.into_iter().flatten() {
                if !pill_hunks.contains(&ix) {
                    pill_hunks.push(ix);
                }
            }
            pill_hunks.sort_unstable();
            let pill_right = bounds.right() - px(SCROLLBAR_WIDTH + PILL_MARGIN);
            let pill_left = pill_right - px(PILL_WIDTH * view.settings().scale());
            // Where the painted text of a display row's first segment ends,
            // from the lines just shaped (only laid-out rows can host a pill).
            let text_right = |display_row: DisplayRow| -> Option<(Pixels, Pixels)> {
                rows.iter()
                    .find(|row| row.display_row == display_row && row.segment == 0)
                    .map(|row| {
                        (
                            row.origin_y,
                            text_origin_x + row.indent_x + row.line.width() - scroll_left,
                        )
                    })
            };
            let mut pills = Vec::new();
            for hunk_ix in pill_hunks {
                let range = view.display_map.diff().hunk_display_range(hunk_ix);
                if range.is_empty() {
                    continue;
                }
                let mut candidate_rows: Vec<DisplayRow> = range.clone().collect();
                if range.start > 0 {
                    candidate_rows.push(range.start - 1);
                }
                let candidates: Vec<PillCandidate> = candidate_rows
                    .into_iter()
                    .filter_map(|display_row| {
                        let (row_y, text_right) = text_right(display_row)?;
                        Some(PillCandidate {
                            display_row,
                            row_y,
                            text_right,
                        })
                    })
                    .collect();
                let first = text_right(range.start).map(|(y, _)| (range.start, y));
                let Some((display_row, row_y, compact)) = place_pill(&candidates, first, pill_left)
                else {
                    continue;
                };
                pills.push(layout_pill(
                    view,
                    hunk_ix,
                    display_row,
                    row_y,
                    compact,
                    pill_right,
                    line_height,
                    &theme,
                    window,
                ));
            }

            // `+`/`−` on the hovered line of a hunk (not while the agent
            // writes: every review control is disabled then), over the
            // line-number column, whose number hides meanwhile.
            let line_icons = view
                .hover_row
                .filter(|_| !turn_active && !minimal)
                .and_then(|row| {
                    let (hunk_ix, line) = view.review_line_at_row(row)?;
                    let wrap_row = view.wrap.to_wrap_row(row, 0);
                    let visual_row = blocks.to_visual_row(wrap_row);
                    if visual_row < first_row || visual_row >= last_row {
                        return None;
                    }
                    let icon_size = line_icon_size(f32::from(numbers_right - numbers_left));
                    let y = wrap_y(wrap_row) + (line_height - px(icon_size)) / 2.;
                    let icon = size(px(icon_size), px(icon_size));
                    let reject_x = numbers_right - px(icon_size);
                    let accept_x = reject_x - px(LINE_ICON_GAP + icon_size);
                    Some(LineIconsLayout {
                        hunk: view.review.hunks[hunk_ix].id,
                        line,
                        display_row: row,
                        accept: Bounds::new(point(accept_x, y), icon),
                        reject: Bounds::new(point(reject_x, y), icon),
                        accept_hitbox: None,
                        reject_hitbox: None,
                    })
                });

            // The placeholder of an empty text field, in the prose font.
            let placeholder =
                view.placeholder
                    .clone()
                    .filter(|_| view.is_empty())
                    .map(|placeholder| {
                        let font = view
                            .style
                            .prose_font
                            .clone()
                            .unwrap_or_else(|| view.style.font.clone());
                        shape_cached(view, &placeholder, &font, font_size, muted_color, window).0
                    });

            // "Turno anterior" over the hovered clock.
            let tooltip = view.hover_clock.and_then(|hunk| {
                let pill = pills
                    .iter()
                    .find(|pill| pill.hunk == hunk && pill.previous_turn)?;
                let (_, glyph) = pill.glyph?;
                let line = shape_ui(view, "Turno anterior", PILL_FONT_SIZE, text_color, window);
                let width = line.width() + px(12.);
                let height = px(22.);
                let above = pill.bounds.top() - height - px(4.);
                let y = if above >= bounds.top() {
                    above
                } else {
                    pill.bounds.bottom() + px(4.)
                };
                let x = (glyph.left() - px(6.)).min(bounds.right() - width);
                Some(TooltipLayout {
                    bounds: Bounds::new(point(x, y), size(width, height)),
                    line,
                })
            });

            let bar = layout_bar(view, bounds, &theme, window);

            // Comment blocks in the viewport (only those: the ones above or
            // below are never built), each an element laid out in the strip
            // of its rows, from the text's left edge to 12 px before the
            // scrollbar. They are prepainted below, outside this update.
            let scale = view.settings().scale();
            let mut block_paints = Vec::new();
            let mut block_probe = Vec::new();
            for placement in blocks.blocks() {
                let Some(range) = blocks.block_range(placement.id) else {
                    continue;
                };
                if range.end <= first_row || range.start >= last_row {
                    continue;
                }
                let Some(layout) = view.block_layout(placement.id).cloned() else {
                    continue;
                };
                let strip = Bounds::new(
                    point(text_origin_x, visual_y(range.start)),
                    size(box_width, line_height * placement.rows as f32),
                );
                let element = view.render_comment_block(&layout, box_width, strip.size.height, cx);
                if probe {
                    block_probe.push(crate::view::CommentBlockRender {
                        id: placement.id,
                        kind: layout.kind,
                        visual_row: range.start,
                        rows: placement.rows,
                        bounds: strip,
                        box_height: layout.height,
                    });
                }
                block_paints.push(BlockPaint {
                    bounds: strip,
                    element,
                });
            }

            // The comment marks: one per row (the first comment of the row),
            // centred in the gap between the numbers and the text, so they
            // never widen the gutter nor touch the git and agent columns.
            let mut marks: Vec<MarkLayout> = Vec::new();
            let mut marked_rows: Vec<DisplayRow> = Vec::new();
            let mark_side = px(COMMENT_MARK_SIZE * scale);
            for layout in &view.block_layouts {
                let (Some(row), Some(_)) = (layout.mark_row, layout.comment) else {
                    continue;
                };
                if marked_rows.contains(&row) {
                    continue;
                }
                marked_rows.push(row);
                let visual = blocks.to_visual_row(view.wrap.to_wrap_row(row, 0));
                if visual < first_row || visual >= last_row {
                    continue;
                }
                let center_x = numbers_right + px(GUTTER_TEXT_GAP / 2.);
                let y = visual_y(visual) + (line_height - mark_side) / 2.;
                marks.push(MarkLayout {
                    id: layout.id,
                    bounds: Bounds::new(
                        point(center_x - mark_side / 2., y),
                        size(mark_side, mark_side),
                    ),
                    hitbox: None,
                });
            }
            let mark_probe: Vec<crate::view::CommentMarkRender> = if probe {
                marks
                    .iter()
                    .filter_map(|mark| {
                        let layout = view.block_layout(mark.id)?;
                        Some(crate::view::CommentMarkRender {
                            id: mark.id,
                            display_row: layout.mark_row?,
                            bounds: mark.bounds,
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            // The tooltip of the hovered mark: the first 200 characters.
            let mark_tooltip = view.hover_mark.and_then(|id| {
                let mark = marks.iter().find(|mark| mark.id == id)?;
                let text = view
                    .comments
                    .iter()
                    .find(|comment| comment.id == id)
                    .map(|comment| crate::comments::tooltip_text(&comment.text))?;
                let font_size = px(PILL_FONT_SIZE * scale);
                let tip_line = px(PILL_FONT_SIZE * 1.5 * scale);
                let padding = px(6. * scale);
                let max_width = px(360. * scale).min(bounds.size.width - px(16.));
                let ui_font = view.style.ui_font.clone();
                let lines: Vec<ShapedLine> = text
                    .split('\n')
                    .flat_map(|paragraph| {
                        wrap_words(paragraph, max_width - padding * 2., |piece| {
                            shape(piece.to_string(), &ui_font, font_size, text_color, window)
                        })
                    })
                    .take(12)
                    .collect();
                let width = lines
                    .iter()
                    .map(|line| line.width())
                    .fold(px(0.), |a, b| a.max(b))
                    + padding * 2.;
                let height = tip_line * lines.len().max(1) as f32 + padding;
                let below = mark.bounds.bottom() + px(4.);
                let y = if below + height <= bounds.bottom() {
                    below
                } else {
                    mark.bounds.top() - px(4.) - height
                };
                let x = mark
                    .bounds
                    .left()
                    .min(bounds.right() - width)
                    .max(bounds.left());
                Some((
                    Bounds::new(point(x, y), size(width, height)),
                    lines,
                    tip_line,
                    padding,
                ))
            });

            // Scrollbar: 8 px, only while there is something to scroll.
            let alpha = view.scrollbar_alpha();
            let scrollbar = (max_scroll > 0. && alpha > 0.).then(|| {
                let track = Bounds::new(
                    point(bounds.right() - px(SCROLLBAR_WIDTH), bounds.top()),
                    size(px(SCROLLBAR_WIDTH), bounds.size.height),
                );
                let visible = f32::from(viewport_height);
                let content = visual_rows as f32 * f32::from(line_height) + end_margin;
                let thumb_height = (visible / content * visible)
                    .max(SCROLLBAR_MIN_THUMB)
                    .min(visible);
                let travel = visible - thumb_height;
                let offset = if max_scroll > 0. {
                    view.scroll_top / max_scroll * travel
                } else {
                    0.
                };
                let thumb = Bounds::new(
                    point(track.left() + px(1.), track.top() + px(offset)),
                    size(px(SCROLLBAR_WIDTH - 2.), px(thumb_height)),
                );
                (track, thumb, alpha)
            });

            let ruler_x = view
                .settings()
                .ruler
                .map(|column| text_origin_x + char_width * column as f32 - scroll_left);

            view.layout = Some(LayoutSnapshot {
                bounds,
                text_origin_x,
                line_height,
                char_width,
                first_visual_row: first_row,
                rows: rows
                    .iter()
                    .map(|row| (row.wrap_row, row.display_row, row.segment, row.line.clone()))
                    .collect(),
                visible_row_count,
            });
            let review_frame = if probe {
                ReviewFrame {
                    pills: pills
                        .iter()
                        .map(|pill| PillRender {
                            hunk: pill.hunk,
                            enabled: pill.enabled,
                            previous_turn: pill.previous_turn,
                            display_row: pill.display_row,
                            compact: pill.compact,
                            opacity: pill.opacity,
                            bounds: pill.bounds,
                            check_color: pill.check_color,
                            cross_color: pill.cross_color,
                            label_color: pill.label_color,
                            hovered: pill.hovered,
                            accept_bounds: pill.accept,
                            reject_bounds: pill.reject,
                            comment_bounds: pill.comment,
                            comment_hovered: pill.comment_hovered,
                            comment_icon_color: pill.comment_icon_color,
                            comment_label_color: pill.comment_label_color,
                        })
                        .collect(),
                    line_icons: line_icons.as_ref().map(|icons| (icons.hunk, icons.line)),
                    line_icon_bounds: line_icons
                        .as_ref()
                        .map(|icons| (icons.accept, icons.reject)),
                    number_column: (numbers_left, numbers_right),
                    bar_buttons: bar
                        .as_ref()
                        .map(|bar| {
                            bar.items
                                .iter()
                                .filter(|item| item.action.is_some())
                                .map(|item| BarButtonRender {
                                    text: item.text.clone(),
                                    glyph_color: item.glyph_color,
                                    label_color: item.label_color,
                                    hovered: item.hovered,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    bar: bar.as_ref().map(|bar| {
                        bar.items
                            .iter()
                            .map(|item| item.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    }),
                    bar_enabled: bar.as_ref().is_some_and(|bar| bar.enabled),
                    tooltip: tooltip
                        .as_ref()
                        .map(|_| "Turno anterior".to_string())
                        .or_else(|| {
                            mark_tooltip.as_ref().and(view.hover_mark).and_then(|id| {
                                view.comments
                                    .iter()
                                    .find(|comment| comment.id == id)
                                    .map(|comment| crate::comments::tooltip_text(&comment.text))
                            })
                        }),
                    hunks: Vec::new(),
                    blocks: block_probe,
                    comment_marks: mark_probe,
                }
            } else {
                ReviewFrame::default()
            };
            // The hovered row's number gives way to the `+`/`−` icons.
            if let Some(icons) = &line_icons {
                for row in probe_rows
                    .iter_mut()
                    .filter(|row| row.display_row == icons.display_row && row.segment == 0)
                {
                    row.line_number = false;
                }
            }
            let placeholder_painted = placeholder.is_some();
            let git_marks = if probe {
                let bars = rows.iter().filter_map(|row| {
                    row.git_bar.map(|(kind, _)| {
                        (
                            kind,
                            Bounds::new(
                                point(git_x, row.origin_y),
                                size(px(GIT_BAR_WIDTH), line_height),
                            ),
                        )
                    })
                });
                // Every other entry of `git_deleted` is the 1 px line of the
                // mark before it.
                let marks = git_deleted
                    .iter()
                    .step_by(2)
                    .map(|(bounds, _)| (GitGutterKind::Deleted, *bounds));
                bars.chain(marks).collect()
            } else {
                Vec::new()
            };
            view.end_frame(
                probe_rows,
                review_frame,
                crate::view::FrameGeometry {
                    gutter_width,
                    text_origin_x,
                    bounds,
                    text_width,
                },
                placeholder_painted,
                git_marks,
            );

            EditorPrepaint {
                hitbox: hitbox.clone(),
                rows,
                pills,
                line_icons,
                bar,
                tooltip,
                hunk_borders,
                git_deleted,
                git_x,
                numbers_right,
                placeholder,
                cursor,
                marked,
                selections,
                brackets,
                matches,
                occurrence_quads,
                current_line,
                text_origin_x,
                gutter_width,
                line_height,
                scroll_left,
                ruler_x,
                max_scroll,
                max_scroll_x,
                scrollbar,
                theme,
                margin_hitboxes: Vec::new(),
                blocks: block_paints,
                marks,
                mark_tooltip,
            }
        });

        // The margins that are not text: the gutter (numbers, git column,
        // review icons) and the scrollbar's strip, whether or not the bar is
        // showing right now (the text never runs under it).
        let gutter = Bounds::new(
            bounds.origin,
            size(prepaint.gutter_width, bounds.size.height),
        );
        let scrollbar_strip = Bounds::new(
            point(bounds.right() - px(SCROLLBAR_WIDTH), bounds.top()),
            size(px(SCROLLBAR_WIDTH), bounds.size.height),
        );
        prepaint.margin_hitboxes = [gutter, scrollbar_strip]
            .into_iter()
            .filter(|margin| margin.size.width > px(0.))
            .map(|margin| window.insert_hitbox(margin, HitboxBehavior::Normal))
            .collect();

        // The comment blocks, laid out and prepainted as children of the
        // editor (their buttons and the field get hitboxes and dispatch
        // nodes under it), clipped to the editor. Before the review
        // controls, which float over everything.
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for block in &mut prepaint.blocks {
                block.element.prepaint_as_root(
                    block.bounds.origin,
                    size(
                        AvailableSpace::Definite(block.bounds.size.width),
                        AvailableSpace::Definite(block.bounds.size.height),
                    ),
                    window,
                    cx,
                );
            }
        });
        for mark in &mut prepaint.marks {
            mark.hitbox =
                Some(window.insert_hitbox(mark.bounds.dilate(px(2.)), HitboxBehavior::Normal));
        }

        // Real hitboxes for the review buttons, inserted on top of the editor.
        // Disabled controls get none, so a click on them does nothing;
        // "Comentar" always has one (it is not a decision).
        for pill in &mut prepaint.pills {
            pill.surface_hitbox = Some(window.insert_hitbox(pill.bounds, HitboxBehavior::Normal));
            if pill.enabled {
                pill.accept_hitbox =
                    Some(window.insert_hitbox(pill.accept, HitboxBehavior::Normal));
                pill.reject_hitbox =
                    Some(window.insert_hitbox(pill.reject, HitboxBehavior::Normal));
            }
            pill.comment_hitbox = Some(window.insert_hitbox(pill.comment, HitboxBehavior::Normal));
        }
        if let Some(icons) = prepaint.line_icons.as_mut() {
            icons.accept_hitbox = Some(window.insert_hitbox(icons.accept, HitboxBehavior::Normal));
            icons.reject_hitbox = Some(window.insert_hitbox(icons.reject, HitboxBehavior::Normal));
        }
        if let Some(bar) = prepaint.bar.as_mut() {
            bar.surface_hitbox = Some(window.insert_hitbox(bar.bounds, HitboxBehavior::Normal));
        }
        if let Some(bar) = prepaint.bar.as_mut()
            && bar.enabled
        {
            let top = bar.bounds.top();
            for item in bar.items.iter_mut().filter(|item| item.action.is_some()) {
                let item_bounds = bar_item_bounds(item.x, item.line.width(), top);
                item.hitbox = Some(window.insert_hitbox(item_bounds, HitboxBehavior::Normal));
            }
        }
        prepaint
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.view.read(cx).focus_handle.clone();
        // A *weak* handler: see `crate::input::WeakInputHandler` for why the
        // strong one GPUI ships leaks the view handle on exit.
        window.handle_input(
            &focus_handle,
            WeakInputHandler::new(bounds, self.view.downgrade()),
            cx,
        );

        let focused = focus_handle.is_focused(window);
        let blink_visible = self.view.read(cx).blink_visible;
        let theme = prepaint.theme;
        let line_height = prepaint.line_height;

        window.paint_quad(fill(bounds, theme::color(theme.background)));

        // Nothing the element paints may leave its bounds: with soft wrap
        // off a long line would otherwise run under whatever sits next to
        // the editor (the right dock).
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            if self.view.read(cx).render_probe() {
                let clip = window.content_mask().bounds;
                self.view.update(cx, |view, _| view.record_paint_clip(clip));
            }
            // Current line, below everything else.
            for quad in &prepaint.current_line {
                window.paint_quad(fill(
                    *quad,
                    theme::alpha(theme.elevated, CURRENT_LINE_ALPHA),
                ));
            }

            for row in &prepaint.rows {
                let row_bounds = Bounds::new(
                    point(bounds.left(), row.origin_y),
                    size(bounds.size.width, line_height),
                );
                // The host's full-width row background (a Markdown fence in
                // the chat composer).
                if let Some(background) = row.background {
                    window.paint_quad(fill(row_bounds, background));
                }
                // Diff backgrounds win over the current-line colour.
                match row.kind {
                    RowKind::Phantom(_) => window.paint_quad(fill(
                        row_bounds,
                        theme::alpha(theme.diff_deleted, DIFF_BG_ALPHA),
                    )),
                    RowKind::Added(_) => window.paint_quad(fill(
                        row_bounds,
                        theme::alpha(theme.diff_added, DIFF_BG_ALPHA),
                    )),
                    RowKind::Normal => {}
                }
                // Word diffs, over the row background and under the text.
                for (quad, color) in &row.words {
                    window.paint_quad(fill(*quad, *color));
                }

                // Git column, after the row background and before the
                // agent's bar, each in its own column.
                if let Some((_, color)) = row.git_bar {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(prepaint.git_x, row.origin_y),
                            size(px(GIT_BAR_WIDTH), line_height),
                        ),
                        color,
                    ));
                }

                // Gutter diff bar of the agent.
                if let Some(color) = match row.kind {
                    RowKind::Phantom(_) => Some(theme::color(theme.diff_deleted)),
                    RowKind::Added(_) => Some(theme::color(theme.diff_added)),
                    RowKind::Normal => None,
                } {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(
                                bounds.left()
                                    + px(GUTTER_PADDING_LEFT + GIT_BAR_WIDTH + GIT_BAR_GAP),
                                row.origin_y,
                            ),
                            size(px(GUTTER_BAR_WIDTH), line_height),
                        ),
                        color,
                    ));
                }
            }

            // Git deletion marks: over the row boundaries, so after every
            // row background.
            for (quad, color) in &prepaint.git_deleted {
                window.paint_quad(fill(*quad, *color));
            }

            // Per-hunk border of the gutter.
            for (quad, color) in &prepaint.hunk_borders {
                window.paint_quad(fill(*quad, *color));
            }

            // Ruler (02-visual §5).
            if let Some(x) = prepaint.ruler_x
                && x > prepaint.text_origin_x
                && x < bounds.right()
            {
                window.paint_quad(fill(
                    Bounds::new(point(x, bounds.top()), size(px(1.), bounds.size.height)),
                    theme::alpha(theme.text_muted, 0.25),
                ));
            }

            // Square corners, under the search matches and the selection.
            for occurrence in &prepaint.occurrence_quads {
                window.paint_quad(fill(
                    *occurrence,
                    theme::alpha(theme.selection, OCCURRENCE_ALPHA),
                ));
            }

            for (quad, is_current) in &prepaint.matches {
                let alpha = if *is_current {
                    SEARCH_CURRENT_ALPHA
                } else {
                    SEARCH_MATCH_ALPHA
                };
                window.paint_quad(fill(*quad, theme::alpha(theme.search_match, alpha)));
            }

            for selection in &prepaint.selections {
                window.paint_quad(fill(*selection, theme::color(theme.selection)));
            }

            // Matching brackets: a subtle `border` box (02-visual §4 surfaces).
            for bracket in &prepaint.brackets {
                window.paint_quad(quad(
                    *bracket,
                    Corners::all(px(2.)),
                    theme::alpha(theme.elevated, 0.8),
                    px(1.),
                    theme::color(theme.border),
                    BorderStyle::default(),
                ));
            }

            // The text is clipped to its column, so a horizontal scroll never
            // paints code over the gutter.
            let text_mask = Bounds::from_corners(
                point(prepaint.text_origin_x - px(2.), bounds.top()),
                bounds.bottom_right(),
            );
            window.with_content_mask(Some(gpui::ContentMask { bounds: text_mask }), |window| {
                for row in &prepaint.rows {
                    let origin = point(
                        prepaint.text_origin_x + row.indent_x - prepaint.scroll_left,
                        row.origin_y,
                    );
                    row.line
                        .paint(origin, line_height, TextAlign::Left, None, window, cx)
                        .ok();
                    if let Some(overlay) = &row.whitespace {
                        overlay
                            .paint(origin, line_height, TextAlign::Left, None, window, cx)
                            .ok();
                    }
                }
            });
            // The placeholder of an empty text field.
            if let Some(placeholder) = &prepaint.placeholder
                && let Some(first) = prepaint.rows.first()
            {
                placeholder
                    .paint(
                        point(prepaint.text_origin_x, first.origin_y),
                        line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
            }
            let icons_row = prepaint.line_icons.as_ref().map(|icons| icons.display_row);
            for row in &prepaint.rows {
                // The `+`/`−` icons replace the number of the hovered row.
                if icons_row == Some(row.display_row) {
                    continue;
                }
                // Line number, right aligned against the number column.
                if let Some(number) = &row.number {
                    let x = prepaint.numbers_right - number.width();
                    number
                        .paint(
                            point(x, row.origin_y),
                            line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        )
                        .ok();
                }
            }

            // IME composition underline.
            for quad in &prepaint.marked {
                window.paint_quad(fill(*quad, theme::color(theme.text_accent)));
            }

            if focused
                && blink_visible
                && let Some(cursor) = prepaint.cursor
            {
                window.paint_quad(fill(cursor, theme::color(theme.cursor)));
            }

            // The comment blocks, over the rows they own and under every
            // floating control.
            for block in &mut prepaint.blocks {
                block.element.paint(window, cx);
            }

            // `+`/`−` of the hovered line.
            if let Some(icons) = &prepaint.line_icons {
                paint_line_icon(
                    icons.accept,
                    true,
                    theme::color(theme.diff_added),
                    &theme,
                    window,
                );
                paint_line_icon(
                    icons.reject,
                    false,
                    theme::color(theme.diff_deleted),
                    &theme,
                    window,
                );
            }

            // Floating pills, on top of the text.
            for pill in &prepaint.pills {
                paint_surface(pill.bounds, 6., pill.opacity, &theme, window);
                // `bg.surface` behind the half under the mouse.
                if let Some(accept) = pill.hovered {
                    let half = if accept { pill.accept } else { pill.reject };
                    let mut hover = fill(
                        Bounds::from_corners(
                            point(
                                half.left().max(pill.bounds.left() + px(2.)),
                                half.top() + px(2.),
                            ),
                            point(
                                half.right().min(pill.bounds.right() - px(2.)),
                                half.bottom() - px(2.),
                            ),
                        ),
                        theme::alpha(theme.surface, pill.opacity),
                    );
                    hover.corner_radii = Corners::all(px(4.));
                    window.paint_quad(hover);
                }
                let text_y = pill.bounds.top() + (pill.bounds.size.height - line_height) / 2.;
                if let Some((glyph, glyph_bounds)) = pill.glyph {
                    paint_glyph(
                        glyph,
                        glyph_bounds,
                        theme::alpha(theme.text_muted, pill.opacity),
                        window,
                    );
                }
                // The compact pill's glyphs were shaped at its opacity.
                let paint_label =
                    |line: &ShapedLine, x: Pixels, window: &mut Window, cx: &mut App| {
                        line.paint(
                            point(x, text_y),
                            line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        )
                        .ok();
                    };
                paint_label(&pill.accept_line, pill.accept_x, window, cx);
                for separator_x in &pill.separators {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(*separator_x, pill.bounds.top() + px(5.)),
                            size(px(1.), pill.bounds.size.height - px(10.)),
                        ),
                        theme::alpha(theme.text_muted, 0.5),
                    ));
                }
                paint_label(&pill.reject_line, pill.reject_x, window, cx);
                // "Comentar": the icon in `text.accent`, the word in `text`.
                if pill.comment_hovered {
                    let mut hover = fill(
                        Bounds::from_corners(
                            point(
                                pill.comment.left().max(pill.bounds.left() + px(2.)),
                                pill.comment.top() + px(2.),
                            ),
                            point(
                                pill.comment.right().min(pill.bounds.right() - px(2.)),
                                pill.comment.bottom() - px(2.),
                            ),
                        ),
                        theme::alpha(theme.surface, pill.opacity),
                    );
                    hover.corner_radii = Corners::all(px(4.));
                    window.paint_quad(hover);
                }
                window
                    .paint_svg(
                        pill.comment_icon,
                        gpui_kit::assets::IconName::MessageSquarePlus.path(),
                        None,
                        gpui::TransformationMatrix::unit(),
                        pill.comment_icon_color,
                        cx,
                    )
                    .ok();
                if let Some(line) = &pill.comment_line {
                    paint_label(line, pill.comment_x, window, cx);
                }
            }

            // The comment marks of the margin.
            for mark in &prepaint.marks {
                window
                    .paint_svg(
                        mark.bounds,
                        gpui_kit::assets::IconName::MessageSquareText.path(),
                        None,
                        gpui::TransformationMatrix::unit(),
                        theme::color(theme.text_accent),
                        cx,
                    )
                    .ok();
            }
            if let Some((tip_bounds, lines, tip_line, padding)) = &prepaint.mark_tooltip {
                let (tip_line, padding) = (*tip_line, *padding);
                paint_surface(*tip_bounds, 4., 1., &theme, window);
                for (ix, line) in lines.iter().enumerate() {
                    line.paint(
                        point(
                            tip_bounds.left() + padding,
                            tip_bounds.top() + padding / 2. + tip_line * ix as f32,
                        ),
                        tip_line,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
                }
            }

            if let Some(tooltip) = &prepaint.tooltip {
                paint_surface(tooltip.bounds, 4., 1., &theme, window);
                tooltip
                    .line
                    .paint(
                        point(
                            tooltip.bounds.left() + px(6.),
                            tooltip.bounds.top() + (tooltip.bounds.size.height - line_height) / 2.,
                        ),
                        line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
            }

            // Floating review bar, bottom right.
            if let Some(bar) = &prepaint.bar {
                paint_surface(bar.bounds, 6., 1., &theme, window);
                if let Some((glyph, glyph_bounds)) = bar.glyph {
                    paint_glyph(glyph, glyph_bounds, theme::color(theme.text_accent), window);
                }
                let text_y = bar.bounds.top() + (px(BAR_HEIGHT) - line_height) / 2.;
                for item in bar.items.iter().filter(|item| item.hovered) {
                    let item_bounds = bar_item_bounds(item.x, item.line.width(), bar.bounds.top());
                    let mut hover = fill(
                        Bounds::from_corners(
                            point(item_bounds.left(), item_bounds.top() + px(3.)),
                            point(item_bounds.right(), item_bounds.bottom() - px(3.)),
                        ),
                        theme::color(theme.surface),
                    );
                    hover.corner_radii = Corners::all(px(4.));
                    window.paint_quad(hover);
                }
                for item in &bar.items {
                    item.line
                        .paint(
                            point(item.x, text_y),
                            line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        )
                        .ok();
                }
            }

            // Scrollbar.
            if let Some((track, thumb, alpha)) = prepaint.scrollbar {
                window.paint_quad(fill(track, theme::alpha(theme.background, alpha * 0.4)));
                let mut quad = fill(thumb, theme::alpha(theme.text_muted, alpha * 0.8));
                quad.corner_radii = Corners::all(px(3.));
                window.paint_quad(quad);
            }
        });

        // A fading scrollbar needs frames while it fades.
        if prepaint.scrollbar.is_some_and(|(_, _, alpha)| alpha < 1.) {
            window.request_animation_frame();
        }

        // Mouse cursors of the review controls: the pointing hand on what
        // reacts, the arrow on the rest of a pill or of the bar and on what is
        // disabled — never the text's I-beam. Later requests win, so the
        // surfaces go first and their buttons after. The margins that are
        // not text (gutter, scrollbar) go before everything: the arrow there,
        // and the gutter's `+`/`−` still get their hand on top.
        let mut control_cursors: Vec<(Bounds<Pixels>, CursorStyle)> = Vec::new();
        {
            let mut set = |hitbox: &Hitbox, style: CursorStyle, window: &mut Window| {
                window.set_cursor_style(style, hitbox);
                control_cursors.push((hitbox.bounds, style));
            };
            for hitbox in &prepaint.margin_hitboxes {
                set(hitbox, CursorStyle::Arrow, window);
            }
            for pill in &prepaint.pills {
                if let Some(hitbox) = &pill.surface_hitbox {
                    set(hitbox, CursorStyle::Arrow, window);
                }
                for hitbox in [
                    &pill.accept_hitbox,
                    &pill.reject_hitbox,
                    &pill.comment_hitbox,
                ]
                .into_iter()
                .flatten()
                {
                    set(hitbox, CursorStyle::PointingHand, window);
                }
            }
            for mark in &prepaint.marks {
                if let Some(hitbox) = &mark.hitbox {
                    set(hitbox, CursorStyle::PointingHand, window);
                }
            }
            if let Some(icons) = &prepaint.line_icons {
                for hitbox in [&icons.accept_hitbox, &icons.reject_hitbox]
                    .into_iter()
                    .flatten()
                {
                    set(hitbox, CursorStyle::PointingHand, window);
                }
            }
            if let Some(bar) = &prepaint.bar {
                if let Some(hitbox) = &bar.surface_hitbox {
                    set(hitbox, CursorStyle::Arrow, window);
                }
                for item in &bar.items {
                    if let Some(hitbox) = &item.hitbox {
                        set(hitbox, CursorStyle::PointingHand, window);
                    }
                }
            }
        }
        self.view
            .update(cx, |view, _| view.control_cursors = control_cursors);

        // Mouse handling.
        struct PillTarget {
            hunk: u64,
            bounds: Bounds<Pixels>,
            accept: Option<Hitbox>,
            reject: Option<Hitbox>,
            comment: Option<Hitbox>,
        }
        let pill_targets: Vec<PillTarget> = prepaint
            .pills
            .iter()
            .map(|pill| PillTarget {
                hunk: pill.hunk,
                bounds: pill.bounds,
                accept: pill.accept_hitbox.clone(),
                reject: pill.reject_hitbox.clone(),
                comment: pill.comment_hitbox.clone(),
            })
            .collect();
        // "Comentar" of each pill, for its hover background.
        let pill_comments: Vec<(u64, Bounds<Pixels>)> = prepaint
            .pills
            .iter()
            .map(|pill| (pill.hunk, pill.comment))
            .collect();
        // The comment marks: a click folds or unfolds, the mouse shows the
        // tooltip.
        let mark_targets: Vec<(u64, Bounds<Pixels>, Option<Hitbox>)> = prepaint
            .marks
            .iter()
            .map(|mark| (mark.id, mark.bounds.dilate(px(2.)), mark.hitbox.clone()))
            .collect();
        let mark_zones: Vec<(u64, Bounds<Pixels>)> = mark_targets
            .iter()
            .map(|(id, bounds, _)| (*id, *bounds))
            .collect();
        // What the mouse can hover for a `bg.surface` highlight: the halves
        // of the enabled pills and the enabled buttons of the bar.
        let pill_halves: Vec<(u64, bool, Bounds<Pixels>)> = prepaint
            .pills
            .iter()
            .filter(|pill| pill.enabled)
            .flat_map(|pill| {
                [
                    (pill.hunk, true, pill.accept),
                    (pill.hunk, false, pill.reject),
                ]
            })
            .collect();
        let bar_buttons: Vec<(usize, Bounds<Pixels>)> = prepaint
            .bar
            .as_ref()
            .filter(|bar| bar.enabled)
            .map(|bar| {
                bar.items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.action.is_some())
                    .map(|(index, item)| {
                        (
                            index,
                            bar_item_bounds(item.x, item.line.width(), bar.bounds.top()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        // The whole painted pill of each hunk: while the mouse is on it, the
        // pill stays (`hover_pill_zone`).
        let pill_zones: Vec<(u64, Bounds<Pixels>)> = prepaint
            .pills
            .iter()
            .map(|pill| (pill.hunk, pill.bounds))
            .collect();
        let clocks: Vec<(u64, Bounds<Pixels>)> = prepaint
            .pills
            .iter()
            .filter(|pill| pill.previous_turn)
            .filter_map(|pill| {
                pill.glyph
                    .map(|(_, glyph)| (pill.hunk, glyph.dilate(px(3.))))
            })
            .collect();
        let icon_targets = prepaint.line_icons.as_ref().map(|icons| {
            (
                icons.hunk,
                icons.line,
                icons.accept_hitbox.clone(),
                icons.reject_hitbox.clone(),
            )
        });
        let bar_targets = prepaint.bar.as_ref().map(|bar| {
            let buttons: Vec<(ReviewAction, Hitbox)> = bar
                .items
                .iter()
                .filter_map(|item| Some((item.action?, item.hitbox.clone()?)))
                .collect();
            (bar.bounds, buttons)
        });
        let hitbox = prepaint.hitbox.clone();
        let scrollbar = prepaint.scrollbar.map(|(track, thumb, _)| (track, thumb));

        window.on_mouse_event({
            let view = self.view.downgrade();
            let hitbox = hitbox.clone();
            let focus_handle = focus_handle.clone();
            let max_scroll = prepaint.max_scroll;
            move |event: &MouseDownEvent, phase: DispatchPhase, window: &mut Window, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                // The review controls first: they sit on top of the text, and a
                // click on them gives the focus back to the editor without
                // moving the cursor (editor.md, review rules).
                let clicked = |target: &Option<Hitbox>, window: &Window| {
                    target
                        .as_ref()
                        .is_some_and(|hitbox| hitbox.is_hovered_at(event.position, window))
                };
                let mut action = None;
                let mut swallowed = false;
                let mut comment_hunk = None;
                let mut toggle_mark = None;
                for pill in &pill_targets {
                    if pill.bounds.contains(&event.position) {
                        swallowed = true;
                        if clicked(&pill.accept, window) {
                            action = Some(ReviewAction::AcceptHunk(pill.hunk));
                        } else if clicked(&pill.reject, window) {
                            action = Some(ReviewAction::RejectHunk(pill.hunk));
                        } else if clicked(&pill.comment, window) {
                            comment_hunk = Some(pill.hunk);
                        }
                        break;
                    }
                }
                if !swallowed
                    && let Some((id, _, _)) = mark_targets
                        .iter()
                        .find(|(_, _, hitbox)| clicked(hitbox, window))
                {
                    swallowed = true;
                    toggle_mark = Some(*id);
                }
                if !swallowed && let Some((hunk, line, accept, reject)) = &icon_targets {
                    if clicked(accept, window) {
                        swallowed = true;
                        action = Some(ReviewAction::AcceptLine {
                            hunk: *hunk,
                            line: *line,
                        });
                    } else if clicked(reject, window) {
                        swallowed = true;
                        action = Some(ReviewAction::RejectLine {
                            hunk: *hunk,
                            line: *line,
                        });
                    }
                }
                if !swallowed
                    && let Some((bar_bounds, buttons)) = &bar_targets
                    && bar_bounds.contains(&event.position)
                {
                    swallowed = true;
                    action = buttons
                        .iter()
                        .find(|(_, hitbox)| hitbox.is_hovered_at(event.position, window))
                        .map(|(action, _)| *action);
                }
                if swallowed {
                    window.focus(&focus_handle, cx);
                    if let Some(hunk) = comment_hunk {
                        // The box takes the keyboard (after the focus above),
                        // and the editor's own focus-on-click must not take
                        // it back.
                        view.update(cx, |view, cx| view.comment_hunk(hunk, window, cx))
                            .ok();
                        window.prevent_default();
                    }
                    if let Some(id) = toggle_mark {
                        view.update(cx, |view, cx| view.toggle_comment(id, cx)).ok();
                    }
                    if let Some(action) = action {
                        // The bar's arrows move like the keys do.
                        view.update(cx, |view, cx| match action {
                            ReviewAction::NextHunk => view.next_hunk(cx),
                            ReviewAction::PrevHunk => view.prev_hunk(cx),
                            _ => {
                                view.emit_review(action, cx);
                            }
                        })
                        .ok();
                    }
                    return;
                }
                if let Some((track, thumb)) = scrollbar
                    && track.contains(&event.position)
                {
                    window.focus(&focus_handle, cx);
                    let _ = view.update(cx, |view, cx| {
                        if thumb.contains(&event.position) {
                            view.scrollbar_drag = Some(f32::from(event.position.y - thumb.top()));
                        } else {
                            // Jump: centre the thumb on the click.
                            let travel = (f32::from(track.size.height - thumb.size.height)).max(1.);
                            let offset =
                                f32::from(event.position.y - track.top() - thumb.size.height / 2.);
                            view.scroll_top = (offset / travel * max_scroll).clamp(0., max_scroll);
                            view.scrollbar_drag = Some(f32::from(thumb.size.height) / 2.);
                        }
                        cx.notify();
                    });
                    return;
                }
                if !hitbox.is_hovered_at(event.position, window) {
                    return;
                }
                window.focus(&focus_handle, cx);
                let _ = view.update(cx, |view, cx| {
                    let point = view.point_for_position(event.position, window);
                    view.begin_selection(point, event.modifiers.shift, event.click_count, cx);
                });
            }
        });

        // The right button (D12): over the text, a click outside the selection
        // moves the cursor there first (inside it, it stays) and the context
        // menu (gpui-kit's `PopupMenu`) opens there; over a comment box, a
        // pill, the bar or the margin, nothing opens.
        let overlay_zones: Vec<Bounds<Pixels>> = prepaint
            .pills
            .iter()
            .map(|pill| pill.bounds)
            .chain(prepaint.bar.as_ref().map(|bar| bar.bounds))
            .chain(prepaint.margin_hitboxes.iter().map(|hitbox| hitbox.bounds))
            .collect();
        window.on_mouse_event({
            let view = self.view.downgrade();
            let hitbox = hitbox.clone();
            let focus_handle = focus_handle.clone();
            move |event: &MouseDownEvent, phase: DispatchPhase, window: &mut Window, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Right {
                    return;
                }
                let on_text = hitbox.is_hovered_at(event.position, window)
                    && !overlay_zones
                        .iter()
                        .any(|zone| zone.contains(&event.position));
                // A text field (the chat composer) has no context menu.
                let enabled = view
                    .read_with(cx, |view, _| view.comments_enabled())
                    .unwrap_or(false);
                if !on_text || !enabled {
                    return;
                }
                window.focus(&focus_handle, cx);
                let _ = view.update(cx, |view, cx| {
                    let point = view.point_for_position(event.position, window);
                    let selection = view.selection_range();
                    let inside =
                        view.has_selection() && selection.start <= point && point <= selection.end;
                    if !inside {
                        view.begin_selection(point, false, 1, cx);
                        view.end_selection();
                    }
                    view.open_context_menu(event.position, window, cx);
                });
                // The menu has the keyboard now; the editor's own
                // focus-on-click must not take it back.
                window.prevent_default();
            }
        });

        window.on_mouse_event({
            let view = self.view.downgrade();
            let max_scroll = prepaint.max_scroll;
            move |event: &MouseMoveEvent, phase: DispatchPhase, window: &mut Window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let _ = view.update(cx, |view, cx| {
                    view.note_mouse_moved(cx);
                    // What the mouse is over, for the pill and the line icons.
                    let hover_row = view.display_row_at(event.position);
                    let hover_clock = clocks
                        .iter()
                        .find(|(_, bounds)| bounds.contains(&event.position))
                        .map(|(hunk, _)| *hunk);
                    let hover_pill = pill_halves
                        .iter()
                        .find(|(_, _, bounds)| bounds.contains(&event.position))
                        .map(|(hunk, accept, _)| (*hunk, *accept));
                    let hover_bar = bar_buttons
                        .iter()
                        .find(|(_, bounds)| bounds.contains(&event.position))
                        .map(|(index, _)| *index);
                    let hover_pill_zone = pill_zones
                        .iter()
                        .find(|(_, bounds)| bounds.contains(&event.position))
                        .map(|(hunk, _)| *hunk);
                    let hover_pill_comment = pill_comments
                        .iter()
                        .find(|(_, bounds)| bounds.contains(&event.position))
                        .map(|(hunk, _)| *hunk);
                    let hover_mark = mark_zones
                        .iter()
                        .find(|(_, bounds)| bounds.contains(&event.position))
                        .map(|(id, _)| *id);
                    if hover_row != view.hover_row
                        || hover_clock != view.hover_clock
                        || hover_pill != view.hover_pill
                        || hover_pill_zone != view.hover_pill_zone
                        || hover_bar != view.hover_bar
                        || hover_pill_comment != view.hover_pill_comment
                        || hover_mark != view.hover_mark
                    {
                        view.hover_row = hover_row;
                        view.hover_clock = hover_clock;
                        view.hover_pill = hover_pill;
                        view.hover_pill_zone = hover_pill_zone;
                        view.hover_bar = hover_bar;
                        view.hover_pill_comment = hover_pill_comment;
                        view.hover_mark = hover_mark;
                        cx.notify();
                    }
                    if let (Some(grab), Some((track, thumb))) = (view.scrollbar_drag, scrollbar) {
                        let travel = f32::from(track.size.height - thumb.size.height).max(1.);
                        let offset = f32::from(event.position.y - track.top()) - grab;
                        view.scroll_top = (offset / travel * max_scroll).clamp(0., max_scroll);
                        cx.notify();
                        return;
                    }
                    if view.selecting && event.dragging() {
                        let point = view.point_for_position(event.position, window);
                        view.update_selection(point, cx);
                    }
                });
            }
        });

        window.on_mouse_event({
            let view = self.view.downgrade();
            move |_: &MouseUpEvent, phase: DispatchPhase, _window: &mut Window, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = view.update(cx, |view, _cx| {
                        view.end_selection();
                        view.scrollbar_drag = None;
                    });
                }
            }
        });

        window.on_mouse_event({
            let view = self.view.downgrade();
            let hitbox = hitbox.clone();
            let max_scroll = prepaint.max_scroll;
            let max_scroll_x = prepaint.max_scroll_x;
            move |event: &ScrollWheelEvent, phase: DispatchPhase, window: &mut Window, cx| {
                if phase != DispatchPhase::Bubble || !hitbox.should_handle_scroll(window) {
                    return;
                }
                // 3 rows per wheel notch; a trackpad reports pixels and keeps
                // them (02-visual §5).
                let delta = event.delta.pixel_delta(line_height * 3.);
                let _ = view.update(cx, |view, cx| {
                    view.scroll_by(delta, max_scroll, max_scroll_x, cx)
                });
            }
        });

        let elapsed = self.started_at.elapsed();
        self.view.update(cx, |view, _cx| {
            view.record_frame(elapsed.as_micros() as u64);
        });
        tracing::debug!(
            target: "cincel_editor::frame",
            rows = prepaint.rows.len(),
            gutter_width = f32::from(prepaint.gutter_width),
            frame_us = elapsed.as_micros() as u64,
            "frame painted"
        );
    }
}
