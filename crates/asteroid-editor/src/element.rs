//! `EditorElement`: the GPUI element that lays out and paints the editor.
//!
//! Only the visible rows are shaped and painted (virtualization); shaped lines
//! are cached in the view, keyed by buffer version + row identity.

use std::time::Instant;

use gpui::{
    App, Bounds, BoxShadow, Corners, DispatchPhase, Element, ElementId, ElementInputHandler,
    Entity, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollWheelEvent,
    ShapedLine, SharedString, Style, TextAlign, TextRun, Window, fill, hsla, point, px, relative,
    size,
};

use crate::display_map::{DisplayCell, RowKind};
use crate::theme;
use crate::view::{EditorView, LayoutSnapshot, LineCacheKey};

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
/// Extra rows shaped above and below the viewport.
const OVERSCAN: u32 = 1;

/// A row ready to paint.
struct RowLayout {
    display_row: u32,
    kind: RowKind,
    line: ShapedLine,
    number: Option<ShapedLine>,
    origin_y: Pixels,
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
    selections: Vec<Bounds<Pixels>>,
    text_origin_x: Pixels,
    gutter_width: Pixels,
    line_height: Pixels,
    max_scroll: f32,
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

fn text_run(text: &str, font: &gpui::Font, color: gpui::Hsla) -> TextRun {
    TextRun {
        len: text.len(),
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn shape(
    text: String,
    font: &gpui::Font,
    font_size: Pixels,
    color: gpui::Hsla,
    window: &Window,
) -> ShapedLine {
    let run = text_run(&text, font, color);
    let runs: &[TextRun] = if run.len == 0 {
        &[]
    } else {
        std::slice::from_ref(&run)
    };
    window
        .text_system()
        .shape_line(SharedString::from(text), font_size, runs, None)
}

/// Shapes a line, reusing the cached layout when the buffer has not changed.
fn shape_cached(
    cache: &mut std::collections::HashMap<LineCacheKey, ShapedLine>,
    key: LineCacheKey,
    text: String,
    font: &gpui::Font,
    font_size: Pixels,
    color: gpui::Hsla,
    window: &Window,
) -> ShapedLine {
    if let Some(line) = cache.get(&key) {
        return line.clone();
    }
    let shaped = shape(text, font, font_size, color, window);
    cache.insert(key, shaped.clone());
    shaped
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

        let prepaint = self.view.update(cx, |view, _cx| {
            let font = view.style.font.clone();
            let font_size = view.style.font_size;
            let line_height = view.style.line_height;
            let text_color = theme::color(theme::TEXT);
            let phantom_color = theme::color_alpha(theme::TEXT, theme::PHANTOM_TEXT_ALPHA);
            let muted_color = theme::color(theme::TEXT_MUTED);

            // Invalidate the shaped-line cache when the buffer changed.
            if view.cache_version != view.buffer.version() {
                view.cache_version = view.buffer.version();
                view.line_cache.clear();
            }

            // Gutter geometry, from the width of a digit.
            let char_width = shape("0".to_string(), &font, font_size, muted_color, window).width();
            let digits = view.display_map.buffer_row_count().max(1).to_string().len() as f32;
            let gutter_width = px(GUTTER_PADDING_LEFT + GUTTER_BAR_WIDTH + GUTTER_BAR_GAP)
                + char_width * digits
                + px(GUTTER_TEXT_GAP);
            let text_origin_x = bounds.left() + gutter_width;

            let row_count = view.display_map.display_row_count();
            let viewport_height = bounds.size.height;
            let visible_row_count = f32::from(viewport_height) / f32::from(line_height);
            let max_scroll =
                (row_count as f32 * f32::from(line_height) - f32::from(viewport_height)).max(0.);

            // Autoscroll to the cursor if it moved off-screen.
            if view.autoscroll {
                view.autoscroll = false;
                let cursor_top = view.cursor.row as f32 * f32::from(line_height);
                let cursor_bottom = cursor_top + f32::from(line_height);
                if cursor_top < view.scroll_top {
                    view.scroll_top = cursor_top;
                } else if cursor_bottom > view.scroll_top + f32::from(viewport_height) {
                    view.scroll_top = cursor_bottom - f32::from(viewport_height);
                }
            }
            view.scroll_top = view.scroll_top.clamp(0., max_scroll);

            let first_row = ((view.scroll_top / f32::from(line_height)).floor() as u32)
                .saturating_sub(OVERSCAN);
            let last_row =
                (first_row + visible_row_count.ceil() as u32 + 2 * OVERSCAN + 1).min(row_count);
            let top =
                bounds.top() - px(view.scroll_top - first_row as f32 * f32::from(line_height));

            let selection = view.selection_range();
            let has_selection = selection.start != selection.end;
            let mut rows = Vec::with_capacity((last_row - first_row) as usize);
            let mut selections = Vec::new();
            let mut cursor = None;

            for display_row in first_row..last_row {
                let origin_y = top + line_height * (display_row - first_row) as f32;
                let cell = view.display_map.to_buffer(display_row);
                let kind = view.display_map.diff().row_kind(display_row);
                let (key, color) = match cell {
                    DisplayCell::Buffer(buffer_row) => {
                        (LineCacheKey::Buffer(buffer_row), text_color)
                    }
                    DisplayCell::Phantom { hunk_ix, line_ix } => {
                        (LineCacheKey::Phantom(hunk_ix, line_ix), phantom_color)
                    }
                };
                let text = view.display_row_text(display_row);
                let is_cursor_row = display_row == view.cursor.row;
                let line = shape_cached(
                    &mut view.line_cache,
                    key,
                    text,
                    &font,
                    font_size,
                    color,
                    window,
                );

                let number = match cell {
                    // The number of the current row is brighter, so it is not cached.
                    DisplayCell::Buffer(buffer_row) if is_cursor_row => Some(shape(
                        (buffer_row + 1).to_string(),
                        &font,
                        font_size,
                        text_color,
                        window,
                    )),
                    DisplayCell::Buffer(buffer_row) => Some(shape_cached(
                        &mut view.line_cache,
                        LineCacheKey::Number(buffer_row),
                        (buffer_row + 1).to_string(),
                        &font,
                        font_size,
                        muted_color,
                        window,
                    )),
                    // Phantom rows carry no line number (02-visual.md §5).
                    DisplayCell::Phantom { .. } => None,
                };

                if has_selection
                    && display_row >= selection.start.row
                    && display_row <= selection.end.row
                {
                    let start_x = if display_row == selection.start.row {
                        line.x_for_index(selection.start.column as usize)
                    } else {
                        px(0.)
                    };
                    let end_x = if display_row == selection.end.row {
                        line.x_for_index(selection.end.column as usize)
                    } else {
                        // Include the newline as half a character.
                        line.width() + char_width / 2.
                    };
                    selections.push(Bounds::from_corners(
                        point(text_origin_x + start_x, origin_y),
                        point(text_origin_x + end_x.max(start_x), origin_y + line_height),
                    ));
                }

                if display_row == view.cursor.row {
                    let x = text_origin_x + line.x_for_index(view.cursor.column as usize);
                    cursor = Some(Bounds::new(point(x, origin_y), size(px(2.), line_height)));
                }

                rows.push(RowLayout {
                    display_row,
                    kind,
                    line,
                    number,
                    origin_y,
                });
            }

            // Floating pill per visible hunk.
            let mut pills = Vec::new();
            for hunk_ix in 0..view.hunks.len() {
                let range = view.display_map.diff().hunk_display_range(hunk_ix);
                if range.start < first_row || range.start >= last_row {
                    continue;
                }
                let origin_y = top + line_height * (range.start - first_row) as f32;
                let pill_bounds = Bounds::new(
                    point(
                        bounds.right() - px(PILL_MARGIN + PILL_WIDTH),
                        origin_y + (line_height - px(PILL_HEIGHT)) / 2.,
                    ),
                    size(px(PILL_WIDTH), px(PILL_HEIGHT)),
                );
                let accept_line = shape(
                    "✓ Aceptar".to_string(),
                    &font,
                    font_size,
                    theme::color(theme::DIFF_ADDED),
                    window,
                );
                let reject_line = shape(
                    "✗ Rechazar".to_string(),
                    &font,
                    font_size,
                    theme::color(theme::DIFF_DELETED),
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

            view.layout = Some(LayoutSnapshot {
                bounds,
                text_origin_x,
                line_height,
                first_row,
                lines: rows.iter().map(|row| row.line.clone()).collect(),
                visible_row_count,
            });

            EditorPrepaint {
                hitbox: hitbox.clone(),
                rows,
                pills,
                cursor,
                selections,
                text_origin_x,
                gutter_width,
                line_height,
                max_scroll,
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
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );

        let focused = focus_handle.is_focused(window);
        let (cursor_row, blink_visible) = {
            let view = self.view.read(cx);
            (view.cursor.row, view.blink_visible)
        };
        let line_height = prepaint.line_height;

        window.paint_quad(fill(bounds, theme::color(theme::BG)));

        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for row in &prepaint.rows {
                let row_bounds = Bounds::new(
                    point(bounds.left(), row.origin_y),
                    size(bounds.size.width, line_height),
                );
                // Row background: diff colors win over the current-line color.
                match row.kind {
                    RowKind::Phantom(_) => window.paint_quad(fill(
                        row_bounds,
                        theme::color_alpha(theme::DIFF_DELETED, theme::DIFF_BG_ALPHA),
                    )),
                    RowKind::Added(_) => window.paint_quad(fill(
                        row_bounds,
                        theme::color_alpha(theme::DIFF_ADDED, theme::DIFF_BG_ALPHA),
                    )),
                    RowKind::Normal => {
                        if row.display_row == cursor_row {
                            window.paint_quad(fill(
                                row_bounds,
                                theme::color_alpha(theme::BG_ELEVATED, 0.6),
                            ));
                        }
                    }
                }

                // Gutter diff bar.
                if let Some(color) = match row.kind {
                    RowKind::Phantom(_) => Some(theme::color(theme::DIFF_DELETED)),
                    RowKind::Added(_) => Some(theme::color(theme::DIFF_ADDED)),
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

            for selection in &prepaint.selections {
                window.paint_quad(fill(*selection, theme::color(theme::SELECTION)));
            }

            for row in &prepaint.rows {
                row.line
                    .paint(
                        point(prepaint.text_origin_x, row.origin_y),
                        line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
            }

            if focused
                && blink_visible
                && let Some(cursor) = prepaint.cursor
            {
                window.paint_quad(fill(cursor, theme::color(theme::CURSOR)));
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
                let mut quad = fill(pill.bounds, theme::color(theme::BG_ELEVATED));
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
        });

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

        window.on_mouse_event({
            let view = self.view.clone();
            let hitbox = hitbox.clone();
            let focus_handle = focus_handle.clone();
            move |event: &MouseDownEvent, phase: DispatchPhase, window: &mut Window, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                for (hunk_ix, accept, reject) in &pill_targets {
                    if accept.is_hovered_at(event.position, window) {
                        window.focus(&focus_handle, cx);
                        view.update(cx, |view, cx| view.accept_hunk(*hunk_ix, cx));
                        return;
                    }
                    if reject.is_hovered_at(event.position, window) {
                        window.focus(&focus_handle, cx);
                        view.update(cx, |view, cx| view.reject_hunk(*hunk_ix, cx));
                        return;
                    }
                }
                if !hitbox.is_hovered_at(event.position, window) {
                    return;
                }
                window.focus(&focus_handle, cx);
                view.update(cx, |view, cx| {
                    let point = view.point_for_position(event.position, window);
                    view.begin_selection(point, event.modifiers.shift, cx);
                });
            }
        });

        window.on_mouse_event({
            let view = self.view.clone();
            move |event: &MouseMoveEvent, phase: DispatchPhase, window: &mut Window, cx| {
                if phase != DispatchPhase::Bubble || !event.dragging() {
                    return;
                }
                view.update(cx, |view, cx| {
                    if view.selecting {
                        let point = view.point_for_position(event.position, window);
                        view.update_selection(point, cx);
                    }
                });
            }
        });

        window.on_mouse_event({
            let view = self.view.clone();
            move |_: &MouseUpEvent, phase: DispatchPhase, _window: &mut Window, cx| {
                if phase == DispatchPhase::Bubble {
                    view.update(cx, |view, _cx| view.end_selection());
                }
            }
        });

        window.on_mouse_event({
            let view = self.view.clone();
            let hitbox = hitbox.clone();
            let max_scroll = prepaint.max_scroll;
            move |event: &ScrollWheelEvent, phase: DispatchPhase, window: &mut Window, cx| {
                if phase != DispatchPhase::Bubble || !hitbox.should_handle_scroll(window) {
                    return;
                }
                // 3 rows per wheel notch (02-visual.md §5).
                let delta = event.delta.pixel_delta(line_height * 3.).y;
                view.update(cx, |view, cx| view.scroll_by(delta, max_scroll, cx));
            }
        });

        let elapsed = self.started_at.elapsed();
        tracing::debug!(
            target: "asteroid_editor::frame",
            rows = prepaint.rows.len(),
            gutter_width = f32::from(prepaint.gutter_width),
            frame_us = elapsed.as_micros() as u64,
            "frame painted"
        );
    }
}
