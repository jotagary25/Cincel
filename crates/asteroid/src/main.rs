//! The `asteroid` binary: CLI, logging and bootstrap.
//!
//! Everything the user sees is built by `asteroid-workspace`; this crate only
//! parses the command line, starts logging, opens the GPUI application and
//! hands over (`docs/specs/modulos/workspace.md`, "Binario `asteroid`").

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use asteroid_workspace::Workspace;
use gpui_kit::assets::Assets;

/// How long `--smoke-test` lets the window live before quitting.
const SMOKE_TEST_DURATION: Duration = Duration::from_secs(1);

/// Exit code for "no usable GPU / could not open a window"
/// (`docs/specs/modulos/workspace.md`).
const EXIT_NO_WINDOW: i32 = 2;

fn main() -> anyhow::Result<()> {
    let args = match Cli::parse(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        // `--help` / `--version` already printed what they had to say.
        Ok(None) => return Ok(()),
        Err(error) => {
            eprintln!("asteroid: {error}");
            eprintln!("{USAGE}");
            std::process::exit(64); // EX_USAGE
        }
    };

    init_logging(args.log_level.as_deref());

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        smoke_test = args.smoke_test,
        project = ?args.project,
        "arrancando Asteroid"
    );

    if let Some(project) = &args.project {
        // Stage E0-a has no project model yet; the path is validated and
        // remembered so the CLI contract does not change later.
        if !project.exists() {
            tracing::warn!(path = %project.display(), "la ruta no existe");
        }
    }

    // Counts rendered frames, so `--smoke-test` can tell "the window opened"
    // from "the window opened and actually drew".
    let frames: asteroid_workspace::FrameCounter = Arc::new(AtomicUsize::new(0));

    let app = gpui_kit::application().with_assets(Assets);
    app.run({
        let frames = frames.clone();
        move |cx| {
            asteroid_workspace::init(cx);
            cx.activate(true);

            let open = Workspace::open_window(frames.clone(), cx);
            cx.spawn(async move |_| {
                if let Err(error) = open.await {
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

    Ok(())
}

/// Sends `tracing` (and the `log` records GPUI and wgpu emit) to stderr.
///
/// `RUST_LOG` wins; `--log-level` is the fallback; `info` is the default.
fn init_logging(log_level: Option<&str>) {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level.unwrap_or("info")))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .init();
}

const USAGE: &str = "\
Uso: asteroid [RUTA] [OPCIONES]

Argumentos:
  RUTA                  carpeta del proyecto a abrir

Opciones:
      --log-level NIVEL nivel de log si no hay RUST_LOG (por defecto: info)
      --smoke-test      abre la ventana, dibuja un cuadro y sale con código 0
  -h, --help            muestra esta ayuda
  -V, --version         muestra la versión

Variables de entorno:
  RUST_LOG                     filtro de logs estilo tracing (tiene prioridad)
  ASTEROID_ALLOW_SOFTWARE_GPU  1 para permitir un rasterizador por software
  ZED_DEVICE_ID                fuerza un dispositivo PCI concreto en GPUI";

/// The parsed command line.
#[derive(Debug, Default)]
struct Cli {
    project: Option<PathBuf>,
    log_level: Option<String>,
    smoke_test: bool,
}

impl Cli {
    /// Parses the arguments. `Ok(None)` means the process should exit
    /// successfully without opening a window (`--help`, `--version`).
    fn parse(args: impl Iterator<Item = String>) -> Result<Option<Self>, String> {
        let mut cli = Self::default();
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => {
                    println!("{USAGE}");
                    return Ok(None);
                }
                "-V" | "--version" => {
                    println!("asteroid {}", env!("CARGO_PKG_VERSION"));
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

        Ok(Some(cli))
    }
}

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
}
