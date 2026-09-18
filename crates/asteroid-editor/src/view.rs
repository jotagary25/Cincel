//! `EditorView`: the GPUI entity that owns the shared buffer, the display
//! pipeline, the selection, the scroll state and the search bar, and that
//! implements `EntityInputHandler` (IME and dead keys).

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use asteroid_syntax::{CancelFlag, HighlightId, Language, LanguageRegistry, SyntaxState};
use asteroid_text::{Buffer, BufferSnapshot, EditSource, Point};
use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, CursorStyle, Entity, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, Font, FontFallbacks, FontFeatures, FontStyle, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, ShapedLine, SharedString,
    Styled, Task, UTF16Selection, Window, div, px,
};

use crate::actions::*;
use crate::display_map::{
    DiffTransformMap, DisplayCell, DisplayMap, DisplayPoint, DisplayRow, PhantomHunk, RowText,
};
use crate::element::EditorElement;
use crate::search::SearchState;
use crate::settings::{EditorSettings, SharedBuffer, detect_indentation};
use crate::theme::{self, EditorTheme};
use crate::wrap_map::{WrapMap, WrapRow, WrapSource};

/// How long the cursor keeps blinking after the last input (02-visual §5).
pub const BLINK_IDLE: Duration = Duration::from_secs(5);
/// Half period of the blink (02-visual §5).
pub const BLINK_INTERVAL: Duration = Duration::from_millis(500);
/// How long the scrollbar stays fully visible after the mouse moves.
pub const SCROLLBAR_HOLD: Duration = Duration::from_millis(1000);
/// How long the scrollbar takes to fade out afterwards.
pub const SCROLLBAR_FADE: Duration = Duration::from_millis(200);

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
}

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
        Self {
            font: Font {
                family,
                features: FontFeatures::default(),
                fallbacks: Some(fallbacks),
                weight: FontWeight::NORMAL,
                style: FontStyle::Normal,
            },
            font_size,
            line_height: font_size * settings.line_height.max(1.),
        }
    }
}

/// Key of the shaped-line cache. The epoch (buffer version, highlight version,
/// wrap width) is kept apart in [`EditorView::cache_epoch`]: when it changes the
/// whole cache is dropped, so the key only needs to identify the row.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LineCacheKey {
    /// A wrapped segment of a display row.
    Row(DisplayRow, u32),
    /// The whitespace overlay of a segment.
    Whitespace(DisplayRow, u32),
    /// A gutter line number.
    Number(u32),
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
    /// Bumped every time a parse lands; part of the shaped-line cache epoch.
    version: u64,
    /// Highlights of the last queried range.
    spans: Vec<(Range<usize>, HighlightId)>,
    /// `(buffer version, highlight version, range)` the spans were built for.
    spans_key: (u64, u64, usize, usize),
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
        }
    }
}

/// The editor entity.
pub struct EditorView {
    buffer: SharedBuffer,
    snapshot: BufferSnapshot,
    registry: Arc<LanguageRegistry>,
    language: Option<Arc<Language>>,
    syntax: SyntaxHost,
    /// Highlights of the phantom rows of each hunk, one vector per deleted line.
    phantom_spans: HashMap<usize, Vec<HighlightSpans>>,

    settings: EditorSettings,
    theme: EditorTheme,
    pub(crate) style: EditorStyle,
    /// Indentation detected in the file, preferred over the settings.
    detected_indent: Option<crate::settings::Indentation>,

    pub(crate) hunks: Vec<PhantomHunk>,
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
    pub(crate) line_cache: HashMap<LineCacheKey, ShapedLine>,
    pub(crate) cache_epoch: (u64, u64, u32, u32),
    /// `(text version, wrap columns, tab size, display rows)` the wrap map was
    /// built for. Kept apart from `cache_epoch` so a single-row edit can update
    /// one row instead of rebuilding the whole map.
    pub(crate) wrap_epoch: (u64, u32, u32, u32),
    pub(crate) blink_visible: bool,
    last_input: Instant,
    pub(crate) autoscroll: bool,
    pub(crate) selecting: bool,
    pub(crate) select_mode: SelectMode,
    select_origin: Range<DisplayPoint>,
    pub(crate) mouse_moved_at: Option<Instant>,
    pub(crate) scrollbar_drag: Option<f32>,
    dirty: bool,
    frame_times: VecDeque<u64>,
    _blink_task: Option<Task<()>>,
    _fade_task: Option<Task<()>>,
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
        let _ = window;
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
            hunks: Vec::new(),
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
            cache_epoch: (u64::MAX, u64::MAX, u32::MAX, 0),
            wrap_epoch: (u64::MAX, u32::MAX, 0, 0),
            blink_visible: true,
            last_input: Instant::now(),
            autoscroll: false,
            selecting: false,
            select_mode: SelectMode::Character,
            select_origin: DisplayPoint::default()..DisplayPoint::default(),
            mouse_moved_at: None,
            scrollbar_drag: None,
            // A buffer handed over already modified (the agent wrote it before
            // the tab opened) starts dirty.
            dirty,
            frame_times: VecDeque::new(),
            _blink_task: None,
            _fade_task: None,
        };
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
    /// text on disk leaves the tab clean. [`asteroid_text::Buffer::is_dirty`]
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
        self.settings = settings;
        if refont {
            self.style = EditorStyle::from_settings(&self.settings, cx);
        }
        self.soft_wrap_override = None;
        self.line_cache.clear();
        self.cache_epoch.2 = u32::MAX;
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

    /// The simulated review hunks currently displayed.
    pub fn hunks(&self) -> &[PhantomHunk] {
        &self.hunks
    }

    /// Replaces the simulated review hunks.
    pub fn set_hunks(&mut self, hunks: Vec<PhantomHunk>, cx: &mut Context<Self>) {
        self.hunks = hunks;
        self.phantom_spans.clear();
        self.rebuild_display_map();
        self.invalidate_layout();
        cx.notify();
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

    pub(crate) fn record_frame(&mut self, micros: u64) {
        if self.frame_times.len() >= 600 {
            self.frame_times.pop_front();
        }
        self.frame_times.push_back(micros);
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
        if !self.search.query.is_empty() {
            let text = self.snapshot.text();
            let from = self.edit_offset(self.cursor);
            self.search.refresh(&text, from);
        }
        self.update_dirty(cx);
        cx.notify();
    }

    fn apply_syntax_events(&mut self, events: &[asteroid_text::BufferEvent]) {
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
        }
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

    fn invalidate_layout(&mut self) {
        self.line_cache.clear();
        self.cache_epoch = (u64::MAX, u64::MAX, u32::MAX, 0);
        self.wrap_epoch = (u64::MAX, u32::MAX, 0, 0);
    }
}

// -- syntax highlighting ----------------------------------------------------

impl EditorView {
    /// Starts (or restarts) a background parse.
    fn request_reparse(&mut self, cx: &mut Context<Self>) {
        if self.syntax.parsing {
            // Let the running parse give up; its completion restarts.
            self.syntax.cancel.cancel();
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
                this.line_cache.clear();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Version of the highlights, part of the shaped-line cache key.
    pub(crate) fn highlight_version(&self) -> u64 {
        self.syntax.version
    }

    /// Highlight spans covering a byte range of the buffer, memoised per frame.
    pub(crate) fn highlights(&mut self, range: Range<usize>) -> &[(Range<usize>, HighlightId)] {
        let key = (
            self.snapshot.version(),
            self.syntax.version,
            range.start,
            range.end,
        );
        if self.syntax.spans_key != key {
            self.syntax.spans = self
                .syntax
                .state
                .as_ref()
                .map(|state| state.highlights(range))
                .unwrap_or_default();
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
        if !self.phantom_spans.contains_key(&hunk_ix) {
            let spans = self.parse_phantom(hunk_ix);
            self.phantom_spans.insert(hunk_ix, spans);
        }
        self.phantom_spans
            .get(&hunk_ix)
            .and_then(|lines| lines.get(line_ix))
            .cloned()
            .unwrap_or_default()
    }

    fn parse_phantom(&self, hunk_ix: usize) -> Vec<Vec<(Range<usize>, HighlightId)>> {
        let Some(hunk) = self.hunks.get(hunk_ix) else {
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
                self.view.hunks[hunk_ix].deleted_text[line_ix].len() as u32
            }
        }
    }

    fn row_text(&self, row: DisplayRow) -> String {
        self.view.display_row_source(row)
    }
}

impl EditorView {
    /// The source text of a display row (no tab expansion).
    pub fn display_row_source(&self, row: DisplayRow) -> String {
        match self.display_map.to_buffer(row) {
            DisplayCell::Buffer(buffer_row) => self.snapshot.line_text(buffer_row),
            DisplayCell::Phantom { hunk_ix, line_ix } => self
                .hunks
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
        self.display_map = DisplayMap::new(self.snapshot.line_count(), self.hunks.clone());
        self.hunks = self.display_map.diff().hunks().to_vec();
        // The wrap map is rebuilt by the element, which is the only place that
        // knows the viewport width; until then it is one version behind, which
        // only costs an approximate vertical movement.
        self.cursor = self.clip_point(self.cursor);
        self.selection_anchor = self.clip_point(self.selection_anchor);
    }

    /// Version of the text the view is showing.
    pub(crate) fn text_version(&self) -> u64 {
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
                    .hunks
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
        let rows_before = self.snapshot.line_count();
        let edit_row = self.snapshot.offset_to_point(range.start).row;
        self.edit_buffer(&[(range.clone(), text)], cx);
        let delta = self.snapshot.line_count() as i64 - rows_before as i64;
        self.adjust_hunks(edit_row, delta);
        self.rebuild_display_map();
        // Fast path: an edit inside one row only changes that row's wrapping.
        if self.wrap.is_enabled()
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
        if !self.search.query.is_empty() {
            let text = self.snapshot.text();
            let from = self.edit_offset(self.cursor);
            self.search.refresh(&text, from);
        }
        self.update_dirty(cx);
    }

    /// Keeps the simulated hunk rows aligned after an edit.
    fn adjust_hunks(&mut self, edit_row: u32, delta: i64) {
        if delta == 0 {
            return;
        }
        let shift = |row: u32| -> u32 { (row as i64 + delta).max(0) as u32 };
        for hunk in &mut self.hunks {
            if hunk.insert_before_buffer_row > edit_row {
                hunk.insert_before_buffer_row = shift(hunk.insert_before_buffer_row);
                hunk.added_rows = shift(hunk.added_rows.start)..shift(hunk.added_rows.end);
            }
        }
    }

    /// Everything that must happen after the user typed or moved.
    fn after_input(&mut self, cx: &mut Context<Self>) {
        self.last_input = Instant::now();
        self.blink_visible = true;
        self.autoscroll = true;
        cx.emit(EditorEvent::CursorMoved {
            point: self.cursor_point(),
        });
        cx.notify();
    }

    fn move_cursor(&mut self, point: DisplayPoint, extend: bool, cx: &mut Context<Self>) {
        self.cursor = self.clip_point(point);
        if !extend {
            self.selection_anchor = self.cursor;
        }
        self.goal_column = None;
        self.after_input(cx);
    }

    fn move_vertically(&mut self, rows: i64, extend: bool, cx: &mut Context<Self>) {
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
            self.search.query.pop();
            self.refresh_search(cx);
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
                edits.push((start..start, unit.clone()));
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
        let cursor_offset = self.edit_offset(self.cursor);
        let anchor_offset = self.edit_offset(self.selection_anchor);
        let delta = |offset: usize| -> i64 {
            edits
                .iter()
                .filter(|(range, _)| range.start <= offset)
                .map(|(range, text)| text.len() as i64 - (range.end - range.start) as i64)
                .sum()
        };
        let cursor_delta = delta(cursor_offset);
        let anchor_delta = delta(anchor_offset);

        let borrowed: Vec<(Range<usize>, &str)> = edits
            .iter()
            .map(|(range, text)| (range.clone(), text.as_str()))
            .collect();
        self.edit_buffer(&borrowed, cx);
        self.rebuild_display_map();
        self.cursor =
            self.display_point_for_offset((cursor_offset as i64 + cursor_delta).max(0) as usize);
        self.selection_anchor =
            self.display_point_for_offset((anchor_offset as i64 + anchor_delta).max(0) as usize);
        self.after_input(cx);
    }

    fn on_undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let restored = {
            let mut buffer = self.buffer.lock();
            let restored = buffer.undo_with_ranges();
            buffer.drain_events();
            restored
        };
        self.after_history(restored.map(|(_, ranges)| ranges), cx);
    }

    fn on_redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
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
        // instead of trying to derive the intermediate snapshots.
        if let Some(state) = self.syntax.state.as_mut() {
            state.reset(self.snapshot.clone());
        } else {
            self.syntax.stale = true;
        }
        self.request_reparse(cx);
        self.rebuild_display_map();
        self.invalidate_layout();
        if let Some(range) = ranges.last() {
            self.cursor = self.display_point_for_offset(range.end);
            self.selection_anchor = self.cursor;
        }
        if !self.search.query.is_empty() {
            let text = self.snapshot.text();
            let from = self.edit_offset(self.cursor);
            self.search.refresh(&text, from);
        }
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
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert_text(&text, cx);
        }
    }

    fn on_save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(EditorEvent::SaveRequested);
    }

    fn on_toggle_soft_wrap(&mut self, _: &ToggleSoftWrap, _: &mut Window, cx: &mut Context<Self>) {
        let enabled = !self.soft_wrap();
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
        self.search_open = true;
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
        let text = self.snapshot.text();
        let from = self.search_from.min(text.len());
        self.search.refresh(&text, from);
        if let Some(range) = self.search.current() {
            self.select_buffer_range(range);
        }
        self.autoscroll = true;
        cx.notify();
    }

    fn on_find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        if !self.search_open {
            self.search_open = true;
        }
        if let Some(range) = self.search.next_match() {
            self.search_from = range.start;
            self.select_buffer_range(range);
        }
        self.autoscroll = true;
        cx.notify();
    }

    fn on_find_prev(&mut self, _: &FindPrev, _: &mut Window, cx: &mut Context<Self>) {
        if !self.search_open {
            self.search_open = true;
        }
        if let Some(range) = self.search.previous_match() {
            self.search_from = range.start;
            self.select_buffer_range(range);
        }
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
        self.refresh_search(cx);
    }

    fn on_toggle_search_case(
        &mut self,
        _: &ToggleSearchCase,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search.case_sensitive = !self.search.case_sensitive;
        self.refresh_search(cx);
    }

    fn select_buffer_range(&mut self, range: Range<usize>) {
        self.selection_anchor = self.display_point_for_offset(range.start);
        self.cursor = self.display_point_for_offset(range.end);
    }

    // -- review (simulated hunks) ------------------------------------------

    /// Index of the hunk the cursor is inside, if any.
    pub fn hunk_under_cursor(&self) -> Option<usize> {
        self.display_map.diff().hunk_at_display_row(self.cursor.row)
    }

    /// Accepts a hunk: the phantom rows and the added background go away.
    pub fn accept_hunk(&mut self, hunk_ix: usize, cx: &mut Context<Self>) {
        if hunk_ix >= self.hunks.len() {
            return;
        }
        self.hunks.remove(hunk_ix);
        self.phantom_spans.clear();
        self.rebuild_display_map();
        self.invalidate_layout();
        cx.notify();
    }

    /// Rejects a hunk: the added rows are replaced by the deleted text.
    pub fn reject_hunk(&mut self, hunk_ix: usize, cx: &mut Context<Self>) {
        if hunk_ix >= self.hunks.len() {
            return;
        }
        let hunk = self.hunks.remove(hunk_ix);
        let added = hunk.added_rows.clone();
        {
            let mut buffer = self.buffer.lock();
            buffer.transact(EditSource::Review, |buffer| {
                buffer.replace_rows(added.clone(), &hunk.deleted_text);
            });
            buffer.drain_events();
        }
        self.snapshot = self.buffer.lock().snapshot();
        if let Some(state) = self.syntax.state.as_mut() {
            state.reset(self.snapshot.clone());
        } else {
            self.syntax.stale = true;
        }
        self.request_reparse(cx);
        let delta = hunk.deleted_text.len() as i64 - (added.end - added.start) as i64;
        let shift = |row: u32| -> u32 { (row as i64 + delta).max(0) as u32 };
        for other in &mut self.hunks {
            if other.insert_before_buffer_row >= added.end {
                other.insert_before_buffer_row = shift(other.insert_before_buffer_row);
                other.added_rows = shift(other.added_rows.start)..shift(other.added_rows.end);
            }
        }
        self.phantom_spans.clear();
        self.rebuild_display_map();
        self.invalidate_layout();
        self.update_dirty(cx);
        cx.notify();
    }

    fn on_accept_hunk(&mut self, _: &AcceptHunk, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.hunk_under_cursor() {
            self.accept_hunk(ix, cx);
        }
    }

    fn on_reject_hunk(&mut self, _: &RejectHunk, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.hunk_under_cursor() {
            self.reject_hunk(ix, cx);
        }
    }

    fn on_accept_line(&mut self, _: &AcceptLine, _: &mut Window, cx: &mut Context<Self>) {
        let Some(hunk_ix) = self.hunk_under_cursor() else {
            return;
        };
        if let DisplayCell::Phantom { line_ix, .. } = self.display_map.to_buffer(self.cursor.row) {
            // Accepting a deleted line confirms its deletion: drop the row.
            self.hunks[hunk_ix].deleted_text.remove(line_ix);
            if self.hunks[hunk_ix].deleted_text.is_empty()
                && self.hunks[hunk_ix].added_rows.is_empty()
            {
                self.hunks.remove(hunk_ix);
            }
            self.phantom_spans.clear();
            self.rebuild_display_map();
            self.invalidate_layout();
            cx.notify();
        }
    }

    fn on_reject_line(&mut self, _: &RejectLine, _: &mut Window, cx: &mut Context<Self>) {
        let Some(hunk_ix) = self.hunk_under_cursor() else {
            return;
        };
        let DisplayCell::Phantom { line_ix, .. } = self.display_map.to_buffer(self.cursor.row)
        else {
            return;
        };
        // Rejecting a deleted line brings it back into the buffer.
        let line = self.hunks[hunk_ix].deleted_text.remove(line_ix);
        let at = self.hunks[hunk_ix].insert_before_buffer_row;
        let offset = self.snapshot.point_to_offset(Point::new(at, 0));
        {
            let mut buffer = self.buffer.lock();
            buffer.transact(EditSource::Review, |buffer| {
                buffer.insert(offset, &format!("{line}\n"));
            });
            buffer.drain_events();
        }
        self.snapshot = self.buffer.lock().snapshot();
        if let Some(state) = self.syntax.state.as_mut() {
            state.reset(self.snapshot.clone());
        }
        self.request_reparse(cx);
        for hunk in &mut self.hunks {
            if hunk.insert_before_buffer_row > at {
                hunk.insert_before_buffer_row += 1;
                hunk.added_rows = hunk.added_rows.start + 1..hunk.added_rows.end + 1;
            }
        }
        if let Some(hunk) = self.hunks.get_mut(hunk_ix) {
            hunk.insert_before_buffer_row += 1;
            hunk.added_rows = hunk.added_rows.start + 1..hunk.added_rows.end + 1;
            if hunk.deleted_text.is_empty() && hunk.added_rows.is_empty() {
                self.hunks.remove(hunk_ix);
            }
        }
        self.phantom_spans.clear();
        self.rebuild_display_map();
        self.invalidate_layout();
        self.update_dirty(cx);
        cx.notify();
    }

    fn on_accept_file(&mut self, _: &AcceptFile, _: &mut Window, cx: &mut Context<Self>) {
        self.hunks.clear();
        self.phantom_spans.clear();
        self.rebuild_display_map();
        self.invalidate_layout();
        cx.notify();
    }

    fn on_reject_file(&mut self, _: &RejectFile, _: &mut Window, cx: &mut Context<Self>) {
        while !self.hunks.is_empty() {
            self.reject_hunk(self.hunks.len() - 1, cx);
        }
    }

    /// First display row of every hunk, ascending.
    fn hunk_starts(&self) -> Vec<DisplayRow> {
        (0..self.hunks.len())
            .map(|ix| self.display_map.diff().hunk_display_range(ix).start)
            .collect()
    }

    fn on_next_hunk(&mut self, _: &NextHunk, _: &mut Window, cx: &mut Context<Self>) {
        let current = self.cursor.row;
        let starts = self.hunk_starts();
        // Wraps around to the first hunk when there is nothing below.
        let next = starts
            .iter()
            .copied()
            .find(|start| *start > current)
            .or_else(|| starts.first().copied());
        if let Some(row) = next {
            self.move_cursor(DisplayPoint::new(row, 0), false, cx);
        }
    }

    fn on_prev_hunk(&mut self, _: &PrevHunk, _: &mut Window, cx: &mut Context<Self>) {
        let current = self.cursor.row;
        let starts = self.hunk_starts();
        let previous = starts
            .iter()
            .copied()
            .rev()
            .find(|start| *start < current)
            .or_else(|| starts.last().copied());
        if let Some(row) = previous {
            self.move_cursor(DisplayPoint::new(row, 0), false, cx);
        }
    }
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
            layout.char_width * self.wrap.indent(display_row) as f32
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

    pub(crate) fn begin_selection(
        &mut self,
        point: DisplayPoint,
        extend: bool,
        click_count: usize,
        cx: &mut Context<Self>,
    ) {
        self.selecting = true;
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
        }
        if self.hunk_under_cursor().is_some() {
            context.push_str(" review_hunk_under_cursor");
        }
        context
    }

    fn render_search_bar(&self) -> impl IntoElement {
        let theme = &self.theme;
        let counter = self.search.counter().unwrap_or_default();
        let query = if self.search.query.is_empty() {
            SharedString::from("Buscar…")
        } else {
            SharedString::from(self.search.query.clone())
        };
        let query_color = if self.search.query.is_empty() {
            theme::color(theme.text_muted)
        } else {
            theme::color(theme.text)
        };
        let toggle = |label: &'static str, on: bool| {
            div()
                .px(px(6.))
                .py(px(1.))
                .text_color(if on {
                    theme::color(theme.text_accent)
                } else {
                    theme::color(theme.text_muted)
                })
                .child(label)
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .w_full()
            .h(px(28.))
            .px(px(8.))
            .bg(theme::color(theme.surface))
            .border_b(px(1.))
            .border_color(theme::color(theme.border))
            .text_size(self.style.font_size)
            .font_family(self.style.font.family.clone())
            .child(
                div()
                    .text_color(theme::color(theme.text_muted))
                    .child("Buscar"),
            )
            .child(div().flex_1().text_color(query_color).child(query))
            .child(toggle(".*", self.search.regex))
            .child(toggle("Aa", self.search.case_sensitive))
            .child(
                div()
                    .text_color(theme::color(theme.text_muted))
                    .child(SharedString::from(counter)),
            )
            .child(
                div()
                    .text_color(theme::color(theme.text_muted))
                    .child("Esc cierra"),
            )
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
            .h(px(28.))
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

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let context = self.key_context();
        let background = theme::color(self.theme.background);
        let search_bar = if self.search_open {
            Some(self.render_search_bar())
        } else {
            None
        };
        let prompt = if self.prompt.is_some() {
            Some(self.render_prompt())
        } else {
            None
        };
        div()
            .key_context(context.as_str())
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .size_full()
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
            .children(prompt)
            .children(search_bar)
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(EditorElement::new(cx.entity())),
            )
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
            self.search.query.push_str(new_text);
            self.refresh_search(cx);
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
