//! Unit tests of `cincel-perf` (spec 08 §3.10).

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::corpus::{self, Options, Rng, Scale};
use crate::frames::{
    CONTENT_COLORS, ContentDetector, Frame, Image, Rect, first_damage_after, intervals_ms,
    stable_frame,
};
use crate::keys::{MASK_CTRL, MASK_SHIFT, char_stroke, parse_combo};
use crate::procfs;
use crate::profile::{self, AppKind};
use crate::report;
use crate::stats::{percentile, summarize};
use crate::{Cmd, parse};

const SMALL: Scale = Scale {
    medium_files: 60,
    medium_dirs: 7,
    medium_lines: (5, 40),
    medium_pngs: 2,
    large_files: 30,
    large_dirs: 4,
    large_text_bytes: (200, 2_000),
    large_binaries: 3,
    large_binary_bytes: (1_000, 5_000),
    five_thousand_lines: 500,
    one_megabyte_bytes: 20_000,
    fifty_thousand_lines: 900,
};

fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

// ---------------------------------------------------------------- corpus

#[test]
fn corpus_is_deterministic() {
    let git = git_available();
    let options = Options {
        scale: SMALL,
        large: true,
        demo: true,
        git,
    };
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let a = corpus::generate(first.path(), &options).unwrap();
    let b = corpus::generate(second.path(), &options).unwrap();
    assert_eq!(a, b, "dos corridas deben dar los mismos hashes");
    let names: Vec<&str> = a.iter().map(|part| part.name).collect();
    assert_eq!(names, ["archivos", "medio", "grande", "demo"]);
    let medium = &a[1];
    // 60 sources + 2 PNG + 3 big files + README.
    assert_eq!(medium.files, 60 + 2 + 3 + 1);
    if git {
        let commit = medium.commit.as_deref().unwrap();
        assert_eq!(commit.len(), 40);
        assert!(a[3].commit.is_some());
    }
    assert_eq!(a[2].files, 30);
    assert_eq!(
        fs::read_to_string(first.path().join("demo-turn.txt")).unwrap(),
        fs::read_to_string(second.path().join("demo-turn.txt")).unwrap()
    );
    // Regenerating over an existing corpus gives the same result again.
    let again = corpus::generate(first.path(), &options).unwrap();
    assert_eq!(again, a);
}

#[test]
fn corpus_big_files_have_the_requested_size() {
    let mut rng = Rng::new(corpus::SEED);
    let five = corpus::rust_lines(&mut rng, 5_000);
    assert_eq!(five.lines().count(), 5_000);
    let fifty = corpus::rust_lines(&mut rng, 50_000);
    assert_eq!(fifty.lines().count(), 50_000);
    // ≈ 2 MB (§3.4).
    assert!(
        (1_500_000..2_600_000).contains(&fifty.len()),
        "{}",
        fifty.len()
    );
    let mega = corpus::rust_bytes(&mut rng, 1_000_000);
    assert!(mega.len() >= 1_000_000 && mega.len() < 1_010_000);
    let lines = mega.lines().count();
    assert!((18_000..32_000).contains(&lines), "{lines} líneas");
}

#[test]
fn corpus_files_are_separate_parts_with_the_three_names() {
    let dir = tempfile::tempdir().unwrap();
    let options = Options {
        scale: SMALL,
        large: false,
        demo: false,
        git: false,
    };
    let parts = corpus::generate(dir.path(), &options).unwrap();
    assert_eq!(parts.len(), 2);
    for name in [
        corpus::FIVE_THOUSAND,
        corpus::ONE_MEGABYTE,
        corpus::FIFTY_THOUSAND,
    ] {
        let original = fs::read(dir.path().join("archivos").join(name)).unwrap();
        let copy = fs::read(dir.path().join("medio/bench").join(name)).unwrap();
        assert_eq!(original, copy, "{name} se copia igual dentro de medio/");
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("archivos").join(corpus::FIVE_THOUSAND))
            .unwrap()
            .lines()
            .count(),
        500
    );
    assert!(!dir.path().join("grande").exists());
}

#[test]
fn png_is_well_formed() {
    let image = corpus::png(3, 2, |x, y| [x as u8, y as u8, 7]);
    assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&image[12..16], b"IHDR");
    assert_eq!(u32::from_be_bytes(image[16..20].try_into().unwrap()), 3);
    assert_eq!(u32::from_be_bytes(image[20..24].try_into().unwrap()), 2);
    // The IEND chunk: empty, with its well-known CRC.
    assert_eq!(&image[image.len() - 12..image.len() - 8], &[0, 0, 0, 0]);
    assert_eq!(&image[image.len() - 8..image.len() - 4], b"IEND");
    assert_eq!(&image[image.len() - 4..], &[0xAE, 0x42, 0x60, 0x82]);
}

#[test]
fn demo_turn_follows_the_fake_shell_edits_format() {
    let turn = corpus::demo_turn();
    let items: Vec<&str> = turn.split(';').collect();
    assert_eq!(items.len(), 4, "3 modificados y 1 creado, separados por ;");
    let created: Vec<&str> = items
        .iter()
        .filter_map(|item| item.split_once('=').map(|(_, value)| value))
        .filter(|value| value.starts_with('@'))
        .collect();
    assert_eq!(created.len(), 1);
    for item in &items {
        let (path, value) = item.split_once('=').unwrap();
        assert!(!value.is_empty());
        let exists = corpus::DEMO_FILES.iter().any(|(name, _)| *name == path);
        assert_eq!(exists, !value.starts_with('@'), "{path}");
    }
    assert_eq!(
        corpus::DEMO_FILES.len(),
        8 + 1,
        "8 archivos de código y un README"
    );
}

// ---------------------------------------------------------------- stats

#[test]
fn percentiles_interpolate_between_ranks() {
    let samples: Vec<f64> = (1..=10).map(f64::from).collect();
    assert_eq!(percentile(&samples, 50.0), Some(5.5));
    assert_eq!(percentile(&samples, 0.0), Some(1.0));
    assert_eq!(percentile(&samples, 100.0), Some(10.0));
    assert!((percentile(&samples, 95.0).unwrap() - 9.55).abs() < 1e-9);
    assert!((percentile(&samples, 90.0).unwrap() - 9.1).abs() < 1e-9);
    assert_eq!(percentile(&[], 50.0), None);
    assert_eq!(percentile(&[f64::NAN, 3.0], 50.0), Some(3.0));
    let summary = summarize(&[4.0, 1.0, 3.0, 2.0]).unwrap();
    assert_eq!(summary.count, 4);
    assert_eq!(summary.p50, 2.5);
    assert_eq!(summary.max, 4.0);
    assert!(summarize(&[]).is_none());
}

// ---------------------------------------------------------------- frames

struct Canvas {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Canvas {
    fn new(width: u32, height: u32, color: [u8; 3]) -> Self {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            data.extend_from_slice(&[color[0], color[1], color[2], 0xFF]);
        }
        Canvas {
            width,
            height,
            data,
        }
    }

    fn fill(&mut self, rect: Rect, color: [u8; 3]) {
        for y in rect.y..rect.y + rect.height {
            for x in rect.x..rect.x + rect.width {
                let offset = ((y as u32 * self.width + x as u32) * 4) as usize;
                self.data[offset..offset + 3].copy_from_slice(&color);
            }
        }
    }

    fn image(&self) -> Image<'_> {
        Image {
            width: self.width,
            height: self.height,
            stride: self.width * 4,
            data: &self.data,
        }
    }

    fn region(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: self.width as i32,
            height: self.height as i32,
        }
    }
}

/// Paints `rect` with many shades, the way antialiased text looks.
fn paint_text(canvas: &mut Canvas, rect: Rect) {
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            let shade = 120 + ((x * 7 + y * 13) % 120) as u8;
            let offset = ((y as u32 * canvas.width + x as u32) * 4) as usize;
            canvas.data[offset..offset + 3].copy_from_slice(&[
                shade,
                shade,
                shade.saturating_add(10),
            ]);
        }
    }
}

#[test]
fn first_content_needs_two_percent_different_from_the_background() {
    let desktop = [64, 64, 64];
    let background = [30, 30, 36];
    let mut detector = ContentDetector::new(Some(desktop));
    let empty = Canvas::new(200, 100, desktop);
    let region = empty.region();

    // Mapped but not drawn yet: the empty desktop is skipped.
    let verdict = detector.feed(&empty.image(), &region);
    assert!(!verdict.drawn && !verdict.content);
    assert_eq!(detector.background(), None);

    // First frame that shows the window: plain background, no content.
    let mut canvas = Canvas::new(200, 100, background);
    let verdict = detector.feed(&canvas.image(), &region);
    assert!(verdict.drawn);
    assert_eq!(verdict.fraction, 0.0);
    assert!(!verdict.content);
    assert_eq!(detector.background(), Some(background));

    // Rounding noise within the tolerance does not count.
    canvas.fill(
        Rect {
            x: 0,
            y: 0,
            width: 200,
            height: 50,
        },
        [32, 31, 34],
    );
    let verdict = detector.feed(&canvas.image(), &region);
    assert_eq!(verdict.fraction, 0.0);
    assert!(!verdict.content);

    // A dark-theme skeleton: panels a few shades off, different but flat.
    let mut canvas = Canvas::new(200, 100, background);
    canvas.fill(
        Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 100,
        },
        [24, 24, 28],
    );
    canvas.fill(
        Rect {
            x: 160,
            y: 0,
            width: 40,
            height: 100,
        },
        [24, 24, 28],
    );
    let verdict = detector.feed(&canvas.image(), &region);
    assert!(verdict.fraction > 0.3, "{}", verdict.fraction);
    assert!(!verdict.content);

    // A flat placeholder over half the window: different, but few colors.
    let mut canvas = Canvas::new(200, 100, background);
    canvas.fill(
        Rect {
            x: 0,
            y: 0,
            width: 140,
            height: 90,
        },
        [255, 255, 255],
    );
    let verdict = detector.feed(&canvas.image(), &region);
    assert!(verdict.fraction > 0.5);
    assert!(verdict.colors < CONTENT_COLORS);
    assert!(!verdict.content);

    // 1 % of the window with text: not enough.
    let mut canvas = Canvas::new(200, 100, background);
    paint_text(
        &mut canvas,
        Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 10,
        },
    );
    let verdict = detector.feed(&canvas.image(), &region);
    assert!(verdict.fraction < 0.02, "{}", verdict.fraction);
    assert!(!verdict.content);

    // 5 % of the window with text: content.
    paint_text(
        &mut canvas,
        Rect {
            x: 0,
            y: 0,
            width: 200,
            height: 5,
        },
    );
    let verdict = detector.feed(&canvas.image(), &region);
    assert!(verdict.fraction >= 0.02, "{}", verdict.fraction);
    assert!(verdict.colors >= CONTENT_COLORS, "{}", verdict.colors);
    assert!(verdict.content);
}

#[test]
fn first_drawn_frame_can_already_have_content() {
    // An app whose first buffer is fully painted: the dominant color is its
    // background and the rest (panels, text) is content.
    let mut canvas = Canvas::new(160, 90, [250, 250, 250]);
    paint_text(
        &mut canvas,
        Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 90,
        },
    );
    let mut detector = ContentDetector::new(Some([64, 64, 64]));
    let verdict = detector.feed(&canvas.image(), &canvas.region());
    assert!(verdict.drawn && verdict.content);
    assert!(
        (verdict.fraction - 0.25).abs() < 0.05,
        "{}",
        verdict.fraction
    );
    assert_eq!(detector.background(), Some([250, 250, 250]));
    // Without a known desktop color the first frame is taken as drawn.
    let verdict = ContentDetector::new(None).feed(&canvas.image(), &canvas.region());
    assert!(verdict.content);
}

fn frame(ms: u64) -> Frame {
    Frame {
        ts_ns: ms * 1_000_000,
        damage: vec![Rect {
            x: 10,
            y: 10,
            width: 5,
            height: 5,
        }],
    }
}

const WINDOW: Rect = Rect {
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
};
const MS: u64 = 1_000_000;

#[test]
fn stable_frame_is_the_first_followed_by_100_ms_of_quiet() {
    // Enter at 1000 ms; frames at 1010, 1030, 1060 (burst), then 1300.
    let frames = vec![
        frame(990),
        frame(1010),
        frame(1030),
        frame(1060),
        frame(1300),
    ];
    let stable = stable_frame(&frames, &WINDOW, 1000 * MS, 100 * MS, 2000 * MS).unwrap();
    assert_eq!(stable.ts_ns, 1060 * MS);

    // The last frame is stable only if the observation lasted 100 ms more.
    let frames = vec![frame(1010), frame(1050)];
    assert!(stable_frame(&frames, &WINDOW, 1000 * MS, 100 * MS, 1100 * MS).is_none());
    let stable = stable_frame(&frames, &WINDOW, 1000 * MS, 100 * MS, 1150 * MS).unwrap();
    assert_eq!(stable.ts_ns, 1050 * MS);

    // Frames before the mark do not count; no frames → none.
    assert!(stable_frame(&[frame(900)], &WINDOW, 1000 * MS, 100 * MS, 5000 * MS).is_none());
}

#[test]
fn damage_outside_the_window_is_ignored() {
    let small = Rect {
        x: 0,
        y: 0,
        width: 5,
        height: 5,
    };
    let frames = vec![frame(1010), frame(1020)];
    assert!(first_damage_after(&frames, &small, 1000 * MS).is_none());
    assert_eq!(
        first_damage_after(&frames, &WINDOW, 1000 * MS)
            .unwrap()
            .ts_ns,
        1010 * MS
    );
    // No damage list at all counts as the whole output.
    let whole = vec![Frame {
        ts_ns: 1005 * MS,
        damage: Vec::new(),
    }];
    assert!(first_damage_after(&whole, &small, 1000 * MS).is_some());
    let intervals = intervals_ms(
        &[frame(0), frame(4), frame(12), frame(40)],
        &WINDOW,
        0,
        30 * MS,
    );
    assert_eq!(intervals, vec![4.0, 8.0]);
}

// ---------------------------------------------------------------- /proc

fn fake_process(
    root: &Path,
    pid: u32,
    ppid: u32,
    name: &str,
    ticks: (u64, u64),
    rss: u64,
    pss: u64,
) {
    let dir = root.join(pid.to_string());
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("stat"),
        format!(
            "{pid} ({name}) S {ppid} {pid} {pid} 0 -1 4194560 100 0 0 0 {} {} 0 0 20 0 1 0 12345 0 0\n",
            ticks.0, ticks.1
        ),
    )
    .unwrap();
    fs::write(
        dir.join("status"),
        format!("Name:\t{name}\nPid:\t{pid}\nPPid:\t{ppid}\nVmRSS:\t  {rss} kB\n"),
    )
    .unwrap();
    fs::write(
        dir.join("smaps_rollup"),
        format!(
            "5581e0a2b000-7ffd9e1d6000 ---p 00000000 00:00 0    [rollup]\nRss:              {rss} kB\nPss:              {pss} kB\nPss_Anon:          10 kB\n"
        ),
    )
    .unwrap();
}

#[test]
fn memory_sums_rss_and_pss_over_the_tree() {
    let proc_dir = tempfile::tempdir().unwrap();
    let root = proc_dir.path();
    fake_process(root, 100, 1, "antigravity", (50, 10), 200_000, 150_000);
    // A command name with spaces and parentheses.
    fake_process(root, 101, 100, "Web Content (x)", (5, 5), 80_000, 30_000);
    fake_process(root, 102, 101, "gpu", (1, 1), 40_000, 20_000);
    fake_process(root, 200, 1, "other", (9, 9), 999_999, 999_999);
    fs::create_dir_all(root.join("self")).unwrap();

    let tree = procfs::tree(root, 100);
    assert_eq!(tree, vec![100, 101, 102]);
    let memory = procfs::memory(root, 100, &tree);
    assert_eq!(memory.rss_main_kb, 200_000);
    assert_eq!(memory.rss_tree_kb, 320_000);
    assert_eq!(memory.pss_tree_kb, 200_000);
    assert_eq!(memory.processes, 3);

    let before = procfs::cpu_ticks(root, &tree);
    assert_eq!(before[&100], 60);
    fake_process(root, 100, 1, "antigravity", (70, 10), 200_000, 150_000);
    fake_process(root, 103, 100, "new", (3, 0), 1, 1);
    let after = procfs::cpu_ticks(root, &procfs::tree(root, 100));
    assert_eq!(procfs::ticks_between(&before, &after), 20 + 3);

    let stat = procfs::parse_stat("42 (a) b) c) R 7 0 0 0 0 0 0 0 0 0 11 22 0 0").unwrap();
    assert_eq!((stat.pid, stat.ppid, stat.cpu_ticks), (42, 7, 33));
    // Descendants of a subreaper, without itself.
    let stats = procfs::all_stats(root);
    assert_eq!(procfs::descendants(&stats, 100, false), vec![101, 102, 103]);
}

// ---------------------------------------------------------------- report

#[test]
fn report_builds_the_table_of_the_spec() {
    let mut lines = Vec::new();
    for (index, first) in [300.0, 320.0, 340.0].iter().enumerate() {
        lines.push(json!({"kind": "launch", "app": "cincel", "tag": "cold", "first_content_ms": first, "mapped_ms": first - 50.0, "rep": index}));
        lines.push(json!({"kind": "launch", "app": "zed", "tag": "cold", "first_content_ms": first + 200.0, "mapped_ms": first}));
    }
    lines.push(json!({"kind": "launch", "app": "cincel", "tag": "cold", "error": "falló", "first_content_ms": 9999.0}));
    lines.push(
        json!({"kind": "quick_open", "app": "cincel", "file": "cinco-mil.rs", "stable_ms": 30.0}),
    );
    lines.push(
        json!({"kind": "quick_open", "app": "cincel", "file": "cinco-mil.rs", "stable_ms": 60.0}),
    );
    lines.push(json!({"kind": "type", "app": "cincel", "file": "cinco-mil.rs", "samples_ms": [8.0, 9.0, 30.0], "missed": 1}));
    lines.push(json!({"kind": "scroll", "app": "cincel", "file": "un-mega.rs", "p95_ms": 8.3, "max_ms": 12.5}));
    lines.push(json!({"kind": "launch", "app": "cincel", "tag": "reposo", "rss_tree_kb": 204_800, "pss_tree_kb": 190_000, "rss_main_kb": 204_800, "cpu_pct": 0.0, "idle_frames": 0}));
    lines.push(json!({"kind": "launch", "app": "antigravity", "tag": "reposo", "rss_tree_kb": 900_000, "pss_tree_kb": 600_000, "cpu_pct": 0.4, "idle_frames": 12}));
    lines.push(json!({"scenario": "startup", "total_ms": 180.5}));

    let table = report::render(&lines);
    let header = table.lines().next().unwrap();
    assert_eq!(
        header,
        "| Métrica | Meta | Cincel | Zed | Antigravity | VS Code | ¿Cincel cumple? |"
    );
    let row = |prefix: &str| {
        table
            .lines()
            .find(|line| line.starts_with(&format!("| {prefix}")))
            .unwrap_or_else(|| panic!("falta la fila {prefix}:\n{table}"))
            .to_owned()
    };
    let m1 = row("M1 Arranque en frío → primer cuadro");
    assert!(m1.contains("320,0 ms · p90 336,0 (n=3)"), "{m1}");
    assert!(m1.contains("520,0 ms"), "{m1}");
    assert!(
        m1.contains("no instalado en la máquina de referencia"),
        "{m1}"
    );
    assert!(m1.ends_with("| sí |"), "{m1}");
    let m3 = row("M3");
    assert!(m3.contains("45,0 ms · p95 58,5 (n=2)"), "{m3}");
    assert!(m3.ends_with("| sí |"), "{m3}");
    let m4 = row("M4");
    assert!(
        m4.contains("p50 9,0 · p95 27,9 · máx 30,0 ms (1 sin cuadro)"),
        "{m4}"
    );
    assert!(m4.ends_with("| **no** |"), "{m4}");
    let scroll = row("M5 Scroll");
    assert!(scroll.contains("p95 8,3 · máx 12,5 ms"), "{scroll}");
    assert!(scroll.ends_with("| sí |"));
    let m7 = row("M7");
    assert!(m7.contains("RSS árbol 200 MB · PSS 186 MB"), "{m7}");
    assert!(m7.contains("RSS árbol 879 MB"), "{m7}");
    assert!(m7.ends_with("| sí |"));
    let m8 = row("M8");
    assert!(m8.contains("0,00 % · 0 cuadros"), "{m8}");
    assert!(m8.contains("0,40 % · 12 cuadros"), "{m8}");
    assert!(m8.ends_with("| sí |"));
    assert!(row("M6 Abrir").ends_with("| sin datos |"));
    assert!(row("M9").contains("medición interna"));
    assert!(table.contains("| startup | total_ms=180.5 |"), "{table}");
    // Each row has the same number of columns as the header.
    let columns = header.matches('|').count();
    for line in table.lines().take_while(|line| line.starts_with('|')) {
        assert_eq!(line.matches('|').count(), columns, "{line}");
    }
}

#[test]
fn report_parse_skips_invalid_lines() {
    let (values, invalid) = report::parse_lines("{\"kind\":\"launch\"}\n\nnot json\n[1]\n");
    assert_eq!(values.len(), 1);
    assert_eq!(invalid, 2);
}

// ---------------------------------------------------------------- CLI, keys, profiles

fn args(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn cli_parses_every_subcommand() {
    let Cmd::Launch(launch) = parse(&args(
        "launch --name zed --tag cold --profile /p --settle-ms 0 --idle-ms 0 --setup-keys ctrl+alt+b,escape --keep-open -- /apps/zed/libexec/zed-editor --foo /corpus/medio",
    ))
    .unwrap() else {
        panic!("launch");
    };
    assert_eq!(launch.name, "zed");
    assert_eq!(launch.tag.as_deref(), Some("cold"));
    assert_eq!(launch.protocol, "wlr");
    assert_eq!(
        (launch.settle_ms, launch.idle_ms, launch.timeout_ms),
        (0, 0, 60_000)
    );
    assert!(launch.keep_open);
    assert_eq!(launch.setup_keys, ["ctrl+alt+b", "escape"]);
    assert_eq!(
        launch.command,
        args("/apps/zed/libexec/zed-editor --foo /corpus/medio")
    );

    assert!(parse(&args("launch --name zed")).is_err(), "sin comando");
    assert!(parse(&args("launch --name zed --toplevel-protocol x -- a")).is_err());
    assert!(parse(&args("launch --name zed --bogus -- a")).is_err());
    assert_eq!(
        parse(&args(
            "quick-open --file cinco-mil.rs --app cincel --repeat 5 --close-after"
        ))
        .unwrap(),
        Cmd::QuickOpen {
            app: "cincel".into(),
            file: "cinco-mil.rs".into(),
            repeat: 5,
            close_after: true
        }
    );
    assert_eq!(
        parse(&args("type")).unwrap(),
        Cmd::Type {
            app: "app".into(),
            file: None,
            count: 100,
            interval_ms: 150,
            key: "a".into()
        }
    );
    assert!(matches!(
        parse(&args("scroll --seconds 1.5 --every-ms 16")).unwrap(),
        Cmd::Scroll { seconds, every_ms: 16, .. } if seconds == 1.5
    ));
    assert!(matches!(
        parse(&args("corpus /c --demo --no-large")).unwrap(),
        Cmd::Corpus {
            demo: true,
            large: false,
            git: true,
            ..
        }
    ));
    assert!(parse(&args("corpus")).is_err());
    assert!(matches!(parse(&args("evict /a /b")).unwrap(), Cmd::Evict(paths) if paths.len() == 2));
    assert!(
        matches!(parse(&args("key ctrl+p text:hola --gap-ms 10")).unwrap(), Cmd::Key { combos, gap_ms: 10 } if combos.len() == 2)
    );
    assert!(matches!(
        parse(&args("stop --pid 12")).unwrap(),
        Cmd::Stop {
            pid: 12,
            profile: None
        }
    ));
    assert!(parse(&args("idle --app zed")).is_err());
    assert_eq!(parse(&args("probe")).unwrap(), Cmd::Probe);
    assert_eq!(
        parse(&args("wait-wayland --timeout-ms 5")).unwrap(),
        Cmd::WaitWayland(5)
    );
    assert!(parse(&args("desconocido")).is_err());
    assert_eq!(parse(&[]).unwrap(), Cmd::Help);
}

#[test]
fn key_combos_map_to_evdev_codes() {
    let stroke = parse_combo("ctrl+p").unwrap();
    assert_eq!(stroke.key, 25);
    assert_eq!(stroke.modifier_mask, MASK_CTRL);
    assert_eq!(parse_combo("enter").unwrap().key, 28);
    assert_eq!(parse_combo("backspace").unwrap().key, 14);
    let shifted = char_stroke('_').unwrap();
    assert_eq!((shifted.key, shifted.modifier_mask), (12, MASK_SHIFT));
    assert_eq!(char_stroke('.').unwrap().key, 52);
    assert_eq!(char_stroke('0').unwrap().key, 11);
    assert_eq!(char_stroke('1').unwrap().key, 2);
    assert!(parse_combo("hyper+x").is_err());
    assert!(char_stroke('ñ').is_err());
}

#[test]
fn profiles_are_isolated_and_never_reused() {
    let dir = tempfile::tempdir().unwrap();
    let cincel = dir.path().join("cincel-1");
    let prepared = profile::prepare(AppKind::Cincel, &cincel).unwrap();
    let env: std::collections::HashMap<_, _> = prepared.env.iter().cloned().collect();
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "CINCEL_CONFIG_DIR",
    ] {
        assert!(Path::new(&env[key]).starts_with(&cincel), "{key}");
    }
    let settings = fs::read_to_string(cincel.join("config/cincel/settings.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(parsed["editor"]["cursor_blink"], false);
    assert!(
        profile::prepare(AppKind::Cincel, &cincel).is_err(),
        "un perfil no se reutiliza"
    );

    let zed = dir.path().join("zed-1");
    let prepared = profile::prepare(AppKind::Zed, &zed).unwrap();
    assert_eq!(prepared.args[0], "--user-data-dir");
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(zed.join("zed-data/config/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["enable_language_server"], false);
    assert_eq!(settings["auto_update"], false);
    assert_eq!(settings["telemetry"]["metrics"], false);
    assert_eq!(settings["cursor_blink"], false);

    let code = dir.path().join("ag-1");
    let prepared = profile::prepare(AppKind::Antigravity, &code).unwrap();
    for flag in [
        "--disable-extensions",
        "--disable-workspace-trust",
        "--skip-welcome",
        "--ozone-platform=wayland",
    ] {
        assert!(prepared.args.iter().any(|arg| arg == flag), "{flag}");
    }
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(code.join("user-data/User/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["editor.cursorBlinking"], "solid");
    assert_eq!(settings["telemetry.telemetryLevel"], "off");
    assert_eq!(settings["update.mode"], "none");
    assert_eq!(AppKind::parse("code").unwrap(), AppKind::VsCode);
}

#[test]
fn vscdb_is_a_valid_sqlite_database() {
    let db = crate::vscdb::item_table(&[("zeta", "1"), ("antigravityOnboarding", "true")]).unwrap();
    assert_eq!(db.len(), 3 * 4096);
    assert_eq!(&db[..16], b"SQLite format 3\0");
    assert_eq!(u16::from_be_bytes([db[16], db[17]]), 4096);
    // Schema page (page 1): a table leaf with 2 cells after the header.
    assert_eq!(db[100], 0x0D);
    assert_eq!(u16::from_be_bytes([db[103], db[104]]), 2);
    // Rows (page 2) and the unique index (page 3).
    assert_eq!(db[4096], 0x0D);
    assert_eq!(u16::from_be_bytes([db[4099], db[4100]]), 2);
    assert_eq!(db[8192], 0x0A);
    let text = String::from_utf8_lossy(&db);
    assert!(
        text.contains("CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)")
    );
    assert!(text.contains("antigravityOnboardingtrue"));

    // When Python's sqlite3 is available, SQLite itself checks the file.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.vscdb");
    fs::write(&path, &db).unwrap();
    let script = "import sqlite3, sys\nc = sqlite3.connect(sys.argv[1])\nprint(c.execute('pragma integrity_check').fetchone()[0])\nprint(c.execute(\"select value from ItemTable where key = 'antigravityOnboarding'\").fetchone()[0])\nprint(c.execute('select count(*) from ItemTable').fetchone()[0])";
    if let Ok(output) = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(&path)
        .output()
        && output.status.success()
    {
        assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\ntrue\n2\n");
    }
}

#[test]
fn antigravity_profile_skips_the_onboarding() {
    let dir = tempfile::tempdir().unwrap();
    let profile_dir = dir.path().join("ag");
    profile::prepare(AppKind::Antigravity, &profile_dir).unwrap();
    let db = fs::read(profile_dir.join("user-data/User/globalStorage/state.vscdb")).unwrap();
    assert!(String::from_utf8_lossy(&db).contains(profile::ANTIGRAVITY_ONBOARDING_KEY));
    let code = dir.path().join("code");
    profile::prepare(AppKind::VsCode, &code).unwrap();
    assert!(
        !code
            .join("user-data/User/globalStorage/state.vscdb")
            .exists()
    );
}

#[test]
fn evict_walks_folders() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lib/sub")).unwrap();
    fs::write(dir.path().join("app"), vec![1u8; 4096]).unwrap();
    fs::write(dir.path().join("lib/sub/data.bin"), vec![2u8; 100]).unwrap();
    std::os::unix::fs::symlink("/etc/hostname", dir.path().join("lib/link")).unwrap();
    let evicted = crate::evict::evict(&[dir.path().to_path_buf()]).unwrap();
    assert_eq!(evicted.files, 2, "los enlaces no se siguen");
    assert_eq!(evicted.bytes, 4196);
    assert_eq!(evicted.errors, 0);
}
