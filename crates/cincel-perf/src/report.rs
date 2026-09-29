//! `report`: the Markdown table of §3.7 from the JSON lines of a run.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;

use crate::corpus::{FIFTY_THOUSAND, FIVE_THOUSAND, ONE_MEGABYTE};
use crate::stats::{Summary, summarize};

/// Columns of the table, in order.
pub const APPS: &[(&str, &str)] = &[
    ("cincel", "Cincel"),
    ("zed", "Zed"),
    ("antigravity", "Antigravity"),
    ("vscode", "VS Code"),
];

/// External acceptance of M4 (16 ms + one 240 Hz frame, §3.2).
pub const TYPING_EXTERNAL_MS: f64 = 20.2;

/// Parses JSON lines, skipping blank and invalid ones (they are counted).
pub fn parse_lines(text: &str) -> (Vec<Value>, usize) {
    let mut values = Vec::new();
    let mut invalid = 0;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        match serde_json::from_str::<Value>(line) {
            Ok(value @ Value::Object(_)) => values.push(value),
            _ => invalid += 1,
        }
    }
    (values, invalid)
}

/// Decimal number the Spanish way (`12,3`).
pub fn num(value: f64, decimals: usize) -> String {
    format!("{value:.decimals$}").replace('.', ",")
}

fn field<'a>(line: &'a Value, name: &str) -> Option<&'a str> {
    line.get(name).and_then(Value::as_str)
}

fn number(line: &Value, name: &str) -> Option<f64> {
    line.get(name).and_then(Value::as_f64)
}

fn is_ok(line: &Value) -> bool {
    line.get("error").is_none_or(Value::is_null)
}

struct Data<'a> {
    lines: &'a [Value],
}

impl<'a> Data<'a> {
    fn select(&self, kind: &str, app: &str) -> impl Iterator<Item = &'a Value> {
        let kind = kind.to_owned();
        let app = app.to_owned();
        self.lines.iter().filter(move |line| {
            field(line, "kind") == Some(kind.as_str())
                && field(line, "app") == Some(app.as_str())
                && is_ok(line)
        })
    }

    fn values(
        &self,
        kind: &str,
        app: &str,
        filter: impl Fn(&Value) -> bool,
        name: &str,
    ) -> Vec<f64> {
        self.select(kind, app)
            .filter(|line| filter(line))
            .filter_map(|line| number(line, name))
            .collect()
    }

    fn has_app(&self, app: &str) -> bool {
        self.lines
            .iter()
            .any(|line| field(line, "app") == Some(app) && field(line, "kind") != Some("stop"))
    }

    fn idle_lines(&self, app: &str) -> Vec<&'a Value> {
        self.lines
            .iter()
            .filter(|line| field(line, "app") == Some(app) && is_ok(line))
            .filter(|line| match field(line, "kind") {
                Some("idle") => true,
                Some("launch") => field(line, "tag") == Some("reposo"),
                _ => false,
            })
            .filter(|line| number(line, "cpu_pct").is_some())
            .collect()
    }

    /// Pooled typing samples of `file`.
    fn typing(&self, app: &str, file: &str) -> (Vec<f64>, u64) {
        let mut samples = Vec::new();
        let mut missed = 0;
        for line in self
            .select("type", app)
            .filter(|l| field(l, "file") == Some(file))
        {
            if let Some(values) = line.get("samples_ms").and_then(Value::as_array) {
                samples.extend(values.iter().filter_map(Value::as_f64));
            }
            missed += line.get("missed").and_then(Value::as_u64).unwrap_or(0);
        }
        (samples, missed)
    }

    fn scroll(&self, app: &str, file: &str) -> Option<(f64, f64)> {
        let lines: Vec<&Value> = self
            .select("scroll", app)
            .filter(|l| field(l, "file") == Some(file))
            .collect();
        let p95 = lines
            .iter()
            .filter_map(|l| number(l, "p95_ms"))
            .fold(None, max_opt);
        let max = lines
            .iter()
            .filter_map(|l| number(l, "max_ms"))
            .fold(None, max_opt);
        p95.zip(max)
    }
}

fn max_opt(acc: Option<f64>, value: f64) -> Option<f64> {
    Some(acc.map_or(value, |acc| acc.max(value)))
}

fn median_p(summary: Option<Summary>, second: &str) -> String {
    match summary {
        Some(s) => {
            let other = if second == "p90" { s.p90 } else { s.p95 };
            format!(
                "{} ms · {second} {} (n={})",
                num(s.p50, 1),
                num(other, 1),
                s.count
            )
        }
        None => "—".to_owned(),
    }
}

/// One table row: label, goal, a cell per app and the verdict for Cincel.
struct Row {
    label: String,
    goal: String,
    cells: Vec<String>,
    verdict: String,
}

fn verdict(value: Option<bool>) -> String {
    match value {
        Some(true) => "sí".to_owned(),
        Some(false) => "**no**".to_owned(),
        None => "sin datos".to_owned(),
    }
}

/// Renders the §3.7 table (plus the internal measurements of Cincel, if
/// any line carries a `scenario`).
pub fn render(lines: &[Value]) -> String {
    let data = Data { lines };
    let present: Vec<bool> = APPS.iter().map(|(app, _)| data.has_app(app)).collect();
    let absent_note = |index: usize| -> Option<String> {
        (!present[index]).then(|| {
            if APPS[index].0 == "vscode" {
                "no instalado en la máquina de referencia".to_owned()
            } else {
                "sin medir".to_owned()
            }
        })
    };
    let mut rows: Vec<Row> = Vec::new();
    let per_app = |make: &dyn Fn(&str) -> String| -> Vec<String> {
        APPS.iter()
            .enumerate()
            .map(|(index, (app, _))| absent_note(index).unwrap_or_else(|| make(app)))
            .collect()
    };

    // M1 / M2: start.
    for (id, tag, label) in [
        (
            "M1",
            "cold",
            "Arranque en frío → primer cuadro con contenido",
        ),
        ("M1", "cold-mapped", "Arranque en frío → ventana mapeada"),
        ("M2", "warm", "Arranque tibio → primer cuadro con contenido"),
    ] {
        let (tag, name) = match tag {
            "cold-mapped" => ("cold", "mapped_ms"),
            other => (other, "first_content_ms"),
        };
        let summary_of = |app: &str| {
            summarize(&data.values("launch", app, |l| field(l, "tag") == Some(tag), name))
        };
        let cincel = summary_of("cincel");
        let (goal, pass) = if id == "M1" && name == "first_content_ms" {
            ("< 400 ms", cincel.map(|s| s.p50 < 400.0))
        } else {
            ("informativa", None)
        };
        rows.push(Row {
            label: format!("{id} {label}"),
            goal: goal.to_owned(),
            cells: per_app(&|app| median_p(summary_of(app), "p90")),
            verdict: if goal == "informativa" {
                "—".to_owned()
            } else {
                verdict(pass)
            },
        });
    }

    // M3: open the 5 000-line file.
    let open_summary = |app: &str, file: &str| {
        summarize(&data.values(
            "quick_open",
            app,
            |l| field(l, "file") == Some(file),
            "stable_ms",
        ))
    };
    rows.push(Row {
        label: format!("M3 Abrir `{FIVE_THOUSAND}` (Ctrl+P → cuadro estable)"),
        goal: "< 50 ms".to_owned(),
        cells: per_app(&|app| median_p(open_summary(app, FIVE_THOUSAND), "p95")),
        verdict: verdict(open_summary("cincel", FIVE_THOUSAND).map(|s| s.p50 < 50.0)),
    });

    // M4: typing in the 5 000-line file.
    let typing_cell = |app: &str, file: &str| {
        let (samples, missed) = data.typing(app, file);
        match summarize(&samples) {
            Some(s) => format!(
                "p50 {} · p95 {} · máx {} ms{}",
                num(s.p50, 1),
                num(s.p95, 1),
                num(s.max, 1),
                if missed > 0 {
                    format!(" ({missed} sin cuadro)")
                } else {
                    String::new()
                }
            ),
            None => "—".to_owned(),
        }
    };
    let typing_pass =
        |file: &str| summarize(&data.typing("cincel", file).0).map(|s| s.p95 <= TYPING_EXTERNAL_MS);
    rows.push(Row {
        label: format!("M4 Tecla → pantalla en `{FIVE_THOUSAND}`"),
        goal: format!("< 16 ms (externa: p95 ≤ {} ms)", num(TYPING_EXTERNAL_MS, 1)),
        cells: per_app(&|app| typing_cell(app, FIVE_THOUSAND)),
        verdict: verdict(typing_pass(FIVE_THOUSAND)),
    });

    // M5 / M6: the big files.
    for (id, file) in [("M5", ONE_MEGABYTE), ("M6", FIFTY_THOUSAND)] {
        rows.push(Row {
            label: format!("{id} Abrir `{file}`"),
            goal: "< 200 ms".to_owned(),
            cells: per_app(&|app| median_p(open_summary(app, file), "p95")),
            verdict: verdict(open_summary("cincel", file).map(|s| s.p50 < 200.0)),
        });
        rows.push(Row {
            label: format!("{id} Scroll en `{file}` (intervalo entre cuadros)"),
            goal: "p95 ≤ 16,7 ms, ninguno > 33 ms".to_owned(),
            cells: per_app(&|app| match data.scroll(app, file) {
                Some((p95, max)) => format!("p95 {} · máx {} ms", num(p95, 1), num(max, 1)),
                None => "—".to_owned(),
            }),
            verdict: verdict(
                data.scroll("cincel", file)
                    .map(|(p95, max)| p95 <= 16.7 && max <= 33.4),
            ),
        });
        rows.push(Row {
            label: format!("{id} Tecla → pantalla en `{file}`"),
            goal: format!("p95 < 16 ms (externa ≤ {} ms)", num(TYPING_EXTERNAL_MS, 1)),
            cells: per_app(&|app| typing_cell(app, file)),
            verdict: verdict(typing_pass(file)),
        });
    }

    // M7 / M8: idle.
    let idle = |app: &str, name: &str| -> Option<f64> {
        let values: Vec<f64> = data
            .idle_lines(app)
            .iter()
            .filter_map(|l| number(l, name))
            .collect();
        summarize(&values).map(|s| s.p50)
    };
    rows.push(Row {
        label: "M7 Memoria en reposo (proyecto mediano, un archivo abierto)".to_owned(),
        goal: "< 300 MB (RSS del árbol)".to_owned(),
        cells: per_app(&|app| match (
            idle(app, "rss_tree_kb"),
            idle(app, "pss_tree_kb"),
            idle(app, "rss_main_kb"),
        ) {
            (Some(rss), Some(pss), main) => format!(
                "RSS árbol {} MB · PSS {} MB{}",
                num(rss / 1024.0, 0),
                num(pss / 1024.0, 0),
                main.map(|m| format!(" · RSS principal {} MB", num(m / 1024.0, 0)))
                    .unwrap_or_default()
            ),
            _ => "—".to_owned(),
        }),
        verdict: verdict(idle("cincel", "rss_tree_kb").map(|rss| rss / 1024.0 < 300.0)),
    });
    rows.push(Row {
        label: "M8 CPU en reposo (60 s)".to_owned(),
        goal: "≤ 0,1 % y 0 cuadros".to_owned(),
        cells: per_app(
            &|app| match (idle(app, "cpu_pct"), idle(app, "idle_frames")) {
                (Some(cpu), frames) => format!(
                    "{} % · {} cuadros",
                    num(cpu, 2),
                    frames.map(|f| num(f, 0)).unwrap_or_else(|| "?".to_owned())
                ),
                _ => "—".to_owned(),
            },
        ),
        verdict: verdict(
            match (idle("cincel", "cpu_pct"), idle("cincel", "idle_frames")) {
                (Some(cpu), Some(frames)) => Some(cpu <= 0.1 && frames == 0.0),
                _ => None,
            },
        ),
    });
    for (id, label, goal) in [
        ("M9", "Tamaño del binario", "20–60 MB"),
        ("M10", "Buscador con 50 000 rutas", "< 50 ms"),
        (
            "M11",
            "Repaso final del turno (proyecto grande)",
            "sin bloqueos > 8 ms",
        ),
    ] {
        rows.push(Row {
            label: format!("{id} {label}"),
            goal: goal.to_owned(),
            cells: APPS
                .iter()
                .map(|(app, _)| {
                    if *app == "cincel" {
                        "medición interna".to_owned()
                    } else {
                        "—".to_owned()
                    }
                })
                .collect(),
            verdict: "ver mediciones internas".to_owned(),
        });
    }

    let mut out = String::new();
    out.push_str("| Métrica | Meta |");
    for (_, title) in APPS {
        let _ = write!(out, " {title} |");
    }
    out.push_str(" ¿Cincel cumple? |\n|---|---|");
    for _ in APPS {
        out.push_str("---|");
    }
    out.push_str("---|\n");
    for row in &rows {
        let _ = write!(out, "| {} | {} |", row.label, row.goal);
        for cell in &row.cells {
            let _ = write!(out, " {cell} |");
        }
        let _ = writeln!(out, " {} |", row.verdict);
    }

    let internal: Vec<&Value> = lines
        .iter()
        .filter(|line| line.get("scenario").is_some())
        .collect();
    if !internal.is_empty() {
        out.push_str("\n**Mediciones internas de Cincel** (`cincel --bench`)\n\n| Escenario | Valores |\n|---|---|\n");
        let mut by_scenario: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for line in internal {
            let scenario = line["scenario"].as_str().unwrap_or("?").to_owned();
            let Value::Object(fields) = line else {
                continue;
            };
            let values: Vec<String> = fields
                .iter()
                .filter(|(key, _)| key.as_str() != "scenario")
                .map(|(key, value)| format!("{key}={value}"))
                .collect();
            by_scenario
                .entry(scenario)
                .or_default()
                .push(values.join(", "));
        }
        for (scenario, entries) in by_scenario {
            for entry in entries {
                let _ = writeln!(out, "| {scenario} | {} |", entry.replace('|', "\\|"));
            }
        }
    }
    out
}
