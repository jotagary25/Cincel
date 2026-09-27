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
use cincel_workspace::{Workspace, WorkspaceOptions};
use gpui_kit::assets::AllAssets;

/// How long `--smoke-test` lets the window live before quitting.
const SMOKE_TEST_DURATION: Duration = Duration::from_secs(1);

/// Exit code for "no usable GPU / could not open a window"
/// (`docs/specs/modulos/workspace.md`).
const EXIT_NO_WINDOW: i32 = 2;

fn main() -> anyhow::Result<()> {
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
    let loaded = Config::load();
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
                    open_last_project: args.project.is_none(),
                    settings_watcher,
                    startup_issues,
                    startup_notices,
                    project_options: Default::default(),
                },
                cx,
            );
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

Variables de entorno:
  RUST_LOG                     filtro de logs estilo tracing (tiene prioridad)
  CINCEL_ALLOW_SOFTWARE_GPU    1 para permitir un rasterizador por software
  CINCEL_CONFIG_DIR            reemplaza ~/.config/cincel (para pruebas o una
                                segunda instancia)
  CINCEL_LOG_DIR               reemplaza ~/.local/state/cincel/log
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
    /// successfully without opening a window (`--help`, `--version`,
    /// `--print-default-settings`, `--print-default-keymap`).
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
