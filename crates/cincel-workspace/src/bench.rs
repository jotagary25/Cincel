//! `cincel --bench`: Cincel's internal measurements
//! (`docs/specs/08-etapa6-cierre-1-0.md` §3.3, decision D6).
//!
//! The same release binary runs a measurement scenario inside the real
//! window and prints one JSON object per line on stdout. Without `--bench`
//! nothing here does anything: the hooks the workspace calls
//! ([`frame_begin`], [`frame_probe`]) read one thread-local flag and return.
//!
//! # Time marks
//!
//! * `t0`: `CINCEL_BENCH_T0` (nanoseconds of `CLOCK_MONOTONIC`, set by
//!   `cincel-perf` right before `spawn`) when present; otherwise the process
//!   start of `/proc/self/stat`, placed on `CLOCK_MONOTONIC` through
//!   `CLOCK_BOOTTIME` (one clock tick of resolution, 10 ms on Linux; the
//!   output says which one was used and its resolution).
//! * `main`, `config`, `window_open`: [`Session::start`], [`mark`] after the
//!   configuration is read, and [`Runner::start`] once the window exists.
//! * **End of a frame.** GPUI 0.3.5 offers no callback after `present`. Its
//!   frame request callback draws, presents and only then returns to the main
//!   loop, so a foreground task spawned while painting runs right after the
//!   frame was presented: that is the first point GPUI lets us observe after
//!   `present`, and it is the "fin del cuadro" every scenario uses. The task
//!   is spawned from the paint of [`FrameProbe`], a zero-size element the
//!   workspace adds as the last child of its root element (so it paints after
//!   everything but GPUI's deferred overlays). Each [`FrameEnd`] also keeps
//!   when the workspace started rendering and when the probe painted.
//!
//! # Scenarios
//!
//! `startup`, `open`, `typing`, `scroll`, `idle`, `finder`, `sweep`,
//! `review-1mb` (`all` runs the eight in that order) and `demo` (screenshots
//! only, not a measurement). When the command line names no corpus the bench
//! generates one in its own temporary directory ([`corpus`]), deleted when
//! the process ends. The file scenarios (`open`, `typing`, `scroll`) take
//! `--bench-file` or the three generated files; `review-1mb` always works on
//! the generated files (it writes into them); `idle`, `finder`, `sweep` and
//! `demo` take `RUTA` or a generated project. `sweep` writes into 200 files
//! of its project and writes their original bytes back when it is done.
//!
//! A measurement run (`--bench` without `--smoke-test`) moves the state, data
//! and cache directories into the bench's temporary directory before
//! anything reads them: recent projects, layouts, the review store and the
//! saved agent connections of the person running it are never touched. The
//! configuration directory stays, so the measured editor uses their settings.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use cincel_editor::EditorView;
use gpui::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Bounds, Context, Element, ElementId, Entity,
    FutureExt as _, GlobalElementId, InspectorElementId, IntoElement, Keystroke, LayoutId,
    Modifiers, MouseMoveEvent, Pixels, PlatformInput, Position, ScrollDelta, ScrollWheelEvent,
    Style, Task, TouchPhase, WeakEntity, Window, WindowHandle, point, px,
};
use gpui_kit::component::Root;
use gpui_kit::component::dock::DockPlacement;
use serde_json::{Value, json};

use crate::center::CenterPanel;
use crate::review::Review;
use crate::workspace::Workspace;

pub mod corpus;

pub use corpus::CorpusSizes;

/// The `tracing` target of the spans `CINCEL_TRACE_TIMINGS=1` reports
/// (configuration, themes, fonts, project open, first scan, review restore).
pub const TIMING_TARGET: &str = "cincel::timing";

/// Environment variable `cincel-perf` passes the launch instant in.
pub const T0_VAR: &str = "CINCEL_BENCH_T0";

/// The queries of `finder` (§3.3: "5 consultas de 3 a 8 letras").
pub const FINDER_QUERIES: [&str; 5] = ["mod", "main", "parse", "config", "handler"];

// ------------------------------------------------------------------ scenario

/// A `--bench` scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// `process_start→main→config→window_open→first_frame`.
    Startup,
    /// Opening a file: `open_file` → end of the first frame that shows it.
    Open,
    /// Key → end of the frame that shows it.
    Typing,
    /// One wheel event per frame for 3 s.
    Scroll,
    /// 30 s of settling, then 60 s counting frames, CPU and memory.
    Idle,
    /// `Ctrl+P` and five queries: key → end of the frame with results.
    Finder,
    /// A synthetic turn over a large project: photo, sweep, main thread gaps.
    Sweep,
    /// 50 agent edits on the 1 MB file, then typing with them pending.
    Review1Mb,
    /// The eight measurements, in order.
    All,
    /// A synthetic turn left pending for screenshots; not a measurement.
    Demo,
}

impl Scenario {
    /// Every scenario the command line accepts.
    pub const ALL: [Scenario; 10] = [
        Scenario::Startup,
        Scenario::Open,
        Scenario::Typing,
        Scenario::Scroll,
        Scenario::Idle,
        Scenario::Finder,
        Scenario::Sweep,
        Scenario::Review1Mb,
        Scenario::All,
        Scenario::Demo,
    ];

    /// The eight measurements `all` runs, in order.
    pub const MEASUREMENTS: [Scenario; 8] = [
        Scenario::Startup,
        Scenario::Open,
        Scenario::Typing,
        Scenario::Scroll,
        Scenario::Idle,
        Scenario::Finder,
        Scenario::Sweep,
        Scenario::Review1Mb,
    ];

    /// The name on the command line and in the output.
    pub fn name(self) -> &'static str {
        match self {
            Scenario::Startup => "startup",
            Scenario::Open => "open",
            Scenario::Typing => "typing",
            Scenario::Scroll => "scroll",
            Scenario::Idle => "idle",
            Scenario::Finder => "finder",
            Scenario::Sweep => "sweep",
            Scenario::Review1Mb => "review-1mb",
            Scenario::All => "all",
            Scenario::Demo => "demo",
        }
    }

    /// The scenario called `name`, if any.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|scenario| scenario.name() == name)
    }

    /// `startup, open, …, demo`, for the usage errors.
    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|scenario| scenario.name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// What to measure, as the command line said it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BenchPlan {
    /// The scenario (`all` expands to the eight measurements).
    pub scenario: Scenario,
    /// `RUTA`: the project of `startup`, `idle`, `finder`, `sweep`, `demo`.
    pub root: Option<PathBuf>,
    /// `--bench-file`: the file of `open`, `typing`, `scroll`.
    pub file: Option<PathBuf>,
    /// `--smoke-test --bench`: only `startup`, and the smoke test quits.
    pub smoke_test: bool,
}

/// Sizes and durations. [`BenchConfig::default`] is §3.3 as written; the
/// tests use [`BenchConfig::tiny`].
#[derive(Clone, Debug)]
pub struct BenchConfig {
    /// The generated corpus.
    pub corpus: CorpusSizes,
    /// Opens per file (`open`).
    pub open_repeats: usize,
    /// Keys per file (`typing`, and `review-1mb` with pending segments).
    /// Even, so the letters and the backspaces leave the file as it was.
    pub typing_keys: usize,
    /// How long `scroll` sends wheel events.
    pub scroll_duration: Duration,
    /// `idle`: time before counting.
    pub idle_settle: Duration,
    /// `idle`: time counted.
    pub idle_window: Duration,
    /// Files `sweep` changes.
    pub sweep_files: usize,
    /// Agent edits `review-1mb` makes per file.
    pub review_edits: usize,
    /// No frame for this long means "at rest".
    pub quiet: Duration,
    /// Longest wait for rest.
    pub settle_max: Duration,
    /// Longest wait for the frame a scenario expects.
    pub frame_timeout: Duration,
    /// Longest wait for the frame of one wheel event.
    pub scroll_frame_timeout: Duration,
    /// Time the watcher gets between `sweep`'s writes and the end of the turn.
    pub watch_settle: Duration,
    /// The tree is considered scanned after this long without new entries.
    pub scan_quiet: Duration,
    /// Longest wait for a scan, a photo or the end of a turn.
    pub work_timeout: Duration,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            corpus: CorpusSizes::default(),
            open_repeats: 10,
            typing_keys: 200,
            scroll_duration: Duration::from_secs(3),
            idle_settle: Duration::from_secs(30),
            idle_window: Duration::from_secs(60),
            sweep_files: 200,
            review_edits: 50,
            quiet: Duration::from_millis(300),
            settle_max: Duration::from_secs(3),
            frame_timeout: Duration::from_secs(5),
            scroll_frame_timeout: Duration::from_millis(250),
            watch_settle: Duration::from_millis(500),
            scan_quiet: Duration::from_secs(1),
            work_timeout: Duration::from_secs(180),
        }
    }
}

impl BenchConfig {
    /// A few lines, files and milliseconds of everything: for the tests,
    /// which check the shape of the output and not the times.
    pub fn tiny() -> Self {
        Self {
            // Longer than any test window is tall, so `scroll` can move.
            corpus: CorpusSizes {
                five_k_lines: 400,
                one_mb_bytes: 24 * 1024,
                fifty_k_lines: 600,
                large_files: 40,
                large_folders: 4,
            },
            open_repeats: 2,
            typing_keys: 6,
            scroll_duration: Duration::from_millis(40),
            idle_settle: Duration::from_millis(20),
            idle_window: Duration::from_millis(40),
            sweep_files: 5,
            review_edits: 3,
            quiet: Duration::from_millis(20),
            settle_max: Duration::from_millis(200),
            frame_timeout: Duration::from_millis(500),
            scroll_frame_timeout: Duration::from_millis(50),
            watch_settle: Duration::from_millis(10),
            scan_quiet: Duration::from_millis(30),
            work_timeout: Duration::from_secs(5),
        }
    }
}

// --------------------------------------------------------------------- clock

/// `CLOCK_MONOTONIC` in nanoseconds (the clock `cincel-perf` and sway use).
pub fn now_ns() -> u64 {
    timespec_ns(rustix::time::clock_gettime(
        rustix::time::ClockId::Monotonic,
    ))
}

fn timespec_ns(time: rustix::time::Timespec) -> u64 {
    (time.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(time.tv_nsec as u64)
}

/// Milliseconds between two marks, rounded to microseconds.
fn ms(from_ns: u64, to_ns: u64) -> f64 {
    round3(to_ns.saturating_sub(from_ns) as f64 / 1_000_000.)
}

fn round3(value: f64) -> f64 {
    (value * 1000.).round() / 1000.
}

/// The fields of `/proc/<pid>/stat` the bench reads, in clock ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatFields {
    /// Field 14.
    pub utime: u64,
    /// Field 15.
    pub stime: u64,
    /// Field 22: start time since boot.
    pub starttime: u64,
}

/// Parses `/proc/<pid>/stat`. The command name (field 2) is in parentheses
/// and may hold spaces, so fields are counted from the last `)`.
pub fn parse_stat(stat: &str) -> Option<StatFields> {
    let rest = &stat[stat.rfind(')')? + 1..];
    // `rest` starts at field 3 (the state).
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let field = |number: usize| fields.get(number - 3)?.parse::<u64>().ok();
    Some(StatFields {
        utime: field(14)?,
        stime: field(15)?,
        starttime: field(22)?,
    })
}

fn own_stat() -> Option<StatFields> {
    parse_stat(&std::fs::read_to_string("/proc/self/stat").ok()?)
}

/// `VmRSS` of `/proc/<pid>/status`, in kB.
pub fn parse_vm_rss_kb(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn own_vm_rss_mb() -> Option<f64> {
    let kb = parse_vm_rss_kb(&std::fs::read_to_string("/proc/self/status").ok()?)?;
    Some(round3(kb as f64 / 1024.))
}

/// Where `t0` came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessStart {
    /// `CLOCK_MONOTONIC` nanoseconds.
    pub ns: u64,
    /// `"CINCEL_BENCH_T0"` or `"proc_stat"`.
    pub source: &'static str,
    /// Resolution of the source, in milliseconds.
    pub resolution_ms: f64,
}

/// `t0`: `CINCEL_BENCH_T0` when set, else the start time of `/proc/self/stat`
/// moved from `CLOCK_BOOTTIME` ticks onto `CLOCK_MONOTONIC`.
pub fn process_start() -> Option<ProcessStart> {
    if let Some(ns) = std::env::var(T0_VAR)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        return Some(ProcessStart {
            ns,
            source: T0_VAR,
            resolution_ms: 0.,
        });
    }
    let ticks = rustix::param::clock_ticks_per_second().max(1);
    let start = own_stat()?.starttime;
    let boot_now = timespec_ns(rustix::time::clock_gettime(rustix::time::ClockId::Boottime));
    let mono_now = now_ns();
    let start_boot = (start as u128 * 1_000_000_000 / ticks as u128) as u64;
    let age = boot_now.saturating_sub(start_boot);
    Some(ProcessStart {
        ns: mono_now.saturating_sub(age),
        source: "proc_stat",
        resolution_ms: 1000. / ticks as f64,
    })
}

fn cpu_ms() -> Option<f64> {
    let stat = own_stat()?;
    let ticks = rustix::param::clock_ticks_per_second().max(1);
    Some((stat.utime + stat.stime) as f64 * 1000. / ticks as f64)
}

// --------------------------------------------------------------------- marks

/// The startup marks the binary takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// First statement of `main`.
    Main,
    /// The configuration was read.
    Config,
    /// The window exists.
    WindowOpen,
}

#[derive(Clone, Copy, Debug, Default)]
struct Marks {
    main: Option<u64>,
    config: Option<u64>,
    window_open: Option<u64>,
    first_frame: Option<u64>,
}

thread_local! {
    /// Whether this thread (GPUI's main thread, or a test's) runs a bench.
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static MARKS: Cell<Marks> = const {
        Cell::new(Marks { main: None, config: None, window_open: None, first_frame: None })
    };
    static PROBE: RefCell<Probe> = RefCell::new(Probe::default());
}

/// Exit code of the bench (`0`, or `1` when a scenario could not run).
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);
/// Whether the `startup` line went out (`--smoke-test --bench` checks it).
static STARTUP_EMITTED: AtomicBool = AtomicBool::new(false);

/// Whether a bench runs on this thread.
pub fn is_active() -> bool {
    ACTIVE.with(Cell::get)
}

/// Turns the hooks on for this thread (the binary through
/// [`Session::start`], the tests directly).
pub fn activate() {
    ACTIVE.with(|active| active.set(true));
}

/// Turns the hooks off and forgets every mark and frame.
pub fn deactivate() {
    ACTIVE.with(|active| active.set(false));
    MARKS.with(|marks| marks.set(Marks::default()));
    PROBE.with_borrow_mut(|probe| *probe = Probe::default());
}

/// Records `mark` now (nothing without a bench).
pub fn mark(mark: Mark) {
    mark_at(mark, now_ns());
}

fn mark_at(mark: Mark, ns: u64) {
    if !is_active() {
        return;
    }
    MARKS.with(|marks| {
        let mut current = marks.get();
        match mark {
            Mark::Main => current.main = Some(ns),
            Mark::Config => current.config = Some(ns),
            Mark::WindowOpen => current.window_open = Some(ns),
        }
        marks.set(current);
    });
}

// --------------------------------------------------------------------- probe

/// One painted frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameEnd {
    /// 1 for the first frame painted since the bench started.
    pub seq: u64,
    /// When the workspace started rendering it.
    pub begin_ns: u64,
    /// When [`FrameProbe`] painted.
    pub paint_ns: u64,
    /// First instant after `present` (see the module documentation).
    pub end_ns: u64,
    /// Whether the scenario's condition held while it was painted.
    pub matched: bool,
}

type Condition = Rc<dyn Fn(&App) -> bool>;

#[derive(Default)]
struct Probe {
    frames: u64,
    begin_ns: Option<u64>,
    condition: Option<Condition>,
    log: VecDeque<FrameEnd>,
    wakers: Vec<async_channel::Sender<()>>,
}

/// How many frames the probe remembers.
const PROBE_LOG: usize = 4096;

/// The workspace is about to render a frame.
pub(crate) fn frame_begin() {
    if !is_active() {
        return;
    }
    let now = now_ns();
    PROBE.with_borrow_mut(|probe| probe.begin_ns = Some(now));
}

/// The probe element, only while a bench runs.
pub(crate) fn frame_probe() -> Option<FrameProbe> {
    is_active().then_some(FrameProbe)
}

/// Frames painted since the bench started.
pub fn frames_painted() -> u64 {
    PROBE.with_borrow(|probe| probe.frames)
}

fn set_condition(condition: impl Fn(&App) -> bool + 'static) {
    PROBE.with_borrow_mut(|probe| probe.condition = Some(Rc::new(condition)));
}

fn clear_condition() {
    PROBE.with_borrow_mut(|probe| probe.condition = None);
}

/// A zero-size element whose paint marks the frame (module documentation).
pub(crate) struct FrameProbe;

impl IntoElement for FrameProbe {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FrameProbe {
    type RequestLayoutState = ();
    type PrepaintState = ();

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
        let style = Style {
            position: Position::Absolute,
            ..Style::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        cx: &mut App,
    ) {
        on_probe_paint(cx);
    }
}

fn on_probe_paint(cx: &mut App) {
    let paint_ns = now_ns();
    let (seq, begin_ns, condition) = PROBE.with_borrow_mut(|probe| {
        probe.frames += 1;
        (
            probe.frames,
            probe.begin_ns.take().unwrap_or(paint_ns),
            probe.condition.clone(),
        )
    });
    let matched = condition.is_none_or(|condition| condition(cx));
    cx.foreground_executor()
        .spawn(async move {
            let end_ns = now_ns();
            let frame = FrameEnd {
                seq,
                begin_ns,
                paint_ns,
                end_ns,
                matched,
            };
            MARKS.with(|marks| {
                let mut current = marks.get();
                if current.first_frame.is_none()
                    && current.window_open.is_some_and(|open| paint_ns >= open)
                {
                    current.first_frame = Some(end_ns);
                    marks.set(current);
                }
            });
            PROBE.with_borrow_mut(|probe| {
                probe.log.push_back(frame);
                while probe.log.len() > PROBE_LOG {
                    probe.log.pop_front();
                }
                for waker in probe.wakers.drain(..) {
                    let _ = waker.try_send(());
                }
            });
        })
        .detach();
}

// ------------------------------------------------------------------- session

/// What the binary keeps for the whole process: the plan and the bench's
/// temporary directory (the generated corpus, and the state of a measurement
/// run), deleted by [`Session::finish`].
pub struct Session {
    runner: Runner,
    dir: tempfile::TempDir,
}

/// What the binary hands to [`Runner::start`] once the window exists.
#[derive(Clone, Debug)]
pub struct Runner {
    plan: BenchPlan,
    work_dir: PathBuf,
}

impl Session {
    /// Turns the hooks on for this thread, records `main` at `started_ns`
    /// and prepares the temporary directory. A measurement run (not
    /// `--smoke-test`) also moves the state, data and cache directories
    /// there; call this before anything reads them.
    pub fn start(plan: BenchPlan, started_ns: u64) -> std::io::Result<Self> {
        activate();
        mark_at(Mark::Main, started_ns);
        let dir = tempfile::Builder::new().prefix("cincel-bench-").tempdir()?;
        if !plan.smoke_test {
            isolate_state(dir.path())?;
        }
        Ok(Self {
            runner: Runner {
                plan,
                work_dir: dir.path().to_path_buf(),
            },
            dir,
        })
    }

    /// The runner to start once the window exists.
    pub fn runner(&self) -> Runner {
        self.runner.clone()
    }

    /// Deletes the temporary directory and returns the exit code.
    pub fn finish(self) -> i32 {
        let smoke_without_startup =
            self.runner.plan.smoke_test && !STARTUP_EMITTED.load(Ordering::SeqCst);
        if let Err(error) = self.dir.close() {
            eprintln!("cincel: no se pudo borrar la carpeta temporal del banco: {error}");
        }
        if smoke_without_startup {
            eprintln!("cincel: --bench: la ventana no llegó a pintar su primer cuadro");
            return 1;
        }
        EXIT_CODE.load(Ordering::SeqCst)
    }
}

/// Moves the XDG state, data and cache directories into a fresh temporary
/// directory, so a `--smoke-test` run (like a `--bench` one) never touches
/// the user's recents, layout, connections or logs. Keep the returned
/// directory alive for the whole process. Call it at the top of `main`,
/// before anything reads the environment.
pub fn isolate_state_in_temp() -> std::io::Result<tempfile::TempDir> {
    let dir = tempfile::Builder::new().prefix("cincel-smoke-").tempdir()?;
    isolate_state(dir.path())?;
    Ok(dir)
}

/// Points `XDG_STATE_HOME`, `XDG_DATA_HOME` and `XDG_CACHE_HOME` into `dir`.
#[allow(unsafe_code)]
fn isolate_state(dir: &Path) -> std::io::Result<()> {
    for (var, sub) in [
        ("XDG_STATE_HOME", "state"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CACHE_HOME", "cache"),
    ] {
        let path = dir.join(sub);
        std::fs::create_dir_all(&path)?;
        // SAFETY: `Session::start` runs at the top of `main`, before the
        // logging thread, GPUI or anything else has started a thread that
        // could read the environment concurrently.
        unsafe { std::env::set_var(var, &path) };
    }
    Ok(())
}

impl Runner {
    /// The window exists: records `window_open`, forces the next frame to
    /// draw (so `first_frame` is a frame GPUI really presents) and starts the
    /// scenario. When it finishes the exit code is set and the application
    /// quits, except under `--smoke-test` (the smoke test quits) and in `demo`
    /// (it stays until `SIGTERM`).
    pub fn start(self, window: WindowHandle<Root>, cx: &mut AsyncApp) {
        mark(Mark::WindowOpen);
        let workspace = window
            .update(cx, |root, window, _| {
                window.refresh();
                root.view()
                    .clone()
                    .downcast::<Workspace>()
                    .ok()
                    .map(|workspace| workspace.downgrade())
            })
            .ok()
            .flatten();
        let smoke = self.plan.smoke_test;
        let Some(workspace) = workspace else {
            print_line(&json!({
                "scenario": self.plan.scenario.name(),
                "error": "la ventana no tiene un área de trabajo",
            }));
            EXIT_CODE.store(1, Ordering::SeqCst);
            if !smoke {
                cx.update(|cx| cx.quit());
            }
            return;
        };
        let sink: Sink = Rc::new(print_line);
        let task = cx.update(|cx| {
            run(
                self.plan,
                BenchConfig::default(),
                self.work_dir,
                window.into(),
                workspace,
                sink,
                cx,
            )
        });
        cx.spawn(async move |cx| match task.await {
            Outcome::Done { ok } => {
                EXIT_CODE.store(if ok { 0 } else { 1 }, Ordering::SeqCst);
                if !smoke {
                    cx.update(|cx| cx.quit());
                }
            }
            Outcome::KeepOpen => {}
        })
        .detach();
    }
}

fn print_line(line: &Value) {
    use std::io::Write as _;
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

// -------------------------------------------------------------------- driver

/// Where the output lines go (stdout in the binary, a vector in the tests).
pub type Sink = Rc<dyn Fn(&Value)>;

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Every scenario ran (`ok`) or one of them could not.
    Done {
        /// No scenario failed.
        ok: bool,
    },
    /// `demo`: the window stays as it is.
    KeepOpen,
}

type BenchResult<T> = Result<T, String>;

/// Runs `plan` in `window`, whose root view is (or holds) `workspace`,
/// generating the corpus under `work_dir` when needed.
pub fn run(
    plan: BenchPlan,
    config: BenchConfig,
    work_dir: PathBuf,
    window: AnyWindowHandle,
    workspace: WeakEntity<Workspace>,
    sink: Sink,
    cx: &mut App,
) -> Task<Outcome> {
    activate();
    let driver = Driver {
        plan,
        config,
        work_dir,
        window,
        workspace,
        sink,
        text_files: None,
        large: None,
    };
    cx.spawn(async move |cx| driver.run(cx).await)
}

struct Driver {
    plan: BenchPlan,
    config: BenchConfig,
    work_dir: PathBuf,
    window: AnyWindowHandle,
    workspace: WeakEntity<Workspace>,
    sink: Sink,
    text_files: Option<corpus::TextFiles>,
    large: Option<PathBuf>,
}

impl Driver {
    async fn run(mut self, cx: &mut AsyncApp) -> Outcome {
        let scenarios: Vec<Scenario> = match self.plan.scenario {
            Scenario::All => Scenario::MEASUREMENTS.to_vec(),
            _ if self.plan.smoke_test => vec![Scenario::Startup],
            scenario => vec![scenario],
        };
        let mut ok = true;
        for scenario in scenarios {
            let result = match scenario {
                Scenario::Startup => self.startup(cx).await,
                Scenario::Open => self.open(cx).await,
                Scenario::Typing => self.typing(cx).await,
                Scenario::Scroll => self.scroll(cx).await,
                Scenario::Idle => self.idle(cx).await,
                Scenario::Finder => self.finder(cx).await,
                Scenario::Sweep => self.sweep(cx).await,
                Scenario::Review1Mb => self.review_1mb(cx).await,
                Scenario::Demo => match self.demo(cx).await {
                    Ok(lines) => {
                        for line in &lines {
                            (self.sink)(line);
                        }
                        clear_condition();
                        return Outcome::KeepOpen;
                    }
                    Err(error) => Err(error),
                },
                Scenario::All => unreachable!("`all` was expanded above"),
            };
            clear_condition();
            match result {
                Ok(lines) => {
                    for line in &lines {
                        (self.sink)(line);
                    }
                    if scenario == Scenario::Startup {
                        STARTUP_EMITTED.store(true, Ordering::SeqCst);
                    }
                }
                Err(error) => {
                    ok = false;
                    tracing::warn!(scenario = scenario.name(), %error, "--bench: el escenario no pudo correr");
                    (self.sink)(&json!({ "scenario": scenario.name(), "error": error }));
                }
            }
        }
        Outcome::Done { ok }
    }

    // ---------------------------------------------------------- helpers

    /// Runs `f` on the workspace, with its window.
    fn ws<R>(
        &self,
        cx: &mut AsyncApp,
        f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> R,
    ) -> BenchResult<R> {
        let workspace = self.workspace.clone();
        cx.update_window(self.window, |_, window, cx| {
            workspace.update(cx, |workspace, cx| f(workspace, window, cx))
        })
        .map_err(|error| format!("la ventana se cerró: {error}"))?
        .map_err(|error| format!("el área de trabajo se cerró: {error}"))
    }

    /// Runs `f` on the window.
    fn win<R>(
        &self,
        cx: &mut AsyncApp,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> BenchResult<R> {
        cx.update_window(self.window, |_, window, cx| f(window, cx))
            .map_err(|error| format!("la ventana se cerró: {error}"))
    }

    fn center(&self, cx: &mut AsyncApp) -> BenchResult<Entity<CenterPanel>> {
        self.ws(cx, |workspace, _, _| workspace.center().clone())
    }

    fn review(&self, cx: &mut AsyncApp) -> BenchResult<Entity<Review>> {
        self.ws(cx, |workspace, _, _| workspace.review().clone())
    }

    async fn sleep(&self, cx: &mut AsyncApp, duration: Duration) {
        cx.background_executor().timer(duration).await;
    }

    /// The first frame painted at or after `after_ns` (whose condition held,
    /// when `need_match`), or `None` after `timeout`.
    async fn frame_after(
        &self,
        cx: &mut AsyncApp,
        after_ns: u64,
        need_match: bool,
        timeout: Duration,
    ) -> Option<FrameEnd> {
        let executor = cx.background_executor().clone();
        let deadline = executor.now() + timeout;
        loop {
            let found = PROBE.with_borrow(|probe| {
                probe
                    .log
                    .iter()
                    .find(|frame| frame.paint_ns >= after_ns && (!need_match || frame.matched))
                    .copied()
            });
            if found.is_some() {
                return found;
            }
            let now = executor.now();
            if now >= deadline {
                return None;
            }
            let (waker, woken) = async_channel::bounded::<()>(1);
            PROBE.with_borrow_mut(|probe| probe.wakers.push(waker));
            if woken
                .recv()
                .with_timeout(deadline - now, &executor)
                .await
                .is_err()
            {
                return None;
            }
        }
    }

    /// Waits until no frame is painted for `quiet` (at most `settle_max`).
    async fn settle(&self, cx: &mut AsyncApp) {
        let executor = cx.background_executor().clone();
        let deadline = executor.now() + self.config.settle_max;
        while executor.now() < deadline {
            let quiet = self.config.quiet;
            if self.frame_after(cx, now_ns(), false, quiet).await.is_none() {
                return;
            }
        }
    }

    /// Polls `done` every millisecond until it holds (or `timeout`).
    async fn poll_until(
        &self,
        cx: &mut AsyncApp,
        timeout: Duration,
        mut done: impl FnMut(&mut AsyncApp) -> BenchResult<bool>,
    ) -> BenchResult<bool> {
        let executor = cx.background_executor().clone();
        let deadline = executor.now() + timeout;
        loop {
            if done(cx)? {
                return Ok(true);
            }
            if executor.now() >= deadline {
                return Ok(false);
            }
            executor.timer(Duration::from_millis(1)).await;
        }
    }

    /// `archivos/`, generated the first time it is needed.
    async fn text_files(&mut self, cx: &mut AsyncApp) -> BenchResult<corpus::TextFiles> {
        if let Some(files) = &self.text_files {
            return Ok(files.clone());
        }
        let dir = self.work_dir.clone();
        let sizes = self.config.corpus.clone();
        let files = cx
            .background_executor()
            .spawn(async move { corpus::text_files(&dir, &sizes) })
            .await
            .map_err(|error| format!("no se pudo generar el corpus: {error}"))?;
        self.text_files = Some(files.clone());
        Ok(files)
    }

    /// `RUTA`, or `grande/` generated the first time it is needed.
    async fn large_root(&mut self, cx: &mut AsyncApp) -> BenchResult<PathBuf> {
        if let Some(root) = &self.plan.root {
            return absolute(root);
        }
        if let Some(root) = &self.large {
            return Ok(root.clone());
        }
        let dir = self.work_dir.clone();
        let (files, folders) = (
            self.config.corpus.large_files,
            self.config.corpus.large_folders,
        );
        let root = cx
            .background_executor()
            .spawn(async move { corpus::large_project(&dir, files, folders) })
            .await
            .map_err(|error| format!("no se pudo generar el proyecto grande: {error}"))?;
        self.large = Some(root.clone());
        Ok(root)
    }

    /// The project and the files of `open`, `typing` and `scroll`.
    async fn file_targets(&mut self, cx: &mut AsyncApp) -> BenchResult<(PathBuf, Vec<PathBuf>)> {
        if let Some(file) = &self.plan.file {
            let file = absolute(file)?;
            if !file.is_file() {
                return Err(format!("no existe el archivo {}", file.display()));
            }
            let root = match &self.plan.root {
                Some(root) => absolute(root)?,
                None => file
                    .parent()
                    .map(Path::to_path_buf)
                    .ok_or("el archivo no tiene carpeta")?,
            };
            return Ok((root, vec![file]));
        }
        let files = self.text_files(cx).await?;
        Ok((files.root.clone(), files.all()))
    }

    /// Opens `root` as the project unless it already is, and waits for the
    /// scan to finish.
    async fn ensure_project(&self, root: &Path, cx: &mut AsyncApp) -> BenchResult<()> {
        let current = self.ws(cx, |workspace, _, cx| {
            workspace
                .project()
                .map(|project| project.read(cx).root().to_path_buf())
        })?;
        if current.as_deref() != Some(root) {
            let target = root.to_path_buf();
            self.ws(cx, move |workspace, window, cx| {
                workspace.open_project(&target, window, cx)
            })?;
            let opened = self.ws(cx, |workspace, _, cx| {
                workspace
                    .project()
                    .map(|project| project.read(cx).root().to_path_buf())
            })?;
            if opened.as_deref() != Some(root) {
                return Err(format!("no se pudo abrir el proyecto {}", root.display()));
            }
        }
        self.wait_scan(cx).await
    }

    /// Waits until the tree stops growing for `scan_quiet`.
    async fn wait_scan(&self, cx: &mut AsyncApp) -> BenchResult<()> {
        let executor = cx.background_executor().clone();
        let deadline = executor.now() + self.config.work_timeout;
        let mut last = usize::MAX;
        let mut stable_since = executor.now();
        loop {
            let len = self.ws(cx, |workspace, _, cx| {
                workspace
                    .project()
                    .map(|project| project.read(cx).worktree().len())
                    .unwrap_or(0)
            })?;
            let now = executor.now();
            if len != last {
                last = len;
                stable_since = now;
            } else if now - stable_since >= self.config.scan_quiet {
                return Ok(());
            }
            if now >= deadline {
                return Err("el proyecto no terminó de recorrerse".to_string());
            }
            executor.timer(Duration::from_millis(50)).await;
        }
    }

    /// The files of the open project, relative paths as the finder lists
    /// them.
    fn project_files(&self, cx: &mut AsyncApp) -> BenchResult<Vec<PathBuf>> {
        self.ws(cx, |workspace, _, cx| {
            workspace
                .project()
                .map(|project| {
                    let project = project.read(cx);
                    project
                        .worktree()
                        .entries()
                        .filter(|entry| entry.kind == cincel_project::EntryKind::File)
                        .map(|entry| project.absolute(&entry.path))
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    /// Closes the tab of `path` if it is open and clean.
    fn close_file(&self, path: &Path, cx: &mut AsyncApp) -> BenchResult<()> {
        let path = path.to_path_buf();
        self.ws(cx, move |workspace, window, cx| {
            workspace.center().clone().update(cx, |center, cx| {
                if let Some(index) = center.tabs().iter().position(|tab| tab.path == path) {
                    center.close_tab(index, window, cx);
                }
            })
        })
    }

    /// Opens `path` pinned (or activates its tab), focuses its editor and
    /// waits for rest.
    async fn open_pinned(
        &self,
        path: &Path,
        cx: &mut AsyncApp,
    ) -> BenchResult<WeakEntity<EditorView>> {
        let target = path.to_path_buf();
        let editor = self.ws(cx, move |workspace, window, cx| {
            workspace.center().clone().update(cx, |center, cx| {
                center.open_file(&target, true, window, cx);
                center.focus_active(window, cx);
                center
                    .active_tab()
                    .filter(|tab| tab.path == target)
                    .map(|tab| tab.editor().downgrade())
            })
        })?;
        let editor = editor.ok_or_else(|| format!("no se pudo abrir {}", path.display()))?;
        self.settle(cx).await;
        Ok(editor)
    }

    /// `keys` keys on the focused editor, letter and backspace alternated,
    /// one per frame: key → end of the frame that shows it, in ms.
    async fn type_keys(
        &self,
        editor: &WeakEntity<EditorView>,
        keys: usize,
        cx: &mut AsyncApp,
    ) -> BenchResult<Vec<f64>> {
        let letter = Keystroke::parse("a").map_err(|error| error.to_string())?;
        let backspace = Keystroke::parse("backspace").map_err(|error| error.to_string())?;
        let version = |cx: &mut AsyncApp| {
            cx.update(|cx| {
                editor
                    .upgrade()
                    .map(|editor| editor.read(cx).text_version())
            })
        };
        let mut samples = Vec::with_capacity(keys);
        for index in 0..keys {
            let key = if index % 2 == 0 {
                letter.clone()
            } else {
                backspace.clone()
            };
            let before = version(cx);
            let start = now_ns();
            self.win(cx, |window, cx| window.dispatch_keystroke(key, cx))?;
            if index == 0 && version(cx) == before {
                return Err("la tecla no llegó al editor".to_string());
            }
            let frame = self
                .frame_after(cx, start, false, self.config.frame_timeout)
                .await
                .ok_or("ningún cuadro mostró la tecla")?;
            samples.push(ms(start, frame.end_ns));
        }
        Ok(samples)
    }

    // --------------------------------------------------------- scenarios

    async fn startup(&self, cx: &mut AsyncApp) -> BenchResult<Vec<Value>> {
        let arrived = self
            .poll_until(cx, self.config.frame_timeout, |_| {
                Ok(MARKS.with(|marks| marks.get().first_frame.is_some()))
            })
            .await?;
        if !arrived {
            return Err("la ventana no pintó su primer cuadro".to_string());
        }
        let marks = MARKS.with(Cell::get);
        let start = process_start().ok_or("no se pudo leer el inicio del proceso")?;
        let (Some(main), Some(config), Some(window_open), Some(first_frame)) = (
            marks.main,
            marks.config,
            marks.window_open,
            marks.first_frame,
        ) else {
            return Err("faltan marcas de arranque".to_string());
        };
        Ok(vec![json!({
            "scenario": "startup",
            "t0_source": start.source,
            "t0_resolution_ms": round3(start.resolution_ms),
            "process_start_to_main_ms": ms(start.ns, main),
            "main_to_config_ms": ms(main, config),
            "config_to_window_open_ms": ms(config, window_open),
            "window_open_to_first_frame_ms": ms(window_open, first_frame),
            "total_ms": ms(start.ns, first_frame),
            "frame_end": "post_present_task",
        })])
    }

    async fn open(&mut self, cx: &mut AsyncApp) -> BenchResult<Vec<Value>> {
        let (root, files) = self.file_targets(cx).await?;
        self.ensure_project(&root, cx).await?;
        let center = self.center(cx)?.downgrade();
        let mut lines = Vec::new();
        for file in files {
            let mut samples = Vec::new();
            for _ in 0..self.config.open_repeats {
                self.close_file(&file, cx)?;
                self.settle(cx).await;
                let (center, target) = (center.clone(), file.clone());
                set_condition(move |cx| {
                    center.upgrade().is_some_and(|center| {
                        center
                            .read(cx)
                            .active_tab()
                            .is_some_and(|tab| tab.path == target)
                    })
                });
                let target = file.clone();
                let start = now_ns();
                self.ws(cx, move |workspace, window, cx| {
                    workspace
                        .center()
                        .clone()
                        .update(cx, |center, cx| center.open_file(&target, true, window, cx))
                })?;
                let frame = self
                    .frame_after(cx, start, true, self.config.frame_timeout)
                    .await;
                clear_condition();
                let frame =
                    frame.ok_or_else(|| format!("{} no llegó a dibujarse", file_name(&file)))?;
                samples.push(ms(start, frame.end_ns));
            }
            self.close_file(&file, cx)?;
            let mut line = file_info(&file);
            line["scenario"] = json!("open");
            line["repeats"] = json!(samples.len());
            line["open_to_frame_ms"] = stats(&samples);
            lines.push(line);
        }
        Ok(lines)
    }

    async fn typing(&mut self, cx: &mut AsyncApp) -> BenchResult<Vec<Value>> {
        let (root, files) = self.file_targets(cx).await?;
        self.ensure_project(&root, cx).await?;
        let mut lines = Vec::new();
        for file in files {
            let editor = self.open_pinned(&file, cx).await?;
            let samples = self.type_keys(&editor, self.config.typing_keys, cx).await?;
            let mut line = file_info(&file);
            line["scenario"] = json!("typing");
            line["keys"] = json!(samples.len());
            line["key_to_frame_ms"] = stats(&samples);
            lines.push(line);
        }
        Ok(lines)
    }

    async fn scroll(&mut self, cx: &mut AsyncApp) -> BenchResult<Vec<Value>> {
        let (root, files) = self.file_targets(cx).await?;
        self.ensure_project(&root, cx).await?;
        let mut lines = Vec::new();
        for file in files {
            let editor = self.open_pinned(&file, cx).await?;
            cx.update(|cx| {
                if let Some(editor) = editor.upgrade() {
                    editor.update(cx, |editor, cx| editor.set_scroll_row(0., cx));
                }
            });
            self.settle(cx).await;
            let position = self.ws(cx, |workspace, window, cx| {
                let size = window.viewport_size();
                let side = |placement| {
                    if workspace.is_dock_open(placement, cx) {
                        workspace
                            .dock_width(placement, cx)
                            .map(f32::from)
                            .unwrap_or(0.)
                    } else {
                        0.
                    }
                };
                let (left, right) = (side(DockPlacement::Left), side(DockPlacement::Right));
                let width = f32::from(size.width);
                point(
                    px((left + (width - right)) / 2.),
                    px(f32::from(size.height) / 2.),
                )
            })?;
            // The pointer first moves over the editor, as a hand would: after
            // `typing` the window is in keyboard modality, which suppresses
            // hover, and a wheel event only scrolls the hovered element.
            self.win(cx, |window, cx| {
                window.dispatch_event(
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: Modifiers::default(),
                    }),
                    cx,
                )
            })?;
            self.settle(cx).await;
            let scroll_row = |cx: &mut AsyncApp| {
                cx.update(|cx| editor.upgrade().map(|editor| editor.read(cx).scroll_row()))
            };
            let executor = cx.background_executor().clone();
            let until = executor.now() + self.config.scroll_duration;
            let (mut ends, mut work) = (Vec::new(), Vec::new());
            let (mut events, mut without_frame, mut moved) = (0usize, 0usize, false);
            let mut direction = -1.0f32;
            let mut row = scroll_row(cx);
            // At most one event per millisecond: a frame never takes less,
            // and it bounds the loop where time does not move by itself (the
            // tests' fake clock, whose frames are synchronous).
            let max_events = self.config.scroll_duration.as_millis().max(1) as usize;
            while executor.now() < until && events < max_events {
                let event = ScrollWheelEvent {
                    position,
                    delta: ScrollDelta::Lines(point(0., direction)),
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                };
                let start = now_ns();
                self.win(cx, |window, cx| {
                    window.dispatch_event(PlatformInput::ScrollWheel(event), cx)
                })?;
                events += 1;
                match self
                    .frame_after(cx, start, false, self.config.scroll_frame_timeout)
                    .await
                {
                    Some(frame) => {
                        ends.push(frame.end_ns);
                        work.push(ms(frame.begin_ns, frame.paint_ns));
                    }
                    None => without_frame += 1,
                }
                let now_row = scroll_row(cx);
                if now_row == row {
                    // The end (or the top) of the file: scroll back.
                    direction = -direction;
                } else {
                    moved = true;
                }
                row = now_row;
            }
            if !moved {
                let size = self.win(cx, |window, _| window.viewport_size())?;
                return Err(format!(
                    "la rueda del mouse no movió el editor ({events} eventos en ({:.0}, {:.0}) \
                     de una ventana de {:.0}×{:.0}, fila {row:?})",
                    f32::from(position.x),
                    f32::from(position.y),
                    f32::from(size.width),
                    f32::from(size.height),
                ));
            }
            let intervals: Vec<f64> = ends.windows(2).map(|pair| ms(pair[0], pair[1])).collect();
            let mut line = file_info(&file);
            line["scenario"] = json!("scroll");
            line["events"] = json!(events);
            line["frames"] = json!(ends.len());
            line["events_without_frame"] = json!(without_frame);
            line["frame_interval_ms"] = stats(&intervals);
            line["intervals_over_33ms"] = json!(intervals.iter().filter(|&&gap| gap > 33.).count());
            line["frame_work_ms"] = stats(&work);
            lines.push(line);
        }
        Ok(lines)
    }

    async fn idle(&mut self, cx: &mut AsyncApp) -> BenchResult<Vec<Value>> {
        let (root, file) = match (self.plan.root.clone(), self.plan.file.clone()) {
            (Some(root), file) => (
                absolute(&root)?,
                file.map(|file| absolute(&file)).transpose()?,
            ),
            (None, Some(_)) => {
                let (root, files) = self.file_targets(cx).await?;
                (root, files.into_iter().next())
            }
            (None, None) => {
                let files = self.text_files(cx).await?;
                (files.root.clone(), Some(files.five_k.clone()))
            }
        };
        self.ensure_project(&root, cx).await?;
        let file = match file {
            Some(file) => Some(file),
            // `RUTA` without `--bench-file`: its first file, in tree order.
            None => self.project_files(cx)?.into_iter().next(),
        };
        if let Some(file) = &file {
            self.open_pinned(file, cx).await?;
        }
        self.sleep(cx, self.config.idle_settle).await;
        let rss_settled = own_vm_rss_mb();
        let (frames_before, cpu_before) = (frames_painted(), cpu_ms());
        self.sleep(cx, self.config.idle_window).await;
        let (frames_after, cpu_after) = (frames_painted(), cpu_ms());
        let cpu = match (cpu_before, cpu_after) {
            (Some(before), Some(after)) => Some(round3(after - before)),
            _ => None,
        };
        let window_ms = self.config.idle_window.as_secs_f64() * 1000.;
        Ok(vec![json!({
            "scenario": "idle",
            "file": file.as_deref().map(file_name),
            "settle_s": self.config.idle_settle.as_secs_f64(),
            "window_s": self.config.idle_window.as_secs_f64(),
            "frames_painted": frames_after - frames_before,
            "cpu_ms": cpu,
            "cpu_percent": cpu.map(|cpu| round3(cpu * 100. / window_ms)),
            "vm_rss_mb": rss_settled,
            "vm_rss_mb_at_end": own_vm_rss_mb(),
        })])
    }

    async fn finder(&mut self, cx: &mut AsyncApp) -> BenchResult<Vec<Value>> {
        let root = self.large_root(cx).await?;
        self.ensure_project(&root, cx).await?;
        let candidates = self.project_files(cx)?.len();
        let finder = self
            .ws(cx, |workspace, _, _| workspace.file_finder().cloned())?
            .ok_or("el proyecto no tiene buscador de archivos")?
            .downgrade();
        let (mut all, mut last, mut results) = (Vec::new(), Vec::new(), Vec::new());
        for query in FINDER_QUERIES {
            let is_open = |cx: &mut AsyncApp| {
                cx.update(|cx| finder.upgrade().is_some_and(|f| f.read(cx).is_open()))
            };
            if !is_open(cx) {
                self.ws(cx, |workspace, window, cx| {
                    workspace.toggle_file_finder(window, cx)
                })?;
            }
            if !is_open(cx) {
                return Err("Ctrl+P no abrió el buscador".to_string());
            }
            self.settle(cx).await;
            for (index, character) in query.char_indices() {
                let prefix = query[..index + character.len_utf8()].to_string();
                let key =
                    Keystroke::parse(&character.to_string()).map_err(|error| error.to_string())?;
                let watched = finder.clone();
                set_condition(move |cx| {
                    watched.upgrade().is_some_and(|finder| {
                        let finder = finder.read(cx);
                        !finder.is_filtering() && finder.query().read(cx).value() == prefix.as_str()
                    })
                });
                let start = now_ns();
                self.win(cx, |window, cx| window.dispatch_keystroke(key, cx))?;
                let frame = self
                    .frame_after(cx, start, true, self.config.frame_timeout)
                    .await;
                clear_condition();
                let frame = frame.ok_or_else(|| format!("sin resultados para «{query}»"))?;
                let sample = ms(start, frame.end_ns);
                all.push(sample);
                if index + character.len_utf8() == query.len() {
                    last.push(sample);
                }
            }
            let found = cx.update(|cx| {
                finder
                    .upgrade()
                    .map(|finder| finder.read(cx).results().len())
                    .unwrap_or(0)
            });
            results.push(json!({ "query": query, "results": found }));
            self.ws(cx, |workspace, window, cx| {
                if let Some(finder) = workspace.file_finder().cloned() {
                    finder.update(cx, |finder, cx| finder.dismiss(window, cx));
                }
            })?;
        }
        Ok(vec![json!({
            "scenario": "finder",
            "candidates": candidates,
            "keys": all.len(),
            "key_to_results_ms": stats(&all),
            "last_key_to_results_ms": stats(&last),
            "queries": results,
        })])
    }

    async fn sweep(&mut self, cx: &mut AsyncApp) -> BenchResult<Vec<Value>> {
        let root = self.large_root(cx).await?;
        self.ensure_project(&root, cx).await?;
        let files = self.project_files(cx)?;
        let files_in_project = files.len();
        // Text files of at most 256 KiB, in tree order.
        let mut originals = Vec::new();
        for path in files {
            if originals.len() == self.config.sweep_files {
                break;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if bytes.len() <= 256 * 1024 && std::str::from_utf8(&bytes).is_ok() {
                originals.push((path, bytes));
            }
        }
        if originals.is_empty() {
            return Err("el proyecto no tiene archivos de texto para cambiar".to_string());
        }
        let restore = Restore(originals);
        let review = self.review(cx)?;
        let monitor = GapMonitor::start(cx);

        // Photo.
        let start = now_ns();
        cx.update(|cx| review.update(cx, |review, cx| review.begin_prompt(cx)));
        let ready = self
            .poll_until(cx, self.config.work_timeout, |cx| {
                Ok(cx.update(|cx| review.read(cx).is_photo_ready()))
            })
            .await?;
        if !ready {
            return Err("la foto del proyecto no terminó".to_string());
        }
        let photo_ms = ms(start, now_ns());
        let photo_gap = monitor.take_max();

        // The agent's changes, on disk. The agent writes from a process of
        // its own, so the bench writes on the background executor: 200
        // writes on the main thread (12–17 ms) are not the editor's
        // blocking and used to count in the `watch` gap.
        let mut restore = restore;
        let originals = std::mem::take(&mut restore.0);
        let (originals, written) = cx
            .background_executor()
            .spawn(async move {
                let mut written = Ok(());
                for (path, bytes) in &originals {
                    let mut changed = bytes.clone();
                    changed.extend_from_slice(b"// cambio del banco de medicion\n");
                    if let Err(error) = std::fs::write(path, changed) {
                        written = Err(format!("no se pudo escribir {}: {error}", path.display()));
                        break;
                    }
                }
                (originals, written)
            })
            .await;
        restore.0 = originals;
        written?;
        self.sleep(cx, self.config.watch_settle).await;
        let watch_gap = monitor.take_max();

        // The end of the turn: the sweep.
        let start = now_ns();
        cx.update(|cx| review.update(cx, |review, cx| review.end_turn(cx)));
        let ended = self
            .poll_until(cx, self.config.work_timeout, |cx| {
                Ok(cx.update(|cx| !review.read(cx).is_turn_active()))
            })
            .await?;
        if !ended {
            return Err("el repaso final del turno no terminó".to_string());
        }
        let sweep_ms = ms(start, now_ns());
        let _ = self
            .frame_after(cx, now_ns(), false, self.config.frame_timeout)
            .await;
        let sweep_gap = monitor.take_max();
        monitor.stop();
        let pending_files = cx.update(|cx| review.read(cx).summary().borrow().files.len());
        let (watch, swept) = cx.update(|cx| {
            let review = review.read(cx);
            (review.last_watch(), review.last_sweep())
        });

        // Leave everything as it was.
        cx.update(|cx| review.update(cx, |review, cx| review.accept_all(cx)));
        let changed_files = restore.0.len();
        drop(restore);
        Ok(vec![json!({
            "scenario": "sweep",
            "files_in_project": files_in_project,
            "changed_files": changed_files,
            "photo_ms": photo_ms,
            "sweep_ms": sweep_ms,
            "pending_files": pending_files,
            "max_main_thread_gap_ms": {
                "photo": photo_gap,
                "watch": watch_gap,
                "sweep": sweep_gap,
            },
            // The review's own accounting of its main-thread slices.
            "watch_adoption": {
                "paths": watch.paths,
                "adopted": watch.adopted,
                "slices": watch.main_slices,
                "max_slice_ms": round3(watch.max_main_slice.as_secs_f64() * 1000.),
            },
            "sweep_max_slice_ms": swept
                .map(|stats| round3(stats.max_main_slice.as_secs_f64() * 1000.)),
        })])
    }

    /// One synthetic turn of `edits` changed lines in `file` (open in a
    /// tab): the time from the write on disk to the end of the first frame
    /// whose editor shows segments (or, for a file over the inline limits,
    /// to the end of the turn), and what the review made of it.
    async fn agent_edits(
        &self,
        file: &Path,
        edits: usize,
        cx: &mut AsyncApp,
    ) -> BenchResult<(WeakEntity<EditorView>, Value)> {
        let editor = self.open_pinned(file, cx).await?;
        let review = self.review(cx)?;
        cx.update(|cx| review.update(cx, |review, cx| review.begin_prompt(cx)));
        let ready = self
            .poll_until(cx, self.config.work_timeout, |cx| {
                Ok(cx.update(|cx| review.read(cx).is_photo_ready()))
            })
            .await?;
        if !ready {
            return Err("la foto del proyecto no terminó".to_string());
        }
        let text = std::fs::read_to_string(file)
            .map_err(|error| format!("no se pudo leer {}: {error}", file.display()))?;
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let step = (lines.len() / (edits + 1)).max(1);
        for edit in 1..=edits {
            if let Some(line) = lines.get_mut(edit * step) {
                line.push_str(&format!(" // cambio del agente {edit}"));
            }
        }
        let mut changed = lines.join("\n");
        changed.push('\n');

        let watched = editor.clone();
        set_condition(move |cx| {
            watched
                .upgrade()
                .is_some_and(|editor| !editor.read(cx).review().hunks.is_empty())
        });
        let start = now_ns();
        std::fs::write(file, changed)
            .map_err(|error| format!("no se pudo escribir {}: {error}", file.display()))?;
        cx.update(|cx| review.update(cx, |review, cx| review.end_turn(cx)));
        let ended = self
            .poll_until(cx, self.config.work_timeout, |cx| {
                Ok(cx.update(|cx| !review.read(cx).is_turn_active()))
            })
            .await?;
        if !ended {
            clear_condition();
            return Err("el turno sintético no terminó".to_string());
        }
        let turn_ended = now_ns();
        let file_level = cx.update(|cx| {
            review
                .read(cx)
                .summary()
                .borrow()
                .file(file)
                .map(|summary| summary.file_level)
        });
        let visible = if file_level == Some(false) {
            self.frame_after(cx, start, true, self.config.frame_timeout)
                .await
        } else {
            self.frame_after(cx, turn_ended, false, self.config.frame_timeout)
                .await
        };
        clear_condition();
        let visible = visible.ok_or("ningún cuadro mostró los cambios del agente")?;
        let inline_hunks = cx.update(|cx| {
            editor
                .upgrade()
                .map(|editor| editor.read(cx).review().hunks.len())
                .unwrap_or(0)
        });
        let mut line = file_info(file);
        line["scenario"] = json!("review-1mb");
        line["agent_edits"] = json!(edits);
        line["file_level"] = json!(file_level);
        line["inline_hunks"] = json!(inline_hunks);
        line["edits_to_frame_ms"] = json!(ms(start, visible.end_ns));
        Ok((editor, line))
    }

    async fn review_1mb(&mut self, cx: &mut AsyncApp) -> BenchResult<Vec<Value>> {
        let files = self.text_files(cx).await?;
        self.ensure_project(&files.root, cx).await?;
        let review = self.review(cx)?;
        let mut lines = Vec::new();

        // The 1 MB file: inline review, then typing with the segments pending.
        let (editor, mut line) = self
            .agent_edits(&files.one_mb, self.config.review_edits, cx)
            .await?;
        if line["inline_hunks"] == json!(0) {
            return Err("el archivo de 1 MB no mostró segmentos en línea".to_string());
        }
        let samples = self.type_keys(&editor, self.config.typing_keys, cx).await?;
        line["typing_with_pending_ms"] = stats(&samples);
        lines.push(line);
        cx.update(|cx| review.update(cx, |review, cx| review.accept_all(cx)));

        // The 50 000 line file: review by file only, no inline segments (M6).
        let (_, line) = self
            .agent_edits(&files.fifty_k, self.config.review_edits, cx)
            .await?;
        lines.push(line);
        cx.update(|cx| review.update(cx, |review, cx| review.accept_all(cx)));
        Ok(lines)
    }

    async fn demo(&mut self, cx: &mut AsyncApp) -> BenchResult<Vec<Value>> {
        let (root, turn) = match &self.plan.root {
            Some(root) => {
                let root = absolute(root)?;
                let candidates = [
                    Some(root.join("demo-turn.txt")),
                    root.parent().map(|parent| parent.join("demo-turn.txt")),
                ];
                let turn = candidates
                    .into_iter()
                    .flatten()
                    .find_map(|path| std::fs::read_to_string(path).ok())
                    .ok_or("no se encontró demo-turn.txt junto al proyecto")?;
                (root, turn)
            }
            None => corpus::demo_project(&self.work_dir).map_err(|error| {
                format!("no se pudo generar el proyecto de demostración: {error}")
            })?,
        };
        self.ensure_project(&root, cx).await?;
        let review = self.review(cx)?;
        cx.update(|cx| review.update(cx, |review, cx| review.begin_prompt(cx)));
        let ready = self
            .poll_until(cx, self.config.work_timeout, |cx| {
                Ok(cx.update(|cx| review.read(cx).is_photo_ready()))
            })
            .await?;
        if !ready {
            return Err("la foto del proyecto no terminó".to_string());
        }
        corpus::apply_shell_edits(&root, turn.trim())
            .map_err(|error| format!("no se pudo aplicar el turno de ejemplo: {error}"))?;
        self.sleep(cx, self.config.watch_settle).await;
        cx.update(|cx| review.update(cx, |review, cx| review.end_turn(cx)));
        let ended = self
            .poll_until(cx, self.config.work_timeout, |cx| {
                Ok(cx.update(|cx| !review.read(cx).is_turn_active()))
            })
            .await?;
        if !ended {
            return Err("el turno de ejemplo no terminó".to_string());
        }
        let (pending_files, pending, first) = cx.update(|cx| {
            let review = review.read(cx);
            let summary = review.summary();
            let summary = summary.borrow();
            (
                summary.files.len(),
                summary.pending,
                summary
                    .files
                    .iter()
                    .find(|(_, file)| !file.file_level)
                    .map(|(path, _)| path.clone()),
            )
        });
        if let Some(first) = &first {
            self.open_pinned(first, cx).await?;
        }
        Ok(vec![json!({
            "scenario": "demo",
            "ready": true,
            "pending_files": pending_files,
            "pending_changes": pending,
            "file": first.as_deref().map(file_name),
        })])
    }
}

/// Writes the original bytes of the files `sweep` changed back, however the
/// scenario ends.
struct Restore(Vec<(PathBuf, Vec<u8>)>);

impl Drop for Restore {
    fn drop(&mut self) {
        for (path, bytes) in &self.0 {
            if let Err(error) = std::fs::write(path, bytes) {
                tracing::error!(path = %path.display(), %error, "--bench: no se pudo restaurar el archivo");
            }
        }
    }
}

/// A main thread task that wakes up every millisecond and keeps the longest
/// gap between two of its runs: how long the main thread was blocked.
struct GapMonitor {
    state: Rc<RefCell<GapState>>,
    _task: Task<()>,
}

struct GapState {
    running: bool,
    last_ns: u64,
    max_ns: u64,
}

impl GapMonitor {
    fn start(cx: &mut AsyncApp) -> Self {
        let state = Rc::new(RefCell::new(GapState {
            running: true,
            last_ns: now_ns(),
            max_ns: 0,
        }));
        let watched = state.clone();
        let executor = cx.background_executor().clone();
        let task = cx.foreground_executor().spawn(async move {
            loop {
                executor.timer(Duration::from_millis(1)).await;
                let mut state = watched.borrow_mut();
                if !state.running {
                    break;
                }
                let now = now_ns();
                state.max_ns = state.max_ns.max(now.saturating_sub(state.last_ns));
                state.last_ns = now;
            }
        });
        Self { state, _task: task }
    }

    /// The longest gap since the last call, in ms (the pending one included).
    fn take_max(&self) -> f64 {
        let mut state = self.state.borrow_mut();
        let now = now_ns();
        let max = state.max_ns.max(now.saturating_sub(state.last_ns));
        state.max_ns = 0;
        state.last_ns = now;
        round3(max as f64 / 1_000_000.)
    }

    fn stop(&self) {
        self.state.borrow_mut().running = false;
    }
}

fn absolute(path: &Path) -> BenchResult<PathBuf> {
    std::path::absolute(path).map_err(|error| format!("ruta inválida {}: {error}", path.display()))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// `file`, `bytes` and `lines` of a measured file.
fn file_info(path: &Path) -> Value {
    let text = std::fs::read(path).unwrap_or_default();
    json!({
        "file": file_name(path),
        "bytes": text.len(),
        "lines": text.iter().filter(|&&byte| byte == b'\n').count(),
    })
}

/// `n`, `p50`, `p95` and `max` (nearest rank) of `samples`, in ms.
pub fn stats(samples: &[f64]) -> Value {
    if samples.is_empty() {
        return json!({ "n": 0, "p50": null, "p95": null, "max": null });
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({
        "n": sorted.len(),
        "p50": percentile(&sorted, 0.50),
        "p95": percentile(&sorted, 0.95),
        "max": sorted[sorted.len() - 1],
    })
}

/// Nearest-rank percentile of an ascending, non-empty slice.
pub fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    let rank = (quantile * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}
