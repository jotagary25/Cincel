//! `Cli::parse` with `--bench` (`docs/specs/08-etapa6-cierre-1-0.md` §3.10:
//! "`Cli::parse` con `--bench` y sus escenarios; escenario desconocido →
//! error de uso").

use std::path::PathBuf;

use cincel_workspace::bench::Scenario;

use super::{BenchArgs, Cli};

fn parse(args: &[&str]) -> Result<Option<Cli>, String> {
    Cli::parse(args.iter().map(|arg| arg.to_string()))
}

fn bench_of(args: &[&str]) -> BenchArgs {
    parse(args)
        .expect("la línea de comandos es válida")
        .expect("abre la ventana")
        .bench
        .expect("pide --bench")
}

#[test]
fn every_scenario_parses_by_its_name() {
    for scenario in Scenario::ALL {
        let bench = bench_of(&["--bench", scenario.name()]);
        assert_eq!(bench.scenario, scenario);
        assert_eq!(bench.file, None);
    }
    assert_eq!(Scenario::ALL.len(), 10);
    assert_eq!(Scenario::MEASUREMENTS.len(), 8);
}

#[test]
fn the_path_and_the_bench_file_go_with_the_scenario() {
    let cli = parse(&[
        "--bench",
        "open",
        "/tmp/corpus",
        "--bench-file",
        "/tmp/corpus/cinco-mil.rs",
    ])
    .unwrap()
    .unwrap();
    assert_eq!(cli.project, Some(PathBuf::from("/tmp/corpus")));
    let bench = cli.bench.unwrap();
    assert_eq!(bench.scenario, Scenario::Open);
    assert_eq!(bench.file, Some(PathBuf::from("/tmp/corpus/cinco-mil.rs")));

    let inline = bench_of(&["--bench", "typing", "--bench-file=/tmp/a.rs"]);
    assert_eq!(inline.file, Some(PathBuf::from("/tmp/a.rs")));

    // The path may come first too.
    let cli = parse(&["demo/", "--bench", "demo"]).unwrap().unwrap();
    assert_eq!(cli.project, Some(PathBuf::from("demo/")));
    assert_eq!(cli.bench.unwrap().scenario, Scenario::Demo);
}

#[test]
fn smoke_test_with_bench_alone_is_startup_and_keeps_the_path() {
    // `target/release/cincel --smoke-test --bench .` (§13).
    let cli = parse(&["--smoke-test", "--bench", "."]).unwrap().unwrap();
    assert!(cli.smoke_test);
    assert_eq!(cli.project, Some(PathBuf::from(".")));
    assert_eq!(cli.bench.unwrap().scenario, Scenario::Startup);

    let cli = parse(&["--bench", "--smoke-test"]).unwrap().unwrap();
    assert_eq!(cli.bench.unwrap().scenario, Scenario::Startup);
    assert!(cli.project.is_none());

    let cli = parse(&["--smoke-test", "--bench", "startup", "."])
        .unwrap()
        .unwrap();
    assert_eq!(cli.bench.unwrap().scenario, Scenario::Startup);
}

#[test]
fn an_unknown_or_missing_scenario_is_a_usage_error() {
    let error = parse(&["--bench", "rapidez"]).unwrap_err();
    assert!(error.contains("desconocido: rapidez"), "{error}");
    assert!(error.contains("review-1mb"), "{error}");

    let error = parse(&["--bench"]).unwrap_err();
    assert!(error.contains("necesita un escenario"), "{error}");

    // Only `startup` goes with the smoke test.
    assert!(parse(&["--smoke-test", "--bench", "idle"]).is_err());
    // `--bench-file` without `--bench`, or without its value.
    assert!(parse(&["--bench-file", "a.rs"]).is_err());
    assert!(parse(&["--bench", "open", "--bench-file"]).is_err());
}

#[test]
fn without_bench_nothing_changes() {
    let cli = parse(&["/tmp/proyecto", "--smoke-test"]).unwrap().unwrap();
    assert!(cli.bench.is_none());
}
