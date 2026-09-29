//! The `cincel` binary: CLI, logging and bootstrap.
//!
//! Everything the user sees is built by `cincel-workspace`; this crate parses
//! the command line, starts logging, reads the configuration, opens the GPUI
//! application and hands over (`docs/specs/modulos/workspace.md`, "Binario
//! `cincel`").

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cincel_settings::{Config, SettingsWatcher};
use cincel_workspace::bench;
use cincel_workspace::{Workspace, WorkspaceOptions};
use gpui_kit::assets::AllAssets;

/// How long `--smoke-test` lets the window live before quitting.
const SMOKE_TEST_DURATION: Duration = Duration::from_secs(1);

/// Exit code for "no usable GPU / could not open a window"
/// (`docs/specs/modulos/workspace.md`).
const EXIT_NO_WINDOW: i32 = 2;

fn main() -> anyhow::Result<()> {
    // `cincel --bench` measures from here (`main`); one clock read otherwise.
    let started_ns = bench::now_ns();
    let args = match Cli::parse(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        // `--help`, `--version` and the `--print-*` flags already printed what
        // they had to say.
        Ok(None) => return Ok(()),
        Err(error) => {
            eprintln!("cincel: {error}");
            eprintln!("{USAGE}");
            std::process::exit(64); // EX_USAGE
        }
    };

    // `--bench` (`docs/specs/08-etapa6-cierre-1-0.md` §3.3): before anything
    // reads the XDG directories, since a measurement run moves the state,
    // data and cache ones into its own temporary directory.
    let bench_session = match &args.bench {
        Some(bench_args) => {
            let plan = bench::BenchPlan {
                scenario: bench_args.scenario,
                root: args.project.clone(),
                file: bench_args.file.clone(),
                smoke_test: args.smoke_test,
            };
            match bench::Session::start(plan, started_ns) {
                Ok(session) => Some(session),
                Err(error) => {
                    eprintln!("cincel: --bench: no se pudo preparar la carpeta temporal: {error}");
                    std::process::exit(1);
                }
            }
        }
        None => None,
    };
    let bench_runner = bench_session.as_ref().map(bench::Session::runner);
    // `--smoke-test` without `--bench` also runs in its own temporary state:
    // it opens a project to check that the window draws, and that must never
    // land in the user's recents (it made Cincel reopen the repository).
    let _smoke_state = if args.smoke_test && args.bench.is_none() {
        match bench::isolate_state_in_temp() {
            Ok(dir) => Some(dir),
            Err(error) => {
                eprintln!("cincel: --smoke-test: no se pudo preparar la carpeta temporal: {error}");
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    // Before anything else reads the XDG directories (logging included: the
    // log directory itself is one of them): move `asteroid` to `cincel`
    // wherever it still exists (`docs/specs/06-etapa4-conexiones-y-cincel.md`
    // §7).
    let migrated_dirs = cincel_settings::migrate_xdg_dirs();

    let _log_guard = init_logging(args.log_level.as_deref());

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        smoke_test = args.smoke_test,
        project = ?args.project,
        config_dir = ?cincel_settings::config_dir(),
        data_dir = ?cincel_settings::data_dir(),
        "arrancando Cincel"
    );

    // Logged again here (rather than only inside `migrate_xdg_dirs`, which
    // runs before logging exists and so cannot write to the file) so it
    // actually reaches `cincel.log`, not just stderr-before-init nowhere.
    for migrated in &migrated_dirs {
        tracing::info!(
            root = ?migrated.root,
            old = %migrated.old.display(),
            new = %migrated.new.display(),
            outcome = ?migrated.outcome,
            "Migrada la configuración de Asteroid a Cincel"
        );
    }
    let startup_notices: Vec<String> = if migrated_dirs.is_empty() {
        Vec::new()
    } else {
        vec!["Migrada la configuración de Asteroid a Cincel".to_owned()]
    };

    if let Some(project) = &args.project
        && !project.exists()
    {
        tracing::warn!(path = %project.display(), "la ruta no existe");
    }

    // The configuration never stops the editor from starting: every problem
    // comes back as an issue, is logged here and shown as a toast once the
    // window exists (`docs/specs/modulos/settings.md`).
    let loaded = {
        // `CINCEL_TRACE_TIMINGS=1` (`bench::TIMING_TARGET`).
        let _span = tracing::info_span!(target: "cincel::timing", "config_load").entered();
        Config::load()
    };
    bench::mark(bench::Mark::Config);
    let startup_issues: Vec<String> = loaded
        .issues
        .iter()
        .map(|issue| {
            tracing::warn!(path = %issue.path, message = %issue.message, "ajuste ignorado");
            issue.to_string()
        })
        .collect();
    let config = loaded.value;

    // Counts rendered frames, so `--smoke-test` can tell "the window opened"
    // from "the window opened and actually drew".
    let frames: cincel_workspace::FrameCounter = Arc::new(AtomicUsize::new(0));

    // `AllAssets` embeds the whole Lucide catalog, not just the subset
    // gpui-kit's own components use: the file tree picks an icon per file type
    // (`docs/specs/02-visual.md` §4), and those icons are outside the subset.
    let app = gpui_kit::application().with_assets(AllAssets);
    app.run({
        let frames = frames.clone();
        move |cx| {
            // Before `init` (and before any window opens) so it never shifts
            // what the existing test suite measures: the fonts stay
            // unregistered there and `--smoke-test` is what checks them
            // (`docs/specs/08-etapa6-cierre-1-0.md` §4.2, D7). A failure here
            // is not fatal: the system fallbacks each font-resolving call
            // already tries first take over.
            // `CINCEL_TRACE_TIMINGS=1` (`bench::TIMING_TARGET`).
            let font_span =
                tracing::info_span!(target: "cincel::timing", "font_registration").entered();
            if let Err(error) = cincel_workspace::fonts::register_embedded(cx) {
                tracing::warn!(%error, "no se pudieron registrar las fuentes embebidas");
            }
            drop(font_span);

            cincel_workspace::init(config, cx);
            cx.activate(true);

            // Hot reload of settings, keymap and themes.
            let settings_watcher = match SettingsWatcher::new() {
                Ok(watcher) => Some(watcher),
                Err(error) => {
                    tracing::warn!(%error, "sin recarga en caliente de la configuración");
                    None
                }
            };

            let open = Workspace::open_window(
                WorkspaceOptions {
                    frames: frames.clone(),
                    project: args.project.clone(),
                    // A measurement run never falls back to the last project.
                    open_last_project: args.project.is_none() && args.bench.is_none(),
                    settings_watcher,
                    startup_issues,
                    startup_notices,
                    project_options: Default::default(),
                },
                cx,
            );
            let bench_runner = bench_runner.clone();
            cx.spawn(async move |cx| match open.await {
                Ok(window) => {
                    if let Some(runner) = bench_runner {
                        runner.start(window, cx);
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "no se pudo abrir la ventana");
                    std::process::exit(EXIT_NO_WINDOW);
                }
            })
            .detach();

            if args.smoke_test {
                let frames = frames.clone();
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(SMOKE_TEST_DURATION).await;
                    let drawn = frames.load(Ordering::Relaxed);
                    if drawn == 0 {
                        tracing::error!("smoke test: la ventana no dibujó ningún cuadro");
                        std::process::exit(1);
                    }
                    tracing::info!(frames = drawn, "smoke test: ok, cerrando");
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
        }
    });

    if args.smoke_test && frames.load(Ordering::Relaxed) == 0 {
        tracing::error!("smoke test: la aplicación terminó sin dibujar");
        std::process::exit(1);
    }

    // `--bench`: the temporary directory goes, and a scenario that could not
    // run is exit code 1.
    if let Some(session) = bench_session {
        let code = session.finish();
        if code != 0 {
            std::process::exit(code);
        }
    }

    Ok(())
}

/// Sends `tracing` (and the `log` records GPUI and wgpu emit) to stderr, as
/// before, and to a daily-rotating file under the log directory
/// (`docs/specs/06-etapa4-conexiones-y-cincel.md` §8).
///
/// `RUST_LOG` wins; `--log-level` is the fallback; `info` is the default.
/// The returned guard must stay alive for the whole process: it owns the
/// background thread that flushes the file writer.
fn init_logging(log_level: Option<&str>) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    cincel_log::init(log_level, &log_dir())
}

/// `~/.local/state/cincel/log`, or `$CINCEL_LOG_DIR` when set (tests, or a
/// user who wants the log somewhere else).
fn log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CINCEL_LOG_DIR") {
        return PathBuf::from(dir);
    }
    cincel_settings::state_dir()
        .map(|dir| dir.join("log"))
        .unwrap_or_else(|| PathBuf::from("."))
}

const USAGE: &str = "\
Uso: cincel [RUTA] [OPCIONES]

Argumentos:
  RUTA                  carpeta del proyecto a abrir (sin ella, el último
                        proyecto o la pantalla de bienvenida)

Opciones:
      --log-level NIVEL nivel de log si no hay RUST_LOG (por defecto: info)
      --smoke-test      abre la ventana, dibuja un cuadro y sale con código 0
      --print-default-settings  imprime el settings.json predeterminado
      --print-default-keymap    imprime el keymap.json predeterminado
  -h, --help            muestra esta ayuda
  -V, --version         muestra la versión

Diagnóstico:
      --bench ESCENARIO mide el rendimiento y escribe una línea JSON por
                        medición: startup, open, typing, scroll, idle, finder,
                        sweep, review-1mb, all (los 8 en orden) o demo (turno
                        de ejemplo para capturas, queda abierto hasta SIGTERM).
                        Sin corpus propio, genera uno en una carpeta temporal.
                        RUTA es el proyecto de startup, idle, finder, sweep y
                        demo; sweep escribe en 200 archivos de RUTA y después
                        los deja como estaban
      --bench-file ARCHIVO  archivo de open, typing y scroll
      --smoke-test --bench  el smoke test más los tiempos de arranque

Variables de entorno:
  RUST_LOG                     filtro de logs estilo tracing (tiene prioridad)
  CINCEL_ALLOW_SOFTWARE_GPU    1 para permitir un rasterizador por software
  CINCEL_CONFIG_DIR            reemplaza ~/.config/cincel (para pruebas o una
                                segunda instancia)
  CINCEL_LOG_DIR               reemplaza ~/.local/state/cincel/log
  CINCEL_BENCH_T0              instante de lanzamiento para --bench
                                (nanosegundos de CLOCK_MONOTONIC)
  CINCEL_TRACE_TIMINGS         1 para registrar cuánto tarda cada tramo del
                                arranque y de la apertura del proyecto
  ZED_DEVICE_ID                fuerza un dispositivo PCI concreto en GPUI";

/// The parsed command line.
#[derive(Debug, Default)]
struct Cli {
    project: Option<PathBuf>,
    log_level: Option<String>,
    smoke_test: bool,
    /// `--bench` (`docs/specs/08-etapa6-cierre-1-0.md` §3.3).
    bench: Option<BenchArgs>,
}

/// `--bench ESCENARIO [--bench-file ARCHIVO]`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BenchArgs {
    scenario: bench::Scenario,
    file: Option<PathBuf>,
}

impl Cli {
    /// Parses the arguments. `Ok(None)` means the process should exit
    /// successfully without opening a window (`--help`, `--version`,
    /// `--print-default-settings`, `--print-default-keymap`).
    fn parse(args: impl Iterator<Item = String>) -> Result<Option<Self>, String> {
        let mut cli = Self::default();
        let mut args = args.peekable();
        // `--bench` takes the next word when it names a scenario; alone it
        // only makes sense with `--smoke-test` (`startup`).
        let mut bench_requested = false;
        let mut bench_scenario = None;
        let mut bench_word = None;
        let mut bench_file = None;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--bench" => {
                    bench_requested = true;
                    if let Some(scenario) = args
                        .peek()
                        .and_then(|next| bench::Scenario::from_name(next))
                    {
                        bench_scenario = Some(scenario);
                        args.next();
                    } else if let Some(next) = args.peek().filter(|next| !next.starts_with('-')) {
                        bench_word = Some(next.clone());
                    }
                }
                "--bench-file" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--bench-file necesita un archivo".to_string())?;
                    bench_file = Some(PathBuf::from(value));
                }
                other if other.starts_with("--bench-file=") => {
                    bench_file = Some(PathBuf::from(&other["--bench-file=".len()..]));
                }
                "-h" | "--help" => {
                    println!("{USAGE}");
                    return Ok(None);
                }
                "-V" | "--version" => {
                    println!("cincel {}", env!("CARGO_PKG_VERSION"));
                    return Ok(None);
                }
                "--print-default-settings" => {
                    print!("{}", cincel_settings::default_settings_jsonc());
                    return Ok(None);
                }
                "--print-default-keymap" => {
                    print!("{}", cincel_settings::default_keymap_jsonc());
                    return Ok(None);
                }
                "--smoke-test" => cli.smoke_test = true,
                "--log-level" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--log-level necesita un valor".to_string())?;
                    cli.log_level = Some(value);
                }
                other if other.starts_with("--log-level=") => {
                    cli.log_level = Some(other["--log-level=".len()..].to_string());
                }
                other if other.starts_with('-') && other != "-" => {
                    return Err(format!("opción desconocida: {other}"));
                }
                other => {
                    if cli.project.is_some() {
                        return Err(format!("sobra el argumento: {other}"));
                    }
                    cli.project = Some(PathBuf::from(other));
                }
            }
        }

        if bench_requested {
            let scenario = match (bench_scenario, cli.smoke_test) {
                (None, true) | (Some(bench::Scenario::Startup), true) => bench::Scenario::Startup,
                (Some(_), true) => {
                    return Err(
                        "--smoke-test --bench solo mide el arranque: quitá el escenario"
                            .to_string(),
                    );
                }
                (Some(scenario), false) => scenario,
                (None, false) => {
                    return Err(match bench_word {
                        Some(word) => format!(
                            "escenario de --bench desconocido: {word} (válidos: {})",
                            bench::Scenario::names()
                        ),
                        None => format!(
                            "--bench necesita un escenario: {}",
                            bench::Scenario::names()
                        ),
                    });
                }
            };
            cli.bench = Some(BenchArgs {
                scenario,
                file: bench_file,
            });
        } else if bench_file.is_some() {
            return Err("--bench-file solo tiene sentido con --bench".to_string());
        }

        Ok(Some(cli))
    }
}

#[cfg(test)]
mod bench_cli_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Cli>, String> {
        Cli::parse(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn parses_an_empty_command_line() {
        let cli = parse(&[]).unwrap().unwrap();
        assert!(cli.project.is_none());
        assert!(!cli.smoke_test);
    }

    #[test]
    fn parses_a_project_path_and_flags() {
        let cli = parse(&["/tmp/proyecto", "--smoke-test", "--log-level", "debug"])
            .unwrap()
            .unwrap();
        assert_eq!(cli.project, Some(PathBuf::from("/tmp/proyecto")));
        assert!(cli.smoke_test);
        assert_eq!(cli.log_level.as_deref(), Some("debug"));
    }

    #[test]
    fn accepts_the_inline_form_of_log_level() {
        let cli = parse(&["--log-level=trace"]).unwrap().unwrap();
        assert_eq!(cli.log_level.as_deref(), Some("trace"));
    }

    #[test]
    fn rejects_unknown_options_and_extra_arguments() {
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["uno", "dos"]).is_err());
        assert!(parse(&["--log-level"]).is_err());
    }

    #[test]
    fn help_and_version_ask_for_a_clean_exit() {
        assert!(parse(&["--help"]).unwrap().is_none());
        assert!(parse(&["-V"]).unwrap().is_none());
    }

    #[test]
    fn printing_the_defaults_asks_for_a_clean_exit() {
        assert!(parse(&["--print-default-settings"]).unwrap().is_none());
        assert!(parse(&["--print-default-keymap"]).unwrap().is_none());
    }

    #[test]
    fn the_printed_defaults_are_the_ones_settings_parses() {
        let settings = cincel_settings::Settings::parse(&cincel_settings::default_settings_jsonc());
        assert!(settings.is_clean(), "{:?}", settings.issues);
        let keymap = cincel_settings::Keymap::parse(&cincel_settings::default_keymap_jsonc());
        assert!(keymap.is_clean(), "{:?}", keymap.issues);
    }
}
