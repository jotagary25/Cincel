//! `EditorElement`: the GPUI element that lays out and paints the editor.
//!
//! Only the wrap rows inside the viewport (± one row) are shaped and painted.
//! The shaped lines live in a content-addressed cache in the view (see
//! `EditorView::shaped_line`): a row is shaped again only when its painted text
//! or one of its colours actually changed, so a keystroke reshapes the row it
//! landed on and nothing else.

use std::ops::Range;
use std::time::Instant;

use asteroid_syntax::HighlightId;
use gpui::{
    App, Bounds, BoxShadow, Corners, DispatchPhase, Element, ElementId, Entity, GlobalElementId,
    Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollWheelEvent, ShapedLine,
    SharedString, Style, TextAlign, TextRun, Window, fill, hsla, point, px, relative, size,
};

use crate::display_map::{DisplayCell, DisplayRow, RowKind, RowText};
use crate::input::WeakInputHandler;
use crate::theme::{
    self, CURRENT_LINE_ALPHA, DIFF_BG_ALPHA, EditorTheme, PHANTOM_TEXT_ALPHA, SEARCH_CURRENT_ALPHA,
    SEARCH_MATCH_ALPHA,
};
use crate::view::{EditorView, LayoutSnapshot, RowRender};
use crate::wrap_map::WrapRow;

/// Left padding of the gutter, before the diff bar.
const GUTTER_PADDING_LEFT: f32 = 4.;
/// Width of the per-row diff bar in the gutter.
const GUTTER_BAR_WIDTH: f32 = 3.;
/// Gap between the diff bar and the line numbers.
const GUTTER_BAR_GAP: f32 = 6.;
/// Gap between the line numbers and the text.
const GUTTER_TEXT_GAP: f32 = 12.;
/// Height of the accept/reject pill.
const PILL_HEIGHT: f32 = 24.;
/// Width of the accept/reject pill.
const PILL_WIDTH: f32 = 190.;
/// Margin between the pill and the right edge of the editor.
const PILL_MARGIN: f32 = 12.;
/// Width of the scrollbar (02-visual §5).
const SCROLLBAR_WIDTH: f32 = 8.;
/// Shortest the scrollbar thumb can get.
const SCROLLBAR_MIN_THUMB: f32 = 24.;
/// Extra wrap rows shaped above and below the viewport.
const OVERSCAN: u32 = 1;

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

/// An accept/reject pill ready to paint.
struct PillLayout {
    hunk_ix: usize,
    bounds: Bounds<Pixels>,
    accept: Bounds<Pixels>,
    reject: Bounds<Pixels>,
    accept_line: ShapedLine,
    reject_line: ShapedLine,
    accept_hitbox: Hitbox,
    reject_hitbox: Hitbox,
}

/// State computed during `prepaint`.
pub struct EditorPrepaint {
    hitbox: Hitbox,
    rows: Vec<RowLayout>,
    pills: Vec<PillLayout>,
    cursor: Option<Bounds<Pixels>>,
    marked: Vec<Bounds<Pixels>>,
    selections: Vec<Bounds<Pixels>>,
    matches: Vec<(Bounds<Pixels>, bool)>,
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
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
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
        let mut pill_hitboxes = Vec::new();

        let prepaint = self.view.update(cx, |view, cx| {
            view.sync_buffer(cx);

            let theme = *view.theme();
            let font = view.style.font.clone();
            let font_size = view.style.font_size;
            let line_height = view.style.line_height;
            let tab_size = view.settings().tab_size;
            let text_color = theme::color(theme.text);
            let muted_color = theme::color(theme.text_muted);
            let phantom_color = theme::alpha(theme.text, PHANTOM_TEXT_ALPHA);

            // Gutter geometry, from the width of a digit.
            let char_width = shape("0".to_string(), &font, font_size, muted_color, window).width();
            let digits = view.display_map.buffer_row_count().max(1).to_string().len() as f32;
            let gutter_width = px(GUTTER_PADDING_LEFT + GUTTER_BAR_WIDTH + GUTTER_BAR_GAP)
                + char_width * digits
                + px(GUTTER_TEXT_GAP);
            let text_origin_x = bounds.left() + gutter_width;
            let text_width = (bounds.size.width - gutter_width - px(SCROLLBAR_WIDTH)).max(px(1.));

            // Soft wrap width in columns.
            let wrap_columns = view.soft_wrap().then(|| {
                ((f32::from(text_width) / f32::from(char_width).max(1.)).floor() as u32).max(1)
            });
            // The wrap map has its own epoch: an edit that stayed inside one
            // row already updated it, so only a real change (the viewport
            // width, the tab size, a row added or removed) rebuilds it.
            if view.wrap.columns() != wrap_columns || view.wrap_epoch != view.wrap_state() {
                view.rebuild_wrap(wrap_columns);
            }

            let wrap_rows = view.wrap_row_count();
            let viewport_height = bounds.size.height;
            let visible_row_count = f32::from(viewport_height) / f32::from(line_height);
            let max_scroll =
                (wrap_rows as f32 * f32::from(line_height) - f32::from(viewport_height)).max(0.);

            // A scroll asked for before the first layout could not be clamped
            // then; do it now, before the cursor gets a say.
            if let Some(row) = view.pending_scroll_row.take() {
                view.scroll_top = row * f32::from(line_height);
            }

            // Autoscroll to the cursor if it moved off-screen.
            if view.autoscroll {
                view.autoscroll = false;
                let cursor_row = view.cursor_wrap_row();
                let cursor_top = cursor_row as f32 * f32::from(line_height);
                let cursor_bottom = cursor_top + f32::from(line_height);
                if cursor_top < view.scroll_top {
                    view.scroll_top = cursor_top;
                } else if cursor_bottom > view.scroll_top + f32::from(viewport_height) {
                    view.scroll_top = cursor_bottom - f32::from(viewport_height);
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
            let last_row =
                (first_row + visible_row_count.ceil() as u32 + 2 * OVERSCAN + 1).min(wrap_rows);
            let top =
                bounds.top() - px(view.scroll_top - first_row as f32 * f32::from(line_height));

            // Highlights of the visible byte range, queried once per frame.
            let (visible_start, visible_end) = view.visible_byte_range(first_row, last_row);
            let spans = view.highlights(visible_start..visible_end).to_vec();

            let selection = view.selection_range();
            let has_selection = view.has_selection();
            let cursor_display_row = view.cursor.row;
            let search_matches: Vec<Range<usize>> = view.search.matches().to_vec();
            let current_match = view.search.current();
            let marked_range = view.marked_range.clone();
            let show_whitespace = view.settings().show_whitespace;
            let scroll_left = px(view.scroll_left);

            let mut rows = Vec::with_capacity((last_row - first_row) as usize);
            let probe = view.render_probe();
            let mut probe_rows = Vec::new();
            let mut selections = Vec::new();
            let mut matches = Vec::new();
            let mut current_line = Vec::new();
            let mut marked = Vec::new();
            let mut cursor = None;
            let mut widest = px(0.);

            for wrap_row in first_row..last_row {
                let origin_y = top + line_height * (wrap_row - first_row) as f32;
                let (display_row, segment) = view.wrap.to_display(wrap_row);
                let cell = view.display_map.to_buffer(display_row);
                let kind = view.display_map.diff().row_kind(display_row);
                let row_text = view.display_row_text(display_row);
                let range = view
                    .wrap
                    .segment_range(display_row, segment, row_text.len());
                let indent_x = if segment > 0 {
                    char_width * view.wrap.indent(display_row) as f32
                } else {
                    px(0.)
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
                    &font,
                    default_color,
                    &theme,
                    opacity,
                );
                let (line, reshaped) = view.shaped_line(&segment_text, &runs, font_size, window);
                if probe {
                    probe_rows.push(RowRender {
                        display_row,
                        segment,
                        highlight_spans: used_spans,
                        reshaped,
                    });
                }
                widest = widest.max(line.width() + indent_x);

                let whitespace = if show_whitespace {
                    let source = row_text.source();
                    let slice = &source[(range.start as usize).min(source.len())
                        ..(range.end as usize).min(source.len())];
                    whitespace_overlay(slice, tab_size).map(|overlay| {
                        shape_cached(
                            view,
                            &overlay,
                            &font,
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
                    DisplayCell::Buffer(buffer_row) if segment == 0 => {
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

                let layout = RowLayout {
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
                };

                let row_bounds = |from: u32, to: u32| -> Bounds<Pixels> {
                    let start_x = text_origin_x + layout.x_for(from) - scroll_left;
                    let end_x = text_origin_x + layout.x_for(to) - scroll_left;
                    Bounds::from_corners(
                        point(start_x, origin_y),
                        point(end_x.max(start_x), origin_y + line_height),
                    )
                };

                if display_row == cursor_display_row {
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

                // Search matches, for real rows only.
                if let DisplayCell::Buffer(_) = cell {
                    let row_start = base;
                    let row_end = base + layout.row_text.len() as usize;
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
                        let is_current = current_match
                            .as_ref()
                            .is_some_and(|current| current == found);
                        matches.push((row_bounds(from, to), is_current));
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

                rows.push(layout);
            }

            // Horizontal scrolling only exists without soft wrap.
            let max_scroll_x = if view.soft_wrap() {
                0.
            } else {
                (f32::from(widest) - f32::from(text_width)).max(0.)
            };
            view.scroll_left = view.scroll_left.clamp(0., max_scroll_x);

            // Floating pill per visible hunk.
            let mut pills = Vec::new();
            for hunk_ix in 0..view.hunks.len() {
                let range = view.display_map.diff().hunk_display_range(hunk_ix);
                let wrap_row = view.wrap.to_wrap_row(range.start, 0);
                if wrap_row < first_row || wrap_row >= last_row {
                    continue;
                }
                let origin_y = top + line_height * (wrap_row - first_row) as f32;
                let pill_bounds = Bounds::new(
                    point(
                        bounds.right() - px(PILL_MARGIN + PILL_WIDTH + SCROLLBAR_WIDTH),
                        origin_y + (line_height - px(PILL_HEIGHT)) / 2.,
                    ),
                    size(px(PILL_WIDTH), px(PILL_HEIGHT)),
                );
                let accept_line = shape(
                    "✓ Aceptar".to_string(),
                    &font,
                    font_size,
                    theme::color(theme.diff_added),
                    window,
                );
                let reject_line = shape(
                    "✗ Rechazar".to_string(),
                    &font,
                    font_size,
                    theme::color(theme.diff_deleted),
                    window,
                );
                // The click targets split the pill in half, so they do not
                // depend on the width of the shaped text.
                let half = px(PILL_WIDTH / 2.);
                let accept = Bounds::new(
                    point(pill_bounds.left(), pill_bounds.top()),
                    size(half, px(PILL_HEIGHT)),
                );
                let reject = Bounds::new(
                    point(pill_bounds.left() + half, pill_bounds.top()),
                    size(half, px(PILL_HEIGHT)),
                );
                pill_hitboxes.push((hunk_ix, accept, reject));
                pills.push(PillLayout {
                    hunk_ix,
                    bounds: pill_bounds,
                    accept,
                    reject,
                    accept_line,
                    reject_line,
                    accept_hitbox: hitbox.clone(),
                    reject_hitbox: hitbox.clone(),
                });
            }

            // Scrollbar: 8 px, only while there is something to scroll.
            let alpha = view.scrollbar_alpha();
            let scrollbar = (max_scroll > 0. && alpha > 0.).then(|| {
                let track = Bounds::new(
                    point(bounds.right() - px(SCROLLBAR_WIDTH), bounds.top()),
                    size(px(SCROLLBAR_WIDTH), bounds.size.height),
                );
                let visible = f32::from(viewport_height);
                let content = wrap_rows as f32 * f32::from(line_height);
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
                first_wrap_row: first_row,
                rows: rows
                    .iter()
                    .map(|row| (row.wrap_row, row.display_row, row.segment, row.line.clone()))
                    .collect(),
                visible_row_count,
            });
            view.end_frame(probe_rows);

            EditorPrepaint {
                hitbox: hitbox.clone(),
                rows,
                pills,
                cursor,
                marked,
                selections,
                matches,
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
            }
        });

        // Real hitboxes for the pill buttons, inserted on top of the editor.
        let mut prepaint = prepaint;
        for (pill, (_, accept, reject)) in prepaint.pills.iter_mut().zip(pill_hitboxes) {
            pill.accept_hitbox = window.insert_hitbox(accept, HitboxBehavior::Normal);
            pill.reject_hitbox = window.insert_hitbox(reject, HitboxBehavior::Normal);
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

        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
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

                // Gutter diff bar.
                if let Some(color) = match row.kind {
                    RowKind::Phantom(_) => Some(theme::color(theme.diff_deleted)),
                    RowKind::Added(_) => Some(theme::color(theme.diff_added)),
                    RowKind::Normal => None,
                } {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(bounds.left() + px(GUTTER_PADDING_LEFT), row.origin_y),
                            size(px(GUTTER_BAR_WIDTH), line_height),
                        ),
                        color,
                    ));
                }
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
                // Line number, right aligned against the text column.
                if let Some(number) = &row.number {
                    let x = prepaint.text_origin_x - px(GUTTER_TEXT_GAP) - number.width();
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

            // Floating pills, on top of everything.
            for pill in &prepaint.pills {
                let corners = Corners::all(px(6.));
                window.paint_drop_shadows(
                    pill.bounds,
                    corners,
                    &[BoxShadow {
                        color: hsla(0., 0., 0., 0.45),
                        offset: point(px(0.), px(2.)),
                        blur_radius: px(8.),
                        spread_radius: px(0.),
                        inset: false,
                    }],
                );
                let mut quad = fill(pill.bounds, theme::color(theme.elevated));
                quad.corner_radii = corners;
                window.paint_quad(quad);
                let text_y = pill.bounds.top() + (px(PILL_HEIGHT) - line_height) / 2.;
                pill.accept_line
                    .paint(
                        point(pill.accept.left() + px(12.), text_y),
                        line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
                pill.reject_line
                    .paint(
                        point(pill.reject.left() + px(4.), text_y),
                        line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
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

        // Mouse handling.
        let pill_targets: Vec<(usize, Hitbox, Hitbox)> = prepaint
            .pills
            .iter()
            .map(|pill| {
                (
                    pill.hunk_ix,
                    pill.accept_hitbox.clone(),
                    pill.reject_hitbox.clone(),
                )
            })
            .collect();
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
                for (hunk_ix, accept, reject) in &pill_targets {
                    if accept.is_hovered_at(event.position, window) {
                        window.focus(&focus_handle, cx);
                        view.update(cx, |view, cx| view.accept_hunk(*hunk_ix, cx))
                            .ok();
                        return;
                    }
                    if reject.is_hovered_at(event.position, window) {
                        window.focus(&focus_handle, cx);
                        view.update(cx, |view, cx| view.reject_hunk(*hunk_ix, cx))
                            .ok();
                        return;
                    }
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

        window.on_mouse_event({
            let view = self.view.downgrade();
            let max_scroll = prepaint.max_scroll;
            move |event: &MouseMoveEvent, phase: DispatchPhase, window: &mut Window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let _ = view.update(cx, |view, cx| {
                    view.note_mouse_moved(cx);
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
            target: "asteroid_editor::frame",
            rows = prepaint.rows.len(),
            gutter_width = f32::from(prepaint.gutter_width),
            frame_us = elapsed.as_micros() as u64,
            "frame painted"
        );
    }
}
