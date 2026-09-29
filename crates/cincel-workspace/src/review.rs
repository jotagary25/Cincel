//! The review of agent edits, wired into the workspace
//! (`docs/specs/modulos/workspace.md`, "Ciclo de revisión";
//! `docs/specs/03-arquitectura.md` §4).
//!
//! [`Review`] owns the [`ReviewStore`] of the open project and is the only
//! place that talks to it. It follows the host protocol documented in
//! `cincel-review`:
//!
//! 0. **The photo** (v2, 2026-09-28: the base comes from the photo of the
//!    project; the tools are only a hint). `begin_prompt` takes a
//!    [`cincel_project::ProjectSnapshot`] of every file before the prompt
//!    goes out (`review_snapshot.rs`). Whatever changes on disk inside the
//!    project until the turn ends is the agent's, whatever it used: the
//!    watcher's [`FilesChanged`] batches, for any path, open or not, and the
//!    end-of-turn sweep of the whole photo against the disk. A path not in
//!    review takes the photo's copy as its base (`file_created` if it was not
//!    there). What Cincel itself writes meanwhile ([`HostWrote`]: a
//!    `Ctrl+S`, a new file, a reject) refreshes the photo and is never the
//!    agent's; other programs' changes are.
//! 1. **Base.** Otherwise captured on the first contact of a turn with a
//!    path (the hints): an `fs/read_text_file` (from the open buffer, before
//!    answering), an `fs/write_text_file` (before applying it), the first
//!    `tool_call` of kind `edit`/`delete`/`move` naming the path, or
//!    `file_created` for a path that did not exist. When the disk already
//!    differs from the photo, the photo is the base.
//! 2. **Truth from disk.** After every completed edit tool call, every
//!    `agentFileChangeReport`, every watcher batch and the final sweep, the
//!    file is re-read from disk into its buffer (minimal edits) and reported
//!    with `file_written` / `file_deleted` / `file_created`. The `diff` the
//!    agent sends is never used. Binary files are out of the review: what
//!    the agent does to them is applied without asking (`review_snapshot.rs`).
//! 3. **Buffer events.** Every buffer in review is subscribed to; its events
//!    are queued by the subscription and drained on the main thread, right
//!    after each operation of this module (so the snapshot is the one right
//!    after the event) and from a wake-up task for edits made elsewhere (the
//!    user typing).
//! 4. **Recompute.** Debounced 50 ms, the job built on the main thread, run on
//!    the background executor, applied back on the main thread.
//! 5. **Decisions.** From the editor ([`cincel_editor::ReviewAction`]), the
//!    global `workspace::*` commands and the review panel. Rejects become one
//!    `EditSource::Review` transaction plus a save (or a file operation).
//! 6. **Turns.** `begin_turn` when a prompt goes out (with the report of the
//!    previous turn as its feedback), `end_turn` on `TurnEnded`.
//! 7. **Persistence.** `~/.local/share/cincel/review/<hash>/`, saved one
//!    second after the last decision and on quit, loaded when the project
//!    opens.
//! 8. **Undo.** `Alt+Shift+U` takes back the last reject (the store's own
//!    stack).
//!
//! The rest of the workspace reads a [`ReviewSummary`] shared through an
//! `Rc<RefCell<…>>` (the tree, the tabs, the status bar and the panel paint
//! from it) and never touches the entity while it paints.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use cincel_editor::{
    EditorView, ReviewAction, ReviewHunkKind, ReviewHunkView, ReviewLineView, ReviewView,
    ReviewWordDiffs,
};
use cincel_project::{BufferChange, BufferHandle, ReloadOutcome};
use cincel_review::{
    DropReason, FileOp, FileReview, FileStatus, HunkId, HunkKind, LinePair, RecomputeResult,
    Revert, ReviewLocation, ReviewStore, Tracked, TurnId,
};
use cincel_text::{BufferEvent, BufferSnapshot, EditSource, Point, Rope, SubscriptionId};
use gpui::{
    AnyWindowHandle, App, Context, Entity, FocusHandle, SharedString, Subscription, Task,
    WeakEntity, Window,
};
use parking_lot::Mutex;

use crate::center::{CenterEvent, CenterPanel};
use crate::project::{FilesChanged, HostWrote, Project, ProjectEvent};

/// The photo of the project and the end-of-turn sweep
/// (`review_snapshot.rs`).
#[path = "review_snapshot.rs"]
mod photo;

pub use photo::{
    SWEEP_NOTICE, SWEEP_NOTICE_AFTER, SWEEP_SLICE, SweepStats, WATCH_SLICE, WatchStats,
};

/// Debounce of the background recompute (`03-arquitectura.md` §4.3).
pub const RECOMPUTE_DEBOUNCE: Duration = Duration::from_millis(50);
/// Debounce of the persistence after a decision.
pub const PERSIST_DEBOUNCE: Duration = Duration::from_secs(1);

/// Shared, read-only view of the review for the painting code.
pub type SharedSummary = Rc<RefCell<ReviewSummary>>;

/// How a file in review relates to what existed before the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileState {
    /// Changed by the agent.
    Modified,
    /// Created by the agent.
    Created,
    /// Deleted by the agent.
    Deleted,
}

/// What the tree, the tabs and the panel need about one file with pending
/// changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSummary {
    /// Pending added lines.
    pub added: u32,
    /// Pending removed lines.
    pub removed: u32,
    /// Modified, created or deleted.
    pub state: FileState,
    /// Only whole-file decisions (too large, without a copy or deleted).
    pub file_level: bool,
    /// Pending hunks (0 for file-level files).
    pub hunks: usize,
    /// A file the photo kept no copy of: it can only be accepted.
    pub no_copy: bool,
}

/// What the tree says next to a file the agent changed whose previous
/// content Cincel did not keep (over `review.max_file_size_kb` or
/// `review.snapshot_max_total_mb`).
pub const NO_COPY_LABEL: &str = "cambiado por el agente, sin copia previa";
/// The tooltip of a file the agent deleted, in the tree.
pub const DELETED_TOOLTIP: &str = "Borrado por el agente · pendiente";
/// Id of the single hunk the tab of a file the agent deleted shows: its whole
/// previous content as read-only phantom rows. The store numbers its hunks
/// from 1 upwards, so this one never names a real hunk.
pub const DELETED_FILE_HUNK: u64 = u64::MAX;
/// The only phantom row of a deleted file the photo kept no copy of.
pub const DELETED_NO_COPY_LINE: &str = "(borrado por el agente: Cincel no guardó una copia previa)";
/// Files a build before the 1.0 fixes saved for the binary files, next to
/// the store's state; removed when the project opens (binary files are out
/// of the review now).
const LEGACY_BINARY_STATE: [&str; 2] = ["binaries.json", "binaries"];

/// The tree's label for a file reviewed outside the store (without a copy),
/// instead of `+N −M`.
pub fn outside_label(file: &FileSummary) -> Option<&'static str> {
    file.no_copy.then_some(NO_COPY_LABEL)
}

/// Everything the painting code reads about the review.
#[derive(Clone, Debug, Default)]
pub struct ReviewSummary {
    /// Project root, to relativize paths.
    pub root: Option<PathBuf>,
    /// Files with pending decisions, by absolute path.
    pub files: BTreeMap<PathBuf, FileSummary>,
    /// Every path the store tracks (pending or not): their buffers stay open
    /// when their tab closes.
    pub tracked: HashSet<PathBuf>,
    /// Pending decisions (hunks, plus one per file-level file).
    pub pending: usize,
    /// `(added, removed)` over every pending file.
    pub total: (u32, u32),
    /// An agent turn is running.
    pub turn_active: bool,
    /// The end-of-turn sweep has been running for over
    /// [`SWEEP_NOTICE_AFTER`]: the status bar says [`SWEEP_NOTICE`].
    pub sweeping: bool,
    /// The review panel is open.
    pub panel_open: bool,
    /// The file the "cambios sin guardar" dialog asks about.
    pub dirty_prompt: Option<SharedString>,
}

impl ReviewSummary {
    /// The pending state of `path`.
    pub fn file(&self, path: &Path) -> Option<&FileSummary> {
        self.files.get(path)
    }

    /// Whether the store tracks `path`.
    pub fn is_tracked(&self, path: &Path) -> bool {
        self.tracked.contains(path)
    }

    /// Whether some file below the folder `dir` has pending changes.
    pub fn has_pending_under(&self, dir: &Path) -> bool {
        self.files
            .range(dir.to_path_buf()..)
            .take_while(|(path, _)| path.starts_with(dir))
            .any(|(path, _)| path != dir)
    }
}

/// `+N −M`, the way tabs, tree and panel print stats.
pub fn stats_label(added: u32, removed: u32) -> String {
    format!("+{added} −{removed}")
}

/// The tree's counter of a file in review: `+N` for a file the agent
/// created, `−N` for one it deleted (N: its lines), `+N −M` otherwise.
pub fn tree_stats_label(file: &FileSummary) -> String {
    match file.state {
        FileState::Created => format!("+{}", file.added),
        FileState::Deleted => format!("−{}", file.removed),
        FileState::Modified => stats_label(file.added, file.removed),
    }
}

/// `count` followed by the word for what is counted, singular or plural
/// ("1 archivo", "3 cambios").
pub fn count_label(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

/// The tree's root line: files with pending changes, pending changes (hunks,
/// plus one per file reviewed as a whole — what the floating bar's "cambio N
/// de M" counts inside one file) and pending lines, each with its word:
/// "1 archivo · 3 cambios · +0 −36".
pub fn totals_label(files: usize, changes: usize, (added, removed): (u32, u32)) -> String {
    format!(
        "{} · {} · {}",
        count_label(files, "archivo", "archivos"),
        count_label(changes, "cambio", "cambios"),
        stats_label(added, removed)
    )
}

/// An answer of the "«x» tiene cambios sin guardar" dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirtyChoice {
    /// Save the user's changes first; the agent's change is reviewed alone.
    Save,
    /// Throw the user's changes away first.
    Discard,
    /// Let the agent write over them; the hunk includes the user's changes.
    Keep,
}

/// Where a navigation command goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    /// `Alt+J`: the next pending change, wrapping across files.
    Next,
    /// `Alt+K`: the previous one.
    Prev,
    /// `Alt+L`: the first change of the next file with changes.
    NextFile,
}

/// State of a row of the review panel (`02-visual.md` §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelState {
    /// Still has pending changes.
    Pending,
    /// Everything in it was decided.
    Decided,
}

/// One row of the review panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelRow {
    /// Absolute path.
    pub path: PathBuf,
    /// Path relative to the project root.
    pub relative: String,
    /// Pending added lines.
    pub added: u32,
    /// Pending removed lines.
    pub removed: u32,
    /// Pending or decided.
    pub state: PanelState,
    /// Created by the agent (`nuevo`).
    pub created: bool,
    /// Deleted by the agent (`eliminado`).
    pub deleted: bool,
    /// Reviewable only as a whole file (too large for inline hunks).
    pub too_large: bool,
    /// The photo kept no copy of it: it can only be accepted.
    pub no_copy: bool,
}

/// A buffer the review listens to.
struct Watched {
    handle: BufferHandle,
    subscription: SubscriptionId,
}

/// An agent request held back while the dirty-buffer dialog is up.
enum Deferred {
    Read {
        line: Option<u32>,
        limit: Option<u32>,
        reply: tokio::sync::oneshot::Sender<Result<String, cincel_acp::FsError>>,
    },
    Write {
        content: String,
        reply: tokio::sync::oneshot::Sender<Result<(), cincel_acp::FsError>>,
    },
    Capture,
    Reread,
}

/// The "¿Rechazar todo el turno?" confirmation the floating bar's "Rechazar
/// todo" opens (`crate::review_close` paints it).
struct RejectTurnPrompt {
    /// Pending decisions of the turn.
    changes: usize,
    /// Files of the turn with something pending.
    files: usize,
    /// Where the keyboard was (the editor), given back on either answer.
    previous_focus: Option<FocusHandle>,
}

/// A pending "cambios sin guardar" question.
struct DirtyPrompt {
    path: PathBuf,
    deferred: Vec<Deferred>,
}

/// Host-side inline limits (`review.max_file_size_kb`, `review.max_lines`).
#[derive(Clone, Copy, Debug)]
struct Limits {
    max_bytes: usize,
    max_lines: usize,
}

impl Limits {
    fn from_settings(settings: &cincel_settings::Settings) -> Self {
        Self {
            max_bytes: (settings.review.max_file_size_kb as usize).saturating_mul(1024),
            max_lines: settings.review.max_lines as usize,
        }
    }

    fn exceeded_by(&self, rope: &Rope) -> bool {
        rope.len_bytes() > self.max_bytes || rope.len_lines() > self.max_lines.saturating_add(1)
    }
}

/// The review entity of a workspace window.
pub struct Review {
    store: ReviewStore,
    project: Option<Entity<Project>>,
    center: WeakEntity<CenterPanel>,
    window: Option<AnyWindowHandle>,
    summary: SharedSummary,
    buffers: HashMap<PathBuf, Watched>,
    /// Filled by the buffer subscriptions, drained on the main thread.
    queue: Arc<Mutex<Vec<(PathBuf, BufferEvent)>>>,
    wake: async_channel::Sender<()>,
    active_turn: Option<TurnId>,
    latest_turn: Option<TurnId>,
    next_turn: u64,
    /// Ended turns whose report has not been sent yet.
    reportable: Vec<TurnId>,
    /// Paths an edit tool call named before they existed.
    absent_before: HashSet<PathBuf>,
    /// Paths where the user chose "Mantener" in this turn.
    keep_over: HashSet<PathBuf>,
    /// The hunk the host calls current, per file.
    current: HashMap<PathBuf, HunkId>,
    /// The line pairs behind the last view pushed to each editor, so a line
    /// action refers to the exact pair the user saw.
    pushed: HashMap<PathBuf, Vec<(u64, Vec<LinePair>)>>,
    /// Files that had pending changes since everything was last decided (the
    /// panel lists them as "decidido" once they have none).
    cycle: BTreeSet<PathBuf>,
    panel_open: bool,
    prompts: VecDeque<DirtyPrompt>,
    /// The bar's "Rechazar todo" waiting for its confirmation.
    reject_turn_prompt: Option<RejectTurnPrompt>,
    /// Keyboard focus of that confirmation (`Esc` cancels it).
    reject_turn_focus: FocusHandle,
    recompute_dirty: BTreeSet<PathBuf>,
    recompute_scheduled: bool,
    persist_scheduled: bool,
    limits: Limits,
    /// The photo of the project of the running turn.
    photo: Option<photo::PhotoState>,
    /// Text files the agent changed or deleted that the photo kept no copy
    /// of (accept only).
    uncopied: BTreeMap<PathBuf, photo::UncopiedChange>,
    /// Takes the photo on the background executor even for an inert project
    /// (a test of the prompt waiting for it).
    photo_in_background: bool,
    /// The sweep has run long enough for the status bar to say so.
    sweep_notice: bool,
    /// How the last end-of-turn sweep went.
    last_sweep: Option<SweepStats>,
    /// The watcher's paths of the running turn still to adopt, in slices.
    watch: photo::WatchQueue,
    project_subscription: Option<Subscription>,
    /// [`FilesChanged`] and [`HostWrote`] of the open project.
    photo_subscriptions: Vec<Subscription>,
    _subscriptions: Vec<Subscription>,
    _wake_task: Task<()>,
}

impl Drop for Review {
    fn drop(&mut self) {
        for (_, watched) in self.buffers.drain() {
            watched.handle.lock().unsubscribe(watched.subscription);
        }
    }
}

impl Review {
    /// Builds the review of the window that shows `center`.
    pub fn new(center: &Entity<CenterPanel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (wake, woken) = async_channel::bounded::<()>(1);
        let wake_task = cx.spawn(async move |this, cx| {
            while woken.recv().await.is_ok() {
                let alive = this
                    .update(cx, |review, cx| review.on_buffers_edited(cx))
                    .is_ok();
                if !alive {
                    break;
                }
            }
        });
        let subscription = cx.subscribe_in(
            center,
            window,
            |this, _, event: &CenterEvent, window, cx| {
                this.on_center_event(event, window, cx);
            },
        );
        let settings = crate::settings::settings(cx);
        Self {
            store: ReviewStore::new(),
            project: None,
            center: center.downgrade(),
            window: Some(window.window_handle()),
            summary: SharedSummary::default(),
            buffers: HashMap::new(),
            queue: Arc::default(),
            wake,
            active_turn: None,
            latest_turn: None,
            next_turn: 1,
            reportable: Vec::new(),
            absent_before: HashSet::new(),
            keep_over: HashSet::new(),
            current: HashMap::new(),
            pushed: HashMap::new(),
            cycle: BTreeSet::new(),
            panel_open: false,
            prompts: VecDeque::new(),
            reject_turn_prompt: None,
            reject_turn_focus: cx.focus_handle(),
            recompute_dirty: BTreeSet::new(),
            recompute_scheduled: false,
            persist_scheduled: false,
            limits: Limits::from_settings(&settings),
            photo: None,
            uncopied: BTreeMap::new(),
            photo_in_background: false,
            sweep_notice: false,
            last_sweep: None,
            watch: photo::WatchQueue::default(),
            project_subscription: None,
            photo_subscriptions: Vec::new(),
            _subscriptions: vec![subscription],
            _wake_task: wake_task,
        }
    }

    /// The summary the painting code reads.
    pub fn summary(&self) -> SharedSummary {
        self.summary.clone()
    }

    /// The store, for the tests and for diagnostics.
    pub fn store(&self) -> &ReviewStore {
        &self.store
    }

    /// Whether an agent turn is running.
    pub fn is_turn_active(&self) -> bool {
        self.active_turn.is_some()
    }

    /// The turn a prompt started last, if any.
    pub fn latest_turn(&self) -> Option<u64> {
        self.latest_turn.map(|turn| turn.0)
    }

    /// Whether the review panel is open.
    pub fn is_panel_open(&self) -> bool {
        self.panel_open
    }

    /// Re-reads `review.*` after a settings reload.
    pub fn reload_settings(&mut self, cx: &mut Context<Self>) {
        self.limits = Limits::from_settings(&crate::settings::settings(cx));
        self.refresh(None, cx);
    }

    // ------------------------------------------------------------- project

    /// Follows the open project: saves the review of the previous one, then
    /// loads the one persisted for `project`
    /// (`03-arquitectura.md` §6: never writes a file while restoring).
    pub fn set_project(&mut self, project: Option<Entity<Project>>, cx: &mut Context<Self>) {
        if self.project.is_some() {
            self.save_now(cx);
        }
        for (_, watched) in self.buffers.drain() {
            watched.handle.lock().unsubscribe(watched.subscription);
        }
        self.queue.lock().clear();
        self.store = ReviewStore::new();
        self.active_turn = None;
        self.latest_turn = None;
        self.next_turn = 1;
        self.reportable.clear();
        self.absent_before.clear();
        self.keep_over.clear();
        self.current.clear();
        self.pushed.clear();
        self.cycle.clear();
        self.panel_open = false;
        self.prompts.clear();
        self.reject_turn_prompt = None;
        self.recompute_dirty.clear();
        self.photo = None;
        self.uncopied.clear();
        self.sweep_notice = false;
        self.watch.clear();
        self.project_subscription = None;
        self.photo_subscriptions.clear();
        self.project = project.clone();

        if let Some(project) = project {
            let root = project.read(cx).root().to_path_buf();
            self.store.set_workspace_root(Some(root));
            self.project_subscription = Some(cx.subscribe(&project, |this, _, event, cx| {
                if let ProjectEvent::BuffersChanged(changes) = event {
                    this.on_disk_changes(changes, cx);
                }
            }));
            self.photo_subscriptions = vec![
                cx.subscribe(&project, |this, _, event: &FilesChanged, cx| {
                    this.on_files_changed(&event.0, cx);
                }),
                cx.subscribe(&project, |this, _, event: &HostWrote, cx| {
                    this.on_host_wrote(&event.0, cx);
                }),
            ];
            self.load(cx);
        }
        self.refresh(None, cx);
    }

    /// `~/.local/share/cincel/review/<hash de ruta>/` for the open project.
    pub fn persist_dir(&self, cx: &App) -> Option<PathBuf> {
        let root = self.project.as_ref()?.read(cx).root().to_path_buf();
        Some(
            dirs::data_dir()?
                .join("cincel")
                .join("review")
                .join(crate::layout::project_hash(&root)),
        )
    }

    /// Writes the review state now.
    pub fn save_now(&mut self, cx: &mut Context<Self>) {
        self.flush(None, cx);
        let Some(dir) = self.persist_dir(cx) else {
            return;
        };
        if let Err(error) = self.store.save(&dir) {
            tracing::warn!(%error, dir = %dir.display(), "no se pudo guardar la revisión");
        }
    }

    fn schedule_persist(&mut self, cx: &mut Context<Self>) {
        if self.persist_scheduled || self.project.is_none() {
            return;
        }
        self.persist_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PERSIST_DEBOUNCE).await;
            let _ = this.update(cx, |review, cx| {
                review.persist_scheduled = false;
                review.save_now(cx);
            });
        })
        .detach();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.persist_dir(cx) else {
            return;
        };
        let report = match self.store.load(&dir, |path| std::fs::read(path).ok()) {
            Ok(report) => report,
            Err(error) => {
                tracing::warn!(%error, dir = %dir.display(), "no se pudo leer la revisión guardada");
                cincel_review::LoadReport::default()
            }
        };
        for (path, reason) in &report.dropped {
            tracing::info!(path = %path.display(), ?reason, "revisión descartada al abrir");
            crate::toast::warn(dropped_message(path, *reason), cx);
        }
        // Pending files reopen their buffers in the background (no tab), so
        // the views are ready the moment a tab shows them.
        for path in &report.restored {
            if !path.is_file() {
                continue;
            }
            if let Some(handle) = self.ensure_buffer(path, cx) {
                let snapshot = handle.lock().snapshot();
                self.store.file_written(path, &snapshot, EditSource::Load);
            }
        }
        remove_legacy_binary_state(&dir);
        let max_turn = self.store.files().map(|file| file.turn_id.0).max();
        if let Some(max_turn) = max_turn {
            self.latest_turn = Some(TurnId(max_turn));
            self.next_turn = max_turn + 1;
        }
        self.cycle = self.store.pending_files().into_iter().collect();
    }

    // ---------------------------------------------------------------- turns

    /// A prompt is going out: closes whatever turn was still open, collects
    /// the report of every ended turn for the agent (then forgets them), and
    /// starts a new turn. Returns the turn id (for
    /// `EditSource::Agent { turn_id }`) and the feedback to attach to
    /// `AgentCommand::Prompt`.
    ///
    /// It also starts the photo of the project the turn is reviewed against
    /// (`review_snapshot.rs`); the prompt must wait for
    /// [`Self::turn_waiter`] before it goes out. When the previous turn is
    /// still being swept (in the background), the new turn only starts once
    /// that is over, and the prompt waits for both.
    pub fn begin_prompt(&mut self, cx: &mut Context<Self>) -> (u64, Option<String>) {
        self.flush(None, cx);
        let turn = TurnId(self.next_turn);
        self.next_turn += 1;
        let ended = self.active_turn.is_none() || self.finish_turn(Some(turn), cx);
        let mut notes = Vec::new();
        for turn in std::mem::take(&mut self.reportable) {
            if let Some(note) = self.store.report_for_agent(turn) {
                notes.push(note);
            }
            self.store.forget_turn(turn);
        }
        if ended {
            self.open_turn(turn, Vec::new(), cx);
        }
        self.refresh(None, cx);
        (turn.0, (!notes.is_empty()).then(|| notes.join("\n")))
    }

    /// Starts `turn` and its photo; `waiters` are told when the photo is
    /// ready.
    fn open_turn(&mut self, turn: TurnId, waiters: Vec<photo::Waiter>, cx: &mut Context<Self>) {
        if self.pending_count() == 0 {
            self.cycle.clear();
        }
        self.store.begin_turn(turn);
        self.active_turn = Some(turn);
        self.latest_turn = Some(turn);
        self.start_photo(turn, waiters, cx);
    }

    /// The agent's turn ended (`AgentEvent::TurnEnded` with any stop
    /// reason, or the process died). Nothing is accepted: what the turn left
    /// waits for the user in the editor.
    ///
    /// First the whole photo is compared with the disk and whatever the
    /// watcher did not report joins the review; only then is the photo
    /// dropped and the turn closed. The comparison runs on the background
    /// executor: until it is over the turn stays active (its decisions stay
    /// disabled) and a new prompt waits (`review_snapshot.rs`).
    pub fn end_turn(&mut self, cx: &mut Context<Self>) {
        self.flush(None, cx);
        if self.active_turn.is_some() {
            self.finish_turn(None, cx);
        }
    }

    /// Closes the running turn once its sweep is over: drops the photo and
    /// keeps the turn for the next prompt's report.
    fn close_turn(&mut self, cx: &mut Context<Self>) {
        photo::release(self.photo.take(), cx);
        self.watch.clear();
        self.sweep_notice = false;
        let Some(turn) = self.active_turn.take() else {
            return;
        };
        self.store.end_turn(turn);
        self.reportable.push(turn);
        self.keep_over.clear();
        self.absent_before.clear();
        self.prune(cx);
        self.refresh(None, cx);
        self.schedule_persist(cx);
    }

    /// The agent went away: its turn is over and nobody will read the answers
    /// held by the dirty-buffer dialog. A prompt waiting for a sweep will
    /// not go out: the turn it would have started is dropped (the sweep
    /// itself goes on and closes the turn).
    pub fn agent_gone(&mut self, cx: &mut Context<Self>) {
        self.prompts.clear();
        if let Some(photo::PhotoState::Sweeping {
            chained, waiters, ..
        }) = &mut self.photo
        {
            *chained = None;
            waiters.clear();
        }
        self.end_turn(cx);
        self.refresh(None, cx);
    }

    fn turn_source(&self) -> EditSource {
        let turn = self.active_turn.or(self.latest_turn).unwrap_or_default();
        EditSource::Agent { turn_id: turn.0 }
    }

    // --------------------------------------------------------- agent -> fs

    /// `fs/read_text_file`: fixes the base from the buffer (first contact),
    /// then answers from memory, unsaved changes included.
    pub fn handle_fs_read(
        &mut self,
        path: PathBuf,
        line: Option<u32>,
        limit: Option<u32>,
        reply: tokio::sync::oneshot::Sender<Result<String, cincel_acp::FsError>>,
        cx: &mut Context<Self>,
    ) {
        if let Some(prompt) = self.prompt_for(&path) {
            prompt.deferred.push(Deferred::Read { line, limit, reply });
            return;
        }
        self.do_read(&path, line, limit, reply, cx);
    }

    fn do_read(
        &mut self,
        path: &Path,
        line: Option<u32>,
        limit: Option<u32>,
        reply: tokio::sync::oneshot::Sender<Result<String, cincel_acp::FsError>>,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project.clone() else {
            let _ = reply.send(Err(cincel_acp::FsError::Internal(
                "no hay un proyecto abierto".to_string(),
            )));
            return;
        };
        self.capture(path, cx);
        let full = project.update(cx, |project, _| {
            project.buffers_mut().read_for_agent(path, None, None)
        });
        let full = match full {
            Ok(text) => text,
            Err(error) => {
                let _ = reply.send(Err(crate::agents::map_open_error(&error)));
                return;
            }
        };
        if let Some(line) = line {
            let total = full.split_inclusive('\n').count().max(1) as u32;
            if line > total {
                let _ = reply.send(Err(cincel_acp::FsError::InvalidParams(format!(
                    "«{}» tiene {total} líneas; se pidió desde la {line}",
                    path.display()
                ))));
                return;
            }
        }
        let sliced = project.update(cx, |project, _| {
            project.buffers_mut().read_for_agent(path, line, limit)
        });
        let _ = reply.send(sliced.map_err(|error| crate::agents::map_open_error(&error)));
        self.refresh(Some(path), cx);
    }

    /// `fs/write_text_file`. A buffer with unsaved changes asks first
    /// ("«x» tiene cambios sin guardar"); the agent waits for the answer,
    /// since it waits for this reply anyway.
    pub fn handle_fs_write(
        &mut self,
        path: PathBuf,
        content: String,
        reply: tokio::sync::oneshot::Sender<Result<(), cincel_acp::FsError>>,
        cx: &mut Context<Self>,
    ) {
        if let Some(prompt) = self.prompt_for(&path) {
            prompt.deferred.push(Deferred::Write { content, reply });
            return;
        }
        if self.needs_prompt(&path, cx) {
            self.ask(path, Deferred::Write { content, reply }, cx);
            return;
        }
        self.do_write(&path, &content, reply, cx);
    }

    fn do_write(
        &mut self,
        path: &Path,
        content: &str,
        reply: tokio::sync::oneshot::Sender<Result<(), cincel_acp::FsError>>,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project.clone() else {
            let _ = reply.send(Err(cincel_acp::FsError::Internal(
                "no hay un proyecto abierto".to_string(),
            )));
            return;
        };
        let source = self.turn_source();
        let EditSource::Agent { turn_id } = source else {
            unreachable!("turn_source is always an agent source");
        };
        let existed = path.is_file();
        if existed {
            self.capture(path, cx);
        }
        self.flush(None, cx);
        let result = project.update(cx, |project, _| {
            project
                .buffers_mut()
                .apply_agent_write(path, content, turn_id)
        });
        match result {
            Ok(write) => {
                let _ = reply.send(Ok(()));
                if write.created || !existed {
                    self.track_created(path, cx);
                } else {
                    self.flush(None, cx);
                    if let Some(handle) = self.ensure_buffer(path, cx) {
                        let snapshot = handle.lock().snapshot();
                        let tracked = self.store.file_written(path, &snapshot, source);
                        self.note(path, tracked);
                    }
                }
                if let Some(center) = self.center.upgrade() {
                    center.update(cx, |center, cx| center.note_agent_write(path, cx));
                }
                project.read(cx).refresh_git();
                self.refresh(Some(path), cx);
            }
            Err(error) => {
                let _ = reply.send(Err(cincel_acp::FsError::Internal(error.to_string())));
            }
        }
    }

    /// A `tool_call` of kind `edit`/`delete`/`move` named `paths`: first
    /// contact, so the base is captured now. A path whose buffer has unsaved
    /// changes asks first, unless the permission for the tool call was
    /// already granted automatically (`auto_granted`): then nobody can wait
    /// for the answer and the buffer is saved (`03-arquitectura.md` §4.1).
    ///
    /// `late_diffs` is for a tool call whose first mention of a path is
    /// already its completion (the file on disk is the agent's by then): the
    /// `oldText` of its `diff` is the base (`03-arquitectura.md` §4.1), and a
    /// `diff` without `oldText` means the file is new.
    pub fn tool_call_started(
        &mut self,
        paths: &[PathBuf],
        late_diffs: &HashMap<PathBuf, Option<String>>,
        auto_granted: bool,
        cx: &mut Context<Self>,
    ) {
        for path in paths {
            if !self.in_project(path, cx) || self.store.file(path).is_some() {
                continue;
            }
            if let Some(prompt) = self.prompt_for(path) {
                prompt.deferred.push(Deferred::Capture);
                continue;
            }
            let open = self
                .project
                .as_ref()
                .is_some_and(|project| project.read(cx).buffers().is_open(path));
            if !open && let Some(old_text) = late_diffs.get(path) {
                match old_text {
                    Some(old_text) => {
                        self.store.capture_base_text(path, old_text);
                    }
                    None => {
                        self.absent_before.insert(path.clone());
                    }
                }
                continue;
            }
            if self.needs_prompt(path, cx) {
                if auto_granted {
                    tracing::info!(path = %path.display(), "permiso ya concedido: se guarda antes de que escriba el agente");
                    self.save_buffer(path, cx);
                } else {
                    self.ask(path.clone(), Deferred::Capture, cx);
                    continue;
                }
            }
            self.capture(path, cx);
        }
        self.refresh(None, cx);
    }

    /// An edit tool call finished: the truth is what is on disk now.
    pub fn tool_call_finished(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        self.reread_paths(paths, cx);
    }

    /// Re-reads `paths` from disk (a completed tool call, the
    /// `agentFileChangeReport` of the turn).
    pub fn reread_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let mut touched = false;
        for path in paths {
            if let Some(prompt) = self.prompt_for(path) {
                prompt.deferred.push(Deferred::Reread);
                continue;
            }
            touched |= self.reread(path, cx);
        }
        if touched && let Some(project) = &self.project {
            project.read(cx).refresh_git();
        }
        self.refresh(None, cx);
    }

    /// Re-reads one path. Returns whether anything was done.
    fn reread(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        if !self.in_project(path, cx) {
            return false;
        }
        self.flush(None, cx);
        let exists = path.is_file();
        let source = self.turn_source();
        if self.store.file(path).is_none() && self.ready_snapshot().is_some() {
            if self.absent_before.remove(path) && exists {
                // A tool call named it before it existed (v1's rule, kept
                // for paths the photo does not cover, such as ignored ones).
                self.track_created(path, cx);
                return true;
            }
            // The photo knows what the file was before the turn, however the
            // agent changed it.
            return self.adopt_change(path, cx);
        }
        if self.store.file(path).is_none() {
            if !exists {
                self.absent_before.remove(path);
                return false;
            }
            let open = self
                .project
                .as_ref()
                .is_some_and(|project| project.read(cx).buffers().is_open(path));
            let known = self
                .project
                .as_ref()
                .is_some_and(|project| project.read(cx).worktree().contains(path));
            if self.absent_before.remove(path) || (!open && !known) {
                self.track_created(path, cx);
                return true;
            }
            if !open {
                // An existing file nobody announced before writing it: there
                // is no way to know what it held before.
                tracing::warn!(path = %path.display(), "archivo tocado por el agente sin base conocida; no se revisa");
                return false;
            }
            // Its buffer still holds the text from before the agent wrote.
            self.capture(path, cx);
        }
        if !exists {
            let previous = self
                .store
                .file(path)
                .map(|file| file.current().rope().clone())
                .unwrap_or_default();
            let tracked = self.store.file_deleted(path, previous);
            self.note(path, tracked);
            if let Some(project) = &self.project {
                project.update(cx, |project, _| {
                    project.buffers_mut().reload_from_disk(path);
                });
            }
            self.tell_center(path, ReloadOutcome::Deleted, cx);
            return true;
        }
        let Some(handle) = self.ensure_buffer(path, cx) else {
            return false;
        };
        let Some(project) = self.project.clone() else {
            return false;
        };
        let force = handle.lock().is_dirty() && self.keep_over.contains(path);
        let outcome = project.update(cx, |project, _| {
            if force {
                project.buffers_mut().discard_changes(path)
            } else {
                project.buffers_mut().reload_from_disk(path)
            }
        });
        self.flush(Some(source), cx);
        let snapshot = handle.lock().snapshot();
        let tracked = self.store.file_written(path, &snapshot, source);
        self.note(path, tracked);
        self.tell_center(path, outcome, cx);
        true
    }

    /// Starts tracking a file the agent created (it exists now, and did not).
    fn track_created(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(handle) = self.ensure_buffer(path, cx) else {
            return;
        };
        // The events of the write that created the buffer describe a text the
        // store never had: the file enters the review whole.
        self.queue.lock().retain(|(queued, _)| queued != path);
        let snapshot = handle.lock().snapshot();
        let text = snapshot.text();
        let tracked = self.store.file_created(path, &text, None);
        self.note(path, tracked);
        let tracked = self.store.file_written(path, &snapshot, self.turn_source());
        self.note(path, tracked);
    }

    /// Fixes the base of `path` if it is not in review yet (first contact).
    fn capture(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !self.in_project(path, cx) {
            return;
        }
        if self.store.file(path).is_some() {
            self.ensure_buffer(path, cx);
            return;
        }
        if !path.is_file() {
            self.absent_before.insert(path.to_path_buf());
            return;
        }
        let Some(handle) = self.ensure_buffer(path, cx) else {
            return;
        };
        self.flush(None, cx);
        let snapshot = handle.lock().snapshot();
        let dirty = handle.lock().is_dirty();
        if !dirty && let Some(before) = self.photo_text(path) {
            let before = photo::normalize(&before);
            if before != snapshot.text() {
                // The agent already changed it by other means: the base is
                // the photo, and the difference is the agent's.
                self.store.capture_base_text(path, &before);
                let tracked = self.store.file_written(path, &snapshot, self.turn_source());
                self.note(path, tracked);
                tracing::debug!(path = %path.display(), "base tomada de la foto");
                return;
            }
        }
        if self.store.capture_base(path, &snapshot) {
            tracing::debug!(path = %path.display(), "base capturada");
        }
    }

    fn in_project(&self, path: &Path, cx: &App) -> bool {
        self.project
            .as_ref()
            .is_some_and(|project| path.starts_with(project.read(cx).root()))
    }

    // ------------------------------------------------------ dirty dialog

    fn prompt_for(&mut self, path: &Path) -> Option<&mut DirtyPrompt> {
        self.prompts.iter_mut().find(|prompt| prompt.path == path)
    }

    /// Whether writing `path` needs the "cambios sin guardar" question.
    fn needs_prompt(&self, path: &Path, cx: &App) -> bool {
        if self.keep_over.contains(path) {
            return false;
        }
        self.project
            .as_ref()
            .is_some_and(|project| project.read(cx).buffers().is_dirty(path))
    }

    fn ask(&mut self, path: PathBuf, first: Deferred, cx: &mut Context<Self>) {
        tracing::info!(path = %path.display(), "el agente va a escribir un archivo con cambios sin guardar");
        self.prompts.push_back(DirtyPrompt {
            path,
            deferred: vec![first],
        });
        self.refresh(None, cx);
    }

    /// The file the dialog asks about, if it is up.
    pub fn dirty_prompt(&self) -> Option<&Path> {
        self.prompts.front().map(|prompt| prompt.path.as_path())
    }

    /// Answers the dialog and lets the held requests through.
    pub fn answer_dirty(&mut self, choice: DirtyChoice, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompts.pop_front() else {
            return;
        };
        let path = prompt.path;
        match choice {
            DirtyChoice::Save => self.save_buffer(&path, cx),
            DirtyChoice::Discard => self.discard_buffer(&path, cx),
            DirtyChoice::Keep => {
                self.keep_over.insert(path.clone());
                // The base is the file as the user last saved it, so the
                // hunk carries the unsaved changes too.
                if self.store.file(&path).is_none()
                    && let Ok(bytes) = std::fs::read(&path)
                    && self
                        .store
                        .capture_base_bytes(&path, &bytes)
                        .unwrap_or(false)
                    && let Some(handle) = self.ensure_buffer(&path, cx)
                {
                    self.flush(None, cx);
                    let snapshot = handle.lock().snapshot();
                    let tracked = self
                        .store
                        .file_written(&path, &snapshot, self.turn_source());
                    self.note(&path, tracked);
                }
            }
        }
        for deferred in prompt.deferred {
            match deferred {
                Deferred::Read { line, limit, reply } => {
                    self.do_read(&path, line, limit, reply, cx)
                }
                Deferred::Write { content, reply } => self.do_write(&path, &content, reply, cx),
                Deferred::Capture => self.capture(&path, cx),
                Deferred::Reread => {
                    self.reread(&path, cx);
                }
            }
        }
        self.refresh(None, cx);
    }

    // ------------------------------------------------------------- buffers

    /// The buffer of `path`, opened in the background if needed, and
    /// subscribed. A new buffer instance for a tracked path is resynced with
    /// `file_written`, as the protocol requires.
    fn ensure_buffer(&mut self, path: &Path, cx: &mut Context<Self>) -> Option<BufferHandle> {
        let project = self.project.clone()?;
        let handle = match project.update(cx, |project, _| project.buffers_mut().open(path)) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::debug!(path = %path.display(), %error, "sin buffer para la revisión");
                return None;
            }
        };
        let same = self
            .buffers
            .get(path)
            .is_some_and(|watched| Arc::ptr_eq(&watched.handle, &handle));
        if !same {
            let replaced = self.buffers.remove(path);
            if let Some(old) = &replaced {
                old.handle.lock().unsubscribe(old.subscription);
            }
            let queue = self.queue.clone();
            let wake = self.wake.clone();
            let key = path.to_path_buf();
            let subscription = handle.lock().subscribe(Box::new(move |event| {
                queue.lock().push((key.clone(), event.clone()));
                let _ = wake.try_send(());
            }));
            self.buffers.insert(
                path.to_path_buf(),
                Watched {
                    handle: handle.clone(),
                    subscription,
                },
            );
            if replaced.is_some() && self.store.file(path).is_some() {
                let snapshot = handle.lock().snapshot();
                let tracked = self.store.file_written(path, &snapshot, EditSource::Load);
                self.note(path, tracked);
            }
        }
        Some(handle)
    }

    /// Feeds the queued buffer events to the store. `source` overrides the
    /// events' own when the host made the edit itself and knows who it is
    /// for; otherwise a `Load` (a reload from disk) during a turn is the
    /// agent's.
    fn flush(&mut self, source: Option<EditSource>, _cx: &mut Context<Self>) -> BTreeSet<PathBuf> {
        let events = std::mem::take(&mut *self.queue.lock());
        let mut changed = BTreeSet::new();
        if events.is_empty() {
            return changed;
        }
        let mut snapshots: HashMap<PathBuf, BufferSnapshot> = HashMap::new();
        for (path, event) in events {
            let Some(watched) = self.buffers.get(&path) else {
                continue;
            };
            let snapshot = snapshots
                .entry(path.clone())
                .or_insert_with(|| watched.handle.lock().snapshot())
                .clone();
            let BufferEvent::Edited { source: own, .. } = &event;
            let source = source.unwrap_or_else(|| self.classify(*own));
            let tracked = self.store.buffer_edited(&path, &event, &snapshot, source);
            if tracked.changed {
                changed.insert(path.clone());
            }
            self.note(&path, tracked);
        }
        changed
    }

    fn classify(&self, own: EditSource) -> EditSource {
        match (own, self.active_turn) {
            (EditSource::Load, Some(turn)) => EditSource::Agent { turn_id: turn.0 },
            (other, _) => other,
        }
    }

    fn note(&mut self, path: &Path, tracked: Tracked) {
        if tracked.needs_recompute {
            self.recompute_dirty.insert(path.to_path_buf());
        }
    }

    /// The wake-up task: somebody edited a buffer in review.
    fn on_buffers_edited(&mut self, cx: &mut Context<Self>) {
        let changed = self.flush(None, cx);
        if changed.is_empty() && self.recompute_dirty.is_empty() {
            return;
        }
        self.after_edits(changed, cx);
    }

    fn after_edits(&mut self, changed: BTreeSet<PathBuf>, cx: &mut Context<Self>) {
        let before = self.summary.borrow().pending;
        self.prune(cx);
        let after = self.pending_count();
        if before != after || changed.len() > 1 {
            self.refresh(None, cx);
        } else if let Some(path) = changed.iter().next() {
            self.refresh(Some(path), cx);
        } else {
            self.schedule_recompute(cx);
        }
    }

    /// The watcher reconciled buffers with the disk.
    fn on_disk_changes(&mut self, changes: &[BufferChange], cx: &mut Context<Self>) {
        self.flush(None, cx);
        let mut any = false;
        for change in changes {
            if self.store.file(&change.path).is_none() {
                continue;
            }
            any = true;
            match change.outcome {
                ReloadOutcome::Deleted => {
                    let previous = self
                        .store
                        .file(&change.path)
                        .map(|file| file.current().rope().clone())
                        .unwrap_or_default();
                    let tracked = self.store.file_deleted(&change.path, previous);
                    self.note(&change.path, tracked);
                }
                ReloadOutcome::Reloaded => {
                    if let Some(handle) = self.ensure_buffer(&change.path, cx) {
                        let snapshot = handle.lock().snapshot();
                        let source = self.classify(EditSource::Load);
                        let tracked = self.store.file_written(&change.path, &snapshot, source);
                        self.note(&change.path, tracked);
                    }
                }
                _ => {}
            }
        }
        if any {
            self.prune(cx);
            self.refresh(None, cx);
        }
    }

    /// Stops listening to buffers that left the review, and closes the ones
    /// only the review had open.
    fn prune(&mut self, cx: &mut Context<Self>) {
        let stale: Vec<PathBuf> = self
            .buffers
            .keys()
            .filter(|path| {
                self.store.file(path).is_none()
                    && !self.prompts.iter().any(|prompt| &prompt.path == *path)
            })
            .cloned()
            .collect();
        if stale.is_empty() {
            return;
        }
        let open_tabs: HashSet<PathBuf> = self
            .center
            .upgrade()
            .map(|center| {
                center
                    .read(cx)
                    .tabs()
                    .iter()
                    .map(|tab| tab.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        for path in stale {
            if let Some(watched) = self.buffers.remove(&path) {
                watched.handle.lock().unsubscribe(watched.subscription);
                let dirty = watched.handle.lock().is_dirty();
                if !open_tabs.contains(&path)
                    && !dirty
                    && let Some(project) = &self.project
                {
                    project.update(cx, |project, _| project.buffers_mut().close(&path));
                }
            }
            self.current.remove(&path);
            self.pushed.remove(&path);
        }
    }

    fn save_buffer(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        match project.update(cx, |project, cx| project.save(path, cx)) {
            Ok(()) => {
                // `HostWrote` arrives after this update; the photo has to
                // know now (a capture may follow right away).
                self.on_host_wrote(path, cx);
                if let Some(center) = self.center.upgrade() {
                    center.update(cx, |center, cx| center.note_saved(path, cx));
                }
                project.read(cx).refresh_git();
            }
            Err(error) => crate::toast::error(error, cx),
        }
    }

    /// Throws the unsaved changes of `path` away, as the user's own edit
    /// (their changes were rebased into the base, so their removal goes
    /// there too).
    ///
    /// The buffer is set to the disk text with minimal edits here rather
    /// than through `BufferStore::discard_changes`, which only reloads when
    /// the disk changed since the last read (it is meant for conflicts).
    fn discard_buffer(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(handle) = self.ensure_buffer(path, cx) else {
            return;
        };
        let disk = match std::fs::read(path) {
            Ok(bytes) => match cincel_text::Buffer::from_bytes(&bytes) {
                Ok(buffer) => buffer.text(),
                Err(_) => return,
            },
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "no se pudo releer para descartar");
                return;
            }
        };
        self.flush(None, cx);
        handle.lock().set_text_minimal(&disk, EditSource::User);
        self.flush(Some(EditSource::User), cx);
        if let Some(center) = self.center.upgrade() {
            center.update(cx, |center, cx| center.note_saved(path, cx));
        }
    }

    fn tell_center(&self, path: &Path, outcome: ReloadOutcome, cx: &mut Context<Self>) {
        if !matches!(
            outcome,
            ReloadOutcome::Reloaded | ReloadOutcome::Conflict | ReloadOutcome::Deleted
        ) {
            return;
        }
        if let Some(center) = self.center.upgrade() {
            center.update(cx, |center, cx| {
                center.apply_buffer_changes(
                    &[BufferChange {
                        path: path.to_path_buf(),
                        outcome,
                    }],
                    cx,
                )
            });
        }
    }

    // ----------------------------------------------------------- recompute

    fn schedule_recompute(&mut self, cx: &mut Context<Self>) {
        if self.recompute_scheduled || self.recompute_dirty.is_empty() {
            return;
        }
        self.recompute_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RECOMPUTE_DEBOUNCE).await;
            let Ok(jobs) = this.update(cx, |review, cx| {
                review.recompute_scheduled = false;
                let changed = review.flush(None, cx);
                review.recompute_dirty.extend(changed);
                let paths = std::mem::take(&mut review.recompute_dirty);
                paths
                    .iter()
                    .filter_map(|path| review.store.recompute_job(path))
                    .collect::<Vec<_>>()
            }) else {
                return;
            };
            if jobs.is_empty() {
                return;
            }
            let results: Vec<RecomputeResult> = cx
                .background_executor()
                .spawn(async move { jobs.iter().map(|job| job.run()).collect() })
                .await;
            let _ = this.update(cx, |review, cx| review.apply_results(results, cx));
        })
        .detach();
    }

    fn apply_results(&mut self, results: Vec<RecomputeResult>, cx: &mut Context<Self>) {
        let mut paths = BTreeSet::new();
        for result in results {
            let path = result.path().to_path_buf();
            let outcome = self.store.apply_recompute(result);
            tracing::trace!(path = %path.display(), ?outcome, "recálculo");
            paths.insert(path);
        }
        self.prune(cx);
        if paths.len() == 1 {
            let path = paths.into_iter().next().expect("one path");
            self.refresh(Some(&path), cx);
        } else {
            self.refresh(None, cx);
        }
    }

    // ----------------------------------------------------------- decisions

    /// A review action from the editor of `path`. `origin` is the buffer row
    /// the cursor was on before the editor moved it (for the navigation).
    pub fn handle_action(
        &mut self,
        path: &Path,
        action: ReviewAction,
        origin: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush(None, cx);
        if action.is_decision() && self.store.turn_active(path) {
            return;
        }
        // The single hunk of a deleted file's tab stands for the file.
        let action = match action {
            ReviewAction::AcceptHunk(DELETED_FILE_HUNK) => ReviewAction::AcceptFile,
            ReviewAction::RejectHunk(DELETED_FILE_HUNK) => ReviewAction::RejectFile,
            ReviewAction::AcceptLine {
                hunk: DELETED_FILE_HUNK,
                ..
            } => ReviewAction::AcceptFile,
            ReviewAction::RejectLine {
                hunk: DELETED_FILE_HUNK,
                ..
            } => ReviewAction::RejectFile,
            other => other,
        };
        match action {
            ReviewAction::AcceptHunk(id) => {
                if let Err(error) = self.store.accept_hunk(HunkId(id)) {
                    tracing::debug!(%error, "aceptar segmento");
                }
                self.decided(path, cx);
            }
            ReviewAction::RejectHunk(id) => match self.store.reject_hunk(HunkId(id)) {
                Ok(revert) => {
                    self.apply_reverts(vec![revert], cx);
                    self.offer_undo("Segmento rechazado", cx);
                    self.decided(path, cx);
                }
                Err(error) => tracing::debug!(%error, "rechazar segmento"),
            },
            ReviewAction::AcceptLine { hunk, line } => {
                if let Some(pair) = self.pushed_pair(path, hunk, line) {
                    if let Err(error) = self.store.accept_line(path, &pair) {
                        tracing::debug!(%error, "aceptar línea");
                    }
                    self.decided(path, cx);
                }
            }
            ReviewAction::RejectLine { hunk, line } => {
                if let Some(pair) = self.pushed_pair(path, hunk, line) {
                    match self.store.reject_line(path, &pair) {
                        Ok(revert) => {
                            self.apply_reverts(vec![revert], cx);
                            self.offer_undo("Línea rechazada", cx);
                        }
                        Err(error) => tracing::debug!(%error, "rechazar línea"),
                    }
                    self.decided(path, cx);
                }
            }
            ReviewAction::AcceptFile => self.accept_file(path, cx),
            ReviewAction::RejectFile => self.reject_file(path, cx),
            ReviewAction::AcceptTurn => self.accept_turn(cx),
            ReviewAction::RejectTurn => self.request_reject_turn(window, cx),
            ReviewAction::NextHunk => self.navigate(
                Nav::Next,
                Some((path.to_path_buf(), origin.unwrap_or(0))),
                window,
                cx,
            ),
            ReviewAction::PrevHunk => self.navigate(
                Nav::Prev,
                Some((path.to_path_buf(), origin.unwrap_or(0))),
                window,
                cx,
            ),
            ReviewAction::NextFile => self.navigate(
                Nav::NextFile,
                Some((path.to_path_buf(), origin.unwrap_or(0))),
                window,
                cx,
            ),
            ReviewAction::OpenReviewPanel => self.set_panel_open(true, cx),
            ReviewAction::UndoLastReject => self.undo_last_reject(cx),
        }
    }

    fn pushed_pair(&self, path: &Path, hunk: u64, line: usize) -> Option<LinePair> {
        self.pushed
            .get(path)?
            .iter()
            .find(|(id, _)| *id == hunk)?
            .1
            .get(line)
            .copied()
    }

    /// Bookkeeping after a decision in `path`: the editor picks the next
    /// hunk itself (`jump_to_next_on_decide`), so the host's current hunk of
    /// the file is dropped.
    fn decided(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.current.remove(path);
        self.prune(cx);
        self.refresh(None, cx);
        self.schedule_persist(cx);
    }

    /// Accepts every pending change of `path`.
    pub fn accept_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.flush(None, cx);
        if let Some(change) = self.uncopied.get(path) {
            if Some(change.turn) != self.active_turn {
                self.uncopied.remove(path);
                self.decided(path, cx);
            }
            return;
        }
        if self.store.turn_active(path) {
            return;
        }
        if let Err(error) = self.store.accept_file(path) {
            tracing::debug!(%error, "aceptar archivo");
        }
        self.decided(path, cx);
    }

    /// Rejects every pending change of `path`.
    pub fn reject_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.flush(None, cx);
        if self.uncopied.contains_key(path) {
            self.warn_uncopied(path, cx);
            return;
        }
        if self.store.turn_active(path) {
            return;
        }
        match self.store.reject_file(path) {
            Ok(revert) => {
                self.apply_reverts(vec![revert], cx);
                self.offer_undo("Archivo rechazado", cx);
            }
            Err(error) => tracing::debug!(%error, "rechazar archivo"),
        }
        self.decided(path, cx);
    }

    /// A reject of a text file the photo kept no copy of: nothing to put
    /// back, it can only be accepted.
    fn warn_uncopied(&self, path: &Path, cx: &mut Context<Self>) {
        crate::toast::warn(
            format!(
                "No hay copia previa de «{}»: solo se puede aceptar",
                file_name(path)
            ),
            cx,
        );
    }

    /// Accepts every uncopied change of `turn` (every turn with `None`).
    fn accept_uncopied(&mut self, turn: Option<TurnId>) {
        self.uncopied
            .retain(|_, change| turn.is_some_and(|turn| change.turn != turn));
    }

    /// A reject of the uncopied changes of `turn` (every turn with `None`):
    /// they stay pending, with a notice each.
    fn reject_uncopied(&self, turn: Option<TurnId>, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = self
            .uncopied
            .iter()
            .filter(|(_, change)| turn.is_none_or(|turn| change.turn == turn))
            .map(|(path, _)| path.clone())
            .collect();
        for path in paths {
            self.warn_uncopied(&path, cx);
        }
    }

    /// The turn `accept_turn`/`reject_turn` act on: the last one that left
    /// something pending.
    fn target_turn(&self) -> Option<TurnId> {
        if let Some(turn) = self.latest_turn
            && (self
                .store
                .files()
                .any(|file| file.turn_id == turn && file.has_pending_work())
                || self.uncopied.values().any(|change| change.turn == turn))
        {
            return Some(turn);
        }
        self.store
            .files()
            .filter(|file| file.has_pending_work())
            .map(|file| file.turn_id)
            .chain(self.uncopied.values().map(|change| change.turn))
            .max()
    }

    /// `workspace::accept_turn` (`Ctrl+Alt+Enter`).
    pub fn accept_turn(&mut self, cx: &mut Context<Self>) {
        self.flush(None, cx);
        if self.active_turn.is_some() {
            crate::toast::info_keyed("review", "El agente todavía está editando", cx);
            return;
        }
        let Some(turn) = self.target_turn() else {
            crate::toast::info_keyed("review", "No hay cambios pendientes", cx);
            return;
        };
        self.store.accept_turn(turn);
        self.accept_uncopied(Some(turn));
        self.all_decided(cx);
    }

    /// `workspace::reject_turn` (`Ctrl+Alt+Backspace`).
    pub fn reject_turn(&mut self, cx: &mut Context<Self>) {
        self.flush(None, cx);
        if self.active_turn.is_some() {
            crate::toast::info_keyed("review", "El agente todavía está editando", cx);
            return;
        }
        let Some(turn) = self.target_turn() else {
            crate::toast::info_keyed("review", "No hay cambios pendientes", cx);
            return;
        };
        let reverts = self.store.reject_turn(turn);
        self.apply_reverts(reverts, cx);
        self.reject_uncopied(Some(turn), cx);
        self.offer_undo("Turno rechazado", cx);
        self.all_decided(cx);
    }

    /// `(pending decisions, files)` of the turn `accept_turn` /
    /// `reject_turn` would decide.
    pub fn turn_counts(&self) -> (usize, usize) {
        let Some(turn) = self.target_turn() else {
            return (0, 0);
        };
        let uncopied = self
            .uncopied
            .values()
            .filter(|change| change.turn == turn)
            .count();
        self.store
            .files()
            .filter(|file| file.turn_id == turn && file.has_pending_work())
            .fold((uncopied, uncopied), |(changes, files), file| {
                (changes + self.decisions(file), files + 1)
            })
    }

    /// `(pending decisions, files)` over the whole review, every turn: what
    /// closing the window or the project asks about (`crate::review_close`).
    pub fn pending_counts(&self) -> (usize, usize) {
        let uncopied = self.uncopied.len();
        self.store
            .files()
            .filter(|file| file.has_pending_work())
            .fold((uncopied, uncopied), |(changes, files), file| {
                (changes + self.decisions(file), files + 1)
            })
    }

    /// The floating bar's "Rechazar todo": the whole turn goes, so it asks
    /// first ("Rechazar N cambios en M archivos" / "Cancelar"). The answer is
    /// [`Self::confirm_reject_turn`] or [`Self::cancel_reject_turn`].
    pub fn request_reject_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush(None, cx);
        if self.active_turn.is_some() {
            crate::toast::info_keyed("review", "El agente todavía está editando", cx);
            return;
        }
        let (changes, files) = self.turn_counts();
        if changes == 0 {
            crate::toast::info_keyed("review", "No hay cambios pendientes", cx);
            return;
        }
        let previous_focus = self
            .reject_turn_prompt
            .take()
            .and_then(|prompt| prompt.previous_focus)
            .or_else(|| window.focused(cx));
        self.reject_turn_prompt = Some(RejectTurnPrompt {
            changes,
            files,
            previous_focus,
        });
        window.focus(&self.reject_turn_focus, cx);
        cx.notify();
    }

    /// `(changes, files)` the "Rechazar todo" confirmation asks about, while
    /// it is up.
    pub fn reject_turn_prompt(&self) -> Option<(usize, usize)> {
        self.reject_turn_prompt
            .as_ref()
            .map(|prompt| (prompt.changes, prompt.files))
    }

    /// The focus handle of the "Rechazar todo" confirmation.
    pub fn reject_turn_focus(&self) -> &FocusHandle {
        &self.reject_turn_focus
    }

    /// The confirmation's "Rechazar N cambios en M archivos": the same as
    /// `workspace::reject_turn`, undo toast included (`Alt+Shift+U`).
    pub fn confirm_reject_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.reject_turn_prompt.take() else {
            return;
        };
        if let Some(previous) = &prompt.previous_focus {
            window.focus(previous, cx);
        }
        self.reject_turn(cx);
        cx.notify();
    }

    /// The confirmation's "Cancelar" (`Esc`): nothing is decided.
    pub fn cancel_reject_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.reject_turn_prompt.take() else {
            return;
        };
        if let Some(previous) = &prompt.previous_focus {
            window.focus(previous, cx);
        }
        cx.notify();
    }

    /// "Aceptar todo" of the review panel.
    pub fn accept_all(&mut self, cx: &mut Context<Self>) {
        self.flush(None, cx);
        if self.active_turn.is_some() {
            crate::toast::info_keyed("review", "El agente todavía está editando", cx);
            return;
        }
        self.store.accept_all();
        self.accept_uncopied(None);
        self.all_decided(cx);
    }

    /// "Rechazar todo" of the review panel.
    pub fn reject_all(&mut self, cx: &mut Context<Self>) {
        self.flush(None, cx);
        if self.active_turn.is_some() {
            crate::toast::info_keyed("review", "El agente todavía está editando", cx);
            return;
        }
        let reverts = self.store.reject_all();
        self.reject_uncopied(None, cx);
        if reverts.is_empty() {
            return;
        }
        self.apply_reverts(reverts, cx);
        self.offer_undo("Cambios rechazados", cx);
        self.all_decided(cx);
    }

    fn all_decided(&mut self, cx: &mut Context<Self>) {
        self.current.clear();
        self.prune(cx);
        self.refresh(None, cx);
        self.schedule_persist(cx);
    }

    /// `Alt+Shift+U`: takes the last rejection back (the store's own stack).
    pub fn undo_last_reject(&mut self, cx: &mut Context<Self>) {
        self.flush(None, cx);
        if !self.store.can_undo_reject() {
            crate::toast::info_keyed("review", "No hay rechazos para deshacer", cx);
            return;
        }
        let reverts = self.store.undo_last_reject();
        self.apply_reverts(reverts, cx);
        self.all_decided(cx);
    }

    /// The "Segmento rechazado · Deshacer (Alt+Shift+U)" toast.
    fn offer_undo(&self, what: &str, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        crate::toast::offer(
            "review-reject",
            format!("{what} · Deshacer (Alt+Shift+U)"),
            "Deshacer",
            move |cx: &mut App| {
                if let Some(review) = weak.upgrade() {
                    review.update(cx, |review, cx| review.undo_last_reject(cx));
                }
            },
            cx,
        );
    }

    /// Carries out rejects: one `EditSource::Review` transaction plus a save
    /// per file, or the file operation.
    fn apply_reverts(&mut self, reverts: Vec<Revert>, cx: &mut Context<Self>) {
        for revert in reverts {
            match revert {
                Revert::Edits { path, edits } => {
                    if edits.is_empty() {
                        continue;
                    }
                    let Some(handle) = self.ensure_buffer(&path, cx) else {
                        tracing::warn!(path = %path.display(), "no se pudo abrir el archivo para revertir");
                        continue;
                    };
                    self.flush(None, cx);
                    {
                        let edits: Vec<(Range<usize>, &str)> = edits
                            .iter()
                            .map(|edit| (edit.range.clone(), edit.text.as_str()))
                            .collect();
                        let mut buffer = handle.lock();
                        buffer.start_transaction(EditSource::Review);
                        buffer.edit_many(&edits);
                        buffer.end_transaction();
                    }
                    self.flush(Some(EditSource::Review), cx);
                    self.save_buffer(&path, cx);
                }
                Revert::File(FileOp::DeleteFile(path)) => {
                    match std::fs::remove_file(&path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            crate::toast::error(
                                format!("No se pudo borrar «{}»: {error}", file_name(&path)),
                                cx,
                            );
                            continue;
                        }
                    }
                    self.on_host_wrote(&path, cx);
                    if let Some(watched) = self.buffers.remove(&path) {
                        watched.handle.lock().unsubscribe(watched.subscription);
                    }
                    self.queue.lock().retain(|(queued, _)| queued != &path);
                    self.close_tab_later(&path, cx);
                    if let Some(project) = &self.project {
                        project.read(cx).refresh_git();
                    }
                }
                Revert::File(FileOp::WriteFile(path, text)) => {
                    self.write_file(&path, &text, cx);
                }
            }
        }
    }

    /// Restores `path` with `text` (a rejected deletion, an undone reject of
    /// a created file).
    fn write_file(&mut self, path: &Path, text: &str, cx: &mut Context<Self>) {
        let open = self
            .project
            .as_ref()
            .is_some_and(|project| project.read(cx).buffers().is_open(path));
        if open {
            if let Some(handle) = self.ensure_buffer(path, cx) {
                self.flush(None, cx);
                handle.lock().set_text_minimal(text, EditSource::Review);
                self.flush(Some(EditSource::Review), cx);
                self.save_buffer(path, cx);
            }
        } else {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(error) = std::fs::write(path, text) {
                crate::toast::error(
                    format!("No se pudo escribir «{}»: {error}", file_name(path)),
                    cx,
                );
                return;
            }
            self.on_host_wrote(path, cx);
        }
        if self.store.file(path).is_some()
            && let Some(handle) = self.ensure_buffer(path, cx)
        {
            let snapshot = handle.lock().snapshot();
            let tracked = self.store.file_written(path, &snapshot, EditSource::Load);
            self.note(path, tracked);
        }
        if let Some(project) = &self.project {
            project.update(cx, |project, _| {
                project.buffers_mut().reload_from_disk(path);
            });
            project.read(cx).refresh_git();
        }
        self.tell_center(path, ReloadOutcome::Reloaded, cx);
    }

    /// Closes the tab of `path` once the current update is over (closing a
    /// tab moves the focus, which needs the window).
    fn close_tab_later(&self, path: &Path, cx: &mut Context<Self>) {
        let (Some(window), Some(center)) = (self.window, self.center.upgrade()) else {
            return;
        };
        let path = path.to_path_buf();
        let project = self.project.clone();
        cx.defer(move |cx| {
            let closed = window.update(cx, |_, window, cx| {
                center.update(cx, |center, cx| center.close_path(&path, window, cx))
            });
            if closed.is_err()
                && let Some(project) = project
            {
                project.update(cx, |project, cx| project.close_buffer(&path, cx));
            }
        });
    }

    // ---------------------------------------------------------- navigation

    /// `Alt+J` / `Alt+K` / `Alt+L`, from the editor or the global commands.
    /// `origin` is where the cursor was; `None` takes the active tab.
    pub fn navigate(
        &mut self,
        nav: Nav,
        origin: Option<(PathBuf, u32)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush(None, cx);
        let origin = origin.or_else(|| {
            let center = self.center.upgrade()?;
            let center = center.read(cx);
            center
                .active_tab()
                .map(|tab| (tab.path.clone(), tab.cursor.row))
        });
        let (path, row) = origin.unwrap_or_default();
        let whole = self
            .store
            .file(&path)
            .is_some_and(|file| self.is_file_level(file));
        let location = match nav {
            Nav::Next => self
                .store
                .next_hunk((&path, if whole { u32::MAX } else { row })),
            Nav::Prev => self.store.prev_hunk((&path, if whole { 0 } else { row })),
            Nav::NextFile => {
                let locations = self.store.locations();
                locations
                    .iter()
                    .find(|location| location.path > path)
                    .or_else(|| locations.iter().find(|location| location.path != path))
                    .or_else(|| locations.first())
                    .cloned()
            }
        };
        let Some(location) = location else {
            crate::toast::info_keyed("review", "No hay cambios pendientes", cx);
            return;
        };
        // Moving onto a file reviewed as a whole lands on the file itself.
        let location = match self.store.file(&location.path) {
            Some(file) if self.is_file_level(file) => ReviewLocation {
                hunk: None,
                row: 0,
                ..location
            },
            _ => location,
        };
        self.go_to(location, window, cx);
    }

    /// Opens the file of `path` on its first pending change (the panel, the
    /// chat's "Ver en el editor").
    pub fn open_file_at_first_hunk(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush(None, cx);
        let location = self
            .store
            .locations()
            .into_iter()
            .find(|location| location.path == path);
        match location {
            Some(location) => self.go_to(location, window, cx),
            None => {
                if let Some(center) = self.center.upgrade() {
                    center.update(cx, |center, cx| center.open_file(path, true, window, cx));
                }
            }
        }
        self.set_panel_open(false, cx);
    }

    fn go_to(&mut self, location: ReviewLocation, window: &mut Window, cx: &mut Context<Self>) {
        let Some(center) = self.center.upgrade() else {
            return;
        };
        let path = location.path.clone();
        center.update(cx, |center, cx| center.open_file(&path, true, window, cx));
        let Some((editor, deleted_review)) = center
            .read(cx)
            .tabs()
            .iter()
            .find(|tab| tab.path == path)
            .map(|tab| (tab.editor().clone(), tab.deleted_review))
        else {
            return;
        };
        let whole = self
            .store
            .file(&path)
            .is_some_and(|file| self.is_file_level(file));
        match location.hunk {
            Some(hunk) if !whole => {
                // A change of `current_index` is what moves the editor, so
                // an unchanged one is cleared first.
                self.current.remove(&path);
                let view = self.view_for(&path);
                editor.update(cx, |editor, cx| editor.set_review(view, cx));
                self.current.insert(path.clone(), hunk);
                let view = self.view_for(&path);
                editor.update(cx, |editor, cx| editor.set_review(view, cx));
            }
            _ => {
                self.current.remove(&path);
                let view = self.view_for_tab(&path, deleted_review);
                editor.update(cx, |editor, cx| {
                    editor.set_review(view, cx);
                    editor.set_cursor(Point::new(location.row, 0), cx);
                });
            }
        }
        self.refresh(None, cx);
    }

    // ---------------------------------------------------------------- panel

    /// Opens or closes the review panel.
    pub fn set_panel_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.panel_open != open {
            self.panel_open = open;
            self.refresh(None, cx);
        }
    }

    /// `Ctrl+Shift+R` and the status bar counter.
    pub fn toggle_panel(&mut self, cx: &mut Context<Self>) {
        self.set_panel_open(!self.panel_open, cx);
    }

    /// The rows of the review panel: every file with pending changes, plus
    /// the ones decided since everything was last clear.
    pub fn panel_rows(&self) -> Vec<PanelRow> {
        let root = self.summary.borrow().root.clone();
        let relative = |path: &Path| {
            root.as_ref()
                .and_then(|root| path.strip_prefix(root).ok())
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/")
        };
        let mut paths: BTreeSet<PathBuf> = self.cycle.clone();
        paths.extend(self.store.pending_files());
        paths.extend(self.uncopied.keys().cloned());
        paths
            .into_iter()
            .map(|path| {
                if let Some(change) = self.uncopied.get(&path) {
                    return PanelRow {
                        relative: relative(&path),
                        added: 0,
                        removed: 0,
                        state: PanelState::Pending,
                        created: false,
                        deleted: change.state == FileState::Deleted,
                        too_large: false,
                        no_copy: true,
                        path,
                    };
                }
                let pending = self
                    .store
                    .file(&path)
                    .filter(|file| file.has_pending_work());
                let (added, removed) = pending.map(FileReview::stats).unwrap_or((0, 0));
                PanelRow {
                    relative: relative(&path),
                    added,
                    removed,
                    state: if pending.is_some() {
                        PanelState::Pending
                    } else {
                        PanelState::Decided
                    },
                    created: pending
                        .is_some_and(|file| matches!(file.status, FileStatus::Created { .. })),
                    deleted: pending
                        .is_some_and(|file| matches!(file.status, FileStatus::Deleted { .. })),
                    too_large: pending.is_some_and(|file| {
                        !matches!(file.status, FileStatus::Deleted { .. })
                            && self.is_file_level(file)
                    }),
                    no_copy: false,
                    path,
                }
            })
            .collect()
    }

    // --------------------------------------------------------------- center

    fn on_center_event(
        &mut self,
        event: &CenterEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            CenterEvent::Review {
                path,
                action,
                origin,
            } => self.handle_action(path, *action, *origin, window, cx),
            CenterEvent::TabOpened { path } => {
                if self.store.file(path).is_some() {
                    // The tab may have opened a new buffer instance.
                    self.ensure_buffer(path, cx);
                }
                self.refresh(Some(path), cx);
            }
            CenterEvent::Saved { .. } => self.schedule_persist(cx),
            CenterEvent::DiscardTracked { path } => {
                self.discard_buffer(path, cx);
                self.refresh(None, cx);
            }
            CenterEvent::DirtyAnswer(choice) => self.answer_dirty(*choice, cx),
        }
    }

    // --------------------------------------------------------------- views

    /// Whether `file` is reviewed only as a whole: the store's own limits
    /// (2 MB / 50 000 lines, a NUL in the text, deleted) or the stricter
    /// `review.max_file_size_kb` / `review.max_lines`.
    fn is_file_level(&self, file: &FileReview) -> bool {
        file.file_level_only()
            || self.limits.exceeded_by(&file.base)
            || self.limits.exceeded_by(file.current().rope())
    }

    /// Pending decisions `file` counts for: its hunks, or one for a file
    /// reviewed as a whole.
    fn decisions(&self, file: &FileReview) -> usize {
        if !file.has_pending_work() {
            0
        } else if self.is_file_level(file) {
            1
        } else {
            file.pending_hunks().count().max(1)
        }
    }

    /// Pending decisions over the whole review (what the status bar counts).
    pub fn pending_count(&self) -> usize {
        self.store
            .files()
            .map(|file| self.decisions(file))
            .sum::<usize>()
            + self.uncopied.len()
    }

    /// The [`ReviewView`] of `path`, as the editor paints it.
    pub fn view_for(&mut self, path: &Path) -> ReviewView {
        let total = self.pending_count();
        let Some(file) = self.store.file(path) else {
            self.pushed.remove(path);
            return ReviewView {
                pending_in_other_files: total,
                ..ReviewView::default()
            };
        };
        let here = self.decisions(file);
        // Both the host and the store have to say the turn runs: the store
        // alone keeps whatever turns it was told about, and a view built
        // after `end_turn` must never come out disabled.
        let turn_active = self.active_turn.is_some() && self.store.turn_active(path);
        let whole = self.is_file_level(file);
        let previous = self.latest_turn.is_some_and(|turn| turn != file.turn_id);
        let built = if whole || !file.has_pending_work() {
            Vec::new()
        } else {
            hunk_views(file, previous)
        };
        let (hunks, pairs): (Vec<ReviewHunkView>, Vec<(u64, Vec<LinePair>)>) = built
            .into_iter()
            .map(|(view, pairs)| {
                let id = view.id;
                (view, (id, pairs))
            })
            .unzip();
        let current_index = self
            .current
            .get(path)
            .and_then(|id| hunks.iter().position(|hunk| hunk.id == id.0));
        self.pushed.insert(path.to_path_buf(), pairs);
        ReviewView {
            pending_in_file: if whole { here } else { hunks.len() },
            pending_in_other_files: total.saturating_sub(here),
            hunks,
            turn_active,
            current_index,
        }
    }

    /// The view of the tab of `path`: the tab of a file the agent deleted
    /// (`deleted_review`, an empty read-only buffer) shows the whole previous
    /// content as one hunk of phantom rows; any other tab gets
    /// [`Self::view_for`].
    pub fn view_for_tab(&mut self, path: &Path, deleted_review: bool) -> ReviewView {
        if deleted_review && let Some(view) = self.deleted_view(path) {
            return view;
        }
        self.view_for(path)
    }

    /// The single "archivo borrado" hunk of a pending deletion: every line
    /// the file had, red and read-only. Its hover buttons decide the whole
    /// file ([`DELETED_FILE_HUNK`] maps to the file decisions in
    /// [`Self::handle_action`]). `None` when `path` is not a pending
    /// deletion.
    fn deleted_view(&mut self, path: &Path) -> Option<ReviewView> {
        let (lines, turn, turn_active) = if let Some(file) = self.store.file(path) {
            let FileStatus::Deleted { previous } = &file.status else {
                return None;
            };
            let lines = rope_lines(previous);
            let turn_active = self.active_turn.is_some() && self.store.turn_active(path);
            (lines, file.turn_id, turn_active)
        } else {
            let change = self
                .uncopied
                .get(path)
                .filter(|change| change.state == FileState::Deleted)?;
            let turn_active = self.active_turn == Some(change.turn);
            (
                vec![DELETED_NO_COPY_LINE.to_string()],
                change.turn,
                turn_active,
            )
        };
        self.pushed.remove(path);
        let total = self.pending_count();
        let rows = lines.len() as u32;
        Some(ReviewView {
            hunks: vec![ReviewHunkView {
                id: DELETED_FILE_HUNK,
                base_rows: 0..rows,
                deleted_lines: lines,
                buffer_rows: 0..0,
                kind: ReviewHunkKind::Deleted,
                word_diffs: None,
                lines: Vec::new(),
                from_previous_turn: self.latest_turn.is_some_and(|latest| latest != turn),
            }],
            turn_active,
            pending_in_file: 1,
            pending_in_other_files: total.saturating_sub(1),
            current_index: Some(0),
        })
    }

    /// Whether `path` is a deletion still waiting for a decision (with or
    /// without a copy).
    pub fn is_pending_deletion(&self, path: &Path) -> bool {
        self.store
            .file(path)
            .is_some_and(|file| matches!(file.status, FileStatus::Deleted { .. }))
            || self
                .uncopied
                .get(path)
                .is_some_and(|change| change.state == FileState::Deleted)
    }

    /// Rebuilds the summary and pushes fresh views to the editors: of `path`
    /// only, or of every tab.
    ///
    /// A tab whose kind no longer matches the review (a file the agent
    /// deleted shown as a normal editor, or the read-only tab of a deletion
    /// that was just decided) is swapped once this update is over
    /// ([`CenterPanel::sync_deleted_reviews`], which needs the window).
    fn refresh(&mut self, path: Option<&Path>, cx: &mut Context<Self>) {
        self.refresh_summary(cx);
        if let Some(center) = self.center.upgrade() {
            let editors: Vec<(PathBuf, Entity<EditorView>, bool)> = center
                .read(cx)
                .tabs()
                .iter()
                .filter(|tab| path.is_none_or(|path| tab.path == path))
                .map(|tab| (tab.path.clone(), tab.editor().clone(), tab.deleted_review))
                .collect();
            for (path, editor, deleted_review) in editors {
                let view = self.view_for_tab(&path, deleted_review);
                editor.update(cx, |editor, cx| editor.set_review(view, cx));
            }
            center.update(cx, |_, cx| cx.notify());
            if center.read(cx).deleted_reviews_out_of_sync(cx) {
                self.sync_center_later(cx);
            }
        }
        self.schedule_recompute(cx);
        cx.notify();
    }

    /// Runs [`CenterPanel::sync_deleted_reviews`] once the current update is
    /// over (swapping a tab moves the focus, which needs the window).
    fn sync_center_later(&self, cx: &mut Context<Self>) {
        let (Some(window), Some(center)) = (self.window, self.center.upgrade()) else {
            return;
        };
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                center.update(cx, |center, cx| center.sync_deleted_reviews(window, cx))
            });
        });
    }

    fn refresh_summary(&mut self, cx: &App) {
        let pending_files = self.store.pending_files();
        self.cycle.extend(pending_files.iter().cloned());
        self.cycle.extend(self.uncopied.keys().cloned());
        let mut files = BTreeMap::new();
        let mut total = (0, 0);
        for path in &pending_files {
            let Some(file) = self.store.file(path) else {
                continue;
            };
            let (added, removed) = file.stats();
            total.0 += added;
            total.1 += removed;
            let whole = self.is_file_level(file);
            files.insert(
                path.clone(),
                FileSummary {
                    added,
                    removed,
                    state: match file.status {
                        FileStatus::Modified => FileState::Modified,
                        FileStatus::Created { .. } => FileState::Created,
                        FileStatus::Deleted { .. } => FileState::Deleted,
                    },
                    file_level: whole,
                    hunks: if whole {
                        0
                    } else {
                        file.pending_hunks().count()
                    },
                    no_copy: false,
                },
            );
        }
        for (path, change) in &self.uncopied {
            files.insert(
                path.clone(),
                FileSummary {
                    added: 0,
                    removed: 0,
                    state: change.state,
                    file_level: true,
                    hunks: 0,
                    no_copy: true,
                },
            );
        }
        let mut summary = self.summary.borrow_mut();
        summary.root = self
            .project
            .as_ref()
            .map(|project| project.read(cx).root().to_path_buf());
        summary.files = files;
        summary.tracked = self.store.files().map(|file| file.path.clone()).collect();
        summary.tracked.extend(self.buffers.keys().cloned());
        summary.pending = self.pending_count();
        summary.total = total;
        summary.turn_active = self.active_turn.is_some();
        summary.sweeping = self.sweep_notice;
        summary.panel_open = self.panel_open;
        summary.dirty_prompt = self
            .prompts
            .front()
            .map(|prompt| SharedString::from(file_name(&prompt.path)));
    }
}

/// The editor's hunks of `file`, with the line pairs behind each one.
pub fn hunk_views(file: &FileReview, previous_turn: bool) -> Vec<(ReviewHunkView, Vec<LinePair>)> {
    let snapshot = file.current();
    file.pending_hunks()
        .map(|hunk| {
            let buffer_rows = file.buffer_rows(hunk);
            let deleted_lines = (hunk.base_rows.start..hunk.base_rows.end)
                .map(|row| line_text(&file.base, row))
                .collect();
            let lines = hunk
                .lines
                .iter()
                .map(|pair| ReviewLineView {
                    base_line: pair
                        .base_row
                        .map(|row| row.saturating_sub(hunk.base_rows.start)),
                    buffer_row: pair
                        .buffer_row
                        .map(|anchor| snapshot.offset_to_point(anchor.offset).row),
                })
                .collect();
            let word_diffs = hunk.word_diffs.as_ref().map(|words| ReviewWordDiffs {
                deleted: words.base.clone(),
                added: words.buffer.clone(),
            });
            let kind = match hunk.kind {
                HunkKind::Added => ReviewHunkKind::Added,
                HunkKind::Deleted => ReviewHunkKind::Deleted,
                HunkKind::Modified => ReviewHunkKind::Modified,
            };
            (
                ReviewHunkView {
                    id: hunk.id.0,
                    base_rows: hunk.base_rows.clone(),
                    deleted_lines,
                    buffer_rows,
                    kind,
                    word_diffs,
                    lines,
                    from_previous_turn: previous_turn,
                },
                hunk.lines.clone(),
            )
        })
        .collect()
}

/// Line `row` of `rope`, without its newline.
fn line_text(rope: &Rope, row: u32) -> String {
    let row = row as usize;
    if row >= rope.len_lines() {
        return String::new();
    }
    let line = rope.line(row).to_string();
    match line.strip_suffix('\n') {
        Some(stripped) => stripped.to_owned(),
        None => line,
    }
}

/// Every line of `rope`, without newlines; the empty row after a trailing
/// newline is not a line (the count the tree's `−N` shows).
fn rope_lines(rope: &Rope) -> Vec<String> {
    rope.to_string()
        .split_inclusive('\n')
        .map(|line| line.strip_suffix('\n').unwrap_or(line).to_owned())
        .collect()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// Removes what an earlier build saved for the binary files in `dir`
/// ([`LEGACY_BINARY_STATE`]): nothing reads it any more.
fn remove_legacy_binary_state(dir: &Path) {
    for name in LEGACY_BINARY_STATE {
        let path = dir.join(name);
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else if path.exists() {
            std::fs::remove_file(&path)
        } else {
            continue;
        };
        match removed {
            Ok(()) => {
                tracing::info!(path = %path.display(), "estado viejo de binarios borrado")
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "no se pudo borrar el estado viejo de binarios")
            }
        }
    }
}

/// The toast of a persisted entry that could not be restored
/// (`02-visual.md` §6.5).
fn dropped_message(path: &Path, reason: DropReason) -> String {
    let _ = reason;
    format!(
        "La revisión de «{}» se descartó: el archivo cambió fuera de Cincel",
        file_name(path)
    )
}
