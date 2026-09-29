//! `EditorView`: the GPUI entity that owns the shared buffer, the display
//! pipeline, the selection, the scroll state and the search bar, and that
//! implements `EntityInputHandler` (IME and dead keys).

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cincel_syntax::{
    CancelFlag, HighlightId, Language, LanguageRegistry, ParseOutcome, SyntaxState,
};
use cincel_text::{Buffer, BufferEvent, BufferSnapshot, EditSource, Point};
use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, CursorStyle, Entity, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, Font, FontFallbacks, FontFeatures, FontStyle, FontWeight,
    Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels,
    Render, Rgba, ShapedLine, SharedString, StatefulInteractiveElement, Styled, Subscription, Task,
    TextRun, UTF16Selection, Window, div, px,
};

use crate::actions::*;
use crate::decorations::{Decorator, TextDecorations};
use crate::display_map::{
    DiffTransformMap, DisplayCell, DisplayMap, DisplayPoint, DisplayRow, PhantomHunk, RowKind,
    RowText,
};
use crate::element::EditorElement;
use crate::review::{ReviewAction, ReviewHunkView, ReviewView};
use crate::search::{
    MatchLocation, PHANTOM_REPLACE_NOTICE, PhantomLines, SearchField, SearchState,
    replace_all_message,
};
use crate::settings::{EditorChrome, EditorSettings, SharedBuffer, detect_indentation};
use crate::theme::{self, EditorTheme};
use crate::wrap_map::{WRAP_UNITS_PER_PX, WrapMap, WrapRow, WrapSource};
use gpui::prelude::FluentBuilder as _;

mod editing;

pub use editing::MAX_BRACKET_SCAN;

/// How long the cursor keeps blinking after the last input (02-visual §5).
pub const BLINK_IDLE: Duration = Duration::from_secs(5);
/// Half period of the blink (02-visual §5).
pub const BLINK_INTERVAL: Duration = Duration::from_millis(500);
/// How long the scrollbar stays fully visible after the mouse moves.
pub const SCROLLBAR_HOLD: Duration = Duration::from_millis(1000);
/// How long the scrollbar takes to fade out afterwards.
pub const SCROLLBAR_FADE: Duration = Duration::from_millis(200);

/// How long a reparse may hold the UI thread before it is handed over to the
/// background executor.
///
/// A one-letter incremental reparse measures 0.6 ms median on a 5 000 line
/// Rust file (`docs/etapas/etapa-1.md`), so typing finishes inside this budget
/// and the frame that paints the keystroke already has the new colours: there
/// is no intermediate frame to flicker. Anything slower — a cold parse, a huge
/// file, a paste that rewrites the document — gives up and goes to the
/// background, where the highlights carried across the edit keep the text
/// coloured meanwhile.
pub const SYNC_PARSE_BUDGET: Duration = Duration::from_millis(2);

/// Shaped lines kept alive beyond the viewport before the cache is trimmed.
const LINE_CACHE_MAX: usize = 1024;
/// Frames a cached line may go unused before a trim drops it.
const LINE_CACHE_KEEP: u64 = 4;
/// Painted frames the render probe remembers.
const RENDER_FRAME_HISTORY: usize = 64;
/// How long "Sin cambios pendientes en este archivo…" stays (02-visual §6.2).
pub const REVIEW_NOTICE_DURATION: Duration = Duration::from_secs(3);
/// Step of the review spinner while the agent writes the file.
pub const SPINNER_INTERVAL: Duration = Duration::from_millis(80);
/// Height of the search bar and of the "Ir a la línea" prompt, in pixels.
///
/// The bar pushes the text down by exactly this much while it is open, in
/// search mode and in replace mode alike: replace mode splits the same strip
/// into two fields side by side instead of adding a row
/// (`docs/specs/07-etapa5-productividad.md` §10.2, decision D16).
pub const SEARCH_BAR_HEIGHT: f32 = 28.;
/// Below this editor width the "Reemplazar" / "Reemplazar todo" buttons turn
/// into glyphs with a tooltip and "Esc cierra" hides.
pub const SEARCH_BAR_COMPACT_WIDTH: f32 = 720.;

/// Highlight spans of one row or byte range, as `SyntaxState` returns them.
pub type HighlightSpans = Vec<(Range<usize>, HighlightId)>;

/// What the editor tells its host (the workspace tab that owns it).
///
/// `Eq` is not implemented because [`EditorEvent::ScrollChanged`] carries an
/// `f32`; `PartialEq` is, which is what matching on an event needs.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorEvent {
    /// The buffer became dirty (`true`) or clean (`false`).
    DirtyChanged(bool),
    /// The cursor moved to a new buffer position.
    CursorMoved {
        /// Position in buffer coordinates; on a phantom row it is the position
        /// an edit would land on.
        point: Point,
    },
    /// The vertical scroll moved, at most once per painted frame, so the host
    /// can persist the position of the tab.
    ScrollChanged {
        /// First visible display row, fractional (see [`EditorView::scroll_row`]).
        row: f32,
    },
    /// The user asked to save (`editor::save`). The host owns the file.
    SaveRequested,
    /// The user decided or navigated the review (pill, gutter icons, floating
    /// bar or keys). The editor does not apply it: the host does and answers
    /// with [`EditorView::set_review`].
    Review(ReviewAction),
}

/// Font and metrics, resolved from [`EditorSettings`] against the fonts the
/// platform actually has (see `docs/etapas/etapa-0.md`, finding 9).
#[derive(Clone, Debug)]
pub struct EditorStyle {
    /// Monospace font with its fallback chain.
    pub font: Font,
    /// Font size in pixels.
    pub font_size: Pixels,
    /// Row height in pixels.
    pub line_height: Pixels,
    /// Interface font of the review pill and the floating bar (02-visual §3:
    /// Inter, then the system sans).
    pub ui_font: Font,
    /// Font of the rows that are not code, when
    /// [`EditorSettings::prose_font_family`] is set.
    pub prose_font: Option<Font>,
}

/// Interface families tried for [`EditorStyle::ui_font`], best first.
const UI_FONT_FAMILIES: &[&str] = &[
    "Inter",
    "Fira Sans",
    "Noto Sans",
    "Cantarell",
    "Ubuntu",
    "DejaVu Sans",
];

impl EditorStyle {
    /// Picks the first family of `settings.font_family` the platform knows and
    /// keeps the rest as fallbacks.
    pub fn from_settings(settings: &EditorSettings, cx: &App) -> Self {
        let available = cx.text_system().all_font_names();
        let family: SharedString = settings
            .font_family
            .iter()
            .find(|name| available.iter().any(|found| found == *name))
            .map(|name| SharedString::from(name.clone()))
            .unwrap_or_else(|| SharedString::from("monospace"));
        let fallbacks = FontFallbacks::from_fonts(
            settings
                .font_family
                .iter()
                .filter(|name| name.as_str() != family.as_ref())
                .cloned()
                .collect(),
        );
        let font_size = px(settings.font_size.max(1.));
        let ui_family = UI_FONT_FAMILIES
            .iter()
            .find(|name| available.iter().any(|found| found == **name))
            .map(|name| SharedString::from(*name))
            .unwrap_or_else(|| family.clone());
        let font = Font {
            family,
            features: FontFeatures::default(),
            fallbacks: Some(fallbacks),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };
        let ui_font = Font {
            family: ui_family,
            fallbacks: font.fallbacks.clone(),
            ..font.clone()
        };
        let prose_font = settings.prose_font_family.as_ref().map(|families| {
            let family = families
                .iter()
                .find(|name| available.iter().any(|found| found == *name))
                .map(|name| SharedString::from(name.clone()))
                .unwrap_or_else(|| ui_font.family.clone());
            Font {
                family,
                fallbacks: font.fallbacks.clone(),
                ..font.clone()
            }
        });
        Self {
            font,
            font_size,
            line_height: font_size * settings.line_height.max(1.),
            ui_font,
            prose_font,
        }
    }
}

/// An entry of the shaped-line cache.
///
/// The cache is **content addressed**: the key is a hash of the exact inputs of
/// the shaper — the painted text, the font size and the `(len, colour)` of
/// every run — instead of the row number. A row whose text and colours did not
/// change therefore keeps its `ShapedLine` across an edit, even when the edit
/// pushed it up or down the document, and a row is re-shaped only when what it
/// paints really changed. `text` and `runs` are kept so a hash collision is
/// caught instead of painting the wrong line.
pub(crate) struct CachedLine {
    line: ShapedLine,
    text: SharedString,
    runs: Vec<(usize, Hsla)>,
    used_frame: u64,
}

/// What one painted wrap row used, recorded only while the render probe is on.
///
/// See [`EditorView::set_render_probe`]. It exists so a test can assert that no
/// frame ever painted a row that had highlights without them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowRender {
    /// The display row painted.
    pub display_row: DisplayRow,
    /// Soft-wrap segment of that row.
    pub segment: u32,
    /// Highlight spans that fell on this segment.
    pub highlight_spans: usize,
    /// Whether the line had to be shaped again for this frame.
    pub reshaped: bool,
    /// How the row was painted (plain, phantom or added).
    pub kind: RowKind,
    /// Whether a line number was painted in the gutter.
    pub line_number: bool,
    /// Word-diff spans painted over the row background.
    pub word_diffs: usize,
    /// Whether a host row background ([`crate::TextDecorations`] or
    /// [`EditorView::set_row_backgrounds`]) was painted behind it.
    pub background: bool,
    /// Whether it was painted with the code font while a prose font is set
    /// (always `true` without one).
    pub monospace: bool,
    /// The bar of the git column painted on it, if any.
    pub git_bar: Option<crate::git_gutter::GitGutterKind>,
    /// Search matches highlighted on this segment (phantom rows included).
    pub search_matches: usize,
    /// Whether one of them is the current match.
    pub current_search_match: bool,
}

/// An accept/reject pill, as the render probe saw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PillRender {
    /// The hunk it decides.
    pub hunk: u64,
    /// Whether its buttons react (`false` while the agent writes).
    pub enabled: bool,
    /// Whether it showed the "Turno anterior" clock.
    pub previous_turn: bool,
    /// Display row the pill sits on (the smart placement of 02-visual §6.1).
    pub display_row: DisplayRow,
    /// Whether no row had room for the full pill, so the compact two-icon
    /// version was painted over the code instead.
    pub compact: bool,
    /// Opacity of the pill (1, or 0.7 for the compact one).
    pub opacity: f32,
    /// Where the pill was painted.
    pub bounds: Bounds<Pixels>,
    /// Colour of the `✓` glyph.
    pub check_color: Hsla,
    /// Colour of the `✗` glyph.
    pub cross_color: Hsla,
    /// Colour of the "Aceptar" / "Rechazar" words.
    pub label_color: Hsla,
    /// The half under the mouse (`Some(true)` = accept), painted on
    /// `bg.surface`.
    pub hovered: Option<bool>,
}

/// A button of the floating bar, as the render probe saw it.
#[derive(Clone, Debug, PartialEq)]
pub struct BarButtonRender {
    /// Its text.
    pub text: String,
    /// Colour of the glyph in front of the word (`✓`, `✗`, arrows), or of the
    /// whole text when it has no glyph.
    pub glyph_color: Hsla,
    /// Colour of the word.
    pub label_color: Hsla,
    /// Whether the mouse is over it (painted on `bg.surface`).
    pub hovered: bool,
}

/// The review overlays of one painted frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReviewFrame {
    /// Pills painted, top to bottom.
    pub pills: Vec<PillRender>,
    /// The `+`/`−` gutter icons, as `(hunk id, line index)`.
    pub line_icons: Option<(u64, usize)>,
    /// Where the `+` and `−` icons were painted.
    pub line_icon_bounds: Option<(Bounds<Pixels>, Bounds<Pixels>)>,
    /// Left and right edges of the line-number column the icons sit in.
    pub number_column: (Pixels, Pixels),
    /// The buttons of the floating bar, in order.
    pub bar_buttons: Vec<BarButtonRender>,
    /// Text of the floating bar, its segments joined by spaces.
    pub bar: Option<String>,
    /// Whether the controls of the bar react.
    pub bar_enabled: bool,
    /// Tooltip painted, if any.
    pub tooltip: Option<String>,
    /// Hunks of the review state the frame was painted from.
    pub hunks: Vec<u64>,
}

/// One painted frame, as the render probe saw it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameRender {
    /// Buffer version this frame painted.
    pub text_version: u64,
    /// Highlight version this frame painted.
    pub highlight_version: u64,
    /// One entry per painted wrap row, in paint order.
    pub rows: Vec<RowRender>,
    /// Pills, gutter icons and floating bar.
    pub review: ReviewFrame,
    /// Width of the gutter (0 with [`EditorChrome::Minimal`]).
    pub gutter_width: Pixels,
    /// X of the first column of text.
    pub text_origin_x: Pixels,
    /// Whether the placeholder was painted (empty buffer).
    pub placeholder: bool,
    /// Bounds the parent gave the element for this frame (everything it
    /// paints stays inside them).
    pub bounds: Bounds<Pixels>,
    /// Width the text column was laid out (and soft-wrapped) at.
    pub text_width: Pixels,
    /// Wrap rows of the whole buffer at [`FrameRender::text_width`].
    pub wrap_rows: u32,
    /// The effective content mask the element painted under: its own
    /// bounds intersected with every mask of its ancestors. `None` until
    /// the frame is painted.
    pub paint_clip: Option<Bounds<Pixels>>,
    /// The git column: one entry per bar painted (one per wrap row) and one
    /// per deletion mark (its 3 × 6 px quad), bars first, top to bottom.
    pub git_marks: Vec<(crate::git_gutter::GitGutterKind, Bounds<Pixels>)>,
}

/// Geometry of one frame handed to [`EditorView::end_frame`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameGeometry {
    pub gutter_width: Pixels,
    pub text_origin_x: Pixels,
    pub bounds: Bounds<Pixels>,
    pub text_width: Pixels,
}

impl FrameRender {
    /// The row this frame painted for `display_row`, first segment first.
    pub fn row(&self, display_row: DisplayRow) -> Option<&RowRender> {
        self.rows.iter().find(|row| row.display_row == display_row)
    }
}

/// What the element measured on the last frame, so mouse events and the IME can
/// be mapped back to display positions.
#[derive(Clone)]
pub(crate) struct LayoutSnapshot {
    pub bounds: Bounds<Pixels>,
    pub text_origin_x: Pixels,
    pub line_height: Pixels,
    pub char_width: Pixels,
    pub first_wrap_row: WrapRow,
    /// One entry per laid-out wrap row: the shaped segment and what it maps to.
    pub rows: Vec<(WrapRow, DisplayRow, u32, ShapedLine)>,
    pub visible_row_count: f32,
}

/// How a drag selects.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectMode {
    Character,
    Word,
    Line,
}

/// Where typed text goes while a small prompt is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Prompt {
    /// `editor::go_to_line`, holding the digits typed so far.
    GoToLine(String),
}

/// Frame timings collected by the element, for the perf checks of the spec.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Frames measured.
    pub count: usize,
    /// Median frame time in microseconds.
    pub p50_us: u64,
    /// 95th percentile in microseconds.
    pub p95_us: u64,
    /// Worst frame in microseconds.
    pub max_us: u64,
}

/// The last highlights known to be good, carried across edits.
///
/// While a reparse is in flight the tree-sitter state is *moved* to the
/// background executor, so there is nothing to query: without this the editor
/// would paint a frame of plain, uncoloured text and colour it again when the
/// parse lands — the flicker of every keystroke. Instead the previous spans are
/// shifted by the byte delta of each edit (spans after the edit move, the ones
/// the edit cut through are clipped) and used until the real result arrives, at
/// which point they are replaced in one go.
struct HighlightCarry {
    /// Buffer byte range the spans cover.
    range: Range<usize>,
    spans: Vec<(Range<usize>, HighlightId)>,
}

/// Tree-sitter state plus the bookkeeping needed to reparse in the background.
struct SyntaxHost {
    /// `None` while the state is on the background executor.
    state: Option<SyntaxState>,
    /// Whether a background parse is running.
    parsing: bool,
    /// Whether the buffer changed while the state was away.
    stale: bool,
    /// Raised to cancel the running parse.
    cancel: CancelFlag,
    /// Bumped every time a parse lands.
    version: u64,
    /// Highlights of the last queried range.
    spans: Vec<(Range<usize>, HighlightId)>,
    /// `(buffer version, highlight version, range)` the spans were built for.
    spans_key: (u64, u64, usize, usize),
    /// Spans to paint with while the state is away; see [`HighlightCarry`].
    carry: Option<HighlightCarry>,
}

impl SyntaxHost {
    fn new(state: Option<SyntaxState>) -> Self {
        Self {
            state,
            parsing: false,
            stale: false,
            cancel: CancelFlag::new(),
            version: 0,
            spans: Vec::new(),
            spans_key: (u64::MAX, u64::MAX, 0, 0),
            carry: None,
        }
    }

    /// Forgets the carried spans: their byte offsets no longer mean anything
    /// (the whole text was replaced, or the language changed).
    fn drop_carry(&mut self) {
        self.carry = None;
        self.spans_key = (u64::MAX, u64::MAX, 0, 0);
    }
}

/// The carried spans that fall inside `range`, clipped to it.
fn carried_spans(
    carry: Option<&HighlightCarry>,
    range: &Range<usize>,
) -> Vec<(Range<usize>, HighlightId)> {
    let Some(carry) = carry else {
        return Vec::new();
    };
    carry
        .spans
        .iter()
        .filter(|(span, _)| span.end > range.start && span.start < range.end)
        .map(|(span, id)| (span.start.max(range.start)..span.end.min(range.end), *id))
        .collect()
}

/// Moves `spans` from the coordinates before an edit to the ones after it.
///
/// `start` is where the replacement begins, `old_len` the bytes it replaced and
/// `new_len` the bytes it wrote. Spans before the edit are untouched, spans
/// after it slide by the delta and a span the edit cuts through is clipped at
/// the edit (dropped when nothing is left of it), which is what "the edited row
/// keeps its spans clipped" means in byte space.
fn shift_spans(
    spans: &mut Vec<(Range<usize>, HighlightId)>,
    start: usize,
    old_len: usize,
    new_len: usize,
) {
    let old_end = start + old_len;
    // Only ever applied to offsets at or past `old_end`, so it cannot underflow.
    let slide = |offset: usize| (offset + new_len).saturating_sub(old_len);
    spans.retain_mut(|(range, _)| {
        if range.end <= start {
            return true;
        }
        if range.start >= old_end {
            range.start = slide(range.start);
            range.end = slide(range.end);
            return true;
        }
        // The edit cuts through this span: keep the part before it, if any.
        if range.start < start {
            range.end = start;
            return true;
        }
        false
    });
}

/// `((text version, cursor), matching brackets)` of the last bracket match.
type BracketCache = ((u64, DisplayPoint), Option<(usize, usize)>);

/// The editor entity.
pub struct EditorView {
    buffer: SharedBuffer,
    snapshot: BufferSnapshot,
    registry: Arc<LanguageRegistry>,
    language: Option<Arc<Language>>,
    syntax: SyntaxHost,
    /// Highlights of the phantom rows of each hunk, one vector per deleted
    /// line, keyed by a hash of the deleted text: a `set_review` that keeps a
    /// hunk keeps its highlights, wherever the hunk moved.
    phantom_spans: HashMap<u64, Vec<HighlightSpans>>,

    settings: EditorSettings,
    theme: EditorTheme,
    pub(crate) style: EditorStyle,
    /// Indentation detected in the file, preferred over the settings.
    detected_indent: Option<crate::settings::Indentation>,

    /// The review of the file, hunks sorted by `buffer_rows.start` (the same
    /// order as `display_map.diff().hunks()`, so the indices match).
    pub(crate) review: ReviewView,
    /// Id of the hunk the host calls current.
    review_current: Option<u64>,
    /// Deleted word ranges per hunk, per deleted line (line-local bytes).
    pub(crate) deleted_words: Vec<Vec<Vec<Range<u32>>>>,
    /// `phantom_spans` key of every hunk.
    phantom_keys: Vec<u64>,
    /// Display row under the mouse, for the pill and the `+`/`−` icons.
    pub(crate) hover_row: Option<DisplayRow>,
    /// Hunk whose "Turno anterior" clock the mouse is over.
    pub(crate) hover_clock: Option<u64>,
    /// `(hunk id, accept half?)` of the pill button under the mouse.
    pub(crate) hover_pill: Option<(u64, bool)>,
    /// Index of the floating bar button under the mouse.
    pub(crate) hover_bar: Option<usize>,
    /// Backgrounds set with [`EditorView::set_row_backgrounds`].
    row_backgrounds: Vec<(Range<u32>, Rgba)>,
    /// The host's [`Decorator`], if any.
    decorator: Option<Decorator>,
    /// What the decorator returned, and for which text version.
    decorations: Option<(u64, TextDecorations)>,
    /// Decorator highlights of the last queried range.
    decorated_spans: Vec<(Range<usize>, HighlightId)>,
    /// Muted text painted while the buffer is empty.
    pub(crate) placeholder: Option<SharedString>,
    /// Whether edits are refused (the chat composer while a permission waits).
    read_only: bool,
    /// `(hunk id, buffer row)` of the last decision, waiting for the host's
    /// answer to jump to the next hunk (`jump_to_next_on_decide`).
    pending_jump: Option<(u64, u32)>,
    /// "Sin cambios pendientes en este archivo…" is showing.
    pub(crate) review_notice: bool,
    /// Frame of the review spinner.
    pub(crate) spinner_phase: u32,
    /// Display-map rebuilds caused by `set_review`, for the tests.
    pub(crate) review_rebuilds: u64,
    pub(crate) display_map: DisplayMap,
    pub(crate) wrap: WrapMap,
    /// Soft wrap toggled for this view only (`editor::toggle_soft_wrap`).
    soft_wrap_override: Option<bool>,

    pub(crate) cursor: DisplayPoint,
    pub(crate) selection_anchor: DisplayPoint,
    /// Column the cursor tries to keep while moving up and down.
    goal_column: Option<u32>,

    pub(crate) scroll_top: f32,
    /// Vertical scroll asked for before the first layout, in display rows: the
    /// viewport height is not known yet, so it cannot be clamped.
    pub(crate) pending_scroll_row: Option<f32>,
    /// Last row reported with [`EditorEvent::ScrollChanged`].
    pub(crate) emitted_scroll_row: f32,
    pub(crate) scroll_left: f32,

    pub(crate) search: SearchState,
    pub(crate) search_open: bool,
    /// Buffer offset the search started from, so typing into the bar does not
    /// walk the matches.
    search_from: usize,
    pub(crate) prompt: Option<Prompt>,

    pub(crate) focus_handle: FocusHandle,
    pub(crate) marked_range: Option<Range<usize>>,
    pub(crate) layout: Option<LayoutSnapshot>,
    /// Content-addressed shaped lines; see [`CachedLine`].
    line_cache: HashMap<u64, CachedLine>,
    /// Painted frames so far, the age stamp of the cache entries.
    frame_counter: u64,
    /// Whether [`RowRender`]s are collected; off by default.
    render_probe: bool,
    render_frames: VecDeque<FrameRender>,
    /// `(text version, wrap columns, tab size, display rows)` the wrap map was
    /// built for. Kept apart from the line cache so a single-row edit can
    /// update one row instead of rebuilding the whole map.
    pub(crate) wrap_epoch: (u64, u32, u32, u32),
    pub(crate) blink_visible: bool,
    last_input: Instant,
    pub(crate) autoscroll: bool,
    /// The pending autoscroll centres the cursor row vertically (a jump to a
    /// change) instead of just bringing it into view. Consumed with
    /// `autoscroll` by the next prepaint.
    pub(crate) autoscroll_center: bool,
    pub(crate) selecting: bool,
    pub(crate) select_mode: SelectMode,
    select_origin: Range<DisplayPoint>,
    pub(crate) mouse_moved_at: Option<Instant>,
    pub(crate) scrollbar_drag: Option<f32>,
    dirty: bool,
    frame_times: VecDeque<u64>,
    /// Whether typed brackets and quotes auto-close.
    auto_close_pairs: bool,
    /// Buffer offsets of the closers the editor inserted itself, which typing
    /// the same closer steps over. Cleared whenever the cursor moves away.
    auto_closers: Vec<usize>,
    /// `((text version, cursor), result)` of the last bracket match.
    bracket_cache: Option<BracketCache>,
    /// Mouse cursor of every review control painted on the last frame, in
    /// paint order (the last one containing a point wins).
    pub(crate) control_cursors: Vec<(Bounds<Pixels>, CursorStyle)>,
    /// The git column of the gutter ([`EditorView::set_git_diff`]).
    pub(crate) git_gutter: crate::git_gutter::GitGutterState,
    _blink_task: Option<Task<()>>,
    _fade_task: Option<Task<()>>,
    _spinner_task: Option<Task<()>>,
    _notice_task: Option<Task<()>>,
    _activation: Option<Subscription>,
}

impl EventEmitter<EditorEvent> for EditorView {}

// -- construction and host API ---------------------------------------------

impl EditorView {
    /// Builds an editor over a shared buffer.
    ///
    /// `language` is the grammar to highlight with (`None` = plain text) and
    /// `registry` is used to resolve injected languages. `settings` and `theme`
    /// are owned by the host and can be replaced at any time with
    /// [`EditorView::set_settings`] and [`EditorView::set_theme`].
    pub fn new(
        buffer: SharedBuffer,
        language: Option<Arc<Language>>,
        registry: Arc<LanguageRegistry>,
        settings: EditorSettings,
        theme: EditorTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (snapshot, dirty) = {
            let mut buffer = buffer.lock();
            buffer.record_events(true);
            (buffer.snapshot(), buffer.is_dirty())
        };
        let text = snapshot.text();
        let detected_indent = detect_indentation(&text, 200);
        let syntax = language
            .clone()
            .map(|language| SyntaxState::new(registry.clone(), language, snapshot.clone()));
        let display_map = DisplayMap::new(snapshot.line_count(), Vec::new());
        let style = EditorStyle::from_settings(&settings, cx);
        let auto_close_pairs = settings.auto_close_pairs;

        let git_gutter = crate::git_gutter::GitGutterState::new(buffer.clone());
        let mut this = Self {
            buffer,
            snapshot,
            registry,
            language,
            syntax: SyntaxHost::new(syntax),
            phantom_spans: HashMap::new(),
            settings,
            theme,
            style,
            detected_indent,
            review: ReviewView::default(),
            review_current: None,
            deleted_words: Vec::new(),
            phantom_keys: Vec::new(),
            hover_row: None,
            hover_clock: None,
            hover_pill: None,
            hover_bar: None,
            row_backgrounds: Vec::new(),
            decorator: None,
            decorations: None,
            decorated_spans: Vec::new(),
            placeholder: None,
            read_only: false,
            pending_jump: None,
            review_notice: false,
            spinner_phase: 0,
            review_rebuilds: 0,
            display_map,
            wrap: WrapMap::new(),
            soft_wrap_override: None,
            cursor: DisplayPoint::default(),
            selection_anchor: DisplayPoint::default(),
            goal_column: None,
            scroll_top: 0.,
            pending_scroll_row: None,
            emitted_scroll_row: 0.,
            scroll_left: 0.,
            search: SearchState::default(),
            search_open: false,
            search_from: 0,
            prompt: None,
            focus_handle: cx.focus_handle(),
            marked_range: None,
            layout: None,
            line_cache: HashMap::new(),
            frame_counter: 0,
            render_probe: false,
            render_frames: VecDeque::new(),
            wrap_epoch: (u64::MAX, u32::MAX, 0, 0),
            blink_visible: true,
            last_input: Instant::now(),
            autoscroll: false,
            autoscroll_center: false,
            selecting: false,
            select_mode: SelectMode::Character,
            select_origin: DisplayPoint::default()..DisplayPoint::default(),
            mouse_moved_at: None,
            scrollbar_drag: None,
            // A buffer handed over already modified (the agent wrote it before
            // the tab opened) starts dirty.
            dirty,
            frame_times: VecDeque::new(),
            auto_close_pairs,
            auto_closers: Vec::new(),
            bracket_cache: None,
            control_cursors: Vec::new(),
            git_gutter,
            _blink_task: None,
            _fade_task: None,
            _spinner_task: None,
            _notice_task: None,
            _activation: None,
        };
        // 02-visual §6.2: the floating bar hides while the window is not
        // active, so a change of activation must repaint.
        this._activation = Some(cx.observe_window_activation(window, |_, _, cx| cx.notify()));
        this.request_reparse(cx);
        this.start_blinking(cx);
        this
    }

    /// The shared buffer, for the host that needs to save it.
    pub fn buffer(&self) -> &SharedBuffer {
        &self.buffer
    }

    /// Whether the buffer differs from what was last saved.
    ///
    /// The question is about content, not about versions: undoing back to the
    /// text on disk leaves the tab clean. [`cincel_text::Buffer::is_dirty`]
    /// is the one that decides; this is the value last reported with
    /// [`EditorEvent::DirtyChanged`].
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Tells the editor the buffer was written to disk.
    pub fn mark_saved(&mut self, cx: &mut Context<Self>) {
        self.buffer.lock().mark_saved();
        self.update_dirty(cx);
    }

    /// The cursor in buffer coordinates.
    pub fn cursor_point(&self) -> Point {
        let offset = self.edit_offset(self.cursor);
        self.snapshot.offset_to_point(offset)
    }

    /// The current settings.
    pub fn settings(&self) -> &EditorSettings {
        &self.settings
    }

    /// Replaces the settings (hot reload).
    pub fn set_settings(&mut self, settings: EditorSettings, cx: &mut Context<Self>) {
        let refont = settings.font_family != self.settings.font_family
            || settings.font_size != self.settings.font_size
            || settings.line_height != self.settings.line_height;
        self.set_auto_close_pairs(settings.auto_close_pairs);
        self.settings = settings;
        if refont {
            self.style = EditorStyle::from_settings(&self.settings, cx);
        }
        self.soft_wrap_override = None;
        // The font itself is not part of the cache key, so a font change is the
        // one thing the content-addressed cache cannot notice by itself.
        self.line_cache.clear();
        self.wrap_epoch = (u64::MAX, u32::MAX, 0, 0);
        cx.notify();
    }

    /// The current theme.
    pub fn theme(&self) -> &EditorTheme {
        &self.theme
    }

    /// Replaces the theme (hot reload).
    pub fn set_theme(&mut self, theme: EditorTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        self.line_cache.clear();
        cx.notify();
    }

    /// Replaces the language and reparses from scratch.
    pub fn set_language(&mut self, language: Option<Arc<Language>>, cx: &mut Context<Self>) {
        self.language = language.clone();
        self.syntax = SyntaxHost::new(language.map(|language| {
            SyntaxState::new(self.registry.clone(), language, self.snapshot.clone())
        }));
        self.phantom_spans.clear();
        self.line_cache.clear();
        self.request_reparse(cx);
        cx.notify();
    }

    /// The language being highlighted, if any.
    pub fn language(&self) -> Option<&Arc<Language>> {
        self.language.as_ref()
    }

    /// The hunks as the display map sees them (phantom text and added rows),
    /// in display order.
    pub fn hunks(&self) -> &[PhantomHunk] {
        self.display_map.diff().hunks()
    }

    /// Replaces the review with simulated hunks (E0 model).
    #[deprecated(note = "use `set_review` with a `ReviewView`")]
    pub fn set_hunks(&mut self, hunks: Vec<PhantomHunk>, cx: &mut Context<Self>) {
        self.set_review(ReviewView::from_phantoms(&hunks), cx);
    }

    /// The review being shown, hunks sorted by `buffer_rows.start`.
    pub fn review(&self) -> &ReviewView {
        &self.review
    }

    /// Whether soft wrap is on for this view.
    pub fn soft_wrap(&self) -> bool {
        self.soft_wrap_override.unwrap_or(self.settings.soft_wrap)
    }

    /// The whole text of the buffer.
    pub fn text(&self) -> String {
        self.snapshot.text()
    }

    /// Number of display rows (buffer rows plus phantom rows).
    pub fn display_row_count(&self) -> u32 {
        self.display_map.display_row_count()
    }

    /// First visible display row, fractional (`1.5` = the viewport starts in
    /// the middle of row 1).
    ///
    /// With soft wrap on the unit is the *wrap* row, which is what the element
    /// scrolls in; with soft wrap off — the default — the two spaces are the
    /// same. A scroll set before the first layout reads back unclamped.
    pub fn scroll_row(&self) -> f32 {
        if let Some(pending) = self.pending_scroll_row {
            return pending;
        }
        let line_height = f32::from(self.style.line_height).max(1.);
        self.scroll_top / line_height
    }

    /// Scrolls to an absolute row, clamped to the document.
    ///
    /// Works before the first layout: the viewport height is unknown then, so
    /// the value is kept and applied — clamped — on the first prepaint. This is
    /// the call a tab makes when it restores a saved position.
    pub fn set_scroll_row(&mut self, row: f32, cx: &mut Context<Self>) {
        let row = row.max(0.);
        let line_height = f32::from(self.style.line_height).max(1.);
        match self.layout.as_ref() {
            Some(layout) => {
                let viewport = f32::from(layout.bounds.size.height);
                let max_scroll = (self.wrap_row_count() as f32 * line_height - viewport).max(0.);
                self.scroll_top = (row * line_height).clamp(0., max_scroll);
                self.pending_scroll_row = None;
            }
            None => self.pending_scroll_row = Some(row),
        }
        cx.notify();
    }

    /// Places the cursor at a buffer point, collapsing the selection.
    ///
    /// The point is clipped to a valid position on a `char` boundary; a phantom
    /// row can never be addressed this way because the argument is in buffer
    /// coordinates. The editor scrolls the cursor into view if it is off-screen
    /// and emits [`EditorEvent::CursorMoved`].
    pub fn set_cursor(&mut self, point: Point, cx: &mut Context<Self>) {
        let point = self.snapshot.clip_point(point);
        let row = self.display_map.to_display_row(point.row);
        self.cursor = self.clip_point(DisplayPoint::new(row, point.column));
        self.selection_anchor = self.cursor;
        self.goal_column = None;
        self.auto_closers.clear();
        self.after_input(cx);
    }

    /// Name and byte range of the innermost named definition around the cursor
    /// — the function, method, struct, class or module the breadcrumb shows.
    ///
    /// `None` when the file has no grammar, has not been parsed yet, or the
    /// cursor sits outside every definition. The range is in the coordinates of
    /// the snapshot the current tree was parsed from, which can lag the buffer
    /// by one edit while a background parse is in flight.
    pub fn symbol_at_cursor(&self) -> Option<(String, Range<usize>)> {
        let state = self.syntax.state.as_ref()?;
        let tree = state.tree()?;
        let offset = self.edit_offset(self.cursor);
        crate::symbol::symbol_at(tree, state.snapshot(), state.language().name(), offset)
    }

    /// Frame timings collected so far.
    pub fn frame_stats(&self) -> FrameStats {
        if self.frame_times.is_empty() {
            return FrameStats::default();
        }
        let mut times: Vec<u64> = self.frame_times.iter().copied().collect();
        times.sort_unstable();
        let at = |q: f32| times[((times.len() as f32 - 1.) * q).round() as usize];
        FrameStats {
            count: times.len(),
            p50_us: at(0.5),
            p95_us: at(0.95),
            max_us: *times.last().unwrap_or(&0),
        }
    }

    /// The mouse cursor the editor shows at a window position, as of the last
    /// painted frame: the pointing hand over an enabled review control (pill
    /// buttons, the gutter `+`/`−`, the floating bar's buttons), the arrow over
    /// the rest of a pill or of the bar, over a disabled control and over the
    /// gutter and the vertical scrollbar (track and thumb), and the I-beam
    /// over the text.
    pub fn cursor_style_at(&self, position: gpui::Point<Pixels>) -> CursorStyle {
        if let Some((_, style)) = self
            .control_cursors
            .iter()
            .rev()
            .find(|(bounds, _)| bounds.contains(&position))
        {
            return *style;
        }
        match self.layout.as_ref() {
            Some(layout) if layout.bounds.contains(&position) => CursorStyle::IBeam,
            _ => CursorStyle::Arrow,
        }
    }

    pub(crate) fn record_frame(&mut self, micros: u64) {
        if self.frame_times.len() >= 600 {
            self.frame_times.pop_front();
        }
        self.frame_times.push_back(micros);
    }
}

// -- text-field API (the chat composer) --------------------------------------

impl EditorView {
    /// Full-width backgrounds per range of buffer rows, painted under the text
    /// (a Markdown fence in the chat composer). Additive to whatever the
    /// [`Decorator`] returns; an empty vector clears them.
    pub fn set_row_backgrounds(&mut self, rows: Vec<(Range<u32>, Rgba)>, cx: &mut Context<Self>) {
        if self.row_backgrounds != rows {
            self.row_backgrounds = rows;
            cx.notify();
        }
    }

    /// The backgrounds set with [`EditorView::set_row_backgrounds`].
    pub fn row_backgrounds(&self) -> &[(Range<u32>, Rgba)] {
        &self.row_backgrounds
    }

    /// Installs (or removes) a [`Decorator`]: while one is installed its
    /// highlights replace the tree-sitter ones, and its row backgrounds and
    /// monospace rows are painted too. It runs once per text version.
    pub fn set_decorator(&mut self, decorator: Option<Decorator>, cx: &mut Context<Self>) {
        self.decorator = decorator;
        self.decorations = None;
        self.decorated_spans.clear();
        self.line_cache.clear();
        self.invalidate_layout();
        cx.notify();
    }

    /// What the decorator returned for the current text, if one is installed.
    pub fn decorations(&mut self) -> Option<&TextDecorations> {
        self.refresh_decorations();
        self.decorations
            .as_ref()
            .map(|(_, decorations)| decorations)
    }

    fn refresh_decorations(&mut self) {
        let Some(decorator) = self.decorator.clone() else {
            self.decorations = None;
            return;
        };
        let version = self.snapshot.version();
        if self
            .decorations
            .as_ref()
            .is_some_and(|(computed, _)| *computed == version)
        {
            return;
        }
        let text = self.snapshot.text();
        self.decorations = Some((version, decorator(&text)));
    }

    /// The background painted behind a buffer row, if any: the decorator's
    /// first, then [`EditorView::set_row_backgrounds`]'s (which win).
    pub(crate) fn row_background(&mut self, row: u32) -> Option<Rgba> {
        self.refresh_decorations();
        let manual = self
            .row_backgrounds
            .iter()
            .rev()
            .find(|(rows, _)| rows.contains(&row))
            .map(|(_, color)| *color);
        manual.or_else(|| {
            self.decorations
                .as_ref()
                .and_then(|(_, decorations)| decorations.background(row))
        })
    }

    /// Whether a buffer row is painted with the code font while a prose font
    /// is set.
    pub(crate) fn is_monospace_row(&mut self, row: u32) -> bool {
        self.refresh_decorations();
        self.decorations
            .as_ref()
            .is_some_and(|(_, decorations)| decorations.is_monospace(row))
    }

    /// The font a display row is painted with.
    pub(crate) fn font_for_cell(&mut self, cell: DisplayCell) -> Font {
        let Some(prose) = self.style.prose_font.clone() else {
            return self.style.font.clone();
        };
        match cell {
            DisplayCell::Buffer(row) if !self.is_monospace_row(row) => prose,
            _ => self.style.font.clone(),
        }
    }

    /// Muted text painted at the first row while the buffer is empty.
    pub fn set_placeholder(&mut self, placeholder: Option<SharedString>, cx: &mut Context<Self>) {
        if self.placeholder != placeholder {
            self.placeholder = placeholder;
            cx.notify();
        }
    }

    /// The placeholder, if one is set.
    pub fn placeholder(&self) -> Option<&SharedString> {
        self.placeholder.as_ref()
    }

    /// Refuses (or accepts again) every edit: typing, pasting, undo, the line
    /// commands and [`EditorView::set_text`]. Moving and selecting still work.
    pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<Self>) {
        if self.read_only != read_only {
            self.read_only = read_only;
            cx.notify();
        }
    }

    /// Whether edits are refused.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Replaces the whole text (one undo step) and puts the cursor at the
    /// buffer offset `cursor`, clipped to the text. Does nothing while
    /// read-only.
    pub fn set_text(&mut self, text: &str, cursor: usize, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let len = self.snapshot.len();
        if self.snapshot.text() != text {
            self.auto_closers.clear();
            self.edit_buffer(&[(0..len, text)], cx);
            self.rebuild_display_map();
        }
        let offset = self.snapshot.clip_offset(cursor.min(self.snapshot.len()));
        self.cursor = self.display_point_for_offset(offset);
        self.selection_anchor = self.cursor;
        self.marked_range = None;
        self.goal_column = None;
        self.after_input(cx);
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.snapshot.len() == 0
    }

    /// The cursor as a buffer byte offset.
    pub fn cursor_offset(&self) -> usize {
        self.edit_offset(self.cursor)
    }

    /// Number of visual rows of the text for the last laid-out width (soft
    /// wrap segments count), at least 1.
    pub fn layout_row_count(&self) -> u32 {
        self.wrap_row_count().max(1)
    }
}

// -- buffer synchronisation -------------------------------------------------

impl EditorView {
    /// Tells the editor the shared buffer changed behind its back (the agent
    /// wrote it, the store reloaded it from disk, another split edited it) and
    /// keeps every derived structure in sync.
    pub fn buffer_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_buffer(cx);
    }

    /// Picks up edits made through the shared handle. Called by the element at
    /// the start of every layout, and by the host through
    /// [`EditorView::buffer_changed`] when it edits the buffer itself.
    pub(crate) fn sync_buffer(&mut self, cx: &mut Context<Self>) {
        let (snapshot, events) = {
            let mut buffer = self.buffer.lock();
            if buffer.version() == self.snapshot.version() {
                buffer.drain_events();
                return;
            }
            (buffer.snapshot(), buffer.drain_events())
        };
        self.snapshot = snapshot;
        self.apply_syntax_events(&events);
        self.rebuild_display_map();
        self.invalidate_layout();
        self.request_reparse(cx);
        self.update_dirty(cx);
        cx.notify();
    }

    fn apply_syntax_events(&mut self, events: &[BufferEvent]) {
        // The colours the editor is already painting move with the text, so the
        // frame that shows the keystroke keeps them even if the reparse has to
        // go to the background.
        self.carry_highlights_through(events);
        let Some(state) = self.syntax.state.as_mut() else {
            self.syntax.stale = true;
            return;
        };
        // One event: the snapshot we hold is exactly the one after it, so the
        // parse stays incremental. More than one (an undo of a grouped
        // transaction): re-seed the state instead of guessing intermediate
        // snapshots.
        if events.len() == 1 {
            state.apply_event(&events[0], self.snapshot.clone());
        } else {
            state.reset(self.snapshot.clone());
            self.syntax.drop_carry();
        }
    }

    /// Shifts the carried highlight spans by the delta of every edit, so they
    /// still describe the text after it.
    fn carry_highlights_through(&mut self, events: &[BufferEvent]) {
        let Some(carry) = self.syntax.carry.as_mut() else {
            return;
        };
        for event in events {
            let BufferEvent::Edited {
                old_ranges,
                new_ranges,
                ..
            } = event;
            for (old, new) in old_ranges.iter().zip(new_ranges.iter()) {
                // `new.start` is where the replacement begins in the text the
                // spans are already in (the previous edits of this event have
                // been applied to them), so no accumulator is needed.
                let start = new.start;
                let old_len = old.end - old.start;
                let new_len = new.end - new.start;
                shift_spans(&mut carry.spans, start, old_len, new_len);
                let old_end = start + old_len;
                if carry.range.start >= old_end {
                    carry.range.start = (carry.range.start + new_len).saturating_sub(old_len);
                }
                if carry.range.end >= old_end {
                    carry.range.end = (carry.range.end + new_len).saturating_sub(old_len);
                }
            }
        }
        // Never let the carried range claim more text than there is.
        let len = self.snapshot.len();
        carry.range.start = carry.range.start.min(len);
        carry.range.end = carry.range.end.min(len);
        // The bytes the edit wrote have no colours of their own yet: the
        // clipped spans leave them in the plain text colour until the reparse
        // lands, which is one word at the cursor rather than the whole file.
        self.syntax.spans_key = (u64::MAX, u64::MAX, 0, 0);
    }

    fn update_dirty(&mut self, cx: &mut Context<Self>) {
        // Content-based, so an undo back to the saved text cleans the tab.
        // Never call this while holding the buffer lock.
        let dirty = self.buffer.lock().is_dirty();
        if dirty != self.dirty {
            self.dirty = dirty;
            cx.emit(EditorEvent::DirtyChanged(dirty));
        }
    }

    /// Drops what an edit invalidated. The shaped lines are **not** touched:
    /// the cache is keyed by content, so the rows the edit did not change keep
    /// their `ShapedLine` and are never shaped again.
    fn invalidate_layout(&mut self) {
        self.wrap_epoch = (u64::MAX, u32::MAX, 0, 0);
    }
}

// -- syntax highlighting ----------------------------------------------------

impl EditorView {
    /// Brings the highlights up to date with the buffer.
    ///
    /// The parse is tried **on the UI thread first**, with a
    /// [`SYNC_PARSE_BUDGET`] deadline. An incremental reparse of one keystroke
    /// is far below it, so the common case finishes here and the very frame
    /// that paints the new character already paints it with its final colours —
    /// there is no intermediate frame at all. Only a parse that blows the
    /// budget (a cold file, a paste that rewrites the document) is handed to
    /// the background executor, and there the carried highlights cover the gap.
    fn request_reparse(&mut self, cx: &mut Context<Self>) {
        if self.syntax.parsing {
            // Let the running parse give up; its completion restarts.
            self.syntax.cancel.cancel();
            return;
        }
        if self.reparse_now() {
            cx.notify();
            return;
        }
        let Some(mut state) = self.syntax.state.take() else {
            return;
        };
        let cancel = CancelFlag::new();
        self.syntax.cancel = cancel.clone();
        self.syntax.parsing = true;
        let task = cx.background_executor().spawn(async move {
            state.reparse(&cancel);
            state
        });
        cx.spawn(async move |this, cx| {
            let state = task.await;
            this.update(cx, |this, cx| {
                this.syntax.state = Some(state);
                this.syntax.parsing = false;
                this.syntax.version += 1;
                let stale = std::mem::take(&mut this.syntax.stale);
                let dirty = this
                    .syntax
                    .state
                    .as_ref()
                    .map(|state| {
                        stale
                            || state.is_dirty()
                            || state.snapshot().version() != this.snapshot.version()
                    })
                    .unwrap_or(false);
                if dirty {
                    if let Some(state) = this.syntax.state.as_mut() {
                        state.reset(this.snapshot.clone());
                    }
                    this.request_reparse(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Tries to finish the parse here and now, within [`SYNC_PARSE_BUDGET`].
    ///
    /// Returns `true` when the tree is up to date afterwards (so nothing needs
    /// to go to the background) and `false` when the caller must spawn.
    fn reparse_now(&mut self) -> bool {
        let Some(state) = self.syntax.state.as_mut() else {
            return false;
        };
        match state.reparse_within(&CancelFlag::new(), SYNC_PARSE_BUDGET) {
            // Already current: nothing to do and nothing to bump.
            ParseOutcome::Unchanged => true,
            ParseOutcome::Parsed { .. } => {
                self.syntax.version += 1;
                self.syntax.stale = false;
                true
            }
            // No grammar: a background parse would not do better.
            ParseOutcome::NoGrammar => true,
            // Out of budget. The edits stay applied to the tree, so the
            // background parse resumes from where this one stopped.
            ParseOutcome::Cancelled => false,
        }
    }

    /// Version of the highlights. It moves only when a parse lands, so a host
    /// (or a test) can tell a frame painted with new colours from one that
    /// reused the ones it already had.
    pub fn highlight_version(&self) -> u64 {
        self.syntax.version
    }

    /// Highlight spans covering a byte range of the buffer, memoised per frame.
    ///
    /// This never comes back empty for text that was coloured a frame ago: when
    /// the tree is away on the background executor (or behind the buffer), the
    /// spans carried across the edit are used instead, clipped to `range`. The
    /// swap to the real result happens in one go, when the parse lands.
    pub(crate) fn highlights(&mut self, range: Range<usize>) -> &[(Range<usize>, HighlightId)] {
        if self.decorator.is_some() {
            self.refresh_decorations();
            self.decorated_spans = self
                .decorations
                .as_ref()
                .map(|(_, decorations)| {
                    decorations
                        .highlights
                        .iter()
                        .filter(|(span, _)| span.end > range.start && span.start < range.end)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            return &self.decorated_spans;
        }
        let key = (
            self.snapshot.version(),
            self.syntax.version,
            range.start,
            range.end,
        );
        if self.syntax.spans_key != key {
            let version = self.snapshot.version();
            let current = self
                .syntax
                .state
                .as_ref()
                .is_some_and(|state| state.parsed_version() == Some(version));
            // Querying the tree is right when it matches the buffer, and the
            // least bad option when the carried spans do not reach this far
            // (the user scrolled into unpainted text while a parse runs).
            let covered = self.syntax.carry.as_ref().is_some_and(|carry| {
                carry.range.start <= range.start && carry.range.end >= range.end
            });
            let fresh = (current || !covered)
                .then(|| {
                    self.syntax
                        .state
                        .as_ref()
                        .map(|state| state.highlights(range.clone()))
                })
                .flatten();
            self.syntax.spans = match fresh {
                Some(spans) if current => {
                    self.syntax.carry = Some(HighlightCarry {
                        range: range.clone(),
                        spans: spans.clone(),
                    });
                    spans
                }
                Some(spans) => spans,
                None => carried_spans(self.syntax.carry.as_ref(), &range),
            };
            self.syntax.spans_key = key;
        }
        &self.syntax.spans
    }

    /// Highlights of one phantom row, parsed on demand: the deleted text of a
    /// hunk is small, so it is parsed on the UI thread the first time it shows.
    pub(crate) fn phantom_highlights(
        &mut self,
        hunk_ix: usize,
        line_ix: usize,
    ) -> Vec<(Range<usize>, HighlightId)> {
        let Some(key) = self.phantom_keys.get(hunk_ix).copied() else {
            return Vec::new();
        };
        if !self.phantom_spans.contains_key(&key) {
            let spans = self.parse_phantom(hunk_ix);
            self.phantom_spans.insert(key, spans);
        }
        self.phantom_spans
            .get(&key)
            .and_then(|lines| lines.get(line_ix))
            .cloned()
            .unwrap_or_default()
    }

    fn parse_phantom(&self, hunk_ix: usize) -> Vec<Vec<(Range<usize>, HighlightId)>> {
        let Some(hunk) = self.display_map.diff().hunks().get(hunk_ix) else {
            return Vec::new();
        };
        let empty = vec![Vec::new(); hunk.deleted_text.len()];
        let Some(language) = self.language.clone() else {
            return empty;
        };
        let text = hunk.deleted_text.join("\n");
        let buffer = Buffer::new(&text);
        let mut state = SyntaxState::new(self.registry.clone(), language, buffer.snapshot());
        state.reparse(&CancelFlag::new());
        let spans = state.highlights(0..text.len());

        let mut per_line = vec![Vec::new(); hunk.deleted_text.len()];
        let mut line_start = 0usize;
        for (line_ix, line) in hunk.deleted_text.iter().enumerate() {
            let line_end = line_start + line.len();
            for (range, id) in &spans {
                let start = range.start.max(line_start);
                let end = range.end.min(line_end);
                if start < end {
                    per_line[line_ix].push((start - line_start..end - line_start, *id));
                }
            }
            line_start = line_end + 1;
        }
        per_line
    }
}

// -- shaped-line cache and render probe -------------------------------------

impl EditorView {
    /// Shapes one painted line, reusing the cached `ShapedLine` when the text,
    /// the font size and every run are exactly the ones it was shaped from.
    ///
    /// Returns `(line, reshaped)`: `reshaped` is `true` only when the shaper
    /// actually ran, which is what "a row that did not change is never shaped
    /// again" means in practice — a keystroke reshapes the edited row and
    /// nothing else, whatever moved in the rest of the document.
    pub(crate) fn shaped_line(
        &mut self,
        text: &str,
        runs: &[TextRun],
        font_size: Pixels,
        window: &Window,
    ) -> (ShapedLine, bool) {
        let signature: Vec<(usize, Hsla)> = runs.iter().map(|run| (run.len, run.color)).collect();
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        // The code and the interface share the cache, so the family is part of
        // the key (a font *change* still clears the cache, see `set_settings`).
        if let Some(run) = runs.first() {
            run.font.family.hash(&mut hasher);
        }
        f32::from(font_size).to_bits().hash(&mut hasher);
        signature.hash(&mut hasher);
        let key = hasher.finish();

        let frame = self.frame_counter;
        if let Some(cached) = self.line_cache.get_mut(&key)
            && cached.text.as_ref() == text
            && cached.runs == signature
        {
            cached.used_frame = frame;
            return (cached.line.clone(), false);
        }

        let text: SharedString = text.to_string().into();
        let line = window
            .text_system()
            .shape_line(text.clone(), font_size, runs, None);
        self.line_cache.insert(
            key,
            CachedLine {
                line: line.clone(),
                text,
                runs: signature,
                used_frame: frame,
            },
        );
        (line, true)
    }

    /// Closes a painted frame: ages the shaped-line cache and, when the render
    /// probe is on, records what the frame painted.
    pub(crate) fn end_frame(
        &mut self,
        rows: Vec<RowRender>,
        mut review: ReviewFrame,
        geometry: FrameGeometry,
        placeholder: bool,
        git_marks: Vec<(crate::git_gutter::GitGutterKind, Bounds<Pixels>)>,
    ) {
        if self.render_probe {
            if self.render_frames.len() >= RENDER_FRAME_HISTORY {
                self.render_frames.pop_front();
            }
            review.hunks = self.review.hunks.iter().map(|hunk| hunk.id).collect();
            self.render_frames.push_back(FrameRender {
                text_version: self.snapshot.version(),
                highlight_version: self.syntax.version,
                rows,
                review,
                gutter_width: geometry.gutter_width,
                text_origin_x: geometry.text_origin_x,
                placeholder,
                bounds: geometry.bounds,
                text_width: geometry.text_width,
                wrap_rows: self.wrap_row_count(),
                paint_clip: None,
                git_marks,
            });
        }
        self.frame_counter += 1;
        if self.line_cache.len() > LINE_CACHE_MAX {
            let frame = self.frame_counter;
            self.line_cache
                .retain(|_, cached| cached.used_frame + LINE_CACHE_KEEP >= frame);
        }
    }

    /// Records the content mask the last probed frame was painted under.
    pub(crate) fn record_paint_clip(&mut self, clip: Bounds<Pixels>) {
        if let Some(frame) = self.render_frames.back_mut() {
            frame.paint_clip = Some(clip);
        }
    }

    /// Whether the render probe is collecting frames.
    pub fn render_probe(&self) -> bool {
        self.render_probe
    }

    /// Turns the render probe on or off. Off by default: while it is on the
    /// view keeps a [`FrameRender`] for each of the last 64 painted frames,
    /// which is what a test inspects to prove that no frame dropped the
    /// highlights of a row.
    pub fn set_render_probe(&mut self, on: bool) {
        self.render_probe = on;
        if !on {
            self.render_frames.clear();
        }
    }

    /// The frames recorded since the probe was turned on (or since the last
    /// [`EditorView::take_render_frames`]), oldest first.
    pub fn render_frames(&self) -> Vec<FrameRender> {
        self.render_frames.iter().cloned().collect()
    }

    /// Same, and forgets them.
    pub fn take_render_frames(&mut self) -> Vec<FrameRender> {
        self.render_frames.drain(..).collect()
    }
}

// -- display helpers --------------------------------------------------------

/// Adapter that lets the wrap map read the display rows.
struct RowSource<'a> {
    view: &'a EditorView,
}

impl WrapSource for RowSource<'_> {
    fn row_count(&self) -> u32 {
        self.view.display_map.display_row_count()
    }

    fn row_len(&self, row: DisplayRow) -> u32 {
        match self.view.display_map.to_buffer(row) {
            DisplayCell::Buffer(buffer_row) => self.view.snapshot.line_len(buffer_row),
            DisplayCell::Phantom { hunk_ix, line_ix } => {
                self.view.display_map.diff().hunks()[hunk_ix].deleted_text[line_ix].len() as u32
            }
        }
    }

    fn row_text(&self, row: DisplayRow) -> String {
        self.view.display_row_source(row)
    }
}

/// A [`WrapSource`] that measures real glyph advances, for a view that paints
/// its prose rows in a proportional font.
struct MeasuredRowSource<'a> {
    view: &'a EditorView,
    window: &'a Window,
}

impl WrapSource for MeasuredRowSource<'_> {
    fn row_count(&self) -> u32 {
        self.view.display_map.display_row_count()
    }

    fn row_len(&self, row: DisplayRow) -> u32 {
        RowSource { view: self.view }.row_len(row)
    }

    fn row_text(&self, row: DisplayRow) -> String {
        self.view.display_row_source(row)
    }

    fn is_measured(&self) -> bool {
        true
    }

    fn char_widths(&self, row: DisplayRow, text: &str) -> Option<Vec<u32>> {
        let view = self.view;
        let monospace = match view.display_map.to_buffer(row) {
            DisplayCell::Buffer(buffer_row) => view
                .decorations
                .as_ref()
                .is_some_and(|(_, decorations)| decorations.is_monospace(buffer_row)),
            DisplayCell::Phantom { .. } => true,
        };
        let font = if monospace {
            view.style.font.clone()
        } else {
            view.style
                .prose_font
                .clone()
                .unwrap_or(view.style.font.clone())
        };
        let font_size = view.style.font_size;
        // A tab and a space are both one byte, so shaping the text with its
        // tabs as spaces keeps every byte offset where it was.
        let painted: String = text
            .chars()
            .map(|ch| if ch == '\t' { ' ' } else { ch })
            .collect();
        let run = TextRun {
            len: painted.len(),
            font,
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs: &[TextRun] = if painted.is_empty() {
            &[]
        } else {
            std::slice::from_ref(&run)
        };
        let line = self.window.text_system().shape_line(
            SharedString::from(painted.clone()),
            font_size,
            runs,
            None,
        );
        let tab = view.settings.tab_size.max(1) as f32;
        let mut widths = Vec::with_capacity(text.len());
        for (byte, ch) in text.char_indices() {
            let next = byte + ch.len_utf8();
            let mut advance = f32::from(line.x_for_index(next) - line.x_for_index(byte));
            if ch == '\t' {
                advance *= tab;
            }
            widths.push((advance * WRAP_UNITS_PER_PX).round().max(1.) as u32);
        }
        Some(widths)
    }
}

impl EditorView {
    /// The source text of a display row (no tab expansion).
    pub fn display_row_source(&self, row: DisplayRow) -> String {
        match self.display_map.to_buffer(row) {
            DisplayCell::Buffer(buffer_row) => self.snapshot.line_text(buffer_row),
            DisplayCell::Phantom { hunk_ix, line_ix } => self
                .display_map
                .diff()
                .hunks()
                .get(hunk_ix)
                .and_then(|hunk| hunk.deleted_text.get(line_ix))
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// The text of a display row, with tabs expanded for painting.
    pub fn display_row_text(&self, row: DisplayRow) -> RowText {
        RowText::expand(self.display_row_source(row), self.settings.tab_size)
    }

    /// Brings the wrap map up to date for a text column `text_width` wide:
    /// monospace columns of `char_width`, or measured glyph advances (in
    /// [`WRAP_UNITS_PER_PX`]) when a prose font is set. Rebuilds only when the
    /// width, the text or the tab size changed.
    pub(crate) fn ensure_wrap(&mut self, text_width: Pixels, char_width: Pixels, window: &Window) {
        let measured = self.style.prose_font.is_some();
        let columns = self.soft_wrap().then(|| {
            if measured {
                ((f32::from(text_width) * WRAP_UNITS_PER_PX).floor() as u32).max(1)
            } else {
                ((f32::from(text_width) / f32::from(char_width).max(1.)).floor() as u32).max(1)
            }
        });
        // The wrap map has its own epoch: an edit that stayed inside one row
        // already updated it, so only a real change (the viewport width, the
        // tab size, a row added or removed) rebuilds it.
        if self.wrap.columns() == columns && self.wrap_epoch == self.wrap_state() {
            return;
        }
        if measured && columns.is_some() {
            // Row fonts depend on the decorations of this text version.
            self.refresh_decorations();
            let mut wrap = std::mem::take(&mut self.wrap);
            wrap.rebuild(
                columns,
                self.settings.tab_size,
                &MeasuredRowSource { view: self, window },
            );
            self.wrap = wrap;
            self.wrap_epoch = self.wrap_state();
        } else {
            self.rebuild_wrap(columns);
        }
    }

    /// X offset of the continuation segments of a soft-wrapped row.
    pub(crate) fn wrap_indent_x(&self, row: DisplayRow, char_width: Pixels) -> Pixels {
        let indent = self.wrap.indent(row) as f32;
        if self.style.prose_font.is_some() {
            px(indent / WRAP_UNITS_PER_PX)
        } else {
            char_width * indent
        }
    }

    /// Rebuilds the wrap map for a viewport of `columns` characters.
    pub(crate) fn rebuild_wrap(&mut self, columns: Option<u32>) {
        let mut wrap = std::mem::take(&mut self.wrap);
        wrap.rebuild(columns, self.settings.tab_size, &RowSource { view: self });
        self.wrap = wrap;
        self.wrap_epoch = self.wrap_state();
    }

    /// The state the wrap map must match: rebuilding it is the only way to get
    /// there, except for the single-row fast path below.
    pub(crate) fn wrap_state(&self) -> (u64, u32, u32, u32) {
        (
            self.snapshot.version(),
            self.wrap.columns().unwrap_or(0),
            self.settings.tab_size,
            self.display_map.display_row_count(),
        )
    }

    /// Re-measures one display row instead of the whole file. Used after an
    /// edit that stayed inside a single row and did not change the row count,
    /// which is what typing looks like.
    fn rewrap_row(&mut self, row: DisplayRow) {
        let mut wrap = std::mem::take(&mut self.wrap);
        wrap.update_row(row, &RowSource { view: self });
        self.wrap = wrap;
        self.wrap_epoch = self.wrap_state();
    }

    fn rebuild_display_map(&mut self) {
        // Both sorts are stable and use the same key, so the review hunks and
        // the display-map hunks keep the same indices.
        self.review.hunks.sort_by_key(|hunk| hunk.buffer_rows.start);
        let phantoms: Vec<PhantomHunk> = self
            .review
            .hunks
            .iter()
            .map(ReviewHunkView::to_phantom)
            .collect();
        self.display_map = DisplayMap::new(self.snapshot.line_count(), phantoms);
        self.refresh_review_caches();
        // The wrap map is rebuilt by the element, which is the only place that
        // knows the viewport width; until then it is one version behind, which
        // only costs an approximate vertical movement.
        self.cursor = self.clip_point(self.cursor);
        self.selection_anchor = self.clip_point(self.selection_anchor);
        // The phantom rows may have changed with the map: the search follows.
        self.research();
    }

    /// Recomputes the search matches (buffer and phantom rows) from the
    /// cursor, keeping the current match when it survives. Does nothing while
    /// there is no query.
    pub(crate) fn research(&mut self) {
        if self.search.query.is_empty() {
            return;
        }
        let from = self.edit_offset(self.cursor);
        self.recompute_search(from);
    }

    /// Recomputes the search matches over the buffer and the phantom rows of
    /// the display map, selecting the first one at or after `from` when the
    /// current one is gone.
    fn recompute_search(&mut self, from: usize) {
        let text = self.snapshot.text();
        let len = text.len();
        let line_count = self.snapshot.line_count();
        let snapshot = &self.snapshot;
        let phantoms: Vec<PhantomLines<'_>> = self
            .display_map
            .diff()
            .hunks()
            .iter()
            .map(|hunk| PhantomLines {
                insert_offset: if hunk.insert_before_buffer_row < line_count {
                    snapshot.line_start_offset(hunk.insert_before_buffer_row)
                } else {
                    len
                },
                lines: &hunk.deleted_text,
            })
            .collect();
        self.search
            .refresh_with_phantoms(&text, &phantoms, from.min(len));
    }

    /// Version of the text the view is showing.
    pub fn text_version(&self) -> u64 {
        self.snapshot.version()
    }

    /// Byte offset of the first byte of a buffer row.
    pub(crate) fn snapshot_line_start(&self, buffer_row: u32) -> usize {
        self.snapshot.line_start_offset(buffer_row)
    }

    /// Buffer byte range covered by a range of wrap rows, for the highlight
    /// query: only the visible rows are ever asked for.
    pub(crate) fn visible_byte_range(&self, first: WrapRow, last: WrapRow) -> (usize, usize) {
        if last <= first {
            return (0, 0);
        }
        let (first_display, _) = self.wrap.to_display(first);
        let (last_display, _) = self.wrap.to_display(last - 1);
        let diff = self.display_map.diff();
        let start_row = diff.edit_target_row(first_display);
        let end_row = diff
            .edit_target_row(last_display)
            .min(self.snapshot.line_count().saturating_sub(1));
        let start = self.snapshot.line_start_offset(start_row);
        let end =
            self.snapshot.line_start_offset(end_row) + self.snapshot.line_len(end_row) as usize;
        (start.min(end), end)
    }

    /// The diff layer.
    pub fn diff(&self) -> &DiffTransformMap {
        self.display_map.diff()
    }

    /// Clamps a display point to a valid position on a `char` boundary.
    pub fn clip_point(&self, point: DisplayPoint) -> DisplayPoint {
        let row = self.display_map.clip_row(point.row);
        let text = self.display_row_source(row);
        let mut column = (point.column as usize).min(text.len());
        while column > 0 && !text.is_char_boundary(column) {
            column -= 1;
        }
        DisplayPoint::new(row, column as u32)
    }

    /// The ordered selection in display space.
    pub fn selection_range(&self) -> Range<DisplayPoint> {
        if self.selection_anchor <= self.cursor {
            self.selection_anchor..self.cursor
        } else {
            self.cursor..self.selection_anchor
        }
    }

    /// Whether anything is selected.
    pub fn has_selection(&self) -> bool {
        self.selection_anchor != self.cursor
    }

    /// The selected text, phantom rows included.
    pub fn selected_text(&self) -> String {
        let range = self.selection_range();
        if range.start == range.end {
            return String::new();
        }
        let mut text = String::new();
        for row in range.start.row..=range.end.row {
            let line = self.display_row_source(row);
            let start = if row == range.start.row {
                (range.start.column as usize).min(line.len())
            } else {
                0
            };
            let end = if row == range.end.row {
                (range.end.column as usize).min(line.len())
            } else {
                line.len()
            };
            text.push_str(&line[start.min(end)..end]);
            if row != range.end.row {
                text.push('\n');
            }
        }
        text
    }

    /// Buffer offset an edit at `point` must be applied to. Phantom rows are
    /// read-only: edits are redirected to the start of the next real row.
    pub(crate) fn edit_offset(&self, point: DisplayPoint) -> usize {
        match self.display_map.to_buffer(point.row) {
            DisplayCell::Buffer(buffer_row) => self
                .snapshot
                .point_to_offset(Point::new(buffer_row, point.column)),
            DisplayCell::Phantom { hunk_ix, .. } => {
                let row = self
                    .display_map
                    .diff()
                    .hunks()
                    .get(hunk_ix)
                    .map(|hunk| hunk.insert_before_buffer_row)
                    .unwrap_or(0);
                self.snapshot.point_to_offset(Point::new(row, 0))
            }
        }
    }

    /// The buffer range an edit must replace (the selection, redirected).
    pub(crate) fn edit_range(&self) -> Range<usize> {
        let range = self.selection_range();
        let start = self.edit_offset(range.start);
        let end = self.edit_offset(range.end);
        start.min(end)..start.max(end)
    }

    /// Display position of a buffer offset.
    pub(crate) fn display_point_for_offset(&self, offset: usize) -> DisplayPoint {
        let point = self.snapshot.offset_to_point(offset);
        DisplayPoint::new(self.display_map.to_display_row(point.row), point.column)
    }

    // -- wrap-row space ----------------------------------------------------

    /// The wrap row of a display point, and the column inside its segment
    /// (counted in characters of the painted text).
    pub(crate) fn point_to_wrap(&self, point: DisplayPoint) -> (WrapRow, u32) {
        let row_text = self.display_row_text(point.row);
        let segment = self.wrap.segment_for_byte(point.row, point.column);
        let range = self.wrap.segment_range(point.row, segment, row_text.len());
        let from = row_text.to_display(range.start) as usize;
        let to = row_text.to_display(point.column) as usize;
        let text = row_text.text();
        let column = text[from.min(text.len())..to.min(text.len())]
            .chars()
            .count() as u32;
        (self.wrap.to_wrap_row(point.row, segment), column)
    }

    /// The display point at `column` characters into a wrap row.
    pub(crate) fn wrap_to_point(&self, wrap_row: WrapRow, column: u32) -> DisplayPoint {
        let (row, segment) = self.wrap.to_display(wrap_row);
        let row_text = self.display_row_text(row);
        let range = self.wrap.segment_range(row, segment, row_text.len());
        let from = row_text.to_display(range.start) as usize;
        let to = row_text.to_display(range.end) as usize;
        let text = row_text.text();
        let slice = &text[from.min(text.len())..to.min(text.len())];
        let offset = slice
            .char_indices()
            .nth(column as usize)
            .map(|(byte, _)| byte)
            .unwrap_or(slice.len());
        let source = row_text.to_source((from + offset) as u32);
        DisplayPoint::new(row, row_text.clip(source))
    }

    /// The wrap row the cursor is on.
    pub fn cursor_wrap_row(&self) -> WrapRow {
        self.point_to_wrap(self.cursor).0
    }

    /// Number of wrap rows.
    pub fn wrap_row_count(&self) -> WrapRow {
        if self.wrap.is_enabled() {
            self.wrap.wrap_row_count().max(1)
        } else {
            self.display_map.display_row_count().max(1)
        }
    }

    fn page_rows(&self) -> u32 {
        self.layout
            .as_ref()
            .map(|layout| layout.visible_row_count.max(1.) as u32)
            .unwrap_or(20)
            .max(1)
    }
}

// -- editing ----------------------------------------------------------------

impl EditorView {
    /// Replaces the selection with `text` (the path taken by typing and IME).
    pub fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let range = self.edit_range();
        self.replace_buffer_range(range, text, cx);
    }

    pub(crate) fn replace_buffer_range(
        &mut self,
        range: Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let rows_before = self.snapshot.line_count();
        let edit_row = self.snapshot.offset_to_point(range.start).row;
        if !self.auto_closers.is_empty() {
            self.shift_auto_closers(&[(range.clone(), text.to_string())]);
        }
        self.edit_buffer(&[(range.clone(), text)], cx);
        let delta = self.snapshot.line_count() as i64 - rows_before as i64;
        self.adjust_hunks(edit_row, delta);
        self.rebuild_display_map();
        // Fast path: an edit inside one row only changes that row's wrapping.
        if self.wrap.is_enabled()
            && self.style.prose_font.is_none()
            && delta == 0
            && !text.contains('\n')
            && self.wrap.row_count() == self.display_map.display_row_count()
        {
            let row = self.display_map.to_display_row(edit_row);
            self.rewrap_row(row);
        }
        let offset = range.start + text.len();
        self.cursor = self.display_point_for_offset(offset);
        self.selection_anchor = self.cursor;
        self.marked_range = None;
        self.goal_column = None;
        self.after_input(cx);
    }

    /// Applies edits inside one `EditSource::User` transaction and re-syncs.
    fn edit_buffer(&mut self, edits: &[(Range<usize>, &str)], cx: &mut Context<Self>) {
        let (snapshot, events) = {
            let mut buffer = self.buffer.lock();
            buffer.transact(EditSource::User, |buffer| {
                // Applied back to front so earlier offsets stay valid.
                for (range, text) in edits.iter().rev() {
                    buffer.replace(range.clone(), text);
                }
            });
            (buffer.snapshot(), buffer.drain_events())
        };
        self.snapshot = snapshot;
        self.apply_syntax_events(&events);
        self.request_reparse(cx);
        self.invalidate_layout();
        // The search matches are recomputed by `rebuild_display_map`, which
        // every caller runs next, once the hunks (and so the phantom rows)
        // have followed the edit.
        self.update_dirty(cx);
    }

    /// Keeps the hunk rows aligned after an edit, until the host sends the
    /// refreshed review: hunks below the edited row slide with it, and a hunk
    /// whose added rows contain it grows or shrinks.
    fn adjust_hunks(&mut self, edit_row: u32, delta: i64) {
        if delta == 0 {
            return;
        }
        let shift = |row: u32| -> u32 { (row as i64 + delta).max(0) as u32 };
        for hunk in &mut self.review.hunks {
            let rows = hunk.buffer_rows.clone();
            if rows.start > edit_row {
                hunk.buffer_rows = shift(rows.start)..shift(rows.end);
            } else if rows.contains(&edit_row) {
                hunk.buffer_rows = rows.start..shift(rows.end).max(rows.start);
            } else {
                continue;
            }
            for line in &mut hunk.lines {
                if let Some(row) = line.buffer_row.as_mut()
                    && *row > edit_row
                {
                    *row = shift(*row);
                }
            }
        }
    }

    /// Everything that must happen after the user typed or moved.
    fn after_input(&mut self, cx: &mut Context<Self>) {
        // The user took over: a jump waiting for the host's answer is void.
        self.pending_jump = None;
        self.last_input = Instant::now();
        self.blink_visible = true;
        self.autoscroll = true;
        // A plain move only brings the cursor into view; a jump to a change
        // asks for the centring again right after this.
        self.autoscroll_center = false;
        cx.emit(EditorEvent::CursorMoved {
            point: self.cursor_point(),
        });
        cx.notify();
    }

    fn move_cursor(&mut self, point: DisplayPoint, extend: bool, cx: &mut Context<Self>) {
        self.auto_closers.clear();
        self.cursor = self.clip_point(point);
        if !extend {
            self.selection_anchor = self.cursor;
        }
        self.goal_column = None;
        self.after_input(cx);
    }

    fn move_vertically(&mut self, rows: i64, extend: bool, cx: &mut Context<Self>) {
        self.auto_closers.clear();
        let (wrap_row, column) = self.point_to_wrap(self.cursor);
        let goal = self.goal_column.unwrap_or(column);
        let target = (wrap_row as i64 + rows).clamp(0, self.wrap_row_count() as i64 - 1) as WrapRow;
        self.cursor = self.clip_point(self.wrap_to_point(target, goal));
        if !extend {
            self.selection_anchor = self.cursor;
        }
        self.goal_column = Some(goal);
        self.after_input(cx);
    }

    // -- word boundaries ---------------------------------------------------

    fn point_left(&self, point: DisplayPoint) -> DisplayPoint {
        if point.column > 0 {
            let text = self.display_row_source(point.row);
            let mut column = (point.column as usize - 1).min(text.len());
            while column > 0 && !text.is_char_boundary(column) {
                column -= 1;
            }
            DisplayPoint::new(point.row, column as u32)
        } else if point.row > 0 {
            let row = point.row - 1;
            DisplayPoint::new(row, self.display_row_source(row).len() as u32)
        } else {
            point
        }
    }

    fn point_right(&self, point: DisplayPoint) -> DisplayPoint {
        let text = self.display_row_source(point.row);
        if (point.column as usize) < text.len() {
            let mut column = point.column as usize + 1;
            while column < text.len() && !text.is_char_boundary(column) {
                column += 1;
            }
            DisplayPoint::new(point.row, column as u32)
        } else if point.row + 1 < self.display_map.display_row_count() {
            DisplayPoint::new(point.row + 1, 0)
        } else {
            point
        }
    }

    fn word_left(&self, point: DisplayPoint) -> DisplayPoint {
        if point.column == 0 {
            return self.point_left(point);
        }
        let text = self.display_row_source(point.row);
        let column = prev_word_boundary(&text, point.column as usize);
        DisplayPoint::new(point.row, column as u32)
    }

    fn word_right(&self, point: DisplayPoint) -> DisplayPoint {
        let text = self.display_row_source(point.row);
        if point.column as usize >= text.len() {
            return self.point_right(point);
        }
        let column = next_word_boundary(&text, point.column as usize);
        DisplayPoint::new(point.row, column as u32)
    }

    /// The word around a display point, for the double click.
    pub(crate) fn word_range_at(&self, point: DisplayPoint) -> Range<DisplayPoint> {
        let text = self.display_row_source(point.row);
        let (start, end) = word_at(&text, point.column as usize);
        DisplayPoint::new(point.row, start as u32)..DisplayPoint::new(point.row, end as u32)
    }

    /// The whole row, for the triple click.
    pub(crate) fn row_range_at(&self, point: DisplayPoint) -> Range<DisplayPoint> {
        let len = self.display_row_source(point.row).len() as u32;
        DisplayPoint::new(point.row, 0)..DisplayPoint::new(point.row, len)
    }

    /// The indentation string of a display row.
    fn row_indent(&self, row: DisplayRow) -> String {
        let text = self.display_row_source(row);
        text.chars()
            .take_while(|ch| *ch == ' ' || *ch == '\t')
            .collect()
    }

    /// One indent level, preferring what the file already uses.
    fn indent_unit(&self) -> String {
        match self.detected_indent {
            Some(indent) if indent.insert_spaces => " ".repeat(indent.size.max(1) as usize),
            Some(_) => "\t".to_string(),
            None => self.settings.indent_unit(),
        }
    }
}

/// Character class used by the word movement.
fn class_of(ch: char) -> u8 {
    if ch.is_whitespace() {
        0
    } else if ch.is_alphanumeric() || ch == '_' {
        1
    } else {
        2
    }
}

/// Byte offset of the previous word boundary inside one line.
pub fn prev_word_boundary(text: &str, from: usize) -> usize {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut ix = chars.partition_point(|(byte, _)| *byte < from);
    if ix == 0 {
        return 0;
    }
    ix -= 1;
    while ix > 0 && class_of(chars[ix].1) == 0 {
        ix -= 1;
    }
    let class = class_of(chars[ix].1);
    while ix > 0 && class_of(chars[ix - 1].1) == class {
        ix -= 1;
    }
    chars[ix].0
}

/// Byte offset of the next word boundary inside one line.
pub fn next_word_boundary(text: &str, from: usize) -> usize {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut ix = chars.partition_point(|(byte, _)| *byte < from);
    if ix >= chars.len() {
        return text.len();
    }
    let class = class_of(chars[ix].1);
    if class == 0 {
        while ix < chars.len() && class_of(chars[ix].1) == 0 {
            ix += 1;
        }
    } else {
        while ix < chars.len() && class_of(chars[ix].1) == class {
            ix += 1;
        }
    }
    chars.get(ix).map(|(byte, _)| *byte).unwrap_or(text.len())
}

/// The word around a byte offset inside one line.
pub fn word_at(text: &str, at: usize) -> (usize, usize) {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let mut ix = chars.partition_point(|(byte, _)| *byte < at);
    if ix >= chars.len() {
        ix = chars.len() - 1;
    }
    let class = class_of(chars[ix].1);
    let mut start = ix;
    while start > 0 && class_of(chars[start - 1].1) == class {
        start -= 1;
    }
    let mut end = ix;
    while end < chars.len() && class_of(chars[end].1) == class {
        end += 1;
    }
    let end_byte = chars.get(end).map(|(byte, _)| *byte).unwrap_or(text.len());
    (chars[start].0, end_byte)
}

// -- actions ----------------------------------------------------------------

impl EditorView {
    fn on_move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        let point = if self.has_selection() {
            self.selection_range().start
        } else {
            self.point_left(self.cursor)
        };
        self.move_cursor(point, false, cx);
    }

    fn on_move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        let point = if self.has_selection() {
            self.selection_range().end
        } else {
            self.point_right(self.cursor)
        };
        self.move_cursor(point, false, cx);
    }

    fn on_move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1, false, cx);
    }

    fn on_move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1, false, cx);
    }

    fn on_select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1, true, cx);
    }

    fn on_select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1, true, cx);
    }

    fn on_select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let point = self.point_left(self.cursor);
        self.move_cursor(point, true, cx);
    }

    fn on_select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let point = self.point_right(self.cursor);
        self.move_cursor(point, true, cx);
    }

    fn on_move_word_left(&mut self, _: &MoveWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let point = self.word_left(self.cursor);
        self.move_cursor(point, false, cx);
    }

    fn on_move_word_right(&mut self, _: &MoveWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let point = self.word_right(self.cursor);
        self.move_cursor(point, false, cx);
    }

    fn on_select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let point = self.word_left(self.cursor);
        self.move_cursor(point, true, cx);
    }

    fn on_select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let point = self.word_right(self.cursor);
        self.move_cursor(point, true, cx);
    }

    /// Home goes to the first non-blank column first, then to column 0.
    fn line_start(&self, point: DisplayPoint) -> DisplayPoint {
        let text = self.display_row_source(point.row);
        let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
        let column = if point.column as usize == indent {
            0
        } else {
            indent
        };
        DisplayPoint::new(point.row, column as u32)
    }

    fn on_move_to_line_start(
        &mut self,
        _: &MoveToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let point = self.line_start(self.cursor);
        self.move_cursor(point, false, cx);
    }

    fn on_move_to_line_end(&mut self, _: &MoveToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(DisplayPoint::new(self.cursor.row, u32::MAX), false, cx);
    }

    fn on_select_to_line_start(
        &mut self,
        _: &SelectToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let point = self.line_start(self.cursor);
        self.move_cursor(point, true, cx);
    }

    fn on_select_to_line_end(
        &mut self,
        _: &SelectToLineEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_cursor(DisplayPoint::new(self.cursor.row, u32::MAX), true, cx);
    }

    fn on_move_page_up(&mut self, _: &MovePageUp, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.page_rows() as i64;
        self.move_vertically(-rows, false, cx);
    }

    fn on_move_page_down(&mut self, _: &MovePageDown, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.page_rows() as i64;
        self.move_vertically(rows, false, cx);
    }

    fn on_select_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.page_rows() as i64;
        self.move_vertically(-rows, true, cx);
    }

    fn on_select_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.page_rows() as i64;
        self.move_vertically(rows, true, cx);
    }

    fn on_move_to_document_start(
        &mut self,
        _: &MoveToDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_cursor(DisplayPoint::new(0, 0), false, cx);
    }

    fn on_move_to_document_end(
        &mut self,
        _: &MoveToDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = self.display_map.display_row_count().saturating_sub(1);
        self.move_cursor(DisplayPoint::new(row, u32::MAX), false, cx);
    }

    fn on_select_to_document_start(
        &mut self,
        _: &SelectToDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_cursor(DisplayPoint::new(0, 0), true, cx);
    }

    fn on_select_to_document_end(
        &mut self,
        _: &SelectToDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = self.display_map.display_row_count().saturating_sub(1);
        self.move_cursor(DisplayPoint::new(row, u32::MAX), true, cx);
    }

    fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let last_row = self.display_map.display_row_count().saturating_sub(1);
        self.auto_closers.clear();
        self.selection_anchor = DisplayPoint::new(0, 0);
        self.cursor = self.clip_point(DisplayPoint::new(last_row, u32::MAX));
        self.after_input(cx);
    }

    fn on_backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(Prompt::GoToLine(digits)) = self.prompt.as_mut() {
            digits.pop();
            cx.notify();
            return;
        }
        if self.search_open {
            self.search.set_status(None);
            if self.replace_field_active() {
                self.search.replacement.pop();
                cx.notify();
            } else {
                self.search.query.pop();
                self.refresh_search(cx);
            }
            return;
        }
        if self.backspace_pair(cx) {
            return;
        }
        let range = self.edit_range();
        let range = if range.start == range.end {
            self.snapshot.previous_char_boundary(range.start)..range.end
        } else {
            range
        };
        if range.start == range.end {
            return;
        }
        self.replace_buffer_range(range, "", cx);
    }

    fn on_delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.edit_range();
        let range = if range.start == range.end {
            range.start..self.snapshot.next_char_boundary(range.end)
        } else {
            range
        };
        if range.start == range.end {
            return;
        }
        self.replace_buffer_range(range, "", cx);
    }

    fn on_delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.has_selection() {
            let range = self.edit_range();
            self.replace_buffer_range(range, "", cx);
            return;
        }
        let end = self.edit_offset(self.cursor);
        let start = self.edit_offset(self.word_left(self.cursor));
        if start >= end {
            let start = self.snapshot.previous_char_boundary(end);
            if start < end {
                self.replace_buffer_range(start..end, "", cx);
            }
            return;
        }
        self.replace_buffer_range(start..end, "", cx);
    }

    fn on_delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.has_selection() {
            let range = self.edit_range();
            self.replace_buffer_range(range, "", cx);
            return;
        }
        let start = self.edit_offset(self.cursor);
        let end = self.edit_offset(self.word_right(self.cursor));
        if end <= start {
            let end = self.snapshot.next_char_boundary(start);
            if end > start {
                self.replace_buffer_range(start..end, "", cx);
            }
            return;
        }
        self.replace_buffer_range(start..end, "", cx);
    }

    fn on_insert_newline(&mut self, _: &InsertNewline, _: &mut Window, cx: &mut Context<Self>) {
        if self.prompt.is_some() {
            self.confirm_prompt(cx);
            return;
        }
        if self.newline_in_pair(cx) {
            return;
        }
        let row = self.selection_range().start.row;
        let mut text = String::from("\n");
        text.push_str(&self.row_indent(row));
        // An opening brace at the end of the row deepens the indentation.
        let source = self.display_row_source(row);
        let trimmed = source[..(self.cursor.column as usize).min(source.len())].trim_end();
        if trimmed.ends_with(['{', '(', '[', ':']) {
            text.push_str(&self.indent_unit());
        }
        self.insert_text(&text, cx);
    }

    /// Display rows the current selection indents, following the usual rule
    /// that a selection ending at column 0 does not include that row.
    fn selected_rows(&self) -> Range<DisplayRow> {
        let range = self.selection_range();
        let end = if range.end.column == 0 && range.end.row > range.start.row {
            range.end.row
        } else {
            range.end.row + 1
        };
        range.start.row..end
    }

    fn on_tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.selected_rows();
        if rows.end - rows.start > 1 {
            self.indent_rows(rows, true, cx);
            return;
        }
        let unit = self.indent_unit();
        self.insert_text(&unit, cx);
    }

    fn on_backtab(&mut self, _: &Backtab, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.selected_rows();
        self.indent_rows(rows, false, cx);
    }

    /// Adds or removes one indent level on a range of display rows.
    fn indent_rows(&mut self, rows: Range<DisplayRow>, add: bool, cx: &mut Context<Self>) {
        let unit = self.indent_unit();
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        for display_row in rows.clone() {
            let DisplayCell::Buffer(buffer_row) = self.display_map.to_buffer(display_row) else {
                continue;
            };
            let start = self.snapshot.line_start_offset(buffer_row);
            let text = self.snapshot.line_text(buffer_row);
            if add {
                // Empty rows get no trailing whitespace.
                if !text.is_empty() {
                    edits.push((start..start, unit.clone()));
                }
            } else {
                let removed = if text.starts_with(&unit) {
                    unit.len()
                } else if text.starts_with('\t') {
                    1
                } else {
                    text.len() - text.trim_start_matches(' ').len()
                }
                .min(unit.len().max(1));
                if removed > 0 {
                    edits.push((start..start + removed, String::new()));
                }
            }
        }
        if edits.is_empty() {
            return;
        }
        // The selection keeps covering the same text: a start at column 0
        // stays there, so every indented row is still selected whole.
        let (anchor, cursor) = self.map_selection(&edits);
        self.apply_edits(edits, anchor, cursor, cx);
    }

    fn on_undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let restored = {
            let mut buffer = self.buffer.lock();
            let restored = buffer.undo_with_ranges();
            buffer.drain_events();
            restored
        };
        self.after_history(restored.map(|(_, ranges)| ranges), cx);
    }

    fn on_redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let restored = {
            let mut buffer = self.buffer.lock();
            let restored = buffer.redo_with_ranges();
            buffer.drain_events();
            restored
        };
        self.after_history(restored.map(|(_, ranges)| ranges), cx);
    }

    fn after_history(&mut self, ranges: Option<Vec<Range<usize>>>, cx: &mut Context<Self>) {
        let Some(ranges) = ranges else {
            return;
        };
        self.snapshot = self.buffer.lock().snapshot();
        // Undo/redo can touch several places at once, so the tree is re-seeded
        // instead of trying to derive the intermediate snapshots. The carried
        // spans go with it: their offsets no longer mean anything.
        if let Some(state) = self.syntax.state.as_mut() {
            state.reset(self.snapshot.clone());
        } else {
            self.syntax.stale = true;
        }
        self.syntax.drop_carry();
        self.request_reparse(cx);
        self.rebuild_display_map();
        self.invalidate_layout();
        if let Some(range) = ranges.last() {
            self.cursor = self.display_point_for_offset(range.end);
            self.selection_anchor = self.cursor;
        }
        self.auto_closers.clear();
        self.research();
        self.update_dirty(cx);
        self.after_input(cx);
    }

    fn on_copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn on_cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let range = self.edit_range();
        if range.start != range.end {
            self.replace_buffer_range(range, "", cx);
        }
    }

    fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if self.search_open {
            // The bar has the keyboard: the clipboard goes to its active field.
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.type_into_search(&text, cx);
            }
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            // A multi-line paste takes the indentation of where it lands.
            let text = self.paste_text(&text);
            self.insert_text(&text, cx);
        }
    }

    fn on_save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(EditorEvent::SaveRequested);
    }

    fn on_toggle_soft_wrap(&mut self, _: &ToggleSoftWrap, _: &mut Window, cx: &mut Context<Self>) {
        let enabled = !self.soft_wrap();
        self.set_soft_wrap(enabled, cx);
    }

    /// Turns soft wrap on or off for this view only, as
    /// `editor::toggle_soft_wrap` (`Alt+Z`) does; the next settings change
    /// goes back to the configured value.
    pub fn set_soft_wrap(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.soft_wrap_override = Some(enabled);
        self.scroll_left = 0.;
        self.invalidate_layout();
        cx.notify();
    }

    fn on_toggle_whitespace(
        &mut self,
        _: &ToggleWhitespace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.show_whitespace = !self.settings.show_whitespace;
        cx.notify();
    }

    fn on_go_to_line(&mut self, _: &GoToLine, _: &mut Window, cx: &mut Context<Self>) {
        if self.settings.chrome == EditorChrome::Minimal {
            cx.propagate();
            return;
        }
        self.prompt = Some(Prompt::GoToLine(String::new()));
        cx.notify();
    }

    fn on_confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm_prompt(cx);
    }

    fn confirm_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(Prompt::GoToLine(digits)) = self.prompt.take() else {
            return;
        };
        if let Ok(line) = digits.trim().parse::<u32>() {
            let buffer_row = line
                .saturating_sub(1)
                .min(self.snapshot.line_count().saturating_sub(1));
            let row = self.display_map.to_display_row(buffer_row);
            self.cursor = self.clip_point(DisplayPoint::new(row, 0));
            self.selection_anchor = self.cursor;
        }
        self.after_input(cx);
    }

    fn on_cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.prompt.take().is_some() {
            cx.notify();
            return;
        }
        if self.search_open {
            self.search_open = false;
            self.search.clear();
            self.search.replace_open = false;
            self.search.field = SearchField::Find;
            cx.notify();
            return;
        }
        if self.has_selection() {
            self.selection_anchor = self.cursor;
            cx.notify();
        }
    }

    // -- search ------------------------------------------------------------

    fn on_find(&mut self, _: &Find, _: &mut Window, cx: &mut Context<Self>) {
        if self.settings.chrome == EditorChrome::Minimal {
            cx.propagate();
            return;
        }
        // `Ctrl+F` is the plain search bar, as before replace existed: from
        // replace mode it goes back to search only (the strip keeps its
        // height either way).
        self.search.replace_open = false;
        self.search.field = SearchField::Find;
        self.open_search_bar(cx);
    }

    /// `editor::find_replace` (`Ctrl+H`): opens the bar in replace mode with
    /// the keyboard in "Buscar…", or, when the bar was already open, moves the
    /// keyboard to "Reemplazar…".
    fn on_find_replace(&mut self, _: &FindReplace, _: &mut Window, cx: &mut Context<Self>) {
        if self.settings.chrome == EditorChrome::Minimal {
            cx.propagate();
            return;
        }
        let was_open = self.search_open;
        self.search.replace_open = true;
        if was_open {
            self.search.field = SearchField::Replace;
            self.search.set_status(None);
            cx.notify();
        } else {
            self.search.field = SearchField::Find;
            self.open_search_bar(cx);
        }
    }

    /// Opens the bar from the selection: a one-line selection becomes the
    /// query.
    fn open_search_bar(&mut self, cx: &mut Context<Self>) {
        self.search_open = true;
        self.search.set_status(None);
        self.search_from = self.edit_offset(self.selection_range().start);
        if self.has_selection() {
            let selected = self.selected_text();
            if !selected.contains('\n') && !selected.is_empty() {
                self.search.query = selected;
            }
        }
        self.refresh_search(cx);
    }

    pub(crate) fn refresh_search(&mut self, cx: &mut Context<Self>) {
        let from = self.search_from;
        self.recompute_search(from);
        if let Some(found) = self.search.current() {
            self.select_match(&found);
        }
        self.autoscroll = true;
        cx.notify();
    }

    /// Whether typed text goes to "Reemplazar…".
    fn replace_field_active(&self) -> bool {
        self.search.replace_open && self.search.field == SearchField::Replace
    }

    /// Text typed or pasted while the bar is open goes to its active field.
    fn type_into_search(&mut self, text: &str, cx: &mut Context<Self>) {
        self.search.set_status(None);
        if self.replace_field_active() {
            self.search.replacement.push_str(text);
            cx.notify();
        } else {
            self.search.query.push_str(text);
            self.refresh_search(cx);
        }
    }

    fn on_find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        if !self.search_open {
            self.search_open = true;
        }
        self.search.set_status(None);
        if let Some(found) = self.search.next_match() {
            self.search_from = self.search.position(&found);
            self.select_match(&found);
        }
        self.autoscroll = true;
        cx.notify();
    }

    fn on_find_prev(&mut self, _: &FindPrev, _: &mut Window, cx: &mut Context<Self>) {
        if !self.search_open {
            self.search_open = true;
        }
        self.search.set_status(None);
        if let Some(found) = self.search.previous_match() {
            self.search_from = self.search.position(&found);
            self.select_match(&found);
        }
        self.autoscroll = true;
        cx.notify();
    }

    fn on_search_next_field(
        &mut self,
        _: &SearchNextField,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch_search_field(cx);
    }

    fn on_search_prev_field(
        &mut self,
        _: &SearchPrevField,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch_search_field(cx);
    }

    /// `Tab` / `Shift+Tab` in the bar: with two fields they alternate; with
    /// one there is nowhere to go, and the key is swallowed all the same so
    /// it never indents the file.
    fn switch_search_field(&mut self, cx: &mut Context<Self>) {
        if !self.search_open {
            return;
        }
        if self.search.replace_open {
            self.search.field = match self.search.field {
                SearchField::Find => SearchField::Replace,
                SearchField::Replace => SearchField::Find,
            };
            cx.notify();
        }
    }

    fn on_replace_next(&mut self, _: &ReplaceNext, _: &mut Window, cx: &mut Context<Self>) {
        self.replace_next(cx);
    }

    fn on_replace_all(&mut self, _: &ReplaceAll, _: &mut Window, cx: &mut Context<Self>) {
        self.replace_all(cx);
    }

    /// "Reemplazar": replaces the current match (one undo step) and moves to
    /// the next one. A phantom match is never replaced (D12): the bar says so
    /// and the next real match becomes the current one, if there is any.
    pub(crate) fn replace_next(&mut self, cx: &mut Context<Self>) {
        if !self.search_open || !self.search.replace_open || self.read_only {
            return;
        }
        let Some(ix) = self.search.current_index() else {
            return;
        };
        let Some(found) = self.search.current() else {
            return;
        };
        match found {
            MatchLocation::Phantom { .. } => {
                self.search
                    .set_status(Some(PHANTOM_REPLACE_NOTICE.to_string()));
                if let Some(next) = self.search.next_buffer_match_after(ix) {
                    self.search.set_current(next);
                    if let Some(found) = self.search.current() {
                        self.search_from = self.search.position(&found);
                        self.select_match(&found);
                    }
                }
                self.autoscroll = true;
                cx.notify();
            }
            MatchLocation::Buffer(range) => {
                let text = self.snapshot.text();
                let replacement = self.search.replacement_at(&text, range.clone());
                let end = range.start + replacement.len();
                self.search.set_status(None);
                self.replace_buffer_range(range, &replacement, cx);
                // The next match is the first one after the inserted text, so
                // a replacement that contains the query is not visited again.
                self.search_from = end;
                self.search.select_at_or_after(end);
                if let Some(found) = self.search.current() {
                    self.select_match(&found);
                }
                self.autoscroll = true;
                cx.notify();
            }
        }
    }

    /// "Reemplazar todo": every real match in one `EditSource::User`
    /// transaction (one `Ctrl+Z` undoes it whole); phantom matches are left
    /// alone and the counter says how many (D12). The cursor lands after the
    /// first replacement.
    pub(crate) fn replace_all(&mut self, cx: &mut Context<Self>) {
        if !self.search_open || !self.search.replace_open || self.read_only {
            return;
        }
        let text = self.snapshot.text();
        let (edits, phantom) = self.search.replace_all_edits(&text);
        let replaced = edits.len();
        if let Some((range, replacement)) = edits.first() {
            let cursor = range.start + replacement.len();
            self.search_from = cursor;
            self.apply_edits(edits, cursor, cursor, cx);
        }
        self.search
            .set_status(Some(replace_all_message(replaced, phantom)));
        self.autoscroll = true;
        cx.notify();
    }

    fn on_toggle_search_regex(
        &mut self,
        _: &ToggleSearchRegex,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search.regex = !self.search.regex;
        self.search.set_status(None);
        self.refresh_search(cx);
    }

    fn on_toggle_search_case(
        &mut self,
        _: &ToggleSearchCase,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search.case_sensitive = !self.search.case_sensitive;
        self.search.set_status(None);
        self.refresh_search(cx);
    }

    /// Selects a match: a buffer range, or a range of a phantom row in
    /// display coordinates (the cursor can sit on phantom rows).
    fn select_match(&mut self, found: &MatchLocation) {
        match found {
            MatchLocation::Buffer(range) => self.select_buffer_range(range.clone()),
            MatchLocation::Phantom {
                hunk_ix,
                line_ix,
                range,
            } => {
                if *hunk_ix >= self.display_map.diff().hunks().len() {
                    return;
                }
                let row = self.display_map.diff().phantom_start(*hunk_ix) + *line_ix as u32;
                self.selection_anchor = self.clip_point(DisplayPoint::new(row, range.start as u32));
                self.cursor = self.clip_point(DisplayPoint::new(row, range.end as u32));
            }
        }
    }

    fn select_buffer_range(&mut self, range: Range<usize>) {
        self.selection_anchor = self.display_point_for_offset(range.start);
        self.cursor = self.display_point_for_offset(range.end);
    }

    // -- review ------------------------------------------------------------

    /// Replaces the review of the file. The host calls it on every change of
    /// its store, so it is cheap when little changed:
    ///
    /// - when every hunk keeps its rows and its deleted text (a flag, a count,
    ///   the word diffs or `turn_active` changed), the display map, the wrap
    ///   map and the phantom highlights are all kept and only a repaint is
    ///   asked for;
    /// - otherwise the display map is rebuilt, but the highlights of every
    ///   phantom block that survived are kept (they are keyed by content) and
    ///   the shaped-line cache is content addressed, so no row whose text did
    ///   not change is shaped again.
    ///
    /// The cursor and the selection keep their place in the text: a hunk that
    /// appears or goes away above them does not move them to another line.
    /// After a decision with [`EditorSettings::jump_to_next_on_decide`], or
    /// when the host changes `current_index`, the cursor moves to that hunk and
    /// scrolls into view.
    pub fn set_review(&mut self, mut review: ReviewView, cx: &mut Context<Self>) {
        let current = review
            .current_index
            .and_then(|ix| review.hunks.get(ix))
            .map(|hunk| hunk.id);
        review.hunks.sort_by_key(|hunk| hunk.buffer_rows.start);
        review.current_index = current.and_then(|id| review.index_of(id));

        let same_rows = self.review.hunks.len() == review.hunks.len()
            && self
                .review
                .hunks
                .iter()
                .zip(&review.hunks)
                .all(|(old, new)| old.same_rows(new));
        let was_turn = self.review.turn_active;
        let was_notice_state = notice_state(&self.review);
        let previous_current = self.review_current;
        let cursor = self.anchor_point(self.cursor);
        let anchor = self.anchor_point(self.selection_anchor);

        self.review = review;
        self.review_current = current;
        if same_rows {
            self.refresh_review_caches();
        } else {
            self.review_rebuilds += 1;
            self.rebuild_display_map();
            self.invalidate_layout();
            self.cursor = self.resolve_anchor(cursor);
            self.selection_anchor = self.resolve_anchor(anchor);
        }

        // The spinner only ticks while the agent writes this file.
        match (was_turn, self.review.turn_active) {
            (false, true) => self.start_spinner(cx),
            (true, false) => self._spinner_task = None,
            _ => {}
        }

        // "Sin cambios pendientes en este archivo…" shows for 3 s when the
        // file runs out of hunks while other files still have some.
        let notice_state = notice_state(&self.review);
        if notice_state && !was_notice_state {
            self.review_notice = true;
            self._notice_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(REVIEW_NOTICE_DURATION).await;
                this.update(cx, |this, cx| {
                    this.review_notice = false;
                    cx.notify();
                })
                .ok();
            }));
        } else if !notice_state {
            self.review_notice = false;
            self._notice_task = None;
        }

        // Where the cursor goes.
        if let Some((decided, buffer_row)) = self.pending_jump {
            if self.review.index_of(decided).is_none() {
                self.pending_jump = None;
                let target = self
                    .review
                    .current_index
                    .or_else(|| {
                        self.review
                            .hunks
                            .iter()
                            .position(|hunk| hunk.buffer_rows.start >= buffer_row)
                    })
                    .or_else(|| (!self.review.hunks.is_empty()).then_some(0));
                if let Some(ix) = target {
                    self.jump_to_hunk(ix, cx);
                }
            }
        } else if let Some(ix) = self.review.current_index
            && current != previous_current
        {
            if self.hunk_under_cursor() == Some(ix) {
                // The host pointed at the hunk the cursor is already in: the
                // cursor stays, but the change still has to be on screen.
                self.autoscroll = true;
                self.autoscroll_center = true;
            } else {
                self.jump_to_hunk(ix, cx);
            }
        }
        cx.notify();
    }

    /// Recomputes what the element reads per hunk: the phantom-highlight keys
    /// and the deleted word ranges split per line.
    fn refresh_review_caches(&mut self) {
        self.phantom_keys = self
            .review
            .hunks
            .iter()
            .map(|hunk| {
                let mut hasher = DefaultHasher::new();
                hunk.deleted_lines.hash(&mut hasher);
                hasher.finish()
            })
            .collect();
        self.deleted_words = self
            .review
            .hunks
            .iter()
            .map(ReviewHunkView::deleted_words_per_line)
            .collect();
        let keys = &self.phantom_keys;
        self.phantom_spans.retain(|key, _| keys.contains(key));
    }

    /// Where a display point is in the text, independently of the hunks.
    fn anchor_point(&self, point: DisplayPoint) -> PointAnchor {
        match self.display_map.to_buffer(point.row) {
            DisplayCell::Buffer(row) => PointAnchor::Buffer(Point::new(row, point.column)),
            DisplayCell::Phantom { hunk_ix, line_ix } => {
                let hunk = &self.review.hunks[hunk_ix];
                PointAnchor::Phantom {
                    hunk: hunk.id,
                    line: line_ix,
                    column: point.column,
                    fallback_row: hunk.buffer_rows.start,
                }
            }
        }
    }

    /// The display point of an anchor in the current display map.
    fn resolve_anchor(&self, anchor: PointAnchor) -> DisplayPoint {
        match anchor {
            PointAnchor::Buffer(point) => {
                let row = self.display_map.to_display_row(point.row);
                self.clip_point(DisplayPoint::new(row, point.column))
            }
            PointAnchor::Phantom {
                hunk,
                line,
                column,
                fallback_row,
            } => match self.review.index_of(hunk) {
                Some(ix) if line < self.review.hunks[ix].deleted_lines.len() => {
                    let row = self.display_map.diff().phantom_start(ix) + line as u32;
                    self.clip_point(DisplayPoint::new(row, column))
                }
                // The phantom row is gone: the cursor lands where the edit
                // would have gone, the first real row below it.
                _ => {
                    let buffer_row = fallback_row.min(self.snapshot.line_count().saturating_sub(1));
                    let row = self.display_map.to_display_row(buffer_row);
                    self.clip_point(DisplayPoint::new(row, 0))
                }
            },
        }
    }

    fn jump_to_hunk(&mut self, ix: usize, cx: &mut Context<Self>) {
        let row = self.display_map.diff().hunk_display_range(ix).start;
        self.jump_to_row(row, cx);
    }

    /// Every jump to a pending change lands here (deciding with
    /// `jump_to_next_on_decide`, `Alt+J`/`Alt+K`, the review panel's
    /// "Revisar", `Alt+L` to the next file): the cursor goes to the start of
    /// `row` and the next prepaint centres it vertically, like Zed's
    /// `scroll_to_center`, unless it is already in the middle third of the
    /// viewport.
    fn jump_to_row(&mut self, row: DisplayRow, cx: &mut Context<Self>) {
        self.move_cursor(DisplayPoint::new(row, 0), false, cx);
        self.autoscroll_center = true;
    }

    fn start_spinner(&mut self, cx: &mut Context<Self>) {
        self._spinner_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SPINNER_INTERVAL).await;
                let ticked = this.update(cx, |this, cx| {
                    this.spinner_phase = this.spinner_phase.wrapping_add(1);
                    cx.notify();
                });
                if ticked.is_err() {
                    break;
                }
            }
        }));
    }

    /// Index of the hunk a display row belongs to: the hunk of a phantom row,
    /// or the hunk whose `buffer_rows` contain a real row.
    pub(crate) fn review_hunk_at_row(&self, row: DisplayRow) -> Option<usize> {
        match self.display_map.to_buffer(row) {
            DisplayCell::Phantom { hunk_ix, .. } => Some(hunk_ix),
            DisplayCell::Buffer(buffer_row) => self
                .review
                .hunks
                .iter()
                .position(|hunk| hunk.buffer_rows.contains(&buffer_row)),
        }
    }

    /// `(hunk index, index into its lines)` of the reviewable line on a row.
    pub(crate) fn review_line_at_row(&self, row: DisplayRow) -> Option<(usize, usize)> {
        let ix = self.review_hunk_at_row(row)?;
        let hunk = &self.review.hunks[ix];
        let line = match self.display_map.to_buffer(row) {
            DisplayCell::Phantom { line_ix, .. } => hunk.line_for_base(line_ix as u32),
            DisplayCell::Buffer(buffer_row) => hunk.line_for_buffer_row(buffer_row),
        }?;
        Some((ix, line))
    }

    /// Index of the hunk the cursor is inside, if any (its phantom rows
    /// count).
    pub fn hunk_under_cursor(&self) -> Option<usize> {
        self.review_hunk_at_row(self.cursor.row)
    }

    /// The hunk the floating bar talks about: the one under the cursor, else
    /// the host's current one, else the next one below the cursor.
    pub(crate) fn review_target(&self) -> Option<usize> {
        if self.review.hunks.is_empty() {
            return None;
        }
        self.hunk_under_cursor()
            .or(self.review.current_index)
            .or_else(|| {
                let starts = self.hunk_starts();
                starts
                    .iter()
                    .position(|start| *start > self.cursor.row)
                    .or(Some(0))
            })
    }

    /// Emits a review action, unless it is a decision and the agent is
    /// writing the file. Returns whether it was emitted.
    pub(crate) fn emit_review(&mut self, action: ReviewAction, cx: &mut Context<Self>) -> bool {
        if action.is_decision() && self.review.turn_active {
            return false;
        }
        if self.settings.jump_to_next_on_decide {
            let decided = match action {
                ReviewAction::AcceptHunk(id)
                | ReviewAction::RejectHunk(id)
                | ReviewAction::AcceptLine { hunk: id, .. }
                | ReviewAction::RejectLine { hunk: id, .. } => Some(id),
                _ => None,
            };
            if let Some(id) = decided
                && let Some(ix) = self.review.index_of(id)
            {
                self.pending_jump = Some((id, self.review.hunks[ix].buffer_rows.start));
            }
        }
        cx.emit(EditorEvent::Review(action));
        true
    }

    /// Accepts a hunk locally: the phantom rows and the added background go
    /// away. This is the E0 simulation; no key or button calls it any more.
    #[deprecated(note = "the host applies `EditorEvent::Review` and calls `set_review`")]
    pub fn accept_hunk(&mut self, hunk_ix: usize, cx: &mut Context<Self>) {
        if hunk_ix >= self.review.hunks.len() {
            return;
        }
        let mut review = self.review.clone();
        review.hunks.remove(hunk_ix);
        review.pending_in_file = review.hunks.len();
        review.current_index = None;
        self.set_review(review, cx);
    }

    /// Rejects a hunk locally: the added rows are replaced by the deleted text.
    /// This is the E0 simulation; no key or button calls it any more.
    #[deprecated(note = "the host applies `EditorEvent::Review` and calls `set_review`")]
    pub fn reject_hunk(&mut self, hunk_ix: usize, cx: &mut Context<Self>) {
        if hunk_ix >= self.review.hunks.len() {
            return;
        }
        let mut review = self.review.clone();
        let hunk = review.hunks.remove(hunk_ix);
        let added = hunk.buffer_rows.clone();
        {
            let mut buffer = self.buffer.lock();
            buffer.transact(EditSource::Review, |buffer| {
                buffer.replace_rows(added.clone(), &hunk.deleted_lines);
            });
            buffer.drain_events();
        }
        self.snapshot = self.buffer.lock().snapshot();
        if let Some(state) = self.syntax.state.as_mut() {
            state.reset(self.snapshot.clone());
        } else {
            self.syntax.stale = true;
        }
        self.syntax.drop_carry();
        self.request_reparse(cx);
        let delta = hunk.deleted_lines.len() as i64 - (added.end - added.start) as i64;
        let shift = |row: u32| -> u32 { (row as i64 + delta).max(0) as u32 };
        for other in &mut review.hunks {
            if other.buffer_rows.start >= added.end {
                other.buffer_rows = shift(other.buffer_rows.start)..shift(other.buffer_rows.end);
                for line in &mut other.lines {
                    if let Some(row) = line.buffer_row.as_mut() {
                        *row = shift(*row);
                    }
                }
            }
        }
        review.pending_in_file = review.hunks.len();
        review.current_index = None;
        // The rows moved under the old display map: rebuild from scratch.
        self.review.hunks.clear();
        self.rebuild_display_map();
        self.set_review(review, cx);
        self.update_dirty(cx);
    }

    fn on_accept_hunk(&mut self, _: &AcceptHunk, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.hunk_under_cursor() {
            let id = self.review.hunks[ix].id;
            self.emit_review(ReviewAction::AcceptHunk(id), cx);
        }
    }

    fn on_reject_hunk(&mut self, _: &RejectHunk, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.hunk_under_cursor() {
            let id = self.review.hunks[ix].id;
            self.emit_review(ReviewAction::RejectHunk(id), cx);
        }
    }

    fn on_accept_line(&mut self, _: &AcceptLine, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((ix, line)) = self.review_line_at_row(self.cursor.row) {
            let hunk = self.review.hunks[ix].id;
            self.emit_review(ReviewAction::AcceptLine { hunk, line }, cx);
        }
    }

    fn on_reject_line(&mut self, _: &RejectLine, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((ix, line)) = self.review_line_at_row(self.cursor.row) {
            let hunk = self.review.hunks[ix].id;
            self.emit_review(ReviewAction::RejectLine { hunk, line }, cx);
        }
    }

    fn on_accept_file(&mut self, _: &AcceptFile, _: &mut Window, cx: &mut Context<Self>) {
        if !self.review.hunks.is_empty() {
            self.emit_review(ReviewAction::AcceptFile, cx);
        }
    }

    fn on_reject_file(&mut self, _: &RejectFile, _: &mut Window, cx: &mut Context<Self>) {
        if !self.review.hunks.is_empty() {
            self.emit_review(ReviewAction::RejectFile, cx);
        }
    }

    /// First display row of every hunk, ascending.
    fn hunk_starts(&self) -> Vec<DisplayRow> {
        (0..self.review.hunks.len())
            .map(|ix| self.display_map.diff().hunk_display_range(ix).start)
            .collect()
    }

    /// Moves to the next hunk of this file, wrapping around, and tells the
    /// host (which may answer with another `current_index` or file).
    pub(crate) fn next_hunk(&mut self, cx: &mut Context<Self>) {
        let current = self.cursor.row;
        let starts = self.hunk_starts();
        let next = starts
            .iter()
            .copied()
            .find(|start| *start > current)
            .or_else(|| starts.first().copied());
        if let Some(row) = next {
            self.jump_to_row(row, cx);
        }
        self.emit_review(ReviewAction::NextHunk, cx);
    }

    /// Moves to the previous hunk of this file, wrapping around, and tells
    /// the host.
    pub(crate) fn prev_hunk(&mut self, cx: &mut Context<Self>) {
        let current = self.cursor.row;
        let starts = self.hunk_starts();
        let previous = starts
            .iter()
            .copied()
            .rev()
            .find(|start| *start < current)
            .or_else(|| starts.last().copied());
        if let Some(row) = previous {
            self.jump_to_row(row, cx);
        }
        self.emit_review(ReviewAction::PrevHunk, cx);
    }

    fn on_next_hunk(&mut self, _: &NextHunk, _: &mut Window, cx: &mut Context<Self>) {
        self.next_hunk(cx);
    }

    fn on_prev_hunk(&mut self, _: &PrevHunk, _: &mut Window, cx: &mut Context<Self>) {
        self.prev_hunk(cx);
    }

    fn on_next_file(&mut self, _: &NextFile, _: &mut Window, cx: &mut Context<Self>) {
        self.emit_review(ReviewAction::NextFile, cx);
    }

    fn on_open_review_panel(
        &mut self,
        _: &OpenReviewPanel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.emit_review(ReviewAction::OpenReviewPanel, cx);
    }

    fn on_undo_last_reject(&mut self, _: &UndoLastReject, _: &mut Window, cx: &mut Context<Self>) {
        self.emit_review(ReviewAction::UndoLastReject, cx);
    }
}

/// Whether a review is in the "nothing left here, some elsewhere" state.
fn notice_state(review: &ReviewView) -> bool {
    review.hunks.is_empty()
        && review.pending_in_file == 0
        && review.pending_in_other_files > 0
        && !review.turn_active
}

/// A display point pinned to the text, so it survives a new display map.
#[derive(Clone, Copy, Debug)]
enum PointAnchor {
    /// A real row.
    Buffer(Point),
    /// A phantom row: line `line` of hunk `hunk`, or the row the edits of
    /// that phantom block went to if the hunk is gone.
    Phantom {
        hunk: u64,
        line: usize,
        column: u32,
        fallback_row: u32,
    },
}

// -- blinking and scrolling -------------------------------------------------

impl EditorView {
    fn start_blinking(&mut self, cx: &mut Context<Self>) {
        self._blink_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let updated = this.update(cx, |this, cx| {
                    // 02-visual §5: the blink stops 5 s after the last input.
                    if !this.settings.cursor_blink || this.last_input.elapsed() > BLINK_IDLE {
                        if !this.blink_visible {
                            this.blink_visible = true;
                            cx.notify();
                        }
                        return;
                    }
                    this.blink_visible = !this.blink_visible;
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        }));
    }

    /// Opacity of the scrollbar right now (1 while the mouse is moving, fading
    /// out afterwards).
    pub(crate) fn scrollbar_alpha(&self) -> f32 {
        if self.scrollbar_drag.is_some() {
            return 1.;
        }
        let Some(at) = self.mouse_moved_at else {
            return 0.;
        };
        let elapsed = at.elapsed();
        if elapsed < SCROLLBAR_HOLD {
            return 1.;
        }
        let fading = (elapsed - SCROLLBAR_HOLD).as_secs_f32() / SCROLLBAR_FADE.as_secs_f32();
        (1. - fading).clamp(0., 1.)
    }

    pub(crate) fn note_mouse_moved(&mut self, cx: &mut Context<Self>) {
        let was_hidden = self.scrollbar_alpha() <= 0.;
        self.mouse_moved_at = Some(Instant::now());
        self._fade_task = Some(cx.spawn(async move |this, cx| {
            // One repaint when the fade starts, one when it is over: in
            // between, the element asks for animation frames itself.
            cx.background_executor().timer(SCROLLBAR_HOLD).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                return;
            }
            cx.background_executor().timer(SCROLLBAR_FADE).await;
            this.update(cx, |this, cx| {
                this.mouse_moved_at = None;
                cx.notify();
            })
            .ok();
        }));
        if was_hidden {
            cx.notify();
        }
    }

    /// Scrolls by `delta_rows` wrap rows (negative scrolls up).
    pub fn scroll_rows(&mut self, delta_rows: f32, cx: &mut Context<Self>) {
        let line_height = f32::from(self.style.line_height);
        let viewport = self
            .layout
            .as_ref()
            .map(|layout| f32::from(layout.bounds.size.height))
            .unwrap_or(0.);
        let max_scroll = (self.wrap_row_count() as f32 * line_height - viewport).max(0.);
        self.scroll_top = (self.scroll_top + delta_rows * line_height).clamp(0., max_scroll);
        cx.notify();
    }

    pub(crate) fn scroll_by(
        &mut self,
        delta: gpui::Point<Pixels>,
        max_scroll: f32,
        max_scroll_x: f32,
        cx: &mut Context<Self>,
    ) {
        self.scroll_top = (self.scroll_top - f32::from(delta.y)).clamp(0., max_scroll.max(0.));
        if !self.soft_wrap() {
            self.scroll_left =
                (self.scroll_left - f32::from(delta.x)).clamp(0., max_scroll_x.max(0.));
        }
        cx.notify();
    }

    // -- mouse -------------------------------------------------------------

    /// Maps a window position to a display point using the last layout.
    pub(crate) fn point_for_position(
        &self,
        position: gpui::Point<Pixels>,
        _window: &Window,
    ) -> DisplayPoint {
        let Some(layout) = self.layout.as_ref() else {
            return DisplayPoint::default();
        };
        let relative_y = position.y - layout.bounds.top() + px(self.scroll_top);
        let row = (f32::from(relative_y) / f32::from(layout.line_height)).floor();
        let wrap_row = (row.max(0.) as u32).min(self.wrap_row_count().saturating_sub(1));
        let x = position.x - layout.text_origin_x + px(self.scroll_left);
        let (display_row, segment) = self.wrap.to_display(wrap_row);
        let indent = if segment > 0 {
            self.wrap_indent_x(display_row, layout.char_width)
        } else {
            px(0.)
        };
        let column = match layout.rows.iter().find(|(wrap, _, _, _)| *wrap == wrap_row) {
            Some((_, _, _, line)) => line.closest_index_for_x((x - indent).max(px(0.))),
            None => {
                let row_text = self.display_row_text(display_row);
                let range = self
                    .wrap
                    .segment_range(display_row, segment, row_text.len());
                let from = row_text.to_display(range.start) as usize;
                let to = row_text.to_display(range.end) as usize;
                let text = row_text.text();
                let slice = &text[from.min(text.len())..to.min(text.len())];
                let chars = (f32::from((x - indent).max(px(0.))) / f32::from(layout.char_width))
                    .round() as usize;
                slice
                    .char_indices()
                    .nth(chars)
                    .map(|(byte, _)| byte)
                    .unwrap_or(slice.len())
            }
        };
        // `column` is a byte offset inside the painted segment; map it back.
        let row_text = self.display_row_text(display_row);
        let range = self
            .wrap
            .segment_range(display_row, segment, row_text.len());
        let from = row_text.to_display(range.start) as usize;
        let source = row_text.to_source((from + column) as u32);
        self.clip_point(DisplayPoint::new(display_row, source))
    }

    /// The display row under a window position, `None` outside the editor
    /// or below the last row.
    pub(crate) fn display_row_at(&self, position: gpui::Point<Pixels>) -> Option<DisplayRow> {
        let layout = self.layout.as_ref()?;
        if !layout.bounds.contains(&position) {
            return None;
        }
        let y = f32::from(position.y - layout.bounds.top()) + self.scroll_top;
        let wrap_row = (y / f32::from(layout.line_height).max(1.)).floor();
        if wrap_row < 0. || wrap_row as u32 >= self.wrap_row_count() {
            return None;
        }
        Some(self.wrap.to_display(wrap_row as u32).0)
    }

    pub(crate) fn begin_selection(
        &mut self,
        point: DisplayPoint,
        extend: bool,
        click_count: usize,
        cx: &mut Context<Self>,
    ) {
        self.selecting = true;
        self.auto_closers.clear();
        self.select_mode = match click_count {
            2 => SelectMode::Word,
            n if n >= 3 => SelectMode::Line,
            _ => SelectMode::Character,
        };
        match self.select_mode {
            SelectMode::Character => {
                self.select_origin = point..point;
                self.move_cursor(point, extend, cx);
            }
            SelectMode::Word => {
                let range = self.word_range_at(point);
                self.select_origin = range.clone();
                self.selection_anchor = range.start;
                self.cursor = range.end;
                self.after_input(cx);
            }
            SelectMode::Line => {
                let range = self.row_range_at(point);
                self.select_origin = range.clone();
                self.selection_anchor = range.start;
                self.cursor = range.end;
                self.after_input(cx);
            }
        }
    }

    pub(crate) fn update_selection(&mut self, point: DisplayPoint, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        match self.select_mode {
            SelectMode::Character => self.move_cursor(point, true, cx),
            SelectMode::Word | SelectMode::Line => {
                let range = if self.select_mode == SelectMode::Word {
                    self.word_range_at(point)
                } else {
                    self.row_range_at(point)
                };
                if range.start < self.select_origin.start {
                    self.selection_anchor = self.select_origin.end;
                    self.cursor = range.start;
                } else {
                    self.selection_anchor = self.select_origin.start;
                    self.cursor = range.end;
                }
                self.after_input(cx);
            }
        }
    }

    pub(crate) fn end_selection(&mut self) {
        self.selecting = false;
        self.select_mode = SelectMode::Character;
    }
}

// -- rendering --------------------------------------------------------------

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EditorView {
    /// The key context of the element right now
    /// (`docs/specs/modulos/editor.md`).
    ///
    /// This is the *node* syntax (identifiers separated by spaces), while the
    /// bindings use the *predicate* syntax (`Editor && searching`).
    pub fn key_context(&self) -> String {
        let mut context = String::from("Editor");
        if self.search_open {
            context.push_str(" searching");
            if self.replace_field_active() {
                context.push_str(" replacing");
            }
        }
        if self.hunk_under_cursor().is_some() {
            context.push_str(" review_hunk_under_cursor");
        }
        context
    }

    /// Whether the search bar uses its compact form (glyph buttons with a
    /// tooltip, no "Esc cierra"): the editor is narrower than
    /// [`SEARCH_BAR_COMPACT_WIDTH`].
    pub(crate) fn search_bar_compact(&self) -> bool {
        self.layout
            .as_ref()
            .is_some_and(|layout| f32::from(layout.bounds.size.width) < SEARCH_BAR_COMPACT_WIDTH)
    }

    /// The search bar: one strip of [`SEARCH_BAR_HEIGHT`] in both modes. In
    /// replace mode "Buscar…" and "Reemplazar…" share it side by side (flex 1
    /// each), followed by the `.*` / `Aa` toggles, the counter (or the status
    /// message) and the "Reemplazar" / "Reemplazar todo" buttons.
    fn render_search_bar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = &self.theme;
        let replace = self.search.replace_open;
        let compact = self.search_bar_compact();
        let counter = self.search.status_or_counter().unwrap_or_default();
        let muted = theme::color(theme.text_muted);
        let toggle = |label: &'static str, on: bool| {
            div()
                .flex_none()
                .px(px(6.))
                .py(px(1.))
                .text_color(if on {
                    theme::color(theme.text_accent)
                } else {
                    muted
                })
                .child(label)
        };
        // A field's text: what was typed (line breaks shown as `⏎`), or the
        // muted placeholder.
        let field_text = |value: &str, placeholder: &'static str| {
            if value.is_empty() {
                (SharedString::from(placeholder), muted)
            } else {
                (
                    SharedString::from(value.replace('\n', "⏎")),
                    theme::color(theme.text),
                )
            }
        };
        let (query, query_color) = field_text(&self.search.query, "Buscar…");

        let bar = div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .w_full()
            .h(px(SEARCH_BAR_HEIGHT))
            .px(px(8.))
            .bg(theme::color(theme.surface))
            .border_b(px(1.))
            .border_color(theme::color(theme.border))
            .text_size(self.style.font_size)
            .font_family(self.style.font.family.clone());

        if !replace {
            // Search mode: exactly the bar of before replace existed.
            return bar
                .child(div().flex_none().text_color(muted).child("Buscar"))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(query_color)
                        .child(query),
                )
                .child(toggle(".*", self.search.regex))
                .child(toggle("Aa", self.search.case_sensitive))
                .child(
                    div()
                        .flex_shrink(1.)
                        .min_w_0()
                        .truncate()
                        .text_color(muted)
                        .child(SharedString::from(counter)),
                )
                .when(!compact, |bar| {
                    bar.child(div().flex_none().text_color(muted).child("Esc cierra"))
                })
                .into_any_element();
        }

        let (replacement, replacement_color) = field_text(&self.search.replacement, "Reemplazar…");
        let field_box = |id: &'static str,
                         text: SharedString,
                         color: Hsla,
                         active: bool,
                         field: SearchField,
                         cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex_1()
                .min_w(px(60.))
                .h(px(SEARCH_BAR_HEIGHT - 8.))
                .flex()
                .items_center()
                .px(px(6.))
                .rounded(px(3.))
                .border_1()
                .border_color(if active {
                    theme::color(theme.border_focus)
                } else {
                    theme::color(theme.border)
                })
                .overflow_hidden()
                .child(div().min_w_0().truncate().text_color(color).child(text))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.search.field = field;
                        window.focus(&this.focus_handle, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
        };
        let tooltip_colors = (
            theme::color(theme.elevated),
            theme::color(theme.border),
            theme::color(theme.text),
        );
        let button = |id: &'static str,
                      label: &'static str,
                      glyph: &'static str,
                      all: bool,
                      cx: &mut Context<Self>| {
            let button = div()
                .id(id)
                .flex_none()
                .px(px(6.))
                .h(px(SEARCH_BAR_HEIGHT - 8.))
                .flex()
                .items_center()
                .rounded(px(3.))
                .border_1()
                .border_color(theme::color(theme.border))
                .text_color(theme::color(theme.text))
                .cursor(CursorStyle::PointingHand)
                .hover(|style| style.bg(theme::color(theme.elevated)))
                .child(if compact { glyph } else { label })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        window.focus(&this.focus_handle, cx);
                        cx.stop_propagation();
                        if all {
                            this.replace_all(cx);
                        } else {
                            this.replace_next(cx);
                        }
                    }),
                );
            if compact {
                let (background, border, text) = tooltip_colors;
                button
                    .tooltip(move |_window, cx| {
                        cx.new(|_| BarTooltip {
                            text: SharedString::from(label),
                            background,
                            border,
                            color: text,
                        })
                        .into()
                    })
                    .into_any_element()
            } else {
                button.into_any_element()
            }
        };
        let field = self.search.field;
        bar.child(field_box(
            "search-find-field",
            query,
            query_color,
            field == SearchField::Find,
            SearchField::Find,
            cx,
        ))
        .child(field_box(
            "search-replace-field",
            replacement,
            replacement_color,
            field == SearchField::Replace,
            SearchField::Replace,
            cx,
        ))
        .child(toggle(".*", self.search.regex))
        .child(toggle("Aa", self.search.case_sensitive))
        .child(
            div()
                .flex_shrink(1.)
                .min_w_0()
                .truncate()
                .text_color(muted)
                .child(SharedString::from(counter)),
        )
        .child(button("search-replace-next", "Reemplazar", "⇄", false, cx))
        .child(button(
            "search-replace-all",
            "Reemplazar todo",
            "⇶",
            true,
            cx,
        ))
        .when(!compact, |bar| {
            bar.child(div().flex_none().text_color(muted).child("Esc cierra"))
        })
        .into_any_element()
    }

    fn render_prompt(&self) -> impl IntoElement {
        let theme = &self.theme;
        let digits = match &self.prompt {
            Some(Prompt::GoToLine(digits)) => digits.clone(),
            None => String::new(),
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .w_full()
            .h(px(SEARCH_BAR_HEIGHT))
            .px(px(8.))
            .bg(theme::color(theme.surface))
            .border_b(px(1.))
            .border_color(theme::color(theme.border))
            .text_size(self.style.font_size)
            .font_family(self.style.font.family.clone())
            .child(
                div()
                    .text_color(theme::color(theme.text_muted))
                    .child("Ir a la línea"),
            )
            .child(
                div()
                    .flex_1()
                    .text_color(theme::color(theme.text))
                    .child(SharedString::from(digits)),
            )
            .child(
                div()
                    .text_color(theme::color(theme.text_muted))
                    .child("Enter confirma · Esc cancela"),
            )
    }
}

/// The tooltip of a compact search-bar button ("Reemplazar",
/// "Reemplazar todo").
struct BarTooltip {
    text: SharedString,
    background: Hsla,
    border: Hsla,
    color: Hsla,
}

impl Render for BarTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(6.))
            .py(px(2.))
            .rounded(px(4.))
            .bg(self.background)
            .border_1()
            .border_color(self.border)
            .text_color(self.color)
            .child(self.text.clone())
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let context = self.key_context();
        let background = theme::color(self.theme.background);
        let search_bar = if self.search_open {
            Some(self.render_search_bar(cx))
        } else {
            None
        };
        let prompt = if self.prompt.is_some() {
            Some(self.render_prompt())
        } else {
            None
        };
        // With an automatic height the element sizes itself, so nothing above
        // it may stretch to the parent's height.
        let auto_height = self.settings.auto_height.is_some();
        let body = if auto_height {
            div().w_full().child(EditorElement::new(cx.entity()))
        } else {
            div()
                .flex_1()
                .overflow_hidden()
                .child(EditorElement::new(cx.entity()))
        };
        div()
            .key_context(context.as_str())
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .when(auto_height, |this| this.w_full())
            .when(!auto_height, |this| this.size_full())
            .flex()
            .flex_col()
            .bg(background)
            .on_action(cx.listener(Self::on_move_left))
            .on_action(cx.listener(Self::on_move_right))
            .on_action(cx.listener(Self::on_move_up))
            .on_action(cx.listener(Self::on_move_down))
            .on_action(cx.listener(Self::on_move_word_left))
            .on_action(cx.listener(Self::on_move_word_right))
            .on_action(cx.listener(Self::on_select_left))
            .on_action(cx.listener(Self::on_select_right))
            .on_action(cx.listener(Self::on_select_up))
            .on_action(cx.listener(Self::on_select_down))
            .on_action(cx.listener(Self::on_select_word_left))
            .on_action(cx.listener(Self::on_select_word_right))
            .on_action(cx.listener(Self::on_move_to_line_start))
            .on_action(cx.listener(Self::on_move_to_line_end))
            .on_action(cx.listener(Self::on_select_to_line_start))
            .on_action(cx.listener(Self::on_select_to_line_end))
            .on_action(cx.listener(Self::on_move_page_up))
            .on_action(cx.listener(Self::on_move_page_down))
            .on_action(cx.listener(Self::on_select_page_up))
            .on_action(cx.listener(Self::on_select_page_down))
            .on_action(cx.listener(Self::on_move_to_document_start))
            .on_action(cx.listener(Self::on_move_to_document_end))
            .on_action(cx.listener(Self::on_select_to_document_start))
            .on_action(cx.listener(Self::on_select_to_document_end))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_backspace))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_delete_word_left))
            .on_action(cx.listener(Self::on_delete_word_right))
            .on_action(cx.listener(Self::on_insert_newline))
            .on_action(cx.listener(Self::on_tab))
            .on_action(cx.listener(Self::on_backtab))
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_redo))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_find))
            .on_action(cx.listener(Self::on_find_next))
            .on_action(cx.listener(Self::on_find_prev))
            .on_action(cx.listener(Self::on_find_replace))
            .on_action(cx.listener(Self::on_replace_next))
            .on_action(cx.listener(Self::on_replace_all))
            .on_action(cx.listener(Self::on_search_next_field))
            .on_action(cx.listener(Self::on_search_prev_field))
            .on_action(cx.listener(Self::on_toggle_search_regex))
            .on_action(cx.listener(Self::on_toggle_search_case))
            .on_action(cx.listener(Self::on_go_to_line))
            .on_action(cx.listener(Self::on_toggle_soft_wrap))
            .on_action(cx.listener(Self::on_toggle_whitespace))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_cancel))
            .on_action(cx.listener(Self::on_confirm))
            .on_action(cx.listener(Self::on_accept_hunk))
            .on_action(cx.listener(Self::on_reject_hunk))
            .on_action(cx.listener(Self::on_accept_line))
            .on_action(cx.listener(Self::on_reject_line))
            .on_action(cx.listener(Self::on_accept_file))
            .on_action(cx.listener(Self::on_reject_file))
            .on_action(cx.listener(Self::on_next_hunk))
            .on_action(cx.listener(Self::on_prev_hunk))
            .on_action(cx.listener(Self::on_next_file))
            .on_action(cx.listener(Self::on_open_review_panel))
            .on_action(cx.listener(Self::on_undo_last_reject))
            .on_action(cx.listener(Self::on_move_line_up))
            .on_action(cx.listener(Self::on_move_line_down))
            .on_action(cx.listener(Self::on_duplicate_line_down))
            .on_action(cx.listener(Self::on_duplicate_line_up))
            .on_action(cx.listener(Self::on_delete_line))
            .on_action(cx.listener(Self::on_toggle_comments))
            .on_action(cx.listener(Self::on_join_lines))
            .on_action(cx.listener(Self::on_select_line))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_move_to_matching_bracket))
            .on_action(cx.listener(Self::on_uppercase))
            .on_action(cx.listener(Self::on_lowercase))
            .on_action(cx.listener(Self::on_sort_lines))
            .children(prompt)
            .children(search_bar)
            .child(body)
    }
}

// -- IME --------------------------------------------------------------------

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.snapshot.range_from_utf16(&range_utf16);
        actual_range.replace(self.snapshot.range_to_utf16(&range));
        Some(self.snapshot.text_in(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = self.edit_range();
        Some(UTF16Selection {
            range: self.snapshot.range_to_utf16(&range),
            reversed: self.cursor < self.selection_anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.snapshot.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // While a prompt or the search bar is open the keyboard belongs to it.
        if let Some(Prompt::GoToLine(digits)) = self.prompt.as_mut() {
            digits.extend(new_text.chars().filter(char::is_ascii_digit));
            cx.notify();
            return;
        }
        if self.search_open {
            self.type_into_search(new_text, cx);
            return;
        }
        // Plain typing (no IME composition, no explicit range) goes through
        // the auto-closed pairs first.
        if range_utf16.is_none()
            && self.marked_range.is_none()
            && self.type_with_pairs(new_text, cx)
        {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range| self.snapshot.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.edit_range());
        self.replace_buffer_range(range, new_text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.prompt.is_some() || self.search_open {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range| self.snapshot.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.edit_range());
        let start = range.start;
        self.replace_buffer_range(range, new_text, cx);
        self.marked_range = if new_text.is_empty() {
            None
        } else {
            Some(start..start + new_text.len())
        };
        if let Some(selected) = new_selected_range_utf16 {
            // The IME reports the selection relative to the marked text.
            let marked = self.snapshot.text_in(start..start + new_text.len());
            let offset = |units: usize| -> usize {
                let mut utf16 = 0;
                let mut bytes = 0;
                for ch in marked.chars() {
                    if utf16 >= units {
                        break;
                    }
                    utf16 += ch.len_utf16();
                    bytes += ch.len_utf8();
                }
                bytes
            };
            self.cursor = self.display_point_for_offset(start + offset(selected.end));
            self.selection_anchor = self.display_point_for_offset(start + offset(selected.start));
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.snapshot.range_from_utf16(&range_utf16);
        let point = self.display_point_for_offset(range.start);
        let (wrap_row, column) = self.point_to_wrap(point);
        let x = layout.text_origin_x + layout.char_width * column as f32 - px(self.scroll_left);
        let y = element_bounds.top()
            + layout.line_height * wrap_row.saturating_sub(layout.first_wrap_row) as f32;
        Some(Bounds::new(
            gpui::point(x, y),
            gpui::size(px(2.), layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let display_point = self.point_for_position(point, window);
        let offset = self.edit_offset(display_point);
        Some(self.snapshot.offset_to_utf16(offset))
    }
}

/// Convenience constructor used by the example and by the tests.
pub fn editor(
    text: &str,
    language: Option<Arc<Language>>,
    registry: Arc<LanguageRegistry>,
    settings: EditorSettings,
    window: &mut Window,
    cx: &mut App,
) -> Entity<EditorView> {
    let buffer = crate::settings::shared(Buffer::new(text));
    cx.new(|cx| {
        EditorView::new(
            buffer,
            language,
            registry,
            settings,
            EditorTheme::default(),
            window,
            cx,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(ranges: &[(usize, usize)]) -> Vec<(Range<usize>, HighlightId)> {
        ranges
            .iter()
            .map(|(start, end)| (*start..*end, HighlightId::Keyword))
            .collect()
    }

    fn ranges(spans: &[(Range<usize>, HighlightId)]) -> Vec<(usize, usize)> {
        spans
            .iter()
            .map(|(range, _)| (range.start, range.end))
            .collect()
    }

    #[test]
    fn an_insertion_slides_the_spans_after_it() {
        // "fn" 0..2, "main" 3..7, "let" 12..15; a character typed at 8.
        let mut carried = spans(&[(0, 2), (3, 7), (12, 15)]);
        shift_spans(&mut carried, 8, 0, 1);
        assert_eq!(ranges(&carried), vec![(0, 2), (3, 7), (13, 16)]);
    }

    #[test]
    fn a_span_the_edit_cuts_through_keeps_its_head() {
        let mut carried = spans(&[(0, 2), (3, 7)]);
        // Typed in the middle of "main".
        shift_spans(&mut carried, 5, 0, 1);
        assert_eq!(ranges(&carried), vec![(0, 2), (3, 5)]);
    }

    #[test]
    fn a_span_the_edit_swallows_is_dropped() {
        let mut carried = spans(&[(0, 2), (3, 7), (9, 11)]);
        // The whole of "main" plus a space is replaced by one character.
        shift_spans(&mut carried, 3, 5, 1);
        assert_eq!(ranges(&carried), vec![(0, 2), (5, 7)]);
    }

    #[test]
    fn a_deletion_pulls_the_spans_back() {
        let mut carried = spans(&[(0, 2), (10, 14)]);
        shift_spans(&mut carried, 3, 4, 0);
        assert_eq!(ranges(&carried), vec![(0, 2), (6, 10)]);
    }

    #[test]
    fn carried_spans_are_clipped_to_the_range_asked_for() {
        let carry = HighlightCarry {
            range: 0..20,
            spans: spans(&[(0, 4), (8, 12), (16, 20)]),
        };
        let clipped = carried_spans(Some(&carry), &(2..10));
        assert_eq!(ranges(&clipped), vec![(2, 4), (8, 10)]);
        assert!(carried_spans(None, &(0..10)).is_empty());
    }

    #[test]
    fn word_boundaries_walk_by_class() {
        let text = "let mi_var = otra(1);";
        assert_eq!(next_word_boundary(text, 0), 3);
        assert_eq!(next_word_boundary(text, 3), 4);
        assert_eq!(next_word_boundary(text, 4), 10);
        assert_eq!(prev_word_boundary(text, 10), 4);
        assert_eq!(prev_word_boundary(text, 4), 0);
        assert_eq!(prev_word_boundary(text, 0), 0);
        assert_eq!(next_word_boundary(text, text.len()), text.len());
    }

    #[test]
    fn word_at_selects_the_whole_word() {
        let text = "hola mundo";
        assert_eq!(word_at(text, 0), (0, 4));
        assert_eq!(word_at(text, 2), (0, 4));
        assert_eq!(word_at(text, 4), (4, 5));
        assert_eq!(word_at(text, 7), (5, 10));
        assert_eq!(word_at("", 0), (0, 0));
    }

    #[test]
    fn word_boundaries_are_char_safe() {
        let text = "añoñ niño";
        // Every boundary must land on a char boundary.
        let mut at = 0;
        while at < text.len() {
            let next = next_word_boundary(text, at);
            assert!(text.is_char_boundary(next), "{next} in {text:?}");
            assert!(next > at);
            at = next;
        }
    }
}
