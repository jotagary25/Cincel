//! `cincel-perf`: external measurement bench (spec 08 §3.2). It measures
//! Cincel, Zed and VS Code-based editors the same way, without instrumenting
//! them, inside an invisible sway desktop (`tools/perf/run.sh`). Every
//! measurement is one JSON line on stdout.

mod clock;
mod corpus;
mod evict;
mod frames;
mod keys;
mod measure;
mod procfs;
mod profile;
mod report;
mod stats;
mod vscdb;
mod wl;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::measure::{Detection, LaunchOptions};
use crate::wl::ToplevelProtocol;

const USAGE: &str = "\
cincel-perf: banco de medición externo de Cincel (spec 08 §3.2).
Cada medición es una línea JSON en la salida estándar.

Uso:
  cincel-perf corpus DIR [--demo] [--no-large] [--no-git]
  cincel-perf evict RUTA...
  cincel-perf launch --name APP [--tag T] [--app-id ID] [--profile DIR]
                     [--toplevel-protocol wlr|ext|none] [--settle-ms 30000]
                     [--idle-ms 60000] [--timeout-ms 60000] [--open-file NOMBRE]
                     [--setup-keys ctrl+alt+b,...] [--keep-open]
                     [--app-log ARCHIVO] -- COMANDO...
  cincel-perf idle --app APP --pid PID [--settle-ms 30000] [--idle-ms 60000]
  cincel-perf quick-open --file NOMBRE [--app APP] [--repeat 1] [--close-after]
  cincel-perf type [--app APP] [--file NOMBRE] [--count 100] [--interval-ms 150] [--key a]
  cincel-perf scroll [--app APP] [--file NOMBRE] [--seconds 3] [--every-ms 8]
  cincel-perf key COMBINACIÓN... [--gap-ms 150]    (ctrl+p, enter, text:hola)
  cincel-perf stop --pid PID [--profile DIR]
  cincel-perf probe
  cincel-perf wait-wayland [--timeout-ms 10000]
  cincel-perf report ARCHIVO.jsonl...

APP: cincel, zed, antigravity o vscode (define el perfil aislado de --profile).
";

/// A parsed command line.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Corpus {
        dir: PathBuf,
        demo: bool,
        large: bool,
        git: bool,
    },
    Evict(Vec<PathBuf>),
    Launch(Box<LaunchArgs>),
    Idle {
        app: String,
        pid: u32,
        settle_ms: u64,
        idle_ms: u64,
    },
    QuickOpen {
        app: String,
        file: String,
        repeat: usize,
        close_after: bool,
    },
    Type {
        app: String,
        file: Option<String>,
        count: usize,
        interval_ms: u64,
        key: String,
    },
    Scroll {
        app: String,
        file: Option<String>,
        seconds: f64,
        every_ms: u64,
    },
    Key {
        combos: Vec<String>,
        gap_ms: u64,
    },
    Stop {
        pid: u32,
        profile: Option<PathBuf>,
    },
    Probe,
    WaitWayland(u64),
    Report(Vec<PathBuf>),
    Help,
}

/// Arguments of `launch`.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchArgs {
    pub name: String,
    pub tag: Option<String>,
    pub app_id: Option<String>,
    pub profile: Option<PathBuf>,
    pub protocol: String,
    pub settle_ms: u64,
    pub idle_ms: u64,
    pub timeout_ms: u64,
    pub open_file: Option<String>,
    pub setup_keys: Vec<String>,
    pub keep_open: bool,
    pub app_log: Option<PathBuf>,
    pub command: Vec<String>,
}

/// Flags and positional arguments of one subcommand.
struct Args {
    items: Vec<String>,
}

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        match self.items.iter().position(|item| item == name) {
            Some(index) => {
                self.items.remove(index);
                true
            }
            None => false,
        }
    }

    fn value(&mut self, name: &str) -> Result<Option<String>> {
        let Some(index) = self.items.iter().position(|item| item == name) else {
            return Ok(None);
        };
        if index + 1 >= self.items.len() {
            bail!("{name} necesita un valor");
        }
        let value = self.items.remove(index + 1);
        self.items.remove(index);
        Ok(Some(value))
    }

    fn parsed<T: std::str::FromStr>(&mut self, name: &str, default: T) -> Result<T> {
        match self.value(name)? {
            Some(text) => text
                .parse()
                .map_err(|_| anyhow::anyhow!("valor inválido para {name}: {text}")),
            None => Ok(default),
        }
    }

    fn required(&mut self, name: &str) -> Result<String> {
        self.value(name)?.with_context(|| format!("falta {name}"))
    }

    /// Remaining positional arguments; any leftover `--flag` is an error.
    fn positional(self) -> Result<Vec<String>> {
        if let Some(unknown) = self.items.iter().find(|item| item.starts_with("--")) {
            bail!("opción desconocida: {unknown}");
        }
        Ok(self.items)
    }

    fn done(self) -> Result<()> {
        let rest = self.positional()?;
        if let Some(extra) = rest.first() {
            bail!("argumento de más: {extra}");
        }
        Ok(())
    }
}

/// Parses the arguments after the program name.
pub fn parse(argv: &[String]) -> Result<Cmd> {
    let Some((sub, rest)) = argv.split_first() else {
        return Ok(Cmd::Help);
    };
    // Everything after `--` belongs to the launched command.
    let (own, command) = match rest.iter().position(|item| item == "--") {
        Some(index) => (rest[..index].to_vec(), rest[index + 1..].to_vec()),
        None => (rest.to_vec(), Vec::new()),
    };
    let mut args = Args { items: own };
    let cmd = match sub.as_str() {
        "-h" | "--help" | "help" => Cmd::Help,
        "corpus" => {
            let demo = args.flag("--demo");
            let large = !args.flag("--no-large");
            let git = !args.flag("--no-git");
            let positional = args.positional()?;
            let [dir] = positional.as_slice() else {
                bail!("corpus necesita exactamente una carpeta");
            };
            Cmd::Corpus {
                dir: PathBuf::from(dir),
                demo,
                large,
                git,
            }
        }
        "evict" => {
            let paths = args.positional()?;
            if paths.is_empty() {
                bail!("evict necesita al menos una ruta");
            }
            Cmd::Evict(paths.into_iter().map(PathBuf::from).collect())
        }
        "launch" => {
            let launch = LaunchArgs {
                name: args.required("--name")?,
                tag: args.value("--tag")?,
                app_id: args.value("--app-id")?,
                profile: args.value("--profile")?.map(PathBuf::from),
                protocol: args.parsed("--toplevel-protocol", "wlr".to_owned())?,
                settle_ms: args.parsed("--settle-ms", 30_000)?,
                idle_ms: args.parsed("--idle-ms", 60_000)?,
                timeout_ms: args.parsed("--timeout-ms", 60_000)?,
                open_file: args.value("--open-file")?,
                setup_keys: args
                    .value("--setup-keys")?
                    .map(|keys| keys.split(',').map(str::to_owned).collect())
                    .unwrap_or_default(),
                keep_open: args.flag("--keep-open"),
                app_log: args.value("--app-log")?.map(PathBuf::from),
                command: command.clone(),
            };
            args.done()?;
            if !["wlr", "ext", "none"].contains(&launch.protocol.as_str()) {
                bail!("--toplevel-protocol debe ser wlr, ext o none");
            }
            if launch.command.is_empty() {
                bail!("launch necesita el comando de la app después de --");
            }
            Cmd::Launch(Box::new(launch))
        }
        "idle" => {
            let cmd = Cmd::Idle {
                app: args.required("--app")?,
                pid: args.required("--pid")?.parse().context("--pid inválido")?,
                settle_ms: args.parsed("--settle-ms", 30_000)?,
                idle_ms: args.parsed("--idle-ms", 60_000)?,
            };
            args.done()?;
            cmd
        }
        "quick-open" => {
            let cmd = Cmd::QuickOpen {
                file: args.required("--file")?,
                app: args.parsed("--app", "app".to_owned())?,
                repeat: args.parsed("--repeat", 1)?,
                close_after: args.flag("--close-after"),
            };
            args.done()?;
            cmd
        }
        "type" => {
            let cmd = Cmd::Type {
                app: args.parsed("--app", "app".to_owned())?,
                file: args.value("--file")?,
                count: args.parsed("--count", 100)?,
                interval_ms: args.parsed("--interval-ms", 150)?,
                key: args.parsed("--key", "a".to_owned())?,
            };
            args.done()?;
            cmd
        }
        "scroll" => {
            let cmd = Cmd::Scroll {
                app: args.parsed("--app", "app".to_owned())?,
                file: args.value("--file")?,
                seconds: args.parsed("--seconds", 3.0)?,
                every_ms: args.parsed("--every-ms", 8)?,
            };
            args.done()?;
            cmd
        }
        "key" => {
            let gap_ms = args.parsed("--gap-ms", 150)?;
            let combos = args.positional()?;
            if combos.is_empty() {
                bail!("key necesita al menos una combinación");
            }
            Cmd::Key { combos, gap_ms }
        }
        "stop" => {
            let cmd = Cmd::Stop {
                pid: args.required("--pid")?.parse().context("--pid inválido")?,
                profile: args.value("--profile")?.map(PathBuf::from),
            };
            args.done()?;
            cmd
        }
        "probe" => {
            args.done()?;
            Cmd::Probe
        }
        "wait-wayland" => {
            let timeout = args.parsed("--timeout-ms", 10_000)?;
            args.done()?;
            Cmd::WaitWayland(timeout)
        }
        "report" => {
            let files = args.positional()?;
            if files.is_empty() {
                bail!("report necesita al menos un archivo .jsonl");
            }
            Cmd::Report(files.into_iter().map(PathBuf::from).collect())
        }
        other => bail!("subcomando desconocido: {other}"),
    };
    Ok(cmd)
}

fn run(cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Help => print!("{USAGE}"),
        Cmd::Corpus {
            dir,
            demo,
            large,
            git,
        } => {
            let options = corpus::Options {
                scale: corpus::Scale::FULL,
                large,
                demo,
                git,
            };
            for part in corpus::generate(&dir, &options)? {
                println!("{}", part.to_json());
            }
        }
        Cmd::Evict(paths) => {
            let evicted = evict::evict(&paths)?;
            println!(
                "{}",
                json!({
                    "kind": "evict",
                    "files": evicted.files,
                    "bytes": evicted.bytes,
                    "errors": evicted.errors,
                })
            );
        }
        Cmd::Launch(args) => {
            let detection = match args.protocol.as_str() {
                "ext" => Detection::Protocol(ToplevelProtocol::Ext),
                "none" => Detection::None,
                _ => Detection::Protocol(ToplevelProtocol::Wlr),
            };
            measure::launch(&LaunchOptions {
                name: args.name,
                tag: args.tag,
                app_id: args.app_id,
                profile: args.profile,
                detection,
                settle_ms: args.settle_ms,
                idle_ms: args.idle_ms,
                timeout_ms: args.timeout_ms,
                open_file: args.open_file,
                setup_keys: args.setup_keys,
                keep_open: args.keep_open,
                app_log: args.app_log,
                command: args.command,
            })?;
        }
        Cmd::Idle {
            app,
            pid,
            settle_ms,
            idle_ms,
        } => measure::idle(&app, pid, settle_ms, idle_ms)?,
        Cmd::QuickOpen {
            app,
            file,
            repeat,
            close_after,
        } => measure::quick_open(&app, &file, repeat, close_after)?,
        Cmd::Type {
            app,
            file,
            count,
            interval_ms,
            key,
        } => measure::type_keys(&app, file.as_deref(), count, interval_ms, &key)?,
        Cmd::Scroll {
            app,
            file,
            seconds,
            every_ms,
        } => measure::scroll(&app, file.as_deref(), seconds, every_ms)?,
        Cmd::Key { combos, gap_ms } => measure::keys(&combos, gap_ms)?,
        Cmd::Stop { pid, profile } => measure::stop(pid, profile.as_deref())?,
        Cmd::Probe => measure::probe()?,
        Cmd::WaitWayland(timeout) => measure::wait_wayland(timeout)?,
        Cmd::Report(files) => {
            let mut lines = Vec::new();
            let mut invalid = 0;
            for file in files {
                let text = std::fs::read_to_string(&file)
                    .with_context(|| format!("leyendo {}", file.display()))?;
                let (values, bad) = report::parse_lines(&text);
                lines.extend(values);
                invalid += bad;
            }
            print!("{}", report::render(&lines));
            if invalid > 0 {
                eprintln!("cincel-perf: {invalid} líneas no eran JSON y se ignoraron");
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match parse(&argv) {
        Ok(cmd) => cmd,
        Err(error) => {
            eprintln!("cincel-perf: {error:#}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cincel-perf: {error:#}");
            ExitCode::FAILURE
        }
    }
}
