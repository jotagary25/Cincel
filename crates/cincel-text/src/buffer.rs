//! The text buffer: rope, version, anchors, transactions, undo/redo, events.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ropey::{Rope, RopeSlice};

use crate::anchor::{Anchor, AnchorId, AnchorSet, Bias};

/// UTF-8 BOM.
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Window in which consecutive [`EditSource::User`] transactions are merged
/// into a single undo unit.
pub const DEFAULT_GROUP_INTERVAL: Duration = Duration::from_millis(300);

/// A position in the buffer: `row` is a 0-based line index, `column` is a
/// 0-based **byte** offset inside that line (always on a `char` boundary).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    /// 0-based line index.
    pub row: u32,
    /// 0-based byte offset inside the line.
    pub column: u32,
}

impl Point {
    /// Builds a new point.
    pub fn new(row: u32, column: u32) -> Self {
        Self { row, column }
    }
}

/// Who produced an edit. Every closed transaction carries one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditSource {
    /// Typed by the person using the editor.
    User,
    /// Written by an agent during the given turn.
    Agent {
        /// Turn the edit belongs to.
        turn_id: u64,
    },
    /// Produced by accepting or rejecting a review hunk.
    Review,
    /// Initial content, or a reload from disk.
    Load,
}

impl EditSource {
    /// Whether two sources may be merged into one undo unit.
    fn groups_with(self, other: Self) -> bool {
        matches!((self, other), (EditSource::User, EditSource::User))
    }
}

/// Line terminator of the file this buffer came from. In memory the rope always
/// holds `\n`; the terminator is re-applied by [`Buffer::to_bytes`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineEnding {
    /// `\n`.
    #[default]
    Lf,
    /// `\r\n`.
    Crlf,
}

impl LineEnding {
    /// The bytes this terminator writes.
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }

    /// Detects the terminator from the first line break in `text`.
    pub fn detect(text: &str) -> Self {
        match text.find('\n') {
            Some(0) => LineEnding::Lf,
            Some(ix) if text.as_bytes()[ix - 1] == b'\r' => LineEnding::Crlf,
            _ => LineEnding::Lf,
        }
    }
}

/// Why a file could not be loaded into a buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoadError {
    /// The bytes are not valid UTF-8. v1 does not transcode: the file must be
    /// opened read-only with a warning.
    NotUtf8,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::NotUtf8 => write!(f, "the file is not valid UTF-8"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Why an edit was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditError {
    /// The buffer is read-only (see [`Buffer::set_read_only`]). v1 opens files
    /// that are not valid UTF-8 this way.
    ReadOnly,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::ReadOnly => write!(f, "the buffer is read-only"),
        }
    }
}

impl std::error::Error for EditError {}

/// What a buffer tells its subscribers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BufferEvent {
    /// One `edit`, `undo` or `redo` step was applied.
    ///
    /// `old_ranges` are the replaced byte ranges in the text *before* this
    /// step, ascending and disjoint. `new_ranges` are the ranges the
    /// replacement occupies in the text *after* it, in the same order.
    Edited {
        /// Source of the transaction this step belongs to.
        source: EditSource,
        /// Replaced ranges, in pre-edit coordinates.
        old_ranges: Vec<Range<usize>>,
        /// Inserted ranges, in post-edit coordinates.
        new_ranges: Vec<Range<usize>>,
        /// Buffer version *after* this step.
        version: u64,
    },
}

/// Handle returned by [`Buffer::subscribe`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubscriptionId(u64);

/// A registered event callback. The `Send` bound keeps [`Buffer`] itself
/// `Send`, so a `BufferStore` can hold one behind a `Mutex`.
type Subscriber = (SubscriptionId, Box<dyn FnMut(&BufferEvent) + Send>);

/// One replacement inside a transaction, stored so it can be undone.
#[derive(Clone, Debug)]
struct EditOp {
    start: usize,
    old_text: String,
    new_text: String,
}

/// A closed undo unit.
#[derive(Clone, Debug)]
struct Transaction {
    source: EditSource,
    ops: Vec<EditOp>,
    closed_at: Instant,
}

/// An immutable, cheap-to-clone, `Send` view of a buffer.
///
/// Cloning is a rope clone: the tree nodes are shared copy-on-write, so it is
/// O(1)-ish regardless of the file size.
#[derive(Clone, Debug)]
pub struct BufferSnapshot {
    rope: Rope,
    version: u64,
    line_ending: LineEnding,
    bom: bool,
    /// Lazily computed content hash, see [`BufferSnapshot::content_hash`].
    hash: OnceLock<u64>,
}

impl Default for BufferSnapshot {
    fn default() -> Self {
        Self {
            rope: Rope::new(),
            version: 0,
            line_ending: LineEnding::default(),
            bom: false,
            hash: OnceLock::new(),
        }
    }
}

impl BufferSnapshot {
    /// Monotonic version of the buffer this snapshot was taken from.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Total length in bytes.
    pub fn len(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }

    /// The underlying rope, for consumers that want chunk-level access
    /// (tree-sitter parsing, diffing).
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Whether both snapshots share the very same rope allocation.
    ///
    /// O(1) and allocation-free: two snapshots of the same buffer taken with
    /// no edit in between are the same instance. `false` is *not* proof that
    /// the texts differ, only that the fast path did not hit; use
    /// [`BufferSnapshot::same_text_as`] for the real answer.
    pub fn ptr_eq(&self, other: &BufferSnapshot) -> bool {
        self.rope.is_instance(&other.rope)
    }

    /// Whether both snapshots hold the same text.
    ///
    /// Cost: O(1) when [`BufferSnapshot::ptr_eq`] hits or the lengths differ;
    /// otherwise the [`content_hash`](BufferSnapshot::content_hash) of each
    /// side, which is O(n) the first time per snapshot and O(1) afterwards.
    /// Equal hashes are confirmed with a chunk comparison, so there are no
    /// false positives.
    pub fn same_text_as(&self, other: &BufferSnapshot) -> bool {
        if self.ptr_eq(other) {
            return true;
        }
        if self.len() != other.len() {
            return false;
        }
        self.content_hash() == other.content_hash() && self.rope == other.rope
    }

    /// A hash of the text, computed on first use and cached in the snapshot.
    ///
    /// O(n) once (a `SipHash` pass over the rope chunks), O(1) on every later
    /// call on the same snapshot, including clones made after it was
    /// computed. Two snapshots with the same text always have the same hash;
    /// it is not a cryptographic digest, so compare texts with
    /// [`BufferSnapshot::same_text_as`] rather than the hash alone.
    pub fn content_hash(&self) -> u64 {
        *self.hash.get_or_init(|| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.rope.len_bytes().hash(&mut hasher);
            for chunk in self.rope.chunks() {
                hasher.write(chunk.as_bytes());
            }
            hasher.finish()
        })
    }

    /// Line terminator to write on save.
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// Whether the file started with a UTF-8 BOM.
    pub fn has_bom(&self) -> bool {
        self.bom
    }

    /// Number of lines. A buffer ending in `\n` has a final empty line.
    pub fn line_count(&self) -> u32 {
        self.rope.len_lines() as u32
    }

    /// The whole text.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// The text of a byte range.
    pub fn text_in(&self, range: Range<usize>) -> String {
        let (start, end) = self.clamp_range(range);
        self.rope.byte_slice(start..end).to_string()
    }

    /// The raw slice of `row`, trailing newline included.
    pub fn line(&self, row: u32) -> RopeSlice<'_> {
        if row >= self.line_count() {
            return RopeSlice::from("");
        }
        self.rope.line(row as usize)
    }

    /// The text of `row` without its trailing newline.
    pub fn line_text(&self, row: u32) -> String {
        let line = self.line(row);
        let mut text = line.to_string();
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }

    /// Length of `row` in bytes, excluding the trailing newline.
    pub fn line_len(&self, row: u32) -> u32 {
        if row >= self.line_count() {
            return 0;
        }
        let line = self.rope.line(row as usize);
        let mut len = line.len_bytes();
        if line.len_chars() > 0 && line.char(line.len_chars() - 1) == '\n' {
            len -= 1;
        }
        len as u32
    }

    /// Byte offset of the first byte of `row`.
    pub fn line_start_offset(&self, row: u32) -> usize {
        let row = row.min(self.line_count().saturating_sub(1));
        self.rope.line_to_byte(row as usize)
    }

    /// Converts a point into a byte offset, clipping it first.
    pub fn point_to_offset(&self, point: Point) -> usize {
        let point = self.clip_point(point);
        self.line_start_offset(point.row) + point.column as usize
    }

    /// Converts a byte offset into a point.
    pub fn offset_to_point(&self, offset: usize) -> Point {
        let offset = offset.min(self.len());
        let row = self.rope.byte_to_line(offset) as u32;
        let column = (offset - self.line_start_offset(row)) as u32;
        Point { row, column }
    }

    /// Clamps `point` to a valid position, snapping the column down to a
    /// `char` boundary.
    pub fn clip_point(&self, point: Point) -> Point {
        let row = point.row.min(self.line_count().saturating_sub(1));
        let line = self.line_text(row);
        let mut column = point.column.min(line.len() as u32) as usize;
        while column > 0 && !line.is_char_boundary(column) {
            column -= 1;
        }
        Point {
            row,
            column: column as u32,
        }
    }

    /// Clamps `offset` to the buffer and snaps it down to a `char` boundary.
    pub fn clip_offset(&self, offset: usize) -> usize {
        let offset = offset.min(self.len());
        let char_ix = self.rope.byte_to_char(offset);
        self.rope.char_to_byte(char_ix)
    }

    /// Converts a byte offset into a UTF-16 code-unit offset.
    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        let offset = offset.min(self.len());
        self.rope.char_to_utf16_cu(self.rope.byte_to_char(offset))
    }

    /// Converts a UTF-16 code-unit offset into a byte offset.
    pub fn offset_from_utf16(&self, offset_utf16: usize) -> usize {
        let clamped = offset_utf16.min(self.rope.len_utf16_cu());
        self.rope.char_to_byte(self.rope.utf16_cu_to_char(clamped))
    }

    /// Converts a byte range into a UTF-16 range.
    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    /// Converts a UTF-16 range into a byte range.
    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    /// Previous `char` boundary before `offset`.
    pub fn previous_char_boundary(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }
        let char_ix = self.rope.byte_to_char(offset);
        let char_ix = if self.rope.char_to_byte(char_ix) < offset {
            char_ix
        } else {
            char_ix.saturating_sub(1)
        };
        self.rope.char_to_byte(char_ix)
    }

    /// Next `char` boundary after `offset`.
    pub fn next_char_boundary(&self, offset: usize) -> usize {
        if offset >= self.len() {
            return self.len();
        }
        let char_ix = self.rope.byte_to_char(offset);
        self.rope
            .char_to_byte((char_ix + 1).min(self.rope.len_chars()))
    }

    /// The bytes to write to disk: BOM (if any) plus the text with the file's
    /// line terminator re-applied.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.rope.len_bytes() + BOM.len());
        if self.bom {
            out.extend_from_slice(&BOM);
        }
        match self.line_ending {
            LineEnding::Lf => {
                for chunk in self.rope.chunks() {
                    out.extend_from_slice(chunk.as_bytes());
                }
            }
            LineEnding::Crlf => {
                for chunk in self.rope.chunks() {
                    for byte in chunk.bytes() {
                        if byte == b'\n' {
                            out.push(b'\r');
                        }
                        out.push(byte);
                    }
                }
            }
        }
        out
    }

    fn clamp_range(&self, range: Range<usize>) -> (usize, usize) {
        let start = self.clip_offset(range.start);
        let end = self.clip_offset(range.end.max(start));
        (start, end)
    }
}

/// A text buffer.
///
/// # Events
///
/// The buffer has **no** dependency on GPUI, so it notifies its owner through
/// plain callbacks: [`Buffer::subscribe`] registers a `FnMut(&BufferEvent)`
/// that runs synchronously right after each edit step (one event per `edit`,
/// `undo` or `redo`, *not* one per transaction, so that consumers such as
/// `cincel-syntax` can turn every event into a valid tree-sitter
/// `InputEdit`). A callback cannot touch the buffer while it runs; it is meant
/// to forward the event into whatever queue the owner uses (a GPUI
/// `cx.emit`, a channel, an `AnchorMap`).
///
/// A pull variant is available too: [`Buffer::record_events`] turns on an
/// internal queue that the owner empties with [`Buffer::drain_events`]. It
/// costs nothing when it is off, which is the default.
pub struct Buffer {
    snapshot: BufferSnapshot,
    /// Anchors the buffer itself tracks (see [`Buffer::track_anchor`]).
    anchors: AnchorSet,
    /// Open transaction: source, recorded ops and nesting depth.
    open: Option<(EditSource, Vec<EditOp>)>,
    depth: u32,
    undo_stack: Vec<Transaction>,
    redo_stack: Vec<Transaction>,
    group_interval: Duration,
    subscribers: Vec<Subscriber>,
    next_subscription: u64,
    /// `Some` while [`Buffer::record_events`] is on.
    queue: Option<Vec<BufferEvent>>,
    /// The contents as they were last written to disk. Keeping the snapshot
    /// (a rope clone, so O(1) and structurally shared) is what lets
    /// [`Buffer::is_dirty`] answer by *content* instead of by version: undoing
    /// back to the saved text bumps the version but restores the text.
    saved: BufferSnapshot,
    /// When set, every edit is refused (see [`Buffer::set_read_only`]).
    read_only: bool,
}

impl fmt::Debug for Buffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Buffer")
            .field("version", &self.snapshot.version)
            .field("len", &self.snapshot.len())
            .field("line_ending", &self.snapshot.line_ending)
            .field("bom", &self.snapshot.bom)
            .field("saved_version", &self.saved.version())
            .field("read_only", &self.read_only)
            .field("anchors", &self.anchors.len())
            .field("undo_stack", &self.undo_stack.len())
            .field("redo_stack", &self.redo_stack.len())
            .finish()
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new("")
    }
}

impl Buffer {
    /// Creates a buffer from `text`. CRLF and lone CR are normalized to LF and
    /// the detected terminator is remembered for [`Buffer::to_bytes`].
    pub fn new(text: &str) -> Self {
        let line_ending = LineEnding::detect(text);
        let mut buffer = Self::empty();
        buffer.snapshot.rope = Rope::from_str(&normalize(text));
        buffer.snapshot.line_ending = line_ending;
        // What we were handed is what is on disk: a fresh buffer is clean.
        buffer.mark_saved();
        buffer
    }

    /// Loads a buffer from raw file bytes: strips a UTF-8 BOM, rejects invalid
    /// UTF-8 and detects the line terminator.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LoadError> {
        let (bom, rest) = if bytes.starts_with(&BOM) {
            (true, &bytes[BOM.len()..])
        } else {
            (false, bytes)
        };
        let text = std::str::from_utf8(rest).map_err(|_| LoadError::NotUtf8)?;
        let mut buffer = Self::new(text);
        buffer.snapshot.bom = bom;
        buffer.mark_saved();
        Ok(buffer)
    }

    /// Loads a buffer from raw file bytes, replacing any invalid UTF-8 byte
    /// with `U+FFFD` instead of failing.
    ///
    /// Returns the buffer and whether anything had to be replaced. When it
    /// did, the buffer comes back **read-only**, as `modulos/text.md` asks:
    /// v1 does not transcode, so saving it would corrupt the file. Clear the
    /// flag with [`Buffer::set_read_only`] if the caller really wants to
    /// rewrite the file as UTF-8.
    pub fn from_bytes_lossy(bytes: &[u8]) -> (Self, bool) {
        let (bom, rest) = if bytes.starts_with(&BOM) {
            (true, &bytes[BOM.len()..])
        } else {
            (false, bytes)
        };
        let text = String::from_utf8_lossy(rest);
        let had_invalid = matches!(text, std::borrow::Cow::Owned(_));
        let mut buffer = Self::new(&text);
        buffer.snapshot.bom = bom;
        buffer.read_only = had_invalid;
        buffer.mark_saved();
        (buffer, had_invalid)
    }

    fn empty() -> Self {
        Self {
            snapshot: BufferSnapshot::default(),
            anchors: AnchorSet::new(),
            open: None,
            depth: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            group_interval: DEFAULT_GROUP_INTERVAL,
            subscribers: Vec::new(),
            next_subscription: 0,
            queue: None,
            saved: BufferSnapshot::default(),
            read_only: false,
        }
    }

    // ---------------------------------------------------------------- reading

    /// A cheap, `Send` copy of the current contents.
    pub fn snapshot(&self) -> BufferSnapshot {
        self.snapshot.clone()
    }

    /// Monotonic version, bumped on every edit step.
    pub fn version(&self) -> u64 {
        self.snapshot.version
    }

    /// Total length in bytes.
    pub fn len_bytes(&self) -> usize {
        self.snapshot.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.snapshot.is_empty()
    }

    /// Number of lines.
    pub fn line_count(&self) -> u32 {
        self.snapshot.line_count()
    }

    /// The whole text.
    pub fn text(&self) -> String {
        self.snapshot.text()
    }

    /// The text of a byte range.
    pub fn text_in(&self, range: Range<usize>) -> String {
        self.snapshot.text_in(range)
    }

    /// The raw slice of `row`, trailing newline included.
    pub fn line(&self, row: u32) -> RopeSlice<'_> {
        self.snapshot.line(row)
    }

    /// The text of `row` without its trailing newline.
    pub fn line_text(&self, row: u32) -> String {
        self.snapshot.line_text(row)
    }

    /// Length of `row` in bytes, excluding the trailing newline.
    pub fn line_len(&self, row: u32) -> u32 {
        self.snapshot.line_len(row)
    }

    /// Byte offset of the first byte of `row`.
    pub fn line_start_offset(&self, row: u32) -> usize {
        self.snapshot.line_start_offset(row)
    }

    /// Converts a point into a byte offset, clipping it first.
    pub fn point_to_offset(&self, point: Point) -> usize {
        self.snapshot.point_to_offset(point)
    }

    /// Converts a byte offset into a point.
    pub fn offset_to_point(&self, offset: usize) -> Point {
        self.snapshot.offset_to_point(offset)
    }

    /// Clamps `point` to a valid position on a `char` boundary.
    pub fn clip_point(&self, point: Point) -> Point {
        self.snapshot.clip_point(point)
    }

    /// Clamps `offset` to the buffer and snaps it to a `char` boundary.
    pub fn clip_offset(&self, offset: usize) -> usize {
        self.snapshot.clip_offset(offset)
    }

    /// Converts a byte offset into a UTF-16 code-unit offset.
    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        self.snapshot.offset_to_utf16(offset)
    }

    /// Converts a UTF-16 code-unit offset into a byte offset.
    pub fn offset_from_utf16(&self, offset_utf16: usize) -> usize {
        self.snapshot.offset_from_utf16(offset_utf16)
    }

    /// Converts a byte range into a UTF-16 range.
    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.snapshot.range_to_utf16(range)
    }

    /// Converts a UTF-16 range into a byte range.
    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.snapshot.range_from_utf16(range)
    }

    /// Previous `char` boundary before `offset`.
    pub fn previous_char_boundary(&self, offset: usize) -> usize {
        self.snapshot.previous_char_boundary(offset)
    }

    /// Next `char` boundary after `offset`.
    pub fn next_char_boundary(&self, offset: usize) -> usize {
        self.snapshot.next_char_boundary(offset)
    }

    // ------------------------------------------------- encoding / line ending

    /// Line terminator to write on save.
    pub fn line_ending(&self) -> LineEnding {
        self.snapshot.line_ending
    }

    /// Overrides the line terminator written on save.
    pub fn set_line_ending(&mut self, line_ending: LineEnding) {
        self.snapshot.line_ending = line_ending;
    }

    /// Whether the file started with a UTF-8 BOM.
    pub fn has_bom(&self) -> bool {
        self.snapshot.bom
    }

    /// Sets whether a UTF-8 BOM is written on save.
    pub fn set_bom(&mut self, bom: bool) {
        self.snapshot.bom = bom;
    }

    /// The bytes to write to disk. A file loaded with [`Buffer::from_bytes`]
    /// and saved unchanged produces identical bytes (BOM and CRLF included),
    /// as long as its terminators were uniform.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.snapshot.to_bytes()
    }

    // ------------------------------------------------------- saved / read-only

    /// The version the contents were last saved at. A fresh buffer starts
    /// clean at version 0.
    pub fn saved_version(&self) -> u64 {
        self.saved.version()
    }

    /// The contents as they were last saved.
    pub fn saved_snapshot(&self) -> &BufferSnapshot {
        &self.saved
    }

    /// Records that the current contents are the ones on disk. Call it after
    /// writing the file and after reloading it.
    pub fn mark_saved(&mut self) {
        self.saved = self.snapshot.clone();
    }

    /// Whether the text differs from what was last saved.
    ///
    /// This is a question about **content**, not about the version: undoing
    /// back to the text on disk leaves the file clean even though every undo
    /// bumps the version. Cost, in order: O(1) when the version has not moved,
    /// O(1) when the snapshots still share their rope, O(1) when the lengths
    /// differ, and otherwise the cached
    /// [`content_hash`](BufferSnapshot::content_hash) of each side — computed
    /// at most once per snapshot, i.e. once per edit.
    pub fn is_dirty(&self) -> bool {
        self.snapshot.version != self.saved.version() && !self.snapshot.same_text_as(&self.saved)
    }

    /// Whether the text is exactly the one that was last saved. The inverse of
    /// [`Buffer::is_dirty`], for call sites that read better this way.
    pub fn content_matches_saved(&self) -> bool {
        !self.is_dirty()
    }

    /// Whether edits are refused.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Makes the buffer refuse (or accept again) edits.
    ///
    /// While it is set, [`Buffer::try_edit`] and [`Buffer::try_edit_many`]
    /// return [`EditError::ReadOnly`], the infallible `edit` family logs a
    /// `tracing::warn` and does nothing, and `undo`/`redo` return `None`.
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    // ---------------------------------------------------------------- anchors

    /// A `Before`-biased anchor at `offset`, valid at the current version.
    pub fn anchor_before(&self, offset: usize) -> Anchor {
        Anchor::before(self.clip_offset(offset))
    }

    /// An `After`-biased anchor at `offset`, valid at the current version.
    pub fn anchor_after(&self, offset: usize) -> Anchor {
        Anchor::after(self.clip_offset(offset))
    }

    /// The byte offset of `anchor`, clipped to the current contents.
    pub fn resolve(&self, anchor: &Anchor) -> usize {
        self.clip_offset(anchor.offset)
    }

    /// Registers an anchor in the buffer's own [`AnchorSet`], so it is
    /// transformed by every edit. Returns a stable handle.
    pub fn track_anchor(&mut self, offset: usize, bias: Bias) -> AnchorId {
        let anchor = Anchor::new(self.clip_offset(offset), bias);
        self.anchors.add(anchor)
    }

    /// The current anchor behind a handle from [`Buffer::track_anchor`].
    pub fn tracked_anchor(&self, id: AnchorId) -> Option<Anchor> {
        self.anchors.anchor(id)
    }

    /// The current offset of a tracked anchor.
    pub fn resolve_tracked(&self, id: AnchorId) -> Option<usize> {
        self.anchors.resolve(id)
    }

    /// Drops a tracked anchor.
    pub fn untrack_anchor(&mut self, id: AnchorId) -> Option<Anchor> {
        self.anchors.remove(id).map(|(anchor, ())| anchor)
    }

    /// The buffer's own anchor set.
    pub fn anchors(&self) -> &AnchorSet {
        &self.anchors
    }

    // ----------------------------------------------------------------- events

    /// Turns the internal event queue on or off. Turning it off drops whatever
    /// it still held.
    pub fn record_events(&mut self, record: bool) {
        self.queue = record.then(Vec::new);
    }

    /// Whether the internal event queue is on.
    pub fn records_events(&self) -> bool {
        self.queue.is_some()
    }

    /// Empties the internal event queue, in edit order. Always empty unless
    /// [`Buffer::record_events`] was turned on.
    pub fn drain_events(&mut self) -> Vec<BufferEvent> {
        match self.queue.as_mut() {
            Some(queue) => std::mem::take(queue),
            None => Vec::new(),
        }
    }

    /// Registers a callback invoked after every edit step. Returns a handle for
    /// [`Buffer::unsubscribe`].
    pub fn subscribe(&mut self, callback: Box<dyn FnMut(&BufferEvent) + Send>) -> SubscriptionId {
        let id = SubscriptionId(self.next_subscription);
        self.next_subscription += 1;
        self.subscribers.push((id, callback));
        id
    }

    /// Drops a subscription.
    pub fn unsubscribe(&mut self, id: SubscriptionId) {
        self.subscribers.retain(|(other, _)| *other != id);
    }

    // ----------------------------------------------------------- transactions

    /// The undo-grouping window for consecutive `User` transactions.
    pub fn group_interval(&self) -> Duration {
        self.group_interval
    }

    /// Overrides the undo-grouping window (tests, settings).
    pub fn set_group_interval(&mut self, interval: Duration) {
        self.group_interval = interval;
    }

    /// Opens a transaction. Nested calls are allowed; only the outermost
    /// [`Buffer::end_transaction`] closes the undo unit, and the source of the
    /// outermost call wins.
    pub fn start_transaction(&mut self, source: EditSource) {
        self.depth += 1;
        if self.open.is_none() {
            self.open = Some((source, Vec::new()));
        }
    }

    /// Closes the current transaction. Returns whether an undo unit was pushed
    /// (an empty transaction pushes nothing).
    pub fn end_transaction(&mut self) -> bool {
        if self.depth == 0 {
            return false;
        }
        self.depth -= 1;
        if self.depth > 0 {
            return false;
        }
        let Some((source, ops)) = self.open.take() else {
            return false;
        };
        self.push_transaction(source, ops)
    }

    /// Runs `body` inside a transaction with `source`.
    pub fn transact<R>(&mut self, source: EditSource, body: impl FnOnce(&mut Self) -> R) -> R {
        self.start_transaction(source);
        let result = body(self);
        self.end_transaction();
        result
    }

    /// Whether an undo unit is available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether a redo unit is available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Undoes the last transaction and returns its source.
    pub fn undo(&mut self) -> Option<EditSource> {
        self.undo_with_ranges().map(|(source, _)| source)
    }

    /// Redoes the last undone transaction and returns its source.
    pub fn redo(&mut self) -> Option<EditSource> {
        self.redo_with_ranges().map(|(source, _)| source)
    }

    /// [`Buffer::undo`] plus the byte ranges the restored text occupies
    /// afterwards, ascending.
    ///
    /// Use them to move the cursor, invalidate caches or scroll to what
    /// changed. Returns `None` when there is nothing to undo or the buffer is
    /// read-only.
    pub fn undo_with_ranges(&mut self) -> Option<(EditSource, Vec<Range<usize>>)> {
        if self.refuse_if_read_only("undo") {
            return None;
        }
        let transaction = self.undo_stack.pop()?;
        let mut affected: Vec<Range<usize>> = Vec::with_capacity(transaction.ops.len());
        for op in transaction.ops.iter().rev() {
            let range = op.start..op.start + op.new_text.len();
            shift_ranges(&mut affected, &range, op.old_text.len());
            let new = self.apply_step(&[range], &op.old_text, transaction.source, false);
            affected.extend(new);
        }
        affected.sort_by_key(|range| (range.start, range.end));
        let source = transaction.source;
        self.redo_stack.push(transaction);
        Some((source, affected))
    }

    /// [`Buffer::redo`] plus the byte ranges the re-applied text occupies
    /// afterwards, ascending.
    pub fn redo_with_ranges(&mut self) -> Option<(EditSource, Vec<Range<usize>>)> {
        if self.refuse_if_read_only("redo") {
            return None;
        }
        let transaction = self.redo_stack.pop()?;
        let mut affected: Vec<Range<usize>> = Vec::with_capacity(transaction.ops.len());
        for op in &transaction.ops {
            let range = op.start..op.start + op.old_text.len();
            shift_ranges(&mut affected, &range, op.new_text.len());
            let new = self.apply_step(&[range], &op.new_text, transaction.source, false);
            affected.extend(new);
        }
        affected.sort_by_key(|range| (range.start, range.end));
        let source = transaction.source;
        self.undo_stack.push(transaction);
        Some((source, affected))
    }

    // ------------------------------------------------------------------ edits

    /// Replaces every range in `ranges` with `new_text`.
    ///
    /// The ranges are clipped, sorted and must not overlap (overlapping ranges
    /// are merged). Outside a transaction the call is wrapped in an implicit
    /// [`EditSource::User`] one.
    ///
    /// On a read-only buffer this does nothing and logs a warning; use
    /// [`Buffer::try_edit`] when the caller wants to handle the refusal.
    pub fn edit(&mut self, ranges: &[Range<usize>], new_text: &str) {
        if self.refuse_if_read_only("edit") {
            return;
        }
        let implicit = self.open.is_none();
        if implicit {
            self.start_transaction(EditSource::User);
        }
        let source = self.open.as_ref().map_or(EditSource::User, |(s, _)| *s);
        let normalized = normalize(new_text);
        let ranges = self.prepare_ranges(ranges);
        let is_noop = normalized.is_empty() && ranges.iter().all(|range| range.is_empty());
        if !ranges.is_empty() && !is_noop {
            self.apply_step(&ranges, &normalized, source, true);
        }
        if implicit {
            self.end_transaction();
        }
    }

    /// Applies several **distinct** replacements as one undo step.
    ///
    /// Unlike [`Buffer::edit`], which puts the same text in every range, each
    /// entry carries its own text. The ranges may come in any order and must
    /// not overlap (an overlapping entry is dropped with a warning); they are
    /// clipped to the buffer and applied in ascending order with a running
    /// offset, which is the same thing as applying them from the end.
    ///
    /// One [`BufferEvent::Edited`] is emitted **per replacement**, so the
    /// "one event = one `InputEdit`" rule still holds, but all of them land in
    /// a single transaction, so one [`Buffer::undo`] takes them all back. When
    /// no transaction is open the call wraps itself in an implicit
    /// [`EditSource::User`] one.
    ///
    /// Returns the ranges the new text occupies afterwards, ascending.
    pub fn edit_many(&mut self, edits: &[(Range<usize>, &str)]) -> Vec<Range<usize>> {
        if self.refuse_if_read_only("edit_many") {
            return Vec::new();
        }
        let prepared = self.prepare_edits(edits);
        if prepared.is_empty() {
            return Vec::new();
        }

        let implicit = self.open.is_none();
        if implicit {
            self.start_transaction(EditSource::User);
        }
        let source = self.open.as_ref().map_or(EditSource::User, |(s, _)| *s);

        let mut applied = Vec::with_capacity(prepared.len());
        let mut delta: isize = 0;
        for (range, text) in &prepared {
            // Shift into the coordinates the previous replacements produced,
            // so every step is a real edit of the current buffer state and its
            // event is a valid `InputEdit`.
            let start = (range.start as isize + delta) as usize;
            let end = start + (range.end - range.start);
            delta += text.len() as isize - (range.end - range.start) as isize;
            let range = start..end;
            let ranges = self.apply_step(std::slice::from_ref(&range), text, source, true);
            applied.extend(ranges);
        }

        if implicit {
            self.end_transaction();
        }
        applied
    }

    /// [`Buffer::edit`] that reports a read-only buffer instead of ignoring
    /// the call.
    pub fn try_edit(&mut self, ranges: &[Range<usize>], new_text: &str) -> Result<(), EditError> {
        if self.read_only {
            return Err(EditError::ReadOnly);
        }
        self.edit(ranges, new_text);
        Ok(())
    }

    /// [`Buffer::edit_many`] that reports a read-only buffer instead of
    /// ignoring the call.
    pub fn try_edit_many(
        &mut self,
        edits: &[(Range<usize>, &str)],
    ) -> Result<Vec<Range<usize>>, EditError> {
        if self.read_only {
            return Err(EditError::ReadOnly);
        }
        Ok(self.edit_many(edits))
    }

    /// Turns the buffer into `text` with the **smallest** set of edits that
    /// gets there (see [`crate::diff::minimal_edits`]), in one transaction
    /// with `source`.
    ///
    /// This is how an agent write or a reload from disk should land: replacing
    /// the whole rope would move every anchor, flatten the undo history and
    /// destroy the review hunks, while a minimal diff leaves untouched regions
    /// literally untouched.
    ///
    /// Returns the ranges the new text occupies afterwards, ascending; empty
    /// when the text already matched.
    pub fn set_text_minimal(&mut self, text: &str, source: EditSource) -> Vec<Range<usize>> {
        if self.refuse_if_read_only("set_text_minimal") {
            return Vec::new();
        }
        let normalized = normalize(text);
        let current = self.text();
        let edits = crate::diff::minimal_edits(&current, &normalized);
        if edits.is_empty() {
            return Vec::new();
        }
        let borrowed: Vec<(Range<usize>, &str)> = edits
            .iter()
            .map(|(range, new_text)| (range.clone(), new_text.as_str()))
            .collect();
        self.start_transaction(source);
        let ranges = self.edit_many(&borrowed);
        self.end_transaction();
        ranges
    }

    /// Clips and sorts the entries of one [`Buffer::edit_many`] call, dropping
    /// overlapping ones.
    fn prepare_edits(&self, edits: &[(Range<usize>, &str)]) -> Vec<(Range<usize>, String)> {
        let mut prepared: Vec<(Range<usize>, String)> = edits
            .iter()
            .map(|(range, text)| {
                let start = self.clip_offset(range.start);
                let end = self.clip_offset(range.end.max(start));
                (start..end, normalize(text))
            })
            .filter(|(range, text)| !(range.is_empty() && text.is_empty()))
            .collect();
        prepared.sort_by_key(|(range, _)| (range.start, range.end));

        let mut out: Vec<(Range<usize>, String)> = Vec::with_capacity(prepared.len());
        for (range, text) in prepared {
            match out.last() {
                Some((last, _)) if range.start < last.end => {
                    tracing::warn!(
                        ?range,
                        previous = ?last,
                        "edit_many: overlapping range dropped"
                    );
                }
                _ => out.push((range, text)),
            }
        }
        out
    }

    /// Whether the buffer refuses edits, logging once per attempt.
    fn refuse_if_read_only(&self, operation: &str) -> bool {
        if self.read_only {
            tracing::warn!(operation, "edit ignored: the buffer is read-only");
        }
        self.read_only
    }

    /// Replaces a single byte range.
    pub fn replace(&mut self, range: Range<usize>, new_text: &str) {
        self.edit(&[range], new_text);
    }

    /// Inserts `text` at `offset`.
    pub fn insert(&mut self, offset: usize, text: &str) {
        let range = offset..offset;
        self.edit(std::slice::from_ref(&range), text);
    }

    /// Deletes a byte range.
    pub fn delete(&mut self, range: Range<usize>) {
        self.edit(&[range], "");
    }

    /// Replaces whole lines `rows` with `lines`, keeping the trailing newline
    /// structure intact. Used by the review "reject" operation.
    pub fn replace_rows(&mut self, rows: Range<u32>, lines: &[String]) {
        let line_count = self.line_count();
        let start_row = rows.start.min(line_count);
        let end_row = rows.end.min(line_count);
        let start = self.line_start_offset(start_row);
        let end = if end_row >= line_count {
            self.len_bytes()
        } else {
            self.line_start_offset(end_row)
        };
        let mut text = String::new();
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
        if end == self.len_bytes() && !lines.is_empty() && !self.ends_with_newline() {
            // The buffer did not end with a newline; do not introduce one.
            text.pop();
        }
        self.replace(start..end, &text);
    }

    fn ends_with_newline(&self) -> bool {
        let rope = &self.snapshot.rope;
        rope.len_chars() > 0 && rope.char(rope.len_chars() - 1) == '\n'
    }

    /// Clips, sorts and merges the ranges of one `edit` call.
    fn prepare_ranges(&self, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
        let mut prepared: Vec<Range<usize>> = ranges
            .iter()
            .map(|range| {
                let start = self.clip_offset(range.start);
                let end = self.clip_offset(range.end.max(start));
                start..end
            })
            .collect();
        prepared.sort_by_key(|range| (range.start, range.end));
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(prepared.len());
        for range in prepared {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        merged
    }

    /// Applies one atomic step: replaces every range with `new_text`, records
    /// the inverse (unless `record` is false, as in undo/redo), transforms
    /// anchors and emits one event.
    fn apply_step(
        &mut self,
        ranges: &[Range<usize>],
        new_text: &str,
        source: EditSource,
        record: bool,
    ) -> Vec<Range<usize>> {
        let mut old_ranges = Vec::with_capacity(ranges.len());
        let mut new_ranges = Vec::with_capacity(ranges.len());
        let mut old_texts = Vec::with_capacity(ranges.len());
        let mut delta: isize = 0;
        for range in ranges {
            old_texts.push(self.snapshot.text_in(range.clone()));
            let new_start = (range.start as isize + delta) as usize;
            old_ranges.push(range.clone());
            new_ranges.push(new_start..new_start + new_text.len());
            delta += new_text.len() as isize - (range.end - range.start) as isize;
        }

        // Apply back to front so that the untouched prefix keeps its offsets.
        for range in ranges.iter().rev() {
            let rope = &mut self.snapshot.rope;
            let char_start = rope.byte_to_char(range.start);
            let char_end = rope.byte_to_char(range.end);
            if char_start != char_end {
                rope.remove(char_start..char_end);
            }
            if !new_text.is_empty() {
                rope.insert(char_start, new_text);
            }
        }

        // Transform anchors front to back: each step works in the coordinates
        // the previous steps already produced.
        for (old, new) in old_ranges.iter().zip(new_ranges.iter()) {
            let old_len = old.end - old.start;
            self.anchors
                .apply_edit(new.start..new.start + old_len, new_text.len());
        }

        self.snapshot.version += 1;

        if record && let Some((_, ops)) = self.open.as_mut() {
            // `start` is stored in post-edit coordinates: undo replays the ops
            // back to front and redo front to back, and in both directions the
            // untouched neighbours keep those offsets.
            for (new, old_text) in new_ranges.iter().zip(old_texts.iter()) {
                ops.push(EditOp {
                    start: new.start,
                    old_text: old_text.clone(),
                    new_text: new_text.to_owned(),
                });
            }
        }

        // The text changed, so the cached content hash is stale.
        self.snapshot.hash = OnceLock::new();

        let event = BufferEvent::Edited {
            source,
            old_ranges,
            new_ranges: new_ranges.clone(),
            version: self.snapshot.version,
        };
        self.emit(&event);
        new_ranges
    }

    fn push_transaction(&mut self, source: EditSource, ops: Vec<EditOp>) -> bool {
        if ops.is_empty() {
            return false;
        }
        let now = Instant::now();
        self.redo_stack.clear();
        if let Some(last) = self.undo_stack.last_mut()
            && last.source.groups_with(source)
            && now.duration_since(last.closed_at) < self.group_interval
        {
            last.ops.extend(ops);
            last.closed_at = now;
            return true;
        }
        self.undo_stack.push(Transaction {
            source,
            ops,
            closed_at: now,
        });
        true
    }

    fn emit(&mut self, event: &BufferEvent) {
        if let Some(queue) = self.queue.as_mut() {
            queue.push(event.clone());
        }
        // Take the callbacks out so a callback can still hold `&Buffer`
        // indirectly; they are put back untouched.
        let mut subscribers: Vec<Subscriber> = std::mem::take(&mut self.subscribers);
        for (_, callback) in &mut subscribers {
            callback(event);
        }
        // A callback may have (un)subscribed; keep both sets.
        subscribers.append(&mut self.subscribers);
        self.subscribers = subscribers;
    }
}

/// Keeps ranges collected from earlier steps pointing at the same text after a
/// step replaced `replaced` with `new_len` bytes.
///
/// The ops of one transaction touch disjoint regions, so a collected range is
/// either entirely before the step (unchanged) or entirely after it (shifted
/// by the length difference).
fn shift_ranges(collected: &mut [Range<usize>], replaced: &Range<usize>, new_len: usize) {
    let old_len = replaced.end - replaced.start;
    if new_len == old_len {
        return;
    }
    let delta = new_len as isize - old_len as isize;
    for range in collected.iter_mut() {
        if range.start >= replaced.end {
            range.start = range.start.saturating_add_signed(delta);
            range.end = range.end.saturating_add_signed(delta);
        }
    }
}

/// Normalizes CRLF and lone CR to LF. The rope always holds LF.
fn normalize(text: &str) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_owned()
    }
}
