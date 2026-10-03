//! Comments for the agent on lines of the code (spec 09 §6.4).
//!
//! A comment is a range of buffer rows plus the user's text. It lives in the
//! [`ReviewStore`] next to the hunks, but it is independent of them:
//!
//! - **Anchors.** Each comment has a start anchor (`Before`, at the start of
//!   its first row) and an end anchor (`After`, at the end of its last row's
//!   text) in an [`AnchorMap`] per path, fed by the host with every
//!   [`BufferEvent`] of that buffer ([`ReviewStore::comment_buffer_event`]).
//!   When its rows disappear the comment collapses onto the nearest row.
//!   A comment made from a pure deletion has no rows (it sits before
//!   `rows.start`, where the red rows are drawn); when a reject puts those
//!   lines back, it covers them.
//! - **Decision marks.** Every accept/reject (hunk, line, file, turn, all)
//!   leaves an `accepted`/`rejected` mark on the comments of that file whose
//!   rows meet the decided rows; the marks of a reject live in its undo entry
//!   and `undo_last_reject` takes them back. Comments survive every decision.
//! - **State.** Computed when the prompt goes out
//!   ([`ReviewStore::take_comments_for_prompt`]): a pending hunk under the
//!   rows wins (`Pending`); otherwise the marks say `Accepted`, `Rejected`,
//!   `Mixed` (both) or `NoAgentChange` (none).
//! - **Sending.** `take_comments_for_prompt` hands every comment out once
//!   (sorted by relative path, then line) and keeps it aside, still following
//!   the buffer, until the next take; `restore_comments` brings it back when
//!   the message did not go out. [`format_feedback`](crate::format_feedback)
//!   writes the agent's section.
//! - **Persistence.** `state.json` v2 (`persist.rs`), with the text of the
//!   commented rows stored as an object to relocate the comment when the file
//!   changed outside Cincel.
//!
//! Binary files take no comments.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use cincel_text::{Anchor, AnchorId, AnchorMap, BufferEvent, BufferSnapshot, Rope};
use serde::{Deserialize, Serialize};

use crate::file::FileReview;
use crate::store::{ReviewStore, detached};
use crate::text::{normalize, rope_is_binary, rope_sha256, sha256_hex};
use crate::types::{FileStatus, Hunk, HunkId, LinePair};

/// Longest comment text, in characters (the editor's box enforces it too).
pub const COMMENT_MAX_CHARS: usize = 8000;
/// Most lines of code sent with one comment.
pub const COMMENT_SNIPPET_MAX_LINES: usize = 120;
/// Most bytes of code sent with one comment.
pub const COMMENT_SNIPPET_MAX_BYTES: usize = 12 * 1024;

/// Identifier of a comment, unique inside one [`ReviewStore`] (and kept
/// across save/load).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CommentId(pub u64);

/// How the agent's change under a comment ended up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentState {
    /// A pending hunk meets the commented rows.
    Pending,
    /// Only accepts touched the rows.
    Accepted,
    /// Only rejects touched the rows.
    Rejected,
    /// Accepts and rejects touched the rows (line by line).
    Mixed,
    /// The agent did not change these rows (a plain selection).
    NoAgentChange,
}

impl CommentState {
    /// The `Review state:` text of the agent's block (spec 09 §6.8).
    pub fn agent_text(self) -> &'static str {
        match self {
            CommentState::Pending => "your change, not reviewed yet",
            CommentState::Accepted => "your change, accepted by the user",
            CommentState::Rejected => "your change, rejected by the user",
            CommentState::Mixed => {
                "your change, partly accepted and partly rejected by the user, line by line"
            }
            CommentState::NoAgentChange => "no change of yours (the user selected these lines)",
        }
    }
}

/// What the lines of a [`SentComment`] are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SentRange {
    /// Rows of the file as it is now.
    Lines,
    /// No rows: the comment sits where a pure deletion removed lines, before
    /// `first_line`.
    RemovedBefore,
    /// The agent deleted the file; the lines are those of the deleted text.
    DeletedFile,
}

/// What the UI shows of an unsent comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentView {
    /// Id, for edit/remove.
    pub id: CommentId,
    /// Path, as the host keys it.
    pub path: PathBuf,
    /// Path relative to the workspace root, `/` separated.
    pub display_path: String,
    /// Buffer rows, end exclusive; empty = before `rows.start` (a comment on
    /// a pure deletion).
    pub rows: Range<u32>,
    /// The user's text.
    pub text: String,
    /// Created from that hunk's "Comentar" button (followed across hunk
    /// splits and merges while the hunk is pending).
    pub from_hunk: Option<HunkId>,
}

/// One comment as it left with a prompt (also what the chat card shows).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentComment {
    /// Id the comment had in the store ([`ReviewStore::restore_comments`]
    /// gives it back under it).
    pub id: CommentId,
    /// Path, as the host keys it.
    pub path: PathBuf,
    /// Path relative to the workspace root, `/` separated.
    pub display_path: String,
    /// First line, 1-based.
    pub first_line: u32,
    /// Last line, 1-based, inclusive (equal to `first_line` for one line and
    /// for [`SentRange::RemovedBefore`]).
    pub last_line: u32,
    /// What the lines are.
    pub kind: SentRange,
    /// State of the agent's change under the comment.
    pub state: CommentState,
    /// Current text of those lines (`None` for a pure deletion and a deleted
    /// file).
    pub code: Option<String>,
    /// Base lines of a pending pure deletion, or of a pending deleted file.
    pub removed: Option<String>,
    /// Lines of `code` (or `removed`) left out by the size limits.
    pub truncated_lines: u32,
    /// The buffer differs from the disk (the code is what the editor shows).
    pub unsaved: bool,
    /// Fence language: the lowercase extension, if it looks like one.
    pub lang: Option<String>,
    /// The user's text.
    pub text: String,
}

/// Why a persisted comment was not restored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentDropReason {
    /// The file no longer exists ("…el archivo ya no existe").
    Missing,
    /// The file is not text any more ("…el archivo ya no es de texto").
    NotText,
}

/// Which bound of a comment an anchor is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    Start,
    End,
}

#[derive(Clone, Debug)]
pub(crate) struct Comment {
    pub(crate) id: CommentId,
    pub(crate) path: PathBuf,
    pub(crate) text: String,
    pub(crate) created_at: u64,
    /// The hunk whose button created it.
    pub(crate) from_hunk: Option<HunkId>,
    /// Created from a hunk's button (survives restarts, the id does not).
    pub(crate) from_button: bool,
    /// Created with no rows (a pure deletion).
    pub(crate) empty: bool,
    pub(crate) accepted: bool,
    pub(crate) rejected: bool,
    /// The agent's deletion of the file was accepted.
    pub(crate) deleted_file: bool,
    start: AnchorId,
    end: AnchorId,
}

/// Anchors and the latest text of one commented path.
#[derive(Clone, Debug, Default)]
pub(crate) struct CommentFile {
    anchors: AnchorMap<(CommentId, End)>,
    snapshot: BufferSnapshot,
}

/// The comments of a [`ReviewStore`].
#[derive(Clone, Debug, Default)]
pub(crate) struct CommentStore {
    files: BTreeMap<PathBuf, CommentFile>,
    /// Unsent comments.
    live: BTreeMap<CommentId, Comment>,
    /// Handed out by the last take; still anchored until the next one.
    in_flight: BTreeMap<CommentId, Comment>,
    next_id: u64,
}

/// A closed interval of rows (`(first, last)`); a pure deletion or a comment
/// with no rows is the single row where it sits.
pub(crate) type Span = (u32, u32);

fn span(rows: &Range<u32>) -> Span {
    if rows.is_empty() {
        (rows.start, rows.start)
    } else {
        (rows.start, rows.end - 1)
    }
}

fn meets(a: Span, b: Span) -> bool {
    a.0 <= b.1 && b.0 <= a.1
}

/// Byte offset where `row` starts (the end of the text past the last row).
fn row_offset(snapshot: &BufferSnapshot, row: u32) -> usize {
    if row >= snapshot.line_count() {
        snapshot.len()
    } else {
        snapshot.line_start_offset(row)
    }
}

/// Byte offset of the end of `row`'s text (before its newline).
fn row_end_offset(snapshot: &BufferSnapshot, row: u32) -> usize {
    (snapshot.line_start_offset(row) + snapshot.line_len(row) as usize).min(snapshot.len())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Caps a comment text at [`COMMENT_MAX_CHARS`].
fn clamp_text(text: String) -> String {
    match text.char_indices().nth(COMMENT_MAX_CHARS) {
        Some((cut, _)) => text[..cut].to_owned(),
        None => text,
    }
}

/// The fence language of `path`: its lowercase extension when it is 1 to 10
/// characters of `[a-z0-9+#-]`.
pub(crate) fn lang_of(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    let ok = (1..=10).contains(&ext.chars().count())
        && ext
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+#-".contains(c));
    ok.then_some(ext)
}

/// Lines (without newlines) capped by [`COMMENT_SNIPPET_MAX_LINES`] and
/// [`COMMENT_SNIPPET_MAX_BYTES`]: the text joined with `\n` and how many
/// lines were left out.
pub(crate) fn cap_lines(lines: &[String]) -> (String, u32) {
    let mut out = String::new();
    let mut shown = 0usize;
    for line in lines {
        if shown == COMMENT_SNIPPET_MAX_LINES {
            break;
        }
        let extra = line.len() + usize::from(shown > 0);
        if out.len() + extra > COMMENT_SNIPPET_MAX_BYTES {
            if shown == 0 {
                // A single huge line: cut it on a char boundary.
                let mut cut = COMMENT_SNIPPET_MAX_BYTES;
                while !line.is_char_boundary(cut) {
                    cut -= 1;
                }
                out.push_str(&line[..cut]);
                shown = 1;
            }
            break;
        }
        if shown > 0 {
            out.push('\n');
        }
        out.push_str(line);
        shown += 1;
    }
    (out, (lines.len() - shown) as u32)
}

fn rows_text(snapshot: &BufferSnapshot, rows: &Range<u32>) -> Vec<String> {
    rows.clone().map(|row| snapshot.line_text(row)).collect()
}

fn text_lines(text: &str) -> Vec<String> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.split('\n').map(str::to_owned).collect()
}

impl CommentStore {
    fn comment(&self, id: CommentId) -> Option<&Comment> {
        self.live.get(&id).or_else(|| self.in_flight.get(&id))
    }

    fn comment_mut(&mut self, id: CommentId) -> Option<&mut Comment> {
        match self.live.get_mut(&id) {
            Some(comment) => Some(comment),
            None => self.in_flight.get_mut(&id),
        }
    }

    fn all(&self) -> impl Iterator<Item = &Comment> {
        self.live.values().chain(self.in_flight.values())
    }

    fn fresh_id(&mut self) -> CommentId {
        self.next_id += 1;
        CommentId(self.next_id)
    }

    /// Rows of `comment` in `snapshot` (the anchors' text, or one the host
    /// gives for the same buffer).
    fn rows_in(&self, comment: &Comment, snapshot: &BufferSnapshot) -> Range<u32> {
        let Some(file) = self.files.get(&comment.path) else {
            return 0..0;
        };
        let len = snapshot.len();
        let start = file.anchors.resolve(comment.start).unwrap_or(0).min(len);
        let end = file
            .anchors
            .resolve(comment.end)
            .unwrap_or(start)
            .clamp(start, len);
        let first = snapshot.offset_to_point(start).row;
        if comment.empty {
            if start == end {
                return first..first;
            }
            let point = snapshot.offset_to_point(end);
            let last = if point.column == 0 {
                point.row
            } else {
                point.row + 1
            };
            return first..last.max(first + 1);
        }
        let last = snapshot.offset_to_point(end).row + 1;
        first..last.max(first + 1)
    }

    /// Rows of `comment` in the latest text the store saw.
    fn rows(&self, comment: &Comment) -> Range<u32> {
        match self.files.get(&comment.path) {
            Some(file) => self.rows_in(comment, &file.snapshot),
            None => 0..0,
        }
    }

    fn snapshot(&self, path: &Path) -> Option<&BufferSnapshot> {
        self.files.get(path).map(|f| &f.snapshot)
    }

    /// Anchors a new comment on `rows` of `snapshot`.
    #[allow(clippy::too_many_arguments)]
    fn insert(
        &mut self,
        id: CommentId,
        path: &Path,
        snapshot: &BufferSnapshot,
        rows: Range<u32>,
        from_hunk: Option<HunkId>,
        from_button: bool,
        text: String,
        created_at: u64,
    ) -> CommentId {
        let file = self.files.entry(path.to_owned()).or_default();
        file.snapshot = snapshot.clone();
        let last_row = snapshot.line_count().saturating_sub(1);
        let empty = rows.is_empty();
        let first = rows.start.min(if empty {
            snapshot.line_count()
        } else {
            last_row
        });
        let (start, end) = if empty {
            let offset = row_offset(snapshot, first);
            (Anchor::before(offset), Anchor::after(offset))
        } else {
            let last = (rows.end - 1).clamp(first, last_row);
            (
                Anchor::before(row_offset(snapshot, first)),
                Anchor::after(row_end_offset(snapshot, last)),
            )
        };
        let start = file.anchors.insert(start, (id, End::Start));
        let end = file.anchors.insert(end, (id, End::End));
        self.next_id = self.next_id.max(id.0);
        self.live.insert(
            id,
            Comment {
                id,
                path: path.to_owned(),
                text,
                created_at,
                from_hunk,
                from_button: from_button || from_hunk.is_some(),
                empty,
                accepted: false,
                rejected: false,
                deleted_file: false,
                start,
                end,
            },
        );
        id
    }

    /// Drops the anchors of `comment` (and the path once nothing is left).
    fn unanchor(&mut self, comment: &Comment) {
        let Some(file) = self.files.get_mut(&comment.path) else {
            return;
        };
        file.anchors.remove(comment.start);
        file.anchors.remove(comment.end);
        if file.anchors.is_empty() {
            self.files.remove(&comment.path);
        }
    }

    fn drop_in_flight(&mut self) {
        for comment in std::mem::take(&mut self.in_flight).into_values() {
            self.unanchor(&comment);
        }
    }
}

impl ReviewStore {
    // ------------------------------------------------------------ editing

    /// Adds a comment on `rows` (buffer rows, end exclusive; empty = before
    /// `rows.start`, for a pure deletion) of `path`, whose buffer is
    /// `snapshot`. `from_hunk`: created from that hunk's "Comentar" button.
    /// The text is capped at [`COMMENT_MAX_CHARS`].
    ///
    /// Returns `None` (nothing added) for blank text and for a binary file.
    pub fn add_comment(
        &mut self,
        path: &Path,
        snapshot: &BufferSnapshot,
        rows: Range<u32>,
        from_hunk: Option<HunkId>,
        text: String,
    ) -> Option<CommentId> {
        if text.trim().is_empty() || rope_is_binary(snapshot.rope()) {
            return None;
        }
        let id = self.comments.fresh_id();
        Some(self.comments.insert(
            id,
            path,
            snapshot,
            rows,
            from_hunk,
            false,
            clamp_text(text),
            now_secs(),
        ))
    }

    /// Replaces the text of a comment. Blank text removes it (an emptied
    /// edit is a delete). `false` if there is no such unsent comment.
    pub fn edit_comment(&mut self, id: CommentId, text: String) -> bool {
        if text.trim().is_empty() {
            return self.remove_comment(id);
        }
        match self.comments.live.get_mut(&id) {
            Some(comment) => {
                comment.text = clamp_text(text);
                true
            }
            None => false,
        }
    }

    /// Removes an unsent comment. `false` if there is none with this id.
    pub fn remove_comment(&mut self, id: CommentId) -> bool {
        match self.comments.live.remove(&id) {
            Some(comment) => {
                self.comments.unanchor(&comment);
                true
            }
            None => false,
        }
    }

    /// Drops every comment of `path` (the file became binary, or is gone).
    /// Returns how many unsent comments were dropped.
    pub fn drop_comments_in(&mut self, path: &Path) -> usize {
        let live: Vec<CommentId> = self
            .comments
            .live
            .values()
            .filter(|c| c.path == path)
            .map(|c| c.id)
            .collect();
        let flying: Vec<CommentId> = self
            .comments
            .in_flight
            .values()
            .filter(|c| c.path == path)
            .map(|c| c.id)
            .collect();
        for id in &flying {
            if let Some(comment) = self.comments.in_flight.remove(id) {
                self.comments.unanchor(&comment);
            }
        }
        for id in &live {
            self.remove_comment(*id);
        }
        live.len()
    }

    /// Feeds one [`BufferEvent`] of the buffer of `path` to the comment
    /// anchors (every source: user, agent reload, reject, undo). `snapshot`
    /// is the buffer after the event (or later, when the host drains a
    /// batch: the anchors only need the text once every event is in).
    /// A no-op for a path without comments.
    pub fn comment_buffer_event(
        &mut self,
        path: &Path,
        event: &BufferEvent,
        snapshot: &BufferSnapshot,
    ) {
        let Some(file) = self.comments.files.get_mut(path) else {
            return;
        };
        file.anchors.apply_event(event);
        file.snapshot = snapshot.clone();
        let BufferEvent::Edited { version, .. } = event;
        let exact = snapshot.version() == *version;
        let store = &mut self.comments;
        let ids: Vec<CommentId> = store
            .all()
            .filter(|c| c.path == path)
            .map(|c| c.id)
            .collect();
        for id in ids {
            let file = store.files.get_mut(path).expect("anchored");
            let comment = match store.live.get_mut(&id) {
                Some(comment) => comment,
                None => store.in_flight.get_mut(&id).expect("listed"),
            };
            comment.deleted_file = false;
            if !(comment.empty && exact) {
                continue;
            }
            // Lines landed where a comment without rows sits (a rejected
            // deletion came back): from now on it covers them.
            let start = file.anchors.resolve(comment.start).unwrap_or(0);
            let end = file.anchors.resolve(comment.end).unwrap_or(start);
            if end <= start {
                continue;
            }
            let point = snapshot.offset_to_point(end);
            let last = if point.column == 0 {
                point
                    .row
                    .saturating_sub(1)
                    .max(snapshot.offset_to_point(start).row)
            } else {
                point.row
            };
            file.anchors.remove(comment.end);
            comment.end = file.anchors.insert(
                Anchor::after(row_end_offset(snapshot, last)),
                (id, End::End),
            );
            comment.empty = false;
        }
    }

    // ------------------------------------------------------------ queries

    /// Every unsent comment, by display path then row. `snapshot_of` gives
    /// the host's buffer of a path when it has one (otherwise the latest
    /// text the store saw is used).
    pub fn comments(
        &self,
        snapshot_of: impl Fn(&Path) -> Option<BufferSnapshot>,
    ) -> Vec<CommentView> {
        let mut out: Vec<CommentView> = self
            .comments
            .live
            .values()
            .map(|comment| {
                let rows = match snapshot_of(&comment.path) {
                    Some(snapshot) if !self.is_deleted(&comment.path) => {
                        self.comments.rows_in(comment, &snapshot)
                    }
                    _ => self.comments.rows(comment),
                };
                CommentView {
                    id: comment.id,
                    path: comment.path.clone(),
                    display_path: self.display_path(&comment.path),
                    from_hunk: self.followed_hunk(comment, &rows),
                    rows,
                    text: comment.text.clone(),
                }
            })
            .collect();
        out.sort_by(|a, b| {
            (a.display_path.as_bytes(), a.rows.start, a.id).cmp(&(
                b.display_path.as_bytes(),
                b.rows.start,
                b.id,
            ))
        });
        out
    }

    /// Number of unsent comments.
    pub fn comment_count(&self) -> usize {
        self.comments.live.len()
    }

    /// Number of unsent comments on `path`.
    pub fn comment_count_in(&self, path: &Path) -> usize {
        self.comments
            .live
            .values()
            .filter(|c| c.path == path)
            .count()
    }

    /// Paths with unsent comments (the host keeps their buffers watched).
    pub fn commented_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self
            .comments
            .live
            .values()
            .map(|c| c.path.clone())
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    /// The state a comment would be sent with right now (`None`: no such
    /// unsent comment).
    pub fn comment_state(&self, id: CommentId) -> Option<CommentState> {
        let comment = self.comments.live.get(&id)?;
        let rows = self.comments.rows(comment);
        Some(self.state_of(comment, &rows))
    }

    fn is_deleted(&self, path: &Path) -> bool {
        self.files
            .get(path)
            .is_some_and(|f| matches!(f.status, FileStatus::Deleted { .. }))
    }

    /// The pending hunk a comment made from a hunk's button belongs to now:
    /// the original one while it is pending, else the pending hunk under its
    /// rows (the hunk was split or merged), else the original id.
    fn followed_hunk(&self, comment: &Comment, rows: &Range<u32>) -> Option<HunkId> {
        if !comment.from_button {
            return None;
        }
        let Some(file) = self.files.get(&comment.path) else {
            return comment.from_hunk;
        };
        if let Some(id) = comment.from_hunk
            && file.pending_hunks().any(|h| h.id == id)
        {
            return Some(id);
        }
        let target = span(rows);
        file.pending_hunks()
            .find(|h| meets(hunk_span(file, h), target))
            .map(|h| h.id)
            .or(comment.from_hunk)
    }

    /// Whether a pending change of the review meets `rows` of `path`.
    fn pending_under(&self, path: &Path, rows: &Range<u32>) -> bool {
        let Some(file) = self.files.get(path) else {
            return false;
        };
        if !file.has_pending_work() {
            return false;
        }
        if file.file_level_only() {
            return true;
        }
        let target = span(rows);
        file.pending_hunks()
            .any(|hunk| meets(hunk_span(file, hunk), target))
    }

    fn state_of(&self, comment: &Comment, rows: &Range<u32>) -> CommentState {
        if self.pending_under(&comment.path, rows) {
            return CommentState::Pending;
        }
        match (comment.accepted, comment.rejected) {
            (true, true) => CommentState::Mixed,
            (true, false) => CommentState::Accepted,
            (false, true) => CommentState::Rejected,
            (false, false) => CommentState::NoAgentChange,
        }
    }

    // ------------------------------------------------------------ marks

    /// Rows a hunk decision covers.
    pub(crate) fn hunk_decision_span(&self, path: &Path, id: HunkId) -> Option<Span> {
        let file = self.files.get(path)?;
        let hunk = file.hunks.iter().find(|h| h.id == id)?;
        Some(hunk_span(file, hunk))
    }

    /// Rows a line decision covers: the buffer row of the line, or for a
    /// base-only line the nearest buffer row of its hunk.
    pub(crate) fn line_decision_span(&self, path: &Path, pair: &LinePair) -> Option<Span> {
        let file = self.files.get(path)?;
        let hunk = file
            .pending_hunks()
            .find(|h| h.lines.iter().any(|p| p == pair))?;
        let row = |anchor: Anchor| file.current.offset_to_point(anchor.offset).row;
        if let Some(anchor) = pair.buffer_row {
            let row = row(anchor);
            return Some((row, row));
        }
        let at = hunk.lines.iter().position(|p| p == pair)?;
        let next = hunk.lines[at..].iter().find_map(|p| p.buffer_row);
        let prev = hunk.lines[..at].iter().rev().find_map(|p| p.buffer_row);
        match next.or(prev) {
            Some(anchor) => {
                let row = row(anchor);
                Some((row, row))
            }
            None => Some(hunk_span(file, hunk)),
        }
    }

    /// Rows a whole-file decision covers (`None`: the whole file).
    pub(crate) fn file_decision_spans(&self, path: &Path) -> Option<Vec<Span>> {
        let file = self.files.get(path)?;
        if file.file_level_only() {
            return None;
        }
        Some(
            file.pending_hunks()
                .map(|hunk| hunk_span(file, hunk))
                .collect(),
        )
    }

    /// Marks the comments of `path` meeting `spans` (`None`: all of them) as
    /// accepted or rejected. Returns the comments whose mark is new (what an
    /// undo of a reject must take back).
    pub(crate) fn mark_comments(
        &mut self,
        path: &Path,
        spans: Option<&[Span]>,
        accepted: bool,
    ) -> Vec<CommentId> {
        let hits: Vec<CommentId> = self
            .comments
            .all()
            .filter(|c| c.path == path)
            .filter(|c| match spans {
                None => true,
                Some(spans) => {
                    let target = span(&self.comments.rows(c));
                    spans.iter().any(|s| meets(*s, target))
                }
            })
            .map(|c| c.id)
            .collect();
        let mut fresh = Vec::new();
        for id in hits {
            let comment = self.comments.comment_mut(id).expect("listed");
            let flag = if accepted {
                &mut comment.accepted
            } else {
                &mut comment.rejected
            };
            if !*flag {
                *flag = true;
                fresh.push(id);
            }
        }
        fresh
    }

    /// The agent's deletion of `path` was accepted: its comments now speak
    /// of a deleted file.
    pub(crate) fn mark_deleted_file(&mut self, path: &Path) {
        let ids: Vec<CommentId> = self
            .comments
            .all()
            .filter(|c| c.path == path)
            .map(|c| c.id)
            .collect();
        for id in ids {
            if let Some(comment) = self.comments.comment_mut(id) {
                comment.deleted_file = true;
            }
        }
    }

    /// Takes back reject marks (undo of a reject).
    pub(crate) fn unmark_rejected(&mut self, ids: &[CommentId]) {
        for id in ids {
            if let Some(comment) = self.comments.comment_mut(*id) {
                comment.rejected = false;
            }
        }
    }

    // ------------------------------------------------------------ sending

    /// Takes every unsent comment out for the prompt that is going out,
    /// sorted by relative path (bytes) then first line. `snapshot_of` gives
    /// the host's buffer of a path and whether it differs from the disk.
    ///
    /// The comments leave the store's counts, views and saved state; they
    /// stay anchored (invisible) until the next take, so
    /// [`Self::restore_comments`] can put them back exactly where they are.
    pub fn take_comments_for_prompt(
        &mut self,
        snapshot_of: impl Fn(&Path) -> Option<(BufferSnapshot, bool)>,
    ) -> Vec<SentComment> {
        self.comments.drop_in_flight();
        let live = std::mem::take(&mut self.comments.live);
        let mut sent: Vec<SentComment> = live
            .values()
            .map(|comment| {
                let host = snapshot_of(&comment.path);
                self.sent_comment(comment, host)
            })
            .collect();
        self.comments.in_flight = live;
        sent.sort_by(|a, b| {
            (a.display_path.as_bytes(), a.first_line, a.id).cmp(&(
                b.display_path.as_bytes(),
                b.first_line,
                b.id,
            ))
        });
        sent
    }

    fn sent_comment(&self, comment: &Comment, host: Option<(BufferSnapshot, bool)>) -> SentComment {
        let review_deleted = self.is_deleted(&comment.path);
        let deleted = review_deleted || comment.deleted_file;
        let mut stored = self
            .comments
            .snapshot(&comment.path)
            .cloned()
            .unwrap_or_default();
        if let Some(FileReview {
            status: FileStatus::Deleted { previous },
            ..
        }) = self.files.get(&comment.path)
            && stored.rope() != previous
        {
            // The buffer the comment was anchored in is gone: the deleted
            // tab shows the deleted text.
            stored = detached(&previous.to_string());
        }
        let (snapshot, dirty) = match host {
            Some((snapshot, dirty)) if !deleted => (snapshot, dirty),
            _ => (stored, false),
        };
        let rows = self.comments.rows_in(comment, &snapshot);
        let state = self.state_of(comment, &rows);
        let kind = if deleted {
            SentRange::DeletedFile
        } else if comment.empty && rows.is_empty() {
            SentRange::RemovedBefore
        } else {
            SentRange::Lines
        };
        let first_line = rows.start + 1;
        let last_line = if rows.is_empty() {
            first_line
        } else {
            rows.end
        };
        let pending = state == CommentState::Pending;
        let (code, removed, truncated_lines) = match kind {
            SentRange::Lines => {
                let (code, cut) = cap_lines(&rows_text(&snapshot, &rows));
                (Some(code), None, cut)
            }
            SentRange::DeletedFile if pending && review_deleted => {
                let (removed, cut) = cap_lines(&rows_text(&snapshot, &rows));
                (None, Some(removed), cut)
            }
            SentRange::RemovedBefore if pending => {
                match self.removed_at(&comment.path, rows.start) {
                    Some(text) => {
                        let (removed, cut) = cap_lines(&text_lines(&text));
                        (None, Some(removed), cut)
                    }
                    None => (None, None, 0),
                }
            }
            _ => (None, None, 0),
        };
        SentComment {
            id: comment.id,
            path: comment.path.clone(),
            display_path: self.display_path(&comment.path),
            first_line,
            last_line,
            kind,
            state,
            code,
            removed,
            truncated_lines,
            unsaved: dirty && kind == SentRange::Lines,
            lang: lang_of(&comment.path),
            text: comment.text.clone(),
        }
    }

    /// Base text of the pending pure deletion drawn before `row` of `path`.
    fn removed_at(&self, path: &Path, row: u32) -> Option<String> {
        let file = self.files.get(path)?;
        file.pending_hunks()
            .find(|hunk| {
                let rows = file.buffer_rows(hunk);
                rows.is_empty() && rows.start == row && !hunk.base_byte_range.is_empty()
            })
            .map(|hunk| file.base_text(hunk))
    }

    /// Puts back comments that were taken for a prompt that did not go out
    /// (spec 09 D16). Comments still kept aside by the last take come back
    /// exactly as they were (anchors and marks); others are rebuilt on their
    /// lines from `snapshot_of` (or the latest text the store saw), with the
    /// marks their state implies.
    pub fn restore_comments(
        &mut self,
        sent: &[SentComment],
        snapshot_of: impl Fn(&Path) -> Option<BufferSnapshot>,
    ) {
        for item in sent {
            if self.comments.live.contains_key(&item.id) {
                continue;
            }
            if let Some(comment) = self.comments.in_flight.remove(&item.id) {
                self.comments.live.insert(item.id, comment);
                continue;
            }
            let snapshot = snapshot_of(&item.path)
                .or_else(|| self.comments.snapshot(&item.path).cloned())
                .or_else(|| self.files.get(&item.path).map(|f| f.current.clone()))
                .unwrap_or_default();
            let start = item.first_line.saturating_sub(1);
            let rows = match item.kind {
                SentRange::RemovedBefore => start..start,
                _ => start..item.last_line.max(start + 1),
            };
            let id = if self.comments.comment(item.id).is_some() {
                self.comments.fresh_id()
            } else {
                item.id
            };
            let id = self.comments.insert(
                id,
                &item.path,
                &snapshot,
                rows,
                None,
                false,
                item.text.clone(),
                now_secs(),
            );
            let deleted_file = item.kind == SentRange::DeletedFile && !self.is_deleted(&item.path);
            let comment = self.comments.live.get_mut(&id).expect("inserted");
            comment.deleted_file = deleted_file;
            (comment.accepted, comment.rejected) = match item.state {
                CommentState::Accepted => (true, false),
                CommentState::Rejected => (false, true),
                CommentState::Mixed => (true, true),
                CommentState::Pending | CommentState::NoAgentChange => (false, false),
            };
        }
    }

    // ------------------------------------------------------------ persistence

    /// The unsent comments as `state.json` entries; their snippets are
    /// written with `write_object` (hashes are returned for the cleanup).
    pub(crate) fn comment_entries(
        &self,
        mut write_object: impl FnMut(&Rope) -> Result<String, crate::ReviewError>,
    ) -> Result<(Vec<CommentEntry>, Vec<String>), crate::ReviewError> {
        let mut entries = Vec::new();
        let mut hashes = Vec::new();
        for comment in self.comments.live.values() {
            let Some(snapshot) = self.comments.snapshot(&comment.path) else {
                continue;
            };
            let rows = self.comments.rows(comment);
            let snippet =
                snapshot.text_in(row_offset(snapshot, rows.start)..row_offset(snapshot, rows.end));
            let snippet_hash = write_object(&Rope::from_str(&snippet))?;
            hashes.push(snippet_hash.clone());
            entries.push(CommentEntry {
                id: comment.id.0,
                path: comment.path.clone(),
                start_row: rows.start,
                end_row: rows.end,
                text: comment.text.clone(),
                created_at: comment.created_at,
                from_hunk: comment.from_button,
                file_hash: rope_sha256(snapshot.rope()),
                snippet_hash,
                accepted: comment.accepted,
                rejected: comment.rejected,
            });
        }
        Ok((entries, hashes))
    }

    /// Restores saved comments (after the files of the review, so a deleted
    /// file in review keeps its comments on its deleted text).
    pub(crate) fn restore_comment_entries(
        &mut self,
        entries: Vec<CommentEntry>,
        read_object: impl Fn(&str) -> Option<String>,
        read_current: &impl Fn(&Path) -> Option<Vec<u8>>,
        report: &mut crate::LoadReport,
    ) {
        for entry in entries {
            let id = CommentId(entry.id);
            if self.comments.comment(id).is_some() {
                continue;
            }
            let text = match self.files.get(&entry.path).map(|f| &f.status) {
                Some(FileStatus::Deleted { previous }) => previous.to_string(),
                _ => {
                    let Some(bytes) = read_current(&entry.path) else {
                        report
                            .comments_dropped
                            .push((entry.path, CommentDropReason::Missing));
                        continue;
                    };
                    match std::str::from_utf8(&bytes) {
                        Ok(text) => normalize(text),
                        Err(_) => {
                            report
                                .comments_dropped
                                .push((entry.path, CommentDropReason::NotText));
                            continue;
                        }
                    }
                }
            };
            let snapshot = match self.comments.snapshot(&entry.path) {
                Some(existing) if existing.rope() == &Rope::from_str(&text) => existing.clone(),
                _ => detached(&text),
            };
            if rope_is_binary(snapshot.rope()) {
                report
                    .comments_dropped
                    .push((entry.path, CommentDropReason::NotText));
                continue;
            }
            let count = snapshot.line_count();
            let empty = entry.end_row <= entry.start_row;
            let rows = if sha256_hex(text.as_bytes()) == entry.file_hash {
                let start = entry.start_row.min(count.saturating_sub(1));
                if empty {
                    start..start
                } else {
                    start..entry.end_row.clamp(start + 1, count.max(start + 1))
                }
            } else {
                report.comments_relocated += 1;
                let snippet = (!empty)
                    .then(|| read_object(&entry.snippet_hash))
                    .flatten()
                    .filter(|s| !s.is_empty());
                let found = snippet.and_then(|snippet| {
                    nearest_occurrence(&text, &snippet, &snapshot, entry.start_row)
                        .map(|row| row..row + (entry.end_row - entry.start_row))
                });
                found.unwrap_or_else(|| {
                    let start = entry.start_row.min(count.saturating_sub(1));
                    if empty {
                        start..start
                    } else {
                        start..start + 1
                    }
                })
            };
            let id = self.comments.insert(
                id,
                &entry.path,
                &snapshot,
                rows.clone(),
                None,
                entry.from_hunk,
                clamp_text(entry.text),
                entry.created_at,
            );
            let followed = {
                let comment = &self.comments.live[&id];
                let rows = self.comments.rows(comment);
                self.followed_hunk(comment, &rows)
            };
            let comment = self.comments.live.get_mut(&id).expect("inserted");
            comment.accepted = entry.accepted;
            comment.rejected = entry.rejected;
            comment.from_hunk = followed;
            report.comments_restored += 1;
        }
    }
}

/// Rows of a hunk in the buffer as a closed span (a pure deletion: the row
/// its red rows are drawn before).
fn hunk_span(file: &FileReview, hunk: &Hunk) -> Span {
    span(&file.buffer_rows(hunk))
}

/// The row where `snippet` (whole rows) appears in `text` closest to `row`.
fn nearest_occurrence(
    text: &str,
    snippet: &str,
    snapshot: &BufferSnapshot,
    row: u32,
) -> Option<u32> {
    let whole_end = |end: usize| end == text.len() || snippet.ends_with('\n');
    text.match_indices(snippet)
        .filter(|(at, _)| *at == 0 || text.as_bytes()[at - 1] == b'\n')
        .filter(|(at, s)| whole_end(at + s.len()) || text.as_bytes()[at + s.len()] == b'\n')
        .map(|(at, _)| snapshot.offset_to_point(at).row)
        .min_by_key(|found| (found.abs_diff(row), *found))
}

/// One comment in `state.json` (v2).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CommentEntry {
    pub id: u64,
    pub path: PathBuf,
    pub start_row: u32,
    pub end_row: u32,
    pub text: String,
    pub created_at: u64,
    pub from_hunk: bool,
    pub file_hash: String,
    pub snippet_hash: String,
    pub accepted: bool,
    pub rejected: bool,
}
