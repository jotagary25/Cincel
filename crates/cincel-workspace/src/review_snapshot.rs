//! The photo of the project behind the review
//! (`docs/specs/03-arquitectura.md` §4, v2: "la base viene de la foto del
//! proyecto; las herramientas son solo una pista").
//!
//! A child module of `crate::review` (it works on [`Review`]'s own fields):
//!
//! 1. **Photo.** [`Review::start_photo`], from `begin_prompt`, takes a
//!    [`ProjectSnapshot`] on the background executor (on the calling thread
//!    for an inert project, the tests); the prompt waits for
//!    [`Review::turn_waiter`]. Hash-only entries whose buffer is open take
//!    the buffer's text as their copy.
//! 2. **Watch.** [`Review::on_files_changed`] gets every watcher batch of the
//!    project ([`crate::project::FilesChanged`]) and adopts each path:
//!    [`Review::adopt_change`] captures the photo's copy as the base of a
//!    path not in review (`file_created` if it was not there) and re-reads
//!    the disk (`file_written` / `file_deleted`). A rename is a deletion
//!    plus a creation. A buffer with unsaved changes keeps them, as the
//!    user's (they join the base). A burst (200 agent writes in one batch)
//!    is adopted in slices of at most [`WATCH_SLICE`] ([`WatchQueue`]); the
//!    sweep adopts whatever is still queued before its own paths.
//! 3. **Sweep.** [`Review::finish_turn`], from `end_turn` (and `agent_gone`,
//!    and `begin_prompt` when turns chain), compares the whole photo with the
//!    disk and adopts whatever the watcher did not report
//!    (`docs/specs/08-etapa6-cierre-1-0.md` §5.3, D12). The comparison
//!    ([`ProjectSnapshot::changes`]) runs on the background executor and the
//!    turn stays active ([`PhotoState::Sweeping`]) until it lands; then the
//!    files that differ are adopted on the main thread in slices of at most
//!    [`SWEEP_SLICE`], so the window never freezes. A prompt sent meanwhile
//!    waits ([`Review::turn_waiter`]); past [`SWEEP_NOTICE_AFTER`] the status
//!    bar says [`SWEEP_NOTICE`]. An inert project (the tests) sweeps inline
//!    unless `set_photo_in_background(true)`.
//! 4. **Attribution.** [`Review::on_host_wrote`]: whatever Cincel writes
//!    during the turn refreshes the photo, so it is never the agent's (during
//!    the sweep the photo is shared with the background thread, so those
//!    paths are set aside instead). Changes by other programs are the
//!    agent's.
//! 5. **Binaries.** Binary files ([`cincel_project::looks_binary`]: not
//!    UTF-8, or a NUL in the first 8 KB) are out of the review: whatever the
//!    agent does to them (create, change, delete, or turn a text file into
//!    one) is applied without asking and never shows as pending. The photo
//!    keeps only their size and hash, to tell them apart.
//! 6. **Text without a copy.** A text file the photo kept no copy of (over
//!    `review.max_file_size_kb` or `review.snapshot_max_total_mb`) cannot be
//!    diffed nor restored: it is kept as an [`UncopiedChange`], whole file,
//!    "cambiado por el agente, sin copia previa", and can only be accepted
//!    (memory only: it is not saved with the review).

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cincel_project::{
    FsEvent, ProjectSnapshot, SnapshotChange, SnapshotContent, SnapshotLimits, looks_binary,
};
use cincel_review::TurnId;
use cincel_text::EditSource;
use gpui::{Context, Task};

use super::{FileState, Review};

/// The longest stretch the adoption of the sweep's results keeps the main
/// thread busy before yielding. The spec's budget is 8 ms of main thread in
/// one go, and GPUI paints right after an update that asked for a frame
/// (3–5 ms with a large file open): a slice leaves room for that frame
/// (E6-G measured 4 ms slices plus a frame at 8.2 ms).
pub const SWEEP_SLICE: Duration = Duration::from_millis(3);
/// How long a sweep runs before the status bar says [`SWEEP_NOTICE`].
pub const SWEEP_NOTICE_AFTER: Duration = Duration::from_secs(1);
/// The status bar while a long sweep runs.
pub const SWEEP_NOTICE: &str = "Revisando los cambios del agente…";

/// The longest stretch the adoption of one watcher batch keeps the main
/// thread busy before yielding (`docs/specs/08-etapa6-cierre-1-0.md` §3.8,
/// E6-G: a burst of 200 agent writes used to be adopted in one 30–40 ms
/// stretch).
pub const WATCH_SLICE: Duration = SWEEP_SLICE;
/// A watcher batch naming at most this many paths is adopted inside its own
/// handler; a bigger one is queued whole and adopted in slices.
pub const WATCH_INLINE_PATHS: usize = 4;

/// Somebody waiting for the turn to be ready (a prompt held back).
pub(super) type Waiter = tokio::sync::oneshot::Sender<()>;

/// The watcher's paths of the running turn still to adopt: a batch of more
/// than [`WATCH_INLINE_PATHS`] is queued here and adopted on the main thread
/// in slices of at most [`WATCH_SLICE`] (an inert project, the tests, adopts
/// it all inline unless `set_photo_in_background(true)`).
#[derive(Debug)]
pub(super) struct WatchQueue {
    /// Paths of a burst's events, not yet expanded (a directory stands for
    /// its files): the slices expand them.
    touched: VecDeque<PathBuf>,
    /// Expanded paths, in the order the batches named them.
    paths: VecDeque<PathBuf>,
    /// The same paths, to skip one a later batch names again.
    queued: BTreeSet<PathBuf>,
    /// A slice is already spawned.
    scheduled: bool,
    /// Something was adopted since the last refresh of the views.
    adopted: bool,
    /// Longest slice before yielding ([`WATCH_SLICE`]; a test shrinks it).
    budget: Duration,
    /// Timings, for [`Review::last_watch`].
    stats: WatchStats,
}

impl Default for WatchQueue {
    fn default() -> Self {
        Self {
            touched: VecDeque::new(),
            paths: VecDeque::new(),
            queued: BTreeSet::new(),
            scheduled: false,
            adopted: false,
            budget: WATCH_SLICE,
            stats: WatchStats::default(),
        }
    }
}

impl WatchQueue {
    fn push(&mut self, path: PathBuf) {
        if self.queued.insert(path.clone()) {
            self.paths.push_back(path);
            self.stats.paths += 1;
        }
    }

    fn pop(&mut self) -> Option<PathBuf> {
        let path = self.paths.pop_front()?;
        self.queued.remove(&path);
        Some(path)
    }

    /// Takes `path` out of the queue; whether it was there.
    fn take(&mut self, path: &Path) -> bool {
        if !self.queued.remove(path) {
            return false;
        }
        self.paths.retain(|queued| queued != path);
        true
    }

    /// The turn is over: nothing left to adopt (the timings stay).
    pub(super) fn clear(&mut self) {
        self.touched.clear();
        self.paths.clear();
        self.queued.clear();
        self.adopted = false;
    }

    fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.touched.is_empty()
    }
}

/// How the adoption of the watcher's batches went since the turn started
/// (`--bench sweep`, the tests).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WatchStats {
    /// Paths the watcher's batches named.
    pub paths: usize,
    /// Paths that entered (or were updated in) the review.
    pub adopted: usize,
    /// The longest single stretch the adoption kept the main thread busy.
    pub max_main_slice: Duration,
    /// How many main-thread stretches it took.
    pub main_slices: usize,
}

/// Where the photo of the running turn is.
pub(super) enum PhotoState {
    /// Being taken on the background executor; `waiters` are told when it
    /// lands.
    Capturing {
        turn: TurnId,
        waiters: Vec<Waiter>,
        /// What Cincel wrote while the photo was being taken: the photo may
        /// have read it before or after, so it is re-read when it lands.
        host_writes: Vec<PathBuf>,
        _task: Task<()>,
    },
    /// Ready: the project as it was before the prompt.
    Ready {
        turn: TurnId,
        snapshot: Arc<ProjectSnapshot>,
    },
    /// The turn ended and its photo is being compared with the disk; the
    /// turn stays active until this is over.
    Sweeping {
        turn: TurnId,
        snapshot: Arc<ProjectSnapshot>,
        /// Prompts held back until the turn is over (and, for a chained
        /// turn, until its own photo is ready).
        waiters: Vec<Waiter>,
        /// What Cincel wrote during the sweep: never the agent's.
        host_writes: Vec<PathBuf>,
        /// The turn a prompt sent during the sweep starts once it is over.
        chained: Option<TurnId>,
        /// The paths the sweep found, still to adopt (`None` until the
        /// background comparison lands).
        pending: Option<VecDeque<PathBuf>>,
        /// Paths adopted so far.
        adopted: usize,
        /// Timings, for [`Review::last_sweep`].
        stats: SweepStats,
        /// When the sweep started.
        started: Instant,
        _task: Task<()>,
    },
}

/// How the last end-of-turn sweep went (`--bench sweep`, the tests).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SweepStats {
    /// Files in the photo.
    pub files: usize,
    /// Differences the comparison found (before setting aside Cincel's own
    /// writes).
    pub changes: usize,
    /// Files that entered (or were updated in) the review.
    pub adopted: usize,
    /// The comparison with the disk (on the background executor, or inline).
    pub compare: Duration,
    /// The longest single stretch the sweep kept the main thread busy.
    pub max_main_slice: Duration,
    /// How many main-thread stretches it took.
    pub main_slices: usize,
    /// From the end of the turn to its close.
    pub total: Duration,
    /// Whether the comparison ran on the background executor.
    pub in_background: bool,
}

impl SweepStats {
    fn main_slice(&mut self, elapsed: Duration) {
        self.max_main_slice = self.max_main_slice.max(elapsed);
        self.main_slices += 1;
    }
}

/// A text file the agent changed or deleted whose previous content the photo
/// did not copy: reviewed outside the store, as a whole, accept only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct UncopiedChange {
    /// The turn that changed it.
    pub(super) turn: TurnId,
    /// Modified or deleted.
    pub(super) state: FileState,
}

/// Lets go of a photo that is no longer needed. The last reference to a
/// photo can hold hundreds of megabytes of copies (271 MB for the 20 000
/// files of the bench's large project): freeing them took 5 ms of the main
/// thread, so it happens on the background executor.
pub(super) fn release(photo: Option<PhotoState>, cx: &mut Context<Review>) {
    match photo {
        Some(PhotoState::Ready { snapshot, .. } | PhotoState::Sweeping { snapshot, .. }) => {
            release_snapshot(snapshot, cx);
        }
        Some(PhotoState::Capturing { .. }) | None => {}
    }
}

fn release_snapshot(snapshot: Arc<ProjectSnapshot>, cx: &mut Context<Review>) {
    cx.background_executor()
        .spawn(async move { drop(snapshot) })
        .detach();
}

/// Text from disk the way a buffer holds it: no BOM, `\n` line endings.
pub(super) fn normalize(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_owned()
    }
}

impl Review {
    // ---------------------------------------------------------------- photo

    /// Whether the photo and the sweep run right here instead of on the
    /// background executor: a project without background pieces (the
    /// tests), unless a test asked for the real thing.
    fn runs_inline(&self, cx: &Context<Self>) -> bool {
        let inert = self
            .project
            .as_ref()
            .is_some_and(|project| !project.read(cx).options().watch_files);
        inert && !self.photo_in_background
    }

    /// Starts the photo of `turn`: on the background executor, or right here
    /// for a project without background pieces (the tests). `waiters` (the
    /// prompt of a turn chained after a sweep) are told when it is ready.
    pub(super) fn start_photo(
        &mut self,
        turn: TurnId,
        waiters: Vec<Waiter>,
        cx: &mut Context<Self>,
    ) {
        release(self.photo.take(), cx);
        // A new turn: its own watcher timings.
        self.watch.clear();
        self.watch.stats = WatchStats::default();
        let Some(project) = self.project.clone() else {
            return;
        };
        let source = project.read(cx).worktree().snapshot_source();
        let settings = crate::settings::settings(cx);
        let limits = SnapshotLimits::from_settings(
            settings.review.max_file_size_kb,
            settings.review.snapshot_max_total_mb,
        );
        if self.runs_inline(cx) {
            let snapshot = source.capture(limits);
            self.photo = Some(PhotoState::Capturing {
                turn,
                waiters,
                host_writes: Vec::new(),
                _task: Task::ready(()),
            });
            self.photo_arrived(turn, snapshot, cx);
            return;
        }
        let capture = cx
            .background_executor()
            .spawn(async move { source.capture(limits) });
        let task = cx.spawn(async move |this, cx| {
            let snapshot = capture.await;
            let _ = this.update(cx, |review, cx| review.photo_arrived(turn, snapshot, cx));
        });
        self.photo = Some(PhotoState::Capturing {
            turn,
            waiters,
            host_writes: Vec::new(),
            _task: task,
        });
    }

    /// The photo of `turn` is ready: keep it (if the turn is still the one
    /// running) and release the prompt.
    fn photo_arrived(
        &mut self,
        turn: TurnId,
        mut snapshot: ProjectSnapshot,
        cx: &mut Context<Self>,
    ) {
        let waiting = matches!(
            &self.photo,
            Some(PhotoState::Capturing { turn: capturing, .. }) if *capturing == turn
        );
        if !waiting || self.active_turn != Some(turn) {
            return;
        }
        if let Some(PhotoState::Capturing { host_writes, .. }) = &mut self.photo {
            for path in host_writes.drain(..) {
                snapshot.refresh(&path);
            }
        }
        // Files the photo kept no copy of take the open buffer's text: the
        // spec's "la base se toma del buffer si está abierto".
        if let Some(project) = &self.project {
            let project = project.read(cx);
            let buffers = project.buffers();
            let open: Vec<PathBuf> = buffers.paths().map(Path::to_path_buf).collect();
            for path in open {
                let hash_only = snapshot
                    .entry(&path)
                    .is_some_and(|entry| entry.content == SnapshotContent::HashOnly);
                if hash_only
                    && !buffers.is_dirty(&path)
                    && let Some(handle) = buffers.get(&path)
                {
                    let text = handle.lock().snapshot().text();
                    snapshot.set_text(&path, Arc::from(text));
                }
            }
        }
        tracing::info!(
            files = snapshot.len(),
            copied = snapshot.copied_bytes(),
            over_total = snapshot.is_over_total(),
            elapsed = ?snapshot.elapsed(),
            "foto del proyecto lista para el turno"
        );
        if snapshot.is_over_total() {
            tracing::warn!(
                "el proyecto supera review.snapshot_max_total_mb: algunos archivos solo guardan su huella"
            );
        }
        let previous = self.photo.replace(PhotoState::Ready {
            turn,
            snapshot: Arc::new(snapshot),
        });
        if let Some(PhotoState::Capturing { waiters, .. }) = previous {
            for waiter in waiters {
                let _ = waiter.send(());
            }
        }
    }

    /// While the photo of the running turn is being taken, or the previous
    /// turn is still being swept, a receiver that resolves when the prompt
    /// can go out (or the wait is abandoned: a dropped sender); `None` when
    /// it can go out now.
    pub fn turn_waiter(&mut self) -> Option<tokio::sync::oneshot::Receiver<()>> {
        match &mut self.photo {
            Some(PhotoState::Capturing { waiters, .. } | PhotoState::Sweeping { waiters, .. }) => {
                let (sender, receiver) = tokio::sync::oneshot::channel();
                waiters.push(sender);
                Some(receiver)
            }
            _ => None,
        }
    }

    /// Takes the photo, and runs the end-of-turn sweep, on the background
    /// executor even for an inert project, so a test can see a prompt wait
    /// for them.
    #[cfg(test)]
    pub(crate) fn set_photo_in_background(&mut self, on: bool) {
        self.photo_in_background = on;
    }

    /// Whether the end-of-turn sweep is running (the turn is still active).
    pub fn is_sweeping(&self) -> bool {
        matches!(self.photo, Some(PhotoState::Sweeping { .. }))
    }

    /// How the last end-of-turn sweep went.
    pub fn last_sweep(&self) -> Option<SweepStats> {
        self.last_sweep
    }

    /// Whether the running turn has its photo.
    pub fn is_photo_ready(&self) -> bool {
        self.ready_snapshot().is_some()
    }

    /// The photo of the running turn, when it is ready.
    pub fn photo(&self) -> Option<&ProjectSnapshot> {
        self.ready_snapshot()
    }

    /// The photo of the running turn: ready, or being swept (the watcher's
    /// batches are still adopted against it).
    pub(super) fn ready_snapshot(&self) -> Option<&ProjectSnapshot> {
        match &self.photo {
            Some(
                PhotoState::Ready { turn, snapshot } | PhotoState::Sweeping { turn, snapshot, .. },
            ) if self.active_turn == Some(*turn) => Some(snapshot),
            _ => None,
        }
    }

    /// Whether Cincel itself wrote `path` while the sweep runs.
    fn host_wrote_during_sweep(&self, path: &Path) -> bool {
        matches!(
            &self.photo,
            Some(PhotoState::Sweeping { host_writes, .. })
                if host_writes.iter().any(|written| written == path)
        )
    }

    /// The photo's text copy of `path`.
    pub(super) fn photo_text(&self, path: &Path) -> Option<Arc<str>> {
        self.ready_snapshot()?.entry(path)?.text().cloned()
    }

    /// Cincel wrote or deleted `path` by itself: during a turn, the photo
    /// takes it, so it is never the agent's. A watcher change of the path
    /// still queued is adopted first, as it would have been had its slice
    /// come before the write.
    pub(super) fn on_host_wrote(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.adopt_queued_now(path, cx);
        match &mut self.photo {
            // Nobody else holds the photo while it is ready: no copy.
            Some(PhotoState::Ready { snapshot, .. }) => Arc::make_mut(snapshot).refresh(path),
            Some(PhotoState::Capturing { host_writes, .. }) => {
                host_writes.push(path.to_path_buf());
            }
            // The background comparison shares the photo: set the path
            // aside instead (it leaves the sweep's list and the watcher's
            // batches until the turn closes).
            Some(PhotoState::Sweeping {
                host_writes,
                pending,
                ..
            }) => {
                host_writes.push(path.to_path_buf());
                if let Some(pending) = pending {
                    pending.retain(|queued| queued != path);
                }
            }
            None => {}
        }
    }

    // ---------------------------------------------------------------- watch

    /// A batch of the watcher: during a turn, every path it names is the
    /// agent's to review (directories expand to their files).
    pub(super) fn on_files_changed(&mut self, events: &[FsEvent], cx: &mut Context<Self>) {
        if self.active_turn.is_none() || self.ready_snapshot().is_none() {
            return;
        }
        let _review_watch =
            tracing::info_span!(target: "cincel::timing", "review_watch_batch").entered();
        let touched: Vec<PathBuf> = events
            .iter()
            .flat_map(|event| match event {
                FsEvent::Renamed { from, to } => vec![from.clone(), to.clone()],
                other => vec![other.path().to_path_buf()],
            })
            .collect();
        let inline = self.runs_inline(cx);
        if !inline && (touched.len() > WATCH_INLINE_PATHS || !self.watch.is_empty()) {
            // A burst goes to the queue as it came, not even looked at: this
            // handler runs right after the project's own share of the batch
            // (reloading the open buffers and their tabs, 3–4 ms for 200
            // files with a 5 000-line file open) and GPUI paints the frame
            // that batch asked for right after it, so the paths are
            // expanded and adopted in the slices. Paths still queued from an
            // earlier batch go first.
            let slice = Instant::now();
            self.watch.touched.extend(touched);
            self.end_watch_slice(slice);
            self.schedule_watch_step(cx);
            return;
        }
        // A small batch (a save, a single agent edit), or an inert project
        // (the tests): adopted right here, as before.
        let slice = Instant::now();
        let mut any = false;
        for path in self.expand_touched(&touched) {
            self.watch.stats.paths += 1;
            if self.adopt_change(&path, cx) {
                any = true;
                self.watch.stats.adopted += 1;
            }
        }
        if any {
            self.after_watch_adoption(cx);
        }
        if !inline {
            self.end_watch_slice(slice);
        }
    }

    /// The paths a watcher event about `touched` asks to adopt: the path
    /// itself, and for a directory every file below it (on disk, in the
    /// photo and, for a removal, in review).
    fn expand_touched(&self, touched: &[PathBuf]) -> BTreeSet<PathBuf> {
        let mut paths = BTreeSet::new();
        let Some(snapshot) = self.ready_snapshot() else {
            return paths;
        };
        for path in touched {
            if path.is_dir() {
                paths.extend(snapshot.files_on_disk_under(path));
                paths.extend(snapshot.paths_under(path));
            } else if !path.exists() {
                // A file, or a whole directory, went away.
                paths.insert(path.clone());
                paths.extend(snapshot.paths_under(path));
                paths.extend(
                    self.store
                        .files()
                        .map(|file| file.path.clone())
                        .chain(self.uncopied.keys().cloned())
                        .filter(|tracked| tracked.starts_with(path) && tracked != path),
                );
            } else {
                paths.insert(path.clone());
            }
        }
        paths
    }

    /// The next queued watcher path to adopt, expanding the next raw one
    /// when the expanded ones run out.
    fn next_watch_path(&mut self) -> Option<PathBuf> {
        loop {
            if let Some(path) = self.watch.pop() {
                return Some(path);
            }
            let touched = self.watch.touched.pop_front()?;
            for path in self.expand_touched(&[touched]) {
                self.watch.push(path);
            }
        }
    }

    /// What adopting watcher paths leaves to do once: git, buffers, views.
    fn after_watch_adoption(&mut self, cx: &mut Context<Self>) {
        if let Some(project) = &self.project {
            project.read(cx).refresh_git();
        }
        self.prune(cx);
        self.refresh(None, cx);
    }

    /// Spawns the next slice of the watch queue, unless one is pending or
    /// there is nothing to do.
    fn schedule_watch_step(&mut self, cx: &mut Context<Self>) {
        if self.watch.scheduled || (self.watch.is_empty() && !self.watch.adopted) {
            return;
        }
        self.watch.scheduled = true;
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |review, cx| review.watch_step(Instant::now(), cx));
        })
        .detach();
    }

    /// Adopts queued watcher paths for at most the queue's budget, then
    /// yields to the main thread's other work and goes on; refreshes the
    /// views (in a stretch of its own when this one is well under way) when
    /// the queue is empty.
    fn watch_step(&mut self, slice: Instant, cx: &mut Context<Self>) {
        self.watch.scheduled = false;
        if self.active_turn.is_none() {
            self.watch.clear();
            return;
        }
        // Once the sweep's comparison landed, its own slices adopt what is
        // left here (first), so the two never run back to back.
        if matches!(
            &self.photo,
            Some(PhotoState::Sweeping {
                pending: Some(_),
                ..
            })
        ) {
            return;
        }
        let budget = self.watch.budget;
        while let Some(path) = self.next_watch_path() {
            if self.adopt_change(&path, cx) {
                self.watch.adopted = true;
                self.watch.stats.adopted += 1;
            }
            if slice.elapsed() >= budget {
                self.end_watch_slice(slice);
                self.schedule_watch_step(cx);
                return;
            }
        }
        if self.watch.adopted {
            if !budget.is_zero() && slice.elapsed() >= budget / 4 {
                self.end_watch_slice(slice);
                self.schedule_watch_step(cx);
                return;
            }
            self.watch.adopted = false;
            self.after_watch_adoption(cx);
        }
        self.end_watch_slice(slice);
    }

    fn end_watch_slice(&mut self, slice: Instant) {
        let stats = &mut self.watch.stats;
        stats.max_main_slice = stats.max_main_slice.max(slice.elapsed());
        stats.main_slices += 1;
    }

    /// Cincel just wrote `path`: a watcher change of it still queued was the
    /// agent's, so it is adopted now, against the photo as it was.
    fn adopt_queued_now(&mut self, path: &Path, cx: &mut Context<Self>) {
        // A raw event about the path, or about a directory above it, is
        // expanded first so the path can be found.
        let (above, rest): (Vec<PathBuf>, Vec<PathBuf>) = std::mem::take(&mut self.watch.touched)
            .into_iter()
            .partition(|touched| path.starts_with(touched));
        self.watch.touched = rest.into();
        for queued in self.expand_touched(&above) {
            self.watch.push(queued);
        }
        if self.watch.take(path) && self.adopt_change(path, cx) {
            self.watch.adopted = true;
            self.watch.stats.adopted += 1;
            self.schedule_watch_step(cx);
        }
    }

    /// How the adoption of the watcher's batches of the running (or last)
    /// turn went.
    pub fn last_watch(&self) -> WatchStats {
        self.watch.stats
    }

    /// Watcher paths still waiting for their slice.
    pub fn watch_queue_len(&self) -> usize {
        self.watch.paths.len() + self.watch.touched.len()
    }

    /// Adopts at most one watcher path per slice (a test of the slicing).
    #[cfg(test)]
    pub(crate) fn set_watch_slice_budget(&mut self, budget: Duration) {
        self.watch.budget = budget;
    }

    // ---------------------------------------------------------------- sweep

    /// Ends the running turn: the whole photo against the disk first
    /// (the end-of-turn sweep), adopting whatever the watcher did not
    /// report, then [`Review::close_turn`].
    ///
    /// Returns `true` when the turn is over on return (no photo to sweep,
    /// or an inline sweep). Otherwise the comparison runs on the background
    /// executor, the turn stays active, and it closes when the results are
    /// adopted; `chained` is the turn a prompt sent meanwhile starts right
    /// after (its prompt waits on [`Review::turn_waiter`]). Called during a
    /// sweep, it only records `chained`.
    pub(super) fn finish_turn(&mut self, chained: Option<TurnId>, cx: &mut Context<Self>) -> bool {
        if let Some(PhotoState::Sweeping {
            chained: waiting, ..
        }) = &mut self.photo
        {
            if chained.is_some() {
                *waiting = chained;
            }
            return false;
        }
        let Some(turn) = self.active_turn else {
            return true;
        };
        let snapshot = match &self.photo {
            Some(PhotoState::Ready {
                turn: ready,
                snapshot,
            }) if *ready == turn => Some(snapshot.clone()),
            _ => None,
        };
        let Some(snapshot) = snapshot else {
            // No photo to compare with (it was still being taken).
            self.close_turn(cx);
            return true;
        };
        let started = Instant::now();
        let mut stats = SweepStats {
            files: snapshot.len(),
            ..SweepStats::default()
        };

        if self.runs_inline(cx) {
            let changes = snapshot.changes();
            stats.compare = started.elapsed();
            stats.changes = changes.len();
            let mut adopted = 0;
            for change in &changes {
                if self.adopt_change(change.path(), cx) {
                    adopted += 1;
                }
            }
            stats.adopted = adopted;
            if adopted > 0
                && let Some(project) = &self.project
            {
                project.read(cx).refresh_git();
            }
            stats.main_slice(started.elapsed());
            stats.total = started.elapsed();
            self.last_sweep = Some(stats);
            self.close_turn(cx);
            return true;
        }

        stats.in_background = true;
        let shared = snapshot.clone();
        let compare = cx.background_executor().spawn(async move {
            let clock = Instant::now();
            let changes = shared.changes();
            (changes, clock.elapsed())
        });
        let task = cx.spawn(async move |this, cx| {
            let (changes, elapsed) = compare.await;
            let _ = this.update(cx, |review, cx| {
                review.sweep_arrived(turn, changes, elapsed, cx)
            });
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SWEEP_NOTICE_AFTER).await;
            let _ = this.update(cx, |review, cx| {
                let still = matches!(
                    &review.photo,
                    Some(PhotoState::Sweeping { turn: sweeping, .. }) if *sweeping == turn
                );
                if still && !review.sweep_notice {
                    tracing::info!("el repaso del turno tarda: aviso en la barra de estado");
                    review.sweep_notice = true;
                    review.refresh_summary(cx);
                    cx.notify();
                }
            });
        })
        .detach();
        stats.main_slice(started.elapsed());
        self.photo = Some(PhotoState::Sweeping {
            turn,
            snapshot,
            waiters: Vec::new(),
            host_writes: Vec::new(),
            chained,
            pending: None,
            adopted: 0,
            stats,
            started,
            _task: task,
        });
        // Nothing the painting code reads changed yet: the turn is still
        // active, so no notify (the observers would only repaint the same).
        false
    }

    /// The background comparison of `turn` landed: set Cincel's own writes
    /// aside and start adopting the rest.
    fn sweep_arrived(
        &mut self,
        turn: TurnId,
        changes: Vec<SnapshotChange>,
        elapsed: Duration,
        cx: &mut Context<Self>,
    ) {
        let slice = Instant::now();
        if self.active_turn != Some(turn) {
            return;
        }
        let Some(PhotoState::Sweeping {
            turn: sweeping,
            host_writes,
            pending,
            stats,
            ..
        }) = &mut self.photo
        else {
            return;
        };
        if *sweeping != turn {
            return;
        }
        stats.compare = elapsed;
        stats.changes = changes.len();
        let paths: VecDeque<PathBuf> = changes
            .into_iter()
            .map(|change| match change {
                SnapshotChange::Created(path)
                | SnapshotChange::Modified(path)
                | SnapshotChange::Removed(path) => path,
            })
            .filter(|path| !host_writes.contains(path))
            .collect();
        tracing::debug!(
            changes = paths.len(),
            ?elapsed,
            "repaso de la foto al terminar el turno"
        );
        *pending = Some(paths);
        self.sweep_step(turn, slice, cx);
    }

    /// Adopts the sweep's paths for at most [`SWEEP_SLICE`], then yields to
    /// the main thread's other work and goes on; closes the turn when none
    /// is left.
    fn sweep_step(&mut self, turn: TurnId, slice: Instant, cx: &mut Context<Self>) {
        loop {
            let sweeping = matches!(
                &self.photo,
                Some(PhotoState::Sweeping {
                    turn: sweeping,
                    pending: Some(_),
                    ..
                }) if *sweeping == turn
            );
            if !sweeping || self.active_turn != Some(turn) {
                return;
            }
            // The watcher's paths still queued go first: they are the
            // agent's too, and the turn must not close before them.
            let next = match self.next_watch_path() {
                Some(path) => Some(path),
                None => match &mut self.photo {
                    Some(PhotoState::Sweeping {
                        pending: Some(pending),
                        ..
                    }) => pending.pop_front(),
                    _ => None,
                },
            };
            let Some(path) = next else {
                break;
            };
            let adopted = self.adopt_change(&path, cx);
            if let Some(PhotoState::Sweeping { adopted: count, .. }) = &mut self.photo {
                *count += usize::from(adopted);
            }
            if slice.elapsed() >= SWEEP_SLICE {
                self.yield_sweep(turn, slice, cx);
                return;
            }
        }
        // Closing the turn refreshes every view: in a stretch of its own,
        // unless this one barely started.
        if slice.elapsed() >= SWEEP_SLICE / 4 {
            self.yield_sweep(turn, slice, cx);
            return;
        }
        self.complete_sweep(turn, slice, cx);
    }

    /// Ends the current main-thread stretch of the sweep (begun at `slice`)
    /// and goes on in a new one, after whatever else the main thread has to
    /// do (input, frames).
    fn yield_sweep(&mut self, turn: TurnId, slice: Instant, cx: &mut Context<Self>) {
        if let Some(PhotoState::Sweeping { stats, .. }) = &mut self.photo {
            stats.main_slice(slice.elapsed());
        }
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |review, cx| review.sweep_step(turn, Instant::now(), cx));
        })
        .detach();
    }

    /// Every path of the sweep of `turn` is adopted: the turn closes, and a
    /// chained turn starts (its prompt goes out once its own photo is
    /// ready). `slice` is when the current main-thread stretch began.
    fn complete_sweep(&mut self, turn: TurnId, slice: Instant, cx: &mut Context<Self>) {
        let Some(PhotoState::Sweeping {
            snapshot,
            waiters,
            chained,
            adopted,
            mut stats,
            started,
            ..
        }) = self.photo.take()
        else {
            return;
        };
        release_snapshot(snapshot, cx);
        if adopted > 0
            && let Some(project) = &self.project
        {
            project.read(cx).refresh_git();
        }
        self.sweep_notice = false;
        self.close_turn(cx);
        stats.adopted = adopted;
        // Closing the turn is main-thread work of the last slice too.
        stats.main_slice(slice.elapsed());
        stats.total = started.elapsed();
        tracing::info!(
            turn = turn.0,
            files = stats.files,
            changes = stats.changes,
            adopted = stats.adopted,
            compare = ?stats.compare,
            max_main_slice = ?stats.max_main_slice,
            total = ?stats.total,
            "repaso del turno terminado"
        );
        self.last_sweep = Some(stats);
        match chained {
            Some(next) => {
                self.open_turn(next, waiters, cx);
                self.refresh(None, cx);
            }
            None => {
                for waiter in waiters {
                    let _ = waiter.send(());
                }
            }
        }
    }

    /// `path` changed on disk during the turn: puts it in review against the
    /// photo. Returns whether anything was done.
    pub(super) fn adopt_change(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        let Some(turn) = self.active_turn else {
            return false;
        };
        if !self.in_project(path, cx) || self.host_wrote_during_sweep(path) {
            return false;
        }
        // A commented text file the agent turned into a binary loses its
        // comments (spec 09 §6.2.11).
        self.drop_comments_if_binary(path, cx);
        let Some(snapshot) = self.ready_snapshot() else {
            return false;
        };
        let ignored = snapshot.is_ignored(path, false);
        let entry = snapshot.entry(path).cloned();
        if self.store.file(path).is_some() {
            // Already in review: its base stays, the disk is the truth.
            return self.reread(path, cx);
        }
        let disk = if path.is_file() {
            std::fs::read(path).ok()
        } else {
            None
        };
        let disk_is_binary = disk.as_deref().is_some_and(looks_binary);
        if self.uncopied.contains_key(path) {
            return self.uncopied_changed_again(path, entry.as_ref(), disk.as_deref());
        }
        match (entry, disk) {
            (None, None) => false,
            (None, Some(_)) => {
                if ignored {
                    return false;
                }
                if disk_is_binary {
                    // Binary files are out of the review: applied as they are.
                    tracing::debug!(path = %path.display(), "binario creado por el agente: fuera de la revisión");
                    return false;
                }
                self.track_created(path, cx);
                tracing::debug!(path = %path.display(), "archivo creado durante el turno");
                true
            }
            (Some(entry), disk) => {
                if disk.as_deref().is_some_and(|bytes| entry.same_bytes(bytes)) {
                    return false;
                }
                if entry.is_binary() || disk_is_binary {
                    // A binary before or after: out of the review.
                    tracing::debug!(path = %path.display(), "binario cambiado por el agente: fuera de la revisión");
                    return false;
                }
                match &entry.content {
                    SnapshotContent::Text(text) => {
                        let before = normalize(text);
                        self.store.capture_base_text(path, &before);
                        let dirty = disk.is_some()
                            && self
                                .project
                                .as_ref()
                                .is_some_and(|project| project.read(cx).buffers().is_dirty(path));
                        if dirty {
                            // The buffer holds the user's unsaved changes (the
                            // disk is a conflict for it): they are the user's
                            // and join the base.
                            let Some(handle) = self.ensure_buffer(path, cx) else {
                                return true;
                            };
                            self.flush(None, cx);
                            let current = handle.lock().snapshot();
                            let tracked = self.store.file_written(path, &current, EditSource::User);
                            self.note(path, tracked);
                            true
                        } else {
                            tracing::debug!(path = %path.display(), "cambio del agente sin pista: base de la foto");
                            self.reread(path, cx)
                        }
                    }
                    SnapshotContent::HashOnly | SnapshotContent::Binary => {
                        // A text file the photo kept no copy of.
                        let state = if disk.is_some() {
                            FileState::Modified
                        } else {
                            FileState::Deleted
                        };
                        self.uncopied
                            .insert(path.to_path_buf(), UncopiedChange { turn, state });
                        tracing::debug!(path = %path.display(), "archivo cambiado por el agente sin copia previa: solo entero");
                        true
                    }
                }
            }
        }
    }

    /// A file already kept as an [`UncopiedChange`] changed again.
    fn uncopied_changed_again(
        &mut self,
        path: &Path,
        entry: Option<&cincel_project::SnapshotEntry>,
        disk: Option<&[u8]>,
    ) -> bool {
        let back = match (entry, disk) {
            (Some(entry), Some(bytes)) => entry.same_bytes(bytes),
            (None, None) => true,
            _ => false,
        };
        if back || disk.is_some_and(looks_binary) {
            // Back to what it was, or now a binary: nothing to review.
            self.uncopied.remove(path);
            return true;
        }
        if let Some(change) = self.uncopied.get_mut(path) {
            change.turn = self.active_turn.unwrap_or(change.turn);
            change.state = if disk.is_some() {
                FileState::Modified
            } else {
                FileState::Deleted
            };
        }
        true
    }
}
