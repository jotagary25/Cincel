//! Per-buffer syntax state: incremental parsing and range highlighting.

use std::ops::{ControlFlow, Range};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use asteroid_text::{BufferEvent, BufferSnapshot, Point as TextPoint, Rope};
use tree_sitter::{
    InputEdit, Node, ParseOptions, Parser, Point as TsPoint, QueryCursor, StreamingIterator, Tree,
};

use crate::highlight::HighlightId;
use crate::language::{Grammar, Language, LanguageRegistry};

/// Budget hint of `docs/specs/modulos/syntax.md`: a parse that goes past this
/// is expected to run on the background executor and to be cancellable.
pub const PARSE_BUDGET_HINT: Duration = Duration::from_millis(5);

/// How deep injections are followed. One level covers Markdown fences and
/// HTML `<script>`/`<style>`, which is what v1 asks for.
const MAX_INJECTION_DEPTH: usize = 1;

/// A flag the UI thread can flip to abandon a parse running in the background.
///
/// tree-sitter 0.27 dropped `Parser::set_cancellation_flag`; the equivalent is
/// a progress callback that returns `ControlFlow::Break`, which is what this
/// wraps.
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    /// A flag that is not raised.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the parse to stop as soon as it notices.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether the flag is raised.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// What a [`SyntaxState::reparse`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseOutcome {
    /// The tree already matched the snapshot.
    Unchanged,
    /// A tree was produced.
    Parsed {
        /// Wall time of the parse.
        duration: Duration,
        /// Whether it fitted in [`PARSE_BUDGET_HINT`].
        within_budget: bool,
    },
    /// The cancel flag was raised, or the deadline passed. The previous tree is
    /// kept and the pending edits stay applied, so the next call resumes
    /// incrementally.
    Cancelled,
    /// The language's grammar or queries failed to compile.
    NoGrammar,
}

/// One injected region: a sub-tree parsed with a different language.
struct InjectionLayer {
    language: Arc<Language>,
    /// Byte range of the region in the outer document.
    range: Range<usize>,
    /// The region's text, owned so the sub-tree has a stable source.
    text: String,
    tree: Tree,
}

/// The syntax state of one buffer.
///
/// # Threading
///
/// The owner keeps the state on the UI thread, calls [`SyntaxState::apply_event`]
/// synchronously for every [`BufferEvent`] (cheap: it only records
/// `InputEdit`s and swaps the snapshot), and then moves the state to the
/// background executor to call [`SyntaxState::reparse`], passing a
/// [`CancelFlag`] it can raise when a newer edit arrives. `SyntaxState` is
/// `Send`, and the [`BufferSnapshot`] it holds is a cheap rope clone, so no
/// text is copied to cross the thread boundary.
pub struct SyntaxState {
    registry: Arc<LanguageRegistry>,
    language: Arc<Language>,
    snapshot: BufferSnapshot,
    parser: Parser,
    tree: Option<Tree>,
    /// Version of the snapshot the tree was produced from.
    parsed_version: u64,
    /// Whether edits have been applied to the tree since the last parse.
    dirty: bool,
    injections: Vec<InjectionLayer>,
}

impl SyntaxState {
    /// Creates the state for `snapshot`. It does **not** parse: call
    /// [`SyntaxState::reparse`], normally from the background executor.
    pub fn new(
        registry: Arc<LanguageRegistry>,
        language: Arc<Language>,
        snapshot: BufferSnapshot,
    ) -> Self {
        Self {
            registry,
            language,
            snapshot,
            parser: Parser::new(),
            tree: None,
            parsed_version: u64::MAX,
            dirty: true,
            injections: Vec::new(),
        }
    }

    /// The language being parsed.
    pub fn language(&self) -> &Arc<Language> {
        &self.language
    }

    /// The snapshot the next parse will use.
    pub fn snapshot(&self) -> &BufferSnapshot {
        &self.snapshot
    }

    /// The buffer version the current tree corresponds to.
    pub fn parsed_version(&self) -> Option<u64> {
        (self.parsed_version != u64::MAX).then_some(self.parsed_version)
    }

    /// Whether the tree is behind the snapshot.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The current tree, if any.
    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    /// Replaces the whole text (a reload from disk, a language change) and
    /// throws the tree away.
    pub fn reset(&mut self, snapshot: BufferSnapshot) {
        self.snapshot = snapshot;
        self.tree = None;
        self.injections.clear();
        self.parsed_version = u64::MAX;
        self.dirty = true;
    }

    /// Records a buffer edit: turns the event into tree-sitter `InputEdit`s,
    /// applies them to the tree and adopts `new` as the text to parse.
    ///
    /// `new` must be the snapshot taken right *after* the event.
    pub fn apply_event(&mut self, event: &BufferEvent, new: BufferSnapshot) {
        let BufferEvent::Edited {
            old_ranges,
            new_ranges,
            ..
        } = event;
        let old = std::mem::replace(&mut self.snapshot, new);
        if let Some(tree) = self.tree.as_mut() {
            for (old_range, new_range) in old_ranges.iter().zip(new_ranges.iter()) {
                let edit = input_edit(&old, &self.snapshot, old_range, new_range);
                tree.edit(&edit);
            }
        }
        self.injections.clear();
        self.dirty = true;
    }

    /// Parses (incrementally when a tree is already there), honouring `cancel`.
    pub fn reparse(&mut self, cancel: &CancelFlag) -> ParseOutcome {
        self.parse_inner(cancel, None)
    }

    /// Same, but also gives up once `budget` has elapsed. Use it when a stale
    /// highlight is better than a blocked executor.
    pub fn reparse_within(&mut self, cancel: &CancelFlag, budget: Duration) -> ParseOutcome {
        self.parse_inner(cancel, Some(budget))
    }

    fn parse_inner(&mut self, cancel: &CancelFlag, deadline: Option<Duration>) -> ParseOutcome {
        if !self.dirty && self.parsed_version == self.snapshot.version() {
            return ParseOutcome::Unchanged;
        }
        let Ok(grammar) = self.language.grammar() else {
            return ParseOutcome::NoGrammar;
        };
        if self.parser.set_language(&grammar.ts_language).is_err() {
            return ParseOutcome::NoGrammar;
        }

        let started = Instant::now();
        let rope = self.snapshot.rope().clone();
        let tree = {
            let mut progress = |_: &tree_sitter::ParseState| {
                if cancel.is_cancelled()
                    || deadline.is_some_and(|deadline| started.elapsed() > deadline)
                {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            let options = ParseOptions::new().progress_callback(&mut progress);
            self.parser.parse_with_options(
                &mut rope_chunks(&rope),
                self.tree.as_ref(),
                Some(options),
            )
        };
        let duration = started.elapsed();

        let Some(tree) = tree else {
            return ParseOutcome::Cancelled;
        };
        self.tree = Some(tree);
        self.parsed_version = self.snapshot.version();
        self.dirty = false;
        self.injections.clear();
        self.build_injections();
        ParseOutcome::Parsed {
            duration,
            within_budget: duration <= PARSE_BUDGET_HINT,
        }
    }

    /// Highlight spans covering `range`, in ascending, non-overlapping order.
    ///
    /// Only the requested range is queried, so this is the call the editor
    /// makes for the visible rows. Returns an empty vector when there is no
    /// tree yet (plain text).
    pub fn highlights(&self, range: Range<usize>) -> Vec<(Range<usize>, HighlightId)> {
        let Some(tree) = self.tree.as_ref() else {
            return Vec::new();
        };
        let Ok(grammar) = self.language.grammar() else {
            return Vec::new();
        };
        let len = self.snapshot.len();
        let range = range.start.min(len)..range.end.min(len);
        if range.is_empty() {
            return Vec::new();
        }

        let rope = self.snapshot.rope();
        let mut spans = flatten(collect_captures(
            grammar,
            tree.root_node(),
            RopeText(rope),
            range.clone(),
            0,
        ));

        for layer in &self.injections {
            if layer.range.end <= range.start || layer.range.start >= range.end {
                continue;
            }
            let Ok(inner) = layer.language.grammar() else {
                continue;
            };
            let local_start = range.start.saturating_sub(layer.range.start);
            let local_end = (range.end.min(layer.range.end)) - layer.range.start;
            let inner_spans = flatten(collect_captures(
                inner,
                layer.tree.root_node(),
                layer.text.as_bytes(),
                local_start..local_end,
                layer.range.start,
            ));
            spans = splice(spans, layer.range.clone(), inner_spans);
        }
        spans
    }

    /// Parses every injected region of the current tree. Runs as part of
    /// `reparse`, on the background executor, so `highlights` stays `&self`.
    fn build_injections(&mut self) {
        if MAX_INJECTION_DEPTH == 0 {
            return;
        }
        let Some(tree) = self.tree.as_ref() else {
            return;
        };
        let Ok(grammar) = self.language.grammar() else {
            return;
        };
        if !grammar.injects_other_languages {
            return;
        }
        let (Some(query), Some(content_ix)) =
            (grammar.injections.as_ref(), grammar.injection_content)
        else {
            return;
        };

        let rope = self.snapshot.rope();
        let mut regions: Vec<(Range<usize>, String)> = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), RopeText(rope));
        while let Some(m) = matches.next() {
            let mut content: Option<Node> = None;
            let mut language: Option<String> = None;
            for capture in m.captures() {
                if capture.index == content_ix {
                    content = Some(capture.node);
                } else if Some(capture.index) == grammar.injection_language {
                    language = Some(node_text(rope, capture.node));
                }
            }
            // `(#set! injection.language "html")` wins when there is no
            // `@injection.language` capture.
            if language.is_none() {
                language = query
                    .property_settings(m.pattern_index)
                    .iter()
                    .find(|property| &*property.key == "injection.language")
                    .and_then(|property| property.value.as_ref())
                    .map(|value| value.to_string());
            }
            if let (Some(node), Some(name)) = (content, language) {
                let range = node.byte_range();
                if !range.is_empty() {
                    regions.push((range, name));
                }
            }
        }

        for (range, name) in regions {
            let Some(language) = self.registry.language(name.trim()) else {
                continue;
            };
            if language.name() == self.language.name() {
                // Same-language injections (Rust macros) add nothing here and
                // would double the work.
                continue;
            }
            let Ok(inner) = language.grammar() else {
                continue;
            };
            let text = self.snapshot.text_in(range.clone());
            let mut parser = Parser::new();
            if parser.set_language(&inner.ts_language).is_err() {
                continue;
            }
            let Some(tree) = parser.parse(text.as_bytes(), None) else {
                continue;
            };
            self.injections.push(InjectionLayer {
                language,
                range,
                text,
                tree,
            });
        }
    }
}

impl std::fmt::Debug for SyntaxState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyntaxState")
            .field("language", &self.language.name())
            .field("version", &self.snapshot.version())
            .field("parsed_version", &self.parsed_version())
            .field("dirty", &self.dirty)
            .field("injections", &self.injections.len())
            .finish()
    }
}

/// One capture, before overlaps are resolved.
struct Capture {
    range: Range<usize>,
    id: HighlightId,
    /// Query pattern index: for the same node the *later* pattern wins, as in
    /// `tree-sitter-highlight`.
    pattern: usize,
}

fn collect_captures<I: AsRef<[u8]>, T: tree_sitter::TextProvider<I>>(
    grammar: &Grammar,
    root: Node<'_>,
    text: T,
    range: Range<usize>,
    offset: usize,
) -> Vec<Capture> {
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(range);
    let mut out = Vec::new();
    let mut matches = cursor.matches(&grammar.highlights, root, text);
    while let Some(m) = matches.next() {
        for capture in m.captures() {
            let Some(Some(id)) = grammar.capture_map.get(capture.index as usize).copied() else {
                continue;
            };
            let node_range = capture.node.byte_range();
            if node_range.is_empty() {
                continue;
            }
            out.push(Capture {
                range: node_range.start + offset..node_range.end + offset,
                id,
                pattern: m.pattern_index,
            });
        }
    }
    out
}

/// Turns overlapping captures into a flat, ascending, non-overlapping list.
///
/// Nesting wins (the innermost capture colours its span) and, for one and the
/// same span, the later query pattern wins: the two rules `tree-sitter-highlight`
/// applies.
fn flatten(mut captures: Vec<Capture>) -> Vec<(Range<usize>, HighlightId)> {
    captures.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then(b.range.end.cmp(&a.range.end))
            .then(a.pattern.cmp(&b.pattern))
    });

    let mut out: Vec<(Range<usize>, HighlightId)> = Vec::with_capacity(captures.len());
    let mut stack: Vec<(usize, HighlightId)> = Vec::new();
    let mut pos = 0usize;

    for capture in captures {
        while let Some(&(end, id)) = stack.last() {
            if end <= capture.range.start {
                if pos < end {
                    push_span(&mut out, pos..end, id);
                    pos = end;
                }
                stack.pop();
            } else {
                break;
            }
        }
        if let Some(&(_, id)) = stack.last()
            && pos < capture.range.start
        {
            push_span(&mut out, pos..capture.range.start, id);
        }
        pos = pos.max(capture.range.start);
        match stack.last_mut() {
            // Same span as the capture already on top: the later pattern wins.
            Some((end, id)) if *end == capture.range.end && pos == capture.range.start => {
                *id = capture.id;
            }
            _ => stack.push((capture.range.end, capture.id)),
        }
    }

    while let Some((end, id)) = stack.pop() {
        if pos < end {
            push_span(&mut out, pos..end, id);
            pos = end;
        }
    }
    out
}

fn push_span(out: &mut Vec<(Range<usize>, HighlightId)>, range: Range<usize>, id: HighlightId) {
    if range.is_empty() {
        return;
    }
    if let Some((last_range, last_id)) = out.last_mut()
        && *last_id == id
        && last_range.end == range.start
    {
        last_range.end = range.end;
        return;
    }
    out.push((range, id));
}

/// Replaces whatever `outer` says about `region` with `inner`.
fn splice(
    outer: Vec<(Range<usize>, HighlightId)>,
    region: Range<usize>,
    inner: Vec<(Range<usize>, HighlightId)>,
) -> Vec<(Range<usize>, HighlightId)> {
    let mut out = Vec::with_capacity(outer.len() + inner.len());
    for (range, id) in outer {
        if range.end <= region.start || range.start >= region.end {
            out.push((range, id));
            continue;
        }
        if range.start < region.start {
            out.push((range.start..region.start, id));
        }
        if range.end > region.end {
            out.push((region.end..range.end, id));
        }
    }
    out.extend(inner);
    out.sort_by_key(|(range, _)| (range.start, range.end));
    out
}

/// Builds the `InputEdit` for one replaced range.
///
/// `old` and `new` are the snapshots before and after the step. The event
/// ranges are applied in ascending order, so the text before `new_range.start`
/// is the same in both snapshots and the start position can be read from
/// either one.
fn input_edit(
    old: &BufferSnapshot,
    new: &BufferSnapshot,
    old_range: &Range<usize>,
    new_range: &Range<usize>,
) -> InputEdit {
    let start_position = ts_point(new.offset_to_point(new_range.start));
    let old_len = old_range.end - old_range.start;

    // Where the replaced text ended, expressed in the frame that still has the
    // old text but already has the previous edits of this step applied.
    let old_start_point = old.offset_to_point(old_range.start);
    let old_end_point = old.offset_to_point(old_range.end);
    let old_end_position = if old_end_point.row == old_start_point.row {
        TsPoint::new(start_position.row, start_position.column + old_len)
    } else {
        TsPoint::new(
            start_position.row + (old_end_point.row - old_start_point.row) as usize,
            old_end_point.column as usize,
        )
    };

    InputEdit {
        start_byte: new_range.start,
        old_end_byte: new_range.start + old_len,
        new_end_byte: new_range.end,
        start_position,
        old_end_position,
        new_end_position: ts_point(new.offset_to_point(new_range.end)),
    }
}

fn ts_point(point: TextPoint) -> TsPoint {
    TsPoint::new(point.row as usize, point.column as usize)
}

/// Feeds the parser straight from the rope, without materializing the text.
fn rope_chunks<'a>(rope: &'a Rope) -> impl FnMut(usize, TsPoint) -> &'a str + 'a {
    move |byte, _| {
        if byte >= rope.len_bytes() {
            return "";
        }
        let (chunk, chunk_start, _, _) = rope.chunk_at_byte(byte);
        &chunk[byte - chunk_start..]
    }
}

/// `TextProvider` over a rope, for query predicates (`#match?`, `#eq?`).
struct RopeText<'a>(&'a Rope);

/// Adapts ropey's `&str` chunks to the `&[u8]` tree-sitter wants.
struct ChunkBytes<'a>(ropey::iter::Chunks<'a>);

impl<'a> Iterator for ChunkBytes<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(str::as_bytes)
    }
}

impl<'a> tree_sitter::TextProvider<&'a [u8]> for RopeText<'a> {
    type I = ChunkBytes<'a>;

    fn text(&mut self, node: Node) -> Self::I {
        let range = node.byte_range();
        let end = range.end.min(self.0.len_bytes());
        let start = range.start.min(end);
        ChunkBytes(self.0.byte_slice(start..end).chunks())
    }
}

fn node_text(rope: &Rope, node: Node<'_>) -> String {
    let range = node.byte_range();
    let end = range.end.min(rope.len_bytes());
    let start = range.start.min(end);
    rope.byte_slice(start..end).to_string()
}
