//! The vertical scrollbar over the transcript
//! (`docs/specs/12-etapa7-ronda4.md` §4, `02-visual.md` §5 "Scroll").
//!
//! It mirrors the code editor's: 8 px against the right edge, a thumb of at
//! least 24 px with radius 3, `text.muted` at 80 % over a track at 40 % of
//! the background, fully visible for [`SCROLLBAR_HOLD`] after the mouse moves
//! over the transcript or its content scrolls, then fading out over
//! [`SCROLLBAR_FADE`]. It is a layer over the list (an absolute `canvas`), so
//! it takes no width from the list or the bubbles, and it exists only while
//! there is something to scroll.
//!
//! The data comes from the list's own [`ListState`]: the canvas is the list's
//! next sibling, so when it prepaints the list has already laid out this
//! frame and its offsets are current (no frame of delay, D3).
//!
//! While it is invisible it asks for no frames: one repaint when it appears
//! (the mouse moved), one when the fade starts, animation frames only during
//! the 200 ms fade, one repaint when it is gone.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Context, Corners, CursorStyle, DispatchPhase, Hitbox, HitboxBehavior, IntoElement,
    ListState, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Styled as _,
    Task, WeakEntity, Window, canvas, fill, point, px, size,
};

use crate::panel::ChatPanel;
use crate::theme::{ChatTheme, alpha};

/// How long the scrollbar stays fully visible after the mouse moves or the
/// transcript scrolls. The same value as the editor's `SCROLLBAR_HOLD`
/// (`cincel-editor` does not export it).
pub const SCROLLBAR_HOLD: Duration = Duration::from_millis(1000);
/// How long the scrollbar takes to fade out afterwards (the editor's
/// `SCROLLBAR_FADE`).
pub const SCROLLBAR_FADE: Duration = Duration::from_millis(200);
/// Width of the track, in pixels (unscaled, like the editor's).
pub const SCROLLBAR_WIDTH: f32 = 8.;
/// Shortest thumb, in pixels.
pub const SCROLLBAR_MIN_THUMB: f32 = 24.;

/// Where the scrollbar was laid out in the last frame, and how opaque.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarGeometry {
    /// The 8 px track, as tall as the list.
    pub track: Bounds<Pixels>,
    /// The thumb, inside the track.
    pub thumb: Bounds<Pixels>,
    /// Opacity, from 1 (visible) down to 0 at the end of the fade.
    pub alpha: f32,
}

/// The panel's side of the scrollbar.
#[derive(Default)]
pub(crate) struct ScrollbarState {
    /// When the mouse last moved over the transcript or its content last
    /// scrolled; `None` once the fade is over.
    shown_at: Option<Instant>,
    /// One repaint when the fade starts and one when it is over.
    fade_task: Option<Task<()>>,
    /// While the thumb is dragged, how far below the thumb's top it was
    /// grabbed.
    drag: Option<f32>,
    /// Whether the mouse rests on the visible track (it does not fade then).
    hover_track: bool,
    /// The list's scroll offset in the last frame, to see it move.
    last_offset: Option<Pixels>,
    /// The last frame's geometry, `None` when nothing was painted.
    painted: Option<ScrollbarGeometry>,
    /// How many animation frames the bar has asked for (R9: none while it
    /// is invisible).
    frames_requested: Rc<Cell<u64>>,
}

impl ScrollbarState {
    /// Forgets the last offset, so jumping to another conversation is not
    /// taken for a scroll.
    pub(crate) fn reset_offset(&mut self) {
        self.last_offset = None;
    }
}

/// Geometry of the track and the thumb over `bounds` (the list's), for a
/// content that scrolls `max` pixels and is scrolled `offset` of them.
fn geometry(bounds: Bounds<Pixels>, max: f32, offset: f32) -> (Bounds<Pixels>, Bounds<Pixels>) {
    let track = Bounds::new(
        point(bounds.right() - px(SCROLLBAR_WIDTH), bounds.top()),
        size(px(SCROLLBAR_WIDTH), bounds.size.height),
    );
    let visible = f32::from(bounds.size.height);
    let content = max + visible;
    let thumb_height = (visible / content * visible)
        .max(SCROLLBAR_MIN_THUMB)
        .min(visible);
    let travel = visible - thumb_height;
    let thumb_top = if max > 0. { offset / max * travel } else { 0. };
    let thumb = Bounds::new(
        point(track.left() + px(1.), track.top() + px(thumb_top)),
        size(px(SCROLLBAR_WIDTH - 2.), px(thumb_height)),
    );
    (track, thumb)
}

/// The scroll offset that puts the thumb's top `thumb_top` pixels below the
/// track's.
fn offset_for_thumb(track: Bounds<Pixels>, thumb: Bounds<Pixels>, max: f32, thumb_top: f32) -> f32 {
    let travel = f32::from(track.size.height - thumb.size.height).max(1.);
    (thumb_top / travel * max).clamp(0., max)
}

/// What the canvas laid out in one frame.
struct Prepaint {
    /// The whole list, to see the mouse move over it.
    area: Hitbox,
    /// How far the content scrolls (frozen while the thumb is dragged).
    max: f32,
    /// The visible bar, when there is one.
    bar: Option<(ScrollbarGeometry, Hitbox)>,
}

impl ChatPanel {
    /// Opacity of the scrollbar at `now`.
    fn scrollbar_alpha(&self, now: Instant) -> f32 {
        let state = &self.scrollbar;
        if state.drag.is_some() || state.hover_track {
            return 1.;
        }
        let Some(at) = state.shown_at else {
            return 0.;
        };
        let elapsed = now.saturating_duration_since(at);
        if elapsed < SCROLLBAR_HOLD {
            return 1.;
        }
        let fading = (elapsed - SCROLLBAR_HOLD).as_secs_f32() / SCROLLBAR_FADE.as_secs_f32();
        (1. - fading).clamp(0., 1.)
    }

    /// Shows the scrollbar for [`SCROLLBAR_HOLD`] from now; whether it was
    /// hidden (the caller repaints then).
    fn note_scrollbar_activity(&mut self, cx: &mut Context<Self>) -> bool {
        let now = cx.background_executor().now();
        let was_hidden = self.scrollbar_alpha(now) <= 0.;
        self.scrollbar.shown_at = Some(now);
        self.scrollbar.fade_task = Some(cx.spawn(async move |this, cx| {
            // One repaint when the fade starts, one when it is over: in
            // between, the canvas asks for animation frames itself.
            cx.background_executor().timer(SCROLLBAR_HOLD).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                return;
            }
            cx.background_executor().timer(SCROLLBAR_FADE).await;
            this.update(cx, |this, cx| {
                this.scrollbar.shown_at = None;
                cx.notify();
            })
            .ok();
        }));
        was_hidden
    }

    /// The scrollbar's geometry and opacity in the last frame; `None` when
    /// none was painted (nothing to scroll, or invisible).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn scrollbar_geometry(&self) -> Option<ScrollbarGeometry> {
        self.scrollbar.painted
    }

    /// How many animation frames the scrollbar has asked for so far.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn scrollbar_frames_requested(&self) -> u64 {
        self.scrollbar.frames_requested.get()
    }

    /// Whether the scrollbar's thumb is being dragged.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn is_dragging_scrollbar(&self) -> bool {
        self.scrollbar.drag.is_some()
    }

    /// The layer over the transcript list: it covers the list (absolute,
    /// inset 0) and paints only the 8 px strip at its right edge.
    pub(crate) fn render_scrollbar(
        &self,
        theme: &ChatTheme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let panel = cx.entity().downgrade();
        let list = self.list.clone();
        let track_color = theme.bg_app;
        let thumb_color = theme.text_muted;
        let frames_requested = self.scrollbar.frames_requested.clone();
        let prepaint = {
            let panel = panel.clone();
            let list = list.clone();
            move |bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App| {
                scrollbar_prepaint(&panel, &list, bounds, window, cx)
            }
        };
        canvas(
            prepaint,
            move |_bounds, prepaint: Option<Prepaint>, window: &mut Window, _cx: &mut App| {
                let Some(prepaint) = prepaint else {
                    return;
                };
                if let Some((bar, _)) = &prepaint.bar {
                    window.paint_quad(fill(bar.track, alpha(track_color, bar.alpha * 0.4)));
                    let mut quad = fill(bar.thumb, alpha(thumb_color, bar.alpha * 0.8));
                    quad.corner_radii = Corners::all(px(3.));
                    window.paint_quad(quad);
                    // A fading scrollbar needs frames while it fades.
                    if bar.alpha < 1. {
                        frames_requested.set(frames_requested.get() + 1);
                        window.request_animation_frame();
                    }
                }
                register_mouse_handlers(panel, list, prepaint, window);
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

/// Reads the list, notes a scroll since the last frame and lays the bar out.
fn scrollbar_prepaint(
    panel: &WeakEntity<ChatPanel>,
    list: &ListState,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Prepaint> {
    let max = f32::from(list.max_offset_for_scrollbar().y).max(0.);
    let offset = f32::from(-list.scroll_px_offset_for_scrollbar().y).clamp(0., max);
    let now = cx.background_executor().now();
    let alpha = panel
        .update(cx, |panel, cx| {
            let state = &mut panel.scrollbar;
            let moved = state
                .last_offset
                .is_some_and(|last| (f32::from(last) - offset).abs() > 0.5);
            state.last_offset = Some(px(offset));
            if moved && max > 0. {
                // Painted in this same frame: no repaint to ask for.
                panel.note_scrollbar_activity(cx);
            }
            let alpha = if max > 0. {
                panel.scrollbar_alpha(now)
            } else {
                0.
            };
            panel.scrollbar.painted = (alpha > 0.).then(|| {
                let (track, thumb) = geometry(bounds, max, offset);
                ScrollbarGeometry {
                    track,
                    thumb,
                    alpha,
                }
            });
            if alpha <= 0. {
                panel.scrollbar.hover_track = false;
            }
            panel.scrollbar.painted
        })
        .ok()?;
    let area = window.insert_hitbox(bounds, HitboxBehavior::Normal);
    let bar = alpha.map(|bar| {
        let hitbox = window.insert_hitbox(bar.track, HitboxBehavior::Normal);
        (bar, hitbox)
    });
    Some(Prepaint { area, max, bar })
}

/// The mouse over the transcript: moving shows the bar, the visible track
/// takes its own presses (a drag of the thumb, a jump elsewhere) and nothing
/// else (`docs/specs/12-etapa7-ronda4.md` R6–R9).
fn register_mouse_handlers(
    panel: WeakEntity<ChatPanel>,
    list: ListState,
    prepaint: Prepaint,
    window: &mut Window,
) {
    let Prepaint { area, max, bar } = prepaint;
    let geometry = bar.as_ref().map(|(bar, _)| *bar);
    if let Some((_, track_hitbox)) = &bar {
        // The arrow over the bar, never the I-beam of the text below.
        window.set_cursor_style(CursorStyle::Arrow, track_hitbox);
    }

    window.on_mouse_event({
        let panel = panel.clone();
        let list = list.clone();
        move |event: &MouseMoveEvent, phase: DispatchPhase, window: &mut Window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let over_area = area.is_hovered_at(event.position, window);
            panel
                .update(cx, |panel, cx| {
                    if let (Some(grab), Some(bar)) = (panel.scrollbar.drag, geometry) {
                        let thumb_top = f32::from(event.position.y - bar.track.top()) - grab;
                        let offset = offset_for_thumb(bar.track, bar.thumb, max, thumb_top);
                        list.set_offset_from_scrollbar(point(px(0.), px(-offset)));
                        cx.notify();
                        return;
                    }
                    let on_track = over_area
                        && geometry.is_some_and(|bar| bar.track.contains(&event.position));
                    let left_track = panel.scrollbar.hover_track && !on_track;
                    let hover_changed = panel.scrollbar.hover_track != on_track;
                    panel.scrollbar.hover_track = on_track;
                    if max <= 0. || !(over_area || left_track) {
                        return;
                    }
                    let was_hidden = panel.note_scrollbar_activity(cx);
                    if was_hidden || hover_changed {
                        cx.notify();
                    }
                })
                .ok();
        }
    });

    window.on_mouse_event({
        let panel = panel.clone();
        let list = list.clone();
        move |event: &MouseDownEvent, phase: DispatchPhase, _window: &mut Window, cx: &mut App| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            let Some(bar) = geometry.filter(|bar| bar.track.contains(&event.position)) else {
                return;
            };
            // The press is the bar's: no bubble control below reacts and the
            // keyboard focus stays where it was.
            cx.stop_propagation();
            panel
                .update(cx, |panel, cx| {
                    list.scrollbar_drag_started();
                    // Like the wheel: the tail stops pulling the view down.
                    list.pause_following_tail();
                    let grab = if bar.thumb.contains(&event.position) {
                        f32::from(event.position.y - bar.thumb.top())
                    } else {
                        // Jump: centre the thumb on the click.
                        let half = f32::from(bar.thumb.size.height) / 2.;
                        let thumb_top = f32::from(event.position.y - bar.track.top()) - half;
                        let offset = offset_for_thumb(bar.track, bar.thumb, max, thumb_top);
                        list.set_offset_from_scrollbar(point(px(0.), px(-offset)));
                        half
                    };
                    panel.scrollbar.drag = Some(grab);
                    cx.notify();
                })
                .ok();
        }
    });

    window.on_mouse_event(
        move |_: &MouseUpEvent, phase: DispatchPhase, _window: &mut Window, cx: &mut App| {
            if phase != DispatchPhase::Capture {
                return;
            }
            panel
                .update(cx, |panel, cx| {
                    if panel.scrollbar.drag.take().is_none() {
                        return;
                    }
                    list.scrollbar_drag_ended();
                    // Released, it stays for the hold and then fades.
                    panel.note_scrollbar_activity(cx);
                    cx.notify();
                })
                .ok();
        },
    );
}
