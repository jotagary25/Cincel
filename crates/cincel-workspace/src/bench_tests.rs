//! Tests of `cincel --bench` (`crate::bench`,
//! `docs/specs/08-etapa6-cierre-1-0.md` §3.10: "cada escenario corre en un
//! proyecto temporal chico y devuelve los campos esperados (sin juzgar
//! tiempos)").
//!
//! The GPUI half runs every scenario through [`bench::run`] in a
//! `TestAppContext` window with [`BenchConfig::tiny`]: the scenarios wait on
//! the executor's timers, which `run_until_parked` advances, so a whole run
//! finishes inside one call. The pure half covers the parsers, the
//! percentiles and the generated corpus.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use cincel_settings::Config;
use gpui::TestAppContext;
use serde_json::Value;

use crate::bench::{self, BenchConfig, BenchPlan, Mark, Outcome, Scenario, corpus};
use crate::project::ProjectOptions;
use crate::test_support::isolate_state;
use crate::workspace::{Workspace, WorkspaceOptions};

// ------------------------------------------------------------- pure tests

#[test]
fn scenario_names_round_trip_and_all_is_the_eight_measurements() {
    for scenario in Scenario::ALL {
        assert_eq!(Scenario::from_name(scenario.name()), Some(scenario));
    }
    assert_eq!(Scenario::from_name("nope"), None);
    let names: Vec<&str> = Scenario::MEASUREMENTS.iter().map(|s| s.name()).collect();
    assert_eq!(
        names,
        [
            "startup",
            "open",
            "typing",
            "scroll",
            "idle",
            "finder",
            "sweep",
            "review-1mb"
        ]
    );
}

#[test]
fn parses_proc_stat_even_with_spaces_in_the_command_name() {
    // Fields 14, 15 and 22 are 7, 3 and 12345; the name has a space and a
    // parenthesis.
    let stat = "4242 (cincel (x) y) S 1 4242 4242 0 -1 4194304 100 0 0 0 7 3 0 0 20 0 9 0 12345 \
                1000000 500 18446744073709551615";
    let fields = bench::parse_stat(stat).expect("se entiende");
    assert_eq!(fields.utime, 7);
    assert_eq!(fields.stime, 3);
    assert_eq!(fields.starttime, 12345);
    assert!(bench::parse_stat("sin paréntesis").is_none());
    // The real one parses too.
    let own = std::fs::read_to_string("/proc/self/stat").unwrap();
    assert!(bench::parse_stat(&own).is_some());
}

#[test]
fn parses_vm_rss() {
    let status = "Name:\tcincel\nVmPeak:\t  900 kB\nVmRSS:\t  123456 kB\nThreads:\t9\n";
    assert_eq!(bench::parse_vm_rss_kb(status), Some(123_456));
    assert_eq!(bench::parse_vm_rss_kb("Name:\tx\n"), None);
}

#[test]
fn the_process_start_is_before_now() {
    let start = bench::process_start().expect("hay /proc/self/stat");
    assert!(start.ns <= bench::now_ns());
    assert!(start.resolution_ms >= 0.);
}

#[test]
fn percentiles_use_the_nearest_rank() {
    let samples: Vec<f64> = (1..=20).map(f64::from).collect();
    assert_eq!(bench::percentile(&samples, 0.5), 10.);
    assert_eq!(bench::percentile(&samples, 0.95), 19.);
    assert_eq!(bench::percentile(&[4.], 0.95), 4.);
    let stats = bench::stats(&[3., 1., 2.]);
    assert_eq!(stats["n"], 3);
    assert_eq!(stats["p50"], 2.);
    assert_eq!(stats["max"], 3.);
    assert!(bench::stats(&[])["p50"].is_null());
}

#[test]
fn the_text_files_have_the_sizes_of_the_spec_and_are_deterministic() {
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let sizes = corpus::CorpusSizes::default();
    let a = corpus::text_files(one.path(), &sizes).unwrap();
    let b = corpus::text_files(two.path(), &sizes).unwrap();
    let lines = |path: &Path| std::fs::read_to_string(path).unwrap().lines().count();
    assert_eq!(lines(&a.five_k), 5_000);
    assert_eq!(lines(&a.fifty_k), 50_000);
    let one_mb = std::fs::metadata(&a.one_mb).unwrap().len();
    assert!(
        (1024 * 1024..1024 * 1024 + 200).contains(&one_mb),
        "{one_mb}"
    );
    // Over the inline review limit (`review.max_file_size_kb` = 2048).
    let fifty_k = std::fs::metadata(&a.fifty_k).unwrap().len();
    assert!(fifty_k > 2048 * 1024, "{fifty_k}");
    for (left, right) in a.all().iter().zip(b.all()) {
        assert_eq!(std::fs::read(left).unwrap(), std::fs::read(right).unwrap());
    }
}

#[test]
fn the_large_project_has_the_requested_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = corpus::large_project(dir.path(), 25, 4).unwrap();
    let count = walk(&root).len();
    // 25 files plus the README.
    assert_eq!(count, 26);
    assert!(root.join("modulo_000/main_00000.rs").is_file());
    assert!(root.join("modulo_003/handler_00003.rs").is_file());
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(walk(&path));
        } else {
            found.push(path);
        }
    }
    found
}

#[test]
fn shell_edits_overwrite_create_delete_and_rename() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "viejo").unwrap();
    std::fs::write(dir.path().join("b.txt"), "b").unwrap();
    std::fs::write(dir.path().join("c.txt"), "c").unwrap();
    corpus::apply_shell_edits(
        dir.path(),
        "a.txt=uno\\ndos;nueva/d.txt=@x\\ty;b.txt=;c.txt=>e.txt",
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "uno\ndos"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("nueva/d.txt")).unwrap(),
        "x\ty"
    );
    assert!(!dir.path().join("b.txt").exists());
    assert!(!dir.path().join("c.txt").exists());
    assert!(dir.path().join("e.txt").is_file());
}

#[test]
fn the_demo_turn_changes_three_files_and_creates_one() {
    let dir = tempfile::tempdir().unwrap();
    let (root, turn) = corpus::demo_project(dir.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("demo-turn.txt")).unwrap(),
        turn
    );
    // Eight files and a README.
    assert_eq!(walk(&root).len(), 9);
    let before: Vec<(std::path::PathBuf, Vec<u8>)> = walk(&root)
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let touched = corpus::apply_shell_edits(&root, &turn).unwrap();
    assert_eq!(touched.len(), 4, "{touched:?}");
    let changed = before
        .iter()
        .filter(|(path, bytes)| std::fs::read(path).unwrap() != *bytes)
        .count();
    assert_eq!(changed, 3);
    assert!(root.join("src/historial.rs").is_file());
    let error = std::fs::read_to_string(root.join("src/error.rs")).unwrap();
    assert!(error.contains("Desborde"), "{error}");
}

#[test]
fn without_a_bench_the_hooks_do_nothing() {
    bench::deactivate();
    assert!(!bench::is_active());
    assert!(bench::frame_probe().is_none());
    bench::frame_begin();
    bench::mark(Mark::Main);
    assert_eq!(bench::frames_painted(), 0);
}

// ------------------------------------------------------------- GPUI tests

/// What one run printed and how it ended.
struct Run {
    lines: Vec<Value>,
    outcome: Outcome,
    /// The bench's working directory (the generated corpus).
    work: tempfile::TempDir,
}

impl Run {
    fn of(&self, scenario: &str) -> Vec<&Value> {
        self.lines
            .iter()
            .filter(|line| line["scenario"] == scenario)
            .collect()
    }

    fn assert_no_errors(&self) {
        for line in &self.lines {
            assert!(line.get("error").is_none(), "{line}");
        }
        assert_eq!(self.outcome, Outcome::Done { ok: true }, "{:?}", self.lines);
    }
}

fn plan(scenario: Scenario, root: Option<&Path>) -> BenchPlan {
    BenchPlan {
        scenario,
        root: root.map(Path::to_path_buf),
        file: None,
        smoke_test: false,
    }
}

/// Runs `plan` in a fresh window (with `project` open, if any) and collects
/// its lines. With `startup_marks`, the binary's marks are simulated around
/// the window.
fn run_bench(
    plan: BenchPlan,
    project: Option<&Path>,
    startup_marks: bool,
    cx: &mut TestAppContext,
) -> Run {
    isolate_state();
    bench::deactivate();
    bench::activate();
    cx.update(|cx| crate::init(Config::default(), cx));
    if startup_marks {
        bench::mark(Mark::Main);
        bench::mark(Mark::Config);
    }
    let options = WorkspaceOptions {
        project: project.map(Path::to_path_buf),
        project_options: ProjectOptions::inert(),
        ..WorkspaceOptions::default()
    };
    let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(options, window, cx));
    cx.run_until_parked();
    if startup_marks {
        bench::mark(Mark::WindowOpen);
        cx.update(|window, _| window.refresh());
    }

    let work = tempfile::tempdir().unwrap();
    let lines = Rc::new(RefCell::new(Vec::new()));
    let outcome = Rc::new(RefCell::new(None));
    let sink: bench::Sink = {
        let lines = lines.clone();
        Rc::new(move |line: &Value| lines.borrow_mut().push(line.clone()))
    };
    let work_dir = work.path().to_path_buf();
    cx.update(|window, cx| {
        let task = bench::run(
            plan,
            BenchConfig::tiny(),
            work_dir,
            window.window_handle(),
            workspace.downgrade(),
            sink,
            cx,
        );
        let outcome = outcome.clone();
        cx.spawn(async move |_| {
            let result = task.await;
            *outcome.borrow_mut() = Some(result);
        })
        .detach();
    });
    // The scenarios wait on the executor's timers: move the fake clock until
    // the run ends (a bound of ten fake minutes, far above any tiny run).
    for _ in 0..60_000 {
        cx.run_until_parked();
        if outcome.borrow().is_some() {
            break;
        }
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(10));
    }
    let outcome = outcome
        .borrow_mut()
        .take()
        .unwrap_or_else(|| panic!("el banco no terminó: {:?}", lines.borrow()));
    bench::deactivate();
    let lines = lines.borrow().clone();
    Run {
        lines,
        outcome,
        work,
    }
}

fn number(line: &Value, key: &str) -> f64 {
    line[key]
        .as_f64()
        .unwrap_or_else(|| panic!("{key} no es un número en {line}"))
}

fn assert_stats(line: &Value, key: &str, n: u64) {
    let stats = &line[key];
    assert_eq!(stats["n"], n, "{key} en {line}");
    for field in ["p50", "p95", "max"] {
        assert!(stats[field].is_f64(), "{key}.{field} en {line}");
    }
}

#[gpui::test]
fn startup_reports_every_mark(cx: &mut TestAppContext) {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("a.rs"), "fn a() {}\n").unwrap();
    let run = run_bench(
        plan(Scenario::Startup, None),
        Some(project.path()),
        true,
        cx,
    );
    run.assert_no_errors();
    let lines = run.of("startup");
    assert_eq!(lines.len(), 1);
    let line = lines[0];
    for key in [
        "process_start_to_main_ms",
        "main_to_config_ms",
        "config_to_window_open_ms",
        "window_open_to_first_frame_ms",
        "total_ms",
        "t0_resolution_ms",
    ] {
        assert!(number(line, key) >= 0., "{key}");
    }
    assert_eq!(line["t0_source"], "proc_stat");
    assert_eq!(line["frame_end"], "post_present_task");
}

#[gpui::test]
fn open_measures_each_generated_file(cx: &mut TestAppContext) {
    let run = run_bench(plan(Scenario::Open, None), None, false, cx);
    run.assert_no_errors();
    let lines = run.of("open");
    let files: Vec<&str> = lines
        .iter()
        .map(|line| line["file"].as_str().unwrap())
        .collect();
    assert_eq!(files, ["cinco-mil.rs", "un-mega.rs", "cincuenta-mil.rs"]);
    for line in lines {
        assert_eq!(line["repeats"], 2);
        assert_stats(line, "open_to_frame_ms", 2);
        assert!(number(line, "bytes") > 0.);
        assert!(number(line, "lines") > 0.);
    }
    // The corpus lives in the bench's own directory.
    assert!(run.work.path().join("archivos/cinco-mil.rs").is_file());
}

#[gpui::test]
fn typing_leaves_the_files_as_they_were(cx: &mut TestAppContext) {
    let run = run_bench(plan(Scenario::Typing, None), None, false, cx);
    run.assert_no_errors();
    let lines = run.of("typing");
    assert_eq!(lines.len(), 3);
    for line in lines {
        assert_eq!(line["keys"], 6);
        assert_stats(line, "key_to_frame_ms", 6);
    }
}

#[gpui::test]
fn a_bench_file_replaces_the_generated_ones(cx: &mut TestAppContext) {
    let project = tempfile::tempdir().unwrap();
    let file = project.path().join("propio.rs");
    let text: String = (0..300).map(|i| format!("let x{i} = {i};\n")).collect();
    std::fs::write(&file, &text).unwrap();
    let plan = BenchPlan {
        file: Some(file.clone()),
        ..plan(Scenario::Typing, Some(project.path()))
    };
    let run = run_bench(plan, None, false, cx);
    run.assert_no_errors();
    let lines = run.of("typing");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["file"], "propio.rs");
    // Letters and backspaces cancel out; nothing was saved either way.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
}

#[gpui::test]
fn scroll_reports_frame_intervals(cx: &mut TestAppContext) {
    let run = run_bench(plan(Scenario::Scroll, None), None, false, cx);
    run.assert_no_errors();
    let lines = run.of("scroll");
    assert_eq!(lines.len(), 3);
    for line in lines {
        assert!(number(line, "events") >= 1.);
        assert!(number(line, "frames") >= 1.);
        assert!(line["frame_interval_ms"]["n"].is_u64());
        assert!(line["frame_work_ms"]["n"].as_u64().unwrap() >= 1);
        assert!(line["intervals_over_33ms"].is_u64());
        assert!(line["events_without_frame"].is_u64());
    }
}

#[gpui::test]
fn idle_reports_frames_cpu_and_memory(cx: &mut TestAppContext) {
    let run = run_bench(plan(Scenario::Idle, None), None, false, cx);
    run.assert_no_errors();
    let lines = run.of("idle");
    assert_eq!(lines.len(), 1);
    let line = lines[0];
    assert_eq!(line["file"], "cinco-mil.rs");
    assert!(line["frames_painted"].is_u64());
    assert!(number(line, "cpu_ms") >= 0.);
    assert!(number(line, "cpu_percent") >= 0.);
    assert!(number(line, "vm_rss_mb") > 0.);
    assert!(number(line, "vm_rss_mb_at_end") > 0.);
}

#[gpui::test]
fn finder_types_five_queries(cx: &mut TestAppContext) {
    let run = run_bench(plan(Scenario::Finder, None), None, false, cx);
    run.assert_no_errors();
    let lines = run.of("finder");
    assert_eq!(lines.len(), 1);
    let line = lines[0];
    // `BenchConfig::tiny`: 40 files plus the README.
    assert_eq!(line["candidates"], 41);
    let keys: usize = bench::FINDER_QUERIES.iter().map(|query| query.len()).sum();
    assert_eq!(line["keys"], keys);
    assert_stats(line, "key_to_results_ms", keys as u64);
    assert_stats(line, "last_key_to_results_ms", 5);
    let queries = line["queries"].as_array().unwrap();
    assert_eq!(queries.len(), 5);
    // "main" names a quarter of the generated files.
    assert!(queries[1]["results"].as_u64().unwrap() > 0, "{line}");
}

#[gpui::test]
fn sweep_finds_the_changes_and_restores_the_files(cx: &mut TestAppContext) {
    let project = tempfile::tempdir().unwrap();
    let root = corpus::large_project(project.path(), 12, 3).unwrap();
    let before: Vec<(std::path::PathBuf, Vec<u8>)> = walk(&root)
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let run = run_bench(plan(Scenario::Sweep, Some(&root)), None, false, cx);
    run.assert_no_errors();
    let lines = run.of("sweep");
    assert_eq!(lines.len(), 1);
    let line = lines[0];
    assert_eq!(line["files_in_project"], 13);
    assert_eq!(line["changed_files"], 5);
    assert_eq!(line["pending_files"], 5, "{line}");
    assert!(number(line, "photo_ms") >= 0.);
    assert!(number(line, "sweep_ms") >= 0.);
    for phase in ["photo", "watch", "sweep"] {
        assert!(line["max_main_thread_gap_ms"][phase].is_f64(), "{phase}");
    }
    for (path, bytes) in before {
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{}", path.display());
    }
}

#[gpui::test]
fn review_1mb_shows_segments_and_types_with_them_pending(cx: &mut TestAppContext) {
    let run = run_bench(plan(Scenario::Review1Mb, None), None, false, cx);
    run.assert_no_errors();
    let lines = run.of("review-1mb");
    assert_eq!(lines.len(), 2);
    let one_mb = lines[0];
    assert_eq!(one_mb["file"], "un-mega.rs");
    assert_eq!(one_mb["agent_edits"], 3);
    assert_eq!(one_mb["file_level"], false);
    assert_eq!(one_mb["inline_hunks"], 3, "{one_mb}");
    assert!(number(one_mb, "edits_to_frame_ms") >= 0.);
    assert_stats(one_mb, "typing_with_pending_ms", 6);
    let fifty_k = lines[1];
    assert_eq!(fifty_k["file"], "cincuenta-mil.rs");
    assert!(fifty_k["inline_hunks"].is_u64());
    assert!(fifty_k["file_level"].is_boolean(), "{fifty_k}");
}

#[gpui::test]
fn demo_leaves_the_sample_turn_pending(cx: &mut TestAppContext) {
    let run = run_bench(plan(Scenario::Demo, None), None, false, cx);
    assert_eq!(run.outcome, Outcome::KeepOpen, "{:?}", run.lines);
    let lines = run.of("demo");
    assert_eq!(lines.len(), 1);
    let line = lines[0];
    assert_eq!(line["ready"], true);
    assert_eq!(line["pending_files"], 4, "{line}");
    assert!(number(line, "pending_changes") >= 4.);
    assert!(line["file"].is_string());
}

#[gpui::test]
fn all_runs_the_eight_measurements_in_order(cx: &mut TestAppContext) {
    let run = run_bench(plan(Scenario::All, None), None, true, cx);
    run.assert_no_errors();
    let mut order: Vec<&str> = Vec::new();
    for line in &run.lines {
        let name = line["scenario"].as_str().unwrap();
        if order.last() != Some(&name) {
            order.push(name);
        }
    }
    let expected: Vec<&str> = Scenario::MEASUREMENTS.iter().map(|s| s.name()).collect();
    assert_eq!(order, expected);
}

#[gpui::test]
fn a_scenario_that_cannot_run_reports_why(cx: &mut TestAppContext) {
    let missing = tempfile::tempdir().unwrap();
    let plan = BenchPlan {
        file: Some(missing.path().join("no-existe.rs")),
        ..plan(Scenario::Open, None)
    };
    let run = run_bench(plan, None, false, cx);
    assert_eq!(run.outcome, Outcome::Done { ok: false });
    assert_eq!(run.lines.len(), 1);
    assert_eq!(run.lines[0]["scenario"], "open");
    assert!(
        run.lines[0]["error"]
            .as_str()
            .unwrap()
            .contains("no existe")
    );
}
